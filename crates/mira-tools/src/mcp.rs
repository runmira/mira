//! MCP (Model Context Protocol) client — wraps remote tools as [`Tool`]s.
//!
//! For every server in the user's `mcp_servers:` config we open a client
//! session at startup, ask it to list its tools, and register each remote
//! tool under a namespaced name (`mcp__<server>__<tool>`) so it lives in the
//! same [`Registry`] as the built-ins. Calls are routed back through the
//! same session; the session (and, for stdio, its child process) stays alive
//! as long as any tool it produced is held.
//!
//! Two transports are supported today:
//!
//! - **stdio** — spawn a subprocess (e.g. `npx @modelcontextprotocol/server-github`)
//!   and speak the protocol over its stdio.
//! - **streamable HTTP** — connect to a remote server URL.
//!
//! Failures at startup (bad command, no such URL, protocol error) are
//! surfaced as `Result` — the caller decides whether to warn or bail. Once
//! connected, an individual failing `call_tool` is reported back to the
//! model as an error tool result, not a panic.
//!
//! [`Registry`]: crate::Registry

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_config::{McpHttpConfig, McpServerConfig, McpStdioConfig};
use mira_core::{ToolCall, ToolResult};
use rmcp::model::{CallToolRequestParams, ContentBlock, Tool as RemoteTool};
use rmcp::service::RunningService;
use rmcp::transport::{
    ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::process::Command;
use tracing::warn;

use crate::context::ToolContext;
use crate::tool::{Action, Tool, ToolError};

/// One connected MCP server, plus the wrapped tool handles produced from
/// its tool list. Holding this keeps the underlying transport (and, for
/// stdio, the child process) alive — drop it to shut the server down.
///
/// We split "the connection" from "the individual tools" because the
/// [`Registry`] takes owned `Tool` values. The tools each hold an `Arc` on
/// the same shared service, so dropping the `Registry` closes the server.
///
/// [`Registry`]: crate::Registry
pub struct McpConnection {
    /// Human name from `mcp_servers.<name>` — used for logging and to build
    /// the `mcp__<name>__<tool>` namespace prefix.
    pub server_name: String,
    /// The wrapped remote tools, ready to be handed to a `Registry`.
    pub tools: Vec<Arc<dyn Tool>>,
}

/// Connect to one configured MCP server. Handshake, then list its tools
/// and produce a wrapped [`Tool`] for each.
///
/// A failure here should abort *this* server only; the caller warns and
/// moves on so a broken entry can't take down startup for everyone.
pub async fn connect(name: &str, cfg: &McpServerConfig) -> Result<McpConnection> {
    let service: Arc<RunningService<RoleClient, ()>> = match cfg {
        McpServerConfig::Stdio(s) => connect_stdio(name, s).await?,
        McpServerConfig::Http(h) => connect_http(name, h).await?,
    };

    let remote_tools = service
        .list_all_tools()
        .await
        .with_context(|| format!("mcp `{name}`: list_tools"))?;

    let tools: Vec<Arc<dyn Tool>> = remote_tools
        .into_iter()
        .map(|t| {
            Arc::new(McpTool::new(name.to_owned(), service.clone(), t)) as Arc<dyn Tool>
        })
        .collect();

    Ok(McpConnection {
        server_name: name.to_owned(),
        tools,
    })
}

async fn connect_stdio(
    name: &str,
    cfg: &McpStdioConfig,
) -> Result<Arc<RunningService<RoleClient, ()>>> {
    if cfg.command.trim().is_empty() {
        return Err(anyhow!("mcp `{name}`: `command` is empty"));
    }
    let args = cfg.args.clone();
    let env = expand_env(&cfg.env);
    let cwd = cfg.cwd.clone();

    let cmd = Command::new(&cfg.command).configure(|c| {
        if !args.is_empty() {
            c.args(&args);
        }
        for (k, v) in &env {
            c.env(k, v);
        }
        if let Some(dir) = &cwd {
            c.current_dir(dir);
        }
    });

    let transport = TokioChildProcess::new(cmd)
        .with_context(|| format!("mcp `{name}`: spawn `{}`", cfg.command))?;

    let service = ()
        .serve(transport)
        .await
        .with_context(|| format!("mcp `{name}`: initialize handshake"))?;
    Ok(Arc::new(service))
}

async fn connect_http(
    name: &str,
    cfg: &McpHttpConfig,
) -> Result<Arc<RunningService<RoleClient, ()>>> {
    if cfg.url.trim().is_empty() {
        return Err(anyhow!("mcp `{name}`: `url` is empty"));
    }
    let transport = if let Some(auth) = &cfg.auth {
        let http_cfg =
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                cfg.url.clone(),
            )
            .auth_header(expand_var(auth));
        StreamableHttpClientTransport::from_config(http_cfg)
    } else {
        StreamableHttpClientTransport::from_uri(cfg.url.clone())
    };
    let service = ()
        .serve(transport)
        .await
        .with_context(|| format!("mcp `{name}`: initialize handshake @ {}", cfg.url))?;
    Ok(Arc::new(service))
}

