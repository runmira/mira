//! The live set of MCP servers.
//!
//! [`McpManager::apply`] takes the servers that should exist and brings
//! the running set in line: new servers connect in the background (in
//! parallel, each with a timeout), removed ones disconnect, edited ones
//! reconnect, and untouched ones keep running. Nothing blocks startup,
//! and nothing needs a restart. Status changes are broadcast so UIs can
//! refresh, and tools reach sessions through [`McpManager::tool_source`].

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use mira_config::McpServerConfig;
use mira_tools::{Tool, ToolSource};
// Roots and logging are deprecated in the newest protocol revision but
// still what most servers in the wild use (e.g. the filesystem server).
#[allow(deprecated)]
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, GetPromptRequestParams,
    Implementation, ListRootsResult, LoggingMessageNotificationParam, Prompt,
    ReadResourceRequestParams, Resource, ResourceContents, Root,
};
use rmcp::service::{NotificationContext, RequestContext, RoleClient, RunningService};
use rmcp::{ClientHandler, ErrorData as McpError, Peer, ServiceExt};
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::auth::{self, PendingSignIn};
use crate::spec::{expand, missing_vars, Scope, ServerSpec, Transport};
use crate::state::{Approval, McpState};
use crate::tool::{self, McpTool};
use crate::vars;

/// Tunables. [`McpOptions::from_env`] reads the same variables Claude
/// Code does.
#[derive(Clone, Debug)]
pub struct McpOptions {
    /// How long a server gets to start and answer `initialize`.
    pub connect_timeout: Duration,
    /// How long one tool call may run.
    pub tool_timeout: Duration,
    /// Characters of tool output kept per call.
    pub max_output_chars: usize,
    pub credentials: PathBuf,
    pub state_file: PathBuf,
    /// Each stdio server's stderr goes to `<log_dir>/<server>.log`.
    pub log_dir: PathBuf,
}

