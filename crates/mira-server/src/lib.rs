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

pub mod acp_host;
pub mod acp_session;
mod agent_secrets;
pub mod agent_spend;
mod agent_threads;
mod agent_updates;
mod agent_worktree;
pub mod approver;
mod aside;
mod browse;
mod browser;
mod browser_recording;
mod chat_import;
pub mod checkpoints;
mod context_api;
mod cwd;
mod delegate;
mod devices;
mod editors;
mod embedded;
mod engine_settings;
pub mod engines_api;
mod engines_reload;
pub mod extensions;
mod file;
mod git;
mod github_connect;
pub mod hooks_api;
pub mod interactive;
mod keep_awake;
pub mod mcp;
mod memory;
mod message_queue;
mod models;
mod oauth;
mod opencode_control;
pub mod pairing;
pub mod plugins;
mod processes;
pub mod protocol;
pub mod provider;
mod pull_requests;
mod remote;
mod review;
mod runtime_admission;
mod runtime_requests;
mod session_activity;
pub mod session_changes;
pub mod session_engine;
mod sessions;
mod settings;
mod skills;
pub mod slot;
mod state;
mod subagents_api;
mod terminal;
mod tests_api;
mod title;
mod transcript_history;
mod undo;
mod unfurl;
mod usage;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::{get, post};
use axum::Router;
use mira_ai::ChatProvider;
use mira_harness::{FileStore, SessionConfig, SessionRecord, SessionStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::Registry;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
use tower_http::compression::predicate::{NotForContentType, Predicate};
use tower_http::compression::{CompressionLayer, CompressionLevel, DefaultPredicate};
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
    // The roster Settings → Subagents edits; every chat's delegation tool
    // reads through this handle, so a saved change applies immediately.
    let agents_live = Arc::new(std::sync::RwLock::new(agents_registry.clone()));
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

    // The engine registry: every backend a turn could run on, derived
    // from the global config (native providers + known agent drivers +
    // `engines:` overrides). Errors here leave an empty registry — the
    // engines API reports "not configured" instead of failing boot.
    let engine_cfg = mira_config::MiraConfig::load_global().unwrap_or_default();
    let engines = Arc::new(crate::state::EnginesHandle::new(
        mira_engine::EngineRegistry::from_config(&engine_cfg),
    ));
    let boot_engines = engines.current();

    // The shared selection: what AgentTool reads for subagent defaults
    // and what `GET /api/engines` reports as active. Seeded from the
    // CLI's resolved model, then pointed at the persisted engine
    // instance (state.yaml's `last_engine`) when that still exists.
    let selection = Arc::new(crate::state::SharedSelection::new(
        Some(cfg.cfg.model.clone()),
        cfg.cfg.small_model.clone(),
    ));
    {
        let mut inst = selection.instance.write().expect("selection lock poisoned");
        if let Some(default) = boot_engines.default_native_instance(&engine_cfg) {
            *inst = Some(default.id.to_string());
        }
        if let Ok(runtime) = mira_config::RuntimeState::load() {
            if let Some(last) = runtime.last_engine {
                if boot_engines.get(&last).is_some() {
                    *inst = Some(last);
                }
            }
        }
    }
    // Fill the provider pool. The CLI-built provider belongs to the
    // config's DEFAULT provider — registering it anywhere else would
    // route another instance's models to the wrong endpoint. Every
    // other native instance builds from config; instances that can't
    // (missing key) are simply absent, and switching to them later
    // surfaces the reason. Finally, activate the selection, falling
    // back to the default when it can't be served.
    let default_instance = boot_engines
        .default_native_instance(&engine_cfg)
        .map(|i| i.id.to_string());
    // Every native instance id, so a background model written
    // `instance:model` routes there (#83).
    swappable.set_known_instances(
        boot_engines
            .instances()
            .filter(|i| i.is_native())
            .map(|i| i.id.to_string()),
    );
    for inst in boot_engines.instances().filter(|i| i.is_native()) {
        let id = inst.id.as_str();
        if Some(id) == default_instance.as_deref() {
            swappable.register(id, cfg.provider.clone());
        } else if let Ok(p) = mira_engine::native::build_native_provider(&engine_cfg, id) {
            swappable.register(id, p);
        }
    }
    let active = selection
        .instance
        .read()
        .expect("selection lock poisoned")
        .clone();
    if let Some(id) = &active {
        if !swappable.activate(id) {
            // The persisted selection can't be served (missing key, …).
            // Fall back to the default instance and say so, rather than
            // pointing the session at a provider that would 401.
            tracing::warn!(instance = %id, "persisted engine not buildable; falling back");
            if let Some(default) = &default_instance {
                swappable.activate(default);
                *selection.instance.write().expect("selection lock poisoned") =
                    Some(default.clone());
            }
        }
    }

    // Build the initial slot. Seeded from the ServerConfig's `resume` (if
    // present) so a `mira serve --resume <id>` picks up where it left off.
    let session_activity = Arc::new(crate::session_activity::ActivityHub::default());
    let deps = crate::slot::SlotDeps {
        session_activity: session_activity.clone(),
        policy: cfg.policy.clone(),
        sandbox: cfg.sandbox.clone(),
        harness_provider: harness_provider.clone(),
        provider_pool: swappable.clone(),
        base_registry: base_registry.clone(),
        agents_registry: agents_registry.clone(),
        agents_live: agents_live.clone(),
        store: cfg.store.clone(),
        memory_runtime: cfg.memory_runtime.clone(),
        scratchpads: scratchpads.clone(),
        selection: selection.clone(),
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
        slot_loads: Arc::new(std::sync::Mutex::new(HashMap::new())),
        session_activity,
        runtime_retries: Arc::new(crate::runtime_requests::RetryQueue::default()),
        slots: Arc::new(RwLock::new(slots)),
        active: Arc::new(RwLock::new(initial_id)),
        policy: cfg.policy.clone(),
        sandbox: cfg.sandbox.clone(),
        provider: swappable,
        harness_provider,
        base_registry,
        agents_registry,
        agents_live,
        compute: cfg.compute.clone(),
        store: cfg.store.clone(),
        extensions: cfg.extensions.clone(),
        skills: cfg.skills.clone(),
        pending_oauth: oauth::new_pending_store(),
        local_port,
        memory_runtime: cfg.memory_runtime.clone(),
        scratchpads,
        selection,
        engines,
        // Forced headless: the browser pane renders the page itself, so a
        // headed Chrome would just pop a second window on the user's desktop
        // behind the app. `MIRA_BROWSER_HEADED=1` opts back into a visible
        // window, which is occasionally useful for stepping through a login
        // or a captcha the agent can't clear.
        // The same process-wide browser the agent's `browser` tool uses
        // (same profile → same instance), so the pane shows what the agent
        // is doing instead of racing it for the profile lock.
        browser: {
            let bcfg = mira_config::MiraConfig::load_global()
                .unwrap_or_default()
                .browser;
            let mut opts = mira_browser::BrowserOptions {
                headless: bcfg
                    .headless
                    .unwrap_or_else(|| std::env::var("MIRA_BROWSER_HEADED").is_err()),
                ..mira_browser::BrowserOptions::default()
            };
            if let Some(exe) = bcfg.executable_path() {
                opts.executable = Some(exe);
            }
            if let Some(dir) = bcfg.profile_dir_path() {
                opts.profile_dir = dir;
            }
            mira_browser::Browser::shared(opts)
        },
    };

    crate::runtime_requests::spawn_dispatcher(state.clone());
    crate::message_queue::recover(state.clone());

    // Filesystem watcher for skills — picks up `npx skills add`
    // installs, hand-authored `SKILL.md` files, and the model's own
    // `write_file` outputs without the user clicking Reload. Broadcasts
    // `SkillsReloaded` on every debounced change so connected clients
    // refetch the roster.
    skills::spawn_skill_watcher(state.clone());

    // Idle external agents are stopped (not forgotten) after a while.
    session_engine::spawn_reaper(state.clone());
    keep_awake::spawn(state.clone());
    engines_reload::spawn(state.clone());

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

    let devices = Arc::new(pairing::Devices::load(pairing::Devices::default_path()));
    // Restarts Cloudflare's connector if remote access was on.
    let remote = remote::Remote::start(remote::Remote::default_dir(), local_port);
    let router = build_router(state, cfg.static_dir.clone(), devices, remote);
    info!(addr = %cfg.bind, "mira serve: listening");

    // Peer addresses feed the pairing guard's "is this local?" check.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .context("axum serve")?;
    Ok(())
}

