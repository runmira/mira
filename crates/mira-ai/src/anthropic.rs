//! Native Anthropic Messages API adapter.
//!
//! Talks to `POST /v1/messages` directly instead of going through the
//! OpenAI-compat shim. That buys us:
//!
//! * Real `cache_control` on system + tools (two breakpoints) with the
//!   cached-token count broken out of `usage`.
//! * Extended thinking (`thinking: { type: "enabled", budget_tokens }`)
//!   mapped from [`ChatRequest::reasoning_effort`].
//! * Anthropic-shaped tool use — `tool_use` / `tool_result` content
//!   blocks — so the model sees its own protocol, not OpenAI's function
//!   calling wrapped over the compat endpoint.
//!
//! The compat path in [`crate::openai`] still works for Anthropic if a
//! user points a `providers.anthropic-compat` entry at
//! `https://api.anthropic.com/v1`; this module is the default for the
//! `anthropic` provider name.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{stream::BoxStream, StreamExt};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Message, Role, ToolCall, ToolCallId};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use crate::event::{ChatEvent, FinishReason, TokenUsage};
use crate::provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError};
use crate::tool_spec::ToolSpec;

/// Wire version pinned to the current stable Messages API. Bumped only
/// when Anthropic ships a breaking change we've verified against.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fallback cap when the caller didn't set `max_tokens`. Anthropic
/// *requires* this field; picking 4096 keeps chat turns comfortable
/// without capping obvious reasoning workloads.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Configuration for the native Anthropic client.
///
/// `base_url` should be `https://api.anthropic.com/v1` in production;
/// tests point it at a local mock. It must not carry a trailing slash and
/// must not include the `/messages` path — the client appends it.
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub api_key: String,
    /// Extra headers — mostly for gateways / proxies. Callers should not
    /// set `x-api-key`, `anthropic-version`, or `content-type` here; the
    /// client writes those itself.
    pub extra_headers: Vec<(String, String)>,
    /// Attach `cache_control: {type: "ephemeral"}` to the first system
    /// block and the final tool spec. Two breakpoints cover the fat
    /// static prefix (system + tool schemas) — the biggest per-turn
    /// slice — while leaving room for callers who want to place their
    /// own cache_control elsewhere later.
    pub prompt_caching: bool,
}

pub struct Anthropic {
    http: reqwest::Client,
    cfg: AnthropicConfig,
}

impl Anthropic {
    pub fn new(cfg: AnthropicConfig) -> Result<Self, ProviderError> {
        if cfg.base_url.is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }
        if cfg.api_key.is_empty() {
            return Err(ProviderError::Config("api_key is empty".into()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, cfg })
    }

    fn headers(&self) -> Result<HeaderMap, ProviderError> {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(
            "x-api-key",
            HeaderValue::from_str(&self.cfg.api_key)
                .map_err(|e| ProviderError::Config(e.to_string()))?,
        );
        h.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        for (k, v) in &self.cfg.extra_headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
            let val = HeaderValue::from_str(v).map_err(|e| ProviderError::Config(e.to_string()))?;
            h.insert(name, val);
        }
        Ok(h)
    }
}