impl McpOptions {
    /// `MCP_TIMEOUT` (ms), `MCP_TOOL_TIMEOUT` (ms), `MAX_MCP_OUTPUT_TOKENS`.
    pub fn from_env() -> Self {
        let ms = |var: &str, default: u64| {
            std::env::var(var)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_millis(default))
        };
        let tokens = std::env::var("MAX_MCP_OUTPUT_TOKENS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(25_000);
        Self {
            connect_timeout: ms("MCP_TIMEOUT", 30_000),
            tool_timeout: ms("MCP_TOOL_TIMEOUT", 600_000),
            max_output_chars: tokens.saturating_mul(4),
            credentials: crate::paths::credentials_file(),
            state_file: crate::paths::state_file(),
            log_dir: crate::paths::log_dir(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Connecting,
    Connected,
    /// A remote server answered 401: sign in to use it.
    NeedsAuth,
    /// A project server the user hasn't approved yet.
    NeedsApproval,
    /// A project server the user rejected.
    Rejected,
    Disabled,
    /// The definition uses `${VAR}`s that have no value: set them (in
    /// your environment, or saved with [`McpManager::set_variable`]).
    NeedsSetup {
        variables: Vec<String>,
    },
    Failed {
        message: String,
    },
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::Connecting => "connecting".into(),
            Status::Connected => "connected".into(),
            Status::NeedsAuth => "needs sign-in".into(),
            Status::NeedsApproval => "needs approval".into(),
            Status::Rejected => "rejected".into(),
            Status::Disabled => "disabled".into(),
            Status::NeedsSetup { variables } => format!("needs {}", variables.join(", ")),
            Status::Failed { message } => format!("failed: {message}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolView {
    /// Name the model sees.
    pub name: String,
    /// Name on the server.
    pub remote_name: String,
    pub description: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromptView {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Vec<PromptArgView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromptArgView {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceView {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// How long starting a sign-in (discovery + registration) may take.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything a UI shows about one server.
#[derive(Clone, Debug, Serialize)]
pub struct ServerView {
    pub name: String,
    pub scope: Scope,
    pub transport: &'static str,
    /// Command line or URL.
    pub target: String,
    pub source: Option<String>,
    pub status: Status,
    pub tools: Vec<ToolView>,
    pub resources: Vec<ResourceView>,
    pub prompts: Vec<PromptView>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub instructions: Option<String>,
    /// Remote servers can use OAuth.
    pub can_sign_in: bool,
    pub signed_in: bool,
    /// `${VAR}`s in the definition with no value. For a remote server's
    /// headers these are optional when OAuth sign-in works.
    pub missing_vars: Vec<String>,
    pub log_path: Option<String>,
    /// The definition, for editing.
    pub config: McpServerConfig,
}

#[derive(Clone, Debug)]
pub enum McpEvent {
    /// A server's status, tools or definition changed.
    Changed(String),
    /// A server was removed.
    Removed(String),
    /// A sign-in finished (successfully or not).
    SignIn {
        server: String,
        error: Option<String>,
    },
}

struct Entry {
    spec: ServerSpec,
    status: Status,
    /// Bumped on every (re)connect so late results from an older attempt
    /// are ignored.
    generation: u64,
    peer: Option<Peer<RoleClient>>,
    stop: Option<CancellationToken>,
    tools: Vec<Arc<McpTool>>,
    resources: Vec<Resource>,
    prompts: Vec<Prompt>,
    server_info: Option<Implementation>,
    instructions: Option<String>,
    /// The last `WWW-Authenticate` challenge, for sign-in.
    challenge: Option<String>,
    signed_in: bool,
    /// Unexpected disconnects in a row, for backoff.
    drops: u32,
}

impl Entry {
    fn new(spec: ServerSpec, status: Status) -> Self {
        Self {
            spec,
            status,
            generation: 0,
            peer: None,
            stop: None,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
            server_info: None,
            instructions: None,
            challenge: None,
            signed_in: false,
            drops: 0,
        }
    }

    fn disconnect(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.cancel();
        }
        self.peer = None;
        self.tools.clear();
        self.resources.clear();
        self.prompts.clear();
    }
}

/// Handle to the MCP servers. Cheap to clone.
#[derive(Clone)]
pub struct McpManager {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    opts: McpOptions,
    servers: RwLock<BTreeMap<String, Entry>>,
    events: broadcast::Sender<McpEvent>,
    pending: tokio::sync::Mutex<HashMap<String, PendingSignIn>>,
    project: RwLock<Option<PathBuf>>,
    me: Weak<Inner>,
}

/// Unexpected disconnects retried before giving up.
const MAX_RECONNECTS: u32 = 5;

impl McpManager {
    pub fn new(opts: McpOptions) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            inner: Arc::new_cyclic(|me| Inner {
                opts,
                servers: RwLock::new(BTreeMap::new()),
                events,
                pending: tokio::sync::Mutex::new(HashMap::new()),
                project: RwLock::new(None),
                me: me.clone(),
            }),
        }
    }

    pub fn options(&self) -> &McpOptions {
        &self.inner.opts
    }

    pub fn subscribe(&self) -> broadcast::Receiver<McpEvent> {
        self.inner.events.subscribe()
    }

    /// Live tools for a [`mira_tools::Registry`].
    pub fn tool_source(&self) -> Arc<dyn ToolSource> {
        Arc::new(Source {
            inner: Arc::downgrade(&self.inner),
        })
    }

    pub fn project(&self) -> Option<PathBuf> {
        self.inner.project.read().unwrap().clone()
    }

    /// Make the running set match `specs`, for `project`.
    pub fn apply(&self, specs: Vec<ServerSpec>, project: Option<PathBuf>) {
        self.inner.apply(specs, project);
    }

    pub fn servers(&self) -> Vec<ServerView> {
        let servers = self.inner.servers.read().unwrap();
        servers.values().map(|e| self.inner.view(e)).collect()
    }

    pub fn server(&self, name: &str) -> Option<ServerView> {
        let servers = self.inner.servers.read().unwrap();
        servers.get(name).map(|e| self.inner.view(e))
    }

    /// Save a value for `${name}` (or with `None`, forget it) and
    /// reconnect the servers that use it.
    pub fn set_variable(&self, name: &str, value: Option<&str>) -> Result<(), String> {
        vars::set(&vars::path_for(&self.inner.opts.credentials), name, value)
            .map_err(|e| format!("{e:#}"))?;
        let mut servers = self.inner.servers.write().unwrap();
        for entry in servers.values_mut() {
            let uses = serde_json::to_string(&entry.spec.transport)
                .unwrap_or_default()
                .contains(&format!("${{{name}"));
            let gated = matches!(
                entry.status,
                Status::Disabled | Status::NeedsApproval | Status::Rejected
            );
            if uses && !gated {
                entry.drops = 0;
                self.inner.start_connect(entry);
            }
        }
        Ok(())
    }

    /// Names of saved variables (values stay private).
    pub fn saved_variables(&self) -> Vec<String> {
        self.inner.vars().into_keys().collect()
    }

    pub fn reconnect(&self, name: &str) -> Result<(), String> {
        let mut servers = self.inner.servers.write().unwrap();
        let entry = servers
            .get_mut(name)
            .ok_or_else(|| format!("no MCP server `{name}`"))?;
        match entry.status {
            Status::Disabled | Status::NeedsApproval | Status::Rejected => {
                return Err(format!("`{name}` is {}", entry.status.label()))
            }
            _ => {}
        }
        entry.drops = 0;
        self.inner.start_connect(entry);
        Ok(())
    }

    /// Turn a server on or off (remembered in `state.json`).
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        let spec = self.spec(name)?;
        let project = self.project();
        self.inner
            .update_state(|st| st.set_disabled(&spec, project.as_deref(), !enabled))?;
        self.inner.reapply();
        Ok(())
    }

    /// Approve or reject a project server.
    pub fn set_approved(&self, name: &str, approve: bool) -> Result<(), String> {
        let spec = self.spec(name)?;
        let project = self
            .project()
            .ok_or_else(|| "no project is open".to_owned())?;
        self.inner
            .update_state(|st| st.set_approval(&spec, &project, approve))?;
        self.inner.reapply();
        Ok(())
    }

    fn spec(&self, name: &str) -> Result<ServerSpec, String> {
        self.inner
            .servers
            .read()
            .unwrap()
            .get(name)
            .map(|e| e.spec.clone())
            .ok_or_else(|| format!("no MCP server `{name}`"))
    }

    /// Wait until no server is still connecting, or `timeout` passes.
    /// For one-shot runs that want every tool before the first turn.
    pub async fn wait_settled(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let busy = self
                .inner
                .servers
                .read()
                .unwrap()
                .values()
                .any(|e| e.status == Status::Connecting);
            if !busy || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Start signing in to a remote server. Returns the URL to open; the
    /// browser comes back to `redirect_uri`, which must hand the full
    /// callback URL to [`Self::finish_sign_in`].
    pub async fn begin_sign_in(&self, name: &str, redirect_uri: &str) -> Result<String, String> {
        let (spec, challenge) = {
            let servers = self.inner.servers.read().unwrap();
            let e = servers
                .get(name)
                .ok_or_else(|| format!("no MCP server `{name}`"))?;
            (e.spec.clone(), e.challenge.clone())
        };
        let saved = self.inner.vars();
        let missing = missing_in(&spec, &saved);
        let transport = expand_transport(&spec, &saved)
            .map_err(|vars| format!("set {} first", vars.join(", ")))?;
        // Discovery and registration are plain HTTP calls with no timeout
        // of their own: a provider that never answers mustn't leave the
        // Sign in button spinning.
        let pending = tokio::time::timeout(
            SIGN_IN_TIMEOUT,
            auth::begin(
                &self.inner.opts.credentials,
                name,
                &transport,
                challenge.as_deref(),
                redirect_uri,
            ),
        )
        .await
        .map_err(|_| {
            format!(
                "`{name}`'s sign-in server didn't answer within {}s; try again",
                SIGN_IN_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| sign_in_error(name, &e, &missing))?;
        let url = pending.auth_url.clone();
        self.inner
            .pending
            .lock()
            .await
            .insert(pending.state.clone(), pending);
        Ok(url)
    }

    /// Finish a sign-in from the redirect URL (with `code` and `state`).
    /// Returns the server's name; it reconnects with the new token.
    pub async fn finish_sign_in(&self, callback_url: &str) -> Result<String, String> {
        let parsed = url::Url::parse(callback_url).map_err(|e| e.to_string())?;
        let param = |k: &str| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        let state = param("state").ok_or("the sign-in callback has no `state`")?;
        let pending = self
            .inner
            .pending
            .lock()
            .await
            .remove(&state)
            .ok_or("this sign-in expired or was already used; start it again")?;
        let server = pending.server.clone();
        let result = match param("error") {
            Some(err) => Err(param("error_description").unwrap_or(err)),
            None => pending
                .session
                .handle_callback_url(callback_url)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
        };
        let _ = self.inner.events.send(McpEvent::SignIn {
            server: server.clone(),
            error: result.as_ref().err().cloned(),
        });
        result?;
        debug!(server, "mcp: signed in");
        let _ = self.reconnect(&server);
        Ok(server)
    }

    /// Sign in from a terminal: a one-shot listener on 127.0.0.1 catches
    /// the redirect. `open` shows or opens the URL.
    pub async fn sign_in_with_loopback(
        &self,
        name: &str,
        open: impl FnOnce(&str),
    ) -> Result<(), String> {
        let (listener, port) = mira_auth::loopback::bind_any()
            .await
            .map_err(|e| e.to_string())?;
        let redirect = format!("http://127.0.0.1:{port}/callback");
        let url = self.begin_sign_in(name, &redirect).await?;
        open(&url);
        let page = mira_auth::loopback::html_page(
            "Signed in",
            &format!("Mira is connected to <b>{name}</b>. You can close this tab."),
            true,
        );
        let cb = mira_auth::loopback::await_callback(listener, &page, Duration::from_secs(300))
            .await
            .map_err(|e| e.to_string())?;
        let mut callback = url::Url::parse(&redirect).map_err(|e| e.to_string())?;
        {
            let mut q = callback.query_pairs_mut();
            for (k, v) in [
                ("code", &cb.code),
                ("state", &cb.state),
                ("error", &cb.error),
                ("error_description", &cb.error_description),
            ] {
                if let Some(v) = v {
                    q.append_pair(k, v);
                }
            }
        }
        self.finish_sign_in(callback.as_str()).await.map(|_| ())
    }

    /// Forget a remote server's sign-in and reconnect without it.
    pub async fn sign_out(&self, name: &str) -> Result<(), String> {
        let spec = self.spec(name)?;
        let transport = expand_transport(&spec, &self.inner.vars())
            .map_err(|vars| format!("set {} first", vars.join(", ")))?;
        let url = transport
            .url()
            .ok_or_else(|| format!("`{name}` isn't a remote server"))?;
        auth::sign_out(&self.inner.opts.credentials, url)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.reconnect(name);
        Ok(())
    }

    /// Call a tool directly (outside a model turn).
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<JsonMap<String, JsonValue>>,
    ) -> Result<CallToolResult, String> {
        self.inner.call_tool(server, tool, arguments).await
    }

    /// Prompts of every connected server, as `(server, prompt)`.
    pub fn prompts(&self) -> Vec<(String, PromptView)> {
        let servers = self.inner.servers.read().unwrap();
        servers
            .iter()
            .filter(|(_, e)| e.status == Status::Connected)
            .flat_map(|(name, e)| e.prompts.iter().map(|p| (name.clone(), prompt_view(p))))
            .collect()
    }

    /// Render a server prompt to the text of a user message.
    pub async fn get_prompt(
        &self,
        server: &str,
        prompt: &str,
        arguments: BTreeMap<String, String>,
    ) -> Result<String, String> {
        let peer = self.inner.peer(server)?;
        let mut params = GetPromptRequestParams::new(prompt);
        if !arguments.is_empty() {
            params = params.with_arguments(
                arguments
                    .into_iter()
                    .map(|(k, v)| (k, JsonValue::String(v)))
                    .collect(),
            );
        }
        let result = tokio::time::timeout(self.inner.opts.tool_timeout, peer.get_prompt(params))
            .await
            .map_err(|_| format!("`{server}` didn't answer in time"))?
            .map_err(|e| format!("`{server}`: {e}"))?;
        let parts: Vec<String> = result
            .messages
            .iter()
            .map(|m| tool::render_blocks(std::slice::from_ref(&m.content)).text)
            .collect();
        Ok(parts.join("\n\n"))
    }

    /// Server-provided usage notes, for the system prompt.
    pub fn instructions(&self) -> Vec<(String, String)> {
        let servers = self.inner.servers.read().unwrap();
        servers
            .iter()
            .filter(|(_, e)| e.status == Status::Connected)
            .filter_map(|(n, e)| e.instructions.clone().map(|i| (n.clone(), i)))
            .collect()
    }

    /// Disconnect everything.
    pub fn shutdown(&self) {
        let mut servers = self.inner.servers.write().unwrap();
        for e in servers.values_mut() {
            e.disconnect();
        }
        servers.clear();
    }
}

fn prompt_view(p: &Prompt) -> PromptView {
    PromptView {
        name: p.name.clone(),
        description: p.description.clone(),
        arguments: p
            .arguments
            .iter()
            .flatten()
            .map(|a| PromptArgView {
                name: a.name.clone(),
                description: a.description.clone(),
                required: a.required.unwrap_or(false),
            })
            .collect(),
    }
}

/// Variables for `${...}`: saved values, then the plugin's own
/// (`${CLAUDE_PLUGIN_ROOT}`), which win.
fn vars_for(spec: &ServerSpec, saved: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut vars = saved.clone();
    vars.extend(spec.extra_vars());
    vars
}

/// Every unset `${VAR}` in a definition.
fn missing_in(spec: &ServerSpec, saved: &BTreeMap<String, String>) -> Vec<String> {
    let vars = vars_for(spec, saved);
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        for v in missing_vars(s, &vars) {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    };
    match &spec.transport {
        Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            add(command);
            args.iter().for_each(|a| add(a));
            env.values().for_each(|v| add(v));
            if let Some(c) = cwd {
                add(c);
            }
        }
        Transport::Http { url, headers, .. } | Transport::Sse { url, headers, .. } => {
            add(url);
            headers.values().for_each(|v| add(v));
        }
    }
    out
}

/// The spec's transport with `${VAR}`s filled in. A header whose
/// variable is unset is left out, so the server can still answer 401
/// and offer OAuth sign-in (this is how plugins like GitHub's, which
/// send `Bearer ${GITHUB_PERSONAL_ACCESS_TOKEN}`, behave in Claude
/// Code). Anything else unset is an error listing the variables.
fn expand_transport(
    spec: &ServerSpec,
    saved: &BTreeMap<String, String>,
) -> Result<Transport, Vec<String>> {
    let vars = vars_for(spec, saved);
    let mut missing: Vec<String> = Vec::new();
    let mut ex = |s: &str| match expand(s, &vars) {
        Ok(v) => v,
        Err(_) => {
            for v in missing_vars(s, &vars) {
                if !missing.contains(&v) {
                    missing.push(v);
                }
            }
            String::new()
        }
    };
    let headers_of = |m: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        m.iter()
            .filter_map(|(k, v)| expand(v, &vars).ok().map(|v| (k.clone(), v)))
            .filter(|(_, v)| !v.trim().is_empty() && v.trim() != "Bearer")
            .collect()
    };
    let transport = match &spec.transport {
        Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } => Transport::Stdio {
            command: ex(command),
            args: args.iter().map(|a| ex(a)).collect(),
            env: env.iter().map(|(k, v)| (k.clone(), ex(v))).collect(),
            cwd: cwd.as_deref().map(&mut ex),
        },
        Transport::Http {
            url,
            headers,
            oauth,
        } => Transport::Http {
            url: ex(url),
            headers: headers_of(headers),
            oauth: oauth.clone(),
        },
        Transport::Sse {
            url,
            headers,
            oauth,
        } => Transport::Sse {
            url: ex(url),
            headers: headers_of(headers),
            oauth: oauth.clone(),
        },
    };
    if missing.is_empty() {
        Ok(transport)
    } else {
        Err(missing)
    }
}

impl Inner {
    pub(crate) fn max_output_chars(&self) -> usize {
        self.opts.max_output_chars
    }

    /// Saved `${VAR}` values.
    fn vars(&self) -> BTreeMap<String, String> {
        vars::load(&vars::path_for(&self.opts.credentials))
    }

    fn emit(&self, e: McpEvent) {
        let _ = self.events.send(e);
    }

    fn update_state(&self, f: impl FnOnce(&mut McpState)) -> Result<(), String> {
        let mut st = McpState::load(&self.opts.state_file).map_err(|e| e.to_string())?;
        f(&mut st);
        st.save(&self.opts.state_file).map_err(|e| e.to_string())
    }

    /// Re-run [`Self::apply`] with the current definitions, after a state
    /// change (enable/disable/approve).
    fn reapply(&self) {
        let specs: Vec<ServerSpec> = self
            .servers
            .read()
            .unwrap()
            .values()
            .map(|e| e.spec.clone())
            .collect();
        let project = self.project.read().unwrap().clone();
        self.apply(specs, project);
    }

    fn apply(&self, specs: Vec<ServerSpec>, project: Option<PathBuf>) {
        let state = McpState::load(&self.opts.state_file).unwrap_or_else(|e| {
            warn!(%e, "mcp: unreadable state file; using defaults");
            McpState::default()
        });
        *self.project.write().unwrap() = project.clone();
        let mut servers = self.servers.write().unwrap();

        let wanted: BTreeMap<String, ServerSpec> =
            specs.into_iter().map(|s| (s.name.clone(), s)).collect();
        let gone: Vec<String> = servers
            .keys()
            .filter(|k| !wanted.contains_key(*k))
            .cloned()
            .collect();
        for name in gone {
            if let Some(mut e) = servers.remove(&name) {
                e.disconnect();
            }
            self.emit(McpEvent::Removed(name));
        }

        for (name, spec) in wanted {
            let gate = if state.is_disabled(&spec, project.as_deref()) {
                Some(Status::Disabled)
            } else {
                match state.approval(&spec, project.as_deref()) {
                    Approval::Pending => Some(Status::NeedsApproval),
                    Approval::Rejected => Some(Status::Rejected),
                    Approval::Approved => None,
                }
            };
            let entry = servers
                .entry(name.clone())
                .or_insert_with(|| Entry::new(spec.clone(), Status::Connecting));
            let changed = entry.spec != spec;
            entry.spec = spec;
            match gate {
                Some(status) => {
                    if entry.status != status {
                        entry.disconnect();
                        entry.status = status;
                        self.emit(McpEvent::Changed(name));
                    }
                }
                None => {
                    let running = matches!(
                        entry.status,
                        Status::Connected
                            | Status::Connecting
                            | Status::NeedsAuth
                            | Status::NeedsSetup { .. }
                    ) && entry.generation > 0;
                    if changed || !running {
                        self.start_connect(entry);
                    }
                }
            }
        }
    }

    /// Disconnect (if needed) and connect in the background.
    fn start_connect(&self, entry: &mut Entry) {
        entry.disconnect();
        entry.generation += 1;
        entry.status = Status::Connecting;
        entry.challenge = None;
        let generation = entry.generation;
        let spec = entry.spec.clone();
        self.emit(McpEvent::Changed(spec.name.clone()));
        let Some(inner) = self.me.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let outcome = inner.open(&spec).await;
            inner.finish_connect(&spec.name, generation, outcome).await;
        });
    }

    async fn open(&self, spec: &ServerSpec) -> Result<Opened, OpenError> {
        let transport = expand_transport(spec, &self.vars()).map_err(OpenError::NeedsSetup)?;
        let handler = Handler {
            server: spec.name.clone(),
            inner: self.me.clone(),
            root: self.project.read().unwrap().clone(),
            log: log_path(&self.opts.log_dir, &spec.name),
        };
        let timeout = self.opts.connect_timeout;
        let connect = async {
            match &transport {
                Transport::Stdio {
                    command,
                    args,
                    env,
                    cwd,
                } => {
                    let log = open_log(&handler.log);
                    let mut cmd = tokio::process::Command::new(command);
                    cmd.args(args).envs(env);
                    if let Some(dir) = cwd {
                        cmd.current_dir(dir);
                    } else if let Some(root) = &handler.root {
                        cmd.current_dir(root);
                    }
                    let mut builder = rmcp::transport::TokioChildProcess::builder(cmd);
                    builder = match log {
                        Some(f) => builder.stderr(f),
                        None => builder.stderr(std::process::Stdio::null()),
                    };
                    let (proc, _) = builder.spawn().map_err(|e| {
                        OpenError::Failed(format!("couldn't start `{command}`: {e}"))
                    })?;
                    handler
                        .serve(proc)
                        .await
                        .map_err(|e| classify(e, false))
                        .map(|s| (s, false))
                }
                Transport::Http { url, headers, .. } => {
                    let mut header_map = HashMap::new();
                    for (k, v) in headers {
                        let name = reqwest::header::HeaderName::try_from(k.as_str())
                            .map_err(|e| OpenError::Failed(format!("header `{k}`: {e}")))?;
                        let value = reqwest::header::HeaderValue::try_from(v.as_str())
                            .map_err(|e| OpenError::Failed(format!("header `{k}`: {e}")))?;
                        header_map.insert(name, value);
                    }
                    let config = rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url.as_str())
                        .custom_headers(header_map);
                    let manager = auth::stored_manager(&self.opts.credentials, url)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(server = %spec.name, %e, "mcp: stored sign-in unusable");
                            None
                        });
                    match manager {
                        Some(m) => {
                            let client = rmcp::transport::auth::AuthClient::new(
                                reqwest013::Client::default(),
                                m,
                            );
                            let t = rmcp::transport::StreamableHttpClientTransport::with_client(
                                client, config,
                            );
                            handler
                                .serve(t)
                                .await
                                .map_err(|e| classify(e, true))
                                .map(|s| (s, true))
                        }
                        None => {
                            let t =
                                rmcp::transport::StreamableHttpClientTransport::from_config(config);
                            handler
                                .serve(t)
                                .await
                                .map_err(|e| classify(e, false))
                                .map(|s| (s, false))
                        }
                    }
                }
                Transport::Sse { url, headers, .. } => {
                    let manager = auth::stored_manager(&self.opts.credentials, url)
                        .await
                        .ok()
                        .flatten()
                        .map(|m| Arc::new(tokio::sync::Mutex::new(m)));
                    let signed_in = manager.is_some();
                    let token: Option<crate::sse::TokenFn> = manager.map(|m| {
                        Arc::new(move || {
                            let m = m.clone();
                            Box::pin(async move { m.lock().await.get_access_token().await.ok() })
                                as futures::future::BoxFuture<'static, Option<String>>
                        }) as crate::sse::TokenFn
                    });
                    let t = crate::sse::connect(url, headers, token, timeout)
                        .await
                        .map_err(|e| match e {
                            crate::sse::SseConnectError::AuthRequired(c) => {
                                OpenError::NeedsAuth(Some(c).filter(|c| !c.is_empty()))
                            }
                            crate::sse::SseConnectError::Failed(m) => OpenError::Failed(m),
                        })?;
                    handler
                        .serve(t)
                        .await
                        .map_err(|e| classify(e, signed_in))
                        .map(|s| (s, signed_in))
                }
            }
        };
        let (service, signed_in) =
            tokio::time::timeout(timeout, connect).await.map_err(|_| {
                OpenError::Failed(format!(
                    "didn't finish starting within {}s (MCP_TIMEOUT)",
                    timeout.as_secs()
                ))
            })??;

        let info = service.peer_info();
        let caps = info.as_ref().map(|i| i.capabilities.clone());
        let list_timeout = timeout;
        let tools = if caps.as_ref().is_none_or(|c| c.tools.is_some()) {
            tokio::time::timeout(list_timeout, service.list_all_tools())
                .await
                .map_err(|_| OpenError::Failed("tools/list timed out".into()))?
                .map_err(|e| OpenError::Failed(format!("tools/list: {e}")))?
        } else {
            Vec::new()
        };
        let resources = if caps.as_ref().is_some_and(|c| c.resources.is_some()) {
            match tokio::time::timeout(list_timeout, service.list_all_resources()).await {
                Ok(Ok(r)) => r,
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let prompts = if caps.as_ref().is_some_and(|c| c.prompts.is_some()) {
            match tokio::time::timeout(list_timeout, service.list_all_prompts()).await {
                Ok(Ok(p)) => p,
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        Ok(Opened {
            server_info: info.as_ref().and_then(|i| i.server_info.clone()),
            instructions: info.as_ref().and_then(|i| i.instructions.clone()),
            service,
            tools,
            resources,
            prompts,
            signed_in,
        })
    }

    async fn finish_connect(
        self: Arc<Self>,
        name: &str,
        generation: u64,
        outcome: Result<Opened, OpenError>,
    ) {
        let mut servers = self.servers.write().unwrap();
        let Some(entry) = servers.get_mut(name) else {
            return;
        };
        if entry.generation != generation {
            return; // superseded; dropping `outcome` closes it
        }
        match outcome {
            Ok(opened) => {
                let remote = entry.spec.is_remote();
                entry.tools = opened
                    .tools
                    .iter()
                    .map(|t| Arc::new(McpTool::new(name, t, remote, self.me.clone())))
                    .collect();
                entry.resources = opened.resources;
                entry.prompts = opened.prompts;
                entry.server_info = opened.server_info;
                entry.instructions = opened.instructions;
                entry.signed_in = opened.signed_in;
                entry.peer = Some(opened.service.peer().clone());
                entry.status = Status::Connected;
                let stop = CancellationToken::new();
                entry.stop = Some(stop.clone());
                debug!(server = name, tools = entry.tools.len(), "mcp: connected");
                let inner = self.clone();
                let name = name.to_owned();
                let service = opened.service;
                tokio::spawn(async move {
                    let cancel = service.cancellation_token();
                    let waiting = service.waiting();
                    tokio::pin!(waiting);
                    tokio::select! {
                        _ = stop.cancelled() => {
                            cancel.cancel();
                            let _ = tokio::time::timeout(Duration::from_secs(5), waiting).await;
                        }
                        _ = &mut waiting => inner.on_closed(&name, generation),
                    }
                });
            }
            Err(OpenError::NeedsAuth(challenge)) => {
                entry.status = Status::NeedsAuth;
                entry.challenge = challenge;
                entry.signed_in = false;
            }
            Err(OpenError::NeedsSetup(variables)) => {
                entry.status = Status::NeedsSetup { variables };
            }
            Err(OpenError::Failed(message)) => {
                debug!(server = name, %message, "mcp: connect failed");
                entry.status = Status::Failed { message };
            }
        }
        drop(servers);
        self.emit(McpEvent::Changed(name.to_owned()));
    }

    /// The connection ended without us asking: retry with backoff.
    fn on_closed(self: &Arc<Self>, name: &str, generation: u64) {
        let delay = {
            let mut servers = self.servers.write().unwrap();
            let Some(entry) = servers.get_mut(name) else {
                return;
            };
            if entry.generation != generation || entry.status != Status::Connected {
                return;
            }
            entry.peer = None;
            entry.stop = None;
            entry.tools.clear();
            entry.drops += 1;
            let message = if entry.spec.is_remote() {
                "the connection closed"
            } else {
                "the server process exited"
            };
            entry.status = Status::Failed {
                message: message.into(),
            };
            debug!(server = name, drops = entry.drops, "mcp: {message}");
            (entry.drops <= MAX_RECONNECTS).then(|| Duration::from_secs(1 << (entry.drops - 1)))
        };
        self.emit(McpEvent::Changed(name.to_owned()));
        if let Some(delay) = delay {
            let inner = self.clone();
            let name = name.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let mut servers = inner.servers.write().unwrap();
                if let Some(entry) = servers.get_mut(&name) {
                    if entry.generation == generation
                        && matches!(entry.status, Status::Failed { .. })
                    {
                        inner.start_connect(entry);
                    }
                }
            });
        }
    }

    fn peer(&self, server: &str) -> Result<Peer<RoleClient>, String> {
        let servers = self.servers.read().unwrap();
        let entry = servers
            .get(server)
            .ok_or_else(|| format!("no MCP server `{server}`"))?;
        entry
            .peer
            .clone()
            .ok_or_else(|| format!("MCP server `{server}` is {}", entry.status.label()))
    }

    pub(crate) async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<JsonMap<String, JsonValue>>,
    ) -> Result<CallToolResult, String> {
        let peer = self.peer(server)?;
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        match tokio::time::timeout(self.opts.tool_timeout, peer.call_tool(params)).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(format!("`{server}`: {e}")),
            Err(_) => Err(format!(
                "`{server}`: `{tool}` didn't finish within {}s (MCP_TOOL_TIMEOUT)",
                self.opts.tool_timeout.as_secs()
            )),
        }
    }

    pub(crate) async fn read_resource(
        &self,
        server: &str,
        uri: &str,
    ) -> Result<Vec<ResourceContents>, String> {
        let peer = self.peer(server)?;
        let r = tokio::time::timeout(
            self.opts.tool_timeout,
            peer.read_resource(ReadResourceRequestParams::new(uri)),
        )
        .await
        .map_err(|_| format!("`{server}` didn't answer in time"))?
        .map_err(|e| format!("`{server}`: {e}"))?;
        Ok(r.contents)
    }

    pub(crate) fn resources(&self) -> Vec<(String, ResourceView)> {
        let servers = self.servers.read().unwrap();
        servers
            .iter()
            .filter(|(_, e)| e.status == Status::Connected)
            .flat_map(|(n, e)| e.resources.iter().map(|r| (n.clone(), resource_view(r))))
            .collect()
    }

    /// Re-fetch a server's lists after a `*/list_changed` notification.
    fn refresh_lists(self: &Arc<Self>, name: &str) {
        let Ok(peer) = self.peer(name) else { return };
        let inner = self.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let tools = peer.list_all_tools().await.ok();
            let resources = peer.list_all_resources().await.ok();
            let prompts = peer.list_all_prompts().await.ok();
            {
                let mut servers = inner.servers.write().unwrap();
                let Some(entry) = servers.get_mut(&name) else {
                    return;
                };
                if entry.status != Status::Connected {
                    return;
                }
                let remote = entry.spec.is_remote();
                if let Some(tools) = tools {
                    entry.tools = tools
                        .iter()
                        .map(|t| Arc::new(McpTool::new(&name, t, remote, inner.me.clone())))
                        .collect();
                }
                if let Some(r) = resources {
                    entry.resources = r;
                }
                if let Some(p) = prompts {
                    entry.prompts = p;
                }
            }
            debug!(server = %name, "mcp: lists refreshed");
            inner.emit(McpEvent::Changed(name));
        });
    }

    fn view(&self, e: &Entry) -> ServerView {
        ServerView {
            name: e.spec.name.clone(),
            scope: e.spec.scope.clone(),
            transport: e.spec.transport.kind(),
            target: e.spec.transport.summary(),
            source: e.spec.source.as_ref().map(|p| p.display().to_string()),
            status: e.status.clone(),
            tools: e
                .tools
                .iter()
                .map(|t| ToolView {
                    name: t.local_name.clone(),
                    remote_name: t.remote_name.clone(),
                    description: t.description.clone(),
                    read_only: t.read_only,
                })
                .collect(),
            resources: e.resources.iter().map(resource_view).collect(),
            prompts: e.prompts.iter().map(prompt_view).collect(),
            server_name: e.server_info.as_ref().map(|i| i.name.clone()),
            server_version: e.server_info.as_ref().map(|i| i.version.clone()),
            instructions: e.instructions.clone(),
            can_sign_in: e.spec.is_remote(),
            signed_in: e.signed_in,
            missing_vars: missing_in(&e.spec, &self.vars()),
            log_path: (!e.spec.is_remote()).then(|| {
                log_path(&self.opts.log_dir, &e.spec.name)
                    .display()
                    .to_string()
            }),
            config: e.spec.to_config(),
        }
    }
}

