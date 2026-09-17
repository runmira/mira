//! Browser-driven approver.
//!
//! When the policy engine returns `Ask`, this approver:
//!
//! 1. Pushes an `ApprovalRequest` frame to whichever WS clients are
//!    attached to this session (via the slot's `event_tx`).
//! 2. Registers a oneshot channel keyed by the tool call id in the slot's
//!    `pending` map.
//! 3. Awaits the client's `Approve { call_id, allow }` reply, which is
//!    routed back through [`resolve`].
//!
//! ## Background mode
//!
//! Slots track a live count of attached WS clients. When a call comes in
//! with zero attached clients, the approver skips the prompt and applies
//! the slot's [`BackgroundMode`]:
//! - `Deny`         — auto-deny, safe default
//! - `AutoApprove`  — auto-approve, use for trusted read-only runs
//! - `Park`         — legacy behavior; wait for a client to attach and
//!                    answer (still bounded by the 10-minute safety timeout)
//!
//! This is what makes "leave the tab, session keeps running" actually
//! useful: a tool call that would previously have parked forever now
//! makes forward progress under a mode the user opted into.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use crate::slot::BackgroundMode;

pub type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

/// Upper bound on how long we wait for a user's Allow/Deny click before
/// treating the pending approval as denied. A parked session with nobody
/// answering would otherwise hold the shell mutex + the approval oneshot
/// forever; 10 minutes leaves the user plenty of time to read a diff and
/// still guarantees eventual forward progress.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct WsApprover {
    event_tx: broadcast::Sender<ServerMsg>,
    pending: PendingMap,
    /// Slot cwd — read through the same Arc so preview computation
    /// targets whatever folder the slot's session is bound to.
    cwd: Arc<RwLock<PathBuf>>,
    /// Slot's live attached-client count. When zero, background mode
    /// applies instead of prompting.
    attached: Arc<AtomicUsize>,
    background_mode: Arc<RwLock<BackgroundMode>>,
}

impl WsApprover {
    pub fn new(
        event_tx: broadcast::Sender<ServerMsg>,
        pending: PendingMap,
        cwd: Arc<RwLock<PathBuf>>,
        attached: Arc<AtomicUsize>,
        background_mode: Arc<RwLock<BackgroundMode>>,
    ) -> Self {
        Self {
            event_tx,
            pending,
            cwd,
            attached,
            background_mode,
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

        // Background short-circuit: nobody's watching → don't bother
        // building an approval oneshot or broadcasting to a channel with
        // no receivers. `Park` falls through to the wait path.
        let attached = self.attached.load(Ordering::SeqCst) > 0;
        if !attached {
            match *self.background_mode.read().await {
                BackgroundMode::Deny => {
                    warn!(
                        call_id = %call.id,
                        "background approve: no client attached; auto-denying (Deny mode)"
                    );
                    return false;
                }
                BackgroundMode::AutoApprove => {
                    warn!(
                        call_id = %call.id,
                        "background approve: no client attached; auto-approving (AutoApprove mode)"
                    );
                    return true;
                }
                BackgroundMode::Park => {
                    // Fall through — same as attached path. Wait for
                    // someone to attach and answer, capped by the safety
                    // timeout below.
                }
            }
        }

        let (tx, rx) = oneshot::channel();
        let call_id: String = call.id.to_string();
        self.pending.lock().await.insert(call_id.clone(), tx);

        // Compute a diff preview for edit/write tools before asking; other
        // tools (bash, etc.) get `None` and the UI shows raw args.
        let cwd_snapshot = self.cwd.read().await.clone();
        let preview = compute_preview(&cwd_snapshot, call).await;

        // `broadcast::send` errors when there are no receivers. Under Park
        // mode this is still legit — a client may attach mid-approval and
        // subscribe fresh; the timeout below caps the wait either way.
        // Under Deny/AutoApprove with a client attached, this is the
        // normal path.
        let _ = self.event_tx.send(ServerMsg::ApprovalRequest {
            call: call.clone(),
            preview,
        });

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
