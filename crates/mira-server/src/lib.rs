//! HTTP + WebSocket server that fronts a Mira `Session` for browser UIs.
//!
//! The CLI builds the harness stack (provider, registry, policy, session)
//! and hands it to [`run`]. The server does the rest: WebSocket at `/ws`
//! for realtime chat, plus static assets under `/` served from disk (dev)
//! or embedded (release, later).
//!
//! ## Wire model
//!
//! - One long-lived `Session` per server invocation.
//! - Outbound events fan out through a `tokio::sync::broadcast` — every
//!   connected WS subscribes.
//! - Approval prompts arrive as [`protocol::ServerMsg::ApprovalRequest`];
//!   the client answers with [`protocol::ClientMsg::Approve`], which
//!   routes back to the awaiting oneshot via [`approver::resolve`].
//! - `GET/PUT /api/settings` reads/writes `~/.mira/mira.yaml`. `PUT`
//!   rebuilds the provider and swaps it into the live session with no
//!   restart required.

pub mod approver;
mod browse;
mod cwd;
mod embedded;
mod file;
mod git;
pub mod interactive;
mod memory;
mod models;
pub mod protocol;
pub mod provider;
mod review;
mod sessions;
mod settings;
mod state;
mod title;
mod undo;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::get;
use axum::Router;
use mira_ai::ChatProvider;
use mira_harness::{Approver, FileStore, Session, SessionConfig, SessionRecord, SessionStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::{Registry, ToolContext};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex, RwLock};
use tower_http::trace::TraceLayer;
use tracing::info;

pub use crate::approver::WsApprover;
pub use crate::provider::SwappableProvider;
pub use crate::state::AppState;

/// True when the binary was built with a real (non-empty) `frontend/dist/`
/// tree — i.e. the UI ships without needing `--static-dir`.
pub fn has_embedded_frontend() -> bool {
    embedded::has_frontend()
}

/// Build args for [`run`]. Everything the server needs except the session,
/// which is constructed inside `run` so it can bake in the WsApprover.
pub struct ServerConfig {
    pub cfg: SessionConfig,
    pub provider: Arc<dyn ChatProvider>,
    pub registry: Arc<Registry>,
    pub policy: Arc<Mutex<Policy>>,
    pub sandbox: Arc<Sandbox>,
    pub cwd: PathBuf,
    pub store: Option<Arc<dyn SessionStore>>,
    pub resume: Option<SessionRecord>,
    /// Address to bind. Use `127.0.0.1:PORT` for local-only.
    pub bind: SocketAddr,
    /// Optional path to a directory with a built frontend (index.html + assets).
    /// If `None`, the server serves an inline placeholder page at `/`.
    pub static_dir: Option<PathBuf>,
}

/// Start the server. Blocks until the process is signaled to exit.
pub async fn run(mut cfg: ServerConfig) -> Result<()> {
    let (events_tx, _rx0) = broadcast::channel::<protocol::ServerMsg>(256);
    let pending = Arc::new(Mutex::new(HashMap::new()));

    // Interactive-tool wiring. Instantiate the channel first so we can hand
    // it to any server-owned Tool (plan, ask_user, …) before those tools go
    // into the registry the harness sees.
    let prompt_channel = interactive::PromptChannel::new(events_tx.clone());
    let prompt_pending = prompt_channel.pending();

    // Copy the caller-provided registry and layer on server-only interactive
    // tools. Registry is `Clone`, so this is cheap; the resulting Arc<Registry>
    // is what the session actually consults.
    let mut registry_owned: Registry = (*cfg.registry).clone();
    registry_owned.register(interactive::PlanTool::new(prompt_channel.clone()));
    let registry = Arc::new(registry_owned);
    cfg.registry = registry.clone();

    let swappable = SwappableProvider::new(cfg.provider.clone());
    let harness_provider: Arc<dyn ChatProvider> = Arc::new(swappable.clone());
    let cwd = Arc::new(RwLock::new(cfg.cwd.clone()));
    let approver: Arc<dyn Approver> = Arc::new(WsApprover::new(
        events_tx.clone(),
        pending.clone(),
        cwd.clone(),
    ));

    let initial_ctx = ToolContext::new(cfg.cwd.clone(), cfg.sandbox.clone());
    let mut session = match cfg.resume.take() {
        Some(record) => Session::resume_from(
            record,
            harness_provider.clone(),
            cfg.registry.clone(),
            cfg.policy.clone(),
            approver.clone(),
            initial_ctx,
        ),
        None => Session::new(
            cfg.cfg.clone(),
            system_prompt(&cfg.cwd, &cfg.registry),
            harness_provider.clone(),
            cfg.registry.clone(),
            cfg.policy.clone(),
            approver.clone(),
            initial_ctx,
        ),
    };
    if let Some(store) = cfg.store.clone() {
        session = session.with_store(store);
    }

    let state = AppState {
        session: Arc::new(RwLock::new(session)),
        policy: cfg.policy.clone(),
        cwd,
        provider: swappable,
        events_tx,
        pending,
        prompt_pending,
        registry: cfg.registry.clone(),
        sandbox: cfg.sandbox.clone(),
        approver,
        harness_provider,
        store: cfg.store.clone(),
    };

    let router = build_router(state, cfg.static_dir.clone());

    let listener = TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    info!(addr = %cfg.bind, "mira serve: listening");

    axum::serve(listener, router).await.context("axum serve")?;
    Ok(())
}

fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    let mut router = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/health", get(health))
        .route(
            "/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        .route("/api/sessions", get(sessions::list_sessions))
        .route(
            "/api/sessions/:id/load",
            axum::routing::post(sessions::load_session),
        )
        .route(
            "/api/sessions/new",
            axum::routing::post(sessions::new_session),
        )
        .route(
            "/api/sessions/:id",
            axum::routing::delete(sessions::delete_session),
        )
        .route("/api/cwd", get(cwd::get_cwd).put(cwd::put_cwd))
        .route("/api/browse", get(browse::browse))
        .route("/api/file", get(file::read_file))
        .route("/api/models", get(models::list_models))
        .route("/api/git/status", get(git::get_status))
        .route(
            "/api/git/worktree",
            axum::routing::post(git::create_worktree),
        )
        .route("/api/memory", get(memory::get_memory))
        .route(
            "/api/memory/append",
            axum::routing::post(memory::append_memory),
        )
        .route("/api/review", axum::routing::post(review::start_review))
        .route("/api/undo", axum::routing::post(undo::apply_undo));

    // Frontend precedence: `--static-dir` (dev/override) > embedded assets
    // baked at compile time > inline placeholder page.
    router = if let Some(dir) = static_dir {
        router.fallback_service(tower_http::services::ServeDir::new(dir))
    } else if embedded::has_frontend() {
        router.fallback(embedded_fallback)
    } else {
        router.route("/", get(inline_index))
    };

    router.with_state(state).layer(TraceLayer::new_for_http())
}

async fn embedded_fallback(uri: axum::http::Uri) -> axum::response::Response {
    embedded::serve(uri.path()).await
}

async fn health() -> &'static str {
    "ok"
}

async fn inline_index() -> axum::response::Html<&'static str> {
    // Placeholder until the built React app is available. Enough to confirm
    // the server is up and shove a WS ping through in the browser console.
    axum::response::Html(include_str!("../assets/placeholder.html"))
}

/// Convenience for the CLI: open a default session store if the user did not
/// pass `--no-persist`.
pub fn default_store() -> Result<Arc<dyn SessionStore>> {
    let store = FileStore::open_default().context("open session store")?;
    Ok(Arc::new(store))
}

/// System prompt for a session bound to `cwd`. Kept here (rather than in the
/// CLI) so `new_session` and `put_cwd` can recompute it from the current
/// folder — the prompt would otherwise lie about the working directory when
/// the user switches folders mid-session.
///
/// Bakes the actual `registry.specs()` list into the prompt so the model
/// stops narrating capabilities it doesn't have (e.g. it used to confidently
/// claim `WebSearch` / `WebFetch` because they're common in its training
/// data). Also drops a "bash unlocks" hint — the model tends to think of
/// `bash` as a fallback rather than the powerful escape hatch it is.
pub fn system_prompt(cwd: &std::path::Path, registry: &Registry) -> String {
    let tool_lines: Vec<String> = registry
        .specs()
        .into_iter()
        .map(|s| format!("- {}: {}", s.name, first_sentence(&s.description)))
        .collect();
    let tools_block = if tool_lines.is_empty() {
        String::from("(no tools registered — you have only free-form text.)")
    } else {
        tool_lines.join("\n")
    };

    let base = format!(
        "You are Mira, an interactive coding agent.\n\
         Working directory: {cwd}\n\n\
         Available tools:\n{tools_block}\n\n\
         `bash` is a general-purpose escape hatch — through it you can run \
         git, gh, docker, npm/pnpm/yarn, cargo, curl, jq, ripgrep, kubectl, \
         and any other CLI on the user's system. Prefer a dedicated tool \
         (edit_file, grep, …) when one fits; drop to bash for everything \
         else instead of claiming you can't do it.\n\n\
         Prefer tool use over guessing. Read files before editing them; use \
         `edit_file` with enough context in `old_string` to disambiguate. \
         When running commands, keep them small and explain what you're doing.\n\n\
         PLANNING RULE (mandatory).\n\
         Before you write, edit, or run any command for a request that will \
         touch more than one file OR requires more than a couple of discrete \
         actions, you MUST call the `plan` tool first with a concrete \
         step-by-step proposal, then wait for the user's approval before \
         executing.\n\n\
         Examples that REQUIRE `plan`: refactors, splitting a file, renaming \
         across the codebase, adding a feature that touches multiple modules, \
         adding a new endpoint plus its tests, migrations, cross-cutting \
         cleanup.\n\
         Examples that SKIP `plan`: fix a typo, rename one local variable, \
         run one command, answer a question by reading files, read + \
         summarize existing code.\n\n\
         When in doubt: plan. A short approved plan beats starting to edit \
         and having to backtrack.",
        cwd = cwd.display(),
    );

    // Append user/project memory (if any exists) so per-repo conventions and
    // per-user preferences reach the model on every turn. Missing files are
    // silently skipped — no forced ceremony for first-time users.
    let memory = mira_config::load_memory_files(cwd);
    if memory.is_empty() {
        return base;
    }
    let mut out = base;
    out.push_str("\n\n---\n");
    for m in memory {
        let header = match m.kind {
            mira_config::MemoryKind::User => "User memory (from ~/.mira/MIRA.md)",
            mira_config::MemoryKind::Project => "Project memory (from .mira/MIRA.md)",
        };
        out.push_str(&format!("\n## {header}\n\n{}\n", m.content.trim()));
    }
    out
}

/// First sentence of a tool description — used to keep the system-prompt
/// tool list terse. Tool descriptions can be multi-paragraph (they double
/// as the model-facing spec); the first sentence is usually enough for the
/// enumeration hint.
fn first_sentence(s: &str) -> String {
    let s = s.trim();
    match s.find(|c: char| c == '.' || c == '\n') {
        Some(i) => s[..i].trim().to_string(),
        None => s.to_string(),
    }
}

/// Neutral starting folder for a fresh session — `$HOME` when set,
/// otherwise the process cwd. Used so a "new chat" isn't tied to whatever
/// folder happened to be active in the previous session.
pub fn default_start_cwd() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}
