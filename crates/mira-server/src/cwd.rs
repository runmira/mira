//! Working-directory API for the *active* slot — read the current cwd or
//! swap the active slot for one bound to a different folder.
//!
//! `GET /api/cwd`   → `{ path, home }`
//! `PUT /api/cwd`   → validates the path, spins up a fresh slot in it,
//!                    marks it active, and emits its Ready. The old slot
//!                    is left running (a background session in the old
//!                    folder keeps making progress).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::RuntimeState;
use mira_harness::SessionConfig;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct CwdView {
    pub path: String,
    pub home: Option<String>,
    /// Session id of the freshly-built slot on a `PUT`. `None` on `GET`
    /// (nothing new was created; the client already knows the active id).
    /// Frontend uses this to `Attach { id }` its WS to the new slot so
    /// the fresh Ready lands in the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CwdUpdate {
    pub path: String,
}

pub async fn get_cwd(State(state): State<AppState>) -> Response {
    Json(CwdView {
        path: state.current_cwd().await.display().to_string(),
        home: std::env::var("HOME").ok(),
        session_id: None,
    })
    .into_response()
}

pub async fn put_cwd(State(state): State<AppState>, Json(u): Json<CwdUpdate>) -> Response {
    let expanded = shellexpand::tilde(&u.path).to_string();
    let path = match std::fs::canonicalize(&expanded) {
        Ok(p) => p,
        Err(e) => {
            return err(
                StatusCode::BAD_REQUEST,
                format!("cannot resolve `{expanded}`: {e}"),
            )
        }
    };
    if !path.is_dir() {
        return err(
            StatusCode::BAD_REQUEST,
            format!("not a directory: {}", path.display()),
        );
    }

    persist_cwd(&path);
    // Project MCP servers, commands and skills follow the folder.
    state.extensions.set_project(Some(path.clone())).await;
    state
        .broadcast_all(crate::protocol::ServerMsg::ExtensionsChanged)
        .await;

    // Spin up a fresh slot bound to the new folder. The previously-active
    // slot is left in the map — it may still have a background turn in
    // flight, and dropping it here would silently kill that work.
    let prev_cfg = state.current_session().await.config().await;
    let cfg = SessionConfig {
        model: prev_cfg.model,
        max_rounds: prev_cfg.max_rounds,
        temperature: prev_cfg.temperature,
        max_tokens: prev_cfg.max_tokens,
        reasoning_effort: prev_cfg.reasoning_effort.clone(),
        service_tier: None,
        response_format: prev_cfg.response_format.clone(),
        compactor_model: prev_cfg.compactor_model.clone(),
        small_model: prev_cfg.small_model.clone(),
        context_window: prev_cfg.context_window,
    };
    let deps = state.slot_deps();
    let slot = crate::slot::build_slot(path.clone(), cfg, None, &deps).await;
    // Same conversation, new folder — the agent setup moves with it.
    // The inherited config starts (blank) on this slot's first prompt,
    // with the agent's own boundary built from the new cwd at spawn.
    *slot.acp_launch.lock().await = state.active_slot().await.acp_launch.lock().await.clone();
    let slot_id = slot.id.clone();
    state.insert_slot(slot.clone()).await;
    state.set_active(slot_id.clone()).await;

    info!(cwd = %path.display(), session_id = %slot_id, "cwd changed → new slot");
    // Emit Ready on the new slot's channel so whichever WS is attached
    // (or about to Attach) picks up the fresh session immediately.
    let _ = slot
        .events_tx
        .send(crate::ws::build_ready(&slot, &state).await);

    Json(CwdView {
        path: path.display().to_string(),
        home: std::env::var("HOME").ok(),
        session_id: Some(slot_id.to_string()),
    })
    .into_response()
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn persist_cwd(path: &std::path::Path) {
    let mut s = RuntimeState::load().unwrap_or_default();
    s.last_cwd = Some(path.to_path_buf());
    if let Err(e) = s.save() {
        warn!(%e, "state.yaml: save failed after cwd change");
    }
}
