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
use futures::StreamExt;
use mira_agents::AgentRegistry;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, ToolSpec};
use mira_core::{Message, Role, ToolCall, ToolResult};
use mira_harness::{Approver, AutoApprover, HarnessEvent, Session, SessionConfig, SessionStore};
use mira_policy::{Policy, PolicyConfig};
use mira_tools::context::{ChildCancel, ToolContext};
use mira_tools::tool::{spec, Action, Tool, ToolError};
use mira_tools::Registry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{info, warn};

use crate::agent_worktree::WorktreeSession;
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

/// Client → server payload for a subagent-review prompt. `approved`
/// controls whether the child's summary flows back to the parent as
/// a success or as an error. Optional `note` lets the reviewer leave
/// a free-text remark — prepended to the summary on approval, used as
/// the error body on denial.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentReviewResponse {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Union of every response shape the client can send. Extend as new
/// interactive tools land (question, options, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptResponse {
    Plan(PlanResponse),
    SubagentReview(SubagentReviewResponse),
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
            Some(_) => {
                // Wrong response kind — a client bug landed a subagent_review
                // (or a future prompt shape) on a plan prompt id. Treat as a
                // graceful cancel so the model gets an actionable result
                // instead of hanging.
                return Ok(ToolResult::ok(
                    call.id.clone(),
                    "Plan prompt received the wrong response kind — treating \
                     as cancelled. Ask the user how they'd like to proceed."
                        .to_owned(),
                ));
            }
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

/* ---------- agent tool (subagents) ---------- */

/// Hard cap on nested subagent spawning. Depth 0 is the user's top-level
/// chat; the `agent` tool at depth N produces a child at depth N+1. When a
/// child's ambient depth already equals this cap, its own `agent` tool
/// refuses to spawn — the model can't accidentally recurse forever.
const MAX_AGENT_DEPTH: usize = 2;

/// Per-child ceiling on rounds when the caller didn't pin `max_rounds`.
/// Deliberately lower than the top-level session default (60) — subagents
/// exist to bound context, so if 30 rounds isn't enough the delegation
/// probably shouldn't be a subagent at all.
const DEFAULT_SUBAGENT_MAX_ROUNDS: usize = 30;

/// The `agent` tool. A parent session invokes it to delegate a bounded
/// piece of work to a child session with:
/// - cold context (child only sees the `prompt` arg; parent history stays
///   in the parent, so exploration work never pollutes it),
/// - a filtered tool set (optional `tools` arg — defaults to the parent's
///   full registry minus this tool's namesake),
/// - `auto` policy + [`AutoApprover`] (subagents run autonomously; if you
///   don't trust it, don't spawn it),
/// - a fresh [`FileGuard`] (undo is per-session anyway; the child's edits
///   still land on the same working tree the parent sees).
///
/// The result handed back to the parent is the child's final assistant
/// text. Tool calls, intermediate reasoning, and warnings from the child
/// are logged but not surfaced — the whole point is a bounded summary.
///
/// [`FileGuard`]: mira_tools::FileGuard
pub struct AgentTool {
    /// Provider handed to child sessions. Typically the parent's swappable
    /// provider so a settings hot-swap flows through to subagents too.
    provider: Arc<dyn ChatProvider>,
    /// Snapshot of the parent's registry taken BEFORE the AgentTool was
    /// registered into it — so subagents inherit peer tools (`read_file`,
    /// `grep`, `bash`, MCP, …) without pulling in `agent` itself. Nested
    /// spawning is re-enabled at construction time by putting a fresh
    /// `AgentTool` into the child registry with an appropriate depth.
    base_registry: Arc<Registry>,
    /// Default model for children when the caller doesn't specify one.
    /// Usually the parent's model at server-boot time. Overridable per
    /// call via the tool arg.
    default_model: String,
    /// Same broadcast the harness → WS forwarder pumps into. AgentTool
    /// pushes `Subagent*` frames here so the parent's UI can render a
    /// live child transcript in the right-side panel. `None` means the
    /// tool was constructed without a channel (headless / test paths) —
    /// events are dropped silently in that case.
    events_tx: Option<broadcast::Sender<ServerMsg>>,
    /// Named agent types loaded at boot (builtins + AGENTS.md merges).
    /// When the caller passes `type: "explore"` we look up the entry
    /// here and use its tools/model/prompt/schema overrides. `None`
    /// (empty registry) means every call falls through to the raw
    /// primitive.
    agents: Arc<AgentRegistry>,
    /// Session store shared with the parent — when set, spawned child
    /// sessions are checkpointed to disk so the SubagentPanel can
    /// reload their transcript after a browser refresh or process
    /// restart. `None` = ephemeral (child transcript lives in memory
    /// only, gone on reload).
    store: Option<Arc<dyn SessionStore>>,
    /// Parent's approver (typically `WsApprover`). Wired when we want
    /// write-capable subagents (`coder`, `documenter`) to route policy
    /// `Ask` decisions back to the user's UI instead of silently
    /// auto-approving them. Read-only agents keep the `AutoApprover`
    /// path — nothing dangerous to gate.
    parent_approver: Option<Arc<dyn Approver>>,
    /// Parent's live `Policy`. When set AND the resolved type routes
    /// approvals to the parent, the child shares the SAME Arc — so
    /// the parent's current mode and any `always allow` rules the user
    /// has accumulated apply to the child too. Without sharing, a
    /// subagent that spawned mid-turn would ignore later mode swaps
    /// (Auto → Yolo etc.), which is what caused the "still asks for
    /// permission after I set allow-everything" bug.
    parent_policy: Option<Arc<Mutex<Policy>>>,
    /// Prompt channel for review-required subagents. When wired and the
    /// resolved type has `review_required: true`, the child's final
    /// summary is broadcast to the UI as a `SubagentReviewRequest` and
    /// the tool blocks until the user approves or denies via
    /// `PromptResponse::SubagentReview`. Not required for other flows —
    /// spawns without a channel still return their summaries normally.
    channel: Option<PromptChannel>,
}

