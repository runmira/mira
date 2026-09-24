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

/* ---------- shared re-exports ---------- */

// The plan/ask payloads and the tools themselves now live in
// `mira_tools::prompt` so the TUI can drive the same tools over its own
// channel. Re-exported here so every server path (`ServerMsg`, WS
// routing, the web UI) keeps compiling unchanged.
pub use mira_tools::prompt::{
    AskUserAnswer, AskUserOption, AskUserProposal, AskUserQuestion, AskUserResponse, AskUserTool,
    PlanProposal, PlanResponse, PlanStep, PlanTool, PromptRequest, PromptResponse,
    SubagentReviewResponse,
};

// The concrete broadcast channel implements the shared trait by
// translating the UI-agnostic request into a `ServerMsg` frame. The
// inherent `ask(…, ServerMsg)` below remains for the subagent-review
// path, which is a server-only shape.
#[async_trait]
impl mira_tools::prompt::PromptChannel for PromptChannel {
    async fn ask(
        &self,
        prompt_id: String,
        request: mira_tools::prompt::PromptRequest,
    ) -> Option<PromptResponse> {
        let msg = match request {
            mira_tools::prompt::PromptRequest::Plan(plan) => ServerMsg::PlanRequest {
                prompt_id: prompt_id.clone(),
                plan,
            },
            mira_tools::prompt::PromptRequest::AskUser(proposal) => ServerMsg::AskUserRequest {
                prompt_id: prompt_id.clone(),
                proposal,
            },
        };
        self.ask(prompt_id, msg).await
    }
}

/* ---------- agent tool (subagents) ---------- */

/// Hard cap on nested subagent spawning. Depth 0 is the user's top-level
/// chat; the `agent` tool at depth N produces a child at depth N+1. When a
/// child's ambient depth already equals this cap, its own `agent` tool
/// refuses to spawn — the model can't accidentally recurse forever.
const MAX_AGENT_DEPTH: usize = 2;

