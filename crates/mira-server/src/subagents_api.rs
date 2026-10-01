//! Settings → Subagents: Mira's own helpers, editable from the app.
//!
//! Subagents are the specialists Mira's model hands work to mid-turn
//! (Scout explores, Iris reviews, Bolt codes, …). They are defined as agent
//! files — built in, from plugins, or in `~/.mira/agents` / the project's
//! `.mira/agents` — and this API is the in-app editor for them:
//!
//! * `GET    /api/subagents`        — every type, enabled or not, with where
//!   it comes from and the tools it could be given.
//! * `POST   /api/subagents`        — create a new one.
//! * `PUT    /api/subagents/:name`  — save edits.
//! * `DELETE /api/subagents/:name`  — remove a custom one, or reset a
//!   customized built-in back to its shipped version.
//!
//! Edits are written as agent files, so what the app saves is exactly what a
//! hand-written file would be. A built-in or plugin type is customized by
//! writing an override to `~/.mira/agents`; a project type is edited in place.
//! After every write the roster is reloaded and swapped into the live handle
//! every chat's delegation tool reads, so changes apply on the next
//! delegation — no restart.

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_agents::{AgentRegistry, AgentType, Face};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::AppState;

#[derive(Serialize)]
pub struct SubagentView {
    #[serde(flatten)]
    agent: AgentType,
    /// A shipped type of the same name exists, so deleting the override
    /// resets rather than removes.
    has_builtin: bool,
    /// Saved from the app or a hand-written file overrides the shipped one.
    customized: bool,
}

#[derive(Serialize)]
pub struct SubagentsView {
    subagents: Vec<SubagentView>,
    /// Tool names a subagent can be limited to.
    tools: Vec<String>,
}

/// The editable fields. Anything not sent keeps its current value.
#[derive(Deserialize)]
pub struct SubagentEdit {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    /// `null` (or absent) keeps; an empty list means "all of the parent's tools".
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    max_rounds: Option<usize>,
    #[serde(default)]
    worktree: Option<bool>,
    #[serde(default)]
    route_approvals_to_parent: Option<bool>,
    #[serde(default)]
    review_required: Option<bool>,
    #[serde(default)]
    parallel_safe: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    face: Option<Face>,
    /// The instructions — the file's body.
    #[serde(default)]
    instructions: Option<String>,
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn current(state: &AppState) -> std::sync::Arc<AgentRegistry> {
    state
        .agents_live
        .read()
        .map(|r| r.clone())
        .unwrap_or_else(|_| state.agents_registry.clone())
}

fn view(state: &AppState) -> SubagentsView {
    let reg = current(state);
    let builtins = mira_agents::builtin();
    let subagents = reg
        .types
        .values()
        .map(|t| {
            let has_builtin = builtins.get(&t.name).is_some();
            SubagentView {
                customized: has_builtin && t.source.as_deref() != Some("builtin"),
                has_builtin,
                agent: t.clone(),
            }
        })
        .collect();
    let mut tools: Vec<String> = state.base_registry.specs().into_iter().map(|s| s.name).collect();
    tools.sort();
    tools.dedup();
    SubagentsView { subagents, tools }
}

/// Reload every tier from disk and swap the result in.
async fn reload(state: &AppState) {
    let cwd = state.current_cwd().await;
    let reg = mira_agents::load_with_plugins(&cwd, &state.extensions.plugin_agent_files());
    if let Ok(mut live) = state.agents_live.write() {
        *live = std::sync::Arc::new(reg);
    }
}

/// Where an edit to `t` is saved: in place for a project type, otherwise
/// as an override in the user's agents directory.
async fn file_for(state: &AppState, t: &AgentType) -> Option<PathBuf> {
    if t.source.as_deref() == Some("project") {
        let cwd = state.current_cwd().await;
        return Some(cwd.join(".mira").join("agents").join(format!("{}.md", t.name)));
    }
    mira_agents::user_agents_dir().map(|d| d.join(format!("{}.md", t.name)))
}

fn apply(t: &mut AgentType, e: SubagentEdit) {
    let some_text = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(v) = e.display_name {
        t.display_name = some_text(Some(v));
    }
    if let Some(v) = e.description {
        t.description = v.trim().to_string();
    }
    if let Some(v) = e.category {
        t.category = some_text(Some(v));
    }
    if let Some(v) = e.tools {
        t.tools = (!v.is_empty()).then_some(v);
    }
    if let Some(v) = e.model {
        t.model = some_text(Some(v));
    }
    if e.max_rounds.is_some() {
        t.max_rounds = e.max_rounds.filter(|n| *n > 0);
    }
    if e.worktree.is_some() {
        t.worktree = e.worktree;
    }
    if e.route_approvals_to_parent.is_some() {
        t.route_approvals_to_parent = e.route_approvals_to_parent;
    }
    if e.review_required.is_some() {
        t.review_required = e.review_required;
    }
    if e.parallel_safe.is_some() {
        t.parallel_safe = e.parallel_safe;
    }
    if e.enabled.is_some() {
        t.enabled = e.enabled;
    }
    if let Some(f) = e.face {
        t.face = Some(f);
    }
    if let Some(v) = e.instructions {
        t.system_prompt_addendum = some_text(Some(v));
    }
}

async fn write(state: &AppState, t: &AgentType) -> Result<(), Response> {
    let Some(path) = file_for(state, t).await else {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "no home directory to save into"));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("create {}: {e}", dir.display())))?;
    }
    // Validate by parsing what is about to be written: a file the loader
    // would reject must never reach disk, or the subagent silently vanishes.
    let md = t.to_markdown();
    mira_agents::parse_agent_md(&md)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("that would not load: {e:#}")))?;
    std::fs::write(&path, md)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("write {}: {e}", path.display())))?;
    reload(state).await;
    Ok(())
}

