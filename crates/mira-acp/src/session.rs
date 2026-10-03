//! The session lifecycle: `initialize` → `session/new` → `session/prompt`,
//! and `session/cancel`.
//!
//! The ordering here is not incidental. ACP requires `initialize` to
//! complete before any other method is valid, and several agents will exit
//! if a second `session/new` arrives while the first is outstanding — so
//! this layer owns that sequencing rather than leaving it to callers.
//!
//! The other thing it owns is the **cancellation contract**, which is a
//! spec MUST and the easiest thing in ACP to get wrong:
//!
//! > When a client sends a `session/cancel` notification … it MUST respond
//! > to all pending `session/request_permission` requests with the
//! > `Cancelled` outcome.
//!
//! Miss it and a cancelled turn deadlocks: the agent is blocked on a
//! permission request that will never be answered, and the user is looking
//! at a spinner. [`AcpSession::cancel`] therefore records every permission
//! request it has in flight and answers all of them on the way out.

use agent_client_protocol::schema::v1::{
    ClientCapabilities, ContentBlock, ElicitationCapabilities, ElicitationFormCapabilities,
    InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, StopReason,
};
use agent_client_protocol::schema::ProtocolVersion;
use serde_json::{json, Value};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::conn::{ConnError, Connection};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("acp: {0}")]
    Conn(#[from] ConnError),
    #[error("initialize has not completed")]
    NotInitialized,
    #[error("the agent reported protocol {got}, but this client speaks {want}")]
    VersionMismatch { got: u16, want: u16 },
    #[error("acp: {0}")]
    Other(String),
}

/// What the client advertises at `initialize`.
///
/// Only capabilities that are genuinely bound are advertised. Claiming
/// `fs` or `terminal` and then refusing the request would make an agent plan
/// around a capability that does not exist, which is worse than it not
/// asking.
#[derive(Clone, Debug, Default)]
pub struct ClientCaps {
    pub fs_read: bool,
    pub fs_write: bool,
    pub terminal: bool,
    /// Form elicitation (`elicitation/create`, `mode: "form"`): the agent
    /// may ask the user structured questions, which the host shows as
    /// Mira's question card. OpenCode only asks its `question` tool's
    /// questions — and its plan prompts — when this is advertised; without
    /// it they are cancelled on the spot ("The user dismissed this
    /// question").
    pub elicitation_form: bool,
}

impl ClientCaps {
    fn to_wire(&self) -> ClientCapabilities {
        // Built field-by-field: the type is `#[non_exhaustive]`, so a struct
        // expression is not available outside the SDK.
        let mut caps = ClientCapabilities::default();
        caps.fs.read_text_file = self.fs_read;
        caps.fs.write_text_file = self.fs_write;
        caps.terminal = self.terminal;
        if self.elicitation_form {
            caps.elicitation =
                Some(ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()));
        }
        caps
    }
}

/// Mira's tool server for one agent session: where it is, and the bearer
/// token that grants this session (and only this session) its tools.
#[derive(Clone, Debug, PartialEq)]
pub struct MiraMcp {
    pub url: String,
    pub token: String,
}

