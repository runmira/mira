use std::path::PathBuf;
use std::sync::Arc;

use mira_ai::ChatProvider;
use mira_harness::{Approver, Session, SessionStore};
use mira_memory::{EpisodicStore, FileEpisodicStore, FileMemoryStore, MemoryStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::{Registry, ToolContext};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::approver::PendingMap;
use crate::interactive::PendingPromptMap;
use crate::mcp::McpBootSnapshot;
use crate::protocol::ServerMsg;
use crate::provider::SwappableProvider;

/// Shared state handed to every axum handler.
///
/// The server hosts a single long-lived `Session`. Multiple browser tabs
/// connect to the same server and see the same conversation. `cwd` and
/// `session` live behind locks so the sessions/cwd APIs can hot-swap them
/// without restarting the server or dropping connected clients.
#[derive(Clone)]
pub struct AppState {
    pub session: Arc<RwLock<Session>>,
    pub policy: Arc<Mutex<Policy>>,
    /// Working directory the harness sees. Shared with the `WsApprover` so
    /// diff previews line up with wherever the user just switched to.
    pub cwd: Arc<RwLock<PathBuf>>,
    /// The provider baked into `session`. Held here too so the settings API
    /// can hot-swap it when the user reconfigures without restarting.
    pub provider: SwappableProvider,
    /// Fan-out for outbound frames. WS handlers subscribe; the harness
    /// forwarder task and the approver both publish.
    pub events_tx: broadcast::Sender<ServerMsg>,
    pub pending: PendingMap,
    /// Oneshot waiters keyed by prompt_id, used by interactive tools
    /// (plan / ask_user / …) to receive the user's reply from the WS.
    pub prompt_pending: PendingPromptMap,

    // --- pieces needed to (re)build Session instances ---
    pub registry: Arc<Registry>,
    pub sandbox: Arc<Sandbox>,
    pub approver: Arc<dyn Approver>,
    pub harness_provider: Arc<dyn ChatProvider>,
    pub store: Option<Arc<dyn SessionStore>>,
    /// Currently-active memory store. Bound to the current cwd's project
    /// path so the per-scope mutex serializes every writer (memory tools,
    /// `/api/memory/append`, future clients). Rebuilt on cwd change via
    /// [`AppState::rebuild_memory_for_cwd`].
    pub memory: Arc<RwLock<Arc<dyn MemoryStore>>>,
    /// Currently-active episodic (cross-session) store. Same lifecycle as
    /// `memory` — rebuilt on cwd change. `memory_remember` writes here;
    /// the memory snapshot renders the most-recent N entries into every
    /// round's live memory block.
    pub episodic: Arc<RwLock<Arc<dyn EpisodicStore>>>,
    /// MCP connect results captured once at server startup. Frozen for the
    /// lifetime of the process — we don't live-reload connections yet, so
    /// the Plugins UI treats yaml drift from this snapshot as
    /// "restart required."
    pub mcp_boot: McpBootSnapshot,
}

impl AppState {
    /// Cheap Arc-clone of the current live session.
    pub async fn current_session(&self) -> Session {
        self.session.read().await.clone()
    }

    pub async fn current_cwd(&self) -> PathBuf {
        self.cwd.read().await.clone()
    }

    /// Fresh ToolContext bound to the current cwd + shared sandbox and
    /// the active memory + episodic stores, so tools inside a session share
    /// the same per-scope mutex with `/api/memory/append` and every
    /// `memory_remember` call routes through the same file handle.
    pub async fn make_tool_ctx(&self) -> ToolContext {
        ToolContext::new(self.current_cwd().await, self.sandbox.clone())
            .with_memory(self.current_memory().await)
            .with_episodic(self.current_episodic().await)
    }

    /// Cheap Arc-clone of the current memory store.
    pub async fn current_memory(&self) -> Arc<dyn MemoryStore> {
        self.memory.read().await.clone()
    }

    /// Cheap Arc-clone of the current episodic store.
    pub async fn current_episodic(&self) -> Arc<dyn EpisodicStore> {
        self.episodic.read().await.clone()
    }

    /// Build fresh [`FileMemoryStore`] + [`FileEpisodicStore`] scoped to
    /// `cwd` and swap them in. Called after any cwd change (`put_cwd`,
    /// `load_session`) so subsequent tool calls, HTTP writes, and episodic
    /// reads target the correct project.
    pub async fn rebuild_memory_for_cwd(&self, cwd: &std::path::Path) {
        let mem: Arc<dyn MemoryStore> = Arc::new(FileMemoryStore::new(
            mira_config::user_memory_path(),
            mira_config::project_memory_path(cwd),
        ));
        *self.memory.write().await = mem;
        let epi: Arc<dyn EpisodicStore> =
            Arc::new(FileEpisodicStore::new(mira_memory::project_episodic_path(cwd)));
        *self.episodic.write().await = epi;
    }
}
