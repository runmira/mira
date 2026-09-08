//! OpenAI-compatible streaming client.
//!
//! Targets the `/chat/completions` endpoint as defined by OpenAI. Every
//! provider that mirrors this schema (OpenRouter, Groq, Together, Anthropic's
//! compat endpoint, llama.cpp server, LM Studio, Ollama `/v1`) works through
//! this single implementation.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{stream::BoxStream, StreamExt};
use mira_core::message::ToolCallKind;
use mira_core::{Message, ToolCall, ToolCallId};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use crate::event::{ChatEvent, FinishReason, TokenUsage, ToolCallBuffer};
use crate::provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError};
use crate::tool_spec::ToolSpec;

/// Configuration for an OpenAI-compatible endpoint.
///
/// `base_url` must not carry a trailing slash and must not include the
/// `/chat/completions` path — the client appends it.
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: String,
    /// Extra headers some providers require (e.g. OpenRouter's `HTTP-Referer`).
    pub extra_headers: Vec<(String, String)>,
    /// Attach `cache_control: {type: "ephemeral"}` to the system message so
    /// providers that support prompt caching (Anthropic via its OpenAI-compat
    /// endpoint) cache the fat static prefix — system prompt + tool schemas.
    ///
    /// Providers that don't understand the field silently ignore it, so leaving
    /// this on isn't dangerous, but the wire body switches to the richer
    /// content-blocks shape which a strict OpenAI-only server *could* reject.
    /// Callers gate this to known-supporting providers.
    pub prompt_caching: bool,
}

pub struct OpenAiCompatible {
    http: reqwest::Client,
    cfg: OpenAiConfig,
}

impl OpenAiCompatible {
    pub fn new(cfg: OpenAiConfig) -> Result<Self, ProviderError> {
        if cfg.base_url.is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, cfg })
    }

    fn headers(&self) -> Result<HeaderMap, ProviderError> {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let auth = format!("Bearer {}", self.cfg.api_key);
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth).map_err(|e| ProviderError::Config(e.to_string()))?,
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
impl ChatProvider for OpenAiCompatible {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let url = format!("{}/models", self.cfg.base_url.trim_end_matches('/'));
        let resp = self.http.get(&url).headers(self.headers()?).send().await?;
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
        let body = WireRequest::from_request(&request, self.cfg.prompt_caching);
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );

        debug!(model = %request.model, tools = request.tools.len(), "chat request");

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

        // Bounded channel — backpressure the network reader if the consumer
        // (the harness) is slow to drain events.
        let (tx, rx) = mpsc::channel::<Result<ChatEvent, ProviderError>>(64);

        tokio::spawn(async move {
            // Pin on the heap so we don't need to reason about whether
            // reqwest's byte stream is Unpin — Pin<Box<_>> always is.
            let mut sse = Box::pin(resp.bytes_stream().eventsource());
            let mut buffer = ToolCallBuffer::default();
            let mut finish: Option<FinishReason> = None;

            while let Some(item) = sse.next().await {
                let evt = match item {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(Err(ProviderError::Decode(e.to_string()))).await;
                        return;
                    }
                };

                if evt.data == "[DONE]" {
                    break;
                }

                let chunk: WireChunk = match serde_json::from_str(&evt.data) {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(?e, data = %evt.data, "malformed stream chunk; skipping");
                        continue;
                    }
                };

                // Usage arrives on a trailer chunk with empty `choices`.
                // Emit it before we bail out on empty choices below.
                if let Some(u) = chunk.usage {
                    let cached = u
                        .prompt_tokens_details
                        .as_ref()
                        .map(|d| d.cached_tokens)
                        .unwrap_or(0);
                    let usage = TokenUsage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cached_input_tokens: cached,
                    };
                    if tx.send(Ok(ChatEvent::Usage(usage))).await.is_err() {
                        return;
                    }
                }

                let Some(choice) = chunk.choices.into_iter().next() else {
                    continue;
                };

                if let Some(reason) = choice.finish_reason.as_deref() {
                    finish = Some(FinishReason::from_wire(Some(reason)));
                }

                let delta = choice.delta;

                if let Some(calls) = delta.tool_calls {
                    for tc in calls {
                        buffer.push_delta(
                            tc.index.unwrap_or(0),
                            tc.id.as_deref(),
                            tc.function.as_ref().and_then(|f| f.name.as_deref()),
                            tc.function.as_ref().and_then(|f| f.arguments.as_deref()),
                        );
                    }
                    continue;
                }

                if let Some(text) = delta.content {
                    if !text.is_empty() && tx.send(Ok(ChatEvent::TextDelta(text))).await.is_err() {
                        return;
                    }
                }
            }

            if !buffer.is_empty() {
                let calls = buffer.take();
                if tx.send(Ok(ChatEvent::ToolCalls(calls))).await.is_err() {
                    return;
                }
            }

            let reason = finish.unwrap_or(FinishReason::Stop);
            let _ = tx.send(Ok(ChatEvent::Done(reason))).await;
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

