//! `mira acp`: Mira as an Agent Client Protocol agent over stdio.
//!
//! Editors that speak ACP (Zed, and others) start `mira acp` and talk
//! JSON-RPC over stdin/stdout, one message per line. Each editor thread
//! is a Mira session in the folder the editor names. Tool calls, diffs
//! and permission prompts show up in the editor's own UI.
//!
//! Stdout carries the protocol only; logs go to stderr.
//!
//! Protocol: <https://agentclientprotocol.com>.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use mira_core::ToolCall;
use mira_harness::{Approver, FileStore, HarnessEvent, Session, SessionConfig, SessionStore};
use mira_policy::{Decision, Mode, Policy, PolicyConfig};
use mira_tools::{builtin, Registry, ToolContext};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

use crate::config::MiraConfig;

const PROTOCOL_VERSION: u64 = 1;
/// Tool output shown in the editor is cut to this many characters.
const MAX_TOOL_TEXT: usize = 4000;
const MODES: [(&str, &str, &str); 5] = [
    ("plan", "Plan", "Read-only: explore and propose a plan."),
    ("manual", "Manual", "Ask before edits and commands."),
    ("auto", "Auto", "Run safe actions, ask for risky ones."),
    ("edit", "Edit", "Edit files freely, ask before commands."),
    ("yolo", "Yolo", "Run everything without asking."),
];

#[derive(clap::Args, Debug, Clone)]
pub struct AcpArgs {}

pub async fn run(cli: &crate::Cli, _args: AcpArgs) -> Result<()> {
    let factory: Arc<dyn SessionFactory> = match MiraFactory::build(cli).await {
        Ok(f) => Arc::new(f),
        // Stay up so the editor can show why, instead of a dead process.
        Err(e) => Arc::new(Broken(format!("{e:#}"))),
    };
    serve(tokio::io::stdin(), tokio::io::stdout(), factory).await
}

/* ---------- sessions ---------- */

/// Makes a Mira session for an editor thread.
#[async_trait]
pub trait SessionFactory: Send + Sync {
    async fn create(
        &self,
        cwd: PathBuf,
        approver: Arc<dyn Approver>,
    ) -> Result<(Session, Arc<Mutex<Policy>>)>;
}

struct Broken(String);

#[async_trait]
impl SessionFactory for Broken {
    async fn create(
        &self,
        _cwd: PathBuf,
        _approver: Arc<dyn Approver>,
    ) -> Result<(Session, Arc<Mutex<Policy>>)> {
        anyhow::bail!("{}", self.0)
    }
}

/// The real factory: provider, tools, MCP servers and plugins are built
/// once; each session gets its own policy, sandbox and memory.
struct MiraFactory {
    cfg: MiraConfig,
    settings: crate::ResolvedSettings,
    provider: Arc<dyn mira_ai::ChatProvider>,
    registry: Arc<Registry>,
    store: Option<Arc<dyn SessionStore>>,
    extensions: mira_server::extensions::Extensions,
}

impl MiraFactory {
    async fn build(cli: &crate::Cli) -> Result<Self> {
        crate::login::auto_refresh_if_needed().await;
        let cwd = std::env::current_dir().context("read cwd")?;
        let cfg = MiraConfig::load(&cwd).context("load config")?;
        mira_config::export_keys_to_env(&cfg);
        let settings = crate::resolve_settings(cli, &cfg)?;
        let provider = mira_ai::build_chat_provider(
            &settings.provider_name,
            settings.base_url.clone(),
            settings.api_key.clone(),
            settings.extra_headers.clone(),
            settings.prompt_caching,
        )
        .context("build provider")?;

        let mut registry = Registry::new();
        builtin::register_core(&mut registry);
        if cfg.memory.tools_enabled() {
            builtin::register_memory(&mut registry);
        }
        let extensions = mira_server::extensions::Extensions::new(Some(cwd.clone()));
        let skills =
            mira_server::extensions::load_skills(Some(&cwd), &extensions.plugin_skill_dirs());
        let skills: builtin::skill::SkillHandle =
            Arc::new(tokio::sync::RwLock::new(Arc::new(skills)));
        builtin::register_skills(&mut registry, skills.clone());
        extensions.attach_skills(skills);
        extensions.reload().await;
        registry.add_source(extensions.mcp().tool_source());

        let store: Option<Arc<dyn SessionStore>> = match FileStore::open_default() {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                tracing::warn!(%e, "acp: persistence disabled");
                None
            }
        };
        let agents = Arc::new(mira_agents::load_with_plugins(
            &cwd,
            &extensions.plugin_agent_files(),
        ));
        let base = Arc::new(registry.clone());
        let mut agent_tool = mira_server::interactive::AgentTool::new(
            provider.clone(),
            base,
            settings.model.clone(),
        )
        .with_agents(agents);
        if let Some(s) = &store {
            agent_tool = agent_tool.with_store(s.clone());
        }
        registry.register(agent_tool);

