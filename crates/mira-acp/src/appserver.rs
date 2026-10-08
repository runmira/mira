//! Codex via `codex app-server` (JSON-RPC over stdio).
//!
//! Unlike `codex exec`, which is one process per prompt, `app-server` holds
//! multi-turn threads in a single process: `thread/start` and `thread/resume`
//! for sessions, `turn/start` and `turn/interrupt` for turns, `model/list`
//! for the advertised catalog, `account/rateLimits/read` for usage state,
//! and `thread/compact/start` for compaction. Approvals arrive as
//! server-initiated JSON-RPC requests that must be answered.
//!

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::driver::{LaunchConfig, PermissionMode};
use crate::events::{
    AcpToolCallStatus as ToolCallStatus, AcpToolKind as ToolKind, EventSource, MiraEvent,
    NormalizedEvent, SessionConfigView, SessionModeView, ToolCallState, ToolContent,
};
use crate::native::{NativeError, NativePermission, PermissionGate, TurnEnd};
use crate::process::AgentProcess;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CODEX_MODEL: &str = "gpt-6-astra";

fn protocol_logging_enabled() -> bool {
    cfg!(debug_assertions)
        || std::env::var("MIRA_CODEX_PROTOCOL_LOG")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
}

fn redact_protocol_value(value: &Value) -> Value {
    fn should_redact(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        key.contains("authorization")
            || key.contains("token")
            || key.contains("api_key")
            || key.contains("apikey")
            || key.contains("password")
            || key.contains("secret")
            || key.contains("cookie")
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if should_redact(k) {
                            Value::String("[redacted]".to_string())
                        } else {
                            redact_protocol_value(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_protocol_value).collect()),
        Value::String(text) if text.starts_with("data:image/") => Value::String("[attached image]".into()),
        _ => value.clone(),
    }
}

fn protocol_log(direction: &str, method: &str, payload: &Value) {
    if !protocol_logging_enabled() {
        return;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let line = json!({
        "ts": ts,
        "direction": direction,
        "method": method,
        "payload": redact_protocol_value(payload),
    });
    let path = std::env::var("MIRA_CODEX_PROTOCOL_LOG_PATH")
        .unwrap_or_else(|_| "/tmp/mira-codex-appserver-protocol.ndjson".to_string());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", line);
    }
}

fn plan_entries_from_value(value: &Value) -> Vec<crate::events::PlanEntry> {
    let steps = value
        .get("plan")
        .or_else(|| value.get("steps"))
        .or_else(|| value.get("entries"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    steps
        .into_iter()
        .filter_map(|step| {
            if let Some(text) = step.as_str() {
                return Some(crate::events::PlanEntry {
                    content: text.to_string(),
                    priority: String::new(),
                    status: "pending".to_string(),
                });
            }
            let content = step
                .get("content")
                .or_else(|| step.get("text"))
                .or_else(|| step.get("description"))
                .and_then(Value::as_str)?
                .to_string();
            let status = step
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
                .to_string();
            let priority = step
                .get("priority")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(crate::events::PlanEntry {
                content,
                priority,
                status,
            })
        })
        .collect()
}

/// Codex's runtime modes, which are approval policy + sandbox, not models.
pub const RUNTIME_MODES: &[(&str, &str)] = &[
    ("plan", "Plan"),
    ("approval-required", "Ask each time"),
    ("auto-accept-edits", "Auto edits"),
    ("auto", "Auto everything"),
    ("full-access", "YOLO"),
];

/// Map a runtime mode onto the `thread/start` policy pair.
pub fn policy_for_mode(mode: &str) -> (&'static str, &'static str) {
    match mode {
        "auto-accept-edits" | "auto" => ("on-request", "workspace-write"),
        "full-access" => ("never", "danger-full-access"),
        // Plan mode explores but never mutates: reads run unprompted, and
        // Codex's own plan-mode prompt keeps it from trying to write.
        "plan" => ("on-request", "read-only"),
        _ => ("untrusted", "read-only"),
    }
}

/// The Codex collaboration mode a runtime mode turns on.
fn collaboration_mode_for(mode: &str) -> &'static str {
    if mode == "plan" { "plan" } else { "default" }
}

fn approvals_reviewer_for_mode(mode: &str) -> &'static str {
    match mode {
        "auto-accept-edits" | "auto" => "auto_review",
        _ => "user",
    }
}

fn sandbox_policy_for_wire(sandbox: &str) -> Value {
    match sandbox {
        "workspace-write" => json!({ "type": "workspaceWrite" }),
        "danger-full-access" => json!({ "type": "dangerFullAccess" }),
        _ => json!({ "type": "readOnly" }),
    }
}

fn codex_thread_config(mira_mcp: Option<&crate::session::MiraMcp>) -> Value {
    let mut config = serde_json::Map::new();
    config.insert("tools.update_plan.enabled".to_string(), json!(true));
    if let Some(m) = mira_mcp {
        let mut headers = serde_json::Map::new();
        headers.insert("Authorization".into(), json!(m.authorization()));
        headers.insert(
            crate::session::MIRA_TOOL_PROFILE_HEADER.into(),
            json!(crate::session::CODEX_TOOL_PROFILE),
        );
        let server = json!({ "url": m.url, "http_headers": headers });
        config.insert("mcp_servers.mira".to_string(), server);
    }
    Value::Object(config)
}

/// Map Mira's permission posture onto a runtime mode id.
pub fn mode_for_permission(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "approval-required",
        PermissionMode::AcceptEdits => "auto-accept-edits",
        PermissionMode::Auto => "auto",
    }
}

/// `thread/start` (or `thread/resume`) and its params.
///
/// `mira_mcp` is Mira's tool server for this chat (its browser, background
/// processes): passed per thread as a config override, the way Codex takes
/// MCP servers from `config.toml` (`[mcp_servers.<name>] url = …`), so the
/// user's own Codex config is untouched.
pub fn thread_open_params(
    cwd: Option<String>,
    policy: &str,
    sandbox: &str,
    model: Option<&str>,
    resume_thread_id: Option<&str>,
    mira_mcp: Option<&crate::session::MiraMcp>,
) -> (&'static str, Value) {
    let mut params = json!({
        "cwd": cwd.unwrap_or_else(|| ".".to_string()),
        "approvalPolicy": policy,
        "sandbox": sandbox,
    });
    params["model"] = json!(model
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "default")
        .unwrap_or(DEFAULT_CODEX_MODEL));
    params["config"] = codex_thread_config(mira_mcp);
    match resume_thread_id {
        Some(tid) => {
            params["threadId"] = json!(tid);
            params["excludeTurns"] = json!(true);
            ("thread/resume", params)
        }
        None => ("thread/start", params),
    }
}

fn initialize_params(name: &str) -> Value {
    json!({
        "clientInfo": { "name": name, "title": "Mira", "version": env!("CARGO_PKG_VERSION") },
        "capabilities": { "experimentalApi": true },
    })
}

/// The default collaboration mode's prompt. Only mode rules belong here: when
/// the model catalog ships its own text for a mode, Codex uses that and drops
/// this entirely, so anything about Mira's tools rides on `additionalContext`.
/// Plan mode sends none, leaving Codex's built-in plan prompt in charge.
const CODEX_DEFAULT_MODE_INSTRUCTIONS: &str = "<collaboration_mode># Collaboration Mode: Default

You are now in Default mode. Any previous instructions for other modes (e.g. Plan mode) are no longer active.

Your active mode changes only when new developer instructions with a different <collaboration_mode>...</collaboration_mode> change it; user requests or tool descriptions do not change mode by themselves. Known mode names are Default and Plan.

## request_user_input availability

Use the request_user_input tool only when it is listed in the available tools for this turn.

In Default mode, strongly prefer making reasonable assumptions and executing the user's request rather than stopping to ask questions. If you absolutely must ask and a reasonable assumption would be risky, ask the user through a question tool. Never write a multiple choice question as a textual assistant message.
</collaboration_mode>";

/// What Mira's tool server adds for Codex, described only when it is attached
/// so the prompt never names tools the turn doesn't have.
const CODEX_MIRA_TOOLS_CONTEXT: &str = "## Mira tools

You are running inside Mira. The `mira` MCP server adds what your own tools cannot do; keep using your native shell and patch tools for workspace work.

- Questions: when you must ask the user and request_user_input is not available, call Mira's `ask_user`. It shows a form in Mira and waits for the answer. Never claim a question was shown without calling a tool.
- Browser: `browser_open`, `browser_snapshot`, `browser_click`, `browser_type`, `browser_press` and `browser_screenshot` drive the browser the user watches in Mira's browser pane. Prefer them over starting a browser of your own. Native computer use has separate host permissions; a denial there does not mean Mira's browser is unavailable.
- Long-running commands (dev servers, watchers): `run_background`, `read_output` and `kill_background`. Their handles belong to the chat, show in Mira's Processes window, and survive an interrupted turn; native exec_command handles may expire when Codex restarts.
- `delegate_task` hands a self-contained task to another engine and returns its answer.
- Secrets: when you need an API key or token, call `request_secret`. The user enters it privately and you get a file path to read it from; never ask for secrets in chat.
- Visuals: when a chart, table, diagram or mockup says more than prose, publish a self-contained page with `html_render`. It shows above your reply, so don't restate it.

Tool names may carry a harness prefix such as `mcp__mira__ask_user`. If a Mira tool fails, say so rather than pretending it ran.";

/// `turn/start.additionalContext`: Codex renders each entry as its own
/// developer message and resends it only when it changes, so unlike the mode
/// prompt it survives models that ship their own collaboration text.
fn codex_additional_context(mira_tools: bool) -> Option<Value> {
    mira_tools.then(|| {
        json!({
            "mira_tools": { "kind": "application", "value": CODEX_MIRA_TOOLS_CONTEXT },
        })
    })
}

fn codex_prompt_input(text: &str, images: &[mira_core::ImageData]) -> Value {
    let mut input = Vec::new();
    if !text.trim().is_empty() { input.push(json!({"type": "text", "text": text})); }
    for image in images { input.push(json!({"type": "image", "url": image.data_url()})); }
    json!(input)
}

fn turn_start_params(
    thread_id: &str,
    text: &str,
    model: Option<&str>,
    cwd: Option<&str>,
    policy: &str,
    sandbox: &str,
    approvals_reviewer: &str,
    effort: Option<&str>,
    service_tier: Option<&str>,
    runtime_mode: &str,
    mira_tools: bool,
) -> Value {
    let model = model
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "default")
        .unwrap_or(DEFAULT_CODEX_MODEL);
    let effort = effort
        .map(str::trim)
        .filter(|e| !e.is_empty() && *e != "default")
        .unwrap_or("medium");
    let mut params = json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": text }],
        "model": model,
        "summary": "detailed",
        "approvalPolicy": policy,
        "approvalsReviewer": approvals_reviewer,
        "sandboxPolicy": sandbox_policy_for_wire(sandbox),
        "effort": effort,
    });
    let collaboration = collaboration_mode_for(runtime_mode);
    let mut settings = json!({ "model": model, "reasoning_effort": effort });
    if collaboration == "default" {
        settings["developer_instructions"] = json!(CODEX_DEFAULT_MODE_INSTRUCTIONS);
    }
    params["collaborationMode"] = json!({ "mode": collaboration, "settings": settings });
    if let Some(context) = codex_additional_context(mira_tools) {
        params["additionalContext"] = context;
    }
    if let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) {
        params["cwd"] = json!(cwd);
    }
    if let Some(tier) = service_tier
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "default")
    {
        params["serviceTier"] = json!(tier);
    }
    params
}

fn codex_rate_limit_windows(v: &Value) -> Vec<crate::events::LimitWindow> {
    fn pct_to_unit(v: &Value) -> Option<f64> {
        let pct = v.get("usedPercent")?.as_f64()?;
        Some((pct / 100.0).clamp(0.0, 1.0))
    }

    fn window(name: String, v: &Value) -> Option<crate::events::LimitWindow> {
        Some(crate::events::LimitWindow {
            name,
            utilization: pct_to_unit(v)?,
            resets_at: v.get("resetsAt").and_then(Value::as_i64),
        })
    }

    let root = v.get("rateLimits").unwrap_or(v);
    let mut out = Vec::new();
    if let Some(w) = root
        .get("primary")
        .and_then(|w| window("five_hour".to_string(), w))
    {
        out.push(w);
    }
    if let Some(w) = root
        .get("secondary")
        .and_then(|w| window("seven_day".to_string(), w))
    {
        out.push(w);
    }

    if out.is_empty() {
        if let Some(by_limit) = v.get("rateLimitsByLimitId").and_then(Value::as_object) {
            for (limit_id, limit) in by_limit {
                let prefix = if limit_id == "codex" {
                    ""
                } else {
                    limit_id.as_str()
                };
                if let Some(w) = limit.get("primary").and_then(|w| {
                    window(
                        if prefix.is_empty() {
                            "five_hour".to_string()
                        } else {
                            format!("{prefix}_five_hour")
                        },
                        w,
                    )
                }) {
                    out.push(w);
                }
                if let Some(w) = limit.get("secondary").and_then(|w| {
                    window(
                        if prefix.is_empty() {
                            "seven_day".to_string()
                        } else {
                            format!("{prefix}_seven_day")
                        },
                        w,
                    )
                }) {
                    out.push(w);
                }
            }
        }
    }
    out
}