#[async_trait]
impl ChatProvider for Anthropic {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let url = format!("{}/models", self.cfg.base_url.trim_end_matches('/'));
        let resp = self.http.get(&url).headers(self.headers()?).send().await?;
        // Anthropic added GET /v1/models in 2024. Older gateways may still
        // 404 — treat that as an empty catalog so `mira serve` doesn't
        // 500 out on a stale proxy.
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Status { status, body });
        }
        let raw: ModelListResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        Ok(raw.data.into_iter().map(Into::into).collect())
    }

    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        if request.response_format.is_some() {
            // Anthropic doesn't have OpenAI's response_format field.
            // Rather than silently dropping the schema, log — callers
            // that need strict JSON should use tool_use-based extraction
            // (see mira-agents' Extractor). This warning fires once per
            // turn, which is loud enough to notice in a REPL.
            warn!(
                "anthropic: response_format is not supported natively; the field is ignored. \
                 Use a tool-based extractor for strict structured output."
            );
        }
        let body = WireRequest::build(&request, self.cfg.prompt_caching)?;
        let url = format!("{}/messages", self.cfg.base_url.trim_end_matches('/'));

        debug!(model = %request.model, tools = request.tools.len(), "anthropic messages request");

        let resp = self
            .http
            .post(&url)
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Status { status, body });
        }

        let (tx, rx) = mpsc::channel::<Result<ChatEvent, ProviderError>>(64);

        tokio::spawn(async move {
            let mut sse = Box::pin(resp.bytes_stream().eventsource());
            let mut state = StreamState::default();

            while let Some(item) = sse.next().await {
                let evt = match item {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(Err(ProviderError::Decode(e.to_string()))).await;
                        return;
                    }
                };

                // Anthropic's stream uses named events (`event: name\n
                // data: {...}`). eventsource-stream surfaces them via
                // `evt.event`; the OpenAI-compat path only sees `message`
                // events, so we dispatch on the name here instead of
                // guessing at data shape.
                match evt.event.as_str() {
                    "message_start" => {
                        if let Ok(v) = serde_json::from_str::<MessageStart>(&evt.data) {
                            state.record_input_usage(&v.message.usage);
                        }
                    }
                    "content_block_start" => {
                        if let Ok(v) = serde_json::from_str::<ContentBlockStart>(&evt.data) {
                            state.begin_block(v.index, v.content_block);
                        }
                    }
                    "content_block_delta" => {
                        if let Ok(v) = serde_json::from_str::<ContentBlockDelta>(&evt.data) {
                            if let Some(evt) = state.push_delta(v.index, v.delta) {
                                if tx.send(Ok(evt)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    "content_block_stop" => {
                        // Nothing to emit — tool_use blocks flush on the
                        // trailing message_delta once we know the turn is
                        // ending. This keeps ordering deterministic:
                        // TextDelta*, ToolCalls, Usage, Done.
                    }
                    "message_delta" => {
                        if let Ok(v) = serde_json::from_str::<MessageDelta>(&evt.data) {
                            state.record_output_usage(&v.usage);
                            if let Some(stop) = v.delta.stop_reason.as_deref() {
                                state.stop = Some(map_stop_reason(stop));
                            }
                        }
                    }
                    "message_stop" => {
                        break;
                    }
                    "error" => {
                        // Anthropic sends structured errors mid-stream
                        // (rate-limit, overloaded). Surface as a decode
                        // error so the harness marks the turn failed.
                        let msg = serde_json::from_str::<ErrorEvent>(&evt.data)
                            .map(|e| format!("{}: {}", e.error.kind, e.error.message))
                            .unwrap_or_else(|_| evt.data.clone());
                        let _ = tx.send(Err(ProviderError::Status {
                            status: 200,
                            body: msg,
                        })).await;
                        return;
                    }
                    "ping" | "" => {
                        // Keepalives — SSE-comment style. Skip.
                    }
                    other => {
                        debug!(kind = %other, "anthropic: unknown stream event; ignoring");
                    }
                }
            }

            let tool_calls = state.take_tool_calls();
            if !tool_calls.is_empty()
                && tx.send(Ok(ChatEvent::ToolCalls(tool_calls))).await.is_err()
            {
                return;
            }

            if let Some(usage) = state.take_usage() {
                if tx.send(Ok(ChatEvent::Usage(usage))).await.is_err() {
                    return;
                }
            }

            let reason = state.stop.unwrap_or(FinishReason::Stop);
            let _ = tx.send(Ok(ChatEvent::Done(reason))).await;
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

fn map_stop_reason(raw: &str) -> FinishReason {
    match raw {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" => FinishReason::Length,
        _ => FinishReason::Other,
    }
}

// ---------- stream reassembly ----------

/// Per-turn state the SSE reader accumulates. Anthropic reports usage in
/// two halves (input on `message_start`, output on `message_delta`) and
/// tool_use blocks stream a name + id up front and JSON fragments after;
/// this struct keeps everything consistent so the emitted `ChatEvent`s
/// are well-ordered.
#[derive(Default)]
struct StreamState {
    /// Content blocks indexed by their `index` field. Sparse — text and
    /// tool_use blocks can interleave with arbitrary indices.
    blocks: Vec<Option<StreamBlock>>,
    /// Prompt tokens from `message_start`. `None` until first seen.
    prompt_tokens: Option<u32>,
    /// Cached-input tokens (read from the prompt cache). Anthropic
    /// reports this separately from the "creation" number.
    cached_input_tokens: u32,
    /// Completion tokens from `message_delta`.
    completion_tokens: u32,
    /// Whether we saw any usage numbers at all. Determines whether we
    /// emit `Usage` (skip when the provider never sent one — kept
    /// consistent with the OpenAI-compat path).
    usage_seen: bool,
    stop: Option<FinishReason>,
}

enum StreamBlock {
    Text,
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
}

impl StreamState {
    fn begin_block(&mut self, index: usize, block: WireContentBlockStart) {
        if self.blocks.len() <= index {
            self.blocks.resize_with(index + 1, || None);
        }
        self.blocks[index] = Some(match block {
            WireContentBlockStart::Text { .. } => StreamBlock::Text,
            WireContentBlockStart::ToolUse { id, name, .. } => StreamBlock::ToolUse {
                id,
                name,
                input: String::new(),
            },
            WireContentBlockStart::Other => StreamBlock::Text,
        });
    }

    fn push_delta(&mut self, index: usize, delta: WireBlockDelta) -> Option<ChatEvent> {
        let slot = self.blocks.get_mut(index)?.as_mut()?;
        match (slot, delta) {
            (StreamBlock::Text, WireBlockDelta::Text { text }) => {
                if text.is_empty() {
                    None
                } else {
                    Some(ChatEvent::TextDelta(text))
                }
            }
            (StreamBlock::ToolUse { input, .. }, WireBlockDelta::InputJson { partial_json }) => {
                input.push_str(&partial_json);
                None
            }
            _ => None,
        }
    }

    fn record_input_usage(&mut self, u: &WireUsage) {
        // input_tokens on Anthropic *excludes* cached-read tokens.
        // TokenUsage.prompt_tokens is defined as the total, with
        // cached_input_tokens a subset — reconstruct that shape.
        let cached = u.cache_read_input_tokens.unwrap_or(0);
        let input = u.input_tokens.unwrap_or(0);
        self.prompt_tokens = Some(input + cached);
        self.cached_input_tokens = cached;
        self.usage_seen |= u.input_tokens.is_some()
            || u.cache_read_input_tokens.is_some()
            || u.cache_creation_input_tokens.is_some();
    }

    fn record_output_usage(&mut self, u: &WireUsage) {
        if let Some(t) = u.output_tokens {
            self.completion_tokens = t;
            self.usage_seen = true;
        }
    }

    fn take_tool_calls(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.blocks)
            .into_iter()
            .filter_map(|slot| match slot? {
                StreamBlock::ToolUse { id, name, input } => Some(ToolCall {
                    id: ToolCallId::from(id),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name,
                        arguments: if input.is_empty() {
                            "{}".to_owned()
                        } else {
                            input
                        },
                    },
                }),
                StreamBlock::Text => None,
            })
            .collect()
    }

    fn take_usage(&mut self) -> Option<TokenUsage> {
        if !self.usage_seen {
            return None;
        }
        Some(TokenUsage {
            prompt_tokens: self.prompt_tokens.unwrap_or(0),
            completion_tokens: self.completion_tokens,
            cached_input_tokens: self.cached_input_tokens,
        })
    }
}

// ---------- wire request ----------

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    /// Top-level system prompt. Either a bare string or an array of
    /// content blocks — the block form is only needed when we want to
    /// mark one with `cache_control`.
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<WireSystem<'a>>,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<WireThinking>,
    stream: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireSystem<'a> {
    Text(String),
    Blocks(Vec<WireSystemBlock<'a>>),
}

#[derive(Serialize)]
struct WireSystemBlock<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self { kind: "ephemeral" }
    }
}

#[derive(Serialize)]
struct WireThinking {
    #[serde(rename = "type")]
    kind: &'static str,
    budget_tokens: u32,
}

fn thinking_from_effort(effort: Option<&str>, max_tokens: u32) -> Option<WireThinking> {
    let budget = match effort? {
        // "off"/"minimal" both mean "don't turn thinking on" — the field
        // stays absent so we don't accidentally light up the surcharge
        // for a caller that turned reasoning off in the UI.
        "off" | "minimal" | "none" => return None,
        "low" => 1_024,
        "medium" => 4_096,
        "high" => 8_192,
        _ => return None,
    };
    // Anthropic requires budget_tokens < max_tokens. If a caller left
    // max_tokens tiny, cap the budget so the request still validates.
    let capped = budget.min(max_tokens.saturating_sub(1));
    if capped < 1024 {
        return None;
    }
    Some(WireThinking {
        kind: "enabled",
        budget_tokens: capped,
    })
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    content: Vec<WireContentBlock<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentBlock<'a> {
    Text {
        text: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Value,
    },
    ToolResult {
        tool_use_id: &'a str,
        content: &'a str,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Serialize)]
struct WireTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

impl<'a> WireRequest<'a> {
    fn build(req: &'a ChatRequest, prompt_caching: bool) -> Result<Self, ProviderError> {
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        let system = build_system(&req.messages, prompt_caching);
        let messages = build_messages(&req.messages)?;
        let tools = build_tools(&req.tools, prompt_caching);
        let thinking = thinking_from_effort(req.reasoning_effort.as_deref(), max_tokens);

        Ok(Self {
            model: &req.model,
            max_tokens,
            system,
            messages,
            tools,
            temperature: req.temperature,
            thinking,
            stream: true,
        })
    }
}

/// Hoist every `Role::System` message out of the message list and into
/// the top-level `system` field. When prompt caching is on, the *first*
/// non-empty system block gets a `cache_control: ephemeral` breakpoint
/// so Anthropic caches the fat static prefix (system + tools). Later
/// system messages — like the harness's live-memory block — stay
/// unmarked so their per-round churn doesn't invalidate that cache.
fn build_system<'a>(messages: &'a [Message], prompt_caching: bool) -> Option<WireSystem<'a>> {
    let systems: Vec<&'a str> = messages
        .iter()
        .filter(|m| matches!(m.role, Role::System))
        .filter_map(|m| m.content.as_deref())
        .filter(|s| !s.is_empty())
        .collect();
    if systems.is_empty() {
        return None;
    }
    if !prompt_caching {
        // Plain-string form — smaller wire, and the whole point of the
        // block form is the cache_control marker we're skipping here.
        return Some(WireSystem::Text(systems.join("\n\n")));
    }
    let mut blocks = Vec::with_capacity(systems.len());
    for (i, text) in systems.into_iter().enumerate() {
        blocks.push(WireSystemBlock {
            kind: "text",
            text,
            cache_control: if i == 0 {
                Some(CacheControl::ephemeral())
            } else {
                None
            },
        });
    }
    Some(WireSystem::Blocks(blocks))
}

/// Convert the flat OpenAI-style message list to Anthropic turns. System
/// messages are handled by [`build_system`] and skipped here. Tool
/// results (`Role::Tool`) collapse into a single user message with a
/// `tool_result` block per call — Anthropic requires them under user, not
/// their own role.
fn build_messages(messages: &[Message]) -> Result<Vec<WireMessage<'_>>, ProviderError> {
    let mut out: Vec<WireMessage<'_>> = Vec::new();
    // Track pending tool_result blocks that need to attach to the *next*
    // user message we emit. Anthropic requires: assistant(tool_use)
    // followed by user(tool_result) — a bare tool message can't stand
    // alone.
    let mut pending_tool_results: Vec<WireContentBlock<'_>> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {}
            Role::Tool => {
                let call_id = msg
                    .tool_call_id
                    .as_ref()
                    .ok_or_else(|| ProviderError::Config("tool message missing tool_call_id".into()))?;
                pending_tool_results.push(WireContentBlock::ToolResult {
                    tool_use_id: call_id.as_str(),
                    content: msg.content.as_deref().unwrap_or(""),
                    is_error: false,
                });
            }
            Role::User => {
                let mut content = std::mem::take(&mut pending_tool_results);
                if let Some(text) = msg.content.as_deref() {
                    if !text.is_empty() {
                        content.push(WireContentBlock::Text { text });
                    }
                }
                if content.is_empty() {
                    // Anthropic rejects empty content arrays; skip.
                    continue;
                }
                out.push(WireMessage {
                    role: "user",
                    content,
                });
            }
            Role::Assistant => {
                // Any tool_results buffered up to this point belong to
                // the *previous* assistant turn — flush them as a user
                // message before the new assistant message goes out.
                if !pending_tool_results.is_empty() {
                    out.push(WireMessage {
                        role: "user",
                        content: std::mem::take(&mut pending_tool_results),
                    });
                }
                let mut content: Vec<WireContentBlock<'_>> = Vec::new();
                if let Some(text) = msg.content.as_deref() {
                    if !text.is_empty() {
                        content.push(WireContentBlock::Text { text });
                    }
                }
                for call in &msg.tool_calls {
                    // Parse the model's original argument string back
                    // into a JSON object — Anthropic wants `input` as
                    // an object, not the stringified form OpenAI uses.
                    // Malformed JSON becomes an empty object rather than
                    // failing the whole request; the model will get a
                    // corrected value on the retry.
                    let input: Value = serde_json::from_str(&call.function.arguments)
                        .unwrap_or_else(|_| Value::Object(Default::default()));
                    content.push(WireContentBlock::ToolUse {
                        id: call.id.as_str(),
                        name: &call.function.name,
                        input,
                    });
                }
                if content.is_empty() {
                    continue;
                }
                out.push(WireMessage {
                    role: "assistant",
                    content,
                });
            }
        }
    }
    // Trailing tool_results (assistant tool_use as the last real turn):
    // append them as a final user message. This is the shape after the
    // harness dispatches tools before sending the next request.
    if !pending_tool_results.is_empty() {
        out.push(WireMessage {
            role: "user",
            content: pending_tool_results,
        });
    }
    Ok(out)
}