fn resource_view(r: &Resource) -> ResourceView {
    ResourceView {
        uri: r.uri.clone(),
        name: r.name.clone(),
        description: r.description.clone(),
        mime_type: r.mime_type.clone(),
    }
}

struct Opened {
    service: RunningService<RoleClient, Handler>,
    tools: Vec<rmcp::model::Tool>,
    resources: Vec<Resource>,
    prompts: Vec<Prompt>,
    server_info: Option<Implementation>,
    instructions: Option<String>,
    signed_in: bool,
}

enum OpenError {
    NeedsAuth(Option<String>),
    NeedsSetup(Vec<String>),
    Failed(String),
}

/// Explain a failed start of sign-in. The common case: the server's
/// authorization server doesn't let apps register themselves (GitHub's
/// doesn't), so there's no OAuth without a client ID of your own.
fn sign_in_error(name: &str, e: &rmcp::transport::auth::AuthError, missing: &[String]) -> String {
    use rmcp::transport::auth::AuthError;
    // A registration endpoint that answers 401/403 only accepts apps it
    // has approved (Figma's does this).
    let refused = matches!(e, AuthError::RegistrationFailed(m)
        if m.contains("403") || m.contains("401") || m.contains("Forbidden"));
    if refused {
        return format!(
            "`{name}` only lets apps it has approved sign in, and it turned Mira away. \
             Use a token header instead if the service offers one{}",
            if missing.is_empty() {
                ".".to_owned()
            } else {
                format!(" (set {}).", missing.join(" / "))
            }
        );
    }
    let no_registration =
        matches!(e, AuthError::RegistrationFailed(m) if m.contains("not supported"));
    let no_oauth =
        matches!(e, AuthError::NoAuthorizationSupport) || matches!(e, AuthError::MetadataError(_));
    if no_registration || no_oauth {
        let why = if no_registration {
            format!(
                "`{name}` doesn't let apps register for sign-in, so Mira can't sign in with OAuth"
            )
        } else {
            format!("`{name}` doesn't offer OAuth sign-in")
        };
        return match missing {
            [] => format!(
                "{why}. Add a token header to the server, or an OAuth app's client ID (`oauth.clientId`)."
            ),
            vars => format!("{why}. Add your {} instead.", vars.join(" / ")),
        };
    }
    format!("couldn't start sign-in for `{name}`: {e}")
}

