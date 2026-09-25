use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use mira_agents::AgentRegistry;
use mira_ai::ChatProvider;
use mira_config::MemoryRuntimeConfig;
use mira_core::SessionId;
use mira_harness::{Approver, Session, SessionStore};
use mira_memory::{EpisodicStore, MemoryStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::{Registry, ToolContext};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::approver::PendingMap;
use crate::interactive::{PendingPromptMap, ScratchpadEntry};
use crate::oauth::PendingFlowStore;
use crate::protocol::ServerMsg;
use crate::provider::SwappableProvider;
use crate::slot::{SessionSlot, SlotDeps};

/// Shared state handed to every axum handler.
///
/// The server hosts a **map** of live [`SessionSlot`]s. Every session has
/// its own broadcast bus, approval map, cwd, memory stores, and approver —
/// so a turn on session A can keep streaming into A's channel even while a
/// browser tab is watching session B.
///
/// `active` is the "most recently attached" session id — HTTP handlers
/// without a session_id in the URL (settings, memory append, undo, …)
/// operate on this slot. The WS handler is different: it tracks each
/// connection's own `attached_id` so two tabs can watch two sessions.
#[derive(Clone)]
pub struct AppState {
    // ---------- multi-session runtime ----------
    /// Live slots keyed by session id. Insert on new / load / cwd-switch;
    /// remove on delete_session (which also aborts the running turn).
    pub slots: Arc<RwLock<HashMap<SessionId, Arc<SessionSlot>>>>,
    /// "Most recently active" pointer used by HTTP handlers that don't
    /// carry a session id. WS attach updates this so the next HTTP call
    /// targets the session the user is watching.
    pub active: Arc<RwLock<SessionId>>,

    // ---------- process-wide shared bits ----------
    pub policy: Arc<Mutex<Policy>>,
    pub sandbox: Arc<Sandbox>,
    pub provider: SwappableProvider,
    pub harness_provider: Arc<dyn ChatProvider>,
    /// Pre-agent, pre-interactive registry snapshot. Slot factories layer
    /// PlanTool / AskUserTool / AgentTool on top per session.
    pub base_registry: Arc<Registry>,
    pub agents_registry: Arc<AgentRegistry>,
    /// `compute:` config handed to every slot's environment manager.
    pub compute: mira_config::ComputeConfig,
    pub store: Option<Arc<dyn SessionStore>>,
    /// MCP servers, plugins, custom commands.
    pub extensions: crate::extensions::Extensions,
    pub skills: mira_tools::builtin::skill::SkillHandle,
    pub pending_oauth: PendingFlowStore,
    pub local_port: u16,
    /// Memory-runtime config carried on state so `build_slot` can consult it
    /// when constructing a slot for a newly loaded session.
    pub memory_runtime: MemoryRuntimeConfig,
    /// Cross-session scratchpad. AgentTool instances scope their entries
    /// by parent session id, so this Mutex is process-shared but the
    /// notes stay isolated per session.
    pub scratchpads: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
    /// Default model handed to AgentTool for subagent spawns when the call
    /// doesn't override it. Snapshotted from the initial session config.
    pub default_model_for_agents: String,
}

impl AppState {
    // ---------- slot lookups ----------

    /// Return the "most recently attached" slot. HTTP handlers with no
    /// session id in the URL default here.
    pub async fn active_slot(&self) -> Arc<SessionSlot> {
        let id = self.active.read().await.clone();
        let slots = self.slots.read().await;
        slots
            .get(&id)
            .cloned()
            .expect("active session id must exist in slots")
    }

    pub async fn slot(&self, id: &SessionId) -> Option<Arc<SessionSlot>> {
        self.slots.read().await.get(id).cloned()
    }

    /// Find a slot by string id — used by handlers that receive a path
    /// parameter and don't want to build a SessionId every time.
    pub async fn slot_str(&self, id: &str) -> Option<Arc<SessionSlot>> {
        self.slot(&SessionId::from(id)).await
    }

    pub async fn list_slots(&self) -> Vec<Arc<SessionSlot>> {
        self.slots.read().await.values().cloned().collect()
    }

    pub async fn insert_slot(&self, slot: Arc<SessionSlot>) {
        let id = slot.id.clone();
        self.slots.write().await.insert(id, slot);
    }

    pub async fn remove_slot(&self, id: &SessionId) -> Option<Arc<SessionSlot>> {
        self.slots.write().await.remove(id)
    }

    pub async fn set_active(&self, id: SessionId) {
        *self.active.write().await = id;
    }

    // ---------- backwards-compat accessors ----------
    //
    // Existing handlers (memory, review, skills, undo, settings, title,
    // cwd, sessions, oauth::refresh, pull_requests, git, …) were written
    // against a singleton `state.session`/`state.events_tx`/`state.cwd`.
    // The methods below preserve that surface by routing to the ACTIVE
    // slot's fields — so a handler operating on "the current session"
    // still behaves the way the user's action just implied.

    /// Cheap clone of the active slot's Session.
    pub async fn current_session(&self) -> Session {
        self.active_slot().await.session.read().await.clone()
    }

    pub async fn current_cwd(&self) -> PathBuf {
        self.active_slot().await.cwd.read().await.clone()
    }

    pub async fn current_memory(&self) -> Arc<dyn MemoryStore> {
        self.active_slot().await.memory.read().await.clone()
    }

    pub async fn current_episodic(&self) -> Arc<dyn EpisodicStore> {
        self.active_slot().await.episodic.read().await.clone()
    }

    /// Active slot's broadcast sender. Fanout target for handlers that
    /// broadcast to whichever tab happens to be watching the active
    /// session (title updates, cwd swap Ready, ModelChanged, …).
    pub async fn events_tx(&self) -> broadcast::Sender<ServerMsg> {
        self.active_slot().await.events_tx.clone()
    }

    pub async fn pending(&self) -> PendingMap {
        self.active_slot().await.pending.clone()
    }

    pub async fn prompt_pending(&self) -> PendingPromptMap {
        self.active_slot().await.prompt_pending.clone()
    }

    pub async fn approver(&self) -> Arc<dyn Approver> {
        self.active_slot().await.approver.clone()
    }

    pub async fn registry(&self) -> Arc<Registry> {
        self.active_slot().await.registry.clone()
    }

    /// Shared handle to the active slot's cwd RwLock. Kept for
    /// backwards-compat with handlers that took an `Arc<RwLock<PathBuf>>`
    /// directly (skills watcher, WsApprover for other slots, …).
    pub async fn cwd_handle(&self) -> Arc<RwLock<PathBuf>> {
        self.active_slot().await.cwd.clone()
    }

    /// Build a `ToolContext` bound to the active slot.
    pub async fn make_tool_ctx(&self) -> ToolContext {
        self.active_slot()
            .await
            .make_tool_ctx(self.sandbox.clone())
            .await
    }

    /// Swap the active slot's memory/episodic stores after its cwd changed.
    pub async fn rebuild_memory_for_cwd(&self, cwd: &std::path::Path) {
        self.active_slot().await.rebuild_memory(cwd).await;
    }

    /// Shared bundle handed to `build_slot` — snapshotted from `AppState`
    /// each time a new slot is spun up.
    pub fn slot_deps(&self) -> SlotDeps {
        SlotDeps {
            policy: self.policy.clone(),
            sandbox: self.sandbox.clone(),
            harness_provider: self.harness_provider.clone(),
            base_registry: self.base_registry.clone(),
            agents_registry: self.agents_registry.clone(),
            store: self.store.clone(),
            memory_runtime: self.memory_runtime.clone(),
            scratchpads: self.scratchpads.clone(),
            default_model_for_agents: self.default_model_for_agents.clone(),
            compute: self.compute.clone(),
            hooks: Some(self.extensions.hook_runner()),
        }
    }

    /// Fan a message out to every live slot's `events_tx`.
    ///
    /// The WS forwarder subscribes to exactly one slot at a time — so a
    /// frame published only on slot A never reaches a client that's
    /// watching slot B. For session-lifecycle events (title updated,
    /// background running/idle, background-mode changed) that would break
    /// sidebar refresh: change slot A's mode and clients watching B would
    /// miss it. Fanning out keeps every attached client's sidebar
    /// eventually consistent regardless of which session they're
    /// currently focused on.
    pub async fn broadcast_all(&self, msg: ServerMsg) {
        for slot in self.list_slots().await {
            let _ = slot.events_tx.send(msg.clone());
        }
    }

    /// Return the slot for `id`, materializing one from a persisted record
    /// if it isn't already loaded. Returns `Err` when persistence is
    /// disabled or the record doesn't exist.
    ///
    /// Both `WS Attach { id }` and `POST /api/sessions/:id/load` go
    /// through this so a click on a sidebar row for a session the server
    /// hasn't opened yet still lands cleanly.
    pub async fn ensure_slot(&self, id: &SessionId) -> Result<Arc<SessionSlot>, String> {
        if let Some(slot) = self.slot(id).await {
            return Ok(slot);
        }
        let Some(store) = self.store.clone() else {
            return Err(
                "persistence disabled — cannot materialize a slot for an unloaded session"
                    .to_string(),
            );
        };
        let record = store.load(id).await.map_err(|e| format!("load: {e}"))?;
        let cwd = record.cwd.clone();
        let cfg = record.cfg.clone();
        let deps = self.slot_deps();
        let slot = crate::slot::build_slot(cwd, cfg, Some(record), &deps).await;
        self.insert_slot(slot.clone()).await;
        Ok(slot)
    }
}