/// The argv for an app-server session. One process hosts the thread.
pub fn launch_args() -> Vec<String> {
    vec!["app-server".to_string()]
}

/// A launch config for presence checks and spawning: the CLI plus the
/// subcommand, no more. Everything else is negotiated over RPC.
pub fn launch(program: std::path::PathBuf) -> crate::driver::LaunchConfig {
    crate::driver::LaunchConfig {
        program,
        args: launch_args(),
        env: Default::default(),
        secret_env: Vec::new(),
        env_deny: Vec::new(),
    }
}

/// A model from `model/list`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodexModel {
    pub value: String,
    pub label: String,
}

/// What one notification means.
///
/// `Event` dwarfs the rest, but each action is mapped and consumed at once
/// — never stored in bulk — so boxing it would only add an allocation per
/// event.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum AppServerAction {
    SessionId(String),
    Event(NormalizedEvent),
    TurnEnded {
        stop_reason: String,
        is_error: bool,
    },
    /// Usage state changed; surfaced on request, not as a transcript event.
    RateLimits(Value),
    /// The server moved the thread to another model. Carries the new id so
    /// the picker can mark current — the one piece `thread/start` withholds.
    ModelRerouted {
        model: String,
    },
}

fn variant(name: &str) -> EventSource {
    EventSource::Acp {
        variant: name.to_string(),
    }
}

/// Map one app-server notification to actions.
///
/// Pure and total like its stream-json cousin: unknown shapes become
/// `Unmodelled`, never silence. Field access is defensive throughout —
/// several shapes below are inferred from names, not observed.
pub fn map_notification(method: &str, p: &Value) -> Vec<AppServerAction> {
    protocol_log("in", method, p);
    let item = p.get("item");
    let item_id = item
        .and_then(|i| i.get("id"))
        .and_then(Value::as_str)
        .or_else(|| p.get("itemId").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();

    if let Some(request) = async_question(p) {
        return vec![AppServerAction::Event(NormalizedEvent {
            source: variant("async_question"),
            event: MiraEvent::RuntimeRequest(request),
        })];
    }
    let runtime = runtime_notifications(method, p);
    if matches!(
        method,
        "turn/started" | "thread/goal/updated" | "thread/goal/cleared"
    ) && !runtime.is_empty()
    {
        return runtime
            .into_iter()
            .map(|event| {
                AppServerAction::Event(NormalizedEvent {
                    source: variant("runtime"),
                    event,
                })
            })
            .collect();
    }
    let mut actions = match method {
        "thread/started" => {
            let tid = p
                .get("threadId")
                .or_else(|| p.get("thread").and_then(|t| t.get("id")))
                .or_else(|| p.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if tid.is_empty() {
                vec![]
            } else {
                vec![AppServerAction::SessionId(tid.to_string())]
            }
        }

        "item/agentMessage/delta" => text_event(
            item_id,
            p.get("delta").and_then(Value::as_str).unwrap_or(""),
            false,
        ),
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => text_event(
            item_id,
            p.get("delta")
                .or_else(|| p.get("text"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            true,
        ),
        // A plan taking shape. Kept as thought rather than a plan panel:
        // without the full entry list a panel would flicker through
        // half-states.
        "item/plan/delta" => text_event(
            item_id,
            p.get("delta").and_then(Value::as_str).unwrap_or(""),
            true,
        ),

        "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
            let text = p.get("delta").and_then(Value::as_str).unwrap_or("");
            if item_id.is_empty() || text.is_empty() {
                vec![]
            } else {
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("tool_call_update"),
                    event: MiraEvent::ToolOutputDelta {
                        id: item_id,
                        text: text.to_string(),
                    },
                })]
            }
        }

        "item/started" => match item {
            Some(i) => map_item(i, true),
            None => vec![],
        },
        "item/completed" => match item {
            Some(i) => map_item(i, false),
            None => vec![],
        },
        "item/plan/updated" | "plan/updated" => {
            let entries = plan_entries_from_value(p);
            if entries.is_empty() {
                vec![]
            } else {
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("plan"),
                    event: MiraEvent::Plan { entries },
                })]
            }
        }

        "mcpServer/startupStatus/updated" if p.get("name").and_then(Value::as_str) == Some("mira") && p.get("status").and_then(Value::as_str) == Some("failed") => vec![AppServerAction::Event(NormalizedEvent {
            source: variant("mcp"), event: MiraEvent::Activity { kind: "tools".into(), title: "Mira tools unavailable".into(), detail: p.get("error").and_then(Value::as_str).unwrap_or("The Mira tool server failed to start. Reconnect this agent to retry.").into() },
        })],
        "turn/completed" => {
            let turn = p.get("turn").unwrap_or(p);
            let failed = turn.get("status").and_then(Value::as_str) == Some("failed") || turn.get("error").is_some_and(|e| !e.is_null());
            let error = turn.get("error").map(Value::to_string).unwrap_or_default().to_lowercase();
            let limited = ["usagelimit", "usage_limit", "ratelimit", "rate_limit", "rate limit"].iter().any(|s| error.contains(s));
            vec![AppServerAction::TurnEnded { stop_reason: if limited { "rate_limited" } else if failed { "error" } else if turn.get("status").and_then(Value::as_str) == Some("interrupted") { "cancelled" } else { "completed" }.into(), is_error: failed }]
        },
        "turn/aborted" => vec![AppServerAction::TurnEnded {
            stop_reason: "cancelled".to_string(),
            is_error: true,
        }],
        "error" if p.get("willRetry").or_else(|| p.get("will_retry")).and_then(Value::as_bool) == Some(true) => vec![AppServerAction::Event(NormalizedEvent {
            source: variant("retry"), event: MiraEvent::Activity { kind: "retry".into(), title: "Retrying request".into(), detail: p.pointer("/error/message").and_then(Value::as_str).unwrap_or("The provider will retry this request.").into() },
        })],
        "error" => vec![AppServerAction::TurnEnded {
            stop_reason: if p.to_string().to_lowercase().contains("usagelimit") || p.to_string().to_lowercase().contains("usage_limit") || p.to_string().to_lowercase().contains("rate_limit") || p.to_string().to_lowercase().contains("rate limit") { "rate_limited" } else { "error" }.to_string(),
            is_error: true,
        }],

        "thread/tokenUsage/updated" => {
            let u = p.get("tokenUsage").unwrap_or(&Value::Null);
            let last = u
                .get("last")
                .and_then(|l| l.get("totalTokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let window = u
                .get("modelContextWindow")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let mut out = Vec::new();
            if last != 0 || window != 0 {
                out.push(AppServerAction::Event(NormalizedEvent {
                    source: variant("usage"),
                    event: MiraEvent::Usage {
                        used: last,
                        size: window,
                        cost: None,
                    },
                }));
            }
            // The thread's running token totals, for the usage ledger.
            // OpenAI counts cached input inside `inputTokens`; split it out
            // so fresh and cached input are priced apart.
            if let Some(t) = u.get("total") {
                let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
                let cached = n("cachedInputTokens");
                let spend = crate::events::ModelSpend {
                    model: String::new(),
                    input_tokens: n("inputTokens").saturating_sub(cached),
                    output_tokens: n("outputTokens"),
                    cached_input_tokens: cached,
                    cost_usd: None,
                };
                if spend.input_tokens + spend.output_tokens + spend.cached_input_tokens > 0 {
                    out.push(AppServerAction::Event(NormalizedEvent {
                        source: variant("spend"),
                        event: MiraEvent::Spend {
                            session: p
                                .get("threadId")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            models: vec![spend],
                        },
                    }));
                }
            }
            out
        }

        "account/rateLimits/updated" => vec![AppServerAction::RateLimits(p.clone())],

        "model/rerouted" => {
            let model = p
                .get("model")
                .and_then(|m| {
                    m.as_str().map(str::to_string).or_else(|| {
                        m.get("id")
                            .or_else(|| m.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .unwrap_or_default();
            if model.is_empty() {
                vec![]
            } else {
                vec![AppServerAction::ModelRerouted { model }]
            }
        }

        "thread/name/updated" => match p.get("name").and_then(Value::as_str) {
            Some(name) => vec![AppServerAction::Event(NormalizedEvent {
                source: variant("session_info"),
                event: MiraEvent::SessionInfo {
                    title: Some(name.to_string()),
                    updated_at: None,
                },
            })],
            None => vec![],
        },

        _ => vec![AppServerAction::Event(NormalizedEvent {
            source: EventSource::Unmodelled {
                method: method.to_string(),
            },
            event: MiraEvent::Unmodelled {
                source: EventSource::Unmodelled {
                    method: method.to_string(),
                },
                reason: p.to_string(),
            },
        })],
    };
    actions.extend(runtime.into_iter().map(|event| {
        AppServerAction::Event(NormalizedEvent {
            source: variant("runtime"),
            event,
        })
    }));
    actions
}

fn async_question(p: &Value) -> Option<crate::runtime::RuntimeRequest> {
    use crate::runtime::*;
    let item = p.get("item")?;
    if item.get("delivery")?.as_str()? != "async" {
        return None;
    }
    let native_id = item.get("id")?.as_str()?.to_owned();
    let native_thread_id = p.get("threadId")?.as_str()?.to_owned();
    if native_id.is_empty() || native_thread_id.is_empty() {
        return None;
    }
    let questions: Vec<_> = item
        .get("questions")?
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(index, q)| {
            let question = q
                .get("title")
                .or_else(|| q.get("question"))?
                .as_str()?
                .to_owned();
            if question.trim().is_empty() {
                return None;
            }
            Some(RuntimeQuestion {
                id: q
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| index.to_string()),
                question,
                header: q.get("header").and_then(Value::as_str).map(str::to_owned),
                options: q
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| {
                                v.as_str()
                                    .or_else(|| v.get("label").and_then(Value::as_str))
                            })
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                required: q.get("required").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect();
    if questions.is_empty() {
        return None;
    }
    Some(RuntimeRequest {
        native_id,
        native_thread_id,
        native_turn_id: p.get("turnId").and_then(Value::as_str).map(str::to_owned),
        questions,
        response_capability: ResponseCapability::Message,
    })
}

fn runtime_notifications(method: &str, p: &Value) -> Vec<MiraEvent> {
    use crate::runtime::*;
    let thread = p
        .get("threadId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method == "turn/started" || method == "turn/completed" || method == "turn/aborted" {
        let turn = p
            .get("turn")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if thread.is_empty() || turn.is_empty() {
            return vec![];
        }
        return vec![MiraEvent::RuntimeTurn(RuntimeTurn {
            native_thread_id: thread.into(),
            native_turn_id: turn.into(),
            running: method == "turn/started",
        })];
    }
    if matches!(method, "item/started" | "item/completed") {
        let item = p.get("item").unwrap_or(&Value::Null);
        if item.get("type").and_then(Value::as_str) == Some("subAgentActivity") {
            let id = item
                .get("agentThreadId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if id.is_empty() {
                return vec![];
            }
            let status = match item.get("kind").and_then(Value::as_str) {
                Some("started") => WorkStatus::Running,
                Some("completed") => WorkStatus::Completed,
                Some("interrupted") => WorkStatus::Cancelled,
                _ => return vec![],
            };
            return vec![MiraEvent::RuntimeWork(RuntimeWork {
                id: format!("subagent:{id}"),
                native_thread_id: Some(id.into()),
                kind: WorkKind::Subagent,
                status,
                title: item
                    .get("agentPath")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })];
        }
        if item.get("type").and_then(Value::as_str) == Some("collabAgentToolCall") {
            if let Some(states) = item.get("agentsStates").and_then(Value::as_object) {
                return states
                    .iter()
                    .map(|(id, state)| {
                        let status = match state.get("status").and_then(Value::as_str) {
                            Some("pendingInit") => WorkStatus::Pending,
                            Some("running") => WorkStatus::Running,
                            Some("completed") => WorkStatus::Completed,
                            Some("interrupted" | "shutdown") => WorkStatus::Cancelled,
                            Some("errored" | "notFound") => WorkStatus::Failed,
                            _ => WorkStatus::Waiting,
                        };
                        MiraEvent::RuntimeWork(RuntimeWork {
                            id: format!("subagent:{id}"),
                            native_thread_id: Some(id.clone()),
                            kind: WorkKind::Subagent,
                            status,
                            title: None,
                        })
                    })
                    .collect();
            }
        }
    }
    if method == "thread/goal/updated" || method == "thread/goal/cleared" {
        if thread.is_empty() {
            return vec![];
        }
        let goal = p.get("goal").unwrap_or(&Value::Null);
        let status = if method == "thread/goal/cleared" {
            WorkStatus::Cancelled
        } else {
            match goal.get("status").and_then(Value::as_str) {
                Some("active") => WorkStatus::Running,
                Some("paused" | "blocked" | "usageLimited" | "budgetLimited") => WorkStatus::Paused,
                Some("completed" | "complete" | "met") => WorkStatus::Completed,
                Some("failed" | "exhausted") => WorkStatus::Failed,
                _ => WorkStatus::Waiting,
            }
        };
        return vec![MiraEvent::RuntimeWork(RuntimeWork {
            id: format!("goal:{thread}"),
            native_thread_id: Some(thread.into()),
            kind: WorkKind::Goal,
            status,
            title: goal
                .get("objective")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })];
    }
    vec![]
}

fn text_event(item_id: String, text: &str, thought: bool) -> Vec<AppServerAction> {
    if text.is_empty() {
        return vec![];
    }
    let mid = (!item_id.is_empty()).then_some(item_id);
    let event = if thought {
        MiraEvent::AgentThought {
            message_id: mid,
            text: text.to_string(),
        }
    } else {
        MiraEvent::AssistantText {
            message_id: mid,
            text: text.to_string(),
        }
    };
    vec![AppServerAction::Event(NormalizedEvent {
        source: variant("agent_message_chunk"),
        event,
    })]
}

/// Map an `item` object from `item/started` / `item/completed`.
fn map_item(i: &Value, started: bool) -> Vec<AppServerAction> {
    let id = i.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.is_empty() {
        return vec![];
    }
    let itype = i.get("type").and_then(Value::as_str).unwrap_or("");
    // Mira emits these tools with its own approval/prompt IDs. Rendering the
    // app-server's wrapper too would duplicate every workspace call and card.
    if itype == "mcpToolCall" && i.get("server").and_then(Value::as_str) == Some("mira")
        && matches!(i.get("tool").and_then(Value::as_str), Some("ask_user" | "plan" | "read_file" | "write_file" | "edit_file" | "grep" | "glob" | "bash")) {
        return vec![];
    }
    match itype {
        "command_execution" | "commandExecution" | "file_change" | "fileChange" | "mcpToolCall"
        | "tool_call" => {
            let tool_name = i
                .get("tool")
                .or_else(|| i.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(itype);
            if matches!(tool_name, "update_plan" | "plan" | "todo_write") {
                let entries = plan_entries_from_value(i.get("input").unwrap_or(i));
                if !entries.is_empty() {
                    return vec![AppServerAction::Event(NormalizedEvent {
                        source: variant("plan"),
                        event: MiraEvent::Plan { entries },
                    })];
                }
            }
            let title = i
                .get("command")
                .and_then(Value::as_str)
                .or_else(|| i.get("tool").and_then(Value::as_str))
                .or_else(|| i.get("name").and_then(Value::as_str))
                .unwrap_or(itype);
            let kind = match itype {
                "file_change" | "fileChange" => ToolKind::Edit,
                "command_execution" | "commandExecution" => ToolKind::Execute,
                _ => ToolKind::Other,
            };
            if started {
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("tool_call"),
                    event: MiraEvent::ToolCall(ToolCallState {
                        id: id.to_string(),
                        title: title.to_string(),
                        name: Some(if itype == "mcpToolCall" { tool_name } else { itype }.to_string()),
                        kind: Some(kind),
                        status: ToolCallStatus::InProgress,
                        content: Vec::new(),
                        locations: Vec::new(),
                        raw_input: i
                            .get("command")
                            .cloned()
                            .or_else(|| i.get("input").cloned())
                            .or_else(|| i.get("arguments").cloned()),
                        raw_output: None,
                    }),
                })]
            } else {
                let mut content = Vec::new();
                if let Some(out) = i
                    .get("aggregated_output")
                    .or_else(|| i.get("aggregatedOutput"))
                    .or_else(|| i.get("output"))
                    .and_then(Value::as_str)
                {
                    content.push(ToolContent::Content {
                        text: out.to_string(),
                    });
                }
                let failed = i
                    .get("exit_code")
                    .or_else(|| i.get("exitCode"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    != 0
                    || i.get("status").and_then(Value::as_str) == Some("failed");
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("tool_call_update"),
                    event: MiraEvent::ToolCallUpdate(ToolCallState {
                        id: id.to_string(),
                        title: title.to_string(),
                        name: Some(if itype == "mcpToolCall" { tool_name } else { itype }.to_string()),
                        kind: Some(kind),
                        status: if failed {
                            ToolCallStatus::Failed
                        } else {
                            ToolCallStatus::Completed
                        },
                        content,
                        locations: Vec::new(),
                        raw_input: None,
                        raw_output: i.get("output").cloned(),
                    }),
                })]
            }
        }
        // Completed prose reconciles partial deltas at the presentation boundary.
        "agent_message" | "agentMessage" if !started && !id.is_empty() => {
            let text = i.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                return vec![];
            }
            let mut events = vec![AppServerAction::Event(NormalizedEvent {
                source: variant("agent_message_snapshot"),
                event: MiraEvent::AssistantSnapshot {
                    message_id: id.to_string(),
                    text: text.to_string(),
                },
            })];
            if let Some(phase) = i.get("phase").and_then(Value::as_str) {
                events.push(AppServerAction::Event(NormalizedEvent { source: variant("message_metadata"), event: MiraEvent::MessageMetadata { message_id: id.into(), phase: phase.into() } }));
            }
            events
        }
        "agent_message" | "agentMessage" | "reasoning" => vec![],
        _ => vec![],
    }
}

async fn remember_native_item(
    method: &str,
    params: &Value,
    items: &Arc<Mutex<HashMap<String, Value>>>,
) {
    if !matches!(method, "item/started" | "item/completed") {
        return;
    }
    let Some(item) = params.get("item") else {
        return;
    };
    let Some(id) = item.get("id").and_then(Value::as_str) else {
        return;
    };
    if id.is_empty() {
        return;
    }
    items.lock().await.insert(id.to_string(), item.clone());
}

fn first_string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            return Some(s);
        }
    }
    None
}