/// Sort a failed handshake into "sign in" or a plain failure, looking
/// through the error chain for the transport's 401.
fn classify(err: impl std::error::Error + 'static, had_token: bool) -> OpenError {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&err);
    while let Some(e) = cur {
        if let Some(a) =
            e.downcast_ref::<rmcp::transport::streamable_http_client::AuthRequiredError>()
        {
            let c = a.www_authenticate_header.clone();
            return OpenError::NeedsAuth(Some(c).filter(|c| !c.is_empty()));
        }
        if let Some(rmcp::transport::auth::AuthError::AuthorizationRequired) =
            e.downcast_ref::<rmcp::transport::auth::AuthError>()
        {
            return OpenError::NeedsAuth(None);
        }
        cur = e.source();
    }
    let text = full_chain(&err);
    if text.contains("Auth required") || (had_token && text.contains("401")) {
        return OpenError::NeedsAuth(None);
    }
    OpenError::Failed(text)
}

/// A readable one-liner from an error chain: rmcp's wrappers name
/// internal types ("Transport [rmcp::…] error"), so keep the causes a
/// person can act on, root cause last.
fn full_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        let mut s = e.to_string();
        // "Send message error Transport [rmcp::…] error: Client error: X"
        // → "X".
        if let Some(i) = s.rfind("] error: ") {
            s = s[i + "] error: ".len()..].to_owned();
        }
        let s = s
            .trim_start_matches("Client error: ")
            .trim_end_matches(", when send initialize request")
            .to_owned();
        let internal = s.contains("rmcp::");
        if !internal && !s.is_empty() && !parts.iter().any(|p| p.contains(&s)) {
            parts.push(s);
        }
        cur = e.source();
    }
    let joined = if parts.is_empty() {
        err.to_string()
    } else {
        parts.join(": ")
    };
    if joined.contains("error sending request") || joined.contains("tcp connect error") {
        let root = parts.last().cloned().unwrap_or_default();
        return format!("couldn't reach the server: {root}");
    }
    joined
}

