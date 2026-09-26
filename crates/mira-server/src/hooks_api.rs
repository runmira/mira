//! `/api/hooks`: the user's hooks (`hooks:` in `~/.mira/mira.yaml`) as a
//! flat list the Settings page can edit, plus read-only plugin hooks.
//!
//! - `GET` lists them.
//! - `PUT {rules}` replaces the user's hooks, after checking they load,
//!   and reloads so they apply right away.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::MiraConfig;
use mira_plugins::hooks::{config_from_rules, rules_from_config, HookRule, HookSet};
use serde::{Deserialize, Serialize};

use crate::mcp::error_json;
use crate::state::AppState;

#[derive(Serialize)]
pub struct HooksView {
    /// Yours, from `mira.yaml`, in order.
    rules: Vec<HookRule>,
    /// From enabled plugins; shown, not editable here.
    plugin_rules: Vec<PluginRule>,
    /// Hooks that couldn't be loaded, with where they came from.
    problems: Vec<String>,
    /// `macos`, `linux` or `windows`, for the notification preset.
    os: &'static str,
}

#[derive(Serialize)]
struct PluginRule {
    plugin: String,
    #[serde(flatten)]
    rule: HookRule,
}

#[derive(Deserialize)]
pub struct SaveHooks {
    rules: Vec<HookRule>,
}

pub async fn get_hooks(State(state): State<AppState>) -> Response {
    match view(&state) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn put_hooks(State(state): State<AppState>, Json(req): Json<SaveHooks>) -> Response {
    let rules: Vec<HookRule> = req.rules.into_iter().map(tidy).collect();
    let config = config_from_rules(&rules);
    // Refuse what wouldn't load, with the reason, instead of saving it.
    if let Some(c) = &config {
        let mut check = HookSet::default();
        check.add(c, None, "hook");
        if let Some(p) = check.problems.first() {
            return error_json(StatusCode::BAD_REQUEST, p.clone());
        }
    }
    let mut cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => return error_json(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };
    cfg.hooks = config;
    if let Err(e) = cfg.save_global() {
        return error_json(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
    }
    crate::mcp::changed(&state).await;
    get_hooks(State(state)).await
}

fn view(state: &AppState) -> Result<HooksView, String> {
    let cfg = MiraConfig::load_global().map_err(|e| format!("{e:#}"))?;
    Ok(HooksView {
        rules: cfg
            .hooks
            .as_ref()
            .map(rules_from_config)
            .unwrap_or_default(),
        plugin_rules: state
            .extensions
            .plugin_hook_rules()
            .into_iter()
            .map(|(plugin, rule)| PluginRule { plugin, rule })
            .collect(),
        problems: state.extensions.hook_problems(),
        os: std::env::consts::OS,
    })
}

/// Trim text and drop empty fields, so a blank box means "not set".
fn tidy(mut r: HookRule) -> HookRule {
    let clean = |v: Option<String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    r.event = r.event.trim().to_owned();
    r.matcher = clean(r.matcher);
    r.command = clean(r.command);
    r.prompt = clean(r.prompt);
    r.timeout = r.timeout.filter(|t| *t > 0.0);
    r
}