fn build_tools<'a>(tools: &'a [ToolSpec], prompt_caching: bool) -> Vec<WireTool<'a>> {
    let last = tools.len().saturating_sub(1);
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| WireTool {
            name: &t.name,
            description: &t.description,
            input_schema: &t.parameters,
            // Attach the second cache_control breakpoint to the *final*
            // tool. Anthropic caches everything up to (and including)
            // the marker — so one on the last tool covers all schemas
            // before it.
            cache_control: if prompt_caching && i == last {
                Some(CacheControl::ephemeral())
            } else {
                None
            },
        })
        .collect()
}

// ---------- wire response types ----------

#[derive(Deserialize)]
struct MessageStart {
    message: MessageStartInner,
}

#[derive(Deserialize)]
struct MessageStartInner {
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize)]
struct ContentBlockStart {
    index: usize,
    content_block: WireContentBlockStart,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentBlockStart {
    Text {
        #[serde(default)]
        #[allow(dead_code)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        #[allow(dead_code)]
        input: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ContentBlockDelta {
    index: usize,
    delta: WireBlockDelta,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireBlockDelta {
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(rename = "input_json_delta")]
    InputJson { partial_json: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct MessageDelta {
    delta: MessageDeltaInner,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize)]
struct MessageDeltaInner {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ErrorEvent {
    error: ErrorInner,
}

#[derive(Deserialize)]
struct ErrorInner {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

impl From<WireModel> for ModelInfo {
    fn from(w: WireModel) -> Self {
        ModelInfo {
            id: w.id,
            display_name: w.display_name,
            owned_by: Some("anthropic".to_owned()),
            context_length: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatRequest;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{Message, ToolCall, ToolCallId};

    fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from(id.to_owned()),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    fn req_with(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "claude-opus-4".into(),
            messages,
            tools: vec![],
            temperature: None,
            max_tokens: Some(2048),
            reasoning_effort: None,
            response_format: None,
        }
    }

    #[test]
    fn system_hoisted_to_top_level_and_removed_from_messages() {
        let req = req_with(vec![
            Message::system("SYS"),
            Message::user("hi"),
        ]);
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["system"], serde_json::json!("SYS"));
        let msgs = json["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn cache_control_only_on_first_system_when_caching_on() {
        let req = req_with(vec![
            Message::system("PREFIX"),
            Message::system("LIVE_MEM"),
            Message::user("hi"),
        ]);
        let body = WireRequest::build(&req, true).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let sys = json["system"].as_array().unwrap();
        assert_eq!(sys.len(), 2);
        assert_eq!(sys[0]["text"], "PREFIX");
        assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");
        assert!(sys[1].get("cache_control").is_none() || sys[1]["cache_control"].is_null());
    }

    #[test]
    fn caching_off_serializes_system_as_string() {
        let req = req_with(vec![
            Message::system("SYS"),
            Message::user("hi"),
        ]);
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert!(json["system"].is_string());
    }

    #[test]
    fn empty_system_produces_no_field() {
        let req = req_with(vec![Message::user("hi")]);
        let body = WireRequest::build(&req, true).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("system").is_none() || json["system"].is_null());
    }

    #[test]
    fn assistant_tool_calls_become_tool_use_blocks_with_parsed_input() {
        let mut assistant = Message::assistant("about to call");
        assistant.tool_calls = vec![tool_call("call_1", "ls", r#"{"path":"."}"#)];
        let req = req_with(vec![
            Message::system("SYS"),
            Message::user("list files"),
            assistant,
        ]);
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let msgs = json["messages"].as_array().unwrap();
        // 2 messages: user, assistant.
        assert_eq!(msgs.len(), 2);
        let a = &msgs[1];
        assert_eq!(a["role"], "assistant");
        let content = a["content"].as_array().unwrap();
        // text + tool_use
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "call_1");
        assert_eq!(content[1]["name"], "ls");
        // input is a JSON *object*, not the string form.
        assert_eq!(content[1]["input"], serde_json::json!({"path": "."}));
    }

    #[test]
    fn tool_messages_fold_into_next_user_turn() {
        let mut assistant = Message::assistant("");
        assistant.tool_calls = vec![tool_call("call_1", "ls", "{}")];
        let req = req_with(vec![
            Message::user("go"),
            assistant,
            Message::tool(ToolCallId::from("call_1".to_owned()), "file1\nfile2"),
            Message::user("thanks"),
        ]);
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let msgs = json["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
        let last_content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(last_content[0]["type"], "tool_result");
        assert_eq!(last_content[0]["tool_use_id"], "call_1");
        assert_eq!(last_content[0]["content"], "file1\nfile2");
        assert_eq!(last_content[1]["type"], "text");
        assert_eq!(last_content[1]["text"], "thanks");
    }

    #[test]
    fn trailing_tool_results_get_their_own_user_message() {
        // The shape right *before* we ask the model to continue after
        // tool calls: assistant(tool_use), tool, tool — no user turn
        // yet. Must produce a final user message with tool_result blocks.
        let mut assistant = Message::assistant("");
        assistant.tool_calls = vec![
            tool_call("c1", "ls", "{}"),
            tool_call("c2", "cat", r#"{"path":"a"}"#),
        ];
        let req = req_with(vec![
            Message::user("go"),
            assistant,
            Message::tool(ToolCallId::from("c1".to_owned()), "ok1"),
            Message::tool(ToolCallId::from("c2".to_owned()), "ok2"),
        ]);
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let msgs = json["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"], "user");
        let content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["tool_use_id"], "c1");
        assert_eq!(content[1]["tool_use_id"], "c2");
    }

    #[test]
    fn tools_get_cache_control_only_on_last_when_caching_on() {
        let req = ChatRequest {
            tools: vec![
                ToolSpec {
                    name: "a".into(),
                    description: "".into(),
                    parameters: serde_json::json!({}),
                },
                ToolSpec {
                    name: "b".into(),
                    description: "".into(),
                    parameters: serde_json::json!({}),
                },
            ],
            ..req_with(vec![Message::user("hi")])
        };
        let body = WireRequest::build(&req, true).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let tools = json["tools"].as_array().unwrap();
        assert!(tools[0].get("cache_control").is_none() || tools[0]["cache_control"].is_null());
        assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn reasoning_effort_off_omits_thinking() {
        let req = ChatRequest {
            reasoning_effort: Some("off".into()),
            ..req_with(vec![Message::user("hi")])
        };
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("thinking").is_none() || json["thinking"].is_null());
    }

    #[test]
    fn reasoning_effort_high_sets_thinking_budget() {
        let req = ChatRequest {
            reasoning_effort: Some("high".into()),
            max_tokens: Some(16_000),
            ..req_with(vec![Message::user("hi")])
        };
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"]["type"], "enabled");
        assert_eq!(json["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn max_tokens_defaults_when_missing() {
        let req = ChatRequest {
            max_tokens: None,
            ..req_with(vec![Message::user("hi")])
        };
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn thinking_budget_capped_below_max_tokens() {
        // budget must be strictly less than max_tokens; guard against
        // a caller that pairs a tiny cap with reasoning=high.
        let req = ChatRequest {
            reasoning_effort: Some("high".into()),
            max_tokens: Some(2000),
            ..req_with(vec![Message::user("hi")])
        };
        let body = WireRequest::build(&req, false).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let budget = json["thinking"]["budget_tokens"].as_u64().unwrap();
        assert!(budget < 2000);
    }

    // ----- streaming reassembly -----

    #[tokio::test]
    async fn stream_reassembles_text_tool_use_and_usage() {
        use crate::event::ChatEvent;
        use futures::StreamExt;

        // Canned Anthropic SSE stream: text + one tool_use.
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":40,\"cache_creation_input_tokens\":0}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"ls\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"p\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ath\\\":\\\"/\\\"}\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        // Feed the canned stream directly through the same SSE +
        // dispatch pipeline used by the real provider. We can't stand
        // up an HTTP server in a unit test cheaply, so build the
        // reader-side loop by hand using the same StreamState logic.
        let mut sse = Box::pin(
            futures::stream::once(async move { Ok::<_, std::io::Error>(bytes::Bytes::from(body)) })
                .eventsource(),
        );
        let mut state = StreamState::default();
        let mut events: Vec<ChatEvent> = Vec::new();

        while let Some(item) = sse.next().await {
            let evt = item.unwrap();
            match evt.event.as_str() {
                "message_start" => {
                    let v: MessageStart = serde_json::from_str(&evt.data).unwrap();
                    state.record_input_usage(&v.message.usage);
                }
                "content_block_start" => {
                    let v: ContentBlockStart = serde_json::from_str(&evt.data).unwrap();
                    state.begin_block(v.index, v.content_block);
                }
                "content_block_delta" => {
                    let v: ContentBlockDelta = serde_json::from_str(&evt.data).unwrap();
                    if let Some(e) = state.push_delta(v.index, v.delta) {
                        events.push(e);
                    }
                }
                "message_delta" => {
                    let v: MessageDelta = serde_json::from_str(&evt.data).unwrap();
                    state.record_output_usage(&v.usage);
                    if let Some(sr) = v.delta.stop_reason.as_deref() {
                        state.stop = Some(map_stop_reason(sr));
                    }
                }
                "message_stop" => break,
                _ => {}
            }
        }

        let tool_calls = state.take_tool_calls();
        events.push(ChatEvent::ToolCalls(tool_calls));
        if let Some(u) = state.take_usage() {
            events.push(ChatEvent::Usage(u));
        }
        events.push(ChatEvent::Done(state.stop.unwrap_or(FinishReason::Stop)));

        // Expect: TextDelta("Hel"), TextDelta("lo"), ToolCalls([ls(...)]),
        // Usage{140/20/40}, Done(ToolCalls).
        assert!(matches!(&events[0], ChatEvent::TextDelta(t) if t == "Hel"));
        assert!(matches!(&events[1], ChatEvent::TextDelta(t) if t == "lo"));
        match &events[2] {
            ChatEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].function.name, "ls");
                assert_eq!(calls[0].id.as_str(), "toolu_1");
                let parsed: Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
                assert_eq!(parsed, serde_json::json!({"path": "/"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
        match &events[3] {
            ChatEvent::Usage(u) => {
                assert_eq!(u.prompt_tokens, 140);
                assert_eq!(u.completion_tokens, 20);
                assert_eq!(u.cached_input_tokens, 40);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        assert!(matches!(&events[4], ChatEvent::Done(FinishReason::ToolCalls)));
    }
}
