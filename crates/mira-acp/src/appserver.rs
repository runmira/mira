//! Codex via `codex app-server` (JSON-RPC over stdio).
//!
//! Unlike `codex exec`, which is one process per prompt, `app-server` holds
//! multi-turn threads in a single process: `thread/start` and `thread/resume`
//! for sessions, `turn/start` and `turn/interrupt` for turns, `model/list`
//! for the advertised catalog, `account/rateLimits/read` for usage state,
//! and `thread/compact/start` for compaction. Approvals arrive as
//! server-initiated JSON-RPC requests that must be answered.
//!

use std::sync::Arc;
use std::time::Duration;

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

/// Codex's runtime modes, which are approval policy + sandbox, not models.
pub const RUNTIME_MODES: &[(&str, &str)] = &[
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
        _ => ("untrusted", "read-only"),
    }
}

/// Map Mira's permission posture onto a runtime mode id.
pub fn mode_for_permission(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "approval-required",
        PermissionMode::AcceptEdits | PermissionMode::Auto => "auto-accept-edits",
    }
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
#[derive(Clone, Debug, PartialEq)]
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
    let item = p.get("item");
    let item_id = item
        .and_then(|i| i.get("id"))
        .and_then(Value::as_str)
        .or_else(|| p.get("itemId").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();

    match method {
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
                    event: MiraEvent::ToolCallUpdate(ToolCallState {
                        id: item_id,
                        title: String::new(),
                        name: None,
                        kind: Some(ToolKind::Execute),
                        status: ToolCallStatus::InProgress,
                        content: vec![ToolContent::Content {
                            text: text.to_string(),
                        }],
                        locations: Vec::new(),
                        raw_input: None,
                        raw_output: None,
                    }),
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

        "turn/completed" => vec![AppServerAction::TurnEnded {
            stop_reason: "completed".to_string(),
            is_error: false,
        }],
        "turn/aborted" => vec![AppServerAction::TurnEnded {
            stop_reason: "cancelled".to_string(),
            is_error: true,
        }],
        "error" => vec![AppServerAction::TurnEnded {
            stop_reason: "error".to_string(),
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
    }
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
    match itype {
        "command_execution" | "file_change" | "mcpToolCall" | "tool_call" => {
            let title = i
                .get("command")
                .and_then(Value::as_str)
                .or_else(|| i.get("tool").and_then(Value::as_str))
                .unwrap_or(itype);
            let kind = match itype {
                "file_change" => ToolKind::Edit,
                "command_execution" => ToolKind::Execute,
                _ => ToolKind::Other,
            };
            if started {
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("tool_call"),
                    event: MiraEvent::ToolCall(ToolCallState {
                        id: id.to_string(),
                        title: title.to_string(),
                        name: Some(itype.to_string()),
                        kind: Some(kind),
                        status: ToolCallStatus::InProgress,
                        content: Vec::new(),
                        locations: Vec::new(),
                        raw_input: i
                            .get("command")
                            .cloned()
                            .or_else(|| i.get("input").cloned()),
                        raw_output: None,
                    }),
                })]
            } else {
                let mut content = Vec::new();
                if let Some(out) = i
                    .get("aggregated_output")
                    .or_else(|| i.get("output"))
                    .and_then(Value::as_str)
                {
                    content.push(ToolContent::Content {
                        text: out.to_string(),
                    });
                }
                let failed = i.get("exit_code").and_then(Value::as_i64).unwrap_or(0) != 0
                    || i.get("status").and_then(Value::as_str) == Some("failed");
                vec![AppServerAction::Event(NormalizedEvent {
                    source: variant("tool_call_update"),
                    event: MiraEvent::ToolCallUpdate(ToolCallState {
                        id: id.to_string(),
                        title: title.to_string(),
                        name: Some(itype.to_string()),
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
        // Agent prose arrives as deltas; the completed item would repeat it.
        "agent_message" | "reasoning" => vec![],
        _ => vec![],
    }
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
        "item/fileChange/requestApproval" => (
            "apply_patch",
            params
                .get("changes")
                .cloned()
                .unwrap_or_else(|| params.clone()),
        ),
        "item/permissions/requestApproval" => ("permission", params.clone()),
        "mcpServer/elicitation/request" => ("mcp", params.clone()),
        "item/tool/requestUserInput" => ("question", params.clone()),
        _ => return None,
    };
    Some(NativePermission {
        request_id: request_id.to_string(),
        tool_name: tool.to_string(),
        input,
        ..Default::default()
    })
}

/// Answer an approval request.
///
/// Decision literals verified against the generated app-server schema
/// (`CommandExecutionRequestApprovalResponse.decision`, same literals for
/// file changes). The `requestUserInput` answer record
/// (`{answers: {questionId: {answers: [...]}}}`) is also verified; choosing
/// the first option on allow is best-effort — UNVERIFIED against real Codex.
pub fn approval_response(method: &str, params: &Value, allow: bool) -> Value {
    match method {
        "item/tool/requestUserInput" => {
            let questions = params
                .get("questions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut answers = serde_json::Map::new();
            if allow {
                for q in &questions {
                    let qid = q.get("id").and_then(Value::as_str).unwrap_or_default();
                    // UNVERIFIED: the option identifier scheme. First option's
                    // label is the best available guess.
                    let first = q
                        .get("options")
                        .and_then(Value::as_array)
                        .and_then(|o| o.first())
                        .and_then(|o| o.get("label").or_else(|| o.get("id")))
                        .and_then(Value::as_str)
                        .unwrap_or("yes");
                    answers.insert(qid.to_string(), json!({ "answers": [first] }));
                }
            }
            json!({ "answers": answers })
        }
        _ => json!({ "decision": if allow { "accept" } else { "decline" } }),
    }
}

/// Parse one `model/list` entry. Field names are defensive: the entry shape
/// was not fully observable, so anything with an id-like and a label-like
/// field counts.
pub fn parse_model_entry(e: &Value) -> Option<CodexModel> {
    let value = e
        .get("id")
        .or_else(|| e.get("slug"))
        .or_else(|| e.get("model"))
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
}

/// Probe a Codex CLI without opening a thread: handshake plus account.
///
/// Short-lived by construction — the process is shut down before returning,
/// so a probe never leaves an app-server running. Anything failing degrades
/// to `None` rather than an error, because absence and failure report the
/// same way to the caller.
pub async fn probe_handshake(program: &std::path::Path) -> Option<AppServerProbe> {
    use crate::conn::NullCallbacks;
    let launch = launch(program.to_path_buf());
    let process = AgentProcess::spawn(&launch, Arc::new(NullCallbacks))
        .await
        .ok()?;
    let conn = process.conn();

    async fn call(conn: &crate::conn::Connection, method: &str, params: Value) -> Option<Value> {
        tokio::time::timeout(CALL_TIMEOUT, conn.request::<Value>(method, params))
            .await
            .ok()?
            .ok()
    }

    let init = call(
        conn,
        "initialize",
        json!({ "clientInfo": { "name": "mira-probe" } }),
    )
    .await?;
    let _ = conn.notify("initialized", json!({})).await;
    let account = call(conn, "account/read", Value::Null).await;
    process.shutdown(Duration::from_millis(500)).await;

    let user_agent = init
        .get("userAgent")
        .or_else(|| init.get("user_agent"))
        .and_then(Value::as_str)
        .unwrap_or("codex")
        .to_string();
    let acct = account.as_ref().and_then(|a| a.get("account"));
    Some(AppServerProbe {
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
    rate_limits: Arc<Mutex<Option<Value>>>,
    /// The fetched catalog, retained so a later `model/rerouted` can re-emit
    /// the same list with the new current marked.
    models: Arc<Mutex<Vec<CodexModel>>>,
    gate: PermissionGate,
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
        let Some(perm) = permission_from_request(method, &rid, &params) else {
            return Err(crate::conn::ConnError::Unhandled {
                method: method.to_string(),
            });
        };
        let allowed = (self.gate)(perm).await.allow;
        Ok(approval_response(method, &params, allowed))
    }

    async fn on_notification(&self, method: &str, params: Value) {
        for action in map_notification(method, &params) {
            match action {
                AppServerAction::SessionId(tid) => {
                    *self.thread_id.lock().await = Some(tid);
                }
                AppServerAction::Event(e) => {
                    let _ = self.tx.send(e).await;
                }
                AppServerAction::TurnEnded {
                    stop_reason,
                    is_error,
                } => {
                    if let Some(tid) = params
                        .get("turn")
                        .and_then(|t| t.get("id"))
                        .and_then(Value::as_str)
                    {
                        *self.active_turn.lock().await = Some(tid.to_string());
                    }
                    let _ = self
                        .end_tx
                        .send(TurnEnd {
                            stop_reason,
                            is_error,
                            rate_limited: None,
                        })
                        .await;
                }
                AppServerAction::RateLimits(v) => {
                    *self.rate_limits.lock().await = Some(v);
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
        if method == "turn/started" {
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
    rate_limits: Arc<Mutex<Option<Value>>>,
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
    ) -> Result<Self, NativeError> {
        let (policy, sandbox) = policy_for_mode(runtime_mode);
        let (tx, rx) = mpsc::channel(256);
        let (end_tx, end_rx) = mpsc::channel(16);
        let model_tx = tx.clone();
        let thread_id = Arc::new(Mutex::new(None));
        let active_turn = Arc::new(Mutex::new(None));
        let rate_limits = Arc::new(Mutex::new(None));
        let models_retained: Arc<Mutex<Vec<CodexModel>>> = Arc::new(Mutex::new(Vec::new()));

        let process = AgentProcess::spawn(
            launch_cfg,
            Arc::new(Callbacks {
                tx,
                end_tx,
                thread_id: thread_id.clone(),
                active_turn: active_turn.clone(),
                rate_limits: rate_limits.clone(),
                models: models_retained.clone(),
                gate,
            }),
        )
        .await
        .map_err(|e| NativeError::Other(e.to_string()))?;
        let conn = process.conn();

        async fn call(
            conn: &crate::conn::Connection,
            method: &str,
            params: Value,
        ) -> Result<Value, NativeError> {
            tokio::time::timeout(CALL_TIMEOUT, conn.request::<Value>(method, params))
                .await
                .map_err(|_| NativeError::Other(format!("{method} timed out")))?
                .map_err(|e| NativeError::Other(e.to_string()))
        }

        // Verified handshake shape: `initialize` answers with agent facts.
        let _init: Value = call(
            conn,
            "initialize",
            json!({ "clientInfo": { "name": "mira" } }),
        )
        .await?;
        let _ = conn.notify("initialized", json!({})).await;

        let mut start_params = json!({
            "cwd": cwd.unwrap_or_else(|| ".".to_string()),
            "approvalPolicy": policy,
            "sandbox": sandbox,
        });
        if let Some(m) = model.as_ref() {
            start_params["model"] = json!(m);
        }
        let opened: Value = if let Some(tid) = resume_thread_id.as_ref() {
            start_params["threadId"] = json!(tid);
            start_params["excludeTurns"] = json!(true);
            call(conn, "thread/resume", start_params).await?
        } else {
            call(conn, "thread/start", start_params).await?
        };
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
        // Unmarked: app-server does not report the running model at thread
        // open, and the picker shows the list without pretending.
        let models = Self::fetch_models(conn).await;
        if !models.is_empty() {
            *models_retained.lock().await = models.clone();
            let _ = model_tx.send(config_options_event(&models, None)).await;
        }

        Ok(Self {
            process,
            thread_id,
            active_turn,
            rate_limits,
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
            let page: Value = match tokio::time::timeout(
                CALL_TIMEOUT,
                conn.request::<Value>("model/list", params),
            )
            .await
            {
                Ok(Ok(p)) => p,
                _ => break,
            };
            for e in page
                .get("data")
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
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
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
        let tid = self
            .session_id()
            .await
            .ok_or_else(|| NativeError::Other("no codex thread yet".to_string()))?;
        let res: Value = tokio::time::timeout(
            CALL_TIMEOUT,
            self.process
                .conn()
                .request::<Value>("turn/start", json!({ "threadId": tid, "prompt": text })),
        )
        .await
        .map_err(|_| NativeError::Other("turn/start timed out".to_string()))?
        .map_err(|e| NativeError::Other(e.to_string()))?;
        if let Some(id) = res
            .get("turn")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
        {
            *self.active_turn.lock().await = Some(id.to_string());
        }
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), NativeError> {
        let tid = self.session_id().await.unwrap_or_default();
        let mut params = json!({ "threadId": tid });
        if let Some(turn) = self.active_turn.lock().await.clone() {
            params["turnId"] = json!(turn);
        }
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
        tokio::time::timeout(
            CALL_TIMEOUT,
            self.process
                .conn()
                .request::<Value>("thread/compact/start", json!({ "threadId": tid })),
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
        tokio::time::timeout(
            CALL_TIMEOUT,
            self.process
                .conn()
                .request::<Value>("account/rateLimits/read", Value::Null),
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

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
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
    fn approvals_answer_with_verified_decision_literals() {
        // `{"decision": ...}` verified against the generated app-server schema.
        let ok = approval_response(
            "item/commandExecution/requestApproval",
            &v(r#"{"command":"ls"}"#),
            true,
        );
        assert_eq!(ok["decision"], "accept");
        let no = approval_response(
            "item/fileChange/requestApproval",
            &v(r#"{"changes":[]}"#),
            false,
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
        assert_eq!(mode_views().len(), 4);
    }

    /// A stand-in `codex app-server`: handshake, thread, paged models, a turn
    /// with an approval round-trip, rate limits, compaction.
    fn fake_codex() -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex");
        std::fs::write(
            &path,
            r#"#!/bin/sh
id_of() { echo "$1" | sed 's/.*"id":\([^,}]*\).*/\1/'; }
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(id_of "$line")
      echo "{\"id\":$id,\"result\":{\"userAgent\":\"fake-codex\",\"codexHome\":\".\",\"platformFamily\":\"unix\",\"platformOs\":\"linux\"}}" ;;
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

        let agent = AppServerAgent::start(&cfg, "approval-required", None, None, None, gate)
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
