//! MCP tools as Mira [`Tool`]s.
//!
//! Each tool holds only the server's name and a weak handle to the
//! manager. Calls go through the manager's current connection, so a
//! server that reconnects keeps working without re-registering anything.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::message::ImageData;
use mira_core::{ToolCall, ToolResult};
use mira_tools::{Action, Tool, ToolContext, ToolError};
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

use crate::manager::Inner;
use crate::spec::{sanitize, tool_name};

/// One tool of one server.
pub struct McpTool {
    pub(crate) server: String,
    pub(crate) remote_name: String,
    pub(crate) local_name: String,
    pub(crate) description: String,
    pub(crate) parameters: JsonValue,
    pub(crate) read_only: bool,
    pub(crate) remote_server: bool,
    pub(crate) inner: Weak<Inner>,
}

impl McpTool {
    pub(crate) fn new(
        server: &str,
        remote: &rmcp::model::Tool,
        remote_server: bool,
        inner: Weak<Inner>,
    ) -> Self {
        let remote_name = remote.name.to_string();
        let mut description = remote
            .description
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("`{remote_name}` from the `{server}` MCP server."));
        // Some servers ship huge descriptions; they cost context every turn.
        const MAX_DESCRIPTION: usize = 2048;
        if description.len() > MAX_DESCRIPTION {
            let cut = floor_char_boundary(&description, MAX_DESCRIPTION);
            description.truncate(cut);
            description.push('…');
        }
        let mut parameters = JsonValue::Object(JsonMap::from_iter(
            remote
                .input_schema
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        ));
        if parameters.get("type").is_none() {
            parameters["type"] = json!("object");
        }
        if parameters.get("properties").is_none() {
            parameters["properties"] = json!({});
        }
        let read_only = remote
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        Self {
            server: server.to_owned(),
            local_name: tool_name(server, &remote_name),
            remote_name,
            description,
            parameters,
            read_only,
            remote_server,
            inner,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
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
        Action::Mcp
    }

    /// `server:tool`, matched by `Mcp(server:tool)` and
    /// `mcp__server__tool` rules.
    fn policy_target(&self, _call: &ToolCall) -> String {
        let local = &self.local_name["mcp__".len()..];
        match local.split_once("__") {
            Some((server, tool)) => format!("{server}:{tool}"),
            None => format!("{}:{}", sanitize(&self.server), self.remote_name),
        }
    }

    fn parallel_safe(&self, _call: &ToolCall) -> bool {
        self.read_only
    }

    /// A hosted server doesn't touch this machine, so it's as usable from
    /// a remote environment as from here. A local (stdio) server may read
    /// or write local files, so it stays out.
    fn remote_capable(&self) -> bool {
        self.remote_server
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let arguments = parse_arguments(&call.function.arguments)?;
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| ToolError::Failed("MCP is shut down".into()))?;
        let result = inner
            .call_tool(&self.server, &self.remote_name, arguments)
            .await
            .map_err(ToolError::Failed)?;
        Ok(to_tool_result(call, &result, inner.max_output_chars()))
    }
}

pub(crate) fn parse_arguments(raw: &str) -> Result<Option<JsonMap<String, JsonValue>>, ToolError> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<JsonValue>(raw) {
        Ok(JsonValue::Object(m)) => Ok(Some(m)),
        Ok(JsonValue::Null) => Ok(None),
        Ok(other) => Err(ToolError::InvalidArgs(format!(
            "expected a JSON object, got {other}"
        ))),
        Err(e) => Err(ToolError::InvalidArgs(e.to_string())),
    }
}

fn to_tool_result(call: &ToolCall, result: &CallToolResult, max_chars: usize) -> ToolResult {
    let mut rendered = render_blocks(&result.content);
    if rendered.text.trim().is_empty() {
        if let Some(structured) = &result.structured_content {
            rendered.text = serde_json::to_string_pretty(structured).unwrap_or_default();
        }
    }
    let text = truncate(rendered.text, max_chars);
    let out = if result.is_error.unwrap_or(false) {
        ToolResult::err(call.id.clone(), text)
    } else {
        ToolResult::ok(call.id.clone(), text)
    };
    out.with_images(rendered.images)
}

