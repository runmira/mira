//! Settings API — read/write `~/.mira/mira.yaml` from the browser.
//!
//! `GET /api/settings` returns the current global config with API keys
//! masked. `PUT /api/settings` accepts a partial update, writes it to
//! disk, and hot-swaps the running session's provider and model.
//!
//! ## Provider lifecycle
//!
//! This module is the bridge between the user-facing settings UI and the
//! AI provider system. When a user changes their provider/model via
//! `PUT /api/settings`:
//! 1. The new config is written to `~/.mira/mira.yaml` (persisted).
//! 2. A new [`ChatProvider`] is built via `build_chat_provider` (from
//!    `mira-ai::factory`) — this is the factory that dispatches to the
//!    right adapter based on provider name (`anthropic` → native Messages
//!    API, `bedrock` → Converse API, everything else → OpenAI-compatible).
//! 3. The provider is hot-swapped into the running session's
//!    `current_session()`, so existing conversations continue with the
//!    new provider without restarting the server.
//! 4. If the new provider is misconfigured (missing `base_url` or
//!    `api_key`), `NullProvider` is used as a graceful fallback — the
//!    server keeps running and the user gets an actionable error message
//!    instead of a crash.
//!
//! ## Key masking
//!
//! API keys are masked (`mask_key`) before they ever leave the server, so
//! the UI can show "a key is set" without ever handling the raw value.
//! The canonical env-var name for each key is the map key itself
//! (`BRAVE_SEARCH_API_KEY`, `TAVILY_API_KEY`, …).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
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
    /// Cheap model for titles, summaries and `model: small` agents.
    pub small_model: Option<String>,
    pub default_mode: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub providers: Vec<ProviderView>,
    /// Third-party service keys (search backends etc.). Masked. The
    /// canonical env-var name for each key is the map key itself
    /// (`BRAVE_SEARCH_API_KEY`, `TAVILY_API_KEY`, …).
    pub keys: Vec<KeyView>,
    /// True when a provider is actually usable — base_url + api_key present.
    pub configured: bool,
    pub config_path: String,
    /// Cross-session memory runtime knobs. Kept in the same view so a UI
    /// panel can render all memory-related controls without a second fetch.
    pub memory: MemoryView,
}