/// Expand `${VAR}` occurrences in each value against the current process env,
/// leaving unknown variables as the literal `${…}` so a missing token is
/// obvious in logs rather than a silent empty string.
fn expand_env(env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    env.iter()
        .map(|(k, v)| (k.clone(), expand_var(v)))
        .collect()
}

fn expand_var(s: &str) -> String {
    shellexpand::env(s)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| s.to_owned())
}

/// A single remote MCP tool exposed to the model via our `Tool` trait.
///
/// The wrapper is a thin adapter: it translates our name and arg JSON into
/// an MCP `call_tool` request, dispatches through the shared service, and
/// flattens the returned content blocks into a string the harness feeds
/// back to the model.
pub struct McpTool {
    /// The server's local name — e.g. `github`.
    server_name: String,
    /// The remote tool's name as returned by `tools/list` — e.g. `create_pr`.
    remote_name: String,
    /// The namespaced tool name exposed to the model — e.g.
    /// `mcp__github__create_pr`. Kept separate so we don't recompute it per
    /// spec() call and so collisions with built-ins are impossible.
    local_name: String,
    description: String,
    parameters: JsonValue,
    service: Arc<RunningService<RoleClient, ()>>,
}

impl McpTool {
    fn new(
        server_name: String,
        service: Arc<RunningService<RoleClient, ()>>,
        remote: RemoteTool,
    ) -> Self {
        let remote_name = remote.name.into_owned();
        let local_name = format!("mcp__{server_name}__{remote_name}");
        let description = remote
            .description
            .map(|c| c.into_owned())
            .unwrap_or_else(|| format!("Remote MCP tool `{remote_name}` from server `{server_name}`."));
        // The remote schema arrives as `Arc<JsonObject>`; clone into a Value so
        // we can hand it to the provider as-is. Fall back to a permissive
        // empty-object schema if the remote omitted one.
        let parameters = JsonValue::Object(JsonMap::from_iter(
            remote.input_schema.iter().map(|(k, v)| (k.clone(), v.clone())),
        ));
        let parameters = if parameters.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            serde_json::json!({ "type": "object", "properties": {} })
        } else {
            parameters
        };
        Self {
            server_name,
            remote_name,
            local_name,
            description,
            parameters,
            service,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.local_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    fn action(&self) -> Action {
        // MCP tools are opaque third-party code — gate them like `bash`
        // rather than pretending they're pure. Users who trust a specific
        // server can add e.g. `Bash(mcp:github:*)` to `allow`.
        Action::Bash
    }

    fn policy_target(&self, _call: &ToolCall) -> String {
        format!("mcp:{}:{}", self.server_name, self.remote_name)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // Model-emitted arguments arrive as a JSON string. MCP wants a
        // `JsonObject`, so we parse and enforce that shape — a non-object
        // top-level value (rare but seen with weaker models) is a client
        // error worth surfacing.
        let arguments: Option<JsonMap<String, JsonValue>> = if call.function.arguments.trim().is_empty()
        {
            None
        } else {
            match serde_json::from_str::<JsonValue>(&call.function.arguments) {
                Ok(JsonValue::Object(m)) => Some(m),
                Ok(JsonValue::Null) => None,
                Ok(other) => {
                    return Err(ToolError::InvalidArgs(format!(
                        "expected JSON object, got {other:?}"
                    )))
                }
                Err(e) => return Err(ToolError::InvalidArgs(e.to_string())),
            }
        };

        let mut params = CallToolRequestParams::new(self.remote_name.clone());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }

        let result = self
            .service
            .call_tool(params)
            .await
            .map_err(|e| ToolError::Failed(format!("mcp `{}`: {e}", self.server_name)))?;

        let body = flatten_content(&result.content);
        let call_id = call.id.clone();
        if result.is_error.unwrap_or(false) {
            Ok(ToolResult::err(call_id, body))
        } else {
            Ok(ToolResult::ok(call_id, body))
        }
    }
}

/// Flatten a `CallToolResult` content vector into a single string.
///
/// MCP servers can return mixed content (text, images, resources, links).
/// The harness feeds a single string back to the model per call, so
/// non-text blocks are rendered as short placeholders. When the tool
/// eventually returns structured data (vision) we'll re-shape this.
fn flatten_content(blocks: &[ContentBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t.text);
            }
            ContentBlock::Image(_) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("[image omitted]");
            }
            ContentBlock::Audio(_) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("[audio omitted]");
            }
            ContentBlock::Resource(r) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                match serde_json::to_string(r) {
                    Ok(s) => out.push_str(&s),
                    Err(e) => {
                        warn!(%e, "mcp: failed to render resource block");
                        out.push_str("[resource]");
                    }
                }
            }
            ContentBlock::ResourceLink(r) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[resource link: {}]", r.uri));
            }
            _ => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("[unrecognised content block]");
            }
        }
    }
    out
}
