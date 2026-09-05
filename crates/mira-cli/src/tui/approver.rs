use std::sync::Arc;

use async_trait::async_trait;
use mira_core::ToolCall;
use mira_harness::Approver;
use mira_policy::Decision;
use tokio::sync::{mpsc, oneshot};

/// A single pending approval request handed to the TUI event loop.
pub struct ApprovalRequest {
    pub call: ToolCall,
    /// The TUI answers by sending true (allow) or false (deny).
    pub reply: oneshot::Sender<bool>,
}

/// Approver that bounces `Decision::Ask` through a channel to the UI.
///
/// The harness calls `approve()` from its spawned loop task; the TUI event
/// loop receives the request on the paired mpsc, displays a modal, and
/// answers via the oneshot. If the mpsc is closed (UI exited) we deny —
/// safer than allowing a call the user never saw.
pub struct TuiApprover {
    tx: mpsc::UnboundedSender<ApprovalRequest>,
}

impl TuiApprover {
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<ApprovalRequest>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(Self { tx }), rx)
    }
}

#[async_trait]
impl Approver for TuiApprover {
    async fn approve(&self, call: &ToolCall, _decision: Decision) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        let req = ApprovalRequest {
            call: call.clone(),
            reply: reply_tx,
        };
        if self.tx.send(req).is_err() {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }
}
