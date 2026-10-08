//! Driving an agent through its own CLI instead of through an ACP adapter.
//!
//! Several coding agents ship a headless, machine-readable mode in the binary
//! the user already has. Claude Code is the clearest case: `claude -p
//! --input-format stream-json --output-format stream-json` is a full agent
//! server — session ids, streaming tool calls, permission round-trips,
//! cancellation and resume — and it needs no second install. The ACP adapter
//! is a third-party shim over exactly this, not a prerequisite.
//!
//! Why bother, when ACP already works:
//!
//! * Detection. Probing `claude-agent-acp` told a user who runs Claude Code
//!   every day that Claude Code was not installed. Probing the agent's own
//!   CLI cannot get that wrong.
//! * Setup. The adapter is a separate global npm install that has to be
//!   version-matched to the CLI.
//! * Reach. Every agent that has this interface can be driven without anyone
//!   maintaining an adapter for it.
//!
//! The cost is a second transport. Everything downstream of it is shared: the
//! native reader emits the same [`NormalizedEvent`]s the ACP path emits, and
//! the same permission gate answers them, so the server, the transcript and
//! the UI are unchanged either way.
//!
//! # What is modelled, and what is not
//!
//! The line protocol is Claude Code's `stream-json`, verified against the
//! installed CLI. Envelopes seen: `system`/`init`, `assistant`, `user`,
//! `result`, `rate_limit_event`. Anything else becomes
//! [`MiraEvent::Unmodelled`] rather than being dropped.
//!
//! The permission round-trip is implemented from the documented control
//! protocol (`control_request`/`control_response`) and is covered by tests
//! against a fake CLI. It has not yet been observed from a real agent turn,
//! because a real turn could not be run while writing this.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

use crate::driver::{LaunchConfig, PermissionMode, Transport};
use crate::events::{
    AcpToolCallStatus as ToolCallStatus, AcpToolKind as ToolKind, ConfigValueView, EventSource,
    LimitWindow, MiraEvent, NormalizedEvent, SessionConfigView, SessionModeView, ToolCallState,
    ToolContent,
};

/// How long to wait for a line before assuming the CLI has gone quiet.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// Which agent's native line protocol we speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeFlavor {
    /// Claude Code's `stream-json`.
    Claude,
    /// Codex's `codex app-server` (JSON-RPC over stdio).
    ///
    /// Implemented in [`crate::appserver`]. This variant exists so drivers
    /// can name the transport; the argv and the session live there, not
    /// here. Like everything Codex-shaped, UNVERIFIED against a real binary
    /// — Codex is not installed on the machine this was written on.
    CodexAppServer,
}

/// A permission the agent is blocked on, in the shape the CLI reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePermission {
    /// The CLI's own id, echoed back in our response.
    pub request_id: String,
    pub tool_name: String,
    /// The tool input, verbatim. Shown to the user so the decision is
    /// informed rather than a blind allow/deny.
    pub input: Value,
    /// The tool call this request gates, when the CLI names it — lets the
    /// UI tie the prompt to the tool card it belongs to.
    pub tool_use_id: Option<String>,
    /// Why the CLI is asking ("Contains simple_expansion"), when it says.
    pub reason: Option<String>,
    /// Permission rules the CLI proposes for "don't ask again" — reused,
    /// rescoped, when the user allows for the session.
    pub suggestions: Vec<Value>,
}

/// How a permission was decided.
///
/// A bare allow/deny is not enough for Claude Code: "allow for this
/// session" is a set of permission rules the CLI applies itself, and
/// answering a question (`AskUserQuestion`) or approving a plan
/// (`ExitPlanMode`) is an allow that carries the user's answer back as the
/// tool's rewritten input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PermissionDecision {
    pub allow: bool,
    /// Replaces the tool input on allow. `None` echoes the original.
    pub updated_input: Option<Value>,
    /// Rules for the CLI to adopt alongside this allow.
    pub updated_permissions: Vec<Value>,
    /// Told to the agent on deny, so it can adjust rather than retry blind.
    pub message: Option<String>,
}

impl From<bool> for PermissionDecision {
    fn from(allow: bool) -> Self {
        PermissionDecision {
            allow,
            ..Default::default()
        }
    }
}

/// Resolves a permission.
pub type PermissionGate = Arc<
    dyn Fn(NativePermission) -> Pin<Box<dyn Future<Output = PermissionDecision> + Send>>
        + Send
        + Sync,
>;

/// What the reader should do with one line.
#[derive(Debug, PartialEq)]
pub enum NativeAction {
    /// Record the session id the CLI allocated.
    SessionId(String),
    /// Forward to the transcript.
    Event(NormalizedEvent),
    /// The agent is blocked on a permission; resolve it and answer.
    Permission(NativePermission),
    /// The turn ended. Carries the CLI's own stop label.
    TurnEnded { stop_reason: String, is_error: bool },
    /// A rate limit, which is a real outcome and not an error to hide.
    RateLimited { resets_at: Option<String> },
}

/// Map one decoded line to actions.
///
/// Pure and total: every input yields at least one action or nothing, and
/// unknown shapes become `Unmodelled` rather than vanishing. That property is
/// what lets this be tested without spawning anything.
pub fn map_line(v: &Value) -> Vec<NativeAction> {
    map_line_with(v, None)
}

/// Map one decoded line to actions, with an optional model list.
///
/// `models` is the agent CLI's own known models (see [`claude_models`]).
/// Without it, an `init` can only report the single running model, and the
/// picker shows one entry. With it, the `config_options` for the model
/// carries the full list with the running one marked current — the same
/// information the CLI's own `/model` menu shows.
/// The model option event, with the running model first and marked current.
/// `current` is `None` before the first turn — the CLI has said nothing yet —
/// in which case the list is unmarked rather than guessed.
fn model_config_event(
    current: Option<&str>,
    known: Option<&[NativeModel]>,
) -> Option<NormalizedEvent> {
    let mut values = match current {
        Some(model) => vec![ConfigValueView {
            value: model.to_string(),
            name: model.to_string(),
            description: None,
        }],
        None => Vec::new(),
    };
    if let Some(known) = known {
        for m in known {
            if Some(m.value.as_str()) != current && !values.iter().any(|e| e.value == m.value) {
                values.push(ConfigValueView {
                    value: m.value.clone(),
                    name: m.label.clone(),
                    description: m.description.clone(),
                });
            }
        }
    }
    if values.is_empty() {
        return None;
    }
    Some(NormalizedEvent {
        source: EventSource::Acp {
            variant: "config_options".to_string(),
        },
        event: MiraEvent::ConfigOptions {
            options: vec![SessionConfigView {
                id: "model".to_string(),
                name: "Model".to_string(),
                description: None,
                category: Some("model".to_string()),
                current: current.map(str::to_string),
                values,
            }],
        },
    })
}

fn modes_event(current: &str) -> NormalizedEvent {
    NormalizedEvent {
        source: EventSource::Acp {
            variant: "modes".to_string(),
        },
        event: MiraEvent::Modes {
            current: current.to_string(),
            available: mode_views(),
        },
    }
}

pub fn map_line_with(v: &Value, models: Option<&[NativeModel]>) -> Vec<NativeAction> {
    let kind = v.get("type").and_then(Value::as_str);
    if let Some("error") = kind {
        return vec![NativeAction::TurnEnded {
            stop_reason: "error".to_string(),
            is_error: true,
        }];
    }
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let mut out = Vec::new();
            if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
                out.push(NativeAction::SessionId(sid.to_string()));
            }

            // Slash commands, so the composer's command list is the agent's.
            if let Some(cmds) = v.get("slash_commands").and_then(Value::as_array) {
                let names: Vec<String> = cmds
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim_start_matches('/').to_string())
                    .collect();
                if !names.is_empty() {
                    out.push(NativeAction::Event(NormalizedEvent {
                        source: EventSource::Acp {
                            variant: "commands".to_string(),
                        },
                        event: MiraEvent::Commands { names },
                    }));
                }
            }

            // Model and permission mode are the CLI's own config surface.
            // `init` refines what `start()` announced: same list, now with
            // the running model marked current.
            if let Some(model) = v.get("model").and_then(Value::as_str) {
                if let Some(ev) = model_config_event(Some(model), models) {
                    out.push(NativeAction::Event(ev));
                }
            }
            if let Some(pm) = v.get("permissionMode").and_then(Value::as_str) {
                out.push(NativeAction::Event(modes_event(pm)));
            }
            out
        }

        // Every other `system` envelope is the CLI narrating itself —
        // `thinking_tokens`, `task_summary`, `post_turn_summary`, hook and
        // retry notices, and `permission_denied`, whose reason already
        // arrives as the failed tool result. Surfacing them as raw JSON
        // warnings buried the answer under diagnostics, so they are
        // logged, not shown.
        Some("system") => {
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            tracing::debug!(subtype, "native: system notice");
            if matches!(subtype, "api_retry" | "retry" | "session_recovery" | "workspace_error") {
                return vec![NativeAction::Event(NormalizedEvent { source: EventSource::Acp { variant: subtype.into() }, event: MiraEvent::Activity {
                    kind: subtype.into(), title: if subtype.contains("retry") { "Retrying request" } else if subtype == "workspace_error" { "Workspace setup failed" } else { "Recovering session" }.into(),
                    detail: v.get("message").and_then(Value::as_str).unwrap_or("").into(),
                } })];
            }
            Vec::new()
        }

        Some("assistant") => {
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mid = v
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            blocks
                .iter()
                .filter_map(|b| map_assistant_block(b, mid.clone()))
                .map(NativeAction::Event)
                .collect()
        }

        Some("user") => {
            // Tool results come back on a `user` envelope.
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                .filter_map(|b| {
                    let id = b
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)?;
                    let text = b.get("content").map(flatten_content).unwrap_or_default();
                    Some(NativeAction::Event(NormalizedEvent {
                        source: EventSource::Acp {
                            variant: "tool_call_update".to_string(),
                        },
                        event: MiraEvent::ToolCallUpdate(ToolCallState {
                            id,
                            title: String::new(),
                            name: None,
                            kind: None,
                            status: if b.get("is_error").and_then(Value::as_bool) == Some(true) {
                                ToolCallStatus::Failed
                            } else {
                                ToolCallStatus::Completed
                            },
                            content: vec![ToolContent::Content { text }],
                            locations: Vec::new(),
                            raw_input: None,
                            raw_output: None,
                        }),
                    }))
                })
                .collect()
        }

        Some("control_request") => {
            let req = v.get("request").cloned().unwrap_or(Value::Null);
            let request_id = req
                .get("request_id")
                .and_then(Value::as_str)
                .or_else(|| v.get("request_id").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            // `can_use_tool` names its tool. Other subtypes
            // (`AskUserQuestion`, `ExitPlanMode`, …) carry no `tool_name`,
            // so the subtype itself is the name — showing "tool" would hide
            // what is actually being asked. Answering is still the same
            // `control_response` allow/deny, which is best-effort for
            // non-tool subtypes: UNVERIFIED against a real turn.
            let subtype = req.get("subtype").and_then(Value::as_str).unwrap_or("");
            let tool_name = req
                .get("tool_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if subtype.is_empty() {
                        "tool".to_string()
                    } else {
                        subtype.to_string()
                    }
                });
            vec![NativeAction::Permission(NativePermission {
                request_id,
                tool_name,
                input: req.get("input").cloned().unwrap_or_else(|| req.clone()),
                tool_use_id: req
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reason: req
                    .get("decision_reason")
                    .and_then(Value::as_str)
                    .filter(|r| !r.is_empty())
                    .map(str::to_string),
                suggestions: req
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            })]
        }

        Some("result") => vec![NativeAction::TurnEnded {
            stop_reason: v
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or("end_turn")
                .to_string(),
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        }],

        // Answers to our own control requests (model, mode, interrupt).
        // Success needs nothing; the change itself arrives as `system`
        // status. A refusal is logged — the caller already reported the
        // optimistic change, and the next `init` corrects it.
        Some("control_response") => {
            let r = v.get("response").cloned().unwrap_or(Value::Null);
            if r.get("subtype").and_then(Value::as_str) == Some("error") {
                tracing::warn!(response = %r, "native: control request refused");
            }
            Vec::new()
        }

        // Sent every turn, limited or not: it is the account's plan usage.
        // Only a status other than `allowed` means this turn hit a limit —
        // treating every arrival as one labelled unrelated errors as
        // usage limits.
        Some("rate_limit_event") => {
            let info = v.get("rate_limit_info").cloned().unwrap_or(Value::Null);
            let mut out = Vec::new();
            if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
                let windows: Vec<LimitWindow> = windows
                    .iter()
                    .filter_map(|(name, w)| {
                        Some(LimitWindow {
                            name: name.clone(),
                            utilization: w.get("utilization")?.as_f64()?,
                            resets_at: w.get("resetsAt").and_then(Value::as_i64),
                        })
                    })
                    .collect();
                if !windows.is_empty() {
                    out.push(NativeAction::Event(NormalizedEvent {
                        source: EventSource::Acp {
                            variant: "limits".to_string(),
                        },
                        event: MiraEvent::Limits { windows },
                    }));
                }
            }
            let status = info
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("allowed");
            if status != "allowed" && status != "allowed_warning" {
                out.push(NativeAction::RateLimited {
                    resets_at: info
                        .get("resetsAt")
                        .and_then(Value::as_i64)
                        .map(describe_reset),
                });
            }
            out
        }

        // Unknown: surfaced, never dropped.
        other => {
            let label = other.unwrap_or("unknown").to_string();
            vec![NativeAction::Event(NormalizedEvent {
                source: EventSource::Unmodelled {
                    method: label.clone(),
                },
                event: MiraEvent::Unmodelled {
                    source: EventSource::Unmodelled { method: label },
                    reason: v.to_string(),
                },
            })]
        }
    }
}

