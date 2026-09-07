//! Settings API — read/write `~/.mira/mira.yaml` from the browser.
//!
//! `GET /api/settings` returns the current global config with API keys
//! masked. `PUT /api/settings` accepts a partial update, writes it to
//! disk, and hot-swaps the running session's provider and model.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_ai::{ChatProvider, NullProvider};
use mira_config::{default_base_url_for, global_path, MiraConfig, RuntimeState};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::protocol::ServerMsg;
use crate::state::AppState;

/// What the UI shows for the current settings. API keys are masked so the
/// UI can indicate "a key is set" without ever handling the raw value.
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_mode: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub providers: Vec<ProviderView>,
    /// True when a provider is actually usable — base_url + api_key present.
    pub configured: bool,
    pub config_path: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderView {
    pub name: String,
    pub base_url: Option<String>,
    pub api_key_masked: Option<String>,
    /// True if a raw `api_key` (not env-var-driven) is stored.
    pub has_api_key: bool,
    pub api_key_env: Option<String>,
}

/// What the UI POSTs. The clearable fields use double-`Option` so we can
/// distinguish "field absent → don't touch" from "field set to null →
/// clear this value". `providers` entries follow the same rule per-field.
#[derive(Debug, Deserialize)]
pub struct SettingsUpdate {
    #[serde(default, deserialize_with = "double_option")]
    pub default_provider: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_mode: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub max_tokens: Option<Option<u32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub temperature: Option<Option<f32>>,
    #[serde(default)]
    pub providers: Vec<ProviderUpdate>,
}

/// Distinguishes "key absent" (returns `None`) from "key present with null"
/// (returns `Some(None)`) so the settings PUT can express both "leave this
/// alone" and "clear this value" over JSON.
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct ProviderUpdate {
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    /// New literal key. `Some("")` clears; `None` leaves whatever's stored.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

pub async fn get_settings(State(_state): State<AppState>) -> Response {
    match MiraConfig::load_global() {
        Ok(cfg) => {
            let view = view_from(&cfg, is_configured(&cfg));
            Json(view).into_response()
        }
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load config: {e}"),
        ),
    }
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(update): Json<SettingsUpdate>,
) -> Response {
    let mut cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load config: {e}"),
            )
        }
    };
    apply(&mut cfg, update);

    if let Err(e) = cfg.save_global() {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        );
    }
    info!(path = %global_path().display(), "settings saved");

    // Rebuild the provider from the fresh config and swap it into the live
    // session. If the config still isn't complete, fall back to NullProvider
    // so the next turn produces a helpful warning instead of the last known
    // (possibly bad) provider silently continuing.
    let provider = build_provider(&cfg);
    state.provider.set(provider);
    // Drop the models cache — the new provider has a different catalog.
    crate::models::invalidate();

    if let Some(model) = cfg.default_model.clone() {
        state.current_session().await.set_model(&model).await;
        // Mirror the WS SetModel path — persist so restarts remember.
        let mut s = RuntimeState::load().unwrap_or_default();
        s.last_model = Some(model.clone());
        if let Err(e) = s.save() {
            warn!(%e, "state.yaml: save failed after settings change");
        }
        let _ = state.events_tx.send(ServerMsg::ModelChanged { model });
    }

    let view = view_from(&cfg, is_configured(&cfg));
    Json(view).into_response()
}

fn view_from(cfg: &MiraConfig, configured: bool) -> SettingsView {
    let providers = cfg
        .providers
        .iter()
        .map(|(name, p)| ProviderView {
            name: name.clone(),
            base_url: p
                .base_url
                .clone()
                .or_else(|| default_base_url_for(name).map(|s| s.to_owned())),
            api_key_masked: p.api_key.as_deref().map(mask_key),
            has_api_key: p.api_key.is_some(),
            api_key_env: p.api_key_env.clone(),
        })
        .collect();
    SettingsView {
        default_provider: cfg.default_provider.clone(),
        default_model: cfg.default_model.clone(),
        default_mode: cfg.default_mode.clone(),
        max_tokens: cfg.max_tokens,
        temperature: cfg.temperature,
        providers,
        configured,
        config_path: global_path().display().to_string(),
    }
}

fn apply(cfg: &mut MiraConfig, u: SettingsUpdate) {
    if let Some(v) = u.default_provider {
        cfg.default_provider = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = u.default_model {
        cfg.default_model = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = u.default_mode {
        cfg.default_mode = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = u.max_tokens {
        cfg.max_tokens = v;
    }
    if let Some(v) = u.temperature {
        cfg.temperature = v;
    }
    for pu in u.providers {
        let entry = cfg.providers.entry(pu.name).or_default();
        if let Some(base_url) = pu.base_url {
            entry.base_url = if base_url.is_empty() {
                None
            } else {
                Some(base_url)
            };
        }
        if let Some(key) = pu.api_key {
            entry.api_key = if key.is_empty() { None } else { Some(key) };
        }
        if let Some(env_name) = pu.api_key_env {
            entry.api_key_env = if env_name.is_empty() {
                None
            } else {
                Some(env_name)
            };
        }
    }
}

/// Try to build a real provider from `cfg`. On any missing piece, fall back
/// to `NullProvider` so the server keeps running and the next chat call
/// produces a clear "configure me" error.
fn build_provider(cfg: &MiraConfig) -> Arc<dyn ChatProvider> {
    let Some(name) = cfg.default_provider.as_deref() else {
        return Arc::new(NullProvider::default());
    };
    let entry = cfg.providers.get(name).cloned().unwrap_or_default();
    let Some(base_url) = entry
        .base_url
        .clone()
        .or_else(|| default_base_url_for(name).map(|s| s.to_owned()))
    else {
        return Arc::new(NullProvider::new(format!(
            "provider `{name}` has no base_url"
        )));
    };
    let Some(api_key) = entry.resolved_api_key() else {
        return Arc::new(NullProvider::new(format!(
            "provider `{name}` has no api_key"
        )));
    };
    let extra_headers = entry
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let prompt_caching = mira_config::prompt_caching_enabled(name, &base_url, entry.prompt_caching);
    match OpenAiCompatible::new(OpenAiConfig {
        base_url,
        api_key,
        extra_headers,
        prompt_caching,
    }) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            warn!(%e, "settings: provider build failed, falling back to null");
            Arc::new(NullProvider::new(format!("provider build failed: {e}")))
        }
    }
}

/// True when the default provider has both a base_url (or a known default)
/// and an api_key resolvable through the same lookup as the real provider
/// build.
fn is_configured(cfg: &MiraConfig) -> bool {
    let Some(name) = cfg.default_provider.as_deref() else {
        return false;
    };
    let entry = cfg.providers.get(name).cloned().unwrap_or_default();
    let base_url_ok = entry.base_url.is_some() || default_base_url_for(name).is_some();
    let key_ok = entry.resolved_api_key().is_some();
    base_url_ok && key_ok
}

fn mask_key(k: &str) -> String {
    let n = k.chars().count();
    if n <= 8 {
        return "*".repeat(n);
    }
    let head: String = k.chars().take(4).collect();
    let tail: String = k
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}
