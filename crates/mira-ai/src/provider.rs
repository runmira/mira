use async_trait::async_trait;
use futures::stream::BoxStream;
use mira_core::Message;
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
}
