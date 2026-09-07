//! Interactive tools — model-invoked capabilities that pause execution and
//! wait for structured user input.
//!
//! Today: [`PlanTool`] — the model proposes a step-by-step plan, the user
//! reviews / edits / approves / cancels, and the decision comes back as the
//! tool result. Next up (using the same [`PromptChannel`] plumbing) will be
//! an ask-user tool for option-picking mid-task.
//!
//! Wire pattern mirrors the approval flow: broadcast a `ServerMsg` to the
//! client, register a oneshot channel keyed by a fresh id, await the
//! client's `ClientMsg::PromptResponse`. See [`resolve`] for the routing
//! end. If no client is connected the tool returns a "cancelled — no UI"
//! result rather than blocking forever.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use mira_tools::context::ToolContext;
use mira_tools::tool::{spec, Action, Tool, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::warn;

use crate::protocol::ServerMsg;

/* ---------- shared channel ---------- */

pub type PendingPromptMap = Arc<Mutex<HashMap<String, oneshot::Sender<PromptResponse>>>>;

/// Broadcast-driven pipe used by every interactive tool. The tool asks the
/// user by sending a `ServerMsg` and awaits the client's reply on a oneshot
/// registered under a fresh `prompt_id`.
#[derive(Clone)]
pub struct PromptChannel {
    events_tx: broadcast::Sender<ServerMsg>,
    pending: PendingPromptMap,
}

impl PromptChannel {
    pub fn new(events_tx: broadcast::Sender<ServerMsg>) -> Self {
        Self {
            events_tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn pending(&self) -> PendingPromptMap {
        self.pending.clone()
    }

    /// Fire a `ServerMsg`, park a oneshot, and wait for a matching
    /// `PromptResponse`. Returns `None` when the socket drops or nobody's
    /// listening — the caller renders that as a graceful cancellation.
    async fn ask(&self, prompt_id: String, msg: ServerMsg) -> Option<PromptResponse> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(prompt_id.clone(), tx);
        if self.events_tx.send(msg).is_err() {
            self.pending.lock().await.remove(&prompt_id);
            warn!(prompt_id, "prompt sent with no listening client");
            return None;
        }
        match rx.await {
            Ok(v) => Some(v),
            Err(_) => {
                warn!(prompt_id, "prompt channel dropped");
                None
            }
        }
    }
}

/// Route a client-supplied `PromptResponse` to whichever tool is waiting on
/// it. Called from the WS reader when a `ClientMsg::PromptResponse` arrives.
pub async fn resolve(
    pending: &PendingPromptMap,
    prompt_id: &str,
    response: PromptResponse,
) -> bool {
    let sender = pending.lock().await.remove(prompt_id);
    match sender {
        Some(tx) => tx.send(response).is_ok(),
        None => false,
    }
}

/* ---------- plan tool ---------- */

/// Proposed plan structure. Mirrored on the wire so the client can render
/// and edit each step; the user's final choice comes back as [`PlanResponse`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanProposal {
    pub title: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanStep {
    pub description: String,
    /// Short justification for the step. Optional so the model can be terse
    /// when the description is self-explanatory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// Client → server payload for a plan prompt reply. When `approved` and
/// `steps` is `Some`, the user edited before approving; the tool returns the
/// edited plan verbatim so the model executes what the user actually agreed
/// to (not what it originally proposed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanResponse {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<PlanStep>>,
    /// Free-text note the user optionally left when cancelling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Union of every response shape the client can send. Extend as new
/// interactive tools land (question, options, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptResponse {
    Plan(PlanResponse),
}

pub struct PlanTool {
    channel: PromptChannel,
}

impl PlanTool {
    pub fn new(channel: PromptChannel) -> Self {
        Self { channel }
    }
}

#[derive(Deserialize)]
struct PlanArgs {
    title: String,
    steps: Vec<PlanStep>,
}

#[async_trait]
impl Tool for PlanTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "plan",
            "Propose a structured, step-by-step plan and pause for user \
             review before executing. Use this at the start of any \
             multi-step task (3+ discrete actions) — refactors, cross-file \
             changes, non-trivial features. The user can approve, edit, or \
             cancel. On approval you'll get back the final agreed plan; \
             execute it step by step using the appropriate tools. On \
             cancellation, stop and ask the user what they'd prefer.",
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "One-line summary of what this plan accomplishes."
                    },
                    "steps": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "description": {
                                    "type": "string",
                                    "description": "One concrete action (imperative)."
                                },
                                "why": {
                                    "type": "string",
                                    "description": "Optional short justification."
                                }
                            },
                            "required": ["description"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["title", "steps"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Purely a UI-side dialog — no filesystem or shell effects. `Pure`
        // means policy engines don't gate it; the user is already gating via
        // the modal.
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: PlanArgs = call.parse_arguments()?;
        let proposal = PlanProposal {
            title: args.title,
            steps: args.steps,
        };
        // Reuse the tool call id as the prompt id so the frontend can attach
        // the plan payload directly onto the in-flight tool card (no separate
        // "which tool call did this plan belong to?" bookkeeping).
        let prompt_id = call.id.to_string();
        let msg = ServerMsg::PlanRequest {
            prompt_id: prompt_id.clone(),
            plan: proposal.clone(),
        };

        let response = match self.channel.ask(prompt_id, msg).await {
            Some(PromptResponse::Plan(p)) => p,
            None => {
                // No UI connected / user dropped the socket. Return a
                // clear-but-not-error result so the model can adapt (e.g.
                // ask the user what to do next in text).
                return Ok(ToolResult::ok(
                    call.id.clone(),
                    "Plan prompt cancelled — no UI available to review the plan. \
                     Ask the user (in plain text) how they'd like to proceed."
                        .to_owned(),
                ));
            }
        };

        if !response.approved {
            let mut body = String::from("Plan cancelled by user.");
            if let Some(note) = &response.note {
                if !note.trim().is_empty() {
                    body.push_str(" Note: ");
                    body.push_str(note.trim());
                }
            }
            body.push_str(
                "\n\nDo not execute the proposed plan. Ask the user what \
                 they'd prefer, or propose a different approach.",
            );
            return Ok(ToolResult::ok(call.id.clone(), body));
        }

        // Approved. If the user edited, echo the FINAL steps back so the
        // model executes what was agreed, not the original proposal.
        let final_steps = response.steps.unwrap_or(proposal.steps);
        let mut body = String::from(
            "Plan approved. Execute step by step using the appropriate tools.\n\nAgreed steps:\n",
        );
        for (i, s) in final_steps.iter().enumerate() {
            body.push_str(&format!("{}. {}", i + 1, s.description));
            if let Some(w) = &s.why {
                body.push_str(&format!(" ({w})"));
            }
            body.push('\n');
        }
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