#[derive(Default)]
pub(crate) struct Rendered {
    pub text: String,
    pub images: Vec<ImageData>,
}

/// Text for the model plus any images, which travel as real image blocks
/// (the providers render them) instead of being dropped.
pub(crate) fn render_blocks(blocks: &[ContentBlock]) -> Rendered {
    let mut out = Rendered::default();
    let push = |s: &str, out: &mut Rendered| {
        if !out.text.is_empty() {
            out.text.push('\n');
        }
        out.text.push_str(s);
    };
    for block in blocks {
        match block {
            ContentBlock::Text(t) => push(&t.text, &mut out),
            ContentBlock::Image(img) => {
                out.images.push(ImageData {
                    media_type: img.mime_type.clone(),
                    data: img.data.clone(),
                });
                push(&format!("[image {}]", img.mime_type), &mut out);
            }
            ContentBlock::Audio(a) => push(&format!("[audio {} omitted]", a.mime_type), &mut out),
            ContentBlock::Resource(r) => push(&render_resource(&r.resource), &mut out),
            ContentBlock::ResourceLink(r) => {
                let what = r.description.as_deref().unwrap_or(&r.name);
                push(&format!("[resource {} — {what}]", r.uri), &mut out)
            }
            _ => push("[unsupported content]", &mut out),
        }
    }
    out
}

pub(crate) fn render_resource(contents: &ResourceContents) -> String {
    match contents {
        ResourceContents::TextResourceContents { uri, text, .. } => {
            format!("<resource uri=\"{uri}\">\n{text}\n</resource>")
        }
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => format!(
            "[binary resource {uri} ({}, ~{} bytes)]",
            mime_type.as_deref().unwrap_or("unknown type"),
            blob.len() * 3 / 4
        ),
        _ => "[resource]".to_owned(),
    }
}

/// Cap a result so one call can't flood the context window; the note
/// tells the model how to get less.
pub(crate) fn truncate(text: String, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text;
    }
    let cut = floor_char_boundary(&text, max_chars);
    format!(
        "{}\n\n[output truncated: {} of {} characters shown. Ask the tool for less, \
         e.g. with a narrower query or pagination.]",
        &text[..cut],
        cut,
        text.len()
    )
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `list_mcp_resources`: what connected servers offer to read.
pub struct ListResources {
    pub(crate) inner: Weak<Inner>,
}

/// `read_mcp_resource`: read one resource by URI.
pub struct ReadResource {
    pub(crate) inner: Weak<Inner>,
}

#[async_trait]
impl Tool for ListResources {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_mcp_resources".into(),
            description: "List resources (files, records, documents) that connected MCP \
                          servers make available. Read one with `read_mcp_resource`."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {"type": "string", "description": "Only this server's resources."}
                }
            }),
        }
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args = parse_arguments(&call.function.arguments)?.unwrap_or_default();
        let only = args.get("server").and_then(JsonValue::as_str);
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| ToolError::Failed("MCP is shut down".into()))?;
        let listing: Vec<JsonValue> = inner
            .resources()
            .into_iter()
            .filter(|(server, _)| only.is_none_or(|o| o == server))
            .map(|(server, r)| {
                json!({
                    "server": server,
                    "uri": r.uri,
                    "name": r.name,
                    "description": r.description,
                    "mimeType": r.mime_type,
                })
            })
            .collect();
        if listing.is_empty() {
            return Ok(ToolResult::ok(
                call.id.clone(),
                "No MCP resources available.",
            ));
        }
        Ok(ToolResult::ok(
            call.id.clone(),
            serde_json::to_string_pretty(&listing).unwrap_or_default(),
        ))
    }
}

