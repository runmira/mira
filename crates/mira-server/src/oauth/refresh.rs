//! Background OAuth token refresh loop.
//!
//! Runs as a spawned tokio task for the server's lifetime. Every
//! [`REFRESH_INTERVAL`] it walks the token store for known
//! OAuth-managed providers (currently just `openai`) and, when a bundle
//! is within [`REFRESH_LEEWAY`] of expiry, uses its refresh token to
//! mint a new access + api-key pair, atomically rewrites the store,
//! updates `mira.yaml`, and hot-swaps the live [`super::super::provider::SwappableProvider`].
//!
//! Errors don't crash the server — they surface as warnings and the
//! loop retries on the next tick. If refresh fails long enough for the
//! token to actually expire, the next chat call gets a 401 and the
//! user can re-sign-in.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::state::AppState;

use super::openai;

/// How often to check for near-expiry tokens.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Refresh a bundle whose expiry is within this many seconds. 10
/// minutes gives real refresh two full intervals of runway before an
/// actual expiry could bite a live chat.
pub const REFRESH_LEEWAY: u64 = 10 * 60;

/// Spawn the refresh task. Returns immediately; the task holds an Arc
/// to `state` and lives as long as any strong reference to the server
/// state does — i.e., the lifetime of `mira serve`.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // First-tick delay so we don't hammer refresh in the first
        // second of boot (the on-boot rehydrate in `serve()` already
        // rebuilt the provider from disk if a valid bundle was there).
        tokio::time::sleep(Duration::from_secs(30)).await;
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&state).await {
                warn!(%e, "oauth refresh: tick failed");
            }
        }
    });
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    // OpenAI is the only OAuth-managed provider with refresh today.
    // OpenRouter's PKCE flow returns a durable API key with no
    // refresh — nothing to do there.
    if let Some(bundle) = super::store::load("openai")? {
        if bundle.is_near_expiry(REFRESH_LEEWAY) {
            debug!(
                expires_at = bundle.expires_at,
                "openai token near expiry — refreshing"
            );
            match openai::refresh_and_persist(state).await {
                Ok(_) => info!("openai token refreshed successfully"),
                Err(e) => warn!(%e, "openai refresh failed; will retry next tick"),
            }
        }
    }
    Ok(())
}

/// Convenience wrapper so `serve()` can rehydrate on boot without
/// duplicating the "load store → check expiry → refresh if needed →
/// hot-swap provider" recipe.
pub async fn boot_rehydrate(state: &AppState) {
    // Fire once to hot-swap the provider off a stored bundle (or
    // refresh it first if it's already stale) so the very first user
    // turn after `mira serve` uses the persistent OAuth key instead of
    // whatever fallback was in yaml.
    if let Err(e) = tick(state).await {
        warn!(%e, "oauth refresh: boot rehydrate failed");
    }
    // Also hot-swap the live provider off whatever's in yaml (which
    // the callback / refresh may have just updated). Uses the shared
    // `build_provider_from` recipe.
    let _ = state; // ARM the borrow so a future scope-only fn can be added
    match mira_config::MiraConfig::load_global() {
        Ok(cfg) => {
            let provider = crate::settings::build_provider_from(&cfg);
            state.provider.set(provider);
            crate::models::invalidate();
        }
        Err(e) => warn!(%e, "oauth refresh: couldn't reload config"),
    }
}

// Silence unused-warning on RwLock import when the file is compiled with
// only the tokio::sync::RwLock re-export path.
#[allow(dead_code)]
fn _unused<T>(_: Arc<RwLock<T>>) {}
