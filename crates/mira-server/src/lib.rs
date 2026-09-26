//! HTTP + WebSocket server that fronts Mira `Session`s for browser UIs.
//!
//! The CLI builds the harness stack (provider, registry, policy, initial
//! session config) and hands it to [`run`]. The server does the rest:
//! WebSocket at `/ws` for realtime chat, plus static assets under `/`
//! served from disk (dev) or embedded (release).
//!
//! ## Multi-session wire model
//!
//! The server holds a MAP of live `SessionSlot`s. Each slot owns its own:
//! - `events_tx` broadcast bus (WS forwarders subscribe per slot)
//! - `pending` approval map (per-slot oneshots)
//! - `cwd`, `memory`, `episodic` (per-session project scope)
//! - `approver` (a WsApprover wired to the slot's channels + background
//!   mode)
//! - `registry` (base + PlanTool/AskUserTool/AgentTool wired to the
//!   slot's prompt channel)
//!
//! A WS connection picks which session it's watching by sending
//! `Attach { session_id }`. Handlers without a session_id in the URL
//! (settings PUT, cwd PUT, memory append, undo, …) route to the
//! `state.active` pointer, updated on each attach.
//!
//! Approval prompts arrive as
//! [`protocol::ServerMsg::ApprovalRequest`]; the client answers with
//! [`protocol::ClientMsg::Approve`], which routes back to the awaiting
//! oneshot via [`approver::resolve`].

mod agent_worktree;
pub mod approver;
mod browse;
mod cwd;
mod embedded;
pub mod extensions;
mod file;
mod git;
mod github_connect;
pub mod hooks_api;
pub mod interactive;
pub mod mcp;
mod memory;
mod models;
mod oauth;
pub mod plugins;
pub mod protocol;
pub mod provider;
mod pull_requests;
mod review;
mod sessions;
mod settings;
mod skills;
pub mod slot;
mod state;
mod terminal;
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
use mira_harness::{FileStore, SessionConfig, SessionRecord, SessionStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::Registry;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
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

/// Build args for [`run`]. Everything the server needs except the initial
/// slot, which is constructed inside `run` so its channels are wired
/// consistently with the WsApprover / PromptChannel / AgentTool.
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
    /// MCP servers, plugins and custom commands. The caller has already
    /// added its MCP tool source to `registry`.
    pub extensions: crate::extensions::Extensions,
    /// Loaded skill roster (bundled + user + project tiers merged).
    pub skills: mira_tools::builtin::skill::SkillHandle,
    /// Named remote environments (`compute:` in mira.yaml).
    pub compute: mira_config::ComputeConfig,
    /// Environment the first session starts in (`--sandbox` /
    /// `compute.default`); `None` = this machine.
    pub initial_environment: Option<String>,
}