/// "in 3h 12m" for a unix-seconds reset time.
fn describe_reset(at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (at - now).max(0);
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 {
        format!("in {h}h {m}m")
    } else {
        format!("in {}m", m.max(1))
    }
}

/// Per-connection parse state for partial messages.
///
/// With `--include-partial-messages` the CLI sends each text and thinking
/// block twice: as `stream_event` deltas while it is generated, then whole
/// inside an `assistant` envelope once it is done. Deltas are what make the
/// reply appear as it is written, so they are forwarded and the whole-block
/// repeat is dropped — per message and per kind, so a message whose text
/// streamed still shows a tool call that did not.
///
/// Subagent traffic (`parent_tool_use_id` set) never writes narration into
/// the parent transcript: several subagents' text interleaved into one
/// reply is unreadable. Their tool calls still pass.
#[derive(Default)]
pub struct StreamState {
    background: std::collections::HashMap<String, crate::runtime::RuntimeWork>,
    runtime_turn: Option<crate::runtime::RuntimeTurn>,
    turn_sequence: u64,
    current: Option<String>,
    last_text_message: Option<String>,
    streamed: std::collections::HashSet<(String, &'static str)>,
    /// Tokens in context after the latest model response: its input
    /// (fresh, cached and cache-written) plus what it wrote.
    context: Option<u64>,
}

/// A result's `modelUsage`, as running spend per model. `None` when the
/// result carries none (or only zeros, as a crashed session's last result
/// does — booking those would read as a reset).
fn model_spend(result: &Value) -> Option<Vec<crate::events::ModelSpend>> {
    let by_model = result.get("modelUsage")?.as_object()?;
    let n = |u: &Value, k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let out: Vec<_> = by_model
        .iter()
        .map(|(model, u)| crate::events::ModelSpend {
            model: model.clone(),
            input_tokens: n(u, "inputTokens"),
            output_tokens: n(u, "outputTokens"),
            cached_input_tokens: n(u, "cacheReadInputTokens") + n(u, "cacheCreationInputTokens"),
            cost_usd: u.get("costUSD").and_then(Value::as_f64),
        })
        .filter(|m| {
            m.input_tokens + m.output_tokens + m.cached_input_tokens > 0
                || m.cost_usd.unwrap_or(0.0) > 0.0
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

/// Context tokens a response's `usage` accounts for.
fn context_tokens(usage: &Value) -> Option<u64> {
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let total = n("input_tokens")
        + n("cache_read_input_tokens")
        + n("cache_creation_input_tokens")
        + n("output_tokens");
    (total > 0).then_some(total)
}

impl StreamState {
    pub fn map(&mut self, v: &Value, models: Option<&[NativeModel]>) -> Vec<NativeAction> {
        use crate::runtime::*;
        let event = |event| NativeAction::Event(NormalizedEvent {
            source: EventSource::Acp { variant: "runtime_lifecycle".into() }, event
        });
        if v["type"] == "system" && v["subtype"] == "background_tasks_changed" {
            let Some(tasks) = v["tasks"].as_array() else { return vec![]; };
            let mut next = std::collections::HashMap::new();
            for task in tasks {
                let frame = json!({"type":"system","subtype":"task_started",
                    "task_id":task["task_id"],"task_type":task["task_type"],
                    "description":task["description"],"session_id":v["session_id"]});
                if let Some(work) = native_background_work(&frame) { next.insert(work.id.clone(),work); }
            }
            let mut out = vec![];
            for (id, old) in &self.background {
                if !next.contains_key(id) { let mut ended = old.clone(); ended.status = WorkStatus::Completed; out.push(event(MiraEvent::RuntimeWork(ended))); }
            }
            for work in next.values() {
                if self.background.get(&work.id) != Some(work) { out.push(event(MiraEvent::RuntimeWork(work.clone()))); }
            }
            self.background = next;
            return out;
        }
        if let Some(work) = native_background_work(v) {
            if work.status.is_active() { self.background.insert(work.id.clone(),work.clone()); }
            else { self.background.remove(&work.id); }
            return vec![event(MiraEvent::RuntimeWork(work))];
        }
        let root = v.get("parent_tool_use_id").is_none_or(Value::is_null);
        let mut lifecycle = vec![];
        if root && self.runtime_turn.is_none() && v["type"] == "system" && v["subtype"] == "init" {
            if let Some(sid) = v["session_id"].as_str() {
                self.turn_sequence += 1;
                let turn = RuntimeTurn { native_thread_id:sid.into(),
                    native_turn_id:format!("claude:{}",self.turn_sequence), running:true };
                lifecycle.push(event(MiraEvent::RuntimeTurn(turn.clone())));
                self.runtime_turn = Some(turn);
            }
        }
        if root && v["type"] == "result" {
            if let Some(mut turn) = self.runtime_turn.take() { turn.running=false; lifecycle.push(event(MiraEvent::RuntimeTurn(turn))); }
        }
        lifecycle.extend(self.map_inner(v,models));
        lifecycle
    }

    fn map_inner(&mut self, v: &Value, models: Option<&[NativeModel]>) -> Vec<NativeAction> {
        if let Some(work) = native_background_work(v) {
            return vec![NativeAction::Event(NormalizedEvent {
                source: EventSource::Acp {
                    variant: "background_work".into(),
                },
                event: MiraEvent::RuntimeWork(work),
            })];
        }
        let sub = v.get("parent_tool_use_id").is_some_and(|p| !p.is_null());
        match v.get("type").and_then(Value::as_str) {
            Some("stream_event") => {
                if sub {
                    return Vec::new();
                }
                self.stream_event(v.get("event").unwrap_or(&Value::Null))
            }
            Some("assistant") => {
                // Subagent traffic has its own context, not the session's.
                if !sub {
                    if let Some(t) = v
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(context_tokens)
                    {
                        self.context = Some(t);
                    }
                }
                let mid = v
                    .get("message")
                    .and_then(|m| m.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let mut events: Vec<_> = map_line_with(v, models)
                    .into_iter()
                    .filter(|a| {
                        let NativeAction::Event(NormalizedEvent { event, .. }) = a else {
                            return true;
                        };
                        let kind = match event {
                            MiraEvent::AssistantText { .. } => "text",
                            MiraEvent::AgentThought { .. } => "thinking",
                            _ => return true,
                        };
                        if sub {
                            return false;
                        }
                        !mid.as_ref()
                            .is_some_and(|m| self.streamed.contains(&(m.clone(), kind)))
                    })
                    .collect();
                if !sub && mid.is_some() {
                    let text = v
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(Value::as_array)
                        .map(|blocks| {
                            blocks
                                .iter()
                                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                                .filter_map(|b| b.get("text").and_then(Value::as_str))
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    if !text.is_empty() {
                        self.last_text_message = mid.clone();
                        events.push(NativeAction::Event(NormalizedEvent {
                            source: EventSource::Acp {
                                variant: "agent_message_snapshot".into(),
                            },
                            event: MiraEvent::AssistantSnapshot {
                                message_id: mid.unwrap(),
                                text,
                            },
                        }));
                    }
                }
                events
            }
            Some("result") => {
                if sub { return Vec::new(); }
                self.streamed.clear();
                self.current = None;
                let mut out = Vec::new();
                if let Some(message_id) = self.last_text_message.take() {
                    if v.get("is_error").and_then(Value::as_bool) != Some(true) {
                        out.push(NativeAction::Event(NormalizedEvent { source: EventSource::Acp { variant: "message_metadata".into() }, event: MiraEvent::MessageMetadata { message_id, phase: "final_answer".into() } }));
                    }
                }
                // Context fill and cost, for the composer's usage ring. The
                // window comes from the result's per-model usage.
                let window = v
                    .get("modelUsage")
                    .and_then(Value::as_object)
                    .and_then(|m| {
                        m.values()
                            .filter_map(|u| u.get("contextWindow")?.as_u64())
                            .max()
                    });
                if let (Some(used), Some(size)) = (self.context, window) {
                    out.push(NativeAction::Event(NormalizedEvent {
                        source: EventSource::Acp {
                            variant: "usage_update".to_string(),
                        },
                        event: MiraEvent::Usage {
                            used,
                            size,
                            cost: v
                                .get("total_cost_usd")
                                .and_then(Value::as_f64)
                                .map(|c| (c, "USD".to_string())),
                        },
                    }));
                }
                // What the session has spent so far, per model (running
                // totals, subagents included), for the usage ledger.
                if let Some(spend) = model_spend(v) {
                    out.push(NativeAction::Event(NormalizedEvent {
                        source: EventSource::Acp {
                            variant: "spend".to_string(),
                        },
                        event: MiraEvent::Spend {
                            session: v
                                .get("session_id")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            models: spend,
                        },
                    }));
                }
                out.extend(map_line_with(v, models));
                out
            }
            _ => map_line_with(v, models),
        }
    }

    fn stream_event(&mut self, ev: &Value) -> Vec<NativeAction> {
        match ev.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.current = ev
                    .get("message")
                    .and_then(|m| m.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Vec::new()
            }
            Some("content_block_delta") => {
                let delta = ev.get("delta").unwrap_or(&Value::Null);
                let (kind, text, event) = match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let t = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        ("text", t, "agent_message_chunk")
                    }
                    Some("thinking_delta") => {
                        let t = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                        ("thinking", t, "agent_thought_chunk")
                    }
                    _ => return Vec::new(),
                };
                if text.is_empty() {
                    return Vec::new();
                }
                if let Some(m) = &self.current {
                    self.streamed.insert((m.clone(), kind));
                }
                let message_id = self.current.clone();
                if kind == "text" { self.last_text_message = message_id.clone(); }
                let text = text.to_string();
                vec![NativeAction::Event(NormalizedEvent {
                    source: EventSource::Acp {
                        variant: event.to_string(),
                    },
                    event: if kind == "text" {
                        MiraEvent::AssistantText { message_id, text }
                    } else {
                        MiraEvent::AgentThought { message_id, text }
                    },
                })]
            }
            _ => Vec::new(),
        }
    }
}

fn map_assistant_block(b: &Value, mid: Option<String>) -> Option<NormalizedEvent> {
    match b.get("type").and_then(Value::as_str)? {
        "text" => Some(NormalizedEvent {
            source: EventSource::Acp {
                variant: "agent_message_chunk".to_string(),
            },
            event: MiraEvent::AssistantText {
                message_id: mid,
                text: b
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
        }),
        // A thinking block whose text was withheld carries nothing to show.
        "thinking" => {
            let text = b.get("thinking").and_then(Value::as_str).unwrap_or("");
            if text.trim().is_empty() {
                return None;
            }
            Some(NormalizedEvent {
                source: EventSource::Acp {
                    variant: "agent_thought_chunk".to_string(),
                },
                event: MiraEvent::AgentThought {
                    message_id: mid,
                    text: text.to_string(),
                },
            })
        }
        "tool_use" => Some(NormalizedEvent {
            source: EventSource::Acp {
                variant: "tool_call".to_string(),
            },
            event: MiraEvent::ToolCall(ToolCallState {
                id: b.get("id").and_then(Value::as_str)?.to_string(),
                title: b
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string(),
                name: b.get("name").and_then(Value::as_str).map(str::to_string),
                kind: b.get("name").and_then(Value::as_str).map(tool_kind),
                status: ToolCallStatus::Pending,
                content: Vec::new(),
                locations: Vec::new(),
                raw_input: b.get("input").cloned(),
                raw_output: None,
            }),
        }),
        _ => None,
    }
}

/// Best-effort map from a tool name to ACP's tool taxonomy, so the UI can
/// pick an icon. Unrecognised tools become `None` rather than being forced
/// into a wrong bucket.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" | "NotebookRead" => ToolKind::Read,
        "Edit" | "Write" | "NotebookEdit" | "MultiEdit" => ToolKind::Edit,
        "Glob" | "Grep" => ToolKind::Search,
        "Bash" | "BashOutput" => ToolKind::Execute,
        "WebFetch" | "WebSearch" => ToolKind::Fetch,
        "Task" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

fn flatten_content(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|i| i.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

/// One model the agent's own CLI knows about.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeModel {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// The `--model` aliases `claude --help` documents, resolving to the latest
/// of each family. Listed first because they never go stale: a full model id
/// rots with every release, an alias does not.
const CLAUDE_MODEL_ALIASES: &[(&str, &str)] = &[
    ("opus", "Opus — latest"),
    ("sonnet", "Sonnet — latest"),
    ("haiku", "Haiku — latest"),
    ("fable", "Fable — latest"),
];

/// Models Claude Code itself knows, from its own data.
///
/// The CLI exposes no list-models command, so this reads the two places the
/// CLI keeps model names: `additionalModelOptionsCache` (its own model menu)
/// and the `lastModelUsage` keys it records per project (models this account
/// has actually run). Every full id returned is one the CLI itself wrote
/// down — nothing is invented, and a model the account cannot use never
/// appears just because it exists publicly.
pub fn claude_models() -> Vec<NativeModel> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data = home
        .as_deref()
        .map(|h| h.join(".claude.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    claude_models_from(data.as_ref())
}

fn claude_models_from(data: Option<&Value>) -> Vec<NativeModel> {
    let mut out: Vec<NativeModel> = CLAUDE_MODEL_ALIASES
        .iter()
        .map(|(v, l)| NativeModel {
            value: v.to_string(),
            label: l.to_string(),
            description: None,
        })
        .collect();
    let mut seen: std::collections::HashSet<String> = out.iter().map(|m| m.value.clone()).collect();
    let mut push = |value: &str, label: Option<&str>, description: Option<&str>| {
        if value.is_empty() || !seen.insert(value.to_string()) {
            return;
        }
        out.push(NativeModel {
            value: value.to_string(),
            label: label.unwrap_or(value).to_string(),
            description: description.map(str::to_string),
        });
    };

    let Some(data) = data else { return out };
    // The CLI's own model menu.
    if let Some(cache) = data
        .get("additionalModelOptionsCache")
        .and_then(Value::as_array)
    {
        for o in cache {
            push(
                o.get("value").and_then(Value::as_str).unwrap_or(""),
                o.get("label").and_then(Value::as_str),
                o.get("description").and_then(Value::as_str),
            );
        }
    }
    // Every model this account has actually run, newest project first is
    // unknowable from the file, so these are sorted for stability.
    let mut used: Vec<String> = Vec::new();
    fn collect(o: &Value, into: &mut Vec<String>) {
        match o {
            Value::Object(m) => {
                if let Some(u) = m.get("lastModelUsage").and_then(Value::as_object) {
                    into.extend(u.keys().cloned());
                }
                for v in m.values() {
                    collect(v, into);
                }
            }
            Value::Array(a) => {
                for v in a {
                    collect(v, into);
                }
            }
            _ => {}
        }
    }
    collect(data, &mut used);
    used.sort();
    used.dedup();
    for id in used {
        push(&id, None, None);
    }
    out
}

/// The permission modes the CLI accepts, as ACP-shaped mode views.
pub fn mode_views() -> Vec<SessionModeView> {
    [
        ("default", "Ask before running commands or editing files"),
        (
            "acceptEdits",
            "Auto-accept file edits, still ask for commands",
        ),
        ("plan", "Plan only, make no changes"),
        ("dontAsk", "Never ask; refuse anything needing approval"),
        ("auto", "Let the agent decide"),
    ]
    .into_iter()
    .map(|(id, desc)| SessionModeView {
        id: id.to_string(),
        name: id.to_string(),
        description: Some(desc.to_string()),
    })
    .collect()
}

/// Map Mira's permission posture onto a `--permission-mode` value.
pub fn permission_mode_arg(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        // Mira's `Auto` still asks when the agent is unsure, so it maps to the
        // CLI's `default`. `auto` would hand the decision to the agent, which
        // is a weaker guarantee than Mira's name implies.
        PermissionMode::Auto => "default",
    }
}

/// Overrides applied when relaunching a native agent.
///
/// A native agent takes `--model` and `--permission-mode` at launch, so
/// changing either means a new process. The session survives through
/// `--resume`, which is why relaunching is a settings change and not a data
/// loss event.
#[derive(Clone, Debug, Default)]
pub struct NativeOverrides {
    pub model: Option<String>,
    /// The agent's own mode id (e.g. `acceptEdits`), passed verbatim.
    pub permission_mode: Option<String>,
    /// Continue this session rather than starting a new one.
    pub resume: Option<String>,
    /// Fork on resume: continue the history under a new session id instead
    /// of reusing the original (`claude --resume <sid> --fork-session`).
    /// Only meaningful alongside `resume`; set without it, it does nothing
    /// rather than forking nothing.
    pub fork: bool,
}

/// Apply relaunch overrides to an already-built command line.
///
/// Kept as a post-step rather than more `launch()` parameters so the common
/// first-start path stays readable: overrides only exist for restarts.
pub fn apply_overrides(launch: &mut LaunchConfig, o: &NativeOverrides) {
    if let Some(pm) = &o.permission_mode {
        // We built this argv, so `--permission-mode <value>` is always
        // present exactly once; replacing the value cannot hit anything else.
        if let Some(i) = launch.args.iter().position(|a| a == "--permission-mode") {
            if let Some(slot) = launch.args.get_mut(i + 1) {
                *slot = pm.clone();
            }
        }
    }
    if let Some(m) = &o.model {
        if let Some(i) = launch.args.iter().position(|a| a == "--model") {
            if let Some(slot) = launch.args.get_mut(i + 1) {
                *slot = m.clone();
            }
        } else {
            launch.args.push("--model".to_string());
            launch.args.push(m.clone());
        }
    }
    if let Some(sid) = &o.resume {
        launch.args.push("--resume".to_string());
        launch.args.push(sid.clone());
        if o.fork {
            launch.args.push("--fork-session".to_string());
        }
    }
}

/// Build the command line for a Claude Code session.
///
/// `resume` continues an existing session rather than starting one, which is
/// what makes a second prompt in the same Mira session reuse the agent's
/// context instead of paying for it again. Other agents' argv live with
/// their transports (`appserver::launch` for Codex): sharing one builder
/// with conditionals is how `claude` once launched as `codex exec --json`.
pub fn launch(
    program: std::path::PathBuf,
    mode: PermissionMode,
    model: Option<&str>,
    resume: Option<&str>,
) -> LaunchConfig {
    let mut args: Vec<String> = vec![
        "-p".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        // Token-by-token text and thinking. Without it a reply appeared all
        // at once when its block finished; the whole block still arrives
        // afterwards and `StreamState` drops the repeat.
        "--include-partial-messages".into(),
        // Thinking text. By default the CLI streams thinking blocks with the
        // text left out (`"thinking": ""` plus a token estimate), so the
        // transcript showed a "Thought" row with nothing in it. `--settings`
        // adds to the user's own settings; it does not replace them.
        "--settings".into(),
        r#"{"showThinkingSummaries":true}"#.into(),
        // We answer permission requests ourselves. `--permission-prompts
        // host` alone is not enough: with no prompt tool named, the CLI has
        // no channel to the host and silently *denies* anything that would
        // prompt (the "Contains simple_expansion" failures). `stdio` routes
        // each one to us as a `can_use_tool` control request instead.
        "--permission-prompts".into(),
        "host".into(),
        "--permission-prompt-tool".into(),
        "stdio".into(),
        "--permission-mode".into(),
        permission_mode_arg(mode).into(),
    ];
    if let Some(m) = model {
        args.push("--model".into());
        args.push(m.to_string());
    }
    if let Some(sid) = resume {
        args.push("--resume".into());
        args.push(sid.to_string());
    }
    LaunchConfig {
        program,
        args,
        env: Default::default(),
        secret_env: Vec::new(),
        env_deny: Vec::new(),
    }
}

/// A running agent, whichever transport is carrying it.
///
/// The server holds one of these and does not care which: the transcript,
/// the permission prompts and the composer's busy state are identical
/// either way. The differences are confined to how a turn is started and how
/// it ends.
#[derive(Clone)]
pub enum AgentHandle {
    /// Over ACP JSON-RPC, via an adapter binary.
    Acp(crate::process::AcpAgent),
    /// Over the agent's own headless line protocol (Claude Code).
    Native(std::sync::Arc<NativeAgent>),
    /// Over Codex's app-server JSON-RPC.
    AppServer(std::sync::Arc<crate::appserver::AppServerAgent>),
}

impl AgentHandle {
    pub fn runtime_capabilities(&self) -> crate::runtime::RuntimeCapabilities {
        match self {
            Self::AppServer(_) => crate::runtime::RuntimeCapabilities {
                asynchronous_questions: true,
                background_work: true,
                native_goals: true,
                steering: crate::runtime::SteeringCapability::Native,
                image_input: true,
                live_model_switch: true,
                live_mode_switch: true,
                cancellation: true,
                ..Default::default()
            },
            Self::Native(_) => crate::runtime::RuntimeCapabilities {
                background_work: true,
                native_fork: true,
                steering: crate::runtime::SteeringCapability::Native,
                image_input: true,
                live_model_switch: true,
                live_mode_switch: true,
                cancellation: true,
                ..Default::default()
            },
            Self::Acp(a) => a.session().runtime_capabilities(),
        }
    }

    /// Which transport is live, for display and for logs.
    pub fn transport(&self) -> Transport {
        match self {
            AgentHandle::Acp(_) => Transport::Acp,
            AgentHandle::Native(_) | AgentHandle::AppServer(_) => Transport::Native,
        }
    }

    /// The auth method ids the agent advertised at `initialize`.
    ///
    /// Only ACP carries them on the handle; the native transports
    /// authenticate out of band (the CLI's own login) and report nothing
    /// here, so an empty list means "we don't know from the agent" — not
    /// "no authentication". That distinction is why callers treat empty
    /// as unknown rather than as a key.
    pub fn advertised_auth_method_ids(&self) -> Vec<String> {
        match self {
            AgentHandle::Acp(a) => a.session().agent_auth_method_ids(),
            AgentHandle::Native(_) | AgentHandle::AppServer(_) => Vec::new(),
        }
    }

    /// The agent's own session id, once it has allocated one.
    pub async fn session_id(&self) -> Option<String> {
        match self {
            AgentHandle::Acp(a) => a.session_id(),
            AgentHandle::Native(n) => n.session_id().await,
            AgentHandle::AppServer(a) => a.session_id().await,
        }
    }

    /// Where this agent's prompt attachments are staged, if anywhere.
    pub fn attachments_dir(&self) -> Option<std::path::PathBuf> {
        match self {
            AgentHandle::Native(n) => n.attachments_dir(),
            _ => None,
        }
    }

    pub async fn stderr_tail(&self) -> String {
        match self {
            AgentHandle::Acp(a) => a.stderr_tail().await,
            AgentHandle::Native(n) => n.stderr_tail().await,
            AgentHandle::AppServer(a) => a.stderr_tail().await,
        }
    }

    /// Run a turn.
    ///
    /// `Ok(Some(stop_reason))` means the turn is finished. `Ok(None)` means it
    /// is in flight and will be announced on [`AgentHandle::turn_ended`],
    /// which is the native case: prompting is fire-and-forget, so the caller
    /// cannot block on it the way ACP's `session/prompt` allows.
    pub async fn prompt_text(&self, text: &str) -> Result<Option<String>, String> {
        match self {
            AgentHandle::Acp(a) => {
                let blocks = vec![serde_json::from_value(json!({
                    "type": "text",
                    "text": text,
                }))
                .map_err(|e| format!("could not encode prompt: {e}"))?];
                let out = a.prompt(blocks).await.map_err(|e| e.to_string())?;
                Ok(Some(out.stop_reason_wire()))
            }
            AgentHandle::Native(n) => {
                n.prompt(text).await.map_err(|e| e.to_string())?;
                Ok(None)
            }
            AgentHandle::AppServer(a) => {
                a.prompt(text).await.map_err(|e| e.to_string())?;
                Ok(None)
            }
        }
    }

    /// Whether prompts to this handle can carry images inline.
    ///
    /// Inline images use each native protocol or negotiated ACP support. Used to pick the
    /// delivery path before composing the prompt.
    pub fn accepts_image_blocks(&self) -> bool {
        self.runtime_capabilities().image_input
    }

    /// Run a turn whose prompt carries images inline.
    ///
    /// ACP and Codex agents receive the images as first-class `image` content
    /// blocks in the `session/prompt` — the protocol's own media
    /// path. Claude receives base64 image blocks and Codex uses app-server inputs.
    pub async fn prompt_with_images(
        &self,
        text: &str,
        images: &[mira_core::ImageData],
    ) -> Result<Option<String>, String> {
        match self {
            AgentHandle::Acp(a) => {
                let mut blocks: Vec<agent_client_protocol::schema::v1::ContentBlock> =
                    vec![serde_json::from_value(json!({
                        "type": "text",
                        "text": text,
                    }))
                    .map_err(|e| format!("could not encode prompt: {e}"))?];
                for img in images {
                    blocks.push(
                        serde_json::from_value(json!({
                            "type": "image",
                            "data": img.data,
                            "mimeType": img.media_type,
                        }))
                        .map_err(|e| format!("could not encode image block: {e}"))?,
                    );
                }
                let out = a.prompt(blocks).await.map_err(|e| e.to_string())?;
                Ok(Some(out.stop_reason_wire()))
            }
            AgentHandle::AppServer(a) => {
                a.prompt_with_images(text, images).await.map_err(|e| e.to_string())?;
                Ok(None)
            }
            AgentHandle::Native(n) => {
                n.user_input(text, images, None).await.map_err(|e| e.to_string())?;
                Ok(None)
            }
        }
    }

    /// Change the agent's permission mode.
    ///
    /// ACP can do this live. A native agent takes `--permission-mode` at
    /// launch and we have not verified any control request for changing it
    /// mid-session, so rather than report success for something that did not
    /// happen, native says what is actually true: restart to apply it.
    pub async fn set_mode(&self, mode_id: &str) -> Result<(), String> {
        match self {
            AgentHandle::Acp(a) => {
                let sid = a
                    .session_id()
                    .ok_or_else(|| "no agent session yet".to_string())?;
                a.session()
                    .set_mode(&sid, mode_id)
                    .await
                    .map_err(|e| e.to_string())
            }
            AgentHandle::Native(n) => n
                .set_permission_mode(mode_id)
                .await
                .map_err(|e| e.to_string()),
            // Codex takes approval policy, sandbox and collaboration mode on
            // every `turn/start`, so a change applies from the next turn.
            AgentHandle::AppServer(a) => {
                a.set_mode(mode_id).await;
                Ok(())
            }
        }
    }

    /// A conversation title from the agent itself, when it can write one.
    pub async fn generate_title(&self, description: &str) -> Option<String> {
        match self {
            AgentHandle::Native(n) => n.generate_title(description).await.ok(),
            _ => None,
        }
    }

    /// What fills the agent's context window, when the agent can say.
    /// Claude Code reports it over its control protocol; ACP and Codex have
    /// no equivalent.
    pub async fn context_usage(&self) -> Result<serde_json::Value, String> {
        match self {
            AgentHandle::Native(n) => n.context_usage().await.map_err(|e| e.to_string()),
            _ => Err("This agent doesn't report what's in its context window.".into()),
        }
    }

    /// Whether mode and model changes apply to the running process, rather
    /// than needing a relaunch.
    pub fn changes_live(&self) -> bool {
        matches!(
            self,
            AgentHandle::Acp(_) | AgentHandle::Native(_) | AgentHandle::AppServer(_)
        )
    }

    /// Set one of the agent's config options (the model picker).
    ///
    /// ACP applies it live and returns the refreshed list. A native agent
    /// takes `--model` at launch, so this reports the same honest limitation
    /// as `set_mode` rather than pretending the picker changed the model.
    pub async fn set_config_option(
        &self,
        option_id: &str,
        value: serde_json::Value,
    ) -> Result<Vec<crate::events::SessionConfigView>, String> {
        match self {
            AgentHandle::Acp(a) => {
                let sid = a
                    .session_id()
                    .ok_or_else(|| "no agent session yet".to_string())?;
                let out = a
                    .session()
                    .set_config_option(&sid, option_id, value)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(out.options)
            }
            AgentHandle::Native(n) if option_id == "model" => {
                let model = value.as_str().unwrap_or_default().to_string();
                n.set_model(&model).await.map_err(|e| e.to_string())?;
                Ok(Vec::new())
            }
            AgentHandle::AppServer(a) if option_id == "model" => {
                let model = value.as_str().unwrap_or_default();
                a.set_model(model).await;
                Ok(Vec::new())
            }
            AgentHandle::Native(_) | AgentHandle::AppServer(_) => Err(format!(
                "{option_id} applies to the next run of this agent — restart it to change the model"
            )),
        }
    }

    pub async fn cancel(&self) -> Result<(), NativeError> {
        match self {
            AgentHandle::Acp(a) => a
                .cancel()
                .await
                .map_err(|e| NativeError::Other(e.to_string())),
            AgentHandle::Native(n) => n.cancel().await,
            AgentHandle::AppServer(a) => a.cancel().await,
        }
    }

    pub async fn shutdown(&self) {
        match self {
            AgentHandle::Acp(a) => a.shutdown().await,
            AgentHandle::Native(n) => n.shutdown().await,
            AgentHandle::AppServer(a) => a.shutdown().await,
        }
    }

    /// Take the event channels for the server pump. Both native transports
    /// expose the same pair, which is why the server does not care which one
    /// is live.
    pub fn take_event_channels(
        &self,
    ) -> Option<(mpsc::Receiver<NormalizedEvent>, mpsc::Receiver<TurnEnd>)> {
        match self {
            AgentHandle::Acp(_) => None,
            AgentHandle::Native(n) => Some((
                n.events.lock().expect("events").take()?,
                n.turn_end.lock().expect("turn end").take()?,
            )),
            AgentHandle::AppServer(a) => Some((
                a.events.lock().expect("events").take()?,
                a.turn_end.lock().expect("turn end").take()?,
            )),
        }
    }
}

/// A running native agent.
pub struct NativeAgent {
    child: Arc<Mutex<tokio::process::Child>>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    stderr: Arc<Mutex<Vec<String>>>,
    session_id: Arc<Mutex<Option<String>>>,
    /// Where prompt attachments are staged for the agent to read. The CLI is
    /// launched with `--add-dir` pointing here (see `process::start_agent`),
    /// so files the user attaches are actually reachable, not just named.
    attachments_dir: Option<std::path::PathBuf>,
    pub events: std::sync::Mutex<Option<mpsc::Receiver<NormalizedEvent>>>,
    /// Fires once per completed turn.
    ///
    /// Native prompting is fire-and-forget, so unlike ACP — where the turn
    /// ends when `session/prompt` returns — nothing else would ever tell the
    /// caller the agent finished. Without this the composer spins forever.
    pub turn_end: std::sync::Mutex<Option<mpsc::Receiver<TurnEnd>>>,
    /// The event wire, kept so a live model or mode change can announce
    /// itself the same way the CLI's own reports do.
    notify: mpsc::Sender<NormalizedEvent>,
    /// The CLI's known models, for re-announcing the model list on change.
    models: Option<Vec<NativeModel>>,
    /// Control requests awaiting their answer, by request id.
    replies: Arc<Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<Value>>>>,
}

/// The JSON we write back to answer a permission: Claude Code's
/// `control_response` envelope. (Codex answers approvals inline over
/// app-server; see `appserver::approval_response`.)
///
/// An allow must echo the tool input back as `updatedInput`: the CLI runs
/// the tool with whatever the host returns, and an allow without it is
/// treated as malformed by current builds.
pub fn permission_response(
    request_id: &str,
    decision: &PermissionDecision,
    input: &Value,
) -> Value {
    let body = if decision.allow {
        let mut b = json!({
            "behavior": "allow",
            "updatedInput": decision.updated_input.clone().unwrap_or_else(|| input.clone()),
        });
        if !decision.updated_permissions.is_empty() {
            b["updatedPermissions"] = Value::Array(decision.updated_permissions.clone());
        }
        b
    } else {
        json!({
            "behavior": "deny",
            "message": decision.message.clone().unwrap_or_else(|| "denied by the user in Mira".to_string()),
        })
    };
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": body,
        }
    })
}

/// Rules for an "allow for this session" answer.
///
/// The CLI's own suggestions are reused but rescoped to `session`: echoed
/// verbatim they usually target `localSettings`, which would quietly turn
/// a session-only choice into a permanent rule in the project. With no
/// suggestion (common for MCP tools and compound commands) the fallback is
/// a whole-tool session rule, so the choice sticks instead of degrading
/// into a one-off allow. `persist` keeps the suggestions' own destination —
/// that is the "always" answer.
pub fn session_permission_updates(
    tool_name: &str,
    suggestions: &[Value],
    persist: bool,
) -> Vec<Value> {
    let scoped: Vec<Value> = suggestions
        .iter()
        .filter(|s| s.is_object())
        .map(|s| {
            let mut s = s.clone();
            if !persist {
                s["destination"] = json!("session");
            }
            s
        })
        .collect();
    if !scoped.is_empty() {
        return scoped;
    }
    vec![json!({
        "type": "addRules",
        "rules": [{ "toolName": tool_name }],
        "behavior": "allow",
        "destination": if persist { "localSettings" } else { "session" },
    })]
}

/// Whether the agent process is still running.
async fn is_alive(child: &Arc<Mutex<tokio::process::Child>>) -> bool {
    matches!(child.lock().await.try_wait(), Ok(None))
}

/// How a native turn finished.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnEnd {
    /// The CLI's own label (`success`, `error_max_turns`, ...).
    pub stop_reason: String,
    pub is_error: bool,
    /// Set when the account hit a usage limit, which is a normal outcome and
    /// worth telling the user plainly rather than rendering as a failure.
    pub rate_limited: Option<String>,
}

/// Errors from the native path.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("could not start {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the agent's stdin closed")]
    StdinClosed,
    #[error("the agent ended before answering")]
    Ended,
    #[error("{0}")]
    Other(String),
}

impl NativeAgent {
    /// Spawn and start reading. Events arrive on [`Self::events`].
    pub async fn start(
        launch_cfg: &LaunchConfig,
        gate: PermissionGate,
        attachments_dir: Option<std::path::PathBuf>,
    ) -> Result<Self, NativeError> {
        Self::start_with_idle(launch_cfg, gate, READ_IDLE_TIMEOUT, attachments_dir).await
    }