impl MiraMcp {
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

/// The env var `mira mcp-bridge` reads its token from — a token in argv
/// would show up in process listings.
pub const MIRA_MCP_TOKEN_ENV: &str = "MIRA_MCP_TOKEN";

/// Mira's tool server as an ACP `mcpServers` entry: over HTTP when the agent
/// takes HTTP servers, otherwise as a stdio server — this same `mira`
/// binary run as `mira mcp-bridge <url>` (every ACP agent takes stdio).
fn mira_mcp_entry(mcp: &MiraMcp, http: bool, exe: Option<std::path::PathBuf>) -> Option<Value> {
    if http {
        return Some(json!({
            "type": "http",
            "name": "mira",
            "url": mcp.url,
            "headers": [{ "name": "Authorization", "value": mcp.authorization() }],
        }));
    }
    let exe = exe?;
    Some(json!({
        "name": "mira",
        "command": exe,
        "args": ["mcp-bridge", mcp.url],
        "env": [{ "name": MIRA_MCP_TOKEN_ENV, "value": mcp.token }],
    }))
}

/// A live ACP conversation with one agent process.
pub struct AcpSession {
    conn: Connection,
    caps: ClientCaps,
    state: Mutex<SessionState>,
    /// The agent's session id, once `session/new` has run.
    session_id: Mutex<Option<String>>,
    /// Shared with the host, which records an id the moment a permission
    /// request arrives. See [`PendingPermissions`] for why the two have to
    /// cooperate rather than each keeping their own list.
    pending_permissions: crate::host::PendingPermissions,
}

#[derive(Default)]
struct SessionState {
    initialized: bool,
    /// The agent takes MCP servers over HTTP (`mcpCapabilities.http`).
    mcp_http: bool,
    /// Mira's own tool server for this session (browser, background
    /// processes), handed to the agent at `session/new` when it can take it.
    mira_mcp: Option<MiraMcp>,
    /// Modes the agent advertises at `initialize`, so a `set_mode` can be
    /// validated before it is sent.
    modes: Vec<Value>,
    /// Config options from `session/new`. ACP has no set-model method: the
    /// model selector *is* a `configOptions` entry with `category: "model"`.
    ///
    /// Two things about the wire shape are easy to get wrong. The option's
    /// payload is *flattened* onto the object, so `type`, `currentValue` and
    /// `options` are siblings of `id` rather than nested under `kind`. And
    /// the SDK deserializes this list with an error-skipping adapter, so an
    /// entry it cannot parse is **dropped with no error at all** — which
    /// means a wrong-shaped payload shows up as an empty model list rather
    /// than as a failure. Treat an unexpectedly short list as suspect.
    config_options: Vec<Value>,
    /// The mode the agent is currently in, and the ones it offers. These
    /// arrive with `session/new`, not `initialize`.
    current_mode: Option<String>,
    /// What the agent said about itself at `initialize`. Kept because it is
    /// the only place an agent's own name and version are available, and
    /// most adapters advertise no auth methods at all.
    version: Option<String>,
    auth_method_ids: Vec<String>,
}

impl AcpSession {
    /// `pending` is the same tracker the host records requests into; pass
    /// `host.pending_permissions.clone()` from the same `AcpHost` so a
    /// cancel can settle them. A fresh tracker works but will never see
    /// anything, so cancelling would silently leave the agent blocked.
    pub fn new(
        conn: Connection,
        caps: ClientCaps,
        pending: crate::host::PendingPermissions,
    ) -> Arc<Self> {
        Arc::new(AcpSession {
            conn,
            caps,
            state: Mutex::new(SessionState::default()),
            session_id: Mutex::new(None),
            pending_permissions: pending,
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Best-effort peek. `None` means the lock is held, which in practice
    /// means initialization is in flight.
    pub fn is_initialized(&self) -> Option<bool> {
        self.state.try_lock().ok().map(|guard| guard.initialized)
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.try_lock().ok().and_then(|g| g.clone())
    }

    /// The version the agent reported, if it reported one. Most adapters
    /// omit `agentInfo`, so `None` is normal rather than a fault.
    pub fn agent_version(&self) -> Option<String> {
        self.state.try_lock().ok().and_then(|g| g.version.clone())
    }

    /// Auth method ids the agent advertised. Empty means it authenticates out
    /// of band, which several adapters do.
    pub fn agent_auth_method_ids(&self) -> Vec<String> {
        self.state
            .try_lock()
            .map(|g| g.auth_method_ids.clone())
            .unwrap_or_default()
    }

    /// The mode the agent is in, and the ones available to switch to.
    pub fn modes(&self) -> (Option<String>, Vec<Value>) {
        match self.state.try_lock() {
            Ok(g) => (g.current_mode.clone(), g.modes.clone()),
            Err(_) => (None, Vec::new()),
        }
    }

    pub fn config_options(&self) -> Vec<Value> {
        self.state
            .try_lock()
            .map(|g| g.config_options.clone())
            .unwrap_or_default()
    }

    /// Step 1. Must succeed before anything else.
    pub async fn initialize(&self) -> Result<InitializeResponse, SessionError> {
        let mut req = InitializeRequest::new(ProtocolVersion::V1);
        req.client_capabilities = self.caps.to_wire();
        let res: InitializeResponse = self
            .conn
            .request("initialize", serde_json::to_value(req).unwrap_or(json!({})))
            .await?;

        // An agent answering a different version may be misreading our
        // requests, so refuse rather than limp along.
        if res.protocol_version != ProtocolVersion::V1 {
            return Err(SessionError::VersionMismatch {
                got: res.protocol_version.as_u16(),
                want: ProtocolVersion::V1.as_u16(),
            });
        }
        // Not an error. An agent that authenticates through its own process
        // — `claude-code-acp` delegates to the `claude` CLI's login, and
        // Codex can be signed in via the ChatGPT app — legitimately advertises
        // no ACP auth methods. Refusing to start there would lock out exactly
        // the subscription-authenticated agents this exists to support.
        if res.auth_methods.is_empty() {
            tracing::info!(
                driver = ?std::env::var("MIRA_ACP_DRIVER").ok(),
                "acp: agent advertises no auth methods; assuming it authenticates out of band"
            );
        }

        {
            let mut st = self.state.lock().await;
            st.initialized = true;
            st.mcp_http = res.agent_capabilities.mcp_capabilities.http;
            st.version = res.agent_info.as_ref().map(|i| i.version.clone());
            st.auth_method_ids = res
                .auth_methods
                .iter()
                .map(|m| m.id().to_string())
                .collect();
        }
        Ok(res)
    }

    /// Offer the agent Mira's tool server (by URL) at the next `session/new`.
    pub async fn set_mira_mcp(&self, mcp: Option<MiraMcp>) {
        self.state.lock().await.mira_mcp = mcp;
    }

    /// Step 2. `cwd` is the agent's working directory; `additional_directories`
    /// are extra roots it may reference.
    pub async fn new_session(
        &self,
        cwd: &std::path::Path,
        additional_directories: Vec<std::path::PathBuf>,
    ) -> Result<NewSessionResponse, SessionError> {
        if !self.state.lock().await.initialized {
            return Err(SessionError::NotInitialized);
        }
        let mut req = NewSessionRequest::new(cwd);
        req.additional_directories = additional_directories;
        let mut params = serde_json::to_value(req).unwrap_or(json!({}));
        // Mira's tools (its browser, background processes), as an MCP server
        // the agent connects to — only when it says it can take one over HTTP.
        let entry = {
            let st = self.state.lock().await;
            st.mira_mcp
                .as_ref()
                .and_then(|m| mira_mcp_entry(m, st.mcp_http, std::env::current_exe().ok()))
        };
        if let (Some(entry), Some(obj)) = (entry, params.as_object_mut()) {
            obj.insert("mcpServers".into(), json!([entry]));
        }
        let res: NewSessionResponse = self.conn.request("session/new", params).await?;
        *self.session_id.lock().await = Some(res.session_id.to_string());
        {
            let mut st = self.state.lock().await;
            if let Some(opts) = &res.config_options {
                st.config_options = opts
                    .iter()
                    .filter_map(|o| serde_json::to_value(o).ok())
                    .collect();
            }
            if let Some(modes) = &res.modes {
                st.modes = modes
                    .available_modes
                    .iter()
                    .filter_map(|m| serde_json::to_value(m).ok())
                    .collect();
                st.current_mode = Some(modes.current_mode_id.to_string());
            }
        }
        Ok(res)
    }

    /// Step 3. Runs a full prompt turn to completion.
    ///
    /// This does **not** return until the agent finishes. Updates arrive via
    /// the connection's notification callback while this is in flight, which
    /// is what lets the UI stream.
    pub async fn prompt(
        &self,
        session_id: &str,
        blocks: Vec<ContentBlock>,
    ) -> Result<PromptResponse, SessionError> {
        let req = PromptRequest::new(session_id.to_string(), blocks);
        Ok(self
            .conn
            .request(
                "session/prompt",
                serde_json::to_value(req).unwrap_or(json!({})),
            )
            .await?)
    }

    /// Cancel a turn, and answer every permission request it left pending.
    ///
    /// The pending answers are sent *after* the cancel notification, which
    /// is the order the spec describes: cancel first so the agent knows the
    /// turn is unwinding, then unblock it.
    pub async fn cancel(&self, session_id: &str) -> Result<(), SessionError> {
        self.conn
            .notify("session/cancel", json!({ "sessionId": session_id }))
            .await?;

        for id in self.pending_permissions.take_all().await {
            // `Cancelled` is not an error and not a rejection. Reporting
            // either would make the agent treat an explicit user cancel as a
            // protocol failure and possibly retry.
            self.conn
                .respond_ok(&id, json!({ "outcome": { "outcome": "cancelled" } }))
                .await?;
        }
        Ok(())
    }

    /// Set the session's permission mode, where the agent supports modes.
    pub async fn set_mode(&self, session_id: &str, mode_id: &str) -> Result<(), SessionError> {
        self.conn
            .request::<Value>(
                "session/set_mode",
                json!({ "sessionId": session_id, "modeId": mode_id }),
            )
            .await?;
        Ok(())
    }

    /// Set a config option — including the model, which ACP models this way.
    ///
    /// The wire field is `configId`, not `optionId`. Both were verified
    /// against the schema (`SetSessionConfigOptionRequest::config_id`) and
    /// against a real agent's request trace; sending `optionId` is silently
    /// wrong, so this is pinned by a test.
    ///
    /// Returns the agent's refreshed option list, which matters because a
    /// change usually moves `currentValue` and the client's copy is stale
    /// afterwards.
    pub async fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: Value,
    ) -> Result<SetConfigOptionOutcome, SessionError> {
        let res: ConfigOptionResponse = self
            .conn
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": session_id,
                    "configId": config_id,
                    "value": value,
                }),
            )
            .await?;
        let options: Vec<crate::events::SessionConfigView> = res
            .config_options
            .iter()
            .map(crate::events::SessionConfigView::from)
            .collect();
        // Keep our copy current so a later read does not report the old value.
        self.state.lock().await.config_options = options
            .iter()
            .filter_map(|o| serde_json::to_value(o).ok())
            .collect();
        Ok(SetConfigOptionOutcome { options })
    }

