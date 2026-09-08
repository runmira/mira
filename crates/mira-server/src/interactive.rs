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
use mira_ai::{ChatProvider, ToolSpec};
use mira_core::{Role, ToolCall, ToolResult};
use mira_harness::{AutoApprover, HarnessEvent, Session, SessionConfig, SessionStore};
use mira_policy::{Policy, PolicyConfig};
use mira_tools::context::ToolContext;
use mira_tools::tool::{spec, Action, Tool, ToolError};
use mira_tools::Registry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{info, warn};

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
        }
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
                "Optional named type. Available types:\n{roster}\n\nWhen set, \
                 defaults (tools/model/prompt) come from the type; explicit \
                 args below still override.",
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

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: AgentArgs = call.parse_arguments()?;

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
            child_registry.register(nested);
        }
        let child_registry = Arc::new(child_registry);

        // Fresh policy: subagents run under `auto` with an AutoApprover so
        // there's no interactive prompt on the child's turn. If you need
        // manual gating for a delegated task, don't delegate it. Rules are
        // empty — same policy for every subagent in MVP.
        let policy_cfg = PolicyConfig {
            mode: mira_policy::Mode::Auto,
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
        };
        let policy = Policy::from_config(&policy_cfg)
            .map_err(|e| ToolError::Failed(format!("subagent policy build failed: {e}")))?;
        let policy = Arc::new(Mutex::new(policy));
        let approver = Arc::new(AutoApprover { approve_asks: true });

        // Fresh ToolContext — sandbox + cwd shared with the parent so
        // edits land on the same working tree; guard is intentionally
        // omitted here because Session::new attaches a session-scoped one
        // itself. Memory + episodic handles carry through so the child
        // can read the same MIRA.md the parent sees.
        let child_ctx = ToolContext::new(ctx.cwd.clone(), ctx.sandbox.clone())
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

        // Explicit done frame so the panel can flip its status pill
        // before the parent's ToolEnd finishes propagating.
        if let Some(tx) = &self.events_tx {
            let _ = tx.send(ServerMsg::SubagentDone {
                parent_call_id: parent_call_id.clone(),
            });
        }

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
        // content so a healthy call is one clean summary.
        let body_with_warnings = if warnings.is_empty() {
            body
        } else {
            let mut out = String::new();
            for w in &warnings {
                out.push_str("[subagent warning] ");
                out.push_str(w);
                out.push('\n');
            }
            out.push('\n');
            out.push_str(&body);
            out
        };

        // Prepend a machine-readable marker with the child's session id
        // so the frontend can rehydrate the SubagentPanel after a
        // browser reload by fetching `/api/sessions/:id/history`. The
        // marker is stripped for display, but it's readable enough that
        // if a model *does* see it, it won't be confused about what it
        // means.
        let content = format!("[mira-agent-id:{child_id}]\n{body_with_warnings}");

        Ok(ToolResult::ok(call.id.clone(), content))
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
        | HarnessEvent::MemoryLearned { .. } => None,
    }
}

fn subagent_system_prompt() -> &'static str {
    "You are a Mira subagent — a bounded delegate spawned by a parent \
     agent to accomplish one specific task.\n\n\
     Rules:\n\
     - You have NO memory of the parent's conversation. The `prompt` you \
       received is the ONLY context you have.\n\
     - Do the task, then return ONE clear plain-text summary as your \
       final message. That summary is the only thing the parent will see.\n\
     - Do not ask clarifying questions — the parent isn't in the loop. \
       Make the best-effort inference and note assumptions in your summary.\n\
     - You may use the tools you were given. Prefer read-only exploration \
       over edits when the task is a research question.\n\
     - Keep the summary tight. Bullet points, file paths, and short \
       findings beat paragraphs."
}
