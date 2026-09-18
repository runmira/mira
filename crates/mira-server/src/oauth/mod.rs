//! Browser-driven OAuth sign-in flows for external providers.
//!
//! Each supported provider has its own submodule (`openrouter`,
//! `openai`) but they share the PKCE primitives + token store + wire
//! helpers in the `mira-auth` crate. The server exposes a
//! `POST /api/auth/<p>/start` endpoint that returns an authorize URL
//! the UI opens in a new tab, and a `GET /api/auth/<p>/callback`
//! endpoint the provider redirects back to. On success the resulting
//! API key is written into the user's `mira.yaml` under
//! `providers.<p>.api_key` so the existing settings +
//! provider-hot-swap machinery reuses it without any special-casing.
//!
//! Nothing in this module owns provider-specific business logic beyond
//! the request/response shape and the "hot-swap the live provider"
//! server-only bit — pure request/response adapters live in
//! `mira_auth::providers`.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

pub mod openai;
pub mod openrouter;
pub mod refresh;

/// A PKCE code verifier stashed server-side while the browser round-
/// trips the authorization request. Keyed by an opaque `flow_id` we
/// return to the UI and expect back as the `state` query parameter on
/// the callback — same value on both sides is the CSRF guard.
///
/// Verifiers self-expire after ~10 minutes (matches OpenRouter's own
/// code TTL). A background sweep could enforce that; for now stale
/// entries just accumulate until server restart — cheap because each
/// entry is a ~128-byte string.
#[derive(Debug, Clone)]
pub struct PendingFlow {
    pub verifier: String,
    pub provider: String,
    pub started_at: std::time::Instant,
}

/// Shared map wrapped so both start and callback handlers see the same
/// state. Uses a tokio Mutex because both handlers are `async`.
pub type PendingFlowStore = Arc<Mutex<HashMap<String, PendingFlow>>>;

pub fn new_pending_store() -> PendingFlowStore {
    Arc::new(Mutex::new(HashMap::new()))
}