        Ok(Self {
            cfg,
            settings,
            provider,
            registry: Arc::new(registry),
            store,
            extensions,
        })
    }
}

#[async_trait]
impl SessionFactory for MiraFactory {
    async fn create(
        &self,
        cwd: PathBuf,
        approver: Arc<dyn Approver>,
    ) -> Result<(Session, Arc<Mutex<Policy>>)> {
        let policy = Policy::from_config(&PolicyConfig {
            mode: self.settings.mode,
            allow: self.cfg.permissions.allow.clone(),
            ask: self.cfg.permissions.ask.clone(),
            deny: self.cfg.permissions.deny.clone(),
        })
        .context("compile policy")?;
        let policy = Arc::new(Mutex::new(policy));

        let memory: Arc<dyn mira_memory::MemoryStore> =
            Arc::new(mira_memory::FileMemoryStore::new(
                mira_config::user_memory_path(),
                mira_config::project_memory_path(&cwd),
            ));
        let episodic: Arc<dyn mira_memory::EpisodicStore> = Arc::new(
            mira_memory::FileEpisodicStore::new(mira_memory::project_episodic_path(&cwd)),
        );
        let sandbox = Arc::new(mira_sandbox::Sandbox::for_workspace(&cwd));
        let ctx = ToolContext::new(cwd.clone(), sandbox)
            .with_memory(memory)
            .with_episodic(episodic.clone());

        let mut cfg = SessionConfig::new(self.settings.model.clone());
        cfg.max_tokens = self.settings.max_tokens;
        cfg.temperature = self.settings.temperature;
        cfg.compactor_model = self.settings.compactor_model.clone();
        let prompt = mira_server::system_prompt(&cwd, &self.registry).replacen(
            "You are Mira, an interactive coding agent.",
            "You are Mira, an interactive coding agent running inside the user's editor.",
            1,
        );
        let mut session = Session::new(
            cfg,
            prompt,
            self.provider.clone(),
            self.registry.clone(),
            policy.clone(),
            approver,
            ctx,
        )
        .with_hooks(self.extensions.hook_runner());
        if let Some(store) = &self.store {
            session = session.with_store(store.clone());
        }
        if self.cfg.memory.inject_context() {
            session = session
                .with_memory_snapshot(mira_server::make_memory_snapshot_with(&cwd, episodic))
                .with_memory_retrieval(mira_server::memory_retrieval_from(&self.cfg.memory));
        }
        Ok((session, policy))
    }
}

/* ---------- JSON-RPC connection ---------- */

type Reply = std::result::Result<Value, Value>;

struct Conn {
    out: mpsc::UnboundedSender<String>,
    next_id: AtomicI64,
    pending: StdMutex<HashMap<i64, oneshot::Sender<Reply>>>,
}

impl Conn {
    fn send(&self, msg: Value) {
        let _ = self.out.send(msg.to_string());
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn respond(&self, id: Value, reply: Reply) {
        match reply {
            Ok(result) => self.send(json!({"jsonrpc": "2.0", "id": id, "result": result})),
            Err(error) => self.send(json!({"jsonrpc": "2.0", "id": id, "error": error})),
        }
    }

    /// Ask the editor something and wait for its answer.
    async fn request(&self, method: &str, params: Value) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        rx.await
            .unwrap_or_else(|_| Err(rpc_error(-32603, "connection closed")))
    }