impl AgentTool {
    pub fn new(
        provider: Arc<dyn ChatProvider>,
        base_registry: Arc<Registry>,
        default_model: String,
    ) -> Self {
        Self {
            provider,
            base_registry,
            default_model,
            events_tx: None,
            agents: Arc::new(AgentRegistry::default()),
            store: None,
            parent_approver: None,
            parent_policy: None,
            channel: None,
        }
    }

    /// Wire the interactive prompt channel so review-required types can
    /// block on a human decision before returning. Also enables future
    /// interactive shapes (question/options prompts) from within the
    /// subagent flow without another builder.
    pub fn with_prompt_channel(mut self, channel: PromptChannel) -> Self {
        self.channel = Some(channel);
        self
    }

    /// Enable live child-event forwarding. When set, each HarnessEvent
    /// from a spawned child is translated into a `Subagent*` frame and
    /// broadcast to every connected WS.
    pub fn with_events_tx(mut self, tx: broadcast::Sender<ServerMsg>) -> Self {
        self.events_tx = Some(tx);
        self
    }

    /// Attach the named-agent-type registry — built-ins + any AGENTS.md
    /// files the loader merged. Callers pass `Arc<AgentRegistry>` so
    /// child spawns can share the same immutable snapshot cheaply.
    pub fn with_agents(mut self, agents: Arc<AgentRegistry>) -> Self {
        self.agents = agents;
        self
    }

    /// Attach the parent's session store so spawned children get
    /// checkpointed to disk. Required for the SubagentPanel to
    /// rebuild a child's transcript after a browser reload.
    pub fn with_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Wire the parent's approver so children of write-capable types
    /// (`coder`, `documenter`, or any custom type with
    /// `route_approvals_to_parent: true`) forward `Ask` decisions to the
    /// same modal the parent would use. Read-only types keep the
    /// `AutoApprover` path.
    pub fn with_parent_approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.parent_approver = Some(approver);
        self
    }

    /// Wire the parent's live `Policy`. Write-capable children share this
    /// Arc so `Auto → Yolo` mode swaps and always-allow rules the user
    /// clicks on the parent's approval modal apply to the child too.
    pub fn with_parent_policy(mut self, policy: Arc<Mutex<Policy>>) -> Self {
        self.parent_policy = Some(policy);
        self
    }

    /// LLM-as-router for `agent { type: "auto", prompt: ... }`. Runs a
    /// tiny classification call on `self.provider` (same provider as the
    /// parent) with a system prompt listing every registered type +
    /// description, and returns the chosen type name. Falls back to
    /// `explore` on any error — network, parse, unknown type — so the
    /// spawn always makes forward progress.
    ///
    /// Uses the parent's `default_model` intentionally: the router runs
    /// on the same account/rate limits the user already trusts, and
    /// modern models pick a one-word answer in a fraction of a second
    /// even at "big" tier. A future `agents.router_model` config knob
    /// could pin a cheaper model, but the current shape gets to
    /// correctness first.
    async fn route_auto(&self, task: &str) -> String {
        let fallback = "explore".to_owned();
        if self.agents.types.is_empty() {
            return fallback;
        }

        let roster = self
            .agents
            .types
            .values()
            .map(|t| {
                let desc = if t.description.is_empty() {
                    "(no description)"
                } else {
                    t.description.as_str()
                };
                format!("- {}: {}", t.name, desc)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = format!(
            "You are a subagent router. Given a task, pick the single best \
             subagent type from the roster below. Reply with ONLY the type \
             name — one word, lowercase, nothing else. No explanation, no \
             punctuation, no code fences.\n\n\
             Roster:\n{roster}"
        );

        let req = ChatRequest {
            model: self.default_model.clone(),
            messages: vec![
                Message::system(system),
                Message::user(format!("Task: {task}")),
            ],
            tools: Vec::new(),
            temperature: Some(0.0),
            max_tokens: Some(32),
            reasoning_effort: None,
            response_format: None,
        };

        let mut stream = match self.provider.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "auto-router: provider call failed; using explore");
                return fallback;
            }
        };

        let mut buf = String::new();
        while let Some(evt) = stream.next().await {
            match evt {
                Ok(ChatEvent::TextDelta(t)) => buf.push_str(&t),
                Ok(ChatEvent::Done(_)) => break,
                Err(e) => {
                    warn!(%e, "auto-router: stream error; using explore");
                    return fallback;
                }
                _ => {}
            }
        }

        // Take the first non-empty line, lowercase it, and strip anything
        // that isn't a valid type-name char. Handles the common failure
        // modes: model prefixes with "```", wraps in quotes, or answers
        // "explore." with a trailing period.
        let raw = buf
            .trim()
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();

        if self.agents.get(&cleaned).is_some() {
            cleaned
        } else {
            warn!(
                picked = %cleaned,
                raw = %buf.trim(),
                "auto-router returned unknown type; using explore"
            );
            fallback
        }
    }
}