/// Per-child ceiling on rounds when the caller didn't pin `max_rounds`.
/// Held at the same generous default as top-level sessions (200) — the
/// prior tighter cap (30) was routinely hit by exploratory work that
/// wasn't actually looping, just doing legit multi-step research. If a
/// subagent genuinely deserves a lower ceiling, its agent-type `.md`
/// still overrides via its own `max_rounds` frontmatter.
const DEFAULT_SUBAGENT_MAX_ROUNDS: usize = 200;

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
    /// Shared per-parent-session scratchpad. Each entry is one note a
    /// subagent posted via `scratchpad_note`; peers spawned from the
    /// same session see it either via a header block prepended to their
    /// initial prompt (spawn-time snapshot) or via `scratchpad_read`
    /// (live poll from within a child turn). Keyed by the parent's
    /// session id so a sibling running in parallel sees the same pad
    /// but two independent sessions stay isolated.
    ///
    /// In-memory only — a process restart wipes the pad. That's the
    /// right call for a *mid-flight coordination* buffer: the notes
    /// describe transient state ("I already grepped auth.rs, nothing
    /// there") that's rarely useful in a later session. If we later
    /// want durability we can serialize entries alongside the parent
    /// SessionRecord.
    scratchpads: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
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
            scratchpads: Arc::new(Mutex::new(HashMap::new())),
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
        // Advertised enum. Always include the sentinel `auto` so the
        // LLM-as-router path is reachable through strict-schema providers
        // (OpenAI structured tools, Anthropic in strict mode) — without
        // this, providers reject the arg before `invoke` ever runs, even
        // though the description mentions `auto` as a valid value.
        let mut type_enum = type_names.clone();
        if !type_enum.iter().any(|n| n == "auto") {
            type_enum.push("auto".to_owned());
        }
        let type_description = if type_names.is_empty() {
            "Named agent type. No types are currently registered — pass \
             `auto` to let the router decide, or omit this field."
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
                        "enum": type_enum,
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
            list.iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>()
        });
        for tool in self.base_registry.tools() {
            // Desktop and browser control stay with the top-level session,
            // where the user is watching and approving each step. A
            // subagent runs on its own policy and must not inherit them.
            if matches!(
                tool.action(),
                mira_tools::Action::Computer | mira_tools::Action::Browser
            ) {
                continue;
            }
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
        // Cross-subagent shared scratchpad: keyed by the parent's session
        // id so peers spawned from the same session share one pad, but
        // two independent sessions stay isolated. When there's no parent
        // session id in context (headless / one-shot tests) we skip
        // wiring entirely — the pad becomes a no-op instead of leaking
        // notes across unrelated calls under a synthetic key.
        let parent_session_id = ctx.session_id.as_ref().map(|s| s.to_string());
        if let Some(pid) = &parent_session_id {
            let author = scratchpad_author(type_def);
            child_registry.register(ScratchpadNoteTool::new(
                self.scratchpads.clone(),
                pid.clone(),
                call.id.to_string(),
                author,
                self.events_tx.clone(),
            ));
            child_registry.register(ScratchpadReadTool::new(
                self.scratchpads.clone(),
                pid.clone(),
            ));
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
        // Pull the parent's explicit deny rules so a read-only subagent
        // spawned on a fresh Auto policy still refuses whatever the user
        // told the parent to refuse (e.g. `Deny(Read("**/.env"))`).
        // Without this the parent's secrets are readable by any spawned
        // `explore`/`reviewer` agent — an approval-UX-bypasses-containment
        // hole the audit called out under Gap #1.
        let inherited_denies: Vec<String> = match &self.parent_policy {
            Some(p) => p.lock().await.deny_source().to_vec(),
            None => Vec::new(),
        };
        let (policy, approver): (Arc<Mutex<Policy>>, Arc<dyn mira_harness::Approver>) =
            if route_to_parent {
                let policy = match &self.parent_policy {
                    Some(p) => p.clone(),
                    None => {
                        warn!(
                            depth = child_depth,
                            "subagent routes to parent but no parent policy wired; using fresh Auto"
                        );
                        Arc::new(Mutex::new(fresh_auto_policy(&inherited_denies)?))
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
                let policy = Arc::new(Mutex::new(fresh_auto_policy(&inherited_denies)?));
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
            let type_name = type_def.map(|t| t.name.as_str()).unwrap_or("agent");
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
        let child_ctx =
            ToolContext::new(child_cwd, ctx.sandbox.clone()).with_agent_depth(child_depth);
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

        // Prepend a snapshot of the shared scratchpad to the child's
        // opening message so a spawn that fires *after* peers have
        // posted notes sees them from turn 1 — waiting for the model to
        // remember to call `scratchpad_read` would defeat the point on
        // short-lived children. When the pad is empty (first spawn of
        // the session, or nobody has posted yet), we pass the prompt
        // through unchanged so the initial context stays lean.
        let prompt_with_pad = if let Some(pid) = &parent_session_id {
            let snapshot = self.scratchpads.lock().await.get(pid).cloned();
            match snapshot.as_deref().and_then(format_scratchpad_block) {
                Some(block) => format!("{block}{}", args.prompt),
                None => args.prompt.clone(),
            }
        } else {
            args.prompt.clone()
        };

        // Drain the child's harness stream to completion. Each event is
        // re-broadcast to the parent's WS with the parent's call_id
        // attached so the SubagentPanel builds a live per-child transcript
        // as tokens arrive. Warnings and tool-call counts are captured
        // locally so a truly empty run still returns something useful in
        // the tool result.
        let mut stream = child.send(prompt_with_pad).await;
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
                let mut out = String::from("Subagent finished without producing a text summary.");
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
        let body_with_warnings = if type_def.and_then(|t| t.review_required).unwrap_or(false) {
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
        | HarnessEvent::Compacted { .. }
        | HarnessEvent::GoalSet { .. }
        | HarnessEvent::GoalCleared
        | HarnessEvent::GoalProgress { .. }
        | HarnessEvent::GoalDone { .. }
        | HarnessEvent::ToolProgress { .. }
        | HarnessEvent::ToolPreview { .. } => None,
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

/// Build the read-only agent fallback policy: `Auto` mode, no allow/ask
/// rules, and the parent's explicit deny list carried through so denies
/// like `Deny(Read("**/.env"))` still apply to spawned children. Under
/// `AutoApprover` this means Read/Pure/Write/Edit auto-allow and Bash
/// auto-approves — safe for `explore`/`reviewer` where the tools list is
/// already restricted upstream to read-only ones.
fn fresh_auto_policy(inherited_denies: &[String]) -> Result<Policy, ToolError> {
    let cfg = PolicyConfig {
        mode: mira_policy::Mode::Auto,
        allow: Vec::new(),
        ask: Vec::new(),
        deny: inherited_denies.to_vec(),
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
       migration, running tests\").\n\
     - Peer subagents may be running in parallel on the same session. \
       They share a scratchpad you can read with `scratchpad_read` and \
       append to with `scratchpad_note`. Use it for coordination: post \
       one-line findings a sibling would want (\"nothing under \
       crates/mira-tools\", \"auth flow is in src/auth/session.rs\") and \
       read it before diving into work that overlaps with a peer. Do \
       NOT put your final summary on the scratchpad — that still comes \
       back to the parent as your final assistant message."
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

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
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

/* ---------- shared scratchpad (cross-subagent findings) ---------- */

/// One note on the shared scratchpad. Peers spawned from the same
/// parent session read these to avoid duplicating each other's work.
/// `author` is the child's type name (or `agent` when no type was
/// pinned) so a reader can attribute findings without guessing.
#[derive(Clone, Debug, Serialize)]
pub struct ScratchpadEntry {
    /// Type name of the subagent that wrote the note (e.g. `explore`).
    pub author: String,
    /// Seconds since UNIX epoch. Best-effort — a system clock going
    /// backwards produces 0, which is fine for ordering (entries are
    /// vec-ordered anyway; this is provenance).
    pub ts: u64,
    /// The note itself. Trimmed at write time; enforced to be
    /// non-empty (empty writes are dropped silently by the tool).
    pub text: String,
}

/// Hard cap on scratchpad size to keep prompt injection bounded. When
/// the pad exceeds this many entries, the oldest is dropped on write —
/// so a runaway note-happy child can't blow the child prompt on peers
/// spawned later. 64 is generous for coordinating a handful of parallel
/// researchers without silently choking the injection block.
const SCRATCHPAD_MAX_ENTRIES: usize = 64;

/// Hard cap on a single note. Notes are meant to be one-liners; a
/// verbose write is either a wrong tool choice (should be the final
/// summary) or an accident. Truncating with an ellipsis keeps the
/// pad legible without failing the write.
const SCRATCHPAD_MAX_NOTE_BYTES: usize = 2048;

/// Return an "author" label for scratchpad entries emitted by this
/// subagent. Falls back to `agent` when no type is pinned so the
/// column stays populated even for raw calls.
fn scratchpad_author(type_def: Option<&mira_agents::AgentType>) -> String {
    type_def
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "agent".to_owned())
}

/// Format the current pad as a Markdown block suitable for prepending
/// to a child's initial user prompt. Returns `None` when the pad is
/// empty — the caller should skip injection entirely rather than emit
/// an empty header.
fn format_scratchpad_block(entries: &[ScratchpadEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Shared notes from peer subagents\n\n\
         These notes were left by other subagents working on this session. \
         Use them to avoid duplicating their work; treat them as advisory, \
         not authoritative. You can add your own with `scratchpad_note` \
         and read the latest with `scratchpad_read`.\n\n",
    );
    for e in entries {
        out.push_str("- ");
        out.push_str(&e.author);
        out.push_str(": ");
        out.push_str(&e.text);
        out.push('\n');
    }
    out.push_str("\n---\n\n");
    Some(out)
}

/// Tool the subagent calls to append a one-line finding to the pad
/// shared with its peers. Bound at construction time to the parent's
/// session id so the note lands in the right pad even when the child
/// itself has a distinct session id.
pub struct ScratchpadNoteTool {
    store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
    parent_session_id: String,
    parent_call_id: String,
    author: String,
    events_tx: Option<broadcast::Sender<ServerMsg>>,
}

impl ScratchpadNoteTool {
    pub fn new(
        store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
        parent_session_id: String,
        parent_call_id: String,
        author: String,
        events_tx: Option<broadcast::Sender<ServerMsg>>,
    ) -> Self {
        Self {
            store,
            parent_session_id,
            parent_call_id,
            author,
            events_tx,
        }
    }
}

#[derive(Deserialize)]
struct ScratchpadNoteArgs {
    text: String,
}

#[async_trait]
impl Tool for ScratchpadNoteTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "scratchpad_note",
            "Post a one-line finding to the shared scratchpad visible to \
             every peer subagent in this session. Use this when you \
             discover something a sibling searcher would want to know \
             (\"nothing relevant under crates/mira-tools\", \"auth flow \
             lives in src/auth/session.rs\") so parallel work doesn't \
             duplicate yours. Notes are advisory, not final answers — \
             keep them to one sentence. Your FINAL summary still goes \
             back to the parent as your assistant text, not here.",
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "One-line finding. Good: 'grepped for WsApprover in crates/, only mira-server/lib.rs hits'. Bad: essays, verbatim tool output, the final summary."
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // No filesystem/shell side effects — pure signalling to peers.
        Action::Pure
    }

    fn parallel_safe(&self, _call: &ToolCall) -> bool {
        true
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ScratchpadNoteArgs = call.parse_arguments()?;
        let mut text = args.text.trim().to_owned();
        if text.is_empty() {
            return Ok(ToolResult::ok(call.id.clone(), "noted (empty)"));
        }
        if text.len() > SCRATCHPAD_MAX_NOTE_BYTES {
            // Truncate at char boundary — a naive slice can panic on
            // multi-byte codepoints. Then tack on an ellipsis so the
            // reader can tell it was cut.
            let mut cut = SCRATCHPAD_MAX_NOTE_BYTES;
            while !text.is_char_boundary(cut) && cut > 0 {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        let entry = ScratchpadEntry {
            author: self.author.clone(),
            ts: now_secs(),
            text: text.clone(),
        };
        {
            let mut guard = self.store.lock().await;
            let pad = guard
                .entry(self.parent_session_id.clone())
                .or_insert_with(Vec::new);
            pad.push(entry.clone());
            // Bound the pad. Drops from the front so the newest are
            // always kept — the assumption is that later findings
            // supersede earlier ones in a well-behaved run.
            while pad.len() > SCRATCHPAD_MAX_ENTRIES {
                pad.remove(0);
            }
        }
        if let Some(tx) = &self.events_tx {
            let _ = tx.send(ServerMsg::SubagentScratchpadNote {
                parent_call_id: self.parent_call_id.clone(),
                parent_session_id: self.parent_session_id.clone(),
                entry: entry.clone(),
            });
        }
        Ok(ToolResult::ok(call.id.clone(), "note posted"))
    }
}

/// Tool the subagent calls to read the current scratchpad — the
/// live view, not the spawn-time snapshot. Useful when a child has
/// been running long enough that a peer may have added notes after
/// spawn.
pub struct ScratchpadReadTool {
    store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
    parent_session_id: String,
}

impl ScratchpadReadTool {
    pub fn new(
        store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>>,
        parent_session_id: String,
    ) -> Self {
        Self {
            store,
            parent_session_id,
        }
    }
}

#[async_trait]
impl Tool for ScratchpadReadTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "scratchpad_read",
            "Read the current shared scratchpad — every note peers have \
             posted so far this session. Prefer this over guessing what \
             siblings are doing when your task overlaps with theirs. \
             Returns each note as `<author>: <text>`; empty when nobody \
             has posted yet.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    fn parallel_safe(&self, _call: &ToolCall) -> bool {
        true
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let guard = self.store.lock().await;
        let body = match guard.get(&self.parent_session_id) {
            None => String::from("(scratchpad empty)"),
            Some(pad) if pad.is_empty() => String::from("(scratchpad empty)"),
            Some(pad) => {
                let mut out = String::new();
                for e in pad {
                    out.push_str("- ");
                    out.push_str(&e.author);
                    out.push_str(": ");
                    out.push_str(&e.text);
                    out.push('\n');
                }
                out
            }
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_ai::NullProvider;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{ToolCall, ToolCallId};
    use mira_sandbox::Sandbox;
    use std::path::PathBuf;

    fn make_agent_tool_with_types(names: &[&str]) -> AgentTool {
        let mut reg = AgentRegistry::default();
        for name in names {
            reg.types.insert(
                (*name).to_owned(),
                mira_agents::AgentType {
                    name: (*name).to_owned(),
                    description: format!("{name} test type"),
                    category: None,
                    tools: None,
                    model: None,
                    max_rounds: None,
                    system_prompt_addendum: None,
                    response_schema: None,
                    parallel_safe: None,
                    route_approvals_to_parent: None,
                    worktree: None,
                    extends: None,
                    review_required: None,
                },
            );
        }
        AgentTool::new(
            Arc::new(NullProvider::default()),
            Arc::new(Registry::new()),
            "test-model".to_owned(),
        )
        .with_agents(Arc::new(reg))
    }

    #[test]
    fn spec_type_enum_always_includes_auto() {
        // With registered types, `auto` must appear alongside them so a
        // strict-schema provider accepts `type: "auto"`.
        let tool = make_agent_tool_with_types(&["explore", "reviewer"]);
        let spec = tool.spec();
        let enum_vals: Vec<String> = spec.parameters["properties"]["type"]["enum"]
            .as_array()
            .expect("type.enum should be an array")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(
            enum_vals.contains(&"auto".to_owned()),
            "auto must be in the enum, got: {enum_vals:?}"
        );
        assert!(enum_vals.contains(&"explore".to_owned()));
        assert!(enum_vals.contains(&"reviewer".to_owned()));
    }

    #[test]
    fn spec_type_enum_includes_auto_when_registry_empty() {
        // Even with no registered types, `auto` should be advertised so
        // the router is reachable. (Empty type_names + no auto would
        // leave the enum empty and providers reject any value.)
        let tool = make_agent_tool_with_types(&[]);
        let spec = tool.spec();
        let enum_vals: Vec<String> = spec.parameters["properties"]["type"]["enum"]
            .as_array()
            .expect("type.enum should be an array")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(enum_vals, vec!["auto".to_owned()]);
    }

    #[test]
    fn format_scratchpad_block_returns_none_when_empty() {
        assert!(format_scratchpad_block(&[]).is_none());
    }

    #[test]
    fn format_scratchpad_block_renders_entries() {
        let entries = vec![
            ScratchpadEntry {
                author: "explore".to_owned(),
                ts: 0,
                text: "found auth flow in src/auth/session.rs".to_owned(),
            },
            ScratchpadEntry {
                author: "cartographer".to_owned(),
                ts: 0,
                text: "no callers in mira-tools".to_owned(),
            },
        ];
        let block = format_scratchpad_block(&entries).expect("non-empty");
        assert!(block.contains("Shared notes from peer subagents"));
        assert!(block.contains("explore: found auth flow"));
        assert!(block.contains("cartographer: no callers"));
        // Must end with the separator so the child's own prompt reads
        // as a distinct section.
        assert!(block.trim_end().ends_with("---"));
    }

    fn make_tool_call(args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::new(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "scratchpad_note".to_owned(),
                arguments: args.to_string(),
            },
        }
    }

    fn dummy_ctx() -> ToolContext {
        ToolContext::new(
            PathBuf::from("."),
            Arc::new(Sandbox::new(PathBuf::from("."))),
        )
    }

    #[tokio::test]
    async fn scratchpad_note_and_read_roundtrip() {
        let store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let key = "sess_a".to_owned();
        let note = ScratchpadNoteTool::new(
            store.clone(),
            key.clone(),
            "call_1".to_owned(),
            "explore".to_owned(),
            None,
        );
        let ctx = dummy_ctx();
        let call = make_tool_call(json!({ "text": "found the auth flow" }));
        note.invoke(&call, &ctx).await.expect("note ok");

        let read = ScratchpadReadTool::new(store.clone(), key.clone());
        let call = make_tool_call(json!({}));
        let result = read.invoke(&call, &ctx).await.expect("read ok");
        assert!(result.content.contains("explore: found the auth flow"));
    }

    #[tokio::test]
    async fn scratchpads_are_isolated_by_session_id() {
        let store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Writer bound to sess_a.
        let note = ScratchpadNoteTool::new(
            store.clone(),
            "sess_a".to_owned(),
            "call_1".to_owned(),
            "explore".to_owned(),
            None,
        );
        let ctx = dummy_ctx();
        note.invoke(&make_tool_call(json!({ "text": "note for A" })), &ctx)
            .await
            .expect("note ok");

        // Reader bound to sess_b — must not see A's notes.
        let read_b = ScratchpadReadTool::new(store.clone(), "sess_b".to_owned());
        let result = read_b
            .invoke(&make_tool_call(json!({})), &ctx)
            .await
            .expect("read ok");
        assert!(
            result.content.contains("empty"),
            "sess_b should be empty, got: {}",
            result.content
        );

        // Reader bound to sess_a — sees its own notes.
        let read_a = ScratchpadReadTool::new(store.clone(), "sess_a".to_owned());
        let result = read_a
            .invoke(&make_tool_call(json!({})), &ctx)
            .await
            .expect("read ok");
        assert!(result.content.contains("note for A"));
    }

    #[tokio::test]
    async fn scratchpad_enforces_entry_cap() {
        let store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let key = "sess_cap".to_owned();
        let note = ScratchpadNoteTool::new(
            store.clone(),
            key.clone(),
            "call_1".to_owned(),
            "explore".to_owned(),
            None,
        );
        let ctx = dummy_ctx();
        for i in 0..(SCRATCHPAD_MAX_ENTRIES + 5) {
            note.invoke(
                &make_tool_call(json!({ "text": format!("note {i}") })),
                &ctx,
            )
            .await
            .expect("note ok");
        }
        let pad = store.lock().await.get(&key).cloned().unwrap_or_default();
        assert_eq!(pad.len(), SCRATCHPAD_MAX_ENTRIES, "cap should hold");
        // Front should have been dropped, so the first surviving entry
        // is note 5 (indices 0..4 are gone).
        assert!(pad.first().unwrap().text.starts_with("note 5"));
        assert!(pad
            .last()
            .unwrap()
            .text
            .starts_with(&format!("note {}", SCRATCHPAD_MAX_ENTRIES + 4)));
    }

    #[tokio::test]
    async fn scratchpad_note_truncates_oversized_text() {
        let store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let key = "sess_trunc".to_owned();
        let note = ScratchpadNoteTool::new(
            store.clone(),
            key.clone(),
            "call_1".to_owned(),
            "explore".to_owned(),
            None,
        );
        let ctx = dummy_ctx();
        let big = "x".repeat(SCRATCHPAD_MAX_NOTE_BYTES * 2);
        note.invoke(&make_tool_call(json!({ "text": big })), &ctx)
            .await
            .expect("note ok");
        let pad = store.lock().await.get(&key).cloned().unwrap_or_default();
        let stored = &pad[0].text;
        // Stored text: at most cap bytes of `x` plus the ellipsis marker.
        assert!(stored.ends_with('…'));
        assert!(
            stored.len() <= SCRATCHPAD_MAX_NOTE_BYTES + '…'.len_utf8(),
            "stored len {} should be within cap+ellipsis",
            stored.len()
        );
    }

    #[tokio::test]
    async fn scratchpad_note_ignores_empty_text() {
        let store: Arc<Mutex<HashMap<String, Vec<ScratchpadEntry>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let note = ScratchpadNoteTool::new(
            store.clone(),
            "sess_empty".to_owned(),
            "call_1".to_owned(),
            "explore".to_owned(),
            None,
        );
        let ctx = dummy_ctx();
        note.invoke(&make_tool_call(json!({ "text": "   " })), &ctx)
            .await
            .expect("note ok");
        assert!(store.lock().await.get("sess_empty").is_none());
    }
}
