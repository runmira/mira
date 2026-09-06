//! Browser-driven approver.
//!
//! When the policy engine returns `Ask`, this approver:
//!
//! 1. Pushes an `ApprovalRequest` frame to the connected client via
//!    `event_tx`.
//! 2. Registers a oneshot channel keyed by the tool call id in `pending`.
//! 3. Awaits the client's `Approve { call_id, allow }` reply, which is
//!    routed back through [`resolve`].
//!
//! If the socket drops mid-approval the oneshot is dropped and `approve`
//! returns `false` — the harness treats that as a denial and moves on.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use mira_core::ToolCall;
use mira_harness::Approver;
use mira_policy::Decision;
use mira_tools::compute_preview;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tracing::warn;

use crate::protocol::ServerMsg;

pub type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

pub struct WsApprover {
    event_tx: broadcast::Sender<ServerMsg>,
    pending: PendingMap,
    /// Shared with `AppState.cwd` — reading through the lock means diff
    /// previews always resolve against whatever folder the user picked most
    /// recently, not the one the server booted in.
    cwd: Arc<RwLock<PathBuf>>,
}

impl WsApprover {
    pub fn new(
        event_tx: broadcast::Sender<ServerMsg>,
        pending: PendingMap,
        cwd: Arc<RwLock<PathBuf>>,
    ) -> Self {
        Self {
            event_tx,
            pending,
            cwd,
        }
    }
}

#[async_trait]
impl Approver for WsApprover {
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool {
        match decision {
            Decision::Allow => return true,
            Decision::Deny => return false,
            Decision::Ask => {}
        }

        let (tx, rx) = oneshot::channel();
        let call_id: String = call.id.to_string();
        self.pending.lock().await.insert(call_id.clone(), tx);

        // Compute a diff preview for edit/write tools before asking; other
        // tools (bash, etc.) get `None` and the UI shows raw args.
        let cwd_snapshot = self.cwd.read().await.clone();
        let preview = compute_preview(&cwd_snapshot, call).await;

        // `broadcast::send` errors when there are no receivers — treat that as
        // "no client to ask" and deny.
        if self
            .event_tx
            .send(ServerMsg::ApprovalRequest {
                call: call.clone(),
                preview,
            })
            .is_err()
        {
            self.pending.lock().await.remove(&call_id);
            warn!(call_id, "approval requested but no client connected");
            return false;
        }

        match rx.await {
            Ok(allow) => allow,
            Err(_) => {
                warn!(call_id, "approval channel dropped");
                false
            }
        }
    }
}

/// Resolve a pending approval — called by the WS reader when the client sends
/// an `Approve` frame. Returns `true` if a waiter was found for the id.
pub async fn resolve(pending: &PendingMap, call_id: &str, allow: bool) -> bool {
    let sender = pending.lock().await.remove(call_id);
    match sender {
        Some(tx) => tx.send(allow).is_ok(),
        None => false,
    }
}
