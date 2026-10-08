//! `GET/PUT /api/engines/:instance/settings` — an external agent's setup,
//! stored in `~/.mira/mira.yaml` under `engines.<instance>`.
//!
//! The browser used to keep this (API key included) in `localStorage` and
//! send it with every start (#79). Now the server is the only place an
//! agent's key lives: reads return whether one is set and a masked copy,
//! never the key, and the same goes for environment values whose names
//! look secret. Writes merge into the engine entry and rebuild the engine
//! registry at once, so the next start uses them.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::state::AppState;

/// One `env` entry. Secret-looking values come back masked, not in full.
#[derive(Debug, Serialize, PartialEq)]
pub struct EnvVarView {
    pub key: String,
    /// The value, for entries that don't look secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A masked copy, for entries that do (`sk-a…wxyz`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masked: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentSettingsView {
    pub instance: String,
    pub driver: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
    pub launch_args: Vec<String>,
    pub env: Vec<EnvVarView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setting_sources: Option<String>,
    /// True when a literal `api_key` is stored.
    pub has_api_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_masked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// The variables this agent reads a key from (`ANTHROPIC_API_KEY`, …).
    /// Empty for agents that only sign in through their own CLI.
    pub api_key_vars: Vec<String>,
}

/// A partial update. Absent fields are left alone; an empty string clears
/// a text field.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsPatch {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub binary_path: Option<String>,
    pub home_path: Option<String>,
    pub launch_args: Option<Vec<String>>,
    /// The whole environment. A `null` value keeps that variable's stored
    /// value, which is how a masked secret survives an edit of the others.
    pub env: Option<BTreeMap<String, Option<String>>>,
    pub effort: Option<String>,
    pub setting_sources: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
}

/// Env names whose values are treated as secrets. The same rule the
/// launcher uses to redact the command it logs.
fn looks_secret(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("token") || l.contains("key") || l.contains("secret") || l.contains("password")
}

