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
///
/// Non-goals for now: response format, seeds, logprobs. Add them behind
/// `Option` fields when a caller actually needs them — no speculative surface.
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
