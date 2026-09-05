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

use crate::event::{ChatEvent, FinishReason, ToolCallBuffer};
use crate::provider::{ChatProvider, ChatRequest, ProviderError};
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
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let body = WireRequest::from_request(&request);
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
    stream: bool,
}

impl<'a> WireRequest<'a> {
    fn from_request(req: &'a ChatRequest) -> Self {
        Self {
            model: &req.model,
            messages: req.messages.iter().map(WireMessage::from).collect(),
            tools: req.tools.iter().map(WireTool::from).collect(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            stream: true,
        }
    }
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

impl<'a> From<&'a Message> for WireMessage<'a> {
    fn from(m: &'a Message) -> Self {
        Self {
            role: match m.role {
                mira_core::Role::System => "system",
                mira_core::Role::User => "user",
                mira_core::Role::Assistant => "assistant",
                mira_core::Role::Tool => "tool",
            },
            content: m.content.as_deref(),
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
    choices: Vec<WireChoice>,
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
