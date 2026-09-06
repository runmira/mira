use std::path::PathBuf;
use std::sync::Arc;

use mira_ai::ChatProvider;
use mira_harness::{Approver, Session, SessionStore};
use mira_policy::Policy;
use mira_sandbox::Sandbox;
use mira_tools::{Registry, ToolContext};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::approver::PendingMap;
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

    // --- pieces needed to (re)build Session instances ---
    pub registry: Arc<Registry>,
    pub sandbox: Arc<Sandbox>,
    pub approver: Arc<dyn Approver>,
    pub harness_provider: Arc<dyn ChatProvider>,
    pub store: Option<Arc<dyn SessionStore>>,
}

impl AppState {
    /// Cheap Arc-clone of the current live session.
    pub async fn current_session(&self) -> Session {
        self.session.read().await.clone()
    }

    pub async fn current_cwd(&self) -> PathBuf {
        self.cwd.read().await.clone()
    }

    /// Fresh ToolContext bound to the current cwd + shared sandbox.
    pub async fn make_tool_ctx(&self) -> ToolContext {
        ToolContext::new(self.current_cwd().await, self.sandbox.clone())
    }
}
