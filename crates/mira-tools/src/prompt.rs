//! Interactive prompt tools — `plan` and `ask_user`.
//!
//! Both pause a turn and wait for structured human input. They talk to
//! the UI through the [`PromptChannel`] trait so the SAME tool works in
//! every frontend: `mira serve` implements it over the WebSocket
//! broadcast (see `mira_server::interactive`), and the TUI implements
//! it over an mpsc pair into the event loop. Wire pattern mirrors the
//! approval flow: fire a request keyed by the tool-call id, park a
//! oneshot, await the structured response. A dropped reply (UI exited,
//! user quit) degrades to `None` — callers turn that into a
//! "cancelled, ask in plain text" result so the turn never hangs.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{spec, Action, Tool, ToolError};

/* ---------- plan payloads ---------- */

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

/// User's plan decision. When `approved` and `steps` is `Some`, the user
/// edited before approving; the tool returns the edited plan verbatim so
/// the model executes what the user actually agreed to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanResponse {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<PlanStep>>,
    /// Free-text note the user optionally left when cancelling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/* ---------- ask-user payloads ---------- */

/// One question the model wants the user to answer — rendered as a card
/// with hotkey-selectable options plus a free-text path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskUserQuestion {
    /// The question itself. Full sentence, question mark included.
    pub question: String,
    /// Short chip-style label (max ~12 chars) rendered above the
    /// question as an at-a-glance topic marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Available choices — 2-4 entries.
    pub options: Vec<AskUserOption>,
    /// Multi-select checkboxes vs. single-select radios.
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskUserOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The model's preferred pick — surfaces a "Recommended" badge.
    #[serde(default)]
    pub recommended: bool,
}

/// The model's structured question set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskUserProposal {
    pub questions: Vec<AskUserQuestion>,
}

/// User-supplied answers, one entry per question in the order posed.
/// Empty vec = user cancelled the whole card.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskUserResponse {
    pub answers: Vec<AskUserAnswer>,
    /// True when the user dismissed the card without answering.
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskUserAnswer {
    /// Labels of options the user selected. Empty on the free-text path.
    #[serde(default)]
    pub picked: Vec<String>,
    /// Free text the user typed. `None` when they picked options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<String>,
}

/* ---------- channel ---------- */

/// What a tool fires at the UI. Extend as new interactive tools land.
#[derive(Clone, Debug)]
pub enum PromptRequest {
    Plan(PlanProposal),
    AskUser(AskUserProposal),
}

/// Reviewer's verdict on a subagent summary (server subagent flow).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentReviewResponse {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Union of every response shape — shared wire types so one channel
/// carries them all.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptResponse {
    Plan(PlanResponse),
    AskUser(AskUserResponse),
    SubagentReview(SubagentReviewResponse),
}

/// The UI side of an interactive prompt. Implemented once per frontend:
/// `mira serve` over WebSocket, the TUI over an mpsc pair.
#[async_trait]
pub trait PromptChannel: Send + Sync {
    /// Fire a prompt at the user and wait for the structured response.
    /// `None` means no UI / the UI went away — callers degrade
    /// gracefully rather than erroring the turn.
    async fn ask(&self, prompt_id: String, request: PromptRequest) -> Option<PromptResponse>;
}

/* ---------- plan tool ---------- */

pub struct PlanTool {
    channel: std::sync::Arc<dyn PromptChannel>,
}