#[derive(Deserialize)]
struct AgentArgs {
    prompt: String,
    /// Optional named type ("explore", "reviewer", …). Looked up in the
    /// AgentRegistry; when present, the type's tools/model/prompt/schema
    /// serve as defaults which the explicit args below can still override.
    #[serde(default)]
    r#type: Option<String>,
    /// Optional allowlist of tool names the child should see. When absent,
    /// the child gets every tool the parent had at server-boot time
    /// (except `agent`, which is re-added with the correct depth).
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// Optional per-call model override. Falls back to `default_model`.
    #[serde(default)]
    model: Option<String>,
    /// Optional per-call round budget. Falls back to
    /// `DEFAULT_SUBAGENT_MAX_ROUNDS`.
    #[serde(default)]
    max_rounds: Option<usize>,
}

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        // Enumerate the loaded types so the model sees the roster and
        // description right in the tool spec, not just as freeform prose.
        let type_names = self.agents.names();
        let type_description = if type_names.is_empty() {
            "Named agent type. No types are currently registered — omit this \
             field."
                .to_owned()
        } else {
            let roster = self
                .agents
                .types
                .values()
                .map(|t| format!("• `{}` — {}", t.name, t.description))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Optional named type. Available types:\n{roster}\n\nSpecial \
                 value `auto` runs a tiny classification call that picks the \
                 best fit from the roster for you — use it when you don't \
                 know which specialist applies, or want the router to try. \
                 When set, defaults (tools/model/prompt) come from the type; \
                 explicit args below still override.",
            )
        };

        spec(
            "agent",
            "Delegate a bounded task to a child agent. Use this when a \
             subtask needs many tool calls (searching a codebase, reading \
             several files, running a probe) that would otherwise clutter \
             your own context. The child starts COLD — it sees only the \
             `prompt` you write, not this conversation. Pick a `type` when \
             one fits (explore for research, reviewer for adversarial diff \
             review). The child returns a single summary string.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Self-contained instructions for the child. \
                                        Include everything it needs — file paths, \
                                        keywords, the shape of the answer you want. \
                                        Do NOT assume it has any context from this \
                                        conversation."
                    },
                    "type": {
                        "type": "string",
                        "enum": type_names,
                        "description": type_description,
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional allowlist of tool names the child \
                                        may use. Overrides the type's default \
                                        allowlist when a `type` is also set."
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model override for the child. \
                                        Overrides the type's default model."
                    },
                    "max_rounds": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional per-turn round cap for the child."
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Subagents can invoke edit/bash tools transitively, but the tool
        // *itself* is pure — no direct filesystem or shell side effects at
        // the parent's level. The child's calls get gated by the child's
        // policy independently.
        Action::Pure
    }

    fn parallel_safe(&self, call: &ToolCall) -> bool {
        // Look up the target type (if any) and consult its explicit
        // `parallel_safe` flag. Read-only types (explore/reviewer/…)
        // opt in; write-capable types (coder/documenter) stay
        // sequential until worktree isolation lands.
        //
        // When the type is unknown or the flag is unset, fall back to a
        // conservative default derived from the effective tool set: a
        // child whose tools are all read-ish is safe to parallelize; any
        // write/edit tool → sequential.
        let args = match call.parse_arguments::<AgentArgs>() {
            Ok(a) => a,
            Err(_) => return false, // bad args → play it safe
        };
        if let Some(name) = args.r#type.as_deref() {
            if let Some(ty) = self.agents.get(name) {
                if let Some(flag) = ty.parallel_safe {
                    return flag;
                }
                return tools_are_read_only(ty.tools.as_deref());
            }
        }
        // No named type — fall back on the caller's explicit tool list
        // (if given) or the safe default when the child would inherit
        // the full parent tool set (writes possible → false).
        args.tools
            .as_deref()
            .map(tools_are_read_only_slice)
            .unwrap_or(false)
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let mut args: AgentArgs = call.parse_arguments()?;

        // `type: "auto"` — LLM-as-router picks the specialist for us
        // before the rest of the resolution runs. Falls back to `explore`
        // internally on any failure so this branch always yields a
        // resolvable type name and the downstream "unknown type" error
        // path never fires for `auto`.
        if args
            .r#type
            .as_deref()
            .map(|s| s.trim().eq_ignore_ascii_case("auto"))
            .unwrap_or(false)
        {
            let routed = self.route_auto(&args.prompt).await;
            info!(
                task = %args.prompt.chars().take(80).collect::<String>(),
                routed = %routed,
                "auto-routed subagent"
            );
            args.r#type = Some(routed);
        }

        // Depth cap. The parent's ambient depth = ctx.agent_depth; the
        // child we'd spawn would be at depth = ctx.agent_depth + 1. Refuse
        // when that would exceed MAX_AGENT_DEPTH so runaway recursion
        // can't happen even if the child model tries to call `agent` again.
        let child_depth = ctx.agent_depth + 1;
        if child_depth > MAX_AGENT_DEPTH {
            return Ok(ToolResult::err(
                call.id.clone(),
                format!(
                    "refused: subagent nesting depth ({child_depth}) exceeds \
                     the hard cap of {MAX_AGENT_DEPTH}. Do the work in the \
                     current session instead."
                ),
            ));
        }

        // Resolve the named type (if any) once. Explicit args on the call
        // still win over the type's defaults — the type provides a
        // baseline, the caller picks per-call overrides.
        let type_def = args
            .r#type
            .as_deref()
            .and_then(|name| self.agents.get(name));
        if let (Some(name), None) = (args.r#type.as_deref(), type_def) {
            // Caller asked for a type we don't have — surface it clearly
            // rather than silently ignoring so bad prompts get flagged.
            return Ok(ToolResult::err(
                call.id.clone(),
                format!(
                    "unknown agent type `{name}`. Available: {}.",
                    if self.agents.names().is_empty() {
                        "(none)".to_owned()
                    } else {
                        self.agents.names().join(", ")
                    }
                ),
            ));
        }

        // Effective tool allowlist: explicit `tools` arg wins, otherwise
        // the type's list, otherwise inherit everything.
        let effective_tools: Option<Vec<String>> = args
            .tools
            .clone()
            .or_else(|| type_def.and_then(|t| t.tools.clone()));

        // Build the child's tool registry. Start from the parent's pre-agent
        // snapshot (so the child inherits peer tools) and apply the
        // effective allowlist. Then, if we're still under the depth cap,
        // re-add an AgentTool tuned to the child's depth so grand-children
        // remain possible up to MAX_AGENT_DEPTH.
        let mut child_registry = Registry::new();
        let allow = effective_tools.as_ref().map(|list| {
            list.iter().cloned().collect::<std::collections::HashSet<_>>()
        });
        for tool in self.base_registry.tools() {
            let name = tool.spec().name;
            if allow.as_ref().map(|s| s.contains(&name)).unwrap_or(true) {
                child_registry.register_arc(tool);
            }
        }
        if child_depth < MAX_AGENT_DEPTH {
            let mut nested = AgentTool::new(
                self.provider.clone(),
                self.base_registry.clone(),
                self.default_model.clone(),
            )
            .with_agents(self.agents.clone());
            if let Some(tx) = &self.events_tx {
                nested = nested.with_events_tx(tx.clone());
            }
            if let Some(store) = &self.store {
                nested = nested.with_store(store.clone());
            }
            if let Some(approver) = &self.parent_approver {
                nested = nested.with_parent_approver(approver.clone());
            }
            if let Some(policy) = &self.parent_policy {
                nested = nested.with_parent_policy(policy.clone());
            }
            if let Some(ch) = &self.channel {
                nested = nested.with_prompt_channel(ch.clone());
            }
            child_registry.register(nested);
        }
        // Streaming intermediate summaries: give every subagent a
        // `progress` tool that broadcasts `SubagentProgress` frames on
        // the events bus. The tool captures the parent's call id at
        // spawn time so its emissions land on the right SubagentPanel
        // tab. Skipped when no events channel is wired (headless / test).
        if let Some(tx) = &self.events_tx {
            let progress = ProgressTool::new(tx.clone(), call.id.to_string());
            child_registry.register(progress);
        }
        let child_registry = Arc::new(child_registry);

        // Approver + policy selection.
        //
        // Write-capable types (`coder`, `documenter`, or any custom type
        // with `route_approvals_to_parent: true`) share the parent's
        // **live Policy Arc** so:
        //   - flipping the parent's mode (Auto → Yolo etc.) applies to
        //     the child immediately, and
        //   - any "always allow" rule the user adds via an approval
        //     modal reaches the child on its next call.
        // They also share the parent's approver, so `Ask` decisions pop
        // the same modal the user sees for their own commands.
        //
        // Read-only types stay on a fresh Auto policy + AutoApprover —
        // there's nothing for the user to review, and interrupting flow
        // would defeat the whole "delegate cheap exploration" purpose.
        let route_to_parent = route_approvals_to_parent(type_def, effective_tools.as_deref());
        let (policy, approver): (Arc<Mutex<Policy>>, Arc<dyn mira_harness::Approver>) =
            if route_to_parent {
                let policy = match &self.parent_policy {
                    Some(p) => p.clone(),
                    None => {
                        warn!(
                            depth = child_depth,
                            "subagent routes to parent but no parent policy wired; using fresh Auto"
                        );
                        Arc::new(Mutex::new(fresh_auto_policy()?))
                    }
                };
                let approver: Arc<dyn mira_harness::Approver> = match &self.parent_approver {
                    Some(a) => a.clone(),
                    None => {
                        warn!(
                            depth = child_depth,
                            "subagent routes to parent but no parent approver wired; auto-approving"
                        );
                        Arc::new(AutoApprover { approve_asks: true })
                    }
                };
                (policy, approver)
            } else {
                let policy = Arc::new(Mutex::new(fresh_auto_policy()?));
                let approver: Arc<dyn mira_harness::Approver> =
                    Arc::new(AutoApprover { approve_asks: true });
                (policy, approver)
            };

        // Worktree isolation for write-capable types (Round 4). When the
        // type has `worktree: true`, spin up an ephemeral `git worktree`
        // off HEAD and swap the child's cwd to it, so parallel writers
        // can't stomp on each other's edits. The `WorktreeSession` is
        // consumed after the child completes — see `merge_and_cleanup`
        // below. Falls back to the parent cwd (with a warning) when the
        // parent isn't a git repo or the worktree creation errors, so a
        // misconfigured type never blocks the spawn entirely.
        let mut worktree: Option<WorktreeSession> = None;
        let child_cwd = if type_def.and_then(|t| t.worktree).unwrap_or(false) {
            let type_name = type_def
                .map(|t| t.name.as_str())
                .unwrap_or("agent");
            match WorktreeSession::try_create(&ctx.cwd, type_name, call.id.as_str()) {
                Ok(Some(w)) => {
                    info!(
                        depth = child_depth,
                        worktree = %w.cwd().display(),
                        "isolating subagent in ephemeral git worktree"
                    );
                    let cwd = w.cwd().to_path_buf();
                    worktree = Some(w);
                    cwd
                }
                Ok(None) => {
                    warn!(
                        "worktree isolation requested but parent cwd is not a git \
                         repo; running subagent in parent cwd"
                    );
                    if let Some(tx) = &self.events_tx {
                        let _ = tx.send(ServerMsg::SubagentWarning {
                            parent_call_id: call.id.to_string(),
                            text: "worktree isolation skipped — parent cwd is not a \
                                   git repository"
                                .to_owned(),
                        });
                    }
                    ctx.cwd.clone()
                }
                Err(e) => {
                    warn!(%e, "worktree create failed; running subagent in parent cwd");
                    if let Some(tx) = &self.events_tx {
                        let _ = tx.send(ServerMsg::SubagentWarning {
                            parent_call_id: call.id.to_string(),
                            text: format!("worktree isolation failed: {e}"),
                        });
                    }
                    ctx.cwd.clone()
                }
            }
        } else {
            ctx.cwd.clone()
        };

        // Fresh ToolContext — sandbox shared with the parent so shell
        // commands still hit the same allowlist; cwd is either the parent's
        // tree or the child's isolated worktree, depending on the type.
        // Guard is intentionally omitted here because Session::new attaches
        // a session-scoped one itself. Memory + episodic handles carry
        // through so the child can read the same MIRA.md the parent sees.
        let child_ctx = ToolContext::new(child_cwd, ctx.sandbox.clone())
            .with_agent_depth(child_depth);
        let child_ctx = if let Some(mem) = ctx.memory.clone() {
            child_ctx.with_memory(mem)
        } else {
            child_ctx
        };
        let child_ctx = if let Some(epi) = ctx.episodic.clone() {
            child_ctx.with_episodic(epi)
        } else {
            child_ctx
        };

        // Compose the child session config. Precedence at each field:
        //   explicit arg → type default → tool default → hard fallback.
        let model = args
            .model
            .filter(|s| !s.trim().is_empty())
            .or_else(|| type_def.and_then(|t| t.model.clone()))
            .unwrap_or_else(|| self.default_model.clone());
        let mut cfg = SessionConfig::new(model.clone());
        cfg.max_rounds = args
            .max_rounds
            .or_else(|| type_def.and_then(|t| t.max_rounds))
            .unwrap_or(DEFAULT_SUBAGENT_MAX_ROUNDS);
        // Deliberately NOT wiring the type's `response_schema` into the
        // child's `response_format` here. Provider-side JSON-schema
        // enforcement runs on EVERY assistant text message, not just the
        // last one — so a model like qwen sees "output must match the
        // schema" and emits a stub JSON on turn 1 (e.g. `{"summary":
        // "starting exploration…", "findings":[]}`) instead of calling
        // tools first. That's why subagents were coming back truncated.
        // The schema is still recorded in the type def and referenced in
        // the system prompt (see `subagent_system_prompt` and each type's
        // `.md`), and the frontend's `tryParseStructured` handles the
        // parse best-effort. If we want a hard contract back, the fix is
        // a `submit_report` tool the model calls on its final turn, not
        // provider-level JSON mode.
        let _ = type_def.and_then(|t| t.response_schema.clone());

        // System prompt: shared base + optional per-type addendum. Kept
        // as separate lines so a persona ("You are Draco…") reads as a
        // second paragraph rather than fighting the base rules.
        let system_prompt = match type_def.and_then(|t| t.system_prompt_addendum.as_deref()) {
            Some(addendum) if !addendum.trim().is_empty() => {
                format!("{}\n\n{addendum}", subagent_system_prompt())
            }
            _ => subagent_system_prompt().to_owned(),
        };

        let mut child = Session::new(
            cfg,
            system_prompt,
            self.provider.clone(),
            child_registry,
            policy,
            approver,
            child_ctx,
        );
        // Persist child sessions so the SubagentPanel can rehydrate its
        // transcript on browser reload. Uses the same store as the
        // parent; child SessionRecords land alongside parents keyed by
        // their own `SessionId`.
        if let Some(store) = &self.store {
            child = child.with_store(store.clone());
        }
        // Tag the child with the parent's session id so the sidebar's
        // list_sessions endpoint can filter subagent records out of the
        // primary chat list — otherwise every spawn shows up as a
        // standalone thread. The parent's id is the tool call's ambient
        // session (set by Session::new on the parent's ToolContext).
        if let Some(parent_id) = ctx.session_id.clone() {
            child = child.with_parent_id(parent_id);
        }

        info!(
            depth = child_depth,
            child = %child.id,
            %model,
            "spawning subagent"
        );

        let parent_call_id = call.id.to_string();
        let child_id = child.id.to_string();

        // Announce the child so the frontend can open a live tab even
        // before any tokens arrive. Ignore send errors — the WS forwarder
        // handles zero-subscriber gracefully; a broadcast with no
        // listeners just drops the frame.
        if let Some(tx) = &self.events_tx {
            let _ = tx.send(ServerMsg::SubagentStarted {
                parent_call_id: parent_call_id.clone(),
                agent_id: child_id.clone(),
                model: model.clone(),
                prompt: args.prompt.clone(),
            });
        }

        // Register the child with the parent's turn-cancel tracker so a
        // Stop button click cascades to every subagent still running.
        // Falls through gracefully (registration is Option-guarded) for
        // headless / test contexts that don't wire a tracker.
        let tracker_ticket = if let Some(tracker) = ctx.child_tracker.clone() {
            let cancel_hook: Box<dyn ChildCancel> = Box::new(SessionCancel(child.clone()));
            let id = tracker.register(cancel_hook).await;
            Some((tracker, id))
        } else {
            None
        };

        // Drain the child's harness stream to completion. Each event is
        // re-broadcast to the parent's WS with the parent's call_id
        // attached so the SubagentPanel builds a live per-child transcript
        // as tokens arrive. Warnings and tool-call counts are captured
        // locally so a truly empty run still returns something useful in
        // the tool result.
        let mut stream = child.send(args.prompt.clone()).await;
        let mut warnings: Vec<String> = Vec::new();
        let mut tool_call_count: usize = 0;
        while let Some(evt) = stream.next().await {
            // Broadcast side-effect happens before we consume/inspect the
            // event so token order is faithful to the child's own stream.
            if let Some(tx) = &self.events_tx {
                if let Some(wire) = subagent_wire(&parent_call_id, &evt) {
                    let _ = tx.send(wire);
                }
            }
            match evt {
                HarnessEvent::Done => break,
                HarnessEvent::Warning(w) => warnings.push(w),
                HarnessEvent::ToolStart(_) => tool_call_count += 1,
                // Token / ToolEnd / TurnComplete / Usage / MemoryLearned
                // are already forwarded above; nothing else to do locally.
                _ => {}
            }
        }

        // De-register from the parent's tracker — child is no longer
        // in-flight, so a subsequent parent-cancel shouldn't try to abort
        // an already-finished task.
        if let Some((tracker, id)) = tracker_ticket {
            tracker.deregister(id).await;
        }

        // Explicit done frame so the panel can flip its status pill
        // before the parent's ToolEnd finishes propagating.
        if let Some(tx) = &self.events_tx {
            let _ = tx.send(ServerMsg::SubagentDone {
                parent_call_id: parent_call_id.clone(),
            });
        }

        // Merge the isolated worktree back into the parent tree (if we
        // ever spun one up). Consumes the session — Drop would otherwise
        // tear down the worktree without harvesting anything. Warnings
        // for failed merges surface both to the parent's transcript
        // (so a human sees them) and to the tool result (so the model
        // reasons about them).
        let merge_summary = if let Some(w) = worktree.take() {
            let report = w.merge_and_cleanup();
            for err in &report.errors {
                warn!(parent_call_id = %parent_call_id, "worktree merge: {err}");
                if let Some(tx) = &self.events_tx {
                    let _ = tx.send(ServerMsg::SubagentWarning {
                        parent_call_id: parent_call_id.clone(),
                        text: format!("worktree merge: {err}"),
                    });
                }
            }
            report.short_summary()
        } else {
            None
        };

        // The tool result is the child's final assistant *text*. Walk
        // history back-to-front for the last assistant message with
        // non-empty content — critical because assistant messages
        // carrying only `tool_calls` (empty text) are common, and taking
        // the raw last-assistant message would surface an empty string
        // and look like the model produced nothing.
        let history = child.history().await;
        let final_text = history
            .iter()
            .rev()
            .find(|m| {
                m.role == Role::Assistant
                    && m.content
                        .as_deref()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
            })
            .and_then(|m| m.content.clone())
            .map(|s| s.trim().to_string());

        let body = match final_text {
            Some(text) => text,
            None => {
                // Fallback: describe what actually happened so the parent
                // can reason about it instead of getting a bare error.
                // Common causes: model looped on tool calls and hit
                // max_rounds, or a small model refused to produce text
                // after tool use.
                let mut out = String::from(
                    "Subagent finished without producing a text summary.",
                );
                out.push_str(&format!(
                    " It made {tool_call_count} tool call{}.",
                    if tool_call_count == 1 { "" } else { "s" }
                ));
                if !warnings.is_empty() {
                    out.push_str("\n\nWarnings from the child:\n");
                    for w in &warnings {
                        out.push_str("- ");
                        out.push_str(w);
                        out.push('\n');
                    }
                }
                if let Some(ref s) = merge_summary {
                    out.push_str("\n[worktree] ");
                    out.push_str(s);
                    out.push('\n');
                }
                out.push_str(
                    "\nRetry with a more direct prompt (\"summarize your \
                     findings in 5 bullets\") or run the investigation \
                     yourself.",
                );
                return Ok(ToolResult::err(call.id.clone(), out));
            }
        };

        // Prepend any warnings the child raised so the parent notices
        // e.g. "hit max_rounds" — but keep the final text as the primary
        // content so a healthy call is one clean summary. Same treatment
        // for the worktree merge summary when isolation was used: the
        // parent LLM sees exactly which files landed in its tree.
        let body_with_warnings = if warnings.is_empty() && merge_summary.is_none() {
            body
        } else {
            let mut out = String::new();
            for w in &warnings {
                out.push_str("[subagent warning] ");
                out.push_str(w);
                out.push('\n');
            }
            if let Some(ref s) = merge_summary {
                out.push_str("[worktree] ");
                out.push_str(s);
                out.push('\n');
            }
            out.push('\n');
            out.push_str(&body);
            out
        };

        // Human-in-the-loop review gate (Round 4). When the type has
        // `review_required: true`, block returning the summary to the
        // parent until the user approves via the prompt channel. Denial
        // becomes a tool-error whose body is the reviewer's note (so the
        // parent LLM can reason about the rejection). Approval with a
        // note prepends `[reviewer] <note>` to the summary. When no
        // channel is wired (headless / test), fail open with a warning
        // — otherwise the tool would hang the whole session waiting for
        // a UI that isn't there.
        let body_with_warnings = if type_def
            .and_then(|t| t.review_required)
            .unwrap_or(false)
        {
            if let Some(channel) = &self.channel {
                let prompt_id = format!("{}-review", parent_call_id);
                let msg = ServerMsg::SubagentReviewRequest {
                    parent_call_id: parent_call_id.clone(),
                    prompt_id: prompt_id.clone(),
                    summary: body_with_warnings.clone(),
                };
                info!(
                    parent_call_id = %parent_call_id,
                    "awaiting human review of subagent summary"
                );
                match channel.ask(prompt_id, msg).await {
                    Some(PromptResponse::SubagentReview(r)) if !r.approved => {
                        let note = r
                            .note
                            .filter(|n| !n.trim().is_empty())
                            .unwrap_or_else(|| "denied by reviewer".to_owned());
                        return Ok(ToolResult::err(
                            call.id.clone(),
                            format!("[reviewer denied] {note}"),
                        ));
                    }
                    Some(PromptResponse::SubagentReview(r)) => match r.note {
                        Some(n) if !n.trim().is_empty() => {
                            format!("[reviewer] {}\n\n{body_with_warnings}", n.trim())
                        }
                        _ => body_with_warnings,
                    },
                    Some(_) => {
                        warn!(
                            parent_call_id = %parent_call_id,
                            "review prompt returned wrong response kind; using summary as-is"
                        );
                        body_with_warnings
                    }
                    None => {
                        warn!(
                            parent_call_id = %parent_call_id,
                            "review prompt channel dropped; using summary as-is"
                        );
                        body_with_warnings
                    }
                }
            } else {
                warn!(
                    parent_call_id = %parent_call_id,
                    "review_required set but no prompt channel wired; using summary as-is"
                );
                body_with_warnings
            }
        } else {
            body_with_warnings
        };

        // Prepend a machine-readable marker with the child's session id
        // so the frontend can rehydrate the SubagentPanel after a
        // browser reload by fetching `/api/sessions/:id/history`. The
        // marker is stripped for display, but it's readable enough that
        // if a model *does* see it, it won't be confused about what it
        // means.
        let content = format!("[mira-agent-id:{child_id}]\n{body_with_warnings}");

        // When the type constrained the response via `response_schema`,
        // the model was forced to emit JSON — parse it now so the
        // parent gets structured data in `ToolResult.data` alongside
        // the raw text. Parse failures are non-fatal: the parent still
        // sees the text (provider may have relaxed strict mode). Only
        // parse the *body* (post-marker) so the marker doesn't corrupt
        // the JSON.
        let mut result = ToolResult::ok(call.id.clone(), content);
        if type_def.and_then(|t| t.response_schema.as_ref()).is_some() {
            match serde_json::from_str::<serde_json::Value>(body_with_warnings.trim()) {
                Ok(value) => {
                    result.data = Some(value);
                }
                Err(e) => {
                    warn!(
                        parent_call = %parent_call_id,
                        %e,
                        "subagent had a response_schema but final text isn't valid JSON; leaving data unset"
                    );
                }
            }
        }
        Ok(result)
    }
}

