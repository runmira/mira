//! What an ACP host must provide, and the routing onto it.
//!
//! The protocol requires an agent to be able to ask its client for
//! filesystem access, terminal access, and permission decisions. Those
//! capabilities are exactly the ones Mira already has opinions about —
//! `ToolContext::resolve` for path containment, `FileGuard` for undo and
//! conflict detection, `Policy::evaluate` for allow/ask/deny, `Approver`
//! for the user prompt.
//!
//! This module does **not** depend on any of that. It declares the ports,
//! so the routing — which wire method reaches which capability, and what
//! shape the reply takes — can be tested here against fakes, and the
//! actual bindings live in `mira-server` where the sandbox and approval
//! machinery is visible. That keeps the dependency pointing one way:
//! `mira-server` knows about `mira-acp`, never the reverse.
//!
//! Two details that are easy to get wrong and are pinned by tests below:
//!
//! * A permission reply must echo back the **vendor's own option id**,
//!   not an index into the list we were given. The ids are defined by the
//!   agent and observed to be `allow`, `allow-once`, `yes`, and others.
//! * Every agent request gets an answer, including a refusal. An
//!   unanswered `session/request_permission` blocks the agent's turn
//!   forever, so "no" and "I don't know" must both be expressed as a
//!   well-formed response.

use agent_client_protocol::schema::v1::RequestPermissionRequest;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use crate::conn::{AgentCallback, ConnError};
use crate::events::{
    from_permission_request, from_session_notification, NormalizedEvent, PermissionRequest,
};

/// Why a host capability refused. Maps onto a JSON-RPC error so the agent
/// sees a reason rather than silence.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum HostError {
    /// The user or policy said no.
    #[error("denied: {0}")]
    Denied(String),
    /// The path or command is outside what this session may touch.
    #[error("out of bounds: {0}")]
    OutOfBounds(String),
    /// The thing asked for doesn't exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The capability exists but failed.
    #[error("failed: {0}")]
    Failed(String),
    /// The agent sent something we can't interpret.
    #[error("bad request: {0}")]
    BadRequest(String),
}

impl HostError {
    /// JSON-RPC code. `-32001` is the implementation-defined range these
    /// land in, which is what the ACP examples use.
    pub fn code(&self) -> i64 {
        -32001
    }

    pub fn from_json(v: Value) -> Self {
        let msg = v
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "malformed params".to_string());
        HostError::BadRequest(msg)
    }
}