    /// [`start`](Self::start) with an explicit idle timeout, for tests.
    ///
    /// Production passes ten minutes; a test passes a second and a script
    /// that goes quiet, and asserts the agent survives its own silence.
    async fn start_with_idle(
        launch_cfg: &LaunchConfig,
        gate: PermissionGate,
        idle_timeout: Duration,
        attachments_dir: Option<std::path::PathBuf>,
    ) -> Result<Self, NativeError> {
        // See the ACP spawn: the program is resolved to a full path, and the
        // child is given a PATH that can find its own dependencies.
        let program = crate::which::resolve_for_spawn(&launch_cfg.program);
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&launch_cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd.env_clear();
        for key in mira_sandbox::safe_child_env() {
            if let Ok(v) = std::env::var(key) {
                if !crate::which::env_denied(key, &launch_cfg.env_deny) {
                    cmd.env(key, v);
                }
            }
        }
        if let Some(path) = crate::which::effective_path() {
            cmd.env("PATH", path);
        }
        // The driver's resolved env — user env, API keys, home isolation.
        // Missing here until now, which meant native launches silently
        // dropped everything `resolve()` had carefully assembled.
        for (k, v) in &launch_cfg.env {
            if !crate::which::env_denied(k, &launch_cfg.env_deny) {
                cmd.env(k, v);
            }
        }

        let mut child = cmd.spawn().map_err(|source| NativeError::Spawn {
            program: launch_cfg.program.display().to_string(),
            source,
        })?;
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().ok_or(NativeError::StdinClosed)?,
        ));
        let stdout = child.stdout.take().ok_or(NativeError::Ended)?;
        let stderr = child.stderr.take();