fn build_router(
    state: AppState,
    static_dir: Option<PathBuf>,
    devices: Arc<pairing::Devices>,
    remote: Arc<remote::Remote>,
) -> Router {
    let mut router = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/ws/terminal", get(terminal::terminal_ws))
        .route("/api/terminals", get(terminal::list))
        .route("/api/terminals/:id", axum::routing::delete(terminal::kill))
        .route("/api/health", get(health))
        .route("/api/version", get(version))
        .route("/api/aside", post(aside::ask))
        .route("/api/sessions/:id/fork", post(sessions::fork_session))
        .route(
            "/api/sessions/:id/relationships",
            get(sessions::relationships),
        )
        .route("/api/sessions/:id/turn-diff", post(git::turn_file_diff))
        .route(
            "/api/processes",
            get(processes::list).post(processes::start),
        )
        .route("/api/processes/:id/output", get(processes::output))
        .route("/api/processes/:id/stop", post(processes::stop))
        .route("/api/processes/:id/restart", post(processes::restart))
        .route(
            "/api/processes/:id",
            axum::routing::delete(processes::forget),
        )
        .route("/api/ports/stop", post(processes::stop_port))
        .route("/api/tests/detect", get(tests_api::detect))
        .route("/api/tests/run", post(tests_api::run))
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
            "/api/acp/external-sessions",
            get(sessions::list_external_agent_sessions),
        )
        .route("/api/acp/agents/:kind/update", post(agent_updates::update))
        .route("/api/acp/turns", get(sessions::list_agent_turns))
        .route("/api/acp/revert", post(sessions::revert_agent_turn))
        .route(
            "/api/sessions/:id/history",
            get(sessions::get_session_history),
        )
        .route(
            "/api/sessions/:id/images/:key",
            get(sessions::get_session_image),
        )
        .route(
            "/api/sessions/:id/preview",
            get(sessions::get_session_preview),
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
        .route(
            "/api/sessions/:id/flags",
            axum::routing::put(sessions::set_session_flags),
        )
        .route("/api/cwd", get(cwd::get_cwd).put(cwd::put_cwd))
        .route("/api/editors", get(editors::list_editors))
        .route("/api/editors/icon/:id", get(editors::editor_icon))
        .route(
            "/api/editors/open",
            axum::routing::post(editors::open_in_editor),
        )
        .route("/api/browse", get(browse::browse))
        .route("/api/browser/action", axum::routing::post(browser::action))
        .route("/api/browser/live", get(browser::live))
        .route("/api/browser/input", axum::routing::post(browser::input))
        .route(
            "/mcp",
            axum::routing::post(browser::mcp).get(browser::mcp_get),
        )
        .route("/api/browser/embeddable", get(browser::embeddable))
        .route("/api/file", get(file::read_file))
        .route("/api/models", get(models::list_models))
        .route("/api/engines", get(engines_api::list_engines))
        .route(
            "/api/subagents",
            get(subagents_api::list).post(subagents_api::create),
        )
        .route(
            "/api/subagents/:name",
            axum::routing::put(subagents_api::update).delete(subagents_api::remove),
        )
        .route(
            "/api/engines/:instance/models",
            get(engines_api::instance_models),
        )
        .route(
            "/api/engines/:instance/settings",
            get(engine_settings::get_settings).put(engine_settings::put_settings),
        )
        .route("/api/import/scan", get(chat_import::scan))
        .route("/api/import", axum::routing::post(chat_import::import))
        .route("/api/git/status", get(git::get_status))
        .route("/api/git/session-diff", get(git::session_diff))
        .route("/api/usage", get(usage::get_usage))
        .route("/api/pricing", get(usage::get_pricing))
        .route("/api/usage/external", get(usage::get_external_usage))
        .route("/api/git/session-changes", get(git::session_changes))
        .route(
            "/api/git/revert-file",
            axum::routing::post(git::revert_file),
        )
        .route("/api/context", get(context_api::breakdown))
        .route(
            "/api/context/drop",
            axum::routing::post(context_api::drop_result),
        )
        .route(
            "/api/checkpoints/preview",
            axum::routing::post(git::checkpoint_preview),
        )
        .route(
            "/api/checkpoints/restore",
            axum::routing::post(git::checkpoint_restore),
        )
        .route(
            "/api/checkpoints/undo",
            axum::routing::post(git::checkpoint_undo),
        )
        .route("/api/git/push", axum::routing::post(git::push))
        .route("/api/git/branch-pr", get(git::branch_pr))
        .route("/api/git/commit/:sha", get(git::commit_card))
        .route("/api/git/branch", get(git::branch_info))
        .route("/api/git/apply", axum::routing::post(git::apply_patch))
        .route("/api/symbol", get(git::find_symbol))
        .route("/api/unfurl", get(unfurl::unfurl))
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
            "/api/prs/:owner/:repo/:number/card",
            get(pull_requests::mention_card),
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

    // Device pairing: its own routes, and a guard in front of everything
    // that only lets in this machine and paired devices (see pairing.rs).
    let pairing_routes = Router::new()
        .route("/api/pairing/me", get(pairing::me))
        .route("/api/pairing/devices", get(pairing::list))
        .route("/api/pairing/code", post(pairing::open_pairing))
        .route("/api/pairing/pair", post(pairing::pair))
        .route(
            "/api/pairing/devices/:id",
            axum::routing::delete(pairing::revoke),
        )
        .with_state(devices.clone());
    // Remote access: reach this computer from anywhere (remote.rs).
    let remote_routes = Router::new()
        .route("/api/remote", get(remote::status))
        .route("/api/remote/enable", post(remote::enable))
        .route("/api/remote/disable", post(remote::disable))
        .with_state(remote);

    router
        .with_state(state)
        .merge(pairing_routes)
        .merge(remote_routes)
        .layer(axum::middleware::from_fn_with_state(
            devices,
            pairing::guard,
        ))
        .layer(compression_layer())
        .layer(TraceLayer::new_for_http())
}