    /// How many permission requests the agent is currently blocked on.
    pub async fn pending_permission_count(&self) -> usize {
        self.pending_permissions.len().await
    }
}

/// What a config-option change reported back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetConfigOptionOutcome {
    /// The agent's refreshed option list.
    pub options: Vec<crate::events::SessionConfigView>,
}

/// The `session/set_config_option` response. Only the field we use is
/// declared; unknown fields are ignored by serde.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOptionResponse {
    #[serde(default)]
    config_options: Vec<agent_client_protocol::schema::v1::SessionConfigOption>,
}

/// The part of a finished turn a caller usually wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptOutcome {
    pub stop_reason: StopReason,
    pub ended_normally: bool,
    pub was_cancelled: bool,
}

impl PromptOutcome {
    /// The agent's stop reason as its wire name, for a UI that reports it
    /// verbatim. Derived from the enum rather than a stored string so it
    /// cannot drift from the value it came from.
    pub fn stop_reason_wire(&self) -> String {
        serde_json::to_value(self.stop_reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// Whether a turn ended because the agent stopped on its own, as opposed to
/// being cancelled or cut short.
///
/// Callers need this distinction: a cancelled turn should not be rendered as
/// a completed answer.
pub fn ended_normally(stop: &PromptResponse) -> bool {
    matches!(
        stop.stop_reason,
        StopReason::EndTurn | StopReason::MaxTokens | StopReason::Refusal
    )
}

/// Whether the turn was cancelled, which the UI reports differently from a
/// completed answer.
pub fn was_cancelled(stop: &PromptResponse) -> bool {
    matches!(stop.stop_reason, StopReason::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::AgentCallback;
    use serde_json::Value;
    use std::sync::Mutex as StdMutex;
    use tokio::io::AsyncBufReadExt;

    /// Records normalized events, so the test can assert the real path:
    /// agent frame → connection → host → event port.
    #[derive(Default)]
    struct Recorded {
        events: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::host::EventPort for Recorded {
        async fn emit(&self, e: crate::events::NormalizedEvent) {
            let tag = match &e.event {
                crate::events::MiraEvent::AssistantText { text, .. } => format!("text:{text}"),
                crate::events::MiraEvent::UserText { text, .. } => format!("user:{text}"),
                crate::events::MiraEvent::ToolCall(s) => format!("tool:{}", s.id),
                other => format!("other:{}", serde_json::to_string(other).unwrap_or_default()),
            };
            self.events.lock().unwrap().push(tag);
        }
    }

    /// Poll rather than sleeping a guessed interval; process and task
    /// startup under a loaded test binary is slower than a fixed sleep.
    async fn wait_until<F: Fn() -> bool>(pred: F, what: &str) {
        for _ in 0..200 {
            if pred() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    struct Rig {
        sess: Arc<AcpSession>,
        /// Reader over what we wrote to the agent.
        to_agent: tokio::io::BufReader<tokio::io::DuplexStream>,
        /// Write half the agent answers on.
        from_agent: tokio::io::DuplexStream,
        host: Arc<crate::host::AcpHost>,
        events: Arc<Recorded>,
    }

    async fn rig(caps: ClientCaps) -> Rig {
        let (to_agent_write, to_agent_read) = tokio::io::duplex(64 * 1024);
        let (agent_write, from_agent_read) = tokio::io::duplex(64 * 1024);
        let recorded = Arc::new(Recorded::default());
        let host = crate::host::AcpHost::new(
            Arc::new(crate::host::DenyAll),
            Arc::new(crate::host::DenyAll),
            Arc::new(crate::host::DenyAll),
            recorded.clone() as Arc<dyn crate::host::EventPort>,
        );
        let conn = crate::conn::Connection::spawn_transport(
            from_agent_read,
            to_agent_write,
            host.clone() as Arc<dyn AgentCallback>,
        );
        let sess = AcpSession::new(conn, caps, host.pending_permissions.clone());
        Rig {
            sess,
            // One long-lived reader, or a later read discards earlier frames.
            to_agent: tokio::io::BufReader::new(to_agent_read),
            from_agent: agent_write,
            host,
            events: recorded,
        }
    }

    impl Rig {
        async fn next_frame(&mut self) -> Value {
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.to_agent.read_line(&mut line),
            )
            .await
            .expect("agent never received a frame")
            .expect("read");
            serde_json::from_str(line.trim()).expect("non-JSON frame")
        }

        async fn reply(&mut self, id: Value, result: Value) {
            use tokio::io::AsyncWriteExt;
            let mut s = serde_json::to_string(&json!({
                "jsonrpc": "2.0", "id": id, "result": result
            }))
            .unwrap();
            s.push('\n');
            self.from_agent.write_all(s.as_bytes()).await.unwrap();
        }

        /// Send an unsolicited frame, as a streaming agent would.
        async fn say(&mut self, v: Value) {
            use tokio::io::AsyncWriteExt;
            let mut s = serde_json::to_string(&v).unwrap();
            s.push('\n');
            self.from_agent.write_all(s.as_bytes()).await.unwrap();
        }
    }

    fn init_ok() -> Value {
        json!({
            "protocolVersion": 1,
            "agentCapabilities": { "loadSession": true, "promptCapabilities": {} },
            "authMethods": [{ "id": "cursor_login", "name": "Sign in" }]
        })
    }

    #[test]
    fn mira_tools_go_over_http_or_through_the_stdio_bridge() {
        let m = MiraMcp { url: "http://127.0.0.1:1/mcp".into(), token: "tok".into() };
        let http = mira_mcp_entry(&m, true, None).unwrap();
        assert_eq!(http["type"], "http");
        assert_eq!(http["url"], m.url);
        assert_eq!(http["headers"], json!([{ "name": "Authorization", "value": "Bearer tok" }]));
        let stdio = mira_mcp_entry(&m, false, Some("/bin/mira".into())).unwrap();
        assert!(stdio.get("type").is_none(), "stdio entries are untagged");
        assert_eq!(stdio["command"], "/bin/mira");
        assert_eq!(stdio["args"], json!(["mcp-bridge", m.url]));
        assert_eq!(stdio["env"], json!([{ "name": MIRA_MCP_TOKEN_ENV, "value": "tok" }]));
        assert!(!stdio.to_string().contains("Bearer"), "no token in argv");
        assert!(mira_mcp_entry(&m, false, None).is_none());
    }

    #[tokio::test]
    async fn initialize_advertises_only_bound_capabilities() {
        // Claiming a capability we would then refuse makes the agent plan
        // around something that does not exist.
        let mut r = rig(ClientCaps {
            fs_read: true,
            fs_write: false,
            terminal: true,
            elicitation_form: true,
        })
        .await;
        let waiter = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let frame = r.next_frame().await;
        assert_eq!(frame["method"], "initialize");
        let caps = &frame["params"]["clientCapabilities"];
        assert_eq!(caps["fs"]["readTextFile"], true);
        assert_eq!(caps["fs"]["writeTextFile"], false);
        assert_eq!(caps["terminal"], true);
        assert_eq!(caps["elicitation"]["form"], json!({}));
        r.reply(frame["id"].clone(), init_ok()).await;
        waiter.await.unwrap().expect("initialize");
    }

    #[tokio::test]
    async fn a_version_mismatch_is_refused_rather_than_limped_along() {
        let mut r = rig(ClientCaps::default()).await;
        let waiter = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let frame = r.next_frame().await;
        let mut res = init_ok();
        res["protocolVersion"] = json!(2);
        r.reply(frame["id"].clone(), res).await;
        let err = waiter.await.unwrap().expect_err("should refuse");
        assert!(
            matches!(err, SessionError::VersionMismatch { got: 2, .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn an_agent_with_no_auth_methods_still_initializes() {
        // `claude-code-acp` authenticates through the `claude` CLI and
        // advertises nothing, so refusing here would lock out the very
        // agents this supports best.
        let mut r = rig(ClientCaps::default()).await;
        let waiter = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let frame = r.next_frame().await;
        let mut res = init_ok();
        res["authMethods"] = json!([]);
        r.reply(frame["id"].clone(), res).await;
        let out = waiter.await.unwrap().expect("should initialize");
        assert!(out.auth_methods.is_empty());
        assert_eq!(r.sess.is_initialized(), Some(true));
    }

    #[tokio::test]
    async fn a_session_cannot_be_created_before_initialize() {
        let r = rig(ClientCaps::default()).await;
        let err = r
            .sess
            .new_session(std::path::Path::new("/tmp"), vec![])
            .await
            .expect_err("should refuse");
        assert!(matches!(err, SessionError::NotInitialized), "got {err:?}");
    }

    #[tokio::test]
    async fn new_session_records_the_agents_session_id_and_options() {
        let mut r = rig(ClientCaps::default()).await;
        // initialize
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let f = r.next_frame().await;
        r.reply(f["id"].clone(), init_ok()).await;
        w.await.unwrap().expect("init");

        // session/new
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.new_session(std::path::Path::new("/repo"), vec![]).await }
        });
        let f = r.next_frame().await;
        assert_eq!(f["method"], "session/new");
        assert_eq!(f["params"]["cwd"], "/repo");
        r.reply(
            f["id"].clone(),
            json!({
                "sessionId": "agent_sess_7",
                "modes": {
                    "currentModeId": "ask",
                    "availableModes": [
                        { "id": "ask", "name": "Ask" },
                        { "id": "auto", "name": "Auto" }
                    ]
                },
                // The option's `kind` is flattened onto the object, so
                // `type`/`currentValue`/`options` are top-level siblings of
                // `id`, not nested under a `kind` key.
                "configOptions": [
                    {
                        "id": "model",
                        "name": "Model",
                        "category": "model",
                        "type": "select",
                        "currentValue": "grok-code",
                        "options": [
                            { "value": "grok-code", "name": "Grok Code" },
                            { "value": "gpt-5", "name": "GPT-5" }
                        ]
                    }
                ]
            }),
        )
        .await;
        let res = w.await.unwrap().expect("new_session");
        assert_eq!(res.session_id.to_string(), "agent_sess_7");
        assert_eq!(r.sess.session_id().as_deref(), Some("agent_sess_7"));
        // ACP has no set-model method; the model selector is a config option.
        assert_eq!(r.sess.config_options().len(), 1);
        assert_eq!(r.sess.config_options()[0]["category"], "model");
        assert_eq!(r.sess.config_options()[0]["currentValue"], "grok-code");
        assert_eq!(r.sess.config_options()[0]["type"], "select");
        assert_eq!(
            r.sess.config_options()[0]["options"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        // Modes arrive with `session/new`, not `initialize`.
        let (current, available) = r.sess.modes();
        assert_eq!(current.as_deref(), Some("ask"));
        assert_eq!(available.len(), 2);
    }

    #[tokio::test]
    async fn a_prompt_returns_the_agents_stop_reason() {
        let mut r = rig(ClientCaps::default()).await;
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let f = r.next_frame().await;
        r.reply(f["id"].clone(), init_ok()).await;
        w.await.unwrap().expect("init");

        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.prompt("agent_sess_7", vec![text("hi")]).await }
        });
        let f = r.next_frame().await;
        assert_eq!(f["method"], "session/prompt");
        r.reply(f["id"].clone(), json!({ "stopReason": "end_turn" }))
            .await;
        let res = w.await.unwrap().expect("prompt");
        assert_eq!(res.stop_reason, StopReason::EndTurn);
        assert!(ended_normally(&res));
        assert!(!was_cancelled(&res));
    }

    /// The spec MUST: a cancel must answer every pending permission request.
    #[tokio::test]
    async fn cancel_answers_every_pending_permission_request() {
        let mut r = rig(ClientCaps::default()).await;

        // Two permission requests arrive, tracked but not yet answered.
        let pending = r.host.pending_permissions.clone();
        pending.track(json!(11)).await;
        pending.track(json!(12)).await;
        assert_eq!(r.sess.pending_permission_count().await, 2);

        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.cancel("s1").await }
        });

        // The cancel notification comes first.
        let first = r.next_frame().await;
        assert_eq!(first["method"], "session/cancel");
        assert!(first.get("id").is_none(), "cancel is a notification");

        // Then each pending request gets a Cancelled outcome.
        let a = r.next_frame().await;
        let b = r.next_frame().await;
        let mut ids: Vec<i64> = vec![a["id"].as_i64().unwrap(), b["id"].as_i64().unwrap()];
        ids.sort();
        assert_eq!(ids, vec![11, 12]);
        for f in [a, b] {
            assert_eq!(f["result"]["outcome"]["outcome"], "cancelled");
        }
        w.await.unwrap().expect("cancel");
        assert_eq!(r.sess.pending_permission_count().await, 0);
    }

    #[tokio::test]
    async fn an_already_answered_permission_is_not_answered_twice() {
        // Answering a settled request would confuse an agent that has moved on.
        let mut r = rig(ClientCaps::default()).await;
        let pending = r.host.pending_permissions.clone();
        pending.track(json!(11)).await;
        pending.settle(&json!(11)).await;
        pending.track(json!(12)).await;

        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.cancel("s1").await }
        });
        let _ = r.next_frame().await;
        let only = r.next_frame().await;
        assert_eq!(only["id"], 12);
        w.await.unwrap().expect("cancel");
    }