/// Translate one child `HarnessEvent` into the matching `Subagent*` wire
/// frame. Returns `None` for events we don't currently surface to the
/// panel (Usage / MemoryLearned / TurnComplete) — those exist for the
/// parent's status bar and would just add noise here.
fn subagent_wire(parent_call_id: &str, evt: &HarnessEvent) -> Option<ServerMsg> {
    let pid = parent_call_id.to_owned();
    match evt {
        HarnessEvent::Token(t) => Some(ServerMsg::SubagentToken {
            parent_call_id: pid,
            text: t.clone(),
        }),
        HarnessEvent::ToolStart(call) => Some(ServerMsg::SubagentToolStart {
            parent_call_id: pid,
            call: call.clone(),
        }),
        HarnessEvent::ToolEnd(result) => Some(ServerMsg::SubagentToolEnd {
            parent_call_id: pid,
            result: result.clone(),
        }),
        HarnessEvent::Warning(w) => Some(ServerMsg::SubagentWarning {
            parent_call_id: pid,
            text: w.clone(),
        }),
        HarnessEvent::Done => None, // Handled explicitly by caller.
        HarnessEvent::TurnComplete
        | HarnessEvent::Usage { .. }
        | HarnessEvent::MemoryLearned { .. }
        | HarnessEvent::Compacted { .. } => None,
    }
}