fn patch_input_from_params(params: &Value) -> Value {
    let item = params.get("item").unwrap_or(&Value::Null);
    let input = item.get("input").unwrap_or(&Value::Null);
    let patch = first_string_field(params, &["changes", "patch", "diff"])
        .or_else(|| first_string_field(item, &["changes", "patch", "diff"]))
        .or_else(|| first_string_field(input, &["changes", "patch", "diff"]));
    let path = first_string_field(params, &["path", "file", "filePath", "file_path"])
        .or_else(|| first_string_field(item, &["path", "file", "filePath", "file_path"]));

    let mut out = serde_json::Map::new();
    if let Some(path) = path {
        out.insert("path".to_string(), Value::String(path.to_string()));
    }
    if let Some(patch) = patch {
        out.insert("patch".to_string(), Value::String(patch.to_string()));
    }
    if out.is_empty() {
        params
            .get("reason")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|reason| json!({ "patch": reason }))
            .unwrap_or_else(|| json!({ "summary": "Patch approval requested" }))
    } else {
        Value::Object(out)
    }
}

async fn enrich_permission_params(
    method: &str,
    params: Value,
    items: &Arc<Mutex<HashMap<String, Value>>>,
) -> Value {
    if !matches!(
        method,
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
    ) {
        return params;
    }
    let Some(item_id) = params.get("itemId").and_then(Value::as_str) else {
        return params;
    };
    let Some(item) = items.lock().await.get(item_id).cloned() else {
        return params;
    };
    let mut merged = match params {
        Value::Object(map) => map,
        other => return other,
    };
    merged
        .entry("item".to_string())
        .or_insert_with(|| item.clone());

    if method == "item/commandExecution/requestApproval" {
        if !merged.contains_key("command") {
            if let Some(command) = item
                .get("command")
                .cloned()
                .or_else(|| item.get("input").and_then(|i| i.get("command")).cloned())
            {
                merged.insert("command".to_string(), command);
            }
        }
    } else {
        for key in [
            "changes",
            "patch",
            "diff",
            "path",
            "file",
            "filePath",
            "file_path",
        ] {
            if merged.contains_key(key) {
                continue;
            }
            if let Some(v) = item
                .get(key)
                .cloned()
                .or_else(|| item.get("input").and_then(|i| i.get(key)).cloned())
            {
                merged.insert(key.to_string(), v);
            }
        }
    }

    Value::Object(merged)
}