        let session_id = Arc::new(Mutex::new(None));
        let replies: Arc<
            Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<Value>>>,
        > = Arc::default();
        let rate_limit: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        // The CLI's own known models, read once.
        let models: Option<Vec<NativeModel>> = Some(claude_models());
        let (tx, rx) = mpsc::channel(256);
        let (end_tx, end_rx) = mpsc::channel(16);

        // Announce everything knowable at spawn. The CLI emits nothing —
        // not even `init` — until it receives input (verified: 12s of
        // silence on held-open stdin), so waiting for `init` leaves the
        // picker, modes and commands empty until the first prompt. `init`
        // refines this on the first turn by marking the running model.
        if let Some(ev) = model_config_event(None, models.as_deref()) {
            let _ = tx.send(ev).await;
        }
        // The launched permission mode is ours — we built the argv — so it
        // is reported as current without waiting to be told.
        let launched_mode = launch_cfg
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--permission-mode").then(|| w[1].as_str()))
            .unwrap_or("default");
        let _ = tx.send(modes_event(launched_mode)).await;

        // stderr: keep a tail for diagnostics. Never parsed.
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        if let Some(se) = stderr {
            let sink = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(se).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let mut s = sink.lock().await;
                    if s.len() >= 200 {
                        s.remove(0);
                    }
                    s.push(l);
                }
            });
        }

        // The child behind a lock before the reader starts, so the reader can
        // tell an idle agent from a dead one (see the loop below).
        let child = Arc::new(Mutex::new(child));
        // stdout: the line protocol. This loop is the whole native client.
        {
            let session_id = session_id.clone();
            let rate_limit = rate_limit.clone();
            let tx = tx.clone();
            let models = models.clone();
            let replies = replies.clone();
            let gate = gate.clone();
            // The reader answers permission requests itself, so it needs the
            // write half. Without this the CLI blocks forever on a tool call
            // nobody can approve — which is the failure mode `--permission-
            // prompts host` is specifically there to avoid.
            let writer = stdin.clone();
            let end_tx = end_tx.clone();
            // The child handle, so an idle timeout can tell "quiet" apart
            // from "dead" instead of assuming the worst.
            let watcher = child.clone();
            // A rate limit arrives as its own event, before the `result`.
            // Emitting a turn end for both meant two turn ends per turn, and
            // a client that ends on the first would then double-count the
            // second. It is held here and attached to the real end instead.
            let rate_limit = rate_limit.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut stream = StreamState::default();
                loop {
                    let line = match tokio::time::timeout(idle_timeout, lines.next_line()).await {
                        Ok(Ok(Some(l))) => l,
                        // stdout closed or the read failed: the agent is gone.
                        Ok(_) | Err(_) if !is_alive(&watcher).await => break,
                        // Idle, not dead. An agent waiting for its next prompt
                        // says nothing for minutes at a time; treating quiet
                        // as death is what produced "the external agent
                        // stopped unexpectedly" ten minutes after the user
                        // stopped typing.
                        _ => continue,
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        // A line we cannot parse is a protocol change, not a
                        // reason to kill the session.
                        tracing::debug!(line = %line, "native: unparseable line");
                        continue;
                    };
                    // An answer to a control request we are waiting on.
                    if v.get("type").and_then(Value::as_str) == Some("control_response") {
                        let id = v["response"]["request_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        if let Some(tx) = replies.lock().await.remove(&id) {
                            let _ = tx.send(v["response"].clone());
                            continue;
                        }
                    }
                    for action in stream.map(&v, models.as_deref()) {
                        match action {
                            NativeAction::SessionId(sid) => {
                                *session_id.lock().await = Some(sid);
                            }
                            NativeAction::Event(e) => {
                                let _ = tx.send(e).await;
                            }
                            NativeAction::TurnEnded {
                                stop_reason,
                                is_error,
                            } => {
                                let was_limited = rate_limit.lock().await.take();
                                let _ = end_tx
                                    .send(TurnEnd {
                                        stop_reason: if was_limited.is_some() && is_error {
                                            "rate_limited".to_string()
                                        } else {
                                            stop_reason
                                        },
                                        is_error,
                                        rate_limited: was_limited,
                                    })
                                    .await;
                            }
                            NativeAction::RateLimited { resets_at } => {
                                *rate_limit.lock().await =
                                    Some(resets_at.unwrap_or_else(|| "shortly".to_string()));
                            }
                            NativeAction::Permission(p) => {
                                // Decided on its own task. Awaiting the user
                                // here stalled the whole stream while an
                                // approval card sat open — no text, no tool
                                // updates, and a second request queued
                                // behind the first.
                                let gate = gate.clone();
                                let writer = writer.clone();
                                tokio::spawn(async move {
                                    let decision = gate(p.clone()).await;
                                    let response =
                                        permission_response(&p.request_id, &decision, &p.input);
                                    let mut w = writer.lock().await;
                                    let _ = w.write_all(response.to_string().as_bytes()).await;
                                    let _ = w.write_all(b"\n").await;
                                    let _ = w.flush().await;
                                });
                            }
                        }
                    }
                }
            });
        }

        Ok(Self {
            child,
            stdin,
            stderr: stderr_tail,
            session_id,
            attachments_dir,
            events: std::sync::Mutex::new(Some(rx)),
            turn_end: std::sync::Mutex::new(Some(end_rx)),
            notify: tx,
            models,
            replies,
        })
    }

    /// Where prompt attachments are staged, if this session has one.
    pub fn attachments_dir(&self) -> Option<std::path::PathBuf> {
        self.attachments_dir.clone()
    }

    /// The agent's session id, once its `init` has been read.
    pub async fn session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    /// Send a user turn.
    pub async fn prompt(&self, text: &str) -> Result<(), NativeError> {
        self.user_input(text, &[], None).await
    }

    /// Native priority input keeps the current stream and background workers alive.
    pub async fn steer(&self, id: &str, text: &str, images: &[mira_core::ImageData]) -> Result<(), NativeError> {
        uuid::Uuid::parse_str(id).map_err(|_|NativeError::Other("Steering requires a stable UUID input ID".into()))?;
        self.user_input(text, images, Some(id)).await
    }

    async fn user_input(&self, text: &str, images: &[mira_core::ImageData], steer_id: Option<&str>) -> Result<(), NativeError> {
        let sid = self.session_id().await.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.write_line(&claude_user_input(&sid, text, images, steer_id)).await
    }

    /// Answer a permission request.
    pub async fn answer_permission(
        &self,
        request_id: &str,
        allow: bool,
        input: &Value,
    ) -> Result<(), NativeError> {
        let response = permission_response(request_id, &allow.into(), input);
        self.write_line(&response).await
    }

    async fn write_line(&self, v: &Value) -> Result<(), NativeError> {
        let mut w = self.stdin.lock().await;
        w.write_all(v.to_string().as_bytes())
            .await
            .map_err(|_| NativeError::StdinClosed)?;
        w.write_all(b"\n")
            .await
            .map_err(|_| NativeError::StdinClosed)?;
        w.flush().await.map_err(|_| NativeError::StdinClosed)
    }

    /// Send a control request. Each gets its own id, so answers never
    /// collide.
    async fn control(&self, request: Value) -> Result<(), NativeError> {
        self.write_line(&json!({
            "type": "control_request",
            "request_id": format!("mira-{}", uuid::Uuid::new_v4()),
            "request": request,
        }))
        .await
    }

    /// Send a control request and wait for its answer.
    async fn ask(&self, request: Value, timeout: Duration) -> Result<Value, NativeError> {
        let id = format!("mira-{}", uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.replies.lock().await.insert(id.clone(), tx);
        self.write_line(
            &json!({ "type": "control_request", "request_id": id, "request": request }),
        )
        .await?;
        let reply = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => v,
            _ => {
                self.replies.lock().await.remove(&id);
                return Err(NativeError::Other(
                    "no answer to the control request".into(),
                ));
            }
        };
        if reply["subtype"] == "error" {
            return Err(NativeError::Other(
                reply["error"].as_str().unwrap_or("refused").to_string(),
            ));
        }
        Ok(reply["response"].clone())
    }

    /// A short title for the conversation, written by the agent's own model
    /// from `description` (typically the first message).
    pub async fn generate_title(&self, description: &str) -> Result<String, NativeError> {
        let r = self
            .ask(
                json!({ "subtype": "generate_session_title", "description": description }),
                Duration::from_secs(30),
            )
            .await?;
        r["title"]
            .as_str()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| NativeError::Other("empty title".into()))
    }

    /// What fills the agent's context window, by its own count: categories
    /// (system prompt, tools, memory files, skills, messages), the window,
    /// the auto-compact threshold, and per-item detail. `summary` detail
    /// uses the last response's figures, so it costs no model call.
    pub async fn context_usage(&self) -> Result<Value, NativeError> {
        self.ask(
            json!({ "subtype": "get_context_usage", "detail": "summary" }),
            Duration::from_secs(20),
        )
        .await
    }

    /// Switch the running agent's model, live.
    ///
    /// The control protocol applies it from the next turn — no restart, no
    /// resume, nothing lost. (Verified against Claude Code 2.1: the next
    /// `init` reports the new model.)
    pub async fn set_model(&self, model: &str) -> Result<(), NativeError> {
        self.control(json!({ "subtype": "set_model", "model": model }))
            .await?;
        if let Some(ev) = model_config_event(Some(model), self.models.as_deref()) {
            let _ = self.notify.send(ev).await;
        }
        Ok(())
    }

    /// Switch the running agent's permission mode, live.
    pub async fn set_permission_mode(&self, mode: &str) -> Result<(), NativeError> {
        self.control(json!({ "subtype": "set_permission_mode", "mode": mode }))
            .await?;
        let _ = self.notify.send(modes_event(mode)).await;
        Ok(())
    }

    /// Ask the agent to stop the current turn.
    ///
    /// The control protocol has an `interrupt`; falling back to a signal is
    /// correct but coarser, so the request is tried first.
    pub async fn cancel(&self) -> Result<(), NativeError> {
        let _ = self.control(json!({ "subtype": "interrupt" })).await;
        Ok(())
    }

    pub async fn stderr_tail(&self) -> String {
        self.stderr.lock().await.join("\n")
    }

    /// Stop the process. Drops stdin first so the CLI exits cleanly.
    pub async fn shutdown(&self) {
        {
            let mut w = self.stdin.lock().await;
            let _ = w.shutdown().await;
        }
        let mut child = self.child.lock().await;
        let exited = tokio::time::timeout(Duration::from_millis(800), child.wait())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
        if !exited {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poll until `pred` holds, so tests do not depend on scheduling order.
    async fn wait_for<F, Fut, T>(pred: F, what: &str) -> T
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Option<T>>,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(v) = pred().await {
                return v;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn init() -> Value {
        serde_json::from_str(
            r#"{"type":"system","subtype":"init","session_id":"sid-1","model":"claude-opus-5-5",
                "permissionMode":"default","slash_commands":["/clear","/compact"]}"#,
        )
        .unwrap()
    }

    #[test]
    fn init_yields_session_id_commands_and_modes() {
        let acts = map_line(&init());
        assert!(acts.contains(&NativeAction::SessionId("sid-1".into())));
        let cmds = acts.iter().find_map(|a| match a {
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::Commands { names },
                ..
            }) => Some(names.clone()),
            _ => None,
        });
        assert_eq!(
            cmds.as_deref(),
            Some(&["clear".to_string(), "compact".to_string()][..])
        );
        assert!(acts.iter().any(|a| matches!(
            a,
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::Modes { .. },
                ..
            })
        )));
    }

    #[test]
    fn assistant_text_thought_and_tool_use_all_map() {
        let acts = map_line(
            &serde_json::from_str(
                r#"{"type":"assistant","message":{"id":"m1","content":[
                {"type":"thinking","thinking":"hmm"},
                {"type":"text","text":"hi"},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
            )
            .unwrap(),
        );
        assert!(acts.iter().any(|a| matches!(
            a,
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::AgentThought { .. },
                ..
            })
        )));
        assert!(acts.iter().any(|a| matches!(a,
            NativeAction::Event(NormalizedEvent { event: MiraEvent::AssistantText { text, .. }, .. }) if text == "hi")));
        assert!(acts.iter().any(|a| matches!(a,
            NativeAction::Event(NormalizedEvent { event: MiraEvent::ToolCall(c), .. }) if c.id == "t1")));
    }

    #[test]
    fn tool_result_arrives_on_a_user_envelope() {
        let acts = map_line(
            &serde_json::from_str(
                r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
            )
            .unwrap(),
        );
        assert!(acts.iter().any(|a| matches!(a,
            NativeAction::Event(NormalizedEvent { event: MiraEvent::ToolCallUpdate(c), .. })
            if c.id == "t1" && c.status == ToolCallStatus::Completed)));
    }

    #[test]
    fn non_tool_control_requests_keep_their_subtype() {
        // AskUserQuestion carries questions, not a tool name. The subtype
        // must survive as the name or the approval dialog reads "tool".
        let acts = map_line(
            &serde_json::from_str(
                r#"{"type":"control_request","request_id":"q1","request":
                {"subtype":"AskUserQuestion","questions":[{"question":"Proceed?"}]}}"#,
            )
            .unwrap(),
        );
        let p = acts.iter().find_map(|a| match a {
            NativeAction::Permission(p) => Some(p.clone()),
            _ => None,
        });
        let p = p.expect("a permission");
        assert_eq!(p.tool_name, "AskUserQuestion");
        assert!(p.input.get("questions").is_some());
    }

    #[test]
    fn control_request_becomes_a_permission() {
        let acts = map_line(
            &serde_json::from_str(
                r#"{"type":"control_request","request_id":"c1","request":
                {"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf /"}}}"#,
            )
            .unwrap(),
        );
        let p = acts.iter().find_map(|a| match a {
            NativeAction::Permission(p) => Some(p.clone()),
            _ => None,
        });
        assert_eq!(p.unwrap().tool_name, "Bash");
    }

    #[test]
    fn unknown_envelopes_are_surfaced_not_dropped() {
        let acts = map_line(&serde_json::from_str(r#"{"type":"brand_new_thing"}"#).unwrap());
        assert!(acts.iter().any(|a| matches!(
            a,
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::Unmodelled { .. },
                ..
            })
        )));
    }

    #[test]
    fn result_ends_the_turn_with_the_clis_own_label() {
        let acts = map_line(
            &serde_json::from_str(r#"{"type":"result","subtype":"success","is_error":false}"#)
                .unwrap(),
        );
        assert!(acts.contains(&NativeAction::TurnEnded {
            stop_reason: "success".into(),
            is_error: false
        }));
    }

    #[test]
    fn resume_is_only_passed_when_asked_for() {
        let p = std::path::PathBuf::from("claude");
        let fresh = launch(p.clone(), PermissionMode::Ask, None, None);
        assert!(!fresh.args.iter().any(|a| a == "--resume"));
        let resumed = launch(p, PermissionMode::Ask, None, Some("sid-1"));
        assert!(resumed.args.windows(2).any(|w| w == ["--resume", "sid-1"]));
    }

    /// A stand-in for the `claude` CLI: announces itself, asks for one
    /// permission, waits to be told what to do, then answers.
    fn fake_cli() -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(
            &path,
            r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-1","model":"fake-model","permissionMode":"default","slash_commands":["/clear"]}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"user"'*)
      echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo hi"}}]}}'
      echo '{"type":"control_request","request_id":"c1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}'
      ;;
    *'"type":"control_response"'*)
      echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"hi"}]}}'
      echo '{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"done"}]}}'
      echo '{"type":"result","subtype":"success","is_error":false}'
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cfg = LaunchConfig {
            program: path,
            args: Vec::new(),
            env: Default::default(),
            secret_env: Vec::new(),
            env_deny: Vec::new(),
        };
        (cfg, dir)
    }

    #[tokio::test]
    async fn a_native_session_answers_permissions_and_ends_the_turn() {
        let (cfg, _dir) = fake_cli();

        // The gate records what it was asked and allows, standing in for the
        // user clicking Allow.
        let asked = Arc::new(Mutex::new(Vec::new()));
        let gate_gate = asked.clone();
        let gate: PermissionGate = Arc::new(move |p: NativePermission| {
            let asked = gate_gate.clone();
            Box::pin(async move {
                asked.lock().await.push(p);
                PermissionDecision::from(true)
            })
        });

        let agent = NativeAgent::start(&cfg, gate, None).await.expect("spawn");
        let evs_rx = agent.events.lock().unwrap().take().expect("events");
        let mut ends_rx = agent.turn_end.lock().unwrap().take().expect("turn end");
        let _ = &evs_rx;

        // The init line must have been read for the session id to exist.
        let sid = wait_for(|| async { agent.session_id().await.or(None) }, "session id").await;
        assert_eq!(sid, "fake-1");

        agent.prompt("hello").await.unwrap();

        // Tool call, then the result that only arrives because the permission
        // was answered, then the turn ends.
        let mut saw_tool = false;
        let mut saw_result_update = false;
        let mut saw_text = false;
        // Bounded by the turn actually ending, with a hard ceiling so a
        // regression fails the test instead of hanging the suite.
        let mut evs = evs_rx;
        let end = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let e = evs.recv().await?;
                match &e.event {
                    MiraEvent::ToolCall(c) if c.id == "t1" => saw_tool = true,
                    MiraEvent::ToolCallUpdate(c) if c.id == "t1" => saw_result_update = true,
                    MiraEvent::AssistantText { text, .. } if text == "done" => saw_text = true,
                    _ => {}
                }
                if saw_tool && saw_result_update && saw_text {
                    return ends_rx.recv().await;
                }
            }
        })
        .await;
        let end = end.expect("turn must end within 20s");
        assert!(saw_tool, "expected the tool call to reach the transcript");
        assert!(
            saw_result_update,
            "the tool result only arrives after the permission is answered"
        );
        assert!(saw_text, "expected the agent's answer");
        assert_eq!(
            end.map(|e| e.stop_reason),
            Some("success".to_string()),
            "the turn must report how it ended"
        );

        // The gate was asked, with the tool input the user needs to judge it.
        let asked = asked.lock().await;
        assert_eq!(asked.len(), 1, "expected exactly one permission");
        assert_eq!(asked[0].tool_name, "Bash");
        assert_eq!(asked[0].input["command"], "echo hi");

        agent.shutdown().await;
    }

    /// The gate's answer is echoed back on stdin as `control_response`. If
    /// that write is malformed the CLI ignores it and the turn never ends, so
    /// assert the exact bytes rather than just "it did not hang".
    #[test]
    fn permission_responses_use_the_documented_control_shape() {
        let input = json!({"command": "echo hi"});
        let deny = permission_response("c1", &false.into(), &input);
        assert_eq!(deny["type"], "control_response");
        assert_eq!(deny["response"]["request_id"], "c1");
        assert_eq!(deny["response"]["response"]["behavior"], "deny");

        // An allow echoes the input: the CLI runs the tool with what the
        // host hands back, and treats an allow without it as malformed.
        let allow = permission_response("c2", &true.into(), &input);
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(allow["response"]["response"]["updatedInput"], input);
    }

    fn texts(acts: &[NativeAction]) -> Vec<String> {
        acts.iter()
            .filter_map(|a| match a {
                NativeAction::Event(NormalizedEvent {
                    event: MiraEvent::AssistantText { text, .. },
                    ..
                }) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn successful_results_identify_the_final_message() {
        let mut stream = StreamState::default();
        stream.map(&json!({"type":"assistant","message":{"id":"answer","content":[{"type":"text","text":"done"}]}}), None);
        let actions = stream.map(&json!({"type":"result","subtype":"success","is_error":false}), None);
        assert!(actions.iter().any(|action| matches!(action, NativeAction::Event(NormalizedEvent { event: MiraEvent::MessageMetadata { message_id, phase }, .. }) if message_id == "answer" && phase == "final_answer")));
        let next = stream.map(&json!({"type":"result","subtype":"success"}), None);
        assert!(!next.iter().any(|action| matches!(action, NativeAction::Event(NormalizedEvent { event: MiraEvent::MessageMetadata { .. }, .. }))));
    }

    #[test]
    fn streamed_text_is_not_repeated_when_the_block_lands() {
        let mut st = StreamState::default();
        let line = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let mut out = Vec::new();
        for l in [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi, "}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}},"parent_tool_use_id":null}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hi, there"}]},"parent_tool_use_id":null}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]},"parent_tool_use_id":null}"#,
        ] {
            out.extend(st.map(&line(l), None));
        }
        assert_eq!(
            texts(&out),
            vec!["Hi, ", "there"],
            "deltas once, whole block dropped"
        );
        assert!(out.iter().any(|a| matches!(a,
            NativeAction::Event(NormalizedEvent { event: MiraEvent::AssistantSnapshot {message_id, text}, .. })
            if message_id == "m1" && text == "Hi, there")));

        assert!(
            out.iter().any(|a| matches!(
                a,
                NativeAction::Event(NormalizedEvent {
                    event: MiraEvent::ToolCall(_),
                    ..
                })
            )),
            "a tool call in a streamed message still arrives"
        );
    }

    #[test]
    fn unstreamed_text_still_arrives_whole() {
        // An older CLI (or a message it chose not to stream) sends only the
        // envelope; dropping it would lose the reply entirely.
        let mut st = StreamState::default();
        let v = serde_json::from_str::<Value>(
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"whole"}]},"parent_tool_use_id":null}"#,
        )
        .unwrap();
        assert_eq!(texts(&st.map(&v, None)), vec!["whole"]);
    }

    #[test]
    fn subagent_narration_stays_out_of_the_parent_transcript() {
        let mut st = StreamState::default();
        for l in [
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sub says"}},"parent_tool_use_id":"toolu_task"}"#,
            r#"{"type":"assistant","message":{"id":"s1","content":[{"type":"text","text":"sub says"}]},"parent_tool_use_id":"toolu_task"}"#,
        ] {
            let v = serde_json::from_str::<Value>(l).unwrap();
            assert!(texts(&st.map(&v, None)).is_empty(), "{l}");
        }
    }

    #[test]
    fn allowing_for_the_session_never_writes_a_permanent_rule() {
        let suggestion = json!({"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"git log:*"}],
            "behavior":"allow","destination":"localSettings"});
        let session = session_permission_updates("Bash", std::slice::from_ref(&suggestion), false);
        assert_eq!(session[0]["destination"], "session");
        assert_eq!(
            session[0]["rules"][0]["ruleContent"], "git log:*",
            "the CLI's own scope is kept"
        );
        // "Always" keeps where the CLI wanted to write it.
        let always = session_permission_updates("Bash", &[suggestion], true);
        assert_eq!(always[0]["destination"], "localSettings");
        // No suggestion: a whole-tool session rule, so the choice sticks.
        let fallback = session_permission_updates("mcp__x__y", &[], false);
        assert_eq!(fallback[0]["rules"][0]["toolName"], "mcp__x__y");
        assert_eq!(fallback[0]["destination"], "session");
    }

    #[test]
    fn a_decision_carries_rewritten_input_and_rules() {
        let d = PermissionDecision {
            allow: true,
            updated_input: Some(json!({"answers": {"Proceed?": "Yes"}})),
            updated_permissions: vec![json!({"type": "addRules"})],
            message: None,
        };
        let r = permission_response("c3", &d, &json!({"questions": []}));
        assert_eq!(
            r["response"]["response"]["updatedInput"]["answers"]["Proceed?"],
            "Yes"
        );
        assert_eq!(
            r["response"]["response"]["updatedPermissions"][0]["type"],
            "addRules"
        );
        let deny = PermissionDecision {
            allow: false,
            message: Some("plan rejected".into()),
            ..Default::default()
        };
        assert_eq!(
            permission_response("c4", &deny, &Value::Null)["response"]["response"]["message"],
            "plan rejected"
        );
    }

    #[test]
    fn can_use_tool_carries_its_reason_and_suggestions() {
        let acts = map_line(&serde_json::from_str(
            r#"{"type":"control_request","request_id":"c9","request":{"subtype":"can_use_tool","tool_name":"Bash",
                "input":{"command":"ls"},"tool_use_id":"toolu_1","decision_reason":"Contains simple_expansion",
                "permission_suggestions":[{"type":"addRules"}]}}"#,
        ).unwrap());
        let Some(NativeAction::Permission(p)) = acts.into_iter().next() else {
            panic!("no permission")
        };
        assert_eq!(p.tool_use_id.as_deref(), Some("toolu_1"));
        assert_eq!(p.reason.as_deref(), Some("Contains simple_expansion"));
        assert_eq!(p.suggestions.len(), 1);
    }

    #[test]
    fn thinking_text_is_requested_and_empty_thinking_is_dropped() {
        let l = launch("claude".into(), PermissionMode::Ask, None, None);
        assert!(l
            .args
            .windows(2)
            .any(|w| w[0] == "--settings" && w[1].contains("showThinkingSummaries")));
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":"","signature":"x"}]}}"#,
        )
        .unwrap();
        assert!(
            map_line(&v).is_empty(),
            "a withheld thought is not an empty Thought row"
        );
    }

    #[test]
    fn a_turn_reports_context_fill_and_cost() {
        let mut st = StreamState::default();
        let line = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        st.map(&line(r#"{"type":"assistant","message":{"id":"m","content":[],"usage":{"input_tokens":10,"cache_read_input_tokens":20886,"cache_creation_input_tokens":8429,"output_tokens":3}},"parent_tool_use_id":null}"#), None);
        let acts = st.map(
            &line(
                r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0193,
            "modelUsage":{"claude-haiku-4-5":{"contextWindow":200000}}}"#,
            ),
            None,
        );
        let usage = acts.iter().find_map(|a| match a {
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::Usage { used, size, cost },
                ..
            }) => Some((*used, *size, cost.clone())),
            _ => None,
        });
        assert_eq!(
            usage,
            Some((29328, 200000, Some((0.0193, "USD".to_string()))))
        );
    }

    #[test]
    fn plan_limits_are_reported_and_only_a_real_limit_flags_the_turn() {
        let ok = map_line(&serde_json::from_str(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1790850000,
               "unifiedWindows":{"five_hour":{"utilization":0.39,"resetsAt":1790850000},"seven_day":{"utilization":0.18,"resetsAt":1791414000}}}}"#,
        ).unwrap());
        assert!(
            !ok.iter()
                .any(|a| matches!(a, NativeAction::RateLimited { .. })),
            "allowed is not a limit"
        );
        let windows = ok
            .iter()
            .find_map(|a| match a {
                NativeAction::Event(NormalizedEvent {
                    event: MiraEvent::Limits { windows },
                    ..
                }) => Some(windows.clone()),
                _ => None,
            })
            .expect("limits");
        assert_eq!(windows.len(), 2);
        assert!(windows
            .iter()
            .any(|w| w.name == "five_hour" && (w.utilization - 0.39).abs() < 1e-9));

        let hit = map_line(&serde_json::from_str(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":4102444800}}"#,
        ).unwrap());
        assert!(hit.iter().any(|a| matches!(a, NativeAction::RateLimited { resets_at: Some(r) } if r.starts_with("in "))));
    }

    #[test]
    fn the_launch_routes_prompts_to_the_host_over_stdio() {
        // Without `--permission-prompt-tool stdio` the CLI has no channel to
        // us and auto-denies every command that would prompt.
        let l = launch("claude".into(), PermissionMode::Ask, None, None);
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--permission-prompt-tool", "stdio"]));
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--permission-prompts", "host"]));
    }

    #[test]
    fn system_narration_is_not_surfaced_as_transcript_noise() {
        for line in [
            r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":50}"#,
            r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","message":"Contains simple_expansion"}"#,
            r#"{"type":"system","subtype":"post_turn_summary","status_category":"completed"}"#,
        ] {
            let v: Value = serde_json::from_str(line).unwrap();
            assert!(map_line(&v).is_empty(), "{line} should be quiet");
        }
    }

    #[test]
    fn claude_models_come_from_the_clis_own_data() {
        let data: Value = serde_json::from_str(
            r#"{"additionalModelOptionsCache":[
                {"value":"claude-fable-5-1[1m]","label":"Fable","description":"Most capable"}],
                "projects":{"/x":{"lastModelUsage":{"claude-opus-4-7":{},"claude-fable-5-1[1m]":{}}}}}"#,
        )
        .unwrap();
        let models = claude_models_from(Some(&data));
        let values: Vec<_> = models.iter().map(|m| m.value.as_str()).collect();
        // Documented aliases first — they never rot.
        assert_eq!(&values[..4], ["opus", "sonnet", "haiku", "fable"]);
        // Then the CLI's own menu entry, with its own label, exactly once.
        assert!(values.contains(&"claude-fable-5-1[1m]"));
        assert_eq!(
            values
                .iter()
                .filter(|v| v.to_string() == "claude-fable-5-1[1m]")
                .count(),
            1
        );
        let fable = models
            .iter()
            .find(|m| m.value == "claude-fable-5-1[1m]")
            .unwrap();
        assert_eq!(fable.label, "Fable");
        // Then usage history.
        assert!(values.contains(&"claude-opus-4-7"));
    }

    #[test]
    fn init_carries_the_full_model_list_with_current_first() {
        let models = vec![
            NativeModel {
                value: "opus".into(),
                label: "Opus — latest".into(),
                description: None,
            },
            NativeModel {
                value: "claude-opus-4-7".into(),
                label: "claude-opus-4-7".into(),
                description: None,
            },
        ];
        let acts = map_line_with(
            &serde_json::from_str(
                r#"{"type":"system","subtype":"init","session_id":"s","model":"claude-opus-5-5"}"#,
            )
            .unwrap(),
            Some(&models),
        );
        let opts = acts.iter().find_map(|a| match a {
            NativeAction::Event(NormalizedEvent {
                event: MiraEvent::ConfigOptions { options },
                ..
            }) => Some(options.clone()),
            _ => None,
        });
        let model = opts.unwrap().into_iter().find(|o| o.id == "model").unwrap();
        assert_eq!(model.current.as_deref(), Some("claude-opus-5-5"));
        let values: Vec<_> = model.values.iter().map(|v| v.value.as_str()).collect();
        assert_eq!(values[0], "claude-opus-5-5");
        assert!(values.contains(&"opus"));
        assert!(values.contains(&"claude-opus-4-7"));
    }

    #[test]
    fn fork_only_applies_alongside_resume() {
        let p = std::path::PathBuf::from("claude");
        let mut l = launch(p.clone(), PermissionMode::Ask, None, None);
        // Fork without a session to fork from is silently meaningless, so it
        // must not reach argv at all.
        apply_overrides(
            &mut l,
            &NativeOverrides {
                fork: true,
                ..Default::default()
            },
        );
        assert!(!l.args.iter().any(|a| a == "--fork-session"));
        apply_overrides(
            &mut l,
            &NativeOverrides {
                resume: Some("s1".into()),
                fork: true,
                ..Default::default()
            },
        );
        assert!(l
            .args
            .windows(3)
            .any(|w| w == ["--resume", "s1", "--fork-session"]));
    }

    #[test]
    fn overrides_replace_mode_and_add_model_and_resume() {
        let p = std::path::PathBuf::from("claude");
        let mut l = launch(p, PermissionMode::Ask, None, None);
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--permission-mode", "default"]));
        apply_overrides(
            &mut l,
            &NativeOverrides {
                model: Some("opus".into()),
                permission_mode: Some("acceptEdits".into()),
                resume: Some("sid-9".into()),
                fork: false,
            },
        );
        // Replaced in place, not appended a second flag.
        assert_eq!(
            l.args.iter().filter(|a| *a == "--permission-mode").count(),
            1
        );
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(l.args.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(l.args.windows(2).any(|w| w == ["--resume", "sid-9"]));
    }

    /// A stand-in that announces a session and then goes quiet without
    /// exiting — the shape of an agent waiting for its next prompt.
    fn quiet_cli() -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(
            &path,
            "#!/bin/sh\necho '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"quiet-1\"}'\nsleep 30\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            LaunchConfig {
                program: path,
                args: Vec::new(),
                env: Default::default(),
                secret_env: Vec::new(),
                env_deny: Vec::new(),
            },
            dir,
        )
    }

    /// The reported failure: the agent "stopped" ten minutes after the user
    /// stopped typing. The reader treated ten minutes of quiet as death,
    /// dropped its senders, and the pump reported an exit for a process that
    /// was still alive. Quiet is not dead: the loop must survive its own idle
    /// timeout while the child lives.
    #[tokio::test]
    async fn a_quiet_but_live_agent_survives_its_idle_timeout() {
        let (cfg, _dir) = quiet_cli();
        let gate: PermissionGate =
            Arc::new(|_p| Box::pin(async { PermissionDecision::from(true) }));
        let agent = NativeAgent::start_with_idle(&cfg, gate, Duration::from_secs(1), None)
            .await
            .expect("spawn");

        // Wait for the announcement first: under a loaded parallel test run
        // the stand-in can take a while to start, and that isn't what this
        // test is about.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while agent.session_id().await.is_none() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // Then past two idle windows with no output at all.
        tokio::time::sleep(Duration::from_millis(2500)).await;

        // Still alive, still has its session, and neither channel closed.
        assert!(is_alive(&agent.child).await, "the process never exited");
        assert_eq!(agent.session_id().await.as_deref(), Some("quiet-1"));
        assert!(
            agent.events.lock().unwrap().is_some() && agent.turn_end.lock().unwrap().is_some(),
            "channels must stay open while the agent lives"
        );
        // And it still takes a prompt after its silence.
        agent.prompt("hello?").await.expect("prompt after idle");
        agent.shutdown().await;
    }

    /// A stand-in that says nothing at all, like the real CLI before its
    /// first input: no `init`, no session id, no events of its own.
    fn silent_cli() -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            LaunchConfig {
                program: path,
                args: vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
                env: Default::default(),
                secret_env: Vec::new(),
                env_deny: Vec::new(),
            },
            dir,
        )
    }

    /// The reported bug: the picker stayed empty until the first message
    /// because everything waited on `init`, which the CLI only emits once
    /// spoken to. Startup must announce what it knows — models unmarked,
    /// launched mode current — with zero turns run.
    #[tokio::test]
    async fn startup_announces_models_and_modes_before_any_prompt() {
        let (cfg, _dir) = silent_cli();
        let gate: PermissionGate =
            Arc::new(|_p| Box::pin(async { PermissionDecision::from(true) }));
        let agent = NativeAgent::start(&cfg, gate, None).await.expect("spawn");

        let mut events = agent.events.lock().unwrap().take().expect("events rx");
        let mut saw_models = false;
        let mut saw_modes = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline && !(saw_models && saw_modes) {
            let Some(e) = events.recv().await else { break };
            match &e.event {
                MiraEvent::ConfigOptions { options }
                    if options
                        .iter()
                        .any(|o| o.id == "model" && !o.values.is_empty()) =>
                {
                    saw_models = true
                }
                MiraEvent::Modes { current, .. } if current == "acceptEdits" => saw_modes = true,
                _ => {}
            }
        }
        assert!(saw_models, "model list must arrive without any prompt");
        assert!(saw_modes, "launched mode must arrive without any prompt");
        // And honestly unmarked: no current model was claimed.
        assert!(
            agent.session_id().await.is_none(),
            "no session exists before the first prompt"
        );
        agent.shutdown().await;
    }

    #[test]
    fn permission_posture_maps_to_a_cli_mode() {
        assert_eq!(permission_mode_arg(PermissionMode::Ask), "default");
        assert_eq!(
            permission_mode_arg(PermissionMode::AcceptEdits),
            "acceptEdits"
        );
        assert_eq!(permission_mode_arg(PermissionMode::Auto), "default");
    }
}