/// gzip/brotli for the web UI and API responses. Matters most when the UI is
/// reached over a network (tunnel, LAN, remote host): the first-load bundle
/// is ~1.3 MB raw and ~380 kB compressed. Session history JSON shrinks the
/// same way.
///
/// Level 4 rather than the default: brotli's default (11) spends hundreds of
/// milliseconds on the larger chunks, which would trade transfer time for an
/// equally long encode. Hashed assets are cached `immutable`, so each is
/// compressed once per client.
///
/// Streaming bodies are left alone: the default predicate already skips SSE
/// (and images, gRPC, tiny bodies), and the NDJSON streams (aside, tests)
/// must not be buffered by the encoder.
fn compression_layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .quality(CompressionLevel::Precise(4))
        .compress_when(
            DefaultPredicate::new().and(NotForContentType::const_new("application/x-ndjson")),
        )
}

async fn embedded_fallback(uri: axum::http::Uri) -> axum::response::Response {
    embedded::serve(uri.path()).await
}

async fn health() -> &'static str {
    "ok"
}

/// This server's version, for the web UI's "up to date" check.
async fn version() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
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
        .map(|s| {
            // Interactive tools carry enforcement clauses the model must see
            // verbatim — a first-sentence truncation drops exactly the
            // "never claim you asked without calling" lines that stop
            // hallucinations like "I've sent you three questions" with no
            // tool call. Everything else stays one line to save context.
            if s.name == "ask_user" || s.name == "plan" {
                format!("- {}: {}", s.name, s.description)
            } else {
                format!("- {}: {}", s.name, first_sentence(&s.description))
            }
        })
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
          TOOL-CALL GROUNDING (mandatory).\n\
          The UI renders a question card or plan panel ONLY from a real \
          `ask_user` / `plan` tool call. Text that looks like questions or a \
          plan shows nothing to the user and returns no answer. Never write \
          \"I've asked / sent / proposed\" unless the tool call already \
          happened in this turn. When the user explicitly names a tool \
          (\"ask me\", \"propose a plan\", \"for approval\"), call that tool \
          immediately — no preamble, no summary first.\n\n\
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

    // Orientation in a large repository (see `mira_tools::repo_map`). Built
    // once, when the session starts, so it stays part of the cached prefix.
    match mira_tools::repo_map::prompt_section_for(cwd, REPO_MAP_WAIT) {
        Some(map) => format!("{base}\n\n{map}"),
        None => base,
    }
}

/// How long a new session waits for the repository map. A first build of
/// a big repository that takes longer finishes in the background, for the
/// next session.
const REPO_MAP_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

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

#[cfg(test)]
mod compression_tests {
    use super::compression_layer;
    use axum::body::Body;
    use axum::http::{header, Request};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn app() -> Router {
        let big = "x".repeat(4096);
        let json = big.clone();
        Router::new()
            .route(
                "/json",
                get(move || async move { ([(header::CONTENT_TYPE, "application/json")], json) }),
            )
            .route(
                "/ndjson",
                get(move || async move { ([(header::CONTENT_TYPE, "application/x-ndjson")], big) }),
            )
            .layer(compression_layer())
    }

    async fn encoding(path: &str) -> Option<String> {
        let res = app()
            .oneshot(
                Request::get(path)
                    .header(header::ACCEPT_ENCODING, "br, gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        res.headers()
            .get(header::CONTENT_ENCODING)
            .map(|v| v.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn compresses_json() {
        assert_eq!(encoding("/json").await.as_deref(), Some("br"));
    }

    /// NDJSON endpoints stream; an encoder would buffer them.
    #[tokio::test]
    async fn leaves_ndjson_streams_alone() {
        assert_eq!(encoding("/ndjson").await, None);
    }
}
