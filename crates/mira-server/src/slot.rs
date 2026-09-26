//! Per-session runtime state.
//!
//! Before multi-session support, every server had exactly one live
//! `Session`, one broadcast bus, and one approval-map. Switching sessions
//! meant *replacing* that singleton — any in-flight turn was orphaned,
//! Session A's approval prompt could pop on the tab watching Session B,
//! and closing the browser cut off progress.
//!
//! `SessionSlot` fixes that by giving every session its own:
//! - `events_tx`  — a broadcast::Sender the harness drains into; WS
//!   clients attached to this session subscribe here so frames never
//!   leak into another session's transcript.
//! - `pending`    — approval oneshots keyed by tool call id. Scoped so
//!   two sessions can independently be waiting on approvals without one
//!   resolving the other.
//! - `prompt_pending` — same idea for interactive-tool prompts (plan /
//!   ask_user / subagent review).
//! - `cwd` / `memory` / `episodic` — a session in project A doesn't share
//!   project-scoped memory with a session in project B.
//! - `approver`   — a `WsApprover` bound to this slot's channels and
//!   background-mode setting.
//! - `registry`   — a slot-specific tool set where PlanTool / AskUserTool /
//!   AgentTool have been rebuilt against this slot's prompt channel.
//! - `turn`       — the JoinHandle of the currently-running turn task, if
//!   any, so Interrupt can cancel and delete_session can tear it down.
//! - `attached`   — a live count of WS forwarders subscribed to this slot.
//!   Drives background-mode auto-decisions (see [`BackgroundMode`]).
//! - `background_mode` — how the approver should answer `Ask` decisions
//!   when nobody's watching. Per-session because the user might trust
//!   Session A to auto-approve while wanting Session B to park.
//!
//! Slots are built by [`build_slot`], which owns the whole "wire up a
//! Session with its per-slot channels" recipe. All server bootstraps —
//! initial `run()`, `POST /api/sessions/new`, `POST /api/sessions/:id/load`,
//! `PUT /api/cwd` — go through it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mira_agents::AgentRegistry;
use mira_ai::ChatProvider;
use mira_config::MemoryRuntimeConfig;
use mira_core::SessionId;
use mira_harness::{Approver, Session, SessionConfig, SessionRecord, SessionStore};
use mira_memory::{EpisodicStore, FileEpisodicStore, FileMemoryStore, MemoryStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::context::ToolProgressSink;
use mira_tools::{BackgroundProcessStore, Registry, ToolContext};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::approver::{PendingMap, WsApprover};
use crate::interactive::{
    AgentTool, AskUserTool, PendingPromptMap, PlanTool, PromptChannel, ScratchpadEntry,
};
use crate::protocol::ServerMsg;

/// Session-lifetime `ToolProgressSink` that writes directly to the slot's
/// broadcast channel. Unlike the harness `TurnProgress`, this is never
/// cleared between turns, so background process drain tasks can keep emitting
/// `ToolProgress` frames even after `invoke` has returned.
struct SessionProgress {
    tx: broadcast::Sender<ServerMsg>,
}

impl ToolProgressSink for SessionProgress {
    fn emit(&self, call_id: &str, line: &str) {
        let _ = self.tx.send(ServerMsg::ToolProgress {
            call_id: call_id.to_owned(),
            line: line.to_owned(),
        });
    }
}

/// How the slot's approver answers `Ask` decisions when no WS client is
/// currently attached. Serialised over the wire so the frontend can show
/// a per-session toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum BackgroundMode {
    /// Auto-deny `Ask` when no client is attached. Safe default — the tool
    /// call fails, the model sees the error, and the session doesn't burn
    /// tokens on approvals no human will see.
    #[default]
    Deny,
    /// Auto-approve `Ask` when no client is attached. Use only when the
    /// session's policy is already narrow (read-only exploration, a
    /// subagent-heavy research task) so silent approval isn't dangerous.
    AutoApprove,
    /// Park indefinitely — pre-multi-session behavior. The tool call
    /// blocks until the user (re-)attaches and answers, or the
    /// approver's 10-minute safety timeout trips.
    Park,
}