impl PlanTool {
    pub fn new(channel: std::sync::Arc<dyn PromptChannel>) -> Self {
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
        // Purely a UI-side dialog — no filesystem or shell effects. The
        // user is already gating via the card.
        Action::Pure
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        _ctx: &crate::context::ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: PlanArgs = call.parse_arguments()?;
        let proposal = PlanProposal {
            title: args.title,
            steps: args.steps,
        };
        let prompt_id = call.id.to_string();

        let response = match self
            .channel
            .ask(prompt_id, PromptRequest::Plan(proposal.clone()))
            .await
        {
            Some(PromptResponse::Plan(p)) => p,
            Some(_) => {
                return Ok(ToolResult::ok(
                    call.id.clone(),
                    "Plan prompt received the wrong response kind — treating \
                     as cancelled. Ask the user how they'd like to proceed."
                        .to_owned(),
                ));
            }
            None => {
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

        // Approved. If the user toggled steps, echo the FINAL set back
        // so the model executes what was agreed, not the original.
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

/* ---------- ask-user tool ---------- */

pub struct AskUserTool {
    channel: std::sync::Arc<dyn PromptChannel>,
}

impl AskUserTool {
    pub fn new(channel: std::sync::Arc<dyn PromptChannel>) -> Self {
        Self { channel }
    }
}

#[derive(Deserialize)]
struct AskUserArgs {
    questions: Vec<AskUserQuestion>,
}

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "ask_user",
            "Ask the user 1-4 structured clarifying questions before \
             committing to a plan or an implementation approach. Prefer \
             this over guessing when a request could plausibly be \
             shaped several ways and the choice will materially affect \
             the code you write — target user vs. admin, permissions \
             model, scope boundary, storage backend, auth flow, \
             framework choice, migration vs. rewrite, etc.\n\n\
             Each question offers 2-4 concrete options; mark exactly \
             one option `recommended: true` when you have a considered \
             preference, otherwise leave them all unmarked. Users can \
             always answer with free text via a built-in \"Tell mira \
             what to do differently\" affordance you don't need to \
             include as an option — the UI adds it automatically.\n\n\
             When NOT to call this: the request is unambiguous, or a \
             single quick file read would resolve the ambiguity, or \
             you're mid-execution and picking would derail the flow. \
             When in doubt: ask. A short clarification round beats \
             building the wrong thing.\n\n\
             After answers come back, use them to shape a concrete \
             `plan` proposal (or execute directly if the request is \
             small). Do not chain `ask_user` calls — pose every \
             question you need in one call.",
            json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": {
                                    "type": "string",
                                    "description": "Full question, ending with '?'."
                                },
                                "header": {
                                    "type": "string",
                                    "description": "Very short label (max ~12 chars) — chip-style topic marker like 'Backend' or 'Scope'."
                                },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "maxItems": 4,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": {
                                                "type": "string",
                                                "description": "Short click target — 1-5 words."
                                            },
                                            "description": {
                                                "type": "string",
                                                "description": "Optional single-sentence explanation of trade-offs."
                                            },
                                            "recommended": {
                                                "type": "boolean",
                                                "description": "Set to true on the model's preferred option; a 'Recommended' badge shows in the UI."
                                            }
                                        },
                                        "required": ["label"],
                                        "additionalProperties": false
                                    }
                                },
                                "multi_select": {
                                    "type": "boolean",
                                    "description": "Set true when several options can apply simultaneously (features to enable, roles to include). Defaults to false = single-choice."
                                }
                            },
                            "required": ["question", "options"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["questions"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        _ctx: &crate::context::ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: AskUserArgs = call.parse_arguments()?;
        let proposal = AskUserProposal {
            questions: args.questions,
        };
        let prompt_id = call.id.to_string();

        let response = match self
            .channel
            .ask(prompt_id, PromptRequest::AskUser(proposal.clone()))
            .await
        {
            Some(PromptResponse::AskUser(r)) => r,
            Some(_) => {
                return Ok(ToolResult::ok(
                    call.id.clone(),
                    "Ask-user prompt received the wrong response kind — treating \
                     as cancelled. Ask the user how they'd like to proceed."
                        .to_owned(),
                ));
            }
            None => {
                return Ok(ToolResult::ok(
                    call.id.clone(),
                    "Ask-user prompt cancelled — no UI available. Ask the user \
                     (in plain text) how they'd like to proceed."
                        .to_owned(),
                ));
            }
        };

        if response.cancelled {
            return Ok(ToolResult::ok(
                call.id.clone(),
                "User dismissed the question card without answering. Ask \
                 for the missing context in plain text, or propose your \
                 best guess and let the user course-correct."
                    .to_owned(),
            ));
        }

        // Compose a readable summary the model can act on. Pair each
        // question with its resolved answer — either the picked
        // options or the free-text "differently" note.
        let mut body = String::from(
            "User answered. Use these choices to shape the next step (typically a `plan` call):\n\n",
        );
        for (idx, q) in proposal.questions.iter().enumerate() {
            let a = response.answers.get(idx);
            let header = q.header.as_deref().unwrap_or("Q");
            body.push_str(&format!("[{header}] {}\n", q.question));
            match a {
                None => body.push_str("  → (no answer captured)\n"),
                Some(a) if a.picked.is_empty() && a.custom.is_none() => {
                    body.push_str("  → (skipped)\n");
                }
                Some(a) => {
                    if !a.picked.is_empty() {
                        body.push_str("  → picked: ");
                        body.push_str(&a.picked.join(", "));
                        body.push('\n');
                    }
                    if let Some(txt) = a.custom.as_deref() {
                        let txt = txt.trim();
                        if !txt.is_empty() {
                            body.push_str("  → user said: ");
                            body.push_str(txt);
                            body.push('\n');
                        }
                    }
                }
            }
            body.push('\n');
        }
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