/// Effective (defaults applied) view of `memory.*` for the UI. Every field
/// carries the value the harness would use *now*, not the raw `Option`
/// from yaml — the panel wants concrete on/off, not "unset means true."
#[derive(Debug, Serialize)]
pub struct MemoryView {
    pub auto_extract: bool,
    pub tools_enabled: bool,
    pub inject_context: bool,
    pub extractor_model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct KeyView {
    pub name: String,
    pub masked: String,
    /// True when the same-named var is set in the shell env — helps the
    /// UI show "using env value" vs "using yaml value" precedence.
    pub from_env: bool,
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
    pub small_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_mode: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub max_tokens: Option<Option<u32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub temperature: Option<Option<f32>>,
    #[serde(default)]
    pub providers: Vec<ProviderUpdate>,
    /// Third-party keys. Empty string clears; absence leaves untouched.
    #[serde(default)]
    pub keys: Vec<KeyUpdate>,
    /// Memory-runtime patch. Absent field = leave untouched; present with
    /// `None` = reset to default (removes the yaml override).
    #[serde(default)]
    pub memory: Option<MemoryUpdate>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryUpdate {
    /// Same double-Option semantics as elsewhere: absent = leave alone,
    /// present-with-null = reset to default, present-with-value = set.
    #[serde(default, deserialize_with = "double_option")]
    pub auto_extract: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option")]
    pub tools_enabled: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option")]
    pub inject_context: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option")]
    pub extractor_model: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct KeyUpdate {
    pub name: String,
    /// New value. `Some("")` clears; `None` leaves whatever's stored.
    #[serde(default)]
    pub value: Option<String>,
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
    // First-run needs the explicit `default_model` from this update to
    // reach the live session; captured before `apply` consumes it.
    let explicit_model = update.default_model.clone().flatten();
    apply(&mut cfg, update);

    if let Err(e) = cfg.save_global() {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        );
    }
    info!(path = %global_path().display(), "settings saved");

    // Push any newly-added search / third-party keys into the running
    // process env so tools that check std::env pick them up on the very
    // next call — no restart needed.
    mira_config::export_keys_to_env(&cfg);

    // Rebuild the default provider from the fresh config and refresh
    // the pool row for it. If the config still isn't complete, fall
    // back to NullProvider so the next turn produces a helpful warning
    // instead of the last known (possibly bad) provider silently
    // continuing.
    let provider = build_provider(&cfg);
    if let Some(name) = cfg.default_provider.clone() {
        state.provider.register(&name, provider);
        // Selecting a provider in settings is also a selection of the
        // engine instance that wraps it.
        state.provider.activate(&name);
        {
            let mut inst = state
                .selection
                .instance
                .write()
                .expect("selection lock poisoned");
            *inst = Some(name);
        }
    } else {
        state.provider.set(provider);
    }
    // Drop the models cache — the active provider has a different
    // catalog now.
    crate::models::invalidate();

    // New sessions read it from the config; the shared selection feeds
    // subagent defaults, so it updates too.
    state
        .current_session()
        .await
        .set_small_model(cfg.small_model.clone())
        .await;
    {
        let mut small = state
            .selection
            .small_model
            .write()
            .expect("selection lock poisoned");
        *small = cfg.small_model.clone().filter(|m| !m.trim().is_empty());
    }

    // Deliberately NOT force-resetting the session's model here. This
    // handler used to set_model(default_model) on every save, clobbering
    // whatever the user had picked in the picker. The model now changes
    // through `SetModel` (picker) or when this update explicitly sets
    // `default_model` — first-run needs the second path.
    if let Some(model) = explicit_model {
        let model = model.trim();
        if !model.is_empty() {
            state.current_session().await.set_model(model).await;
            {
                let mut m = state
                    .selection
                    .model
                    .write()
                    .expect("selection lock poisoned");
                *m = Some(model.to_owned());
            }
            let mut s = RuntimeState::load().unwrap_or_default();
            s.last_model = Some(model.to_owned());
            if let Err(e) = s.save() {
                warn!(%e, "state.yaml: save failed after settings change");
            }
            let _ = state
                .events_tx()
                .await
                .send(ServerMsg::ModelChanged {
                    model: model.to_owned(),
                    instance: state
                        .selection
                        .instance
                        .read()
                        .expect("selection lock poisoned")
                        .clone(),
                });
        }
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
    // Merge yaml-stored keys with any that only exist in the shell env, so
    // the UI shows the complete set of what tools would actually see.
    let mut keys: Vec<KeyView> = cfg
        .keys
        .iter()
        .map(|(name, value)| KeyView {
            name: name.clone(),
            masked: if value.is_empty() {
                String::new()
            } else {
                mask_key(value)
            },
            from_env: std::env::var_os(name).is_some(),
        })
        .collect();
    // Include known well-known keys even when unset so the UI can prompt
    // the user to add one. Extend this list as we add tools that need
    // third-party API keys.
    for well_known in [
        "BRAVE_SEARCH_API_KEY",
        "TAVILY_API_KEY",
        "GITHUB_TOKEN",
        "SLACK_BOT_TOKEN",
        "SLACK_APP_TOKEN",
    ] {
        if !keys.iter().any(|k| k.name == well_known) {
            keys.push(KeyView {
                name: well_known.to_owned(),
                masked: String::new(),
                from_env: std::env::var_os(well_known).is_some(),
            });
        }
    }
    keys.sort_by(|a, b| a.name.cmp(&b.name));

    SettingsView {
        default_provider: cfg.default_provider.clone(),
        default_model: cfg.default_model.clone(),
        small_model: cfg.small_model.clone(),
        default_mode: cfg.default_mode.clone(),
        max_tokens: cfg.max_tokens,
        temperature: cfg.temperature,
        providers,
        keys,
        configured,
        config_path: global_path().display().to_string(),
        memory: MemoryView {
            auto_extract: cfg.memory.auto_extract_enabled(),
            tools_enabled: cfg.memory.tools_enabled(),
            inject_context: cfg.memory.inject_context(),
            extractor_model: cfg.memory.extractor_model().map(str::to_owned),
        },
    }
}

fn apply(cfg: &mut MiraConfig, u: SettingsUpdate) {
    if let Some(v) = u.default_provider {
        cfg.default_provider = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = u.default_model {
        cfg.default_model = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = u.small_model {
        cfg.small_model = v.filter(|s| !s.trim().is_empty());
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
    for ku in u.keys {
        if let Some(value) = ku.value {
            if value.is_empty() {
                cfg.keys.remove(&ku.name);
            } else {
                cfg.keys.insert(ku.name, value);
            }
        }
    }
    if let Some(mu) = u.memory {
        if let Some(v) = mu.auto_extract {
            cfg.memory.auto_extract = v;
        }
        if let Some(v) = mu.tools_enabled {
            cfg.memory.tools_enabled = v;
        }
        if let Some(v) = mu.inject_context {
            cfg.memory.inject_context = v;
        }
        if let Some(v) = mu.extractor_model {
            cfg.memory.extractor_model = v.filter(|s| !s.is_empty());
        }
    }
}

/// Public alias so other modules (e.g. the OAuth callback) can hot-swap
/// the live provider without duplicating this recipe. Kept behind the
/// same name for callers inside settings.rs itself.
pub fn build_provider_from(cfg: &MiraConfig) -> Arc<dyn ChatProvider> {
    build_provider(cfg)
}

/// Build a provider for the config's default provider — the shared
/// recipe from `mira-engine`, with this module's fail-soft policy on
/// top (`NullProvider` keeps the server usable; the error message
/// surfaces on the next turn instead of failing the save).
fn build_provider(cfg: &MiraConfig) -> Arc<dyn ChatProvider> {
    let Some(name) = cfg.default_provider.as_deref() else {
        return Arc::new(NullProvider::default());
    };
    match mira_engine::native::build_native_provider(cfg, name) {
        Ok(p) => p,
        Err(mira_engine::EngineState::NotConfigured { reason }) => {
            Arc::new(NullProvider::new(reason))
        }
        Err(other) => {
            warn!(state = ?other, "settings: provider build failed, falling back to null");
            Arc::new(NullProvider::new(format!("provider build failed: {other:?}")))
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
    let key_ok = entry.resolved_api_key().is_some() || name == "bedrock";
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