/// Names of tools known to be side-effect-free. Kept in sync with the
/// crate's read-only built-ins — extending this list is safe as long as
/// the tool truly makes no filesystem or shell writes.
const READ_ONLY_TOOL_NAMES: &[&str] = &[
    "read_file",
    "grep",
    "glob",
    "find_symbol",
    "find_references",
    "find_callers",
    "task_list",
    "task_get",
    "memory_read",
    "memory_search",
    "web_fetch",
    "web_search",
];

fn tools_are_read_only(names: Option<&[String]>) -> bool {
    names.map(tools_are_read_only_slice).unwrap_or(false)
}

fn tools_are_read_only_slice(names: &[String]) -> bool {
    !names.is_empty()
        && names
            .iter()
            .all(|n| READ_ONLY_TOOL_NAMES.contains(&n.as_str()))
}

/// Build the read-only agent fallback policy: `Auto` mode, no rules.
/// Under `AutoApprover`, this means Read/Pure/Write/Edit auto-allow and
/// Bash auto-approves — safe for `explore`/`reviewer` where the tools
/// list is already restricted upstream to read-only ones.
fn fresh_auto_policy() -> Result<Policy, ToolError> {
    let cfg = PolicyConfig {
        mode: mira_policy::Mode::Auto,
        allow: Vec::new(),
        ask: Vec::new(),
        deny: Vec::new(),
    };
    Policy::from_config(&cfg)
        .map_err(|e| ToolError::Failed(format!("subagent policy build failed: {e}")))
}