    fn resolve(&self, msg: &Value) {
        let Some(id) = msg.get("id").and_then(Value::as_i64) else {
            return;
        };
        let Some(tx) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let reply = match msg.get("error") {
            Some(e) => Err(e.clone()),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = tx.send(reply);
    }
}

fn rpc_error(code: i64, message: impl Into<String>) -> Value {
    json!({"code": code, "message": message.into()})
}

/* ---------- agent ---------- */

struct AcpSession {
    session: Session,
    policy: Arc<Mutex<Policy>>,
    cwd: PathBuf,
    /// Set while a prompt runs; `session/cancel` flips it.
    cancel: StdMutex<Option<watch::Sender<bool>>>,
}

struct Agent {
    conn: Arc<Conn>,
    factory: Arc<dyn SessionFactory>,
    sessions: Mutex<HashMap<String, Arc<AcpSession>>>,
}

/// Serve ACP on `input`/`output` until the editor closes `input`.
pub async fn serve<R, W>(input: R, output: W, factory: Arc<dyn SessionFactory>) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(line) = rx.recv().await {
            if output.write_all(line.as_bytes()).await.is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });

    let agent = Arc::new(Agent {
        conn: Arc::new(Conn {
            out: tx,
            next_id: AtomicI64::new(0),
            pending: StdMutex::new(HashMap::new()),
        }),
        factory,
        sessions: Mutex::new(HashMap::new()),
    });

    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                agent.conn.respond(
                    Value::Null,
                    Err(rpc_error(-32700, format!("parse error: {e}"))),
                );
                continue;
            }
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, msg.get("id").cloned()) {
            (Some(method), Some(id)) => {
                let agent = agent.clone();
                tokio::spawn(async move {
                    let reply = agent.request(&method, params).await;
                    agent.conn.respond(id, reply);
                });
            }
            (Some(method), None) => agent.notification(&method, params).await,
            (None, Some(_)) => agent.conn.resolve(&msg),
            (None, None) => {}
        }
    }

    // The editor is gone: stop running turns.
    for s in agent.sessions.lock().await.values() {
        if let Some(c) = s.cancel.lock().unwrap().take() {
            let _ = c.send(true);
        }
    }
    drop(agent);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
    Ok(())
}

