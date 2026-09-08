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
pub mod mcp;
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
    /// Cross-session memory runtime knobs — controls the post-round
    /// auto-extractor. Defaults to on with no extractor-model override.
    pub memory_runtime: mira_config::MemoryRuntimeConfig,
    /// Per-MCP-server connect results captured by the caller before it
    /// registered the tools. Frozen for the process lifetime — the
    /// Plugins UI diffs yaml against this to decide "restart required."
    pub mcp_boot: Vec<crate::mcp::McpBootStatus>,
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

    // Swappable provider up front — the AgentTool below needs to hold it
    // so subagents follow any provider hot-swap the user makes in Settings.
    let swappable = SwappableProvider::new(cfg.provider.clone());
    let harness_provider: Arc<dyn ChatProvider> = Arc::new(swappable.clone());

    // Copy the caller-provided registry and layer on server-only interactive
    // tools. Registry is `Clone`, so this is cheap; the resulting Arc<Registry>
    // is what the session actually consults.
    //
    // Order matters here for the `agent` tool: it needs a snapshot of the
    // registry BEFORE itself so subagents inherit peer tools without a
    // reference to `agent` itself (nested `agent` calls are re-added at
    // construction time with the correct depth cap).
    let mut registry_owned: Registry = (*cfg.registry).clone();
    registry_owned.register(interactive::PlanTool::new(prompt_channel.clone()));
    let base_registry = Arc::new(registry_owned.clone());

    // Named subagent types: builtins + `~/.mira/AGENTS.md` + `<cwd>/.mira/AGENTS.md`.
    // Loaded once at boot so the tool spec that goes to the model reflects
    // the roster on the machine that started the server.
    let agents_registry = Arc::new(mira_agents::load(&cfg.cwd));
    tracing::info!(
        count = agents_registry.names().len(),
        types = ?agents_registry.names(),
        "agent types loaded"
    );

    // Build the parent's approver up front so the AgentTool can hold a
    // reference to it (write-capable subagents route Ask decisions back
    // through the same modal the user sees). cwd is `Arc<RwLock<...>>`
    // shared with the WsApprover so folder swaps flow through to diff
    // previews for both parent and child.
    let cwd = Arc::new(RwLock::new(cfg.cwd.clone()));
    let approver: Arc<dyn Approver> = Arc::new(WsApprover::new(
        events_tx.clone(),
        pending.clone(),
        cwd.clone(),
    ));

    let mut agent_tool = interactive::AgentTool::new(
        harness_provider.clone(),
        base_registry,
        cfg.cfg.model.clone(),
    )
    .with_agents(agents_registry.clone())
    // Wire the shared events broadcast so subagent child events fan
    // out to the connected WSes and light up the SubagentPanel live.
    .with_events_tx(events_tx.clone())
    // Route write-capable subagent approvals to the parent's UI so
    // `bash rm -rf ...` inside a coder subagent pops the same modal the
    // parent would. Read-only types stay on the auto-approver path.
    .with_parent_approver(approver.clone())
    // Share the parent's live Policy Arc — write-capable children see
    // the same mode + rules the parent does, so mode swaps (Auto →
    // Yolo) and always-allow rules the user accumulates propagate to
    // the child immediately.
    .with_parent_policy(cfg.policy.clone());
    // Persist child sessions when the parent's store is available. The
    // panel uses `/api/sessions/:id/history` to rebuild a child's
    // transcript on browser reload.
    if let Some(store) = &cfg.store {
        agent_tool = agent_tool.with_store(store.clone());
    }
    registry_owned.register(agent_tool);
    let registry = Arc::new(registry_owned);
    cfg.registry = registry.clone();

    // Build the memory + episodic stores BEFORE constructing the initial
    // Session so its `tool_ctx` gets them wired from turn zero. Otherwise
    // the very first session's memory tools would fail with "memory store
    // not wired" until the user triggered a folder-swap or session-swap
    // (which is when make_tool_ctx would first run).
    let memory_store: Arc<dyn mira_memory::MemoryStore> =
        Arc::new(mira_memory::FileMemoryStore::new(
            mira_config::user_memory_path(),
            mira_config::project_memory_path(&cfg.cwd),
        ));
    let episodic_store: Arc<dyn mira_memory::EpisodicStore> = Arc::new(
        mira_memory::FileEpisodicStore::new(mira_memory::project_episodic_path(&cfg.cwd)),
    );
    let initial_ctx = ToolContext::new(cfg.cwd.clone(), cfg.sandbox.clone())
        .with_memory(memory_store.clone())
        .with_episodic(episodic_store.clone());
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
    // Live memory: re-read user + project MIRA.md on every round so edits
    // from `/remember`, the memory tools, or the user's own text editor
    // reach the model without a session restart. The system prompt above
    // no longer appends memory itself — the snapshot is the single source.
    // Skip the snapshot entirely when `memory.inject_context` is false, so
    // the second system message doesn't get emitted at all — useful when
    // bisecting whether the memory block is confusing the model.
    if cfg.memory_runtime.inject_context() {
        session = session.with_memory_snapshot(make_memory_snapshot_with(
            &cfg.cwd,
            episodic_store.clone(),
        ));
    }
    // Auto-extractor: post-round background pass that appends durable
    // facts to episodic. Off if the user disabled it via `mira.yaml`.
    if cfg.memory_runtime.auto_extract_enabled() {
        session = session.with_auto_extract(mira_harness::AutoExtractConfig::enabled(
            cfg.memory_runtime.extractor_model().map(str::to_owned),
        ));
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
        memory: Arc::new(RwLock::new(memory_store)),
        episodic: Arc::new(RwLock::new(episodic_store)),
        mcp_boot: Arc::new(cfg.mcp_boot),
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
            "/api/sessions/:id/history",
            get(sessions::get_session_history),
        )
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
        .route(
            "/api/sessions/:id/title",
            axum::routing::patch(sessions::set_session_title),
        )
        .route(
            "/api/sessions/:id/title/regenerate",
            axum::routing::post(sessions::regenerate_session_title),
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
        .route("/api/undo", axum::routing::post(undo::apply_undo))
        .route("/api/mcp", get(mcp::get_mcp).put(mcp::put_mcp));

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

/// Build the standard live-memory snapshot for a session bound to `cwd`.
/// Every `Session::new` / `Session::resume_from` site in the server should
/// pass the result through `session.with_memory_snapshot(...)` so mid-
/// session memory edits land on the very next round.
///
/// The two-arg overload (`make_memory_snapshot`) is the standalone form —
/// used by the CLI, which doesn't share an episodic store across handlers.
/// Server-side callers should prefer [`make_memory_snapshot_with`] so the
/// snapshot renders the same episodic entries `memory_remember` writes to.
pub fn make_memory_snapshot(cwd: &std::path::Path) -> Arc<dyn mira_memory::MemorySnapshot> {
    let epi: Arc<dyn mira_memory::EpisodicStore> = Arc::new(
        mira_memory::FileEpisodicStore::new(mira_memory::project_episodic_path(cwd)),
    );
    make_memory_snapshot_with(cwd, epi)
}

/// Server variant that shares an existing episodic-store handle with the
/// snapshot — so `memory_remember` (writer) and the snapshot renderer
/// (reader) hit the same file through the same mutex.
pub fn make_memory_snapshot_with(
    cwd: &std::path::Path,
    episodic: Arc<dyn mira_memory::EpisodicStore>,
) -> Arc<dyn mira_memory::MemorySnapshot> {
    Arc::new(
        mira_memory::FileMemorySnapshot::new(
            mira_config::user_memory_path(),
            mira_config::project_memory_path(cwd),
        )
        .with_episodic(episodic),
    )
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
         and having to backtrack.\n\n\
         DELEGATION.\n\
         When a subtask would take many tool calls to investigate — searching \
         a large codebase for every use of X, reading half a dozen files to \
         answer one question, running an exploratory probe — prefer the \
         `agent` tool. Its child starts COLD, so write a self-contained \
         prompt with the file paths, keywords, and shape of answer you want. \
         The child's summary comes back as one message and its 20 tool calls \
         never touch your context. Do NOT delegate the actual writing you \
         were asked to do — subagents are for research and bounded probes, \
         not the deliverable.",
        cwd = cwd.display(),
    );

    // Memory (user + project MIRA.md) used to be baked into the prompt here,
    // but that made every session a snapshot — mid-session edits, agent tool
    // writes, and `/remember` calls only took effect on the *next* session.
    // The harness now injects a live memory block on every round via a
    // `MemorySnapshot`; this function returns the stable, cacheable prefix.
    base
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
