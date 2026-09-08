use async_trait::async_trait;
use futures::stream::BoxStream;
use mira_core::Message;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{event::ChatEvent, tool_spec::ToolSpec};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("provider returned {status}: {body}")]
    Status { status: u16, body: String },

    #[error("stream decode error: {0}")]
    Decode(String),

    #[error("bad configuration: {0}")]
    Config(String),
}

impl From<ProviderError> for mira_core::Error {
    fn from(e: ProviderError) -> Self {
        mira_core::Error::Provider(e.to_string())
    }
}

/// A single completion request.
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Reasoning effort (`"minimal" | "low" | "medium" | "high"`) — passed
    /// through to providers that expose it. `None` skips the field.
    pub reasoning_effort: Option<String>,
    /// Constrain the response shape. When set, the provider is told to
    /// emit JSON matching the schema (OpenAI's `response_format:
    /// json_schema` mode). Callers that want typed results out of a
    /// subagent set this; leave `None` for freeform prose.
    pub response_format: Option<ResponseFormat>,
}

/// How the model's text output should be shaped.
///
/// Providers that don't support structured output silently ignore this
/// (or error at the wire level — the harness surfaces those). Currently
/// only the OpenAI-compatible provider wires it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Freeform JSON — any valid JSON object. Cheaper to enforce than a
    /// full schema; useful when the caller only needs "must be JSON".
    JsonObject,
    /// Strict JSON matching a JSON Schema. `name` is a short identifier
    /// the provider surfaces in errors. `strict: true` asks the provider
    /// to reject drift (unknown fields, missing required keys); providers
    /// that don't support strict fall back to best-effort.
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(default)]
        strict: bool,
    },
}

/// One entry in a provider's model catalog. `id` is the value you pass as
/// `ChatRequest::model`; the extras are metadata the UI can surface.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Provider-specific bucket (OpenRouter groups by "openai", "anthropic", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
}

/// Anything that can turn a `ChatRequest` into a stream of `ChatEvent`s.
///
/// Implementations own their HTTP client and auth. The harness holds this
/// behind `Arc<dyn ChatProvider>` so it can be swapped mid-session.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError>;

    /// Fetch the provider's model catalog. Default returns an empty list —
    /// providers that don't expose one (or don't want to) fall back to a
    /// text input on the UI side.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}
