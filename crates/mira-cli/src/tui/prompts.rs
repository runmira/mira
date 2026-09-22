//! TUI-side prompt channel — routes `plan` / `ask_user` tool requests
//! into the event loop the same way `TuiApprover` routes approvals.
//!
//! The tool fires a [`PromptRequest`] (with the tool-call id as the
//! prompt id) and parks on a oneshot; the event loop receives it, pops
//! an interactive card, and answers with a [`PromptResponse`] built
//! from the user's keys. If the UI drops the receiver (user quit), the
//! oneshot resolves to `None` and the tool degrades to a graceful
//! "cancelled" result — a turn can never hang on a missing UI.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use mira_tools::prompt::{PromptChannel, PromptRequest, PromptResponse};

/// One tool-initiated prompt on its way to the event loop.
pub struct TuiPrompt {
    /// Tool-call id — also the reply key.
    pub prompt_id: String,
    pub request: PromptRequest,
    /// Completes the round-trip back to the waiting tool.
    pub reply: oneshot::Sender<PromptResponse>,
}

/// The channel handed to `PlanTool` / `AskUserTool` at registry-build
/// time. Cheap to clone (`Arc` around the sender).
#[derive(Clone)]
pub struct TuiPromptChannel {
    tx: mpsc::UnboundedSender<TuiPrompt>,
}

impl TuiPromptChannel {
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<TuiPrompt>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(Self { tx }), rx)
    }
}

#[async_trait]
impl PromptChannel for TuiPromptChannel {
    async fn ask(&self, prompt_id: String, request: PromptRequest) -> Option<PromptResponse> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let prompt = TuiPrompt {
            prompt_id,
            request,
            reply: reply_tx,
        };
        // Unbounded send only fails when the event loop is gone (user
        // quit) — exactly the "no UI" case the tools degrade on.
        self.tx.send(prompt).ok()?;
        reply_rx.await.ok()
    }
}