/// Start the server. Blocks until the process is signaled to exit.
pub async fn run(cfg: ServerConfig) -> Result<()> {
    // Swappable provider up front — every slot's AgentTool holds a
    // reference so a settings hot-swap flows through to subagents.
    let swappable = SwappableProvider::new(cfg.provider.clone());
    let harness_provider: Arc<dyn ChatProvider> = Arc::new(swappable.clone());

    // Named subagent types — loaded once at boot.
    let agents_registry = Arc::new(mira_agents::load_with_plugins(
        &cfg.cwd,
        &cfg.extensions.plugin_agent_files(),
    ));
    tracing::info!(
        count = agents_registry.names().len(),
        types = ?agents_registry.names(),
        "agent types loaded"
    );

    // The base registry is the caller's registry — slot factories layer
    // PlanTool/AskUserTool/AgentTool on top per session.
    let base_registry = cfg.registry.clone();

    // Cross-session scratchpad — shared across every slot's AgentTool.
    // Entries are keyed by parent session id inside, so a slot's peers see
    // each other's notes but two separate sessions stay isolated.
    let scratchpads = Arc::new(Mutex::new(HashMap::new()));

    // Bind the listener up-front so the OAuth callback URL can embed the
    // real port before AppState is finalised.
    terminal::configure(cfg.bind);
    let listener = TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    let local_port = listener
        .local_addr()
        .map(|a| a.port())
        .unwrap_or(cfg.bind.port());

    // `prompt` hooks ask the small model, on the swappable provider so a
    // settings change reaches them too.
    cfg.extensions
        .set_hook_model(harness_provider.clone(), cfg.cfg.background_model(None));

    // Build the initial slot. Seeded from the ServerConfig's `resume` (if
    // present) so a `mira serve --resume <id>` picks up where it left off.
    let deps = crate::slot::SlotDeps {
        policy: cfg.policy.clone(),
        sandbox: cfg.sandbox.clone(),
        harness_provider: harness_provider.clone(),
        base_registry: base_registry.clone(),
        agents_registry: agents_registry.clone(),
        store: cfg.store.clone(),
        memory_runtime: cfg.memory_runtime.clone(),
        scratchpads: scratchpads.clone(),
        default_model_for_agents: cfg.cfg.model.clone(),
        small_model_for_agents: cfg.cfg.small_model.clone(),
        compute: cfg.compute.clone(),
        hooks: Some(cfg.extensions.hook_runner()),
    };
    let initial_slot =
        crate::slot::build_slot(cfg.cwd.clone(), cfg.cfg.clone(), cfg.resume, &deps).await;
    let initial_id = initial_slot.id.clone();
    if let Some(target) = cfg.initial_environment.clone() {
        crate::slot::spawn_environment_switch(initial_slot.clone(), target);
    }

    let mut slots = HashMap::new();
    slots.insert(initial_id.clone(), initial_slot);

    let state = AppState {
        slots: Arc::new(RwLock::new(slots)),
        active: Arc::new(RwLock::new(initial_id)),
        policy: cfg.policy.clone(),
        sandbox: cfg.sandbox.clone(),
        provider: swappable,
        harness_provider,
        base_registry,
        agents_registry,
        compute: cfg.compute.clone(),
        store: cfg.store.clone(),
        extensions: cfg.extensions.clone(),
        skills: cfg.skills.clone(),
        pending_oauth: oauth::new_pending_store(),
        local_port,
        memory_runtime: cfg.memory_runtime.clone(),
        scratchpads,
        default_model_for_agents: cfg.cfg.model.clone(),
        small_model_for_agents: cfg.cfg.small_model.clone(),
    };

    // Filesystem watcher for skills — picks up `npx skills add`
    // installs, hand-authored `SKILL.md` files, and the model's own
    // `write_file` outputs without the user clicking Reload. Broadcasts
    // `SkillsReloaded` on every debounced change so connected clients
    // refetch the roster.
    skills::spawn_skill_watcher(state.clone());

    // MCP status changes (a server connecting, dropping, asking for
    // sign-in) → `ExtensionsChanged` to every tab, debounced so a burst of
    // connects at startup is one refresh.
    {
        let state = state.clone();
        let mut rx = state.extensions.mcp().subscribe();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match rx.recv().await {
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                while rx.try_recv().is_ok() {}
                state
                    .broadcast_all(crate::protocol::ServerMsg::ExtensionsChanged)
                    .await;
            }
        });
    }

    // OAuth token refresh: rotate ChatGPT / Codex short-lived API keys
    // before they expire so a signed-in session survives long chats
    // without a re-signin.
    oauth::refresh::boot_rehydrate(&state).await;
    oauth::refresh::spawn(state.clone());

    let router = build_router(state, cfg.static_dir.clone());
    info!(addr = %cfg.bind, "mira serve: listening");

    axum::serve(listener, router).await.context("axum serve")?;
    Ok(())
}

fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    let mut router = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/ws/terminal", get(terminal::terminal_ws))
        .route("/api/terminals", get(terminal::list))
        .route("/api/terminals/:id", axum::routing::delete(terminal::kill))
        .route("/api/health", get(health))
        .route(
            "/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        .route(
            "/api/auth/openrouter/start",
            axum::routing::post(oauth::openrouter::start),
        )
        .route(
            "/api/auth/openrouter/callback",
            get(oauth::openrouter::callback),
        )
        .route(
            "/api/auth/openai/start",
            axum::routing::post(oauth::openai::start),
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
        .route(
            "/api/sessions/:id/background",
            axum::routing::put(sessions::set_background_mode_http),
        )
        .route("/api/cwd", get(cwd::get_cwd).put(cwd::put_cwd))
        .route("/api/browse", get(browse::browse))
        .route("/api/file", get(file::read_file))
        .route("/api/models", get(models::list_models))
        .route("/api/git/status", get(git::get_status))
        .route("/api/git/session-diff", get(git::session_diff))
        .route("/api/git/session-changes", get(git::session_changes))
        .route(
            "/api/git/revert-file",
            axum::routing::post(git::revert_file),
        )
        .route("/api/git/push", axum::routing::post(git::push))
        .route("/api/git/branch-pr", get(git::branch_pr))
        .route("/api/git/commit", axum::routing::post(git::commit))
        .route(
            "/api/git/worktree",
            axum::routing::post(git::create_worktree),
        )
        .route("/api/skills", get(skills::list_skills))
        .route(
            "/api/skills/reload",
            axum::routing::post(skills::reload_skills),
        )
        .route("/api/skills/:name", get(skills::get_skill))
        .route("/api/memory", get(memory::get_memory))
        .route(
            "/api/memory/append",
            axum::routing::post(memory::append_memory),
        )
        .route("/api/review", axum::routing::post(review::start_review))
        .route("/api/undo", axum::routing::post(undo::apply_undo))
        .route(
            "/api/hooks",
            get(hooks_api::get_hooks).put(hooks_api::put_hooks),
        )
        .route("/api/mcp", get(mcp::get_mcp))
        .route("/api/mcp/servers", axum::routing::post(mcp::save_server))
        .route("/api/mcp/variables", axum::routing::post(mcp::set_variable))
        .route(
            "/api/mcp/tools/enabled",
            axum::routing::post(mcp::set_tool_enabled),
        )
        .route(
            "/api/mcp/tool-loading",
            axum::routing::post(mcp::set_tool_loading),
        )
        .route(
            "/api/mcp/servers/:name",
            axum::routing::delete(mcp::delete_server),
        )
        .route(
            "/api/mcp/servers/:name/reconnect",
            axum::routing::post(mcp::reconnect),
        )
        .route(
            "/api/mcp/servers/:name/enabled",
            axum::routing::post(mcp::set_enabled),
        )
        .route(
            "/api/mcp/servers/:name/approval",
            axum::routing::post(mcp::set_approval),
        )
        .route(
            "/api/mcp/servers/:name/sign-in",
            axum::routing::post(mcp::sign_in),
        )
        .route(
            "/api/mcp/servers/:name/sign-out",
            axum::routing::post(mcp::sign_out),
        )
        .route("/api/mcp/oauth/callback", get(mcp::oauth_callback))
        .route("/api/commands", get(mcp::list_commands))
        .route("/api/plugins", get(plugins::get_plugins))
        .route("/api/plugins/detail/:id", get(plugins::get_detail))
        .route(
            "/api/plugins/install",
            axum::routing::post(plugins::install),
        )
        .route(
            "/api/plugins/:id/uninstall",
            axum::routing::post(plugins::uninstall),
        )
        .route(
            "/api/plugins/:id/enabled",
            axum::routing::post(plugins::set_enabled),
        )
        .route(
            "/api/plugins/marketplaces",
            axum::routing::post(plugins::add_marketplace),
        )
        .route(
            "/api/plugins/marketplaces/:name/update",
            axum::routing::post(plugins::update_marketplace),
        )
        .route(
            "/api/plugins/marketplaces/:name",
            axum::routing::delete(plugins::remove_marketplace),
        )
        .route(
            "/api/github/connect",
            get(github_connect::get_connect).post(github_connect::post_connect),
        )
        .route("/api/prs", get(pull_requests::list_pull_requests))
        .route(
            "/api/prs/:owner/:repo/:number",
            get(pull_requests::get_pull_request),
        )
        .route(
            "/api/prs/:owner/:repo/:number/files",
            get(pull_requests::get_pull_request_files),
        )
        .route(
            "/api/prs/:owner/:repo/:number/comments",
            axum::routing::post(pull_requests::post_pull_request_comment),
        )
        .route(
            "/api/prs/:owner/:repo/:number/reviews",
            axum::routing::post(pull_requests::post_pull_request_review),
        )
        .route(
            "/api/prs/:owner/:repo/:number/merge",
            axum::routing::put(pull_requests::merge_pull_request),
        );

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
    axum::response::Html(include_str!("../assets/placeholder.html"))
}

