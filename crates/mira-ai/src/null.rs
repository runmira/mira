//! A placeholder provider used when no real one is configured yet.
//!
//! `mira serve` boots with this so the UI can render and prompt for
//! settings before the user has picked a provider. Every [`stream`] call
//! resolves to a `ProviderError::Config` — the harness reports the error
//! as a warning and returns to the user.
//!
//! [`stream`]: crate::ChatProvider::stream

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::{ChatEvent, ChatProvider, ChatRequest, ProviderError};

pub struct NullProvider {
    reason: String,
}

impl NullProvider {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Default for NullProvider {
    fn default() -> Self {
        Self::new("no provider configured — open Settings to add one")
    }
}

#[async_trait]
impl ChatProvider for NullProvider {
    async fn stream(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        Err(ProviderError::Config(self.reason.clone()))
    }
}