fn claude_user_input(sid: &str, text: &str, images: &[mira_core::ImageData], steer_id: Option<&str>) -> Value {
    let mut content = vec![json!({"type":"text", "text":text})];
    content.extend(images.iter().map(|image| json!({"type":"image", "source":{
        "type":"base64", "media_type":image.media_type, "data":image.data
    }})));
    let mut input = json!({"type":"user", "message":{"role":"user", "content":content},
        "parent_tool_use_id":null, "session_id":sid});
    if let Some(id) = steer_id {
        input["priority"] = json!("now");
        input["uuid"] = json!(id);
    }
    input
}

/// Claude background tasks keep the runtime alive after its parent result.
fn native_background_work(v: &Value) -> Option<crate::runtime::RuntimeWork> {
    use crate::runtime::*;
    if v.get("type")?.as_str()? != "system" {
        return None;
    }
    let subtype = v.get("subtype")?.as_str()?;
    let status = match subtype {
        "task_started" | "task_progress" => WorkStatus::Running,
        "task_notification" => match v.get("status").and_then(Value::as_str) {
            Some("failed") => WorkStatus::Failed,
            Some("stopped") => WorkStatus::Cancelled,
            Some("completed") => WorkStatus::Completed,
            _ => return None,
        },
        _ => return None,
    };
    let id = v.get("task_id")?.as_str()?;
    if id.is_empty() {
        return None;
    }
    Some(RuntimeWork {
        id: format!("task:{id}"),
        native_thread_id: v
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kind: if matches!(v["task_type"].as_str(),Some("local_agent" | "agent" | "subagent")) { WorkKind::Subagent } else { WorkKind::Task },
        status,
        title: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod background_work_tests {
    use super::*;
    #[test]
    fn claude_result_does_not_forget_background_task_ownership() {
        let mut stream = StreamState::default();
        let mut activity = crate::runtime::RuntimeActivity::default();
        for frame in [
            json!({"type":"system","subtype":"task_started","task_id":"job","session_id":"s","description":"build"}),
            json!({"type":"result","subtype":"success","is_error":false,"session_id":"s"}),
        ] {
            for action in stream.map(&frame, None) {
                if let NativeAction::Event(NormalizedEvent {
                    event: MiraEvent::RuntimeWork(w),
                    ..
                }) = action
                {
                    activity.work(&w);
                }
            }
        }
        assert!(activity.has_pending_work());
        for action in stream.map(&json!({"type":"system","subtype":"task_notification","task_id":"job","status":"completed"}), None) {
            if let NativeAction::Event(NormalizedEvent { event: MiraEvent::RuntimeWork(w), .. }) = action { activity.work(&w); }
        }
        assert!(!activity.has_pending_work());
    }
}

#[cfg(test)]
mod native_input_lifecycle_tests {
    use super::*;
    #[test]
    fn steering_is_priority_input_with_real_image_blocks() {
        let input=claude_user_input("session","follow up",&[mira_core::ImageData {media_type:"image/png".into(),data:"aGVsbG8=".into(),source:None}],Some("request"));
        assert_eq!(input["priority"],"now");assert_eq!(input["uuid"],"request");
        assert_eq!(input["message"]["content"][1]["source"]["type"],"base64");
        assert_eq!(input["message"]["content"][1]["source"]["media_type"],"image/png");
        assert!(claude_user_input("s","normal",&[],None).get("priority").is_none());
    }
    #[test]
    fn roster_reconciliation_removes_finished_work_and_ignores_malformed_rosters() {
        let mut stream=StreamState::default();let mut activity=crate::runtime::RuntimeActivity::default();
        let frames=[json!({"type":"system","subtype":"background_tasks_changed","session_id":"s","tasks":[{"task_id":"one","task_type":"local_bash"}]}),
                    json!({"type":"system","subtype":"background_tasks_changed","tasks":null})];
        for frame in frames {for action in stream.map(&frame,None) { if let NativeAction::Event(NormalizedEvent {event:MiraEvent::RuntimeWork(work),..})=action {activity.work(&work);} }}
        assert!(activity.has_pending_work());
        for action in stream.map(&json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),None) {
            if let NativeAction::Event(NormalizedEvent {event:MiraEvent::RuntimeWork(work),..})=action {activity.work(&work);}
        }
        assert!(!activity.has_pending_work());
    }
    #[test]
    fn idle_notification_does_not_open_a_turn_but_root_init_does() {
        let mut stream=StreamState::default();
        let notification=stream.map(&json!({"type":"system","subtype":"task_notification","task_id":"one","status":"completed"}),None);
        assert!(!notification.iter().any(|a|matches!(a,NativeAction::Event(NormalizedEvent {event:MiraEvent::RuntimeTurn(_),..}))));
        let wake=stream.map(&json!({"type":"system","subtype":"init","session_id":"s"}),None);
        assert!(wake.iter().any(|a|matches!(a,NativeAction::Event(NormalizedEvent {event:MiraEvent::RuntimeTurn(t),..}) if t.running)));
        let end=stream.map(&json!({"type":"result","subtype":"success","session_id":"s"}),None);
        assert!(end.iter().any(|a|matches!(a,NativeAction::Event(NormalizedEvent {event:MiraEvent::RuntimeTurn(t),..}) if !t.running)));
    }
}