pub async fn list(State(state): State<AppState>) -> Response {
    Json(view(&state)).into_response()
}

pub async fn update(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(edit): Json<SubagentEdit>,
) -> Response {
    let Some(mut t) = current(&state).get(&name).cloned() else {
        return err(StatusCode::NOT_FOUND, format!("no subagent named `{name}`"));
    };
    apply(&mut t, edit);
    if let Err(r) = write(&state, &t).await {
        return r;
    }
    Json(view(&state)).into_response()
}

pub async fn create(State(state): State<AppState>, Json(edit): Json<SubagentEdit>) -> Response {
    let name = edit.name.clone().unwrap_or_default().trim().to_ascii_lowercase();
    if !valid_name(&name) {
        return err(
            StatusCode::BAD_REQUEST,
            "the id must be 1–40 lowercase letters, digits, `-` or `_`",
        );
    }
    if current(&state).get(&name).is_some() {
        return err(StatusCode::CONFLICT, format!("`{name}` already exists"));
    }
    let mut t = AgentType {
        name: name.clone(),
        description: String::new(),
        source: Some("user".into()),
        ..Default::default()
    };
    apply(&mut t, edit);
    if t.description.is_empty() {
        return err(StatusCode::BAD_REQUEST, "say what it's for — the model picks subagents by their description");
    }
    if let Err(r) = write(&state, &t).await {
        return r;
    }
    Json(view(&state)).into_response()
}

pub async fn remove(State(state): State<AppState>, AxumPath(name): AxumPath<String>) -> Response {
    let Some(t) = current(&state).get(&name).cloned() else {
        return err(StatusCode::NOT_FOUND, format!("no subagent named `{name}`"));
    };
    match t.source.as_deref() {
        Some("builtin") => return err(StatusCode::BAD_REQUEST, "built-in subagents can be turned off, not deleted"),
        Some("plugin") => return err(StatusCode::BAD_REQUEST, "this one comes from a plugin — disable the plugin instead"),
        _ => {}
    }
    let Some(path) = file_for(&state, &t).await else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "no home directory");
    };
    if let Err(e) = std::fs::remove_file(&path) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("remove {}: {e}", path.display()));
    }
    reload(&state).await;
    Json(view(&state)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_identifier_shaped() {
        assert!(valid_name("scout-2"));
        assert!(!valid_name("Scout"));
        assert!(!valid_name("../etc"));
        assert!(!valid_name(""));
    }

    #[test]
    fn an_edit_changes_only_what_it_sends() {
        let mut t = mira_agents::builtin().get("reviewer").unwrap().clone();
        let before_tools = t.tools.clone();
        apply(
            &mut t,
            serde_json::from_value(json!({ "display_name": "Judge", "enabled": false })).unwrap(),
        );
        assert_eq!(t.display_name.as_deref(), Some("Judge"));
        assert_eq!(t.enabled, Some(false));
        assert_eq!(t.tools, before_tools, "unsent fields keep their value");
        // An empty tool list means "no restriction", not "no tools".
        apply(&mut t, serde_json::from_value(json!({ "tools": [] })).unwrap());
        assert!(t.tools.is_none());
    }
}