fn log_path(dir: &Path, server: &str) -> PathBuf {
    dir.join(format!("{}.log", crate::spec::sanitize(server)))
}

fn open_log(path: &Path) -> Option<std::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// rmcp client callbacks for one server.
#[derive(Clone)]
struct Handler {
    server: String,
    inner: Weak<Inner>,
    root: Option<PathBuf>,
    log: PathBuf,
}

#[allow(deprecated)]
impl ClientHandler for Handler {
    fn get_info(&self) -> ClientInfo {
        let mut capabilities = ClientCapabilities::default();
        capabilities.roots = Some(Default::default());
        ClientInfo::new(
            capabilities,
            Implementation::new("mira", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, McpError> {
        let roots = self
            .root
            .iter()
            .filter_map(|p| url::Url::from_directory_path(p).ok())
            .map(|u| Root::new(u.to_string()))
            .collect();
        Ok(ListRootsResult::new(roots))
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.refresh_lists(&self.server);
        }
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.refresh_lists(&self.server);
        }
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.refresh_lists(&self.server);
        }
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        if let Some(mut f) = open_log(&self.log) {
            use std::io::Write;
            let _ = writeln!(f, "[{:?}] {}", params.level, params.data);
        }
    }
}

/// The registry's view of the MCP tools: every connected server's tools,
/// plus the resource tools when some server has resources.
struct Source {
    inner: Weak<Inner>,
}

impl ToolSource for Source {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let Some(inner) = self.inner.upgrade() else {
            return Vec::new();
        };
        let servers = inner.servers.read().unwrap();
        let mut out: Vec<Arc<dyn Tool>> = Vec::new();
        let mut any_resources = false;
        for e in servers.values() {
            if e.status != Status::Connected {
                continue;
            }
            any_resources |= !e.resources.is_empty();
            out.extend(e.tools.iter().map(|t| t.clone() as Arc<dyn Tool>));
        }
        drop(servers);
        if any_resources {
            out.extend(tool::resource_tools(&inner));
        }
        out
    }
}