#[async_trait]
impl Tool for ReadResource {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_mcp_resource".into(),
            description: "Read a resource from an MCP server by URI (see `list_mcp_resources`)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {"type": "string", "description": "Server name."},
                    "uri": {"type": "string", "description": "Resource URI."}
                },
                "required": ["server", "uri"]
            }),
        }
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args = parse_arguments(&call.function.arguments)?.unwrap_or_default();
        let server = args
            .get("server")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("`server` is required".into()))?;
        let uri = args
            .get("uri")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("`uri` is required".into()))?;
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| ToolError::Failed("MCP is shut down".into()))?;
        let contents = inner
            .read_resource(server, uri)
            .await
            .map_err(ToolError::Failed)?;
        let text = contents
            .iter()
            .map(render_resource)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolResult::ok(
            call.id.clone(),
            truncate(text, inner.max_output_chars()),
        ))
    }
}

/// The tools currently offered, for the registry.
pub(crate) fn resource_tools(inner: &Arc<Inner>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ListResources {
            inner: Arc::downgrade(inner),
        }),
        Arc::new(ReadResource {
            inner: Arc::downgrade(inner),
        }),
    ]
}

/// `search_mcp_tools`: find MCP tools by keyword, with their input
/// schemas, when tools load on demand.
pub struct SearchTools {
    pub(crate) inner: Weak<Inner>,
}

/// `call_mcp_tool`: run an MCP tool found with `search_mcp_tools`.
pub struct CallTool {
    pub(crate) inner: Weak<Inner>,
}

pub(crate) fn on_demand_tools(inner: &Arc<Inner>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchTools {
            inner: Arc::downgrade(inner),
        }),
        Arc::new(CallTool {
            inner: Arc::downgrade(inner),
        }),
    ]
}

/// Score a tool against search words: name hits count more than
/// description hits. Zero means no match.
fn score(t: &McpTool, words: &[String]) -> usize {
    let name = t.local_name.to_lowercase();
    let desc = t.description.to_lowercase();
    words
        .iter()
        .map(|w| {
            usize::from(name.contains(w.as_str())) * 3 + usize::from(desc.contains(w.as_str()))
        })
        .sum()
}

#[async_trait]
impl Tool for SearchTools {
    fn spec(&self) -> ToolSpec {
        let servers = self
            .inner
            .upgrade()
            .map(|i| {
                let mut names: Vec<String> =
                    i.enabled_tools().iter().map(|t| t.server.clone()).collect();
                names.sort();
                names.dedup();
                names.join(", ")
            })
            .unwrap_or_default();
        ToolSpec {
            name: "search_mcp_tools".into(),
            description: format!(
                "Find tools from connected MCP servers ({servers}). Returns matching tools \
                 with their input schemas; run one with `call_mcp_tool`. Search by what \
                 you want to do (\"create issue\", \"query database\"), optionally \
                 limited to one server. An empty query lists every tool name."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Keywords describing the task."},
                    "server": {"type": "string", "description": "Only this server's tools."},
                    "limit": {"type": "integer", "description": "Most results to return (default 8)."}
                }
            }),
        }
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args = parse_arguments(&call.function.arguments)?.unwrap_or_default();
        let query = args.get("query").and_then(JsonValue::as_str).unwrap_or("");
        let only = args.get("server").and_then(JsonValue::as_str);
        let limit = args
            .get("limit")
            .and_then(JsonValue::as_u64)
            .unwrap_or(8)
            .clamp(1, 50) as usize;
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| ToolError::Failed("MCP is shut down".into()))?;
        let remote = ctx.compute.is_remote();
        let tools: Vec<Arc<McpTool>> = inner
            .enabled_tools()
            .into_iter()
            .filter(|t| only.is_none_or(|o| t.server == o || sanitize(&t.server) == o))
            .filter(|t| !remote || t.remote_server)
            .collect();
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() > 1)
            .collect();
        if words.is_empty() {
            let listing: Vec<JsonValue> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.local_name,
                        "server": t.server,
                        "description": t.description.lines().next().unwrap_or(""),
                    })
                })
                .collect();
            let text = serde_json::to_string_pretty(&listing).unwrap_or_default();
            return Ok(ToolResult::ok(
                call.id.clone(),
                format!(
                    "{} tools. Search with a query to get input schemas.\n{text}",
                    listing.len()
                ),
            ));
        }
        let mut ranked: Vec<(usize, &Arc<McpTool>)> = tools
            .iter()
            .map(|t| (score(t, &words), t))
            .filter(|(s, _)| *s > 0)
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.local_name.cmp(&b.1.local_name)));
        let hits: Vec<JsonValue> = ranked
            .into_iter()
            .take(limit)
            .map(|(_, t)| {
                json!({
                    "name": t.local_name,
                    "server": t.server,
                    "description": t.description,
                    "input_schema": t.parameters,
                    "read_only": t.read_only,
                })
            })
            .collect();
        let text = if hits.is_empty() {
            "No matching tools. Try other words, or an empty query to list them all.".to_owned()
        } else {
            serde_json::to_string_pretty(&hits).unwrap_or_default()
        };
        Ok(ToolResult::ok(call.id.clone(), text))
    }
}