// ---------- wire types ----------

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// OpenAI reasoning-effort passthrough. Providers that don't
    /// recognise it (Groq, OpenRouter for most models, Ollama, …) drop
    /// unknown fields silently, so it's safe to always send when set.
    /// `"off"` is stripped upstream so the field only appears when the
    /// user actually wants reasoning.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    stream: bool,
    /// Opt in to the OpenAI usage trailer on streaming responses. Providers
    /// that don't understand this field drop it (Ollama, some OpenRouter
    /// models) — the harness gracefully treats missing usage as zero.
    stream_options: StreamOptions,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

impl<'a> WireRequest<'a> {
    fn from_request(req: &'a ChatRequest, prompt_caching: bool) -> Self {
        // Strip `"off"` (Mira UI sentinel) so we send *no* field for it —
        // OpenAI rejects unknown values with a 400 rather than ignoring.
        let effort = req.reasoning_effort.as_deref().filter(|v| *v != "off");
        // Only the *first* system message gets the cache-control breakpoint.
        // The harness may inject a second system message (the live memory
        // block); leaving it unmarked keeps it out of the cached prefix so
        // mid-session edits don't invalidate what's cached upstream.
        let mut first_system_seen = false;
        let messages = req
            .messages
            .iter()
            .map(|m| {
                let is_first_system = matches!(m.role, mira_core::Role::System)
                    && !std::mem::replace(&mut first_system_seen, true);
                WireMessage::from_message(m, prompt_caching, is_first_system)
            })
            .collect();
        Self {
            model: &req.model,
            messages,
            tools: req.tools.iter().map(WireTool::from).collect(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            reasoning_effort: effort,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
        }
    }
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    /// Either a bare string (`"hi"`) or an array of content blocks with
    /// optional `cache_control` markers. Providers that don't understand the
    /// richer shape either accept it (OpenAI, most compat layers) or reject
    /// it (some strict local servers) — we only emit blocks when the caller
    /// asked for prompt caching.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<WireContent<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireContent<'a> {
    Text(&'a str),
    Blocks(Vec<WireContentBlock<'a>>),
}

#[derive(Serialize)]
struct WireContentBlock<'a> {
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

impl<'a> WireMessage<'a> {
    fn from_message(m: &'a Message, prompt_caching: bool, is_first_system: bool) -> Self {
        // Mark the (first) system prompt with cache_control when caching is on.
        // Anthropic caches the prefix up to (and including) this breakpoint —
        // which covers tools + system, the biggest static chunk of every turn.
        // A second system message (the harness's live memory block) is left
        // unmarked so its per-round churn doesn't invalidate the cache.
        let content = m.content.as_deref().map(|text| {
            if prompt_caching && is_first_system && !text.is_empty() {
                WireContent::Blocks(vec![WireContentBlock {
                    kind: "text",
                    text,
                    cache_control: Some(CacheControl::ephemeral()),
                }])
            } else {
                WireContent::Text(text)
            }
        });
        Self {
            role: match m.role {
                mira_core::Role::System => "system",
                mira_core::Role::User => "user",
                mira_core::Role::Assistant => "assistant",
                mira_core::Role::Tool => "tool",
            },
            content,
            tool_calls: m.tool_calls.iter().map(WireToolCall::from).collect(),
            tool_call_id: m.tool_call_id.as_ref().map(ToolCallId::as_str),
            name: m.name.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolFn<'a>,
}

#[derive(Serialize)]
struct WireToolFn<'a> {
    name: &'a str,
    arguments: &'a str,
}

impl<'a> From<&'a ToolCall> for WireToolCall<'a> {
    fn from(c: &'a ToolCall) -> Self {
        Self {
            id: c.id.as_str(),
            kind: match c.kind {
                ToolCallKind::Function => "function",
            },
            function: WireToolFn {
                name: &c.function.name,
                arguments: &c.function.arguments,
            },
        }
    }
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolSpec<'a>,
}

#[derive(Serialize)]
struct WireToolSpec<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

impl<'a> From<&'a ToolSpec> for WireTool<'a> {
    fn from(t: &'a ToolSpec) -> Self {
        Self {
            kind: "function",
            function: WireToolSpec {
                name: &t.name,
                description: &t.description,
                parameters: &t.parameters,
            },
        }
    }
}

#[derive(Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChoice>,
    /// Only present on the final usage-trailer chunk (empty `choices`) when
    /// the caller opted in via `stream_options.include_usage`.
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    /// OpenAI reports cached tokens under `prompt_tokens_details.cached_tokens`.
    /// Anthropic's compat layer surfaces the same shape. Missing on providers
    /// that don't support prompt caching.
    #[serde(default)]
    prompt_tokens_details: Option<WirePromptDetails>,
}

#[derive(Deserialize)]
struct WirePromptDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Deserialize)]
struct WireChoice {
    delta: WireDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(Deserialize)]