impl Agent {
    async fn request(&self, method: &str, params: Value) -> Reply {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": {
                        "image": false,
                        "audio": false,
                        "embeddedContext": true,
                    },
                },
                "authMethods": [],
                "agentInfo": {
                    "name": "mira",
                    "title": "Mira",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            })),
            "authenticate" => Ok(json!({})),
            "session/new" => self.new_session(params).await,
            "session/prompt" => self.prompt(params).await,
            "session/set_mode" => self.set_mode(params).await,
            _ => Err(rpc_error(-32601, format!("method not found: {method}"))),
        }
    }

    async fn notification(&self, method: &str, params: Value) {
        if method == "session/cancel" {
            if let Some(s) = self.session(&params).await {
                if let Some(c) = s.cancel.lock().unwrap().as_ref() {
                    let _ = c.send(true);
                }
            }
        }
    }

    async fn session(&self, params: &Value) -> Option<Arc<AcpSession>> {
        let id = params.get("sessionId")?.as_str()?;
        self.sessions.lock().await.get(id).cloned()
    }

    async fn new_session(&self, params: Value) -> Reply {
        let cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| rpc_error(-32602, "cwd must be an absolute path"))?;
        let session_id = Arc::new(OnceLock::new());
        let approver = Arc::new(AcpApprover {
            conn: self.conn.clone(),
            session_id: session_id.clone(),
            cwd: cwd.clone(),
        });
        let (session, policy) = self
            .factory
            .create(cwd.clone(), approver)
            .await
            .map_err(|e| rpc_error(-32603, format!("{e:#}")))?;
        let id = session.id.to_string();
        let _ = session_id.set(id.clone());
        let mode = policy.lock().await.mode();
        self.sessions.lock().await.insert(
            id.clone(),
            Arc::new(AcpSession {
                session,
                policy,
                cwd,
                cancel: StdMutex::new(None),
            }),
        );
        Ok(json!({
            "sessionId": id,
            "modes": {
                "currentModeId": mode_id(mode),
                "availableModes": MODES
                    .iter()
                    .map(|(id, name, description)| json!({
                        "id": id, "name": name, "description": description,
                    }))
                    .collect::<Vec<_>>(),
            },
        }))
    }

    async fn set_mode(&self, params: Value) -> Reply {
        let s = self
            .session(&params)
            .await
            .ok_or_else(|| rpc_error(-32602, "unknown session"))?;
        let mode = params
            .get("modeId")
            .and_then(Value::as_str)
            .and_then(|m| crate::parse_mode(m).ok())
            .ok_or_else(|| rpc_error(-32602, "unknown mode"))?;
        s.policy.lock().await.set_mode(mode);
        Ok(json!({}))
    }

    async fn prompt(&self, params: Value) -> Reply {
        let s = self
            .session(&params)
            .await
            .ok_or_else(|| rpc_error(-32602, "unknown session"))?;
        let text = prompt_text(params.get("prompt").unwrap_or(&Value::Null));
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        if let Some(prev) = s.cancel.lock().unwrap().replace(cancel_tx) {
            let _ = prev.send(true);
        }

        let sid = s.session.id.to_string();
        let mut edits: HashMap<String, (PathBuf, Option<String>)> = HashMap::new();
        let mut stream = s.session.send(text).await;
        let cancelled = loop {
            tokio::select! {
                changed = cancel_rx.changed() => {
                    if changed.is_err() || *cancel_rx.borrow() {
                        break true;
                    }
                }
                event = stream.next() => match event {
                    None | Some(HarnessEvent::Done) => break false,
                    Some(e) => self.forward(&sid, &s.cwd, e, &mut edits),
                },
            }
        };
        drop(stream);
        s.cancel.lock().unwrap().take();
        Ok(json!({"stopReason": if cancelled { "cancelled" } else { "end_turn" }}))
    }

    fn update(&self, sid: &str, update: Value) {
        self.conn.notify(
            "session/update",
            json!({"sessionId": sid, "update": update}),
        );
    }

    /// Turn one harness event into editor updates.
    fn forward(
        &self,
        sid: &str,
        cwd: &Path,
        event: HarnessEvent,
        edits: &mut HashMap<String, (PathBuf, Option<String>)>,
    ) {
        match event {
            HarnessEvent::Token(text) => self.update(
                sid,
                json!({"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": text}}),
            ),
            HarnessEvent::Warning(text) => self.update(
                sid,
                json!({"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": format!("\n\n> ⚠ {text}\n\n")}}),
            ),
            HarnessEvent::ToolStart(call) => {
                let mut update = tool_call_json(&call, cwd);
                update["sessionUpdate"] = json!("tool_call");
                update["status"] = json!("in_progress");
                self.update(sid, update);
            }
            HarnessEvent::ToolPreview { call_id, preview } => {
                // Remember the file as it is now; the diff is sent once
                // the edit has landed.
                let path = absolute(cwd, &preview.path);
                let before = std::fs::read_to_string(&path).ok();
                edits.insert(call_id, (path, before));
            }
            HarnessEvent::ToolEnd(result) => {
                let mut content = Vec::new();
                if let Some((path, before)) = edits.remove(result.call_id.as_str()) {
                    if !result.is_error {
                        if let Ok(after) = std::fs::read_to_string(&path) {
                            content.push(json!({
                                "type": "diff",
                                "path": path,
                                "oldText": before,
                                "newText": after,
                            }));
                        }
                    }
                }
                if content.is_empty() && !result.content.is_empty() {
                    content.push(json!({
                        "type": "content",
                        "content": {"type": "text", "text": clip(&result.content)},
                    }));
                }
                self.update(
                    sid,
                    json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": result.call_id.as_str(),
                        "status": if result.is_error { "failed" } else { "completed" },
                        "content": content,
                    }),
                );
            }
            _ => {}
        }
    }
}

/* ---------- permissions ---------- */

struct AcpApprover {
    conn: Arc<Conn>,
    session_id: Arc<OnceLock<String>>,
    cwd: PathBuf,
}