/// Decide whether this spawn should forward `Ask` decisions to the
/// parent's approver. Explicit type flag wins; otherwise fall back to
/// a conservative default: read-only tool sets auto-approve, anything
/// with write/edit/bash capability routes to the parent.
fn route_approvals_to_parent(
    ty: Option<&mira_agents::AgentType>,
    effective_tools: Option<&[String]>,
) -> bool {
    if let Some(t) = ty {
        if let Some(flag) = t.route_approvals_to_parent {
            return flag;
        }
    }
    match effective_tools {
        Some(names) => !tools_are_read_only_slice(names),
        // Full inheritance from the parent means the child could touch
        // anything — treat that as write-capable.
        None => true,
    }
}

/// Thin wrapper that lets a `Session` satisfy the `ChildCancel` contract
/// without mira-tools taking a dependency on the harness types. When
/// the parent's turn is interrupted, the tracker calls
/// `SessionCancel::cancel` on each registered child, which routes to
/// `Session::cancel` — same path a manual stop would take.
struct SessionCancel(Session);

#[async_trait]
impl ChildCancel for SessionCancel {
    async fn cancel(&self) {
        let _ = self.0.cancel().await;
    }
}

fn subagent_system_prompt() -> &'static str {
    "You are a Mira subagent — a bounded delegate spawned by a parent \
     agent to accomplish one specific task.\n\n\
     WORKFLOW (mandatory, in this order):\n\
     1. INVESTIGATE. Call your tools (grep / read_file / glob / find_symbol / \
        find_references / find_callers / bash) enough times to actually answer the task. Multiple tool calls \
        across multiple turns is normal and expected — a real research \
        question typically needs 5-15 tool calls before you have enough to \
        conclude. Do NOT produce your final summary until you have \
        gathered concrete evidence.\n\
     2. SYNTHESIZE. Once you've read the relevant files and confirmed the \
        answer, produce ONE final assistant message containing your \
        summary. That message is the only thing the parent sees.\n\n\
     Hard rules:\n\
     - Your very first turn should almost always be a tool call, not a \
       text answer. If you emit text on turn 1 without calling any tool, \
       you have failed the task.\n\
     - You have NO memory of the parent's conversation. The `prompt` you \
       received is the ONLY context you have.\n\
     - Do not ask clarifying questions — the parent isn't in the loop. \
       Make the best-effort inference and note assumptions in your summary.\n\
     - Prefer read-only exploration over edits when the task is a research \
       question.\n\
     - Keep the final summary tight. Bullet points, file paths, and short \
       findings beat paragraphs. Cite `path/to/file.rs:LINE` for every \
       concrete claim.\n\
     - If your type has a schema in its addendum, your FINAL message must \
       match it — but only your final message, not intermediate turns.\n\
     - When your task takes many tool calls (5+ turns of exploration or \
       edits), call the `progress` tool every few calls with a one-line \
       status. The parent and the user see those updates live in the \
       subagent panel — they're how you avoid looking stuck. Don't emit \
       progress on every turn; just at meaningful checkpoints \
       (\"found the auth flow, reading callers now\", \"finished the \
       migration, running tests\")."
}