/// One session's isolated runtime. Clone the `Arc<SessionSlot>` to hand a
/// handle to a task or handler; individual fields have their own
/// `Arc<Mutex<..>>` / `Arc<RwLock<..>>` guards where mutation is needed.
pub struct SessionSlot {
    pub id: SessionId,
    pub session: Arc<RwLock<Session>>,
    pub events_tx: broadcast::Sender<ServerMsg>,
    pub pending: PendingMap,
    pub prompt_pending: PendingPromptMap,
    pub cwd: Arc<RwLock<PathBuf>>,
    pub memory: Arc<RwLock<Arc<dyn MemoryStore>>>,
    pub episodic: Arc<RwLock<Arc<dyn EpisodicStore>>>,
    pub registry: Arc<Registry>,
    pub approver: Arc<dyn Approver>,
    /// JoinHandle of the currently-running turn task, if any. Interrupt
    /// aborts through this; delete_session takes the handle and aborts
    /// so a dead slot doesn't keep pumping tokens into a dropped channel.
    pub turn: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Live count of WS forwarders subscribed to this slot's `events_tx`.
    /// Bumped by [`AttachGuard::new`]; decremented on drop. Approver reads
    /// this to decide whether the session is "background" for the purposes
    /// of unanswered approvals.
    pub attached: Arc<AtomicUsize>,
    pub background_mode: Arc<RwLock<BackgroundMode>>,
    /// Where this session's tools run. Bound to the slot's worktree:
    /// switching worktrees makes a new slot, so each worktree keeps its
    /// own environment.
    pub environments: Arc<mira_compute::EnvironmentManager>,
}

impl SessionSlot {
    /// Build a fresh `ToolContext` bound to the slot's current cwd, the
    /// shared sandbox, and the slot's active memory + episodic stores.
    pub async fn make_tool_ctx(&self, sandbox: Arc<Sandbox>) -> ToolContext {
        let cwd = self.cwd.read().await.clone();
        let memory = self.memory.read().await.clone();
        let episodic = self.episodic.read().await.clone();
        ToolContext::new(cwd, sandbox)
            .with_memory(memory)
            .with_episodic(episodic)
    }

    /// Swap the memory + episodic stores to point at `cwd`. Called on
    /// per-slot cwd changes (session load into a new folder, `PUT /api/cwd`).
    pub async fn rebuild_memory(&self, cwd: &std::path::Path) {
        let mem: Arc<dyn MemoryStore> = Arc::new(FileMemoryStore::new(
            mira_config::user_memory_path(),
            mira_config::project_memory_path(cwd),
        ));
        *self.memory.write().await = mem;
        let epi: Arc<dyn EpisodicStore> = Arc::new(FileEpisodicStore::new(
            mira_memory::project_episodic_path(cwd),
        ));
        *self.episodic.write().await = epi;
    }

    /// True while at least one WS client is subscribed to this slot's
    /// event stream. Approver uses it to decide whether to short-circuit
    /// `Ask` decisions via [`BackgroundMode`].
    pub fn is_attached(&self) -> bool {
        self.attached.load(Ordering::SeqCst) > 0
    }

    /// True while a spawned turn task is still holding the slot's turn
    /// mutex. Cheap probe used by the sessions API to badge the sidebar
    /// with a "running" indicator.
    pub async fn is_running(&self) -> bool {
        let guard = self.turn.lock().await;
        match guard.as_ref() {
            Some(h) => !h.is_finished(),
            None => false,
        }
    }
}

/// RAII counter for attached WS clients. On construction, bumps the slot's
/// `attached` counter; on drop, decrements. The WS handler stores one of
/// these per connection, so a client that vanishes without a clean close
/// still releases its increment.
pub struct AttachGuard {
    counter: Arc<AtomicUsize>,
}