impl CallTool {
    /// The tool a call names, among enabled tools.
    fn target(&self, call: &ToolCall) -> Option<Arc<McpTool>> {
        let args = parse_arguments(&call.function.arguments).ok()??;
        let name = args.get("name")?.as_str()?;
        self.inner
            .upgrade()?
            .enabled_tools()
            .into_iter()
            .find(|t| t.local_name == name)
    }
}

#[async_trait]
impl Tool for CallTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "call_mcp_tool".into(),
            description: "Run an MCP tool found with `search_mcp_tools`. Pass its exact \
                          `name` and `arguments` matching its input schema."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The tool's name, e.g. mcp__github__create_issue."},
                    "arguments": {"type": "object", "description": "Arguments matching the tool's input schema."}
                },
                "required": ["name"]
            }),
        }
    }

    fn action(&self) -> Action {
        Action::Mcp
    }

    /// The underlying tool's `server:tool`, so `Mcp(...)` rules apply
    /// exactly as when the tool is offered directly.
    fn policy_target(&self, call: &ToolCall) -> String {
        match self.target(call) {
            Some(t) => {
                let inner_call = ToolCall {
                    id: call.id.clone(),
                    kind: call.kind,
                    function: mira_core::message::ToolCallFunction {
                        name: t.local_name.clone(),
                        arguments: String::new(),
                    },
                };
                t.policy_target(&inner_call)
            }
            None => "unknown:unknown".into(),
        }
    }

    fn parallel_safe(&self, call: &ToolCall) -> bool {
        self.target(call).is_some_and(|t| t.read_only)
    }

    /// Checked per call: only hosted servers' tools run from a remote
    /// environment.
    fn remote_capable(&self) -> bool {
        true
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args = parse_arguments(&call.function.arguments)?.unwrap_or_default();
        let name = args
            .get("name")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("`name` is required".into()))?;
        let Some(tool) = self.target(call) else {
            return Err(ToolError::InvalidArgs(format!(
                "no enabled MCP tool `{name}`; find tools with `search_mcp_tools`"
            )));
        };
        if ctx.compute.is_remote() && !tool.remote_server {
            return Err(ToolError::Failed(format!(
                "`{name}` runs on this machine, so it isn't available in a remote environment"
            )));
        }
        let arguments = match args.get("arguments") {
            None | Some(JsonValue::Null) => String::new(),
            Some(JsonValue::Object(m)) => JsonValue::Object(m.clone()).to_string(),
            Some(JsonValue::String(s)) => s.clone(),
            Some(other) => {
                return Err(ToolError::InvalidArgs(format!(
                    "`arguments` must be an object, got {other}"
                )))
            }
        };
        let inner_call = ToolCall {
            id: call.id.clone(),
            kind: call.kind,
            function: mira_core::message::ToolCallFunction {
                name: tool.local_name.clone(),
                arguments,
            },
        };
        tool.invoke(&inner_call, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_survive_and_text_is_capped() {
        let blocks = vec![
            ContentBlock::text("hello"),
            ContentBlock::image("aGk=", "image/png"),
        ];
        let r = render_blocks(&blocks);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].media_type, "image/png");
        assert!(r.text.starts_with("hello\n[image image/png]"));

        let long = "é".repeat(100);
        let t = truncate(long, 51);
        assert!(t.starts_with(&"é".repeat(25)));
        assert!(t.contains("output truncated: 50 of 200"));
    }
}