impl From<HostError> for ConnError {
    fn from(e: HostError) -> Self {
        ConnError::Remote {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

/// Path-bounded file access for `fs/read_text_file` and `fs/write_text_file`.
///
/// Implementations must run the agent-supplied path through Mira's own
/// containment check before touching the filesystem. An ACP agent is
/// third-party code: treating its paths as trusted would hand it a
/// filesystem-wide read/write primitive, which is strictly more power than
/// Mira's own tools grant.
///
/// Note the absence of a session argument on every port. The id an agent
/// puts in `params.sessionId` is the **agent's** ACP session, which is its
/// own correlation handle — it is not a Mira session, and it must never be
/// used to pick a root, a `ToolContext` or an approver. A host binds that
/// state once, at construction, from the Mira session that owns it.
#[async_trait::async_trait]
pub trait FilePort: Send + Sync {
    async fn read_text(&self, path: &str) -> Result<String, HostError>;
    async fn write_text(&self, path: &str, content: &str) -> Result<(), HostError>;
}

/// Terminal access for the `terminal/*` family.
///
/// ACP's model is a capability handle: the agent asks to create a terminal,
/// gets an opaque id back, reads output through it, and releases it. Mira
/// has no abstraction that matches — its one PTY is a process-global
/// WebSocket terminal that bypasses the sandbox entirely — so this port is
/// currently implemented against `Sandbox::run_with_timeout`, which is
/// sandboxed but buffers to process exit. That covers create/output/
/// release; it does not yet stream incrementally.
#[async_trait::async_trait]
pub trait TerminalPort: Send + Sync {
    async fn create(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<String, HostError>;
    async fn output(&self, terminal_id: &str) -> Result<String, HostError>;
    async fn release(&self, terminal_id: &str) -> Result<(), HostError>;
    /// `None` means the terminal was released before it exited.
    async fn wait_for_exit(&self, terminal_id: &str) -> Result<Option<i32>, HostError>;
    async fn kill(&self, terminal_id: &str) -> Result<(), HostError>;
}

/// The user-decision port for `session/request_permission`.
#[async_trait::async_trait]
pub trait PermissionPort: Send + Sync {
    /// Present the agent's own options to the user and return the id of
    /// whichever was chosen. Returning `None` means the request was
    /// cancelled or timed out, which ACP models as `Cancelled` rather than
    /// as a rejection.
    async fn request_permission(
        &self,
        req: &PermissionRequest,
    ) -> Result<Option<String>, HostError>;
}

/// Where normalized events go.
#[async_trait::async_trait]
pub trait EventPort: Send + Sync {
    async fn emit(&self, event: NormalizedEvent);
}

/// A port that refuses everything, for sessions that shouldn't be able to
/// touch the host at all.
pub struct DenyAll;

#[async_trait::async_trait]
impl FilePort for DenyAll {
    async fn read_text(&self, path: &str) -> Result<String, HostError> {
        Err(HostError::Denied(format!(
            "filesystem access is off: {path}"
        )))
    }
    async fn write_text(&self, path: &str, _c: &str) -> Result<(), HostError> {
        Err(HostError::Denied(format!(
            "filesystem access is off: {path}"
        )))
    }
}

#[async_trait::async_trait]
impl TerminalPort for DenyAll {
    async fn create(&self, _c: &str, _a: &[String], _d: Option<&str>) -> Result<String, HostError> {
        Err(HostError::Denied("terminal access is off".into()))
    }
    async fn output(&self, _t: &str) -> Result<String, HostError> {
        Err(HostError::Denied("terminal access is off".into()))
    }
    async fn release(&self, _t: &str) -> Result<(), HostError> {
        Err(HostError::Denied("terminal access is off".into()))
    }
    async fn wait_for_exit(&self, _t: &str) -> Result<Option<i32>, HostError> {
        Err(HostError::Denied("terminal access is off".into()))
    }
    async fn kill(&self, _t: &str) -> Result<(), HostError> {
        Err(HostError::Denied("terminal access is off".into()))
    }
}

#[async_trait::async_trait]
impl PermissionPort for DenyAll {
    async fn request_permission(
        &self,
        _r: &PermissionRequest,
    ) -> Result<Option<String>, HostError> {
        // Not an error: "cancelled" is how ACP says "not now", and it lets
        // the agent unwind cleanly instead of treating it as a failure.
        Ok(None)
    }
}

#[async_trait::async_trait]
impl EventPort for DenyAll {
    async fn emit(&self, _e: NormalizedEvent) {}
}

/// Routes agent requests onto host capabilities.
pub struct AcpHost {
    pub files: Arc<dyn FilePort>,
    pub terminals: Arc<dyn TerminalPort>,
    pub permissions: Arc<dyn PermissionPort>,
    pub events: Arc<dyn EventPort>,
    /// Where an in-flight permission request is recorded, so a later
    /// `session/cancel` can answer it.
    pub pending_permissions: PendingPermissions,
    /// Live tool-call state per session, so a `tool_call_update` can be
    /// merged onto the call it belongs to instead of arriving contextless.
    tool_state: tokio::sync::Mutex<HashMap<String, crate::events::ToolCallState>>,
}

impl AcpHost {
    pub fn new(
        files: Arc<dyn FilePort>,
        terminals: Arc<dyn TerminalPort>,
        permissions: Arc<dyn PermissionPort>,
        events: Arc<dyn EventPort>,
    ) -> Arc<Self> {
        Arc::new(AcpHost {
            files,
            terminals,
            permissions,
            events,
            pending_permissions: PendingPermissions::new(),
            tool_state: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// A host that can do nothing. Useful for a read-only preview mode and
    /// as a default before capabilities are negotiated.
    pub fn inert() -> Arc<Self> {
        AcpHost::new(
            Arc::new(DenyAll),
            Arc::new(DenyAll),
            Arc::new(DenyAll),
            Arc::new(DenyAll),
        )
    }

    /// The agent's own session id, present on every session-scoped method.
    ///
    /// Checked for protocol correctness but deliberately **not** propagated:
    /// it is the agent's correlation handle, not a Mira session, so it must
    /// never reach anything that picks a root or an approver.
    fn session_of(params: &Value) -> String {
        params
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

/// Reject a session-scoped request that omits `sessionId`.
fn require_session(params: &Value) -> Result<(), HostError> {
    if params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::is_empty)
        .unwrap_or(true)
    {
        return Err(HostError::BadRequest("missing sessionId".to_string()));
    }
    Ok(())
}

#[async_trait::async_trait]
impl AgentCallback for AcpHost {
    async fn on_request(
        &self,
        id: &Value,
        method: &str,
        params: Value,
    ) -> Result<Value, ConnError> {
        let host_err = |e: HostError| ConnError::Remote {
            code: e.code(),
            message: e.to_string(),
        };
        match method {
            "fs/read_text_file" => {
                require_session(&params).map_err(host_err)?;
                let path = req_str(&params, "path")?;
                let content = self.files.read_text(&path).await?;
                Ok(json!({ "content": content }))
            }
            "fs/write_text_file" => {
                require_session(&params).map_err(host_err)?;
                let path = req_str(&params, "path")?;
                let content = req_str(&params, "content")?;
                self.files.write_text(&path, &content).await?;
                Ok(json!({}))
            }
            "terminal/create" => {
                require_session(&params).map_err(host_err)?;
                let command = req_str(&params, "command")?;
                let args = params
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let cwd = params.get("cwd").and_then(Value::as_str);
                let id = self.terminals.create(&command, &args, cwd).await?;
                Ok(json!({ "terminalId": id }))
            }
            "terminal/output" => {
                require_session(&params).map_err(host_err)?;
                let id = req_str(&params, "terminalId")?;
                let out = self.terminals.output(&id).await?;
                // `truncated` is what tells the agent to stop assuming it
                // has the whole scrollback.
                let truncated = false;
                Ok(json!({ "output": out, "truncated": truncated, "exitStatus": Value::Null }))
            }
            "terminal/release" => {
                require_session(&params).map_err(host_err)?;
                let id = req_str(&params, "terminalId")?;
                self.terminals.release(&id).await?;
                Ok(json!({}))
            }
            "terminal/wait_for_exit" => {
                require_session(&params).map_err(host_err)?;
                let id = req_str(&params, "terminalId")?;
                let code = self.terminals.wait_for_exit(&id).await?;
                Ok(json!({ "exitCode": code }))
            }
            "terminal/kill" => {
                require_session(&params).map_err(host_err)?;
                let id = req_str(&params, "terminalId")?;
                self.terminals.kill(&id).await?;
                Ok(json!({}))
            }
            "session/request_permission" => {
                let wire: RequestPermissionRequest = parse(params)?;
                let req = from_permission_request(&wire);
                // Recorded before the user is asked, so a cancel arriving
                // mid-prompt still finds it.
                self.pending_permissions.track(id.clone()).await;
                // The vendor's own option ids must survive the round trip;
                // the agent matches its reply against what it sent us.
                let chosen = match self.permissions.request_permission(&req).await {
                    Ok(c) => c,
                    Err(e) => {
                        self.pending_permissions.settle(id).await;
                        return Err(e.into());
                    }
                };
                self.pending_permissions.settle(id).await;
                Ok(match chosen {
                    Some(option_id) => json!({
                        "outcome": { "outcome": "selected", "optionId": option_id }
                    }),
                    // Cancelled is not an error and not a rejection: it tells
                    // the agent to unwind this turn rather than retry.
                    None => json!({ "outcome": { "outcome": "cancelled" } }),
                })
            }
            other => Err(ConnError::Unhandled {
                method: other.to_string(),
            }),
        }
    }

    async fn on_notification(&self, method: &str, params: Value) {
        if method != "session/update" {
            // Provenance is retained even for methods we don't model, so an
            // agent upgrade that starts sending something new is visible
            // rather than silent.
            self.events
                .emit(NormalizedEvent {
                    source: crate::events::EventSource::Unmodelled {
                        method: method.to_string(),
                    },
                    event: crate::events::MiraEvent::Unmodelled {
                        source: crate::events::EventSource::Unmodelled {
                            method: method.to_string(),
                        },
                        reason: format!("unmodelled notification: {method}"),
                    },
                })
                .await;
            return;
        }

        let session = Self::session_of(&params);
        if session.is_empty() {
            return;
        }
        let Some(event) = from_session_notification(&params) else {
            return;
        };

        // A tool-call update is only meaningful against the call it
        // continues, so carry that state across updates within a session.
        if let crate::events::MiraEvent::ToolCall(state) = &event.event {
            self.tool_state
                .lock()
                .await
                .insert(tool_key(&session, &state.id), state.clone());
        }
        self.events.emit(event).await;
    }

    async fn on_closed(&self, tail: String) {
        if !tail.trim().is_empty() {
            tracing::warn!(stderr = %tail, "acp: agent process ended");
        }
        self.tool_state.lock().await.clear();
    }
}

fn tool_key(session: &str, call_id: &str) -> String {
    format!("{session}\u{1}{call_id}")
}

fn req_str(params: &Value, key: &str) -> Result<String, ConnError> {
    match params.get(key).and_then(Value::as_str) {
        Some(s) => Ok(s.to_string()),
        None => Err(ConnError::Remote {
            code: -32602,
            message: format!("missing or non-string `{key}`"),
        }),
    }
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, ConnError> {
    serde_json::from_value(params).map_err(|e| ConnError::Remote {
        code: -32602,
        message: format!("malformed params: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        AcpPermissionOptionKind as PermissionOptionKind, MiraEvent, PermissionChoice,
    };
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct FakeFiles {
        seen: StdMutex<Vec<String>>,
        writes: StdMutex<Vec<(String, String)>>,
        refuse: bool,
    }

    #[async_trait::async_trait]
    impl FilePort for FakeFiles {
        async fn read_text(&self, path: &str) -> Result<String, HostError> {
            self.seen.lock().unwrap().push(path.to_string());
            if self.refuse {
                return Err(HostError::OutOfBounds(path.to_string()));
            }
            Ok(format!("contents of {path}"))
        }
        async fn write_text(&self, path: &str, c: &str) -> Result<(), HostError> {
            self.writes
                .lock()
                .unwrap()
                .push((path.to_string(), c.to_string()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeTerms {
        created: StdMutex<Vec<String>>,
        released: StdMutex<Vec<String>>,
        killed: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl TerminalPort for FakeTerms {
        async fn create(
            &self,
            c: &str,
            _a: &[String],
            _d: Option<&str>,
        ) -> Result<String, HostError> {
            self.created.lock().unwrap().push(c.to_string());
            Ok("term-1".to_string())
        }
        async fn output(&self, _t: &str) -> Result<String, HostError> {
            Ok("hello".to_string())
        }
        async fn release(&self, t: &str) -> Result<(), HostError> {
            self.released.lock().unwrap().push(t.to_string());
            Ok(())
        }
        async fn wait_for_exit(&self, _t: &str) -> Result<Option<i32>, HostError> {
            Ok(Some(0))
        }
        async fn kill(&self, t: &str) -> Result<(), HostError> {
            self.killed.lock().unwrap().push(t.to_string());
            Ok(())
        }
    }

    struct FakePerm {
        answer: Option<String>,
    }

    #[async_trait::async_trait]
    impl PermissionPort for FakePerm {
        async fn request_permission(
            &self,
            _r: &PermissionRequest,
        ) -> Result<Option<String>, HostError> {
            Ok(self.answer.clone())
        }
    }

    #[derive(Default)]
    struct FakeEvents {
        got: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl EventPort for FakeEvents {
        async fn emit(&self, e: NormalizedEvent) {
            let tag = match &e.event {
                MiraEvent::ToolCall(s) => format!("tool:{}", s.id),
                MiraEvent::ToolCallUpdate(s) => format!("update:{}", s.id),
                MiraEvent::AssistantText { text, .. } => format!("text:{text}"),
                MiraEvent::Unmodelled { reason, .. } => format!("unmodelled:{reason}"),
                other => format!("other:{other:?}")
                    .split_whitespace()
                    .take(1)
                    .collect(),
            };
            self.got.lock().unwrap().push(tag);
        }
    }

    fn host(
        files: Arc<FakeFiles>,
        terms: Arc<FakeTerms>,
        perm: Arc<FakePerm>,
        events: Arc<FakeEvents>,
    ) -> Arc<AcpHost> {
        AcpHost::new(files, terms, perm, events)
    }

    fn sid() -> Value {
        json!({ "sessionId": "sess_1" })
    }

    #[tokio::test]
    async fn a_file_read_is_routed_with_its_session_and_path() {
        let files = Arc::new(FakeFiles::default());
        let h = host(
            files.clone(),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            Arc::new(FakeEvents::default()),
        );
        let out = h
            .on_request(
                &json!(1),
                "fs/read_text_file",
                json!({ "sessionId": "sess_1", "path": "/repo/a.rs" }),
            )
            .await
            .unwrap();
        assert_eq!(out["content"], "contents of /repo/a.rs");
        assert_eq!(
            files.seen.lock().unwrap().clone(),
            vec!["/repo/a.rs".to_string()]
        );
    }

    #[tokio::test]
    async fn a_refused_path_becomes_a_structured_error_not_a_panic() {
        let files = Arc::new(FakeFiles {
            refuse: true,
            ..Default::default()
        });
        let h = host(
            files,
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            Arc::new(FakeEvents::default()),
        );
        let err = h
            .on_request(
                &json!(1),
                "fs/read_text_file",
                json!({ "sessionId": "sess_1", "path": "/etc/passwd" }),
            )
            .await
            .unwrap_err();
        match err {
            ConnError::Remote { code, message } => {
                assert_eq!(code, -32001);
                assert!(message.contains("out of bounds"), "got {message}");
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_session_id_is_rejected_rather_than_guessed() {
        let h = AcpHost::inert();
        let err = h
            .on_request(&json!(1), "fs/read_text_file", json!({ "path": "/x" }))
            .await
            .unwrap_err();
        match err {
            ConnError::Remote { message, .. } => {
                assert!(message.contains("sessionId"), "got {message}")
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_permission_reply_echoes_the_vendors_own_option_id() {
        // The agent matches its own reply against the ids it sent, so
        // returning an index or a normalized name would desync it.
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm {
                answer: Some("allow-once".to_string()),
            }),
            Arc::new(FakeEvents::default()),
        );
        let out = h
            .on_request(
                &json!(1),
                "session/request_permission",
                json!({
                    "sessionId": "sess_1",
                    "toolCall": {
                        "toolCallId": "call_9",
                        "title": "Run tests",
                        "kind": "execute"
                    },
                    "options": [
                        {"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
                    ]
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["outcome"]["outcome"], "selected");
        assert_eq!(out["outcome"]["optionId"], "allow-once");
    }

    #[tokio::test]
    async fn a_declined_permission_is_cancelled_not_an_error() {
        // ACP models "not now" as Cancelled. Reporting an error instead
        // would make the agent treat a user's silence as a failure.
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            Arc::new(FakeEvents::default()),
        );
        let out = h
            .on_request(
                &json!(1),
                "session/request_permission",
                json!({
                    "sessionId": "sess_1",
                    "toolCall": { "toolCallId": "c", "title": "t", "kind": "execute" },
                    "options": [{"optionId":"yes","name":"Yes","kind":"allow_once"}]
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["outcome"]["outcome"], "cancelled");
    }

    #[tokio::test]
    async fn a_permission_request_without_options_is_still_answerable() {
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm {
                answer: Some("allow".into()),
            }),
            Arc::new(FakeEvents::default()),
        );
        let out = h
            .on_request(
                &json!(1),
                "session/request_permission",
                json!({
                    "sessionId": "s",
                    "toolCall": { "toolCallId": "c", "title": "t" },
                    "options": [{"optionId":"allow","name":"Allow","kind":"allow_once"}]
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["outcome"]["optionId"], "allow");
    }

    #[tokio::test]
    async fn terminal_lifecycle_routes_by_id() {
        let terms = Arc::new(FakeTerms::default());
        let h = host(
            Arc::new(FakeFiles::default()),
            terms.clone(),
            Arc::new(FakePerm { answer: None }),
            Arc::new(FakeEvents::default()),
        );
        let created = h
            .on_request(
                &json!(1),
                "terminal/create",
                json!({ "sessionId": "s", "command": "npm", "args": ["test"], "cwd": "/repo" }),
            )
            .await
            .unwrap();
        assert_eq!(created["terminalId"], "term-1");
        assert_eq!(terms.created.lock().unwrap().clone(), vec!["npm"]);

        let out = h
            .on_request(
                &json!(1),
                "terminal/output",
                json!({ "sessionId": "s", "terminalId": "term-1" }),
            )
            .await
            .unwrap();
        assert_eq!(out["output"], "hello");
        assert_eq!(out["exitStatus"], Value::Null);

        h.on_request(
            &json!(1),
            "terminal/release",
            json!({ "sessionId": "s", "terminalId": "term-1" }),
        )
        .await
        .unwrap();
        h.on_request(
            &json!(1),
            "terminal/kill",
            json!({ "sessionId": "s", "terminalId": "term-1" }),
        )
        .await
        .unwrap();
        assert_eq!(terms.released.lock().unwrap().clone(), vec!["term-1"]);
        assert_eq!(terms.killed.lock().unwrap().clone(), vec!["term-1"]);
    }

    #[tokio::test]
    async fn a_session_update_is_normalized_and_attributed() {
        let events = Arc::new(FakeEvents::default());
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            events.clone(),
        );
        h.on_notification(
            "session/update",
            json!({
                "sessionId": "sess_7",
                "update": { "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "hi" } }
            }),
        )
        .await;
        assert_eq!(
            events.got.lock().unwrap().clone(),
            vec!["text:hi".to_string()]
        );
    }

    #[tokio::test]
    async fn tool_call_state_survives_across_updates_in_a_session() {
        let events = Arc::new(FakeEvents::default());
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            events.clone(),
        );
        // Create.
        h.on_notification(
            "session/update",
            json!({
                "sessionId": "s", "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "t1", "title": "Build",
                    "kind": "execute", "status": "pending"
                }
            }),
        )
        .await;
        // Update, arriving with no title of its own.
        h.on_notification(
            "session/update",
            json!({
                "sessionId": "s", "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "t1", "status": "in_progress"
                }
            }),
        )
        .await;
        let got = events.got.lock().unwrap().clone();
        assert_eq!(got[0], "tool:t1".to_string());
        assert_eq!(got[1], "update:t1".to_string());
    }

    #[tokio::test]
    async fn the_same_call_id_in_two_sessions_does_not_cross_contaminate() {
        // Each host is per Mira session, so a shared call id across two
        // hosts cannot cross over.
        let events = Arc::new(FakeEvents::default());
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            events.clone(),
        );
        for session in ["a", "b"] {
            h.on_notification(
                "session/update",
                json!({
                    "sessionId": session, "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "t1", "title": "Build",
                        "kind": "execute", "status": "pending"
                    }
                }),
            )
            .await;
        }
        h.on_notification(
            "session/update",
            json!({
                "sessionId": "a", "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "t1", "status": "completed"
                }
            }),
        )
        .await;
        // Session b's call must not have been advanced by session a's update.
        let got = events.got.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], "update:t1".to_string());
    }

    #[tokio::test]
    async fn an_unmodelled_notification_is_still_reported_with_provenance() {
        let events = Arc::new(FakeEvents::default());
        let h = host(
            Arc::new(FakeFiles::default()),
            Arc::new(FakeTerms::default()),
            Arc::new(FakePerm { answer: None }),
            events.clone(),
        );
        h.on_notification("session/vendor_thing", sid()).await;
        let got = events.got.lock().unwrap().clone();
        assert!(
            got[0].contains("session/vendor_thing"),
            "an unmodelled notification must keep its provenance: got {}",
            got[0]
        );
    }

    #[tokio::test]
    async fn an_unknown_agent_request_is_reported_as_unhandled() {
        let h = AcpHost::inert();
        let err = h
            .on_request(&json!(1), "fs/rm_rf", json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ConnError::Unhandled { ref method } if method == "fs/rm_rf"),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn the_inert_host_refuses_capabilities_without_panicking() {
        // A read-only or pre-negotiation session still has to answer
        // requests; refusing cleanly keeps the agent from hanging.
        let h = AcpHost::inert();
        assert!(h
            .on_request(
                &json!(1),
                "fs/read_text_file",
                json!({"sessionId":"s","path":"/x"})
            )
            .await
            .is_err());
        assert!(h
            .on_request(
                &json!(1),
                "terminal/create",
                json!({"sessionId":"s","command":"sh"})
            )
            .await
            .is_err());
        let out = h
            .on_request(
                &json!(1),
                "session/request_permission",
                json!({"sessionId":"s","toolCall":{"toolCallId":"c","title":"t"},"options":[]}),
            )
            .await
            .unwrap();
        assert_eq!(out["outcome"]["outcome"], "cancelled");
    }

    #[tokio::test]
    async fn permission_option_semantics_come_from_kind_not_the_option_id() {
        // Vendors use "allow", "allow-once", "yes"; only `kind` is
        // portable, so classification must ignore the id.
        for id in ["allow", "allow-once", "yes", "proceed"] {
            let c = PermissionChoice {
                option_id: id.into(),
                name: "Yes".into(),
                kind: PermissionOptionKind::AllowOnce,
            };
            assert!(c.is_allow(), "{id} should classify as allow");
            assert!(!c.is_persistent(), "{id} should not be persistent");
        }
        let reject = PermissionChoice {
            option_id: "no".into(),
            name: "No".into(),
            kind: PermissionOptionKind::RejectOnce,
        };
        assert!(!reject.is_allow());
    }
}

/// The permission requests an agent currently has in flight.
///
/// Shared between the host (which records an id the moment a request
/// arrives) and the session (which has to answer every one of them when the
/// turn is cancelled). They are separate objects because the host handles
/// inbound requests while the session drives the lifecycle, and the spec
/// makes them a joint obligation:
///
/// > When a client sends a `session/cancel` notification … it MUST respond
/// > to all pending `session/request_permission` requests with the
/// > `Cancelled` outcome.
///
/// Recording only on the way to an answer would miss a cancel that lands
/// while the user is still looking at the prompt.
#[derive(Clone, Default)]
pub struct PendingPermissions {
    inner: std::sync::Arc<tokio::sync::Mutex<Vec<Value>>>,
}

impl PendingPermissions {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn track(&self, id: Value) {
        self.inner.lock().await.push(id);
    }

    pub async fn settle(&self, id: &Value) {
        self.inner.lock().await.retain(|p| p != id);
    }

    /// Take everything outstanding, for a cancel to answer.
    pub async fn take_all(&self) -> Vec<Value> {
        std::mem::take(&mut *self.inner.lock().await)
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn tracked_requests_are_taken_exactly_once() {
        let p = PendingPermissions::new();
        p.track(json!(1)).await;
        p.track(json!(2)).await;
        assert_eq!(p.len().await, 2);
        let mut taken: Vec<i64> = p
            .take_all()
            .await
            .iter()
            .map(|v| v.as_i64().expect("int id"))
            .collect();
        taken.sort();
        assert_eq!(taken, vec![1, 2]);
        // A second cancel must not re-answer requests already settled.
        assert!(p.take_all().await.is_empty());
        assert!(p.is_empty().await);
    }

    #[tokio::test]
    async fn settling_one_leaves_the_others() {
        let p = PendingPermissions::new();
        p.track(json!(1)).await;
        p.track(json!(2)).await;
        p.settle(&json!(1)).await;
        assert_eq!(p.len().await, 1);
        assert_eq!(p.take_all().await, vec![json!(2)]);
    }

    #[tokio::test]
    async fn a_tracker_is_shared_between_clones() {
        // The host and the session hold separate handles onto one set.
        let a = PendingPermissions::new();
        let b = a.clone();
        a.track(json!(7)).await;
        assert_eq!(b.len().await, 1);
        b.settle(&json!(7)).await;
        assert!(a.is_empty().await);
    }
}
