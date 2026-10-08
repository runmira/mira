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
//!
//! A request's model may also name its instance, `instance:model`
//! (`groq:llama-3.1-8b-instant`): background jobs configured that way
//! (compaction, titles, memory extraction, goal checks) run on that
//! instance whatever the chat is on. A prefix that isn't a known instance
//! is part of the model id (`llama3:8b`), and a known instance that can't
//! serve (no key) degrades to the active provider with a warning.

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
    /// Every configured instance id, buildable or not, so `x:model` can
    /// tell "instance x" from a model id that contains a colon.
    known: std::collections::HashSet<String>,
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
                known: Default::default(),
            })),
        }
    }

    /// A session owns its active pointer. Entries are shared by Arc, while
    /// selecting one in a sibling session cannot redirect this session.
    pub fn fork(&self, instance: Option<&str>) -> Self {
        let pool = self.pool.read().expect("provider pool lock poisoned");
        let active = match instance {
            Some(id) => pool.entries.get(id).cloned(),
            None => pool.active.clone(),
        };
        Self {
            pool: Arc::new(RwLock::new(Pool {
                active,
                entries: pool.entries.clone(),
                known: pool.known.clone(),
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

    /// Drop an instance's provider (its config was removed). The active
    /// pointer is untouched: a turn already on it finishes there.
    pub fn unregister(&self, instance: &str) {
        self.pool
            .write()
            .expect("provider pool lock poisoned")
            .entries
            .remove(instance);
    }

    /// Record every configured instance id (see the module docs).
    pub fn set_known_instances(&self, ids: impl IntoIterator<Item = String>) {
        self.pool
            .write()
            .expect("provider pool lock poisoned")
            .known = ids.into_iter().collect();
    }

    /// Where a request goes: the instance its model names, or the active
    /// delegate. Returns the request with any `instance:` prefix removed.
    fn route(&self, mut request: ChatRequest) -> (Option<Arc<dyn ChatProvider>>, ChatRequest) {
        let pool = self.pool.read().expect("provider pool lock poisoned");
        if let Some((instance, model)) = request.model.split_once(':') {
            if let Some(p) = pool.entries.get(instance) {
                request.model = model.to_string();
                return (Some(p.clone()), request);
            }
            if pool.known.contains(instance) {
                tracing::warn!(
                    %instance,
                    "background model names engine `{instance}`, which can't serve right now; using the active provider"
                );
                request.model = model.to_string();
            }
        }
        (pool.active.clone(), request)
    }

    /// Ids of every registered instance.
    pub fn instances(&self) -> Vec<String> {
        self.pool
            .read()
            .expect("provider pool lock poisoned")
            .entries
            .keys()
            .cloned()
            .collect()
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
        let (delegate, request) = self.route(request);
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

    fn request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            service_tier: None,
            response_format: None,
        }
    }

    #[test]
    fn instance_prefixed_models_route_to_that_instance() {
        let active = provider();
        let groq = provider();
        let pool = SwappableProvider::new(active.clone());
        pool.register("groq", groq.clone());
        pool.set_known_instances(["groq".to_string(), "anthropic-work".to_string()]);

        // A background job pinned to groq runs there, with the bare model id.
        let (p, r) = pool.route(request("groq:llama-3.1-8b-instant"));
        assert!(Arc::ptr_eq(&p.unwrap(), &groq));
        assert_eq!(r.model, "llama-3.1-8b-instant");
        // A colon inside a model id isn't an instance.
        let (p, r) = pool.route(request("llama3:8b"));
        assert!(Arc::ptr_eq(&p.unwrap(), &active));
        assert_eq!(r.model, "llama3:8b");
        // A configured instance that can't serve degrades to the active one.
        let (p, r) = pool.route(request("anthropic-work:claude-haiku-4-5"));
        assert!(Arc::ptr_eq(&p.unwrap(), &active));
        assert_eq!(r.model, "claude-haiku-4-5");
        // Forks (each chat's pool) keep the routing.
        let (p, _) = pool.fork(None).route(request("groq:x"));
        assert!(Arc::ptr_eq(&p.unwrap(), &groq));
    }

    #[test]
    fn session_selection_does_not_redirect_siblings_or_boot_defaults() {
        let pool = SwappableProvider::new(provider());
        pool.register("a", provider());
        pool.register("b", provider());
        pool.activate("a");
        let a = pool.fork(Some("a"));
        let b = pool.fork(Some("b"));
        pool.activate("b");
        assert_eq!(a.active_instance().as_deref(), Some("a"));
        assert_eq!(b.active_instance().as_deref(), Some("b"));
        a.activate("b");
        b.activate("a");
        assert_eq!(a.active_instance().as_deref(), Some("b"));
        assert_eq!(b.active_instance().as_deref(), Some("a"));
        assert!(pool.fork(Some("missing")).active_instance().is_none());
    }
    #[test]
    fn activate_switches_and_reports_the_active_instance() {
        let swappable = SwappableProvider::new(provider());
        assert_eq!(
            swappable.active_instance(),
            None,
            "boot delegate has no id yet"
        );

        swappable.register("openrouter", provider());
        swappable.register("anthropic", provider());
        assert!(swappable.activate("anthropic"));
        assert_eq!(swappable.active_instance().as_deref(), Some("anthropic"));
        assert!(swappable.activate("openrouter"));
        assert_eq!(swappable.active_instance().as_deref(), Some("openrouter"));
        assert!(
            !swappable.activate("nope"),
            "unknown instance must be refused"
        );
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