fn mask(k: &str) -> String {
    let n = k.chars().count();
    if n <= 8 {
        return "•".repeat(n.max(4));
    }
    let head: String = k.chars().take(4).collect();
    let tail: String = k.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

fn text(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Build the view from an instance's driver, entry flags and config blob.
fn view(
    instance: &str,
    driver: &str,
    display_name: Option<String>,
    enabled: bool,
    cfg: &Map<String, Value>,
) -> AgentSettingsView {
    let env = cfg
        .get("env")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v)))
                .map(|(k, v)| {
                    if looks_secret(k) {
                        EnvVarView {
                            key: k.clone(),
                            value: None,
                            masked: Some(mask(v)),
                        }
                    } else {
                        EnvVarView {
                            key: k.clone(),
                            value: Some(v.to_string()),
                            masked: None,
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let api_key = text(cfg, "api_key");
    AgentSettingsView {
        instance: instance.to_string(),
        driver: driver.to_string(),
        display_name,
        enabled,
        binary_path: text(cfg, "binary_path"),
        home_path: text(cfg, "home_path"),
        launch_args: cfg
            .get("launch_args")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        env,
        effort: text(cfg, "effort"),
        setting_sources: text(cfg, "setting_sources"),
        has_api_key: api_key.is_some(),
        api_key_masked: api_key.as_deref().map(mask),
        api_key_env: text(cfg, "api_key_env"),
        api_key_vars: mira_acp::drivers::by_kind(driver)
            .map(|d| d.api_key_env_vars().iter().map(|v| v.to_string()).collect())
            .unwrap_or_default(),
    }
}

/// Merge `patch` into an engine entry's config blob.
fn apply(entry: &mut mira_config::EngineInstanceConfig, patch: AgentSettingsPatch) {
    let mut cfg = entry
        .config
        .take()
        .and_then(|v| match v {
            Value::Object(o) => Some(o),
            _ => None,
        })
        .unwrap_or_default();
    let set_text = |cfg: &mut Map<String, Value>, key: &str, v: Option<String>| {
        if let Some(v) = v {
            let v = v.trim().to_string();
            if v.is_empty() {
                cfg.remove(key);
            } else {
                cfg.insert(key.to_string(), Value::String(v));
            }
        }
    };
    if let Some(name) = patch.display_name {
        let name = name.trim().to_string();
        entry.display_name = (!name.is_empty()).then_some(name);
    }
    if let Some(on) = patch.enabled {
        // `true` is the default; only a switched-off agent needs a line.
        entry.enabled = (!on).then_some(false);
    }
    set_text(&mut cfg, "binary_path", patch.binary_path);
    set_text(&mut cfg, "home_path", patch.home_path);
    set_text(&mut cfg, "effort", patch.effort);
    set_text(&mut cfg, "setting_sources", patch.setting_sources);
    set_text(&mut cfg, "api_key", patch.api_key);
    set_text(&mut cfg, "api_key_env", patch.api_key_env);
    if let Some(args) = patch.launch_args {
        if args.is_empty() {
            cfg.remove("launch_args");
        } else {
            cfg.insert("launch_args".into(), json!(args));
        }
    }
    if let Some(env) = patch.env {
        let old = cfg
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let next: Map<String, Value> = env
            .into_iter()
            .filter(|(k, _)| !k.trim().is_empty())
            .filter_map(|(k, v)| match v {
                Some(v) => Some((k, Value::String(v))),
                // Keep the stored value; drop it if there was none.
                None => old.get(&k).cloned().map(|v| (k, v)),
            })
            .collect();
        if next.is_empty() {
            cfg.remove("env");
        } else {
            cfg.insert("env".into(), Value::Object(next));
        }
    }
    entry.config = (!cfg.is_empty()).then_some(Value::Object(cfg));
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// The external instance's driver, or the error to answer with.
fn external_driver(state: &AppState, instance: &str) -> Result<String, Response> {
    let engines = state.engines.current();
    match engines.get(instance) {
        Some(inst) if inst.is_native() => Err(error(
            StatusCode::BAD_REQUEST,
            format!("`{instance}` is a provider; its key is set under Settings → Provider"),
        )),
        Some(inst) => Ok(inst.driver.to_string()),
        None => Err(error(
            StatusCode::NOT_FOUND,
            format!("unknown engine instance `{instance}`"),
        )),
    }
}

fn current_view(state: &AppState, instance: &str, driver: &str) -> AgentSettingsView {
    let engines = state.engines.current();
    let inst = engines.get(instance);
    view(
        instance,
        driver,
        inst.and_then(|i| i.display_name.clone()),
        inst.is_none_or(|i| i.enabled),
        &inst.map(|i| i.config_object()).unwrap_or_default(),
    )
}

pub async fn get_settings(State(state): State<AppState>, Path(instance): Path<String>) -> Response {
    match external_driver(&state, &instance) {
        Ok(driver) => Json(current_view(&state, &instance, &driver)).into_response(),
        Err(e) => e,
    }
}

pub async fn put_settings(
    State(state): State<AppState>,
    Path(instance): Path<String>,
    Json(patch): Json<AgentSettingsPatch>,
) -> Response {
    let driver = match external_driver(&state, &instance) {
        Ok(d) => d,
        Err(e) => return e,
    };
    // One writer at a time: two quick saves must not interleave their
    // read-modify-write of the file.
    static WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = WRITE.lock().await;
    let mut cfg = match mira_config::MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("mira.yaml doesn't parse, so it can't be updated: {e:#}"),
            )
        }
    };
    let entry = cfg.engines.entry(instance.clone()).or_default();
    apply(entry, patch);
    // An override of a built-in agent with nothing left in it is no
    // override: drop the empty block rather than leave `codex: {}` behind.
    if entry.driver.is_none()
        && entry.display_name.is_none()
        && entry.enabled.is_none()
        && entry.model.is_none()
        && entry.config.is_none()
        && entry.extra.is_empty()
    {
        cfg.engines.remove(&instance);
    }
    if let Err(e) = cfg.save_global() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
    }
    // Rebuild now rather than waiting for the file watcher, so a start
    // right after saving uses the new settings.
    crate::engines_reload::reload(&state).await;
    crate::engines_api::refresh_cached_flags(&state, &instance);
    Json(current_view(&state, &instance, &driver)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(v: Value) -> mira_config::EngineInstanceConfig {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_view_never_carries_a_key_or_a_secret_env_value() {
        let e = entry(json!({ "config": {
            "api_key": "sk-ant-abcdefghijklmnop",
            "env": { "GITHUB_TOKEN": "ghp_1234567890abcd", "CODEX_PROFILE": "work" },
        }}));
        let v = view("codex", "codex", None, true, e.config.as_ref().unwrap().as_object().unwrap());
        let body = serde_json::to_string(&v).unwrap();
        assert!(!body.contains("sk-ant-abcdefghijklmnop"));
        assert!(!body.contains("ghp_1234567890abcd"));
        assert!(v.has_api_key);
        assert_eq!(v.api_key_masked.as_deref(), Some("sk-a…mnop"));
        assert!(v.env.contains(&EnvVarView {
            key: "CODEX_PROFILE".into(),
            value: Some("work".into()),
            masked: None,
        }));
        assert!(v.env.iter().any(|e| e.key == "GITHUB_TOKEN" && e.value.is_none()));
    }

    #[test]
    fn a_null_env_value_keeps_the_stored_secret() {
        let mut e = entry(json!({ "config": { "env": { "GITHUB_TOKEN": "ghp_secret", "OLD": "x" } } }));
        apply(
            &mut e,
            AgentSettingsPatch {
                env: Some(BTreeMap::from([
                    ("GITHUB_TOKEN".to_string(), None),
                    ("NEW".to_string(), Some("y".to_string())),
                ])),
                ..Default::default()
            },
        );
        let env = e.config.unwrap()["env"].clone();
        assert_eq!(env, json!({ "GITHUB_TOKEN": "ghp_secret", "NEW": "y" }));
    }

    #[test]
    fn empty_strings_clear_and_absent_fields_stay() {
        let mut e = entry(json!({ "config": { "api_key": "sk-old", "home_path": "~/.codex-work" } }));
        apply(
            &mut e,
            AgentSettingsPatch {
                api_key: Some(String::new()),
                enabled: Some(false),
                ..Default::default()
            },
        );
        let cfg = e.config.unwrap();
        assert!(cfg.get("api_key").is_none());
        assert_eq!(cfg["home_path"], "~/.codex-work");
        assert_eq!(e.enabled, Some(false));

        let mut e = entry(json!({ "enabled": false }));
        apply(
            &mut e,
            AgentSettingsPatch {
                enabled: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(e.enabled, None, "on is the default, so the line goes");
    }
}
