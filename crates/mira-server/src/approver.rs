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
use std::time::Duration;

use async_trait::async_trait;
use mira_core::ToolCall;
use mira_harness::Approver;
use mira_policy::Decision;
use mira_tools::compute_preview;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tracing::warn;

use crate::protocol::ServerMsg;

pub type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

/// Upper bound on how long we wait for a user's Allow/Deny click before
/// treating the pending approval as denied. A connected client that goes
/// silent (tab backgrounded, network glitch, user walked away) would
/// otherwise park the entire harness on that one call forever — the
/// turn is stuck, cancellation can't drain the map, and the shell mutex
/// is held. 10 minutes gives the user plenty of time to read a diff and
/// still guarantees eventual forward progress.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

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

        // Bounded wait — an unanswered oneshot used to hang the whole
        // harness (audit Gap #2). On timeout we forget the pending entry
        // so a late reply from the client doesn't try to send into a
        // dropped channel, and we treat it as a denial (same effect as
        // a dropped socket).
        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(allow)) => allow,
            Ok(Err(_)) => {
                warn!(call_id, "approval channel dropped");
                false
            }
            Err(_) => {
                self.pending.lock().await.remove(&call_id);
                warn!(
                    call_id,
                    timeout_secs = APPROVAL_TIMEOUT.as_secs(),
                    "approval request timed out; treating as denied"
                );
                false
            }
        }
    }
}

/// Drain every pending approval as a denial. Called on session
/// cancel/interrupt so a Stop button also releases whatever approval
/// modal was blocking the turn. Returns how many pending entries were
/// resolved.
pub async fn drain_pending_as_denied(pending: &PendingMap) -> usize {
    let entries: Vec<oneshot::Sender<bool>> = {
        let mut guard = pending.lock().await;
        guard.drain().map(|(_, tx)| tx).collect()
    };
    let n = entries.len();
    for tx in entries {
        let _ = tx.send(false);
    }
    n
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