/// Build the permission the gate decides on, from an approval request.
pub fn permission_from_request(
    method: &str,
    request_id: &str,
    params: &Value,
) -> Option<NativePermission> {
    let (tool, input) = match method {
        "item/commandExecution/requestApproval" => (
            "shell",
            params
                .get("command")
                .cloned()
                .unwrap_or_else(|| params.clone()),
        ),
        "item/fileChange/requestApproval" => ("apply_patch", patch_input_from_params(params)),
        "item/permissions/requestApproval" => ("permission", params.clone()),
        // An MCP server asking through Codex: a form to fill, a link to
        // open, or a bare confirmation (how Codex approves MCP tool calls).
        "mcpServer/elicitation/request" => ("mcp_elicitation", params.clone()),
        "item/tool/requestUserInput" => ("request_user_input", params.clone()),
        _ => return None,
    };
    Some(NativePermission {
        request_id: request_id.to_string(),
        tool_name: tool.to_string(),
        tool_use_id: params
            .get("itemId")
            .or_else(|| params.get("callId"))
            .or_else(|| params.get("toolCallId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        reason: params
            .get("reason")
            .or_else(|| params.get("message"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string),
        input,
        ..Default::default()
    })
}

/// Return actual user answers for questions, never an inferred first option.
pub fn approval_response(
    method: &str,
    params: &Value,
    decision: &crate::native::PermissionDecision,
) -> Value {
    if method == "item/tool/requestUserInput" {
        let mut answers = serde_json::Map::new();
        if decision.allow {
            if let (Some(questions), Some(given)) = (
                params.get("questions").and_then(Value::as_array),
                decision
                    .updated_input
                    .as_ref()
                    .and_then(|v| v.get("answers"))
                    .and_then(Value::as_object),
            ) {
                for q in questions {
                    if let Some(id) = q.get("id").and_then(Value::as_str) {
                        if let Some(answer) = given.get(id) {
                            answers.insert(id.to_owned(), answer.clone());
                        }
                    }
                }
            }
        }
        return json!({ "answers": answers });
    }
    if method == "mcpServer/elicitation/request" {
        // MCP's own answer shape. Form values ride in `updated_input.content`.
        if !decision.allow {
            return json!({ "action": "decline" });
        }
        let mut reply = json!({ "action": "accept" });
        if let Some(content) = decision
            .updated_input
            .as_ref()
            .and_then(|v| v.get("content"))
            .filter(|c| c.is_object())
        {
            reply["content"] = content.clone();
        }
        return reply;
    }
    if method == "item/permissions/requestApproval" {
        // Permission-profile grants have their own response schema. Limit
        // the answer to what was requested and to this turn, so selecting
        // Ask again never inherits a standing process-level grant.
        let mut permissions = serde_json::Map::new();
        if decision.allow {
            if let Some(requested) = params.get("permissions").and_then(Value::as_object) {
                for key in ["fileSystem", "network"] {
                    if let Some(value) = requested.get(key) {
                        permissions.insert(key.to_owned(), value.clone());
                    }
                }
            }
        }
        return json!({ "permissions": permissions, "scope": "turn" });
    }
    json!({ "decision": if decision.allow { "accept" } else { "decline" } })
}

/// Prefer the inference model slug over a catalog id; tolerate older catalogs.
pub fn parse_model_entry(e: &Value) -> Option<CodexModel> {
    if e.get("hidden").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let value = e
        .get("model")
        .or_else(|| e.get("slug"))
        .or_else(|| e.get("value"))
        .or_else(|| e.get("id"))
        .and_then(Value::as_str)?;
    let label = e
        .get("name")
        .or_else(|| e.get("displayName"))
        .or_else(|| e.get("label"))
        .and_then(Value::as_str)
        .unwrap_or(value);
    Some(CodexModel {
        value: value.to_string(),
        label: label.to_string(),
    })
}

/// The modes this transport offers, as ACP-shaped views.
pub fn mode_views() -> Vec<SessionModeView> {
    RUNTIME_MODES
        .iter()
        .map(|(id, name)| SessionModeView {
            id: id.to_string(),
            name: name.to_string(),
            description: Some(
                match *id {
                    "plan" => "Plan only, make no changes",
                    "approval-required" => "Ask before running commands or editing files",
                    "auto-accept-edits" => "Auto-accept file edits, still ask for commands",
                    "auto" => "Let the agent decide within the workspace",
                    _ => "No prompts; everything is allowed",
                }
                .to_string(),
            ),
        })
        .collect()
}

/// What the app-server handshake reveals: identity, not capability.
#[derive(Clone, Debug, PartialEq)]
pub struct AppServerProbe {
    pub user_agent: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub models: Vec<CodexModel>,
}

/// Probe a Codex CLI without opening a thread: handshake plus account.
///
/// Short-lived by construction — the process is shut down before returning,
/// so a probe never leaves an app-server running. Anything failing degrades
/// to `None` rather than an error, because absence and failure report the
/// same way to the caller.
pub async fn probe_handshake(launch: &LaunchConfig) -> Option<AppServerProbe> {
    use crate::conn::NullCallbacks;
    let process = AgentProcess::spawn(launch, Arc::new(NullCallbacks))
        .await
        .ok()?;
    let conn = process.conn();

    async fn call(conn: &crate::conn::Connection, method: &str, params: Value) -> Option<Value> {
        tokio::time::timeout(CALL_TIMEOUT, conn.request::<Value>(method, params))
            .await
            .ok()?
            .ok()
    }

    let init = call(conn, "initialize", initialize_params("mira-probe")).await?;
    let _ = conn.notify("initialized", json!({})).await;
    let account = call(conn, "account/read", json!({})).await;
    let models = AppServerAgent::fetch_models(conn).await;
    process.shutdown(Duration::from_millis(500)).await;

    let user_agent = init
        .get("userAgent")
        .or_else(|| init.get("user_agent"))
        .and_then(Value::as_str)
        .unwrap_or("codex")
        .to_string();
    let acct = account.as_ref().and_then(|a| a.get("account"));
    Some(AppServerProbe {
        models,
        user_agent,
        email: acct
            .and_then(|a| a.get("email"))
            .and_then(Value::as_str)
            .map(str::to_string),
        plan: acct
            .and_then(|a| a.get("planType").or_else(|| a.get("plan")))
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// The model option event: the catalog with `current` marked. `None` means
/// unmarked — the server has not said what runs, and the picker shows the
/// list without pretending.
fn config_options_event(models: &[CodexModel], current: Option<&str>) -> NormalizedEvent {
    NormalizedEvent {
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
                values: models
                    .iter()
                    .map(|m| crate::events::ConfigValueView {
                        value: m.value.clone(),
                        name: m.label.clone(),
                        description: None,
                    })
                    .collect(),
            }],
        },
    }
}

struct Callbacks {
    tx: mpsc::Sender<NormalizedEvent>,
    end_tx: mpsc::Sender<TurnEnd>,
    thread_id: Arc<Mutex<Option<String>>>,
    active_turn: Arc<Mutex<Option<String>>>,
    goal_active: Arc<std::sync::atomic::AtomicBool>,
    rate_limits: Arc<Mutex<Option<Value>>>,
    /// Native Codex approval requests often carry only an `itemId` plus
    /// protocol bookkeeping. Keep the original item so the permission card can
    /// show the command or patch instead of the request wrapper.
    items: Arc<Mutex<HashMap<String, Value>>>,
    /// The fetched catalog, retained so a later `model/rerouted` can re-emit
    /// the same list with the new current marked.
    models: Arc<Mutex<Vec<CodexModel>>>,
    gate: PermissionGate,
    turns: Arc<TurnContext>,
    /// The proposed plan of the current plan-mode turn, from Codex's `plan`
    /// item (or its deltas, when the completed item arrives empty).
    proposed_plan: Arc<Mutex<String>>,
    /// A plan-mode turn that ended with a plan goes here instead of ending:
    /// the reviewer shows Mira's plan card and either continues the turn
    /// (build, or revise) or releases the held end.
    plan_review: mpsc::Sender<PlanReview>,
}

/// A finished plan waiting for the user, with the turn end it holds back.
struct PlanReview {
    markdown: String,
    end: TurnEnd,
}

/// What every `turn/start` is built from. Shared between the agent and the
/// plan reviewer, so a turn started after a plan approval matches one the
/// user sent.
struct TurnContext {
    model: Arc<Mutex<String>>,
    /// Runtime mode id (`plan`, `approval-required`, …). Applied per turn.
    mode: Mutex<String>,
    /// Where an approved plan gets built: the last non-plan mode.
    build_mode: Mutex<String>,
    cwd: Option<String>,
    effort: Option<String>,
    service_tier: Option<String>,
    /// Whether Mira's tool server is attached to this thread.
    mira_tools: bool,
}

impl TurnContext {
    async fn params(&self, thread_id: &str, text: &str, images: &[mira_core::ImageData]) -> Value {
        let model = self.model.lock().await.clone();
        let mode = self.mode.lock().await.clone();
        let (policy, sandbox) = policy_for_mode(&mode);
        let mut params = turn_start_params(
            thread_id,
            text,
            Some(model.as_str()),
            self.cwd.as_deref(),
            policy,
            sandbox,
            approvals_reviewer_for_mode(&mode),
            self.effort.as_deref(),
            self.service_tier.as_deref(),
            &mode,
            self.mira_tools,
        );
        params["input"] = codex_prompt_input(text, images);
        params
    }

    async fn set_mode(&self, mode: &str) {
        if mode != "plan" {
            *self.build_mode.lock().await = mode.to_string();
        }
        *self.mode.lock().await = mode.to_string();
    }

    async fn in_plan_mode(&self) -> bool {
        *self.mode.lock().await == "plan"
    }
}

fn modes_event(current: &str) -> NormalizedEvent {
    NormalizedEvent {
        source: variant("modes"),
        event: MiraEvent::Modes {
            current: current.to_string(),
            available: mode_views(),
        },
    }
}

/// Send `turn/start` and record the turn it opened.
async fn start_turn(
    conn: &crate::conn::Connection,
    params: Value,
    active_turn: &Mutex<Option<String>>,
) -> Result<(), NativeError> {
    protocol_log("out", "turn/start", &params);
    let res: Value = tokio::time::timeout(CALL_TIMEOUT, conn.request::<Value>("turn/start", params))
        .await
        .map_err(|_| NativeError::Other("turn/start timed out".to_string()))?
        .map_err(|e| NativeError::Other(e.to_string()))?;
    protocol_log("in", "turn/start", &res);
    if let Some(id) = res
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
    {
        *active_turn.lock().await = Some(id.to_string());
    }
    Ok(())
}

/// What to send Codex after the user reviewed its plan, if anything. An
/// approval builds in the pre-plan mode; a rejection with a reason goes back
/// as a revision request in plan mode; a dismissal ends the turn.
fn plan_follow_up(decision: &crate::native::PermissionDecision) -> Option<String> {
    if decision.allow {
        let edited = decision
            .updated_input
            .as_ref()
            .and_then(|v| v.get("plan"))
            .and_then(Value::as_str)
            .filter(|p| !p.trim().is_empty());
        return Some(match edited {
            Some(plan) => format!("The user approved this plan, as edited. Implement it.\n\n{plan}"),
            None => "The user approved the plan. Implement it.".to_string(),
        });
    }
    decision
        .message
        .clone()
        .filter(|m| m.starts_with("The user rejected the plan"))
}

/// Review plans one at a time, as Codex finishes them.
async fn review_plans(
    conn: crate::conn::Connection,
    mut rx: mpsc::Receiver<PlanReview>,
    gate: PermissionGate,
    turns: Arc<TurnContext>,
    thread_id: Arc<Mutex<Option<String>>>,
    active_turn: Arc<Mutex<Option<String>>>,
    tx: mpsc::Sender<NormalizedEvent>,
    end_tx: mpsc::Sender<TurnEnd>,
) {
    let mut n = 0u64;
    while let Some(review) = rx.recv().await {
        n += 1;
        let decision = gate(NativePermission {
            request_id: format!("codex-plan-{n}"),
            tool_name: "ExitPlanMode".to_string(),
            input: json!({ "plan": review.markdown }),
            ..Default::default()
        })
        .await;
        let Some(text) = plan_follow_up(&decision) else {
            let _ = end_tx.send(review.end).await;
            continue;
        };
        if decision.allow {
            let build = turns.build_mode.lock().await.clone();
            turns.set_mode(&build).await;
            let _ = tx.send(modes_event(&build)).await;
        }
        let tid = thread_id.lock().await.clone().unwrap_or_default();
        let params = turns.params(&tid, &text, &[]).await;
        if let Err(e) = start_turn(&conn, params, &active_turn).await {
            tracing::warn!("codex: could not continue after plan review: {e}");
            let _ = end_tx.send(review.end).await;
        }
    }
}

#[async_trait::async_trait]
impl crate::conn::AgentCallback for Callbacks {
    async fn on_request(
        &self,
        id: &Value,
        method: &str,
        params: Value,
    ) -> Result<Value, crate::conn::ConnError> {
        let rid = id
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| id.to_string());
        let params = enrich_permission_params(method, params, &self.items).await;
        let Some(perm) = permission_from_request(method, &rid, &params) else {
            return Err(crate::conn::ConnError::Unhandled {
                method: method.to_string(),
            });
        };
        let decision = (self.gate)(perm).await;
        Ok(approval_response(method, &params, &decision))
    }

    async fn on_notification(&self, method: &str, params: Value) {
        remember_native_item(method, &params, &self.items).await;
        let root = self.thread_id.lock().await.clone();
        let belongs_to_root = params
            .get("threadId")
            .and_then(Value::as_str)
            .is_none_or(|thread| root.as_deref().is_none_or(|id| id == thread));
        if belongs_to_root {
            match method {
                "turn/started" => self.proposed_plan.lock().await.clear(),
                "item/plan/delta" => {
                    if let Some(d) = params.get("delta").and_then(Value::as_str) {
                        self.proposed_plan.lock().await.push_str(d);
                    }
                }
                "item/completed" => {
                    let item = params.get("item").unwrap_or(&Value::Null);
                    if item.get("type").and_then(Value::as_str) == Some("plan") {
                        if let Some(text) = item.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                            *self.proposed_plan.lock().await = text.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
        if belongs_to_root && matches!(method, "thread/goal/updated" | "thread/goal/cleared") {
            let active = method != "thread/goal/cleared"
                && params
                    .get("goal")
                    .and_then(|g| g.get("status"))
                    .and_then(Value::as_str)
                    == Some("active");
            self.goal_active
                .store(active, std::sync::atomic::Ordering::SeqCst);
        }
        for action in map_notification(method, &params) {
            match action {
                AppServerAction::SessionId(tid) => {
                    let mut root = self.thread_id.lock().await;
                    if root.is_none() {
                        *root = Some(tid);
                    }
                }
                AppServerAction::Event(e) => {
                    // Child narration belongs to the child's thread; only
                    // its normalized lifecycle contributes to root ownership.
                    if !belongs_to_root
                        && !matches!(
                            &e.event,
                            MiraEvent::RuntimeTurn(_) | MiraEvent::RuntimeWork(_)
                        )
                    {
                        continue;
                    }
                    let _ = self.tx.send(e).await;
                }
                AppServerAction::TurnEnded {
                    stop_reason,
                    is_error,
                } => {
                    if !belongs_to_root {
                        continue;
                    }
                    *self.active_turn.lock().await = None;
                    let end = TurnEnd {
                        stop_reason,
                        is_error,
                        rate_limited: None,
                    };
                    let plan = std::mem::take(&mut *self.proposed_plan.lock().await);
                    if !end.is_error
                        && end.stop_reason == "completed"
                        && !plan.trim().is_empty()
                        && self.turns.in_plan_mode().await
                    {
                        let _ = self.plan_review.send(PlanReview { markdown: plan, end }).await;
                        continue;
                    }
                    let _ = self.end_tx.send(end).await;
                }
                AppServerAction::RateLimits(v) => {
                    *self.rate_limits.lock().await = Some(v.clone());
                    let windows = codex_rate_limit_windows(&v);
                    if !windows.is_empty() {
                        let _ = self
                            .tx
                            .send(NormalizedEvent {
                                source: variant("limits"),
                                event: MiraEvent::Limits { windows },
                            })
                            .await;
                    }
                }
                AppServerAction::ModelRerouted { model } => {
                    let models = self.models.lock().await.clone();
                    if !models.is_empty() {
                        let _ = self
                            .tx
                            .send(config_options_event(&models, Some(&model)))
                            .await;
                    }
                }
            }
        }
        // `turn/started` carries the active turn id without other content.
        if method == "turn/started" && belongs_to_root {
            if let Some(tid) = params
                .get("turn")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
            {
                *self.active_turn.lock().await = Some(tid.to_string());
            }
        }
    }
}

/// A running Codex app-server session.
pub struct AppServerAgent {
    process: AgentProcess,
    thread_id: Arc<Mutex<Option<String>>>,
    active_turn: Arc<Mutex<Option<String>>>,
    goal_active: Arc<std::sync::atomic::AtomicBool>,
    rate_limits: Arc<Mutex<Option<Value>>>,
    model: Arc<Mutex<String>>,
    turns: Arc<TurnContext>,
    events_tx: mpsc::Sender<NormalizedEvent>,
    pub events: std::sync::Mutex<Option<mpsc::Receiver<NormalizedEvent>>>,
    pub turn_end: std::sync::Mutex<Option<mpsc::Receiver<TurnEnd>>>,
}

/// Errors share the native vocabulary: same caller, same handling.
impl AppServerAgent {
    /// Spawn, handshake, and open (or resume) a thread.
    pub async fn start(
        launch_cfg: &LaunchConfig,
        runtime_mode: &str,
        cwd: Option<String>,
        model: Option<String>,
        resume_thread_id: Option<String>,
        gate: PermissionGate,
        mira_mcp: Option<crate::session::MiraMcp>,
    ) -> Result<Self, NativeError> {
        let (policy, sandbox) = policy_for_mode(runtime_mode);
        let (tx, rx) = mpsc::channel(256);
        let (end_tx, end_rx) = mpsc::channel(16);
        let model_tx = tx.clone();
        let thread_id = Arc::new(Mutex::new(None));
        let active_turn = Arc::new(Mutex::new(None));
        let goal_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rate_limits = Arc::new(Mutex::new(None));
        let current_model = Arc::new(Mutex::new(
            model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty() && *m != "default")
                .unwrap_or(DEFAULT_CODEX_MODEL)
                .to_string(),
        ));
        let models_retained: Arc<Mutex<Vec<CodexModel>>> = Arc::new(Mutex::new(Vec::new()));
        let native_items: Arc<Mutex<HashMap<String, Value>>> = Arc::new(Mutex::new(HashMap::new()));
        let turns = Arc::new(TurnContext {
            model: current_model.clone(),
            mode: Mutex::new(runtime_mode.to_string()),
            build_mode: Mutex::new(
                if runtime_mode == "plan" { "approval-required" } else { runtime_mode }.to_string(),
            ),
            cwd: cwd.clone(),
            effort: None,
            service_tier: None,
            mira_tools: mira_mcp.is_some(),
        });
        let (plan_tx, plan_rx) = mpsc::channel(4);
        let events_tx = tx.clone();
        let reviewer_tx = tx.clone();
        let reviewer_end_tx = end_tx.clone();
        let reviewer_gate = gate.clone();

        let process = AgentProcess::spawn(
            launch_cfg,
            Arc::new(Callbacks {
                tx,
                end_tx,
                thread_id: thread_id.clone(),
                active_turn: active_turn.clone(),
                goal_active: goal_active.clone(),
                rate_limits: rate_limits.clone(),
                items: native_items.clone(),
                models: models_retained.clone(),
                gate,
                turns: turns.clone(),
                proposed_plan: Arc::new(Mutex::new(String::new())),
                plan_review: plan_tx,
            }),
        )
        .await
        .map_err(|e| NativeError::Other(e.to_string()))?;
        let conn = process.conn();
        tokio::spawn(review_plans(
            conn.clone(),
            plan_rx,
            reviewer_gate,
            turns.clone(),
            thread_id.clone(),
            active_turn.clone(),
            reviewer_tx,
            reviewer_end_tx,
        ));
        let _ = model_tx.send(modes_event(runtime_mode)).await;

        async fn call(
            conn: &crate::conn::Connection,
            method: &str,
            params: Value,
        ) -> Result<Value, NativeError> {
            protocol_log("out", method, &params);
            let res = tokio::time::timeout(CALL_TIMEOUT, conn.request::<Value>(method, params))
                .await
                .map_err(|_| NativeError::Other(format!("{method} timed out")))?
                .map_err(|e| NativeError::Other(e.to_string()))?;
            protocol_log("in", method, &res);
            Ok(res)
        }

        // Verified handshake shape: `initialize` answers with agent facts.
        let _init: Value = call(conn, "initialize", initialize_params("mira")).await?;
        let _ = conn.notify("initialized", json!({})).await;

        let (method, start_params) = thread_open_params(
            cwd.clone(),
            policy,
            sandbox,
            model.as_deref(),
            resume_thread_id.as_deref(),
            mira_mcp.as_ref(),
        );
        let opened: Value = call(conn, method, start_params).await?;
        let tid = opened
            .get("threadId")
            .or_else(|| opened.get("thread").and_then(|t| t.get("id")))
            .or_else(|| opened.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| resume_thread_id.as_deref().unwrap_or(""))
            .to_string();
        if !tid.is_empty() {
            *thread_id.lock().await = Some(tid);
        }

        // The catalog, so the picker lists what Codex advertises rather than
        // a hardcoded guess. Best-effort: a session without it still works,
        // and the event flows through the same channel as everything else.
        // Mark the server-reported model, falling back to an explicit pick.
        // Older servers may omit it; leave their default unmarked.
        let models = Self::fetch_models(conn).await;
        if !models.is_empty() {
            *models_retained.lock().await = models.clone();
            let _ = model_tx
                .send(config_options_event(
                    &models,
                    opened
                        .get("model")
                        .and_then(Value::as_str)
                        .or(model.as_deref()),
                ))
                .await;
        }

        if let Some(limits) = tokio::time::timeout(
            CALL_TIMEOUT,
            conn.request::<Value>("account/rateLimits/read", json!({})),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        {
            *rate_limits.lock().await = Some(limits.clone());
            let windows = codex_rate_limit_windows(&limits);
            if !windows.is_empty() {
                let _ = model_tx
                    .send(NormalizedEvent {
                        source: variant("limits"),
                        event: MiraEvent::Limits { windows },
                    })
                    .await;
            }
        }

        Ok(Self {
            process,
            thread_id,
            active_turn,
            goal_active,
            rate_limits,
            model: current_model,
            turns,
            events_tx,
            events: std::sync::Mutex::new(Some(rx)),
            turn_end: std::sync::Mutex::new(Some(end_rx)),
        })
    }

    /// Fetch the advertised catalog, following page cursors. Best-effort:
    /// anything failing yields what arrived so far rather than an error.
    async fn fetch_models(conn: &crate::conn::Connection) -> Vec<CodexModel> {
        let mut values = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match cursor.clone() {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            protocol_log("out", "model/list", &params);
            let page: Value = match tokio::time::timeout(
                CALL_TIMEOUT,
                conn.request::<Value>("model/list", params),
            )
            .await
            {
                Ok(Ok(p)) => {
                    protocol_log("in", "model/list", &p);
                    p
                }
                _ => break,
            };
            for e in page
                .get("data")
                .or_else(|| page.get("models"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(m) = parse_model_entry(&e) {
                    if !values.iter().any(|v: &CodexModel| v.value == m.value) {
                        values.push(m);
                    }
                }
            }
            let next = page
                .get("nextCursor")
                .or_else(|| page.get("next_cursor"))
                .or_else(|| page.get("cursor"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if next == cursor {
                break;
            }
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        values
    }

    pub async fn session_id(&self) -> Option<String> {
        self.thread_id.lock().await.clone()
    }

    /// Send a turn. Returns once the turn is accepted, not when it finishes;
    /// completion arrives as `turn/completed` on the event channel.
    pub async fn prompt(&self, text: &str) -> Result<(), NativeError> {
        self.prompt_with_images(text, &[]).await
    }

    pub async fn prompt_with_images(&self, text: &str, images: &[mira_core::ImageData]) -> Result<(), NativeError> {
        let tid = self
            .session_id()
            .await
            .ok_or_else(|| NativeError::Other("no codex thread yet".to_string()))?;
        let params = self.turns.params(&tid, text, images).await;
        start_turn(self.process.conn(), params, &self.active_turn).await
    }

    /// Switch the runtime mode. Codex takes approval policy, sandbox and
    /// collaboration mode on every `turn/start`, so this applies from the
    /// next turn without restarting the thread.
    pub async fn set_mode(&self, mode: &str) {
        self.turns.set_mode(mode).await;
        let _ = self.events_tx.send(modes_event(mode)).await;
    }

    pub async fn steer(&self, text: &str) -> Result<(), NativeError> {
        self.steer_with_images(text, &[]).await
    }

    pub async fn steer_with_images(&self, text: &str, images: &[mira_core::ImageData]) -> Result<(), NativeError> {
        let thread_id = self
            .session_id()
            .await
            .ok_or_else(|| NativeError::Other("no codex thread".into()))?;
        let turn_id = self
            .active_turn
            .lock()
            .await
            .clone()
            .ok_or_else(|| NativeError::Other("no active codex turn to steer".into()))?;
        let params = json!({ "threadId": thread_id, "expectedTurnId": turn_id, "input": codex_prompt_input(text, images) });
        tokio::time::timeout(
            CALL_TIMEOUT,
            self.process.conn().request::<Value>("turn/steer", params),
        )
        .await
        .map_err(|_| NativeError::Other("turn/steer timed out".into()))?
        .map_err(|e| NativeError::Other(e.to_string()))?;
        Ok(())
    }
    pub async fn has_active_turn(&self) -> bool {
        self.active_turn.lock().await.is_some()
    }

    /// Remember the selected model for subsequent turns. Codex app-server takes
    /// the model on `thread/start` and `turn/start`; sending a separate native
    /// control request is the stale-rollout path that produced missing-model
    /// errors.
    pub async fn set_model(&self, model: &str) {
        let model = model.trim();
        let next = if model.is_empty() || model == "default" {
            DEFAULT_CODEX_MODEL
        } else {
            model
        };
        *self.model.lock().await = next.to_string();
    }

    pub async fn cancel(&self) -> Result<(), NativeError> {
        let tid = self.session_id().await.unwrap_or_default();
        if self.goal_active.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::timeout(
                Duration::from_secs(3),
                self.process
                    .conn()
                    .request::<Value>("thread/goal/set", json!({"threadId":tid,"status":"paused"})),
            )
            .await
            .map_err(|_| NativeError::Other("pausing the native goal timed out".into()))?
            .map_err(|e| NativeError::Other(e.to_string()))?;
            self.goal_active
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        let mut params = json!({ "threadId": tid });
        if let Some(turn) = self.active_turn.lock().await.clone() {
            params["turnId"] = json!(turn);
        }
        protocol_log("out", "turn/interrupt", &params);
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            self.process
                .conn()
                .request::<Value>("turn/interrupt", params),
        )
        .await;
        Ok(())
    }

    /// Ask Codex to compact the thread. Best-effort: older servers may not
    /// implement it, and a failed compaction must not fail the session.
    pub async fn compact(&self) -> Result<(), NativeError> {
        let tid = self.session_id().await.unwrap_or_default();
        let params = json!({ "threadId": tid });
        protocol_log("out", "thread/compact/start", &params);
        tokio::time::timeout(
            CALL_TIMEOUT,
            self.process
                .conn()
                .request::<Value>("thread/compact/start", params),
        )
        .await
        .map_err(|_| NativeError::Other("thread/compact/start timed out".to_string()))?
        .map_err(|e| NativeError::Other(e.to_string()))?;
        Ok(())
    }

    /// The last usage snapshot the server pushed, if any.
    pub async fn rate_limits(&self) -> Option<Value> {
        self.rate_limits.lock().await.clone()
    }

    /// Query usage state on demand.
    pub async fn query_rate_limits(&self) -> Option<Value> {
        let params = Value::Null;
        protocol_log("out", "account/rateLimits/read", &params);
        tokio::time::timeout(
            CALL_TIMEOUT,
            self.process
                .conn()
                .request::<Value>("account/rateLimits/read", params),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
    }

    pub async fn stderr_tail(&self) -> String {
        self.process.stderr_tail().await
    }

    pub async fn shutdown(&self) {
        self.process.shutdown(Duration::from_millis(800)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::PermissionGate;

    #[test]
    fn mira_workspace_calls_have_one_stream_owner() {
        let own = json!({"id":"mcp-1","type":"mcpToolCall","server":"mira","tool":"read_file"});
        assert!(map_item(&own, true).is_empty());
        assert!(map_item(&own, false).is_empty());
        let browser = json!({"id":"mcp-2","type":"mcpToolCall","server":"mira","tool":"browser_snapshot"});
        assert!(!map_item(&browser, true).is_empty());
        let other = json!({"id":"mcp-3","type":"mcpToolCall","server":"other","tool":"read_file"});
        assert!(!map_item(&other, true).is_empty());
    }

    #[test]
    fn failed_completion_preserves_usage_limit_and_mcp_failure() {
        let actions = map_notification("turn/completed", &json!({"turn":{"status":"failed","error":{"codexErrorInfo":"usageLimitExceeded"}}}));
        assert!(matches!(&actions[0], AppServerAction::TurnEnded { stop_reason, is_error: true } if stop_reason == "rate_limited"));
        let actions = map_notification("mcpServer/startupStatus/updated", &json!({"name":"mira","status":"failed","error":"Connection refused"}));
        assert!(matches!(&actions[0], AppServerAction::Event(NormalizedEvent { event: MiraEvent::Activity { title, detail, .. }, .. }) if title == "Mira tools unavailable" && detail == "Connection refused"));
    }

    #[test]
    fn codex_images_are_native_inputs_even_without_text() {
        let image = mira_core::ImageData::png("YWJj");
        assert_eq!(codex_prompt_input("", &[image.clone()]), json!([{"type":"image", "url":"data:image/png;base64,YWJj"}]));
        assert_eq!(codex_prompt_input("look", &[image]), json!([{"type":"text","text":"look"}, {"type":"image","url":"data:image/png;base64,YWJj"}]));
        assert_eq!(codex_prompt_input("text", &[]), json!([{"type":"text","text":"text"}]));
    }

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn completed_messages_are_authoritative_and_tool_output_is_incremental() {
        let complete = map_notification(
            "item/completed",
            &serde_json::json!({
                "item": {"id":"answer", "type":"agentMessage", "text":"full answer"}
            }),
        );
        assert!(complete.iter().any(|a| matches!(a,
            AppServerAction::Event(NormalizedEvent { event: MiraEvent::AssistantSnapshot {message_id, text}, .. })
            if message_id == "answer" && text == "full answer")));
        let output = map_notification(
            "item/commandExecution/outputDelta",
            &serde_json::json!({"itemId":"cmd", "delta":"line\n"}),
        );
        assert!(output.iter().any(|a| matches!(a,
            AppServerAction::Event(NormalizedEvent { event: MiraEvent::ToolOutputDelta {id, text}, .. })
            if id == "cmd" && text == "line\n")));
    }

    #[test]
    fn deltas_become_text_and_thought() {
        let a = map_notification(
            "item/agentMessage/delta",
            &v(r#"{"delta":"hi","itemId":"i1"}"#),
        );
        assert!(a.iter().any(|x| matches!(x,
            AppServerAction::Event(NormalizedEvent { event: MiraEvent::AssistantText { text, .. }, .. })
            if text == "hi")));

        let t = map_notification(
            "item/reasoning/textDelta",
            &v(r#"{"delta":"hmm","itemId":"i1"}"#),
        );
        assert!(t.iter().any(|x| matches!(
            x,
            AppServerAction::Event(NormalizedEvent {
                event: MiraEvent::AgentThought { .. },
                ..
            })
        )));
    }

    #[test]
    fn items_become_tool_lifecycle() {
        let started = map_notification(
            "item/started",
            &v(r#"{"item":{"id":"i1","type":"command_execution","command":"ls"}}"#),
        );
        assert!(started.iter().any(|x| matches!(x,
            AppServerAction::Event(NormalizedEvent { event: MiraEvent::ToolCall(c), .. })
            if c.id == "i1" && c.status == ToolCallStatus::InProgress)));

        let done = map_notification(
            "item/completed",
            &v(
                r#"{"item":{"id":"i1","type":"command_execution","aggregated_output":"a","exit_code":0}}"#,
            ),
        );
        assert!(done.iter().any(|x| matches!(x,
            AppServerAction::Event(NormalizedEvent { event: MiraEvent::ToolCallUpdate(c), .. })
            if c.id == "i1" && c.status == ToolCallStatus::Completed)));
    }

    #[test]
    fn native_plan_tool_items_become_plan_cards() {
        let actions = map_notification(
            "item/started",
            &json!({
                "item": {
                    "id": "plan-1",
                    "type": "tool_call",
                    "tool": "update_plan",
                    "input": {
                        "plan": [
                            { "content": "Wire native request-user-input", "status": "completed" },
                            { "content": "Render plan cards", "status": "in_progress", "priority": "high" }
                        ]
                    }
                }
            }),
        );
        assert!(actions.iter().any(|x| matches!(
            x,
            AppServerAction::Event(NormalizedEvent {
                event: MiraEvent::Plan { entries },
                ..
            }) if entries.len() == 2
                && entries[0].content == "Wire native request-user-input"
                && entries[0].status == "completed"
                && entries[1].priority == "high"
        )));
    }

    #[test]
    fn retryable_errors_do_not_end_the_turn() {
        let actions = map_notification("error", &json!({"willRetry":true,"error":{"message":"provider busy"}}));
        assert!(!actions.iter().any(|action| matches!(action, AppServerAction::TurnEnded { .. })));
        assert!(actions.iter().any(|action| matches!(action, AppServerAction::Event(NormalizedEvent { event: MiraEvent::Activity { kind, .. }, .. }) if kind == "retry")));
        assert!(map_notification("error", &json!({"willRetry":false})).iter().any(|action| matches!(action, AppServerAction::TurnEnded { is_error:true, .. })));
    }

    #[test]
    fn turn_endings_map_to_stop_reasons() {
        assert!(
            map_notification("turn/completed", &v(r#"{"turn":{"id":"t"}}"#)).contains(
                &AppServerAction::TurnEnded {
                    stop_reason: "completed".into(),
                    is_error: false
                }
            )
        );
        assert!(map_notification("turn/aborted", &v(r#"{}"#)).contains(
            &AppServerAction::TurnEnded {
                stop_reason: "cancelled".into(),
                is_error: true
            }
        ));
    }

    #[test]
    fn token_usage_becomes_a_usage_event() {
        let a = map_notification(
            "thread/tokenUsage/updated",
            &v(
                r#"{"tokenUsage":{"total":{"totalTokens":100},"last":{"totalTokens":20},"modelContextWindow":200000}}"#,
            ),
        );
        assert!(a.iter().any(|x| matches!(
            x,
            AppServerAction::Event(NormalizedEvent {
                event: MiraEvent::Usage {
                    used: 20,
                    size: 200000,
                    ..
                },
                ..
            })
        )));
    }

    #[test]
    fn questions_return_only_the_users_answers_by_id() {
        let params = json!({ "questions": [
            { "id": "q1", "question": "Pick one", "options": [{ "label": "First" }, { "label": "Second" }] },
            { "id": "q2", "question": "Pick one" }
        ] });
        let permission =
            permission_from_request("item/tool/requestUserInput", "r", &params).unwrap();
        assert_eq!(permission.tool_name, "request_user_input");
        let decision = crate::native::PermissionDecision {
            allow: true,
            updated_input: Some(json!({ "answers": {
                "q1": { "answers": ["Second"] },
                "q2": { "answers": ["My own answer"] },
                "unrequested": { "answers": ["ignore"] }
            } })),
            ..Default::default()
        };
        let response = approval_response("item/tool/requestUserInput", &params, &decision);
        assert_eq!(
            response,
            json!({ "answers": {
            "q1": { "answers": ["Second"] }, "q2": { "answers": ["My own answer"] }
        } })
        );
        for allow in [true, false] {
            assert_eq!(
                approval_response("item/tool/requestUserInput", &params, &allow.into()),
                json!({ "answers": {} })
            );
        }
    }

    #[test]
    fn thread_open_overrides_only_mira_mcp_server() {
        let mcp = crate::session::MiraMcp {
            url: "http://127.0.0.1:7777/mcp".into(),
            token: "secret-token".into(),
        };
        let (_method, params) = thread_open_params(
            Some("/tmp/project".into()),
            "untrusted",
            "read-only",
            Some("gpt-5-codex"),
            None,
            Some(&mcp),
        );
        assert_eq!(params["model"], "gpt-5-codex");
        for key in ["mcp_servers.mira"] {
            let mira = &params["config"][key];
            assert_eq!(mira["url"], "http://127.0.0.1:7777/mcp");
            assert_eq!(mira["http_headers"]["Authorization"], "Bearer secret-token");
        }
        let (_method, default_params) = thread_open_params(
            Some("/tmp/project".into()),
            "untrusted",
            "read-only",
            None,
            None,
            None,
        );
        assert_eq!(default_params["model"], DEFAULT_CODEX_MODEL);
    }

    #[test]
    fn turns_use_typed_input_and_models_use_the_inference_slug() {
        assert_eq!(
            turn_start_params(
                "t",
                "hello",
                Some("gpt-5-codex"),
                Some("/r"),
                "untrusted",
                "read-only",
                "user",
                None,
                None, "approval-required", false
            )["threadId"],
            "t"
        );
        let turn = turn_start_params(
            "t",
            "hello",
            Some("gpt-5-codex"),
            Some("/r"),
            "untrusted",
            "read-only",
            "user",
            None,
            None, "approval-required", false
        );
        assert_eq!(turn["input"], json!([{ "type": "text", "text": "hello" }]));
        assert_eq!(turn["model"], "gpt-5-codex");
        assert!(
            turn["collaborationMode"]["settings"]["developer_instructions"]
                .as_str()
                .unwrap()
                .contains("request_user_input")
        );
        assert_eq!(turn["collaborationMode"]["mode"], "default");
        assert_eq!(
            turn["collaborationMode"]["settings"]["model"],
            "gpt-5-codex"
        );
        assert_eq!(turn["approvalPolicy"], "untrusted");
        assert_eq!(turn["approvalsReviewer"], "user");
        assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(turn["cwd"], "/r");
        assert_eq!(
            turn_start_params(
                "t",
                "hello",
                None,
                None,
                "on-request",
                "workspace-write",
                "auto_review",
                Some("high"),
                Some("priority"), "approval-required", false
            )["model"],
            DEFAULT_CODEX_MODEL
        );
        assert_eq!(
            parse_model_entry(
                &json!({ "id": "catalog-id", "model": "inference-slug", "displayName": "Model" })
            )
            .unwrap()
            .value,
            "inference-slug"
        );
        assert!(parse_model_entry(&json!({ "id": "hidden", "hidden": true })).is_none());
    }

    #[test]
    fn approvals_answer_with_verified_decision_literals() {
        // `{"decision": ...}` verified against the generated app-server schema.
        let ok = approval_response(
            "item/commandExecution/requestApproval",
            &v(r#"{"command":"ls"}"#),
            &true.into(),
        );
        assert_eq!(ok["decision"], "accept");
        let no = approval_response(
            "item/fileChange/requestApproval",
            &v(r#"{"changes":[]}"#),
            &false.into(),
        );
        assert_eq!(no["decision"], "decline");

        let p = permission_from_request(
            "item/commandExecution/requestApproval",
            "41",
            &v(r#"{"command":"rm -rf /"}"#),
        )
        .unwrap();
        assert_eq!(p.tool_name, "shell");
        assert_eq!(p.request_id, "41");
        assert!(permission_from_request("nope/unknown", "1", &v(r#"{}"#)).is_none());
    }

    #[test]
    fn permission_profiles_grant_only_requested_access_for_the_turn() {
        let params = json!({ "permissions": { "network": { "enabled": true },
            "fileSystem": { "write": ["/tmp/project"] } }, "threadId": "t" });
        let allow = approval_response("item/permissions/requestApproval", &params, &true.into());
        assert_eq!(allow, json!({ "permissions": params["permissions"], "scope": "turn" }));
        let deny = approval_response("item/permissions/requestApproval", &params, &false.into());
        assert_eq!(deny, json!({ "permissions": {}, "scope": "turn" }));
        assert!(allow.get("decision").is_none());
        assert!(allow.get("threadId").is_none());
    }

    #[test]
    fn model_entries_parse_defensively() {
        assert_eq!(
            parse_model_entry(&v(r#"{"id":"gpt-5","name":"GPT 5"}"#)),
            Some(CodexModel {
                value: "gpt-5".into(),
                label: "GPT 5".into()
            })
        );
        assert_eq!(
            parse_model_entry(&v(r#"{"slug":"x-mini"}"#)),
            Some(CodexModel {
                value: "x-mini".into(),
                label: "x-mini".into()
            })
        );
        assert!(parse_model_entry(&v(r#"{"nope":1}"#)).is_none());
    }

    #[test]
    fn reroute_carries_the_new_model_id() {
        let a = map_notification("model/rerouted", &v(r#"{"model":"gpt-5-mini"}"#));
        assert!(a.contains(&AppServerAction::ModelRerouted {
            model: "gpt-5-mini".into()
        }));
        // Nested shape, same outcome.
        let b = map_notification("model/rerouted", &v(r#"{"model":{"id":"gpt-5"}}"#));
        assert!(b.contains(&AppServerAction::ModelRerouted {
            model: "gpt-5".into()
        }));
        // Nothing to mark with: silence, not a guess.
        assert!(map_notification("model/rerouted", &v(r#"{}"#)).is_empty());
    }

    #[test]
    fn config_options_event_marks_current_without_reordering() {
        let models = vec![
            CodexModel {
                value: "gpt-5".into(),
                label: "GPT 5".into(),
            },
            CodexModel {
                value: "gpt-5-mini".into(),
                label: "GPT 5 Mini".into(),
            },
        ];
        let unmarked = config_options_event(&models, None);
        let MiraEvent::ConfigOptions { options } = unmarked.event else {
            panic!("expected config options");
        };
        assert!(options[0].current.is_none());
        assert_eq!(options[0].values.len(), 2);
        let marked = config_options_event(&models, Some("gpt-5-mini"));
        let MiraEvent::ConfigOptions { options } = marked.event else {
            panic!("expected config options");
        };
        assert_eq!(options[0].current.as_deref(), Some("gpt-5-mini"));
        // Same list, same order — only the marker moved.
        assert_eq!(options[0].values.len(), 2);
        assert_eq!(options[0].values[0].value, "gpt-5");
    }

    #[test]
    fn runtime_modes_cover_the_postures() {
        assert_eq!(
            policy_for_mode("approval-required"),
            ("untrusted", "read-only")
        );
        assert_eq!(
            policy_for_mode("full-access"),
            ("never", "danger-full-access")
        );
        assert_eq!(
            mode_for_permission(PermissionMode::Ask),
            "approval-required"
        );
        assert_eq!(mode_for_permission(PermissionMode::Auto), "auto");
        assert_eq!(mode_for_permission(PermissionMode::AcceptEdits), "auto-accept-edits");
        assert_eq!(mode_views().len(), 5);
    }

    /// A stand-in `codex app-server`: handshake, thread, paged models, a turn
    /// with an approval round-trip, rate limits, compaction.
    fn fake_codex() -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex");
        std::fs::write(
            &path,
            r#"#!/bin/sh
# The id of the request being answered. Anchored to the start of the
# line: a request's params can carry multi-line text (Codex sends
# developer instructions with real newlines), and a greedy `.*"id":N.*`
# would then match across that text and return the tail of it instead of
# the number — which the client silently discards as a malformed frame.
id_of() { printf '%s' "$1" | sed -E 's/^\{"jsonrpc":"2\.0","id":([0-9]+),.*/\1/'; }
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"userAgent\":\"fake-codex\",\"codexHome\":\".\",\"platformFamily\":\"unix\",\"platformOs\":\"linux\"}}" ;;
    *'"method":"account/read"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"account\":null}}" ;;
    *'"method":"thread/start"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"threadId\":\"th-1\"}}" ;;
    *'"method":"model/list"'*)
      id=$(id_of "$line")
      case "$line" in
        *'"cursor"'*) echo "{\"id\":$id,\"result\":{\"data\":[{\"id\":\"gpt-5-mini\",\"name\":\"GPT 5 Mini\"}]}}" ;;
        *) echo "{\"id\":$id,\"result\":{\"data\":[{\"id\":\"gpt-5\",\"name\":\"GPT 5\"}],\"nextCursor\":\"c1\"}}" ;;
      esac ;;
    *'"method":"turn/start"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"tu-1\"}}}"
      echo '{"method":"item/agentMessage/delta","params":{"delta":"hello","itemId":"i1"}}'
      echo '{"id":901,"method":"item/commandExecution/requestApproval","params":{"command":"echo hi"}}' ;;
    *'"id":901'*)
      echo '{"method":"item/completed","params":{"item":{"id":"i1","type":"command_execution","aggregated_output":"hi","exit_code":0}}}'
      echo '{"method":"model/rerouted","params":{"model":"gpt-5-mini"}}'
      echo '{"method":"turn/completed","params":{"turn":{"id":"tu-1"}}}' ;;
    *'"method":"account/rateLimits/read"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"rateLimits\":{\"primary\":{\"usedPercent\":11}}}}" ;;
    *'"method":"thread/compact/start"'*|*'"method":"turn/interrupt"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{}}" ;;
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
        (
            LaunchConfig {
                program: path,
                args: vec!["app-server".to_string()],
                env: Default::default(),
                secret_env: Vec::new(),
                env_deny: Vec::new(),
            },
            dir,
        )
    }

    #[test]
    fn plan_mode_uses_codexs_own_plan_prompt_and_stays_read_only() {
        let turn = turn_start_params(
            "t", "plan it", Some("gpt-5"), None, "on-request", "read-only", "user", None, None,
            "plan", true,
        );
        assert_eq!(turn["collaborationMode"]["mode"], "plan");
        assert!(
            turn["collaborationMode"]["settings"].get("developer_instructions").is_none(),
            "plan mode must leave Codex's built-in plan prompt in charge"
        );
        assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(policy_for_mode("plan"), ("on-request", "read-only"));
        assert!(mode_views().iter().any(|m| m.id == "plan"));
    }

    #[test]
    fn mira_tool_guidance_rides_on_additional_context_not_the_mode_prompt() {
        let with = turn_start_params(
            "t", "hi", None, None, "untrusted", "read-only", "user", None, None,
            "approval-required", true,
        );
        let ctx = with["additionalContext"]["mira_tools"]["value"].as_str().unwrap();
        assert!(ctx.contains("ask_user") && ctx.contains("browser_open"));
        assert_eq!(with["additionalContext"]["mira_tools"]["kind"], "application");
        // The mode prompt carries mode rules only: newer models drop it.
        let mode_text = with["collaborationMode"]["settings"]["developer_instructions"]
            .as_str()
            .unwrap();
        assert!(!mode_text.contains("Mira"), "{mode_text}");
        // No tool server, no claim about its tools.
        let without = turn_start_params(
            "t", "hi", None, None, "untrusted", "read-only", "user", None, None,
            "approval-required", false,
        );
        assert!(without.get("additionalContext").is_none());
    }

    #[test]
    fn codex_tags_its_tool_server_connection() {
        let mcp = crate::session::MiraMcp { url: "http://127.0.0.1:1/mcp".into(), token: "t".into() };
        let config = codex_thread_config(Some(&mcp));
        assert_eq!(
            config["mcp_servers.mira"]["http_headers"][crate::session::MIRA_TOOL_PROFILE_HEADER],
            crate::session::CODEX_TOOL_PROFILE
        );
    }

    #[test]
    fn mcp_elicitations_are_answered_in_mcps_own_shape() {
        use crate::native::PermissionDecision;
        let m = "mcpServer/elicitation/request";
        let params = json!({ "serverName": "linear", "message": "Pick a team", "requestedSchema": {} });
        assert_eq!(permission_from_request(m, "r", &params).unwrap().tool_name, "mcp_elicitation");
        assert_eq!(approval_response(m, &params, &PermissionDecision::from(false)), json!({ "action": "decline" }));
        assert_eq!(approval_response(m, &params, &PermissionDecision::from(true)), json!({ "action": "accept" }));
        let filled = PermissionDecision {
            allow: true,
            updated_input: Some(json!({ "content": { "team": "ENG" } })),
            ..Default::default()
        };
        assert_eq!(
            approval_response(m, &params, &filled),
            json!({ "action": "accept", "content": { "team": "ENG" } })
        );
    }

    #[test]
    fn plan_reviews_become_build_revise_or_nothing() {
        use crate::native::PermissionDecision;
        let approved = plan_follow_up(&PermissionDecision::from(true)).unwrap();
        assert!(approved.contains("Implement"));
        let edited = plan_follow_up(&PermissionDecision {
            allow: true,
            updated_input: Some(json!({ "plan": "1. Do the thing" })),
            ..Default::default()
        })
        .unwrap();
        assert!(edited.ends_with("1. Do the thing"));
        let revise = plan_follow_up(&PermissionDecision {
            allow: false,
            message: Some("The user rejected the plan: too big. Revise it and propose again.".into()),
            ..Default::default()
        });
        assert!(revise.unwrap().contains("too big"));
        let dismissed = plan_follow_up(&PermissionDecision {
            allow: false,
            message: Some("The plan was not reviewed. Wait for the user.".into()),
            ..Default::default()
        });
        assert!(dismissed.is_none());
    }

    #[tokio::test]
    async fn an_approved_plan_continues_into_a_build_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex");
        let turns_log = dir.path().join("turns.log");
        std::fs::write(
            &path,
            format!(
                r##"#!/bin/sh
id_of() {{ printf '%s' "$1" | sed -E 's/^\{{"jsonrpc":"2\.0","id":([0-9]+),.*/\1/'; }}
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*|*'"method":"model/list"'*|*'"method":"account/rateLimits/read"'*)
      id=$(id_of "$line"); echo "{{\"id\":$id,\"result\":{{}}}}" ;;
    *'"method":"thread/start"'*)
      id=$(id_of "$line"); echo "{{\"id\":$id,\"result\":{{\"threadId\":\"th-1\"}}}}" ;;
    *'"method":"turn/start"'*'"mode":"plan"'*)
      echo plan >> '{log}'
      id=$(id_of "$line"); echo "{{\"id\":$id,\"result\":{{\"turn\":{{\"id\":\"tu-plan\"}}}}}}"
      echo '{{"method":"turn/started","params":{{"turn":{{"id":"tu-plan"}}}}}}'
      echo '{{"method":"item/completed","params":{{"item":{{"id":"p1","type":"plan","text":"Ship it: 1. Write code, 2. Test it"}}}}}}'
      echo '{{"method":"turn/completed","params":{{"turn":{{"id":"tu-plan","status":"completed"}}}}}}' ;;
    *'"method":"turn/start"'*)
      case "$line" in *Implement*) echo build >> '{log}' ;; *) echo other >> '{log}' ;; esac
      id=$(id_of "$line"); echo "{{\"id\":$id,\"result\":{{\"turn\":{{\"id\":\"tu-build\"}}}}}}"
      echo '{{"method":"turn/started","params":{{"turn":{{"id":"tu-build"}}}}}}'
      echo '{{"method":"turn/completed","params":{{"turn":{{"id":"tu-build","status":"completed"}}}}}}' ;;
  esac
done
"##,
                log = turns_log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cfg = LaunchConfig {
            program: path,
            args: vec!["app-server".to_string()],
            env: Default::default(),
            secret_env: Vec::new(),
            env_deny: Vec::new(),
        };
        let asked = Arc::new(Mutex::new(Vec::new()));
        let gate_asked = asked.clone();
        let gate: PermissionGate = Arc::new(move |p: NativePermission| {
            let asked = gate_asked.clone();
            Box::pin(async move {
                asked.lock().await.push(p);
                crate::native::PermissionDecision::from(true)
            })
        });

        let agent = AppServerAgent::start(&cfg, "auto", None, None, None, gate, None)
            .await
            .expect("handshake + thread/start");
        let mut events = agent.events.lock().unwrap().take().unwrap();
        let mut ends = agent.turn_end.lock().unwrap().take().unwrap();
        agent.set_mode("plan").await;
        agent.prompt("add a feature").await.expect("turn/start");

        // One turn end for the whole plan → approve → build sequence.
        let end = tokio::time::timeout(Duration::from_secs(15), ends.recv())
            .await
            .expect("turn ends")
            .unwrap();
        assert_eq!(end.stop_reason, "completed");
        assert_eq!(
            std::fs::read_to_string(&turns_log).unwrap().lines().collect::<Vec<_>>(),
            ["plan", "build"]
        );
        let asked = asked.lock().await;
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].tool_name, "ExitPlanMode");
        assert!(asked[0].input["plan"].as_str().unwrap().contains("Write code"));
        // Approval returns to the mode chosen before planning.
        let mut back_to = None;
        while let Ok(e) = events.try_recv() {
            if let MiraEvent::Modes { current, .. } = e.event {
                back_to = Some(current);
            }
        }
        assert_eq!(back_to.as_deref(), Some("auto"));
        assert!(!agent.turns.in_plan_mode().await);
        agent.shutdown().await;
    }

    #[test]
    fn codex_rate_limits_become_plan_windows() {
        let v = json!({
            "rateLimits": {
                "primary": { "usedPercent": 72, "windowDurationMins": 300, "resetsAt": 1791161140 },
                "secondary": { "usedPercent": 43, "windowDurationMins": 10080, "resetsAt": 1791707035 }
            }
        });
        let windows = codex_rate_limit_windows(&v);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].name, "five_hour");
        assert_eq!(windows[0].utilization, 0.72);
        assert_eq!(windows[0].resets_at, Some(1791161140));
        assert_eq!(windows[1].name, "seven_day");
        assert_eq!(windows[1].utilization, 0.43);
        assert_eq!(windows[1].resets_at, Some(1791707035));
    }

    #[tokio::test]
    async fn account_probe_fetches_models_without_opening_a_thread() {
        let (cfg, _dir) = fake_codex();
        let probe = probe_handshake(&cfg).await.expect("probe");
        assert_eq!(
            probe
                .models
                .iter()
                .map(|m| m.value.as_str())
                .collect::<Vec<_>>(),
            ["gpt-5", "gpt-5-mini"]
        );
    }

    #[test]
    fn mira_tools_ride_on_the_thread_config() {
        let mcp = crate::session::MiraMcp {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "tok".into(),
        };
        let (m, p) = thread_open_params(
            Some("/r".into()),
            "untrusted",
            "read-only",
            Some("gpt-5"),
            None,
            Some(&mcp),
        );
        assert_eq!(m, "thread/start");
        assert_eq!(
            p["config"]["mcp_servers.mira"]["url"],
            "http://127.0.0.1:1/mcp"
        );
        assert_eq!(
            p["config"]["mcp_servers.mira"]["http_headers"]["Authorization"],
            "Bearer tok"
        );
        assert_eq!(p["model"], "gpt-5");
        let (m, p) = thread_open_params(
            None,
            "never",
            "danger-full-access",
            None,
            Some("th-9"),
            None,
        );
        assert_eq!(m, "thread/resume");
        assert_eq!(p["threadId"], "th-9");
        assert_eq!(p["config"]["tools.update_plan.enabled"], true);
        assert!(
            p["config"].get("mcp_servers.mira").is_none(),
            "no tool server, no mcp override"
        );
    }

    /// The fake's id extraction is load-bearing: if it ever returns
    /// anything but the bare request id, every reply it echoes is
    /// malformed, the frame parser drops it silently, and the test that
    /// uses it fails as an opaque 30s timeout rather than a clear diff.
    ///
    /// `turn_start_params` embeds multi-paragraph developer instructions,
    /// which is exactly the text a greedy `.*"id":N.*` used to match into.
    /// The first `"id":N` of a JSON-RPC frame, read the way the fake
    /// script's `id_of` reads it (anchored at the head of the line).
    fn regex_lite() -> impl Fn(&str) -> Option<String> {
        move |line: &str| {
            let prefix = r#"{"jsonrpc":"2.0","id":"#;
            let rest = line.strip_prefix(prefix)?;
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!digits.is_empty() && rest[digits.len()..].starts_with(',')).then_some(digits)
        }
    }

    #[test]
    fn the_fakes_id_extraction_survives_a_full_turn_start_request() {
        let (_cfg, _dir) = fake_codex();
        let params = turn_start_params(
            "th-1",
            "hello",
            Some("gpt-5"),
            Some("/tmp"),
            "on-request",
            "workspace-write",
            "never-ask",
            Some("medium"),
            None, "approval-required", false
        );
        let frame = serde_json::json!({
            "jsonrpc": "2.0", "id": 7, "method": "turn/start", "params": params,
        });
        let line = serde_json::to_string(&frame).unwrap();
        // The extraction the shell script performs, mirrored: anchored to
        // the head of the frame, so multi-line params can't be matched into.
        let re = regex_lite();
        assert_eq!(re(&line).as_deref(), Some("7"), "frame: {line}");
        // The shape the old greedy expression matched into.
        assert!(line.contains("request_user_input") || line.contains("Collaboration Mode"));
    }

    #[tokio::test]
    async fn a_full_app_server_session() {
        let (cfg, _dir) = fake_codex();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let gate_asked = asked.clone();
        let gate: PermissionGate = Arc::new(move |p: NativePermission| {
            let asked = gate_asked.clone();
            Box::pin(async move {
                asked.lock().await.push(p);
                crate::native::PermissionDecision::from(true)
            })
        });

        let agent = AppServerAgent::start(&cfg, "approval-required", None, None, None, gate, None)
            .await
            .expect("handshake + thread/start");
        assert_eq!(agent.session_id().await.as_deref(), Some("th-1"));

        let mut events = agent.events.lock().unwrap().take().expect("events rx");
        let mut ends = agent.turn_end.lock().unwrap().take().expect("turn end rx");

        // The catalog arrives as config options without any turn.
        let mut saw_models = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline && !saw_models {
            let Some(e) = events.recv().await else { break };
            if let MiraEvent::ConfigOptions { options } = &e.event {
                if let Some(m) = options.iter().find(|o| o.id == "model") {
                    let vs: Vec<_> = m.values.iter().map(|x| x.value.as_str()).collect();
                    assert!(
                        vs.contains(&"gpt-5") && vs.contains(&"gpt-5-mini"),
                        "{vs:?}"
                    );
                    saw_models = true;
                }
            }
        }
        assert!(saw_models, "model/list must surface as config options");

        agent.prompt("hello").await.expect("turn/start");

        // Delta, then the approval round-trip, a reroute marking, then the end.
        let mut saw_text = false;
        let mut saw_update = false;
        let mut saw_marked = false;
        let end = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                // Biased like the server pump: drain events before accepting
                // the end. An unbiased select lets the turn end win while
                // its own events are still queued, which fails the test
                // despite every frame arriving in order.
                tokio::select! {
                    biased;
                    Some(e) = events.recv() => match &e.event {
                        MiraEvent::AssistantText { text, .. } if text == "hello" => saw_text = true,
                        MiraEvent::ToolCallUpdate(c) if c.id == "i1" => saw_update = true,
                        MiraEvent::ConfigOptions { options }
                            if options.iter().any(|o| {
                                o.id == "model" && o.current.as_deref() == Some("gpt-5-mini")
                            }) => {
                                saw_marked = true;
                            }
                        _ => {}
                    },
                    Some(stop) = ends.recv() => return Some(stop),
                    else => return None,
                }
            }
        })
        .await
        .expect("turn ended")
        .expect("stop reason");
        assert!(saw_text && saw_update);
        assert_eq!(end.stop_reason, "completed");
        assert!(
            saw_marked,
            "a reroute must re-emit the list with the new current marked"
        );

        let asked = asked.lock().await;
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].tool_name, "shell");

        agent.compact().await.expect("compact");
        assert!(
            agent.query_rate_limits().await.is_some(),
            "rate limits readable"
        );
        agent.shutdown().await;
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn async_questions_are_message_requests_not_live_rpc_permissions() {
        let payload = json!({"threadId":"t", "turnId":"turn", "item":{"id":"i", "type":"agentMessage", "delivery":"async", "questions":[{"title":"Pick", "options":["A", "B"]}]}});
        for method in ["item/started", "item/completed"] {
            let actions = map_notification(method, &payload);
            assert_eq!(actions.len(), 1);
            match &actions[0] {
                AppServerAction::Event(NormalizedEvent {
                    event: MiraEvent::RuntimeRequest(r),
                    ..
                }) => {
                    assert_eq!(r.native_thread_id, "t");
                    assert_eq!(r.native_id, "i");
                    assert_eq!(
                        r.response_capability,
                        crate::runtime::ResponseCapability::Message
                    );
                    assert_eq!(r.questions[0].options, vec!["A", "B"]);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    #[test]
    fn goals_remain_active_between_turns_but_release_on_pause_or_limits() {
        let mut activity = crate::runtime::RuntimeActivity::default();
        for (status, pending) in [
            ("active", true),
            ("paused", false),
            ("active", true),
            ("usageLimited", false),
            ("active", true),
            ("complete", false),
        ] {
            for action in map_notification(
                "thread/goal/updated",
                &json!({"threadId":"t", "goal":{"status":status, "objective":"finish"}}),
            ) {
                if let AppServerAction::Event(NormalizedEvent {
                    event: MiraEvent::RuntimeWork(w),
                    ..
                }) = action
                {
                    activity.work(&w);
                }
            }
            assert_eq!(activity.has_pending_work(), pending, "{status}");
        }
    }
}

#[cfg(test)]
mod thread_isolation_tests {
    use super::*;
    use crate::conn::AgentCallback;
    #[tokio::test]
    async fn child_threads_cannot_redirect_or_finish_the_root_turn() {
        let (tx, mut events) = mpsc::channel(16);
        let (end_tx, mut ends) = mpsc::channel(16);
        let root = Arc::new(Mutex::new(Some("root".to_owned())));
        let active = Arc::new(Mutex::new(Some("root-turn".to_owned())));
        let callbacks = Callbacks {
            tx,
            end_tx,
            thread_id: root.clone(),
            active_turn: active.clone(),
            goal_active: Default::default(),
            rate_limits: Default::default(),
            items: Default::default(),
            models: Default::default(),
            gate: Arc::new(|_| Box::pin(async { Default::default() })),
            turns: Arc::new(TurnContext {
                model: Default::default(),
                mode: Mutex::new("approval-required".into()),
                build_mode: Mutex::new("approval-required".into()),
                cwd: None,
                effort: None,
                service_tier: None,
                mira_tools: false,
            }),
            proposed_plan: Default::default(),
            plan_review: mpsc::channel(1).0,
        };
        callbacks
            .on_notification("thread/started", json!({"thread":{"id":"child"}}))
            .await;
        callbacks
            .on_notification(
                "turn/started",
                json!({"threadId":"child", "turn":{"id":"child-turn"}}),
            )
            .await;
        callbacks
            .on_notification(
                "item/agentMessage/delta",
                json!({"threadId":"child", "itemId":"child-message", "delta":"child narration"}),
            )
            .await;
        callbacks
            .on_notification(
                "turn/completed",
                json!({"threadId":"child", "turn":{"id":"child-turn", "status":"completed"}}),
            )
            .await;
        assert_eq!(root.lock().await.as_deref(), Some("root"));
        assert_eq!(active.lock().await.as_deref(), Some("root-turn"));
        assert!(ends.try_recv().is_err());
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event.event, MiraEvent::AssistantText { .. }));
        }
        callbacks
            .on_notification(
                "turn/completed",
                json!({"threadId":"root", "turn":{"id":"root-turn", "status":"completed"}}),
            )
            .await;
        assert!(ends.try_recv().is_ok());
        assert!(active.lock().await.is_none());
    }
}

#[cfg(all(test, unix))]
mod async_transport_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn async_questions_keep_streaming_and_answer_steers_the_owned_turn() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("codex-fixture");
        std::fs::write(&program, r#"#!/bin/sh
id_of() { printf '%s' "$1" | sed -E 's/^\{"jsonrpc":"2\.0","id":([0-9]+),.*/\1/'; }
while IFS= read -r line; do
  id=$(id_of "$line")
  case "$line" in
    *'"method":"thread/start"'*) echo "{\"id\":$id,\"result\":{\"threadId\":\"root\"}}" ;;
    *'"method":"turn/start"'*)
      echo '{"method":"turn/started","params":{"threadId":"root","turn":{"id":"turn-1"}}}'
      echo '{"method":"item/completed","params":{"threadId":"root","turnId":"turn-1","item":{"id":"question","type":"agentMessage","delivery":"async","questions":[{"title":"Which option?","options":["A","B"]}]}}}'
      echo '{"method":"item/agentMessage/delta","params":{"threadId":"root","itemId":"message","delta":"still working"}}'
      echo "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}" ;;
    *'"method":"turn/steer"'*)
      case "$line" in
        *'"expectedTurnId":"turn-1"'*'"threadId":"root"'*|*'"threadId":"root"'*'"expectedTurnId":"turn-1"'*)
          echo "{\"id\":$id,\"result\":{}}"
          echo '{"method":"turn/completed","params":{"threadId":"root","turn":{"id":"turn-1","status":"completed"}}}' ;;
        *) echo "{\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"wrong turn ownership\"}}" ;;
      esac ;;
    *'"method":"initialized"'*) ;;
    *) echo "{\"id\":$id,\"result\":{}}" ;;
  esac
done
"#).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut launch = launch(program);
        launch.args.clear();
        let gate: PermissionGate = Arc::new(|_| {
            Box::pin(async { panic!("async question must not enter a blocking RPC gate") })
        });
        let agent =
            AppServerAgent::start(&launch, "approval-required", None, None, None, gate, None)
                .await
                .unwrap();
        let mut events = agent.events.lock().unwrap().take().unwrap();
        let mut ends = agent.turn_end.lock().unwrap().take().unwrap();
        agent.prompt("work").await.unwrap();
        let mut question = false;
        let mut text = false;
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = events.recv().await {
                match event.event {
                    MiraEvent::RuntimeRequest(_) => question = true,
                    MiraEvent::AssistantText { text: value, .. } if value == "still working" => {
                        text = true
                    }
                    _ => {}
                }
                if question && text {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(question && text);
        assert!(ends.try_recv().is_err());
        agent.steer("A").await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(2), ends.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!ended.is_error);
        agent.shutdown().await;
    }
}
