//! `GET /api/models` — the provider's model catalog.
//!
//! Proxies through the live [`SwappableProvider`] so the list always
//! matches whichever provider settings the user last saved. The response
//! is cached for `TTL` so opening the picker repeatedly doesn't hit the
//! upstream every time; the cache is dropped whenever the provider is
//! swapped (see `settings::put_settings`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_ai::ModelInfo;
use once_cell::sync::Lazy;
use serde::Serialize;

use crate::state::AppState;

const TTL: Duration = Duration::from_secs(300);

struct CacheSlot {
    at: Instant,
    models: Vec<ModelInfo>,
}

static CACHE: Lazy<Mutex<Option<CacheSlot>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Serialize)]
pub struct ModelListView {
    pub models: Vec<ModelInfo>,
    pub cached: bool,
}

pub async fn list_models(State(state): State<AppState>) -> Response {
    if let Some(hit) = read_cache() {
        return Json(ModelListView {
            models: hit,
            cached: true,
        })
        .into_response();
    }

    let provider = state.harness_provider.clone();
    match provider.list_models().await {
        Ok(mut models) => {
            models.sort_by_key(|m| m.id.to_lowercase());
            write_cache(&models);
            Json(ModelListView {
                models,
                cached: false,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("upstream: {e}") })),
        )
            .into_response(),
    }
}

/// Called by the settings handler when the user swaps providers — otherwise
/// the cache would serve stale entries from the old endpoint.
pub fn invalidate() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

fn read_cache() -> Option<Vec<ModelInfo>> {
    let guard = CACHE.lock().ok()?;
    let slot = guard.as_ref()?;
    if slot.at.elapsed() < TTL {
        Some(slot.models.clone())
    } else {
        None
    }
}

fn write_cache(models: &[ModelInfo]) {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(CacheSlot {
            at: Instant::now(),
            models: models.to_vec(),
        });
    }
}