struct WireToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireToolFnDelta>,
}

#[derive(Deserialize)]
struct WireToolFnDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// `GET /v1/models` response shape. The OpenAI standard is
/// `{ data: [{ id, owned_by, ... }] }`. OpenRouter extends each entry with
/// `name` (display) and `context_length` — we pick those up when present.
#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
}

impl From<WireModel> for ModelInfo {
    fn from(w: WireModel) -> Self {
        ModelInfo {
            id: w.id,
            display_name: w.name,
            owned_by: w.owned_by,
            context_length: w.context_length,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatRequest;
    use mira_core::Message;

    fn base_req() -> ChatRequest {
        ChatRequest {
            model: "test".into(),
            messages: vec![Message::system("SYS"), Message::user("hello")],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
        }
    }

    #[test]
    fn prompt_caching_off_serializes_content_as_string() {
        let req = base_req();
        let wire = WireRequest::from_request(&req, false);
        let json = serde_json::to_string(&wire).unwrap();
        // System content stays a plain string; no cache_control anywhere.
        assert!(json.contains(r#""content":"SYS""#));
        assert!(!json.contains("cache_control"));
        assert!(!json.contains(r#""type":"text""#));
    }

    #[test]
    fn prompt_caching_on_marks_system_with_cache_control() {
        let req = base_req();
        let wire = WireRequest::from_request(&req, true);
        let json = serde_json::to_string(&wire).unwrap();
        // System becomes a content-block array with cache_control ephemeral.
        assert!(json.contains(r#""text":"SYS""#));
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
        // User message is unchanged — no caching, no blocks.
        assert!(json.contains(r#""content":"hello""#));
    }

    #[test]
    fn prompt_caching_skips_empty_system() {
        let mut req = base_req();
        req.messages[0] = Message::system("");
        let wire = WireRequest::from_request(&req, true);
        let json = serde_json::to_string(&wire).unwrap();
        // Empty system prompt would produce an invalid text block; leave it as
        // a bare "" so the provider handles it uniformly.
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn prompt_caching_marks_only_first_system_when_two_present() {
        // The harness injects a second system message per round (live memory
        // block). It MUST NOT get its own cache_control breakpoint — that
        // would spend another breakpoint on a value that varies turn to turn
        // and defeat the caching we're trying to protect.
        let req = ChatRequest {
            model: "test".into(),
            messages: vec![
                Message::system("PREFIX"),
                Message::system("LIVE_MEMORY"),
                Message::user("hello"),
            ],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
        };
        let wire = WireRequest::from_request(&req, true);
        let json = serde_json::to_string(&wire).unwrap();
        // PREFIX is a content-block array with cache_control.
        assert!(json.contains(r#""text":"PREFIX""#));
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
        // LIVE_MEMORY is a plain string, no cache_control.
        assert!(json.contains(r#""content":"LIVE_MEMORY""#));
        // Only one cache_control anywhere in the wire body.
        assert_eq!(json.matches("cache_control").count(), 1);
    }
}