#[async_trait]
impl Approver for AcpApprover {
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool {
        match decision {
            Decision::Allow => return true,
            Decision::Deny => return false,
            Decision::Ask => {}
        }
        let Some(sid) = self.session_id.get() else {
            return false;
        };
        let mut tool_call = tool_call_json(call, &self.cwd);
        tool_call["status"] = json!("pending");
        let reply = self
            .conn
            .request(
                "session/request_permission",
                json!({
                    "sessionId": sid,
                    "toolCall": tool_call,
                    "options": [
                        {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
                    ],
                }),
            )
            .await;
        match reply {
            Ok(v) => v["outcome"]["outcome"] == "selected" && v["outcome"]["optionId"] == "allow",
            Err(_) => false,
        }
    }
}

/* ---------- mapping helpers ---------- */

/// The fields ACP's `tool_call` and permission requests share.
fn tool_call_json(call: &ToolCall, cwd: &Path) -> Value {
    let name = call.function.name.as_str();
    let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);
    let arg = |k: &str| args.get(k).and_then(Value::as_str);
    let detail = ["command", "path", "pattern", "query", "url", "description"]
        .iter()
        .find_map(|k| arg(k));
    let title = match detail {
        Some(d) => format!("{name}: {}", one_line(d)),
        None => name.to_owned(),
    };
    let mut locations = Vec::new();
    if let Some(path) = arg("path") {
        let mut loc = json!({"path": absolute(cwd, path)});
        if let Some(line) = args.get("line").and_then(Value::as_u64) {
            loc["line"] = json!(line);
        }
        locations.push(loc);
    }
    json!({
        "toolCallId": call.id.as_str(),
        "title": title,
        "kind": tool_kind(name),
        "rawInput": args,
        "locations": locations,
    })
}

fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "file_outline" | "memory_read" | "read_output" => "read",
        "grep" | "glob" | "ast_grep" | "find_symbol" | "find_references" | "find_callers"
        | "memory_search" | "web_search" => "search",
        "edit_file" | "write_file" | "apply_patch" | "memory_edit" | "memory_append" => "edit",
        "bash" | "run_background" | "kill_background" | "verify" | "rustfmt" | "commit" => {
            "execute"
        }
        "web_fetch" | "browser" => "fetch",
        _ => "other",
    }
}

fn mode_id(mode: Mode) -> &'static str {
    match mode {
        Mode::Plan => "plan",
        Mode::Manual => "manual",
        Mode::Auto => "auto",
        Mode::Edit => "edit",
        Mode::Yolo => "yolo",
    }
}