impl AttachGuard {
    pub fn new(slot: &SessionSlot) -> Self {
        slot.attached.fetch_add(1, Ordering::SeqCst);
        Self {
            counter: slot.attached.clone(),
        }
    }
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Everything a slot factory needs but that lives above the session — the
/// shared pieces reused across every slot in the process.
#[derive(Clone)]
pub struct SlotDeps {
    pub policy: Arc<Mutex<Policy>>,
    pub sandbox: Arc<Sandbox>,
    pub harness_provider: Arc<dyn ChatProvider>,
    /// Pre-`AgentTool`, pre-interactive registry snapshot. The slot factory
    /// clones this and layers in `PlanTool` / `AskUserTool` / `AgentTool`
    /// wired to the slot's own prompt channel.
    pub base_registry: Arc<Registry>,
    pub agents_registry: Arc<AgentRegistry>,
    pub store: Option<Arc<dyn SessionStore>>,
    pub memory_runtime: MemoryRuntimeConfig,
    /// Cross-subagent scratchpad shared across all slots. Keyed inside by
    /// parent session id, so two sessions can't see each other's notes
    /// even though the map is process-global.
    pub scratchpads: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
    /// Default model handed to spawned subagents when the call doesn't
    /// pin one. Snapshotted at boot; hot-swapping this would require a
    /// slot rebuild.
    pub default_model_for_agents: String,
    /// What subagents asking for `model: small` (or `haiku`) run on.
    pub small_model_for_agents: Option<String>,
    /// `compute:` config: named remote environments.
    pub compute: mira_config::ComputeConfig,
    /// Lifecycle hooks (plugins' and the user's).
    pub hooks: Option<Arc<dyn mira_harness::HookRunner>>,
}

/// Wire up a session with all its per-slot machinery.
///
/// `resume` selects between "load an existing SessionRecord from disk"
/// and "create a fresh Session". Either way, the returned slot is
/// self-contained — its events_tx, pending map, approver, and registry
/// all point at each other, and no other session shares them.
pub async fn build_slot(
    cwd: PathBuf,
    cfg: SessionConfig,
    resume: Option<SessionRecord>,
    deps: &SlotDeps,
) -> Arc<SessionSlot> {
    let (events_tx, _rx0) = broadcast::channel::<ServerMsg>(256);
    // NOTE: we intentionally drop `_rx0` — its only purpose was to keep the
    // channel alive before subscribers attach. Sends with zero receivers
    // return Err(SendError), which the harness forwarder happily ignores.
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let cwd_lock = Arc::new(RwLock::new(cwd.clone()));
    let attached = Arc::new(AtomicUsize::new(0));
    let background_mode = Arc::new(RwLock::new(BackgroundMode::default()));

    let memory_store: Arc<dyn MemoryStore> = Arc::new(FileMemoryStore::new(
        mira_config::user_memory_path(),
        mira_config::project_memory_path(&cwd),
    ));
    let episodic_store: Arc<dyn EpisodicStore> = Arc::new(FileEpisodicStore::new(
        mira_memory::project_episodic_path(&cwd),
    ));
    let memory = Arc::new(RwLock::new(memory_store.clone()));
    let episodic = Arc::new(RwLock::new(episodic_store.clone()));

    let approver: Arc<dyn Approver> = Arc::new(WsApprover::new(
        events_tx.clone(),
        pending.clone(),
        cwd_lock.clone(),
        attached.clone(),
        background_mode.clone(),
    ));

    let prompt_channel = PromptChannel::new(events_tx.clone());
    let prompt_pending = prompt_channel.pending();
    // Shared-trait view of the same channel for the moved tools.
    let prompt_shared: std::sync::Arc<dyn mira_tools::prompt::PromptChannel> =
        std::sync::Arc::new(prompt_channel.clone());

    // Slot-specific registry: base + interactive tools wired to this slot's
    // prompt channel + a fresh AgentTool sharing the process-wide scratchpad
    // map (keyed inside by parent session id — see AgentTool).
    let mut registry_owned: Registry = (*deps.base_registry).clone();
    registry_owned.register(PlanTool::new(prompt_shared.clone()));
    registry_owned.register(AskUserTool::new(prompt_shared.clone()));
    let mut agent_tool = AgentTool::new(
        deps.harness_provider.clone(),
        deps.base_registry.clone(),
        deps.default_model_for_agents.clone(),
    )
    .with_agents(deps.agents_registry.clone())
    .with_small_model(deps.small_model_for_agents.clone())
    .with_events_tx(events_tx.clone())
    .with_parent_approver(approver.clone())
    .with_parent_policy(deps.policy.clone())
    .with_prompt_channel(prompt_channel.clone());
    if let Some(store) = &deps.store {
        agent_tool = agent_tool.with_store(store.clone());
    }
    registry_owned.register(agent_tool);
    // Background process tools — registered last so they share the session's
    // bg_store that's also wired into the ToolContext below.
    mira_tools::builtin::register_background(&mut registry_owned);
    let registry = Arc::new(registry_owned);

    let environments = Arc::new(mira_compute::EnvironmentManager::new(
        cwd.clone(),
        mira_compute::EnvironmentManager::default_patch_dir(),
        deps.compute.clone(),
    ));
    // Session-lifetime progress sink — never cleared between turns so
    // background process drain tasks can keep emitting ToolProgress frames.
    let bg_progress: Arc<dyn ToolProgressSink> = Arc::new(SessionProgress {
        tx: events_tx.clone(),
    });
    let bg_store = BackgroundProcessStore::new();

    let initial_ctx = ToolContext::new(cwd.clone(), deps.sandbox.clone())
        .with_memory(memory_store.clone())
        .with_episodic(episodic_store.clone())
        .with_compute_slot(environments.slot())
        .with_bg_progress(bg_progress)
        .with_bg_processes(bg_store);

    let mut session = match resume {
        Some(record) => Session::resume_from(
            record,
            deps.harness_provider.clone(),
            registry.clone(),
            deps.policy.clone(),
            approver.clone(),
            initial_ctx,
        ),
        None => Session::new(
            cfg,
            crate::system_prompt(&cwd, &registry),
            deps.harness_provider.clone(),
            registry.clone(),
            deps.policy.clone(),
            approver.clone(),
            initial_ctx,
        ),
    };
    if let Some(store) = deps.store.clone() {
        session = session.with_store(store);
    }
    if let Some(hooks) = deps.hooks.clone() {
        session = session.with_hooks(hooks);
    }
    if deps.memory_runtime.inject_context() {
        session = session.with_memory_snapshot(crate::make_memory_snapshot_with(
            &cwd,
            episodic_store.clone(),
        ));
        session = session.with_memory_retrieval(crate::memory_retrieval_from(&deps.memory_runtime));
    }
    if deps.memory_runtime.auto_extract_enabled() {
        session = session.with_auto_extract(mira_harness::AutoExtractConfig::enabled(
            deps.memory_runtime.extractor_model().map(str::to_owned),
        ));
    }

    let id = session.id.clone();
    Arc::new(SessionSlot {
        id,
        session: Arc::new(RwLock::new(session)),
        events_tx,
        pending,
        prompt_pending,
        cwd: cwd_lock,
        memory,
        episodic,
        registry,
        approver,
        turn: Arc::new(Mutex::new(None)),
        attached,
        background_mode,
        environments,
    })
}

/// Switch `slot`'s environment in the background, streaming progress
/// and the outcome on its event channel. The model gets a note about
/// the new situation before its next turn.
pub fn spawn_environment_switch(slot: Arc<SessionSlot>, target: String) {
    tokio::spawn(async move {
        let tx = slot.events_tx.clone();
        let progress: mira_compute::env::Progress = Arc::new(move |text: String| {
            let _ = tx.send(ServerMsg::EnvironmentProgress { text });
        });
        let result = slot.environments.switch(&target, progress).await;
        let status = slot.environments.status().await;
        let msg = match result {
            Ok(report) => {
                if !report.model_note.is_empty() {
                    slot.session
                        .read()
                        .await
                        .push_note(report.model_note.clone())
                        .await;
                }
                ServerMsg::EnvironmentSwitched {
                    lines: switch_lines(&report),
                    conflicts: report
                        .pulled
                        .as_ref()
                        .map(|p| p.conflicts.clone())
                        .unwrap_or_default(),
                    from: report.from,
                    to: report.to,
                    error: None,
                    status,
                }
            }
            Err(e) => ServerMsg::EnvironmentSwitched {
                from: status.current.clone(),
                to: target,
                lines: Vec::new(),
                conflicts: Vec::new(),
                error: Some(e.to_string()),
                status,
            },
        };
        let _ = slot.events_tx.send(msg);
    });
}

fn switch_lines(r: &mira_compute::SwitchReport) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(p) = &r.pulled {
        if p.changed() {
            lines.push(format!(
                "Merged changes from `{}` into the worktree:",
                r.from
            ));
            lines.extend(p.stat.lines().map(str::to_owned));
            if let Some(path) = &p.patch_path {
                lines.push(format!("Patch kept at {}", path.display()));
            }
        } else {
            lines.push(format!("`{}` had no changes.", r.from));
        }
    }
    lines
}
