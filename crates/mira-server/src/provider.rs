//! A `ChatProvider` whose inner delegate can be swapped at runtime.
//!
//! The harness holds `Arc<dyn ChatProvider>` once and never asks for it
//! again. To let the settings UI change providers mid-session without
//! rebuilding the whole session, we wrap the real provider in a
//! [`SwappableProvider`] whose `stream()` looks up the current delegate
//! on every call.
//!
//! In-flight streams keep using the delegate they started with — the swap
//! only affects new turns.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::stream::BoxStream;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, ModelInfo, ProviderError};

#[derive(Clone)]
pub struct SwappableProvider {
    inner: Arc<RwLock<Arc<dyn ChatProvider>>>,
}

impl SwappableProvider {
    pub fn new(initial: Arc<dyn ChatProvider>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial)),
        }
    }

    /// Replace the delegate. Fast — just an `Arc` swap.
    pub fn set(&self, next: Arc<dyn ChatProvider>) {
        *self.inner.write().expect("provider lock poisoned") = next;
    }
}

#[async_trait]
impl ChatProvider for SwappableProvider {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let delegate = { self.inner.read().expect("provider lock poisoned").clone() };
        delegate.stream(request).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let delegate = { self.inner.read().expect("provider lock poisoned").clone() };
        delegate.list_models().await
    }
}
