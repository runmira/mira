//! A `ChatProvider` that routes by engine instance.
//!
//! The harness holds `Arc<dyn ChatProvider>` once and never asks which
//! backend is behind it. To let the user switch engines mid-session
//! without rebuilding sessions, the server wraps every configured
//! native provider in a [`SwappableProvider`]: a pool keyed by instance
//! id plus an active pointer. `stream()` resolves the pointer on every
//! call, so a selection change is an `Arc` swap — in-flight streams
//! keep the delegate they started with, and new turns run on whoever
//! was just picked.
//!
//! History never mentions instances: it is provider-neutral, so the
//! same conversation can flow through OpenRouter at turn 4 and
//! Anthropic at turn 5 with no conversion pass.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::stream::BoxStream;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, ModelInfo, ProviderError};

#[derive(Clone, Default)]
pub struct SwappableProvider {
    pool: Arc<RwLock<Pool>>,
}

#[derive(Default)]
struct Pool {
    /// The delegate every request goes to right now.
    active: Option<Arc<dyn ChatProvider>>,
    /// Providers registered per engine instance. The active pointer
    /// usually references one of these, but it may also stand alone —
    /// the CLI builds the boot provider before instances are derived,
    /// and that provider is active even when its instance id is not
    /// known to the server.
    entries: HashMap<String, Arc<dyn ChatProvider>>,
}

impl SwappableProvider {
    /// Start with one active delegate and an empty instance pool. The
    /// delegate is the CLI-built boot provider; the pool fills in from
    /// config as instances register.
    pub fn new(initial: Arc<dyn ChatProvider>) -> Self {
        Self {
            pool: Arc::new(RwLock::new(Pool {
                active: Some(initial),
                entries: HashMap::new(),
            })),
        }
    }

    /// Replace the active delegate — the settings hot-swap path for the
    /// *currently selected* instance. When the active delegate belongs
    /// to a registered instance, its pool row is refreshed too, so the
    /// rebuilt provider and the pool stay the same object.
    pub fn set(&self, next: Arc<dyn ChatProvider>) {
        let mut pool = self.pool.write().expect("provider pool lock poisoned");
        if let Some(active) = &pool.active {
            if let Some(id) = pool
                .entries
                .iter()
                .find(|(_, p)| Arc::ptr_eq(p, active))
                .map(|(id, _)| id.clone())
            {
                pool.entries.insert(id, next.clone());
            }
        }
        pool.active = Some(next);
    }

    /// Register (or replace) the provider for one engine instance
    /// without activating it. Call `activate` to switch.
    pub fn register(&self, instance: &str, provider: Arc<dyn ChatProvider>) {
        self.pool
            .write()
            .expect("provider pool lock poisoned")
            .entries
            .insert(instance.to_string(), provider);
    }

    /// Point at a registered instance. Returns `false` when no
    /// provider is registered under that id — the caller keeps the
    /// previous selection and surfaces the reason itself.
    pub fn activate(&self, instance: &str) -> bool {
        let mut pool = self.pool.write().expect("provider pool lock poisoned");
        match pool.entries.get(instance) {
            Some(p) => {
                pool.active = Some(p.clone());
                true
            }
            None => false,
        }
    }

    /// The instance id the active delegate was registered under, when
    /// it was registered through [`Self::register`].
    pub fn active_instance(&self) -> Option<String> {
        let pool = self.pool.read().expect("provider pool lock poisoned");
        let active = pool.active.as_ref()?;
        pool.entries
            .iter()
            .find(|(_, p)| Arc::ptr_eq(p, active))
            .map(|(id, _)| id.clone())
    }
}

#[async_trait]
impl ChatProvider for SwappableProvider {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let delegate = {
            self.pool
                .read()
                .expect("provider pool lock poisoned")
                .active
                .clone()
        };
        // The active slot is only ever empty before `new` runs — which
        // the constructor makes impossible — but a `None` here should
        // fail the request rather than panic the harness.
        match delegate {
            Some(d) => d.stream(request).await,
            None => Err(ProviderError::Config(
                "no provider is configured — pick one in Settings".into(),
            )),
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let delegate = {
            self.pool
                .read()
                .expect("provider pool lock poisoned")
                .active
                .clone()
        };
        match delegate {
            Some(d) => d.list_models().await,
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_ai::NullProvider;

    fn provider() -> Arc<dyn ChatProvider> {
        Arc::new(NullProvider::default())
    }

    #[test]
    fn activate_switches_and_reports_the_active_instance() {
        let swappable = SwappableProvider::new(provider());
        assert_eq!(swappable.active_instance(), None, "boot delegate has no id yet");

        swappable.register("openrouter", provider());
        swappable.register("anthropic", provider());
        assert!(swappable.activate("anthropic"));
        assert_eq!(swappable.active_instance().as_deref(), Some("anthropic"));
        assert!(swappable.activate("openrouter"));
        assert_eq!(swappable.active_instance().as_deref(), Some("openrouter"));
        assert!(!swappable.activate("nope"), "unknown instance must be refused");
        // Refused activation keeps the old pointer.
        assert_eq!(swappable.active_instance().as_deref(), Some("openrouter"));
    }

    #[test]
    fn set_replaces_the_delegate_without_touching_the_pool() {
        let swappable = SwappableProvider::new(provider());
        swappable.register("groq", provider());
        swappable.activate("groq");
        swappable.set(provider());
        // The hot-swap is instance-agnostic; the pool keeps its rows.
        assert_eq!(swappable.active_instance().as_deref(), Some("groq"));
    }
}