/// Flatten ACP content blocks into the user's message.
fn prompt_text(blocks: &Value) -> String {
    let mut out = String::new();
    for block in blocks.as_array().into_iter().flatten() {
        let piece = match block.get("type").and_then(Value::as_str) {
            Some("text") => block["text"].as_str().unwrap_or_default().to_owned(),
            Some("resource_link") => format!("@{}", uri_path(block["uri"].as_str().unwrap_or(""))),
            Some("resource") => {
                let r = &block["resource"];
                let uri = uri_path(r["uri"].as_str().unwrap_or(""));
                match r["text"].as_str() {
                    Some(text) => format!("\n<file path=\"{uri}\">\n{text}\n</file>\n"),
                    None => format!("@{uri}"),
                }
            }
            _ => continue,
        };
        if !out.is_empty() && !out.ends_with(char::is_whitespace) {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    out
}

fn uri_path(uri: &str) -> &str {
    uri.strip_prefix("file://").unwrap_or(uri)
}

fn absolute(cwd: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

fn one_line(s: &str) -> String {
    let line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(80).collect::<String>())
    } else {
        line
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TOOL_TEXT {
        return s.to_owned();
    }
    format!("{}\n…", s.chars().take(MAX_TOOL_TEXT).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::{self, BoxStream};
    use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    /// Round 1 writes a file; round 2 replies.
    struct Scripted {
        rounds: StdMutex<usize>,
    }

    #[async_trait]
    impl ChatProvider for Scripted {
        async fn stream(
            &self,
            _request: ChatRequest,
        ) -> std::result::Result<
            BoxStream<'static, std::result::Result<ChatEvent, ProviderError>>,
            ProviderError,
        > {
            let n = {
                let mut r = self.rounds.lock().unwrap();
                *r += 1;
                *r
            };
            let events = if n == 1 {
                vec![
                    ChatEvent::ToolCalls(vec![ToolCall {
                        id: "c1".into(),
                        kind: ToolCallKind::Function,
                        function: ToolCallFunction {
                            name: "write_file".into(),
                            arguments: json!({"path": "hello.txt", "content": "hi\n"}).to_string(),
                        },
                    }]),
                    ChatEvent::Done(FinishReason::ToolCalls),
                ]
            } else {
                vec![
                    ChatEvent::TextDelta("wrote it".into()),
                    ChatEvent::Done(FinishReason::Stop),
                ]
            };
            Ok(stream::iter(events.into_iter().map(Ok)).boxed())
        }
    }

    struct TestFactory;

    #[async_trait]
    impl SessionFactory for TestFactory {
        async fn create(
            &self,
            cwd: PathBuf,
            approver: Arc<dyn Approver>,
        ) -> Result<(Session, Arc<Mutex<Policy>>)> {
            let mut registry = Registry::new();
            builtin::register_core(&mut registry);
            let policy = Arc::new(Mutex::new(Policy::from_config(&PolicyConfig {
                mode: Mode::Manual,
                ..Default::default()
            })?));
            let ctx = ToolContext::new(
                cwd.clone(),
                Arc::new(mira_sandbox::Sandbox::for_workspace(&cwd)),
            );
            let session = Session::new(
                SessionConfig::new("test-model"),
                "sys",
                Arc::new(Scripted {
                    rounds: StdMutex::new(0),
                }),
                Arc::new(registry),
                policy.clone(),
                approver,
                ctx,
            );
            Ok((session, policy))
        }
    }

    async fn write<W: AsyncWriteExt + Unpin>(w: &mut W, msg: Value) {
        w.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn a_prompt_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        let (client, agent_io) = tokio::io::duplex(1 << 16);
        let (agent_read, agent_write) = tokio::io::split(agent_io);
        tokio::spawn(serve(agent_read, agent_write, Arc::new(TestFactory)));
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut lines = BufReader::new(client_read).lines();

        write(
            &mut client_write,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}),
        )
        .await;
        let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(init["result"]["protocolVersion"], 1);

        write(&mut client_write, json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd": cwd, "mcpServers": []}})).await;
        let new: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let sid = new["result"]["sessionId"].as_str().unwrap().to_owned();
        assert_eq!(new["result"]["modes"]["currentModeId"], "manual");

        write(
            &mut client_write,
            json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
                "sessionId": sid,
                "prompt": [{"type":"text","text":"make hello.txt"}],
            }}),
        )
        .await;

        let mut updates = Vec::new();
        let stop = loop {
            let msg: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            if msg["method"] == "session/request_permission" {
                // Manual mode asks before writing; allow it.
                assert_eq!(msg["params"]["toolCall"]["kind"], "edit");
                write(
                    &mut client_write,
                    json!({"jsonrpc":"2.0","id": msg["id"],
                    "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}}),
                )
                .await;
            } else if msg["method"] == "session/update" {
                updates.push(msg["params"]["update"].clone());
            } else if msg["id"] == 3 {
                break msg["result"]["stopReason"].clone();
            }
        };
        assert_eq!(stop, "end_turn");
        assert_eq!(
            std::fs::read_to_string(cwd.join("hello.txt")).unwrap(),
            "hi\n"
        );

        let kinds: Vec<&str> = updates
            .iter()
            .filter_map(|u| u["sessionUpdate"].as_str())
            .collect();
        assert!(kinds.contains(&"tool_call"), "{kinds:?}");
        let done = updates
            .iter()
            .find(|u| u["sessionUpdate"] == "tool_call_update")
            .unwrap();
        assert_eq!(done["status"], "completed", "{done}");
        let text: String = updates
            .iter()
            .filter(|u| u["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|u| u["content"]["text"].as_str())
            .collect();
        assert!(text.contains("wrote it"), "{text}");
    }

    #[test]
    fn prompt_blocks_flatten() {
        let text = prompt_text(&json!([
            {"type": "text", "text": "look at"},
            {"type": "resource_link", "uri": "file:///repo/src/main.rs", "name": "main.rs"},
            {"type": "resource", "resource": {"uri": "file:///repo/a.txt", "text": "abc"}},
        ]));
        assert!(text.starts_with("look at @/repo/src/main.rs"));
        assert!(text.contains("<file path=\"/repo/a.txt\">\nabc\n</file>"));
    }
}