    #[tokio::test]
    async fn session_updates_arrive_while_a_prompt_is_in_flight() {
        // The prompt call blocks until the turn ends, so updates have to
        // reach the callback concurrently for the UI to stream at all.
        let mut r = rig(ClientCaps::default()).await;
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.initialize().await }
        });
        let f = r.next_frame().await;
        r.reply(f["id"].clone(), init_ok()).await;
        w.await.unwrap().expect("init");

        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.prompt("s1", vec![text("hi")]).await }
        });
        let f = r.next_frame().await;
        // An update lands before the turn reports its stop reason.
        r.say(json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": { "sessionId": "s1",
                        "update": { "sessionUpdate": "agent_message_chunk",
                                    "content": { "type": "text", "text": "working" } } }
        }))
        .await;
        wait_until(
            || !r.events.events.lock().unwrap().is_empty(),
            "the update to reach the event port",
        )
        .await;
        let got = r.events.events.lock().unwrap().clone();
        assert_eq!(got[0], "text:working".to_string());
        r.reply(f["id"].clone(), json!({ "stopReason": "end_turn" }))
            .await;
        w.await.unwrap().expect("prompt");
    }

    #[tokio::test]
    async fn cancelling_with_nothing_pending_sends_only_the_notification() {
        let mut r = rig(ClientCaps::default()).await;
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.cancel("s1").await }
        });
        let f = r.next_frame().await;
        assert_eq!(f["method"], "session/cancel");
        w.await.unwrap().expect("cancel");
    }

    #[tokio::test]
    async fn set_config_option_is_how_a_model_is_chosen() {
        let mut r = rig(ClientCaps::default()).await;
        let w = tokio::spawn({
            let s = r.sess.clone();
            async move { s.set_config_option("s1", "model", json!("grok-code")).await }
        });
        let f = r.next_frame().await;
        assert_eq!(f["method"], "session/set_config_option");
        // `configId`, NOT `optionId`. The schema calls it
        // `SetSessionConfigOptionRequest::config_id`, and a real agent's
        // request trace shows the same. Sending `optionId` looks plausible
        // and is silently wrong, so it is pinned here.
        assert_eq!(
            f["params"]["configId"], "model",
            "the wire field is configId"
        );
        assert!(
            f["params"].get("optionId").is_none(),
            "optionId must not be sent: {f:?}"
        );
        assert_eq!(f["params"]["value"], "grok-code");
        r.reply(
            f["id"].clone(),
            json!({
                "configOptions": [
                    { "id": "model", "name": "Model", "category": "model",
                      "type": "select", "currentValue": "grok-code",
                      "options": [ { "value": "grok-code", "name": "Grok Code" } ] }
                ]
            }),
        )
        .await;
        let outcome = w.await.unwrap().expect("set_config_option");
        // The agent's refreshed list comes back, so the picker can show the
        // new current value rather than a stale one.
        assert_eq!(outcome.options.len(), 1);
        assert_eq!(outcome.options[0].current.as_deref(), Some("grok-code"));
        assert_eq!(r.sess.config_options().len(), 1);
    }

    /// A text content block, via the SDK's own constructor.
    fn text(t: &str) -> ContentBlock {
        ContentBlock::Text(agent_client_protocol::schema::v1::TextContent::new(t))
    }
}