/// Convenience for the CLI: open a default session store if the user did not
/// pass `--no-persist`.
pub fn default_store() -> Result<Arc<dyn SessionStore>> {
    let store = FileStore::open_default().context("open session store")?;
    Ok(Arc::new(store))
}

/// Build the standard live-memory snapshot for a session bound to `cwd`.
pub fn make_memory_snapshot(cwd: &std::path::Path) -> Arc<dyn mira_memory::MemorySnapshot> {
    let epi: Arc<dyn mira_memory::EpisodicStore> = Arc::new(mira_memory::FileEpisodicStore::new(
        mira_memory::project_episodic_path(cwd),
    ));
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

/// Translate the `mira.yaml`-shaped `MemoryRuntimeConfig` into the
/// harness's `MemoryRetrievalConfig`.
pub fn memory_retrieval_from(
    cfg: &mira_config::MemoryRuntimeConfig,
) -> mira_harness::MemoryRetrievalConfig {
    let mut out = mira_harness::MemoryRetrievalConfig {
        enabled: cfg.retrieval_enabled(),
        ..Default::default()
    };
    if let Some(budget) = cfg.retrieval_token_budget() {
        out.token_budget = budget as usize;
    }
    out
}

/// System prompt for a session bound to `cwd`. Passed into
/// `Session::new`; kept here (rather than in the CLI) so `new_session`,
/// `put_cwd`, and every slot factory can recompute it from the current
/// folder.
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
         CLARIFY FIRST WHEN AMBIGUOUS.\n\
         Before you propose a plan for a request that could plausibly be \
         shaped several different ways — target user vs. admin, permissions \
         model, scope boundary, storage backend, framework choice, migration \
         vs. rewrite — call the `ask_user` tool with 1-4 structured multiple- \
         choice questions. Mark exactly one option `recommended: true` when \
         you have a considered preference. The UI automatically adds a \
         \"Tell mira what to do differently\" free-text path to every \
         question, so you don't need to include it as an option. Use the \
         user's answers to shape the subsequent `plan` call. Do NOT chain \
         `ask_user` calls — ask everything you need in one round. Skip \
         `ask_user` when the request is unambiguous, when a quick file read \
         would resolve the ambiguity, or when you're mid-execution and \
         picking would derail the flow.\n\n\
         DELEGATION.\n\
         When a subtask would take many tool calls to investigate — searching \
         a large codebase for every use of X, reading half a dozen files to \
         answer one question, running an exploratory probe — prefer the \
         `agent` tool. Its child starts COLD, so write a self-contained \
         prompt with the file paths, keywords, and shape of answer you want. \
         The child's summary comes back as one message and its 20 tool calls \
         never touch your context. Do NOT delegate the actual writing you \
         were asked to do — subagents are for research and bounded probes, \
         not the deliverable.\n\n\
         RESPONSE STYLE.\n\
         Your text is rendered by a terminal markdown renderer (bold, \
         italic, inline `code`, fenced code blocks, bullets). Use it \
         sparingly to guide the eye:\n\
         - Wrap every identifier — file paths, function/type/variable \
           names, CLI flags, env vars, config keys — in `inline code`. \
           `src/foo.rs`, `httpOnly`, `--no-verify`, `$OPENAI_API_KEY`.\n\
         - Use *italic* for a concept you're calling out mid-sentence \
           (\"the gap is that we're setting the session cookie without \
           *httpOnly*\"). Not for emphasis-as-shouting.\n\
         - Use **bold** only for a section header or a term the user \
           will scan for. Never bold entire sentences.\n\
         - Fenced code blocks with a language tag (```rust, ```ts, \
           ```bash) for multi-line snippets. Single-line commands can \
           stay inline in backticks.\n\
         - Prose stays plain. No decorative rules like `---` or emoji \
           chrome; the renderer draws its own structure.",
        cwd = cwd.display(),
    );

    base
}

fn first_sentence(s: &str) -> String {
    let s = s.trim();
    match s.find(['.', '\n']) {
        Some(i) => s[..i].trim().to_string(),
        None => s.to_string(),
    }
}

/// Neutral starting folder for a fresh session — `$HOME` when set,
/// otherwise the process cwd.
pub fn default_start_cwd() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

// Unused imports elsewhere reference these; keep them re-exported for the
// small-handler crates that used to reach into `state.session` directly.
#[allow(unused_imports)]
pub(crate) use tokio::sync::broadcast;