/* ---------- progress tool (streaming intermediate summaries) ---------- */

/// Tool the subagent calls to yield a one-line status update to the
/// parent + the user. Emits a `SubagentProgress` frame on the events
/// channel; the frontend renders these as chips in the SubagentPanel
/// so a long delegation doesn't feel opaque.
///
/// Constructed per-spawn inside `AgentTool::invoke` with the parent's
/// events channel + the parent's tool-call id baked in. The subagent's
/// registry gets this instance directly so the model has a first-class
/// tool call (not some out-of-band side channel) to signal progress.
pub struct ProgressTool {
    events_tx: broadcast::Sender<ServerMsg>,
    parent_call_id: String,
}

impl ProgressTool {
    pub fn new(events_tx: broadcast::Sender<ServerMsg>, parent_call_id: String) -> Self {
        Self {
            events_tx,
            parent_call_id,
        }
    }
}

#[derive(Deserialize)]
struct ProgressArgs {
    /// One-line status. Model-facing description spells out the shape.
    text: String,
}

#[async_trait]
impl Tool for ProgressTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "progress",
            "Emit an intermediate status update visible to the parent and \
             the user while you keep working. Use ONCE every few tool \
             calls during long investigations or multi-file edits so the \
             parent isn't blind while you're exploring. Keep it to one \
             sentence: what you just learned or what you're doing next. \
             Do NOT emit progress on every turn — noise is worse than \
             silence.",
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "One-line status. Good: 'Found 3 files touching WsApprover; reading main.rs now', 'Migration applied, running tests'. Bad: essays, verbatim tool output, repeated messages."
                    }
                },
                "required": ["text"]
            }),
        )
    }

    fn action(&self) -> Action {
        // No side effects on the filesystem or shell — pure signalling.
        Action::Pure
    }

    fn parallel_safe(&self, _call: &ToolCall) -> bool {
        true
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: ProgressArgs = call.parse_arguments()?;
        let text = args.text.trim().to_owned();
        if text.is_empty() {
            // Nothing to broadcast — still succeed so the model doesn't
            // get error-looped over an empty string.
            return Ok(ToolResult::ok(call.id.clone(), "noted (empty)"));
        }
        // Fire-and-forget: a zero-subscriber broadcast just drops the
        // frame, which is fine for headless / test contexts.
        let _ = self.events_tx.send(ServerMsg::SubagentProgress {
            parent_call_id: self.parent_call_id.clone(),
            text: text.clone(),
        });
        // Return an ack the model can key off. Keep it terse — the model
        // shouldn't be reading progress-tool results as instructions.
        Ok(ToolResult::ok(call.id.clone(), "progress noted"))
    }
}
