//! Sessions API — list recent sessions for the current cwd and resume one
//! into the live server without a restart.
//!
//! `GET  /api/sessions`           → summary list, newest first
//! `POST /api/sessions/:id/load`  → swap the live session for the stored one
//!
//! Delete is not exposed yet — the underlying store doesn't have a delete
//! primitive. Add when needed.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_core::{Role, SessionId};
use mira_harness::{Session, SessionConfig, SessionRecord};
use serde::Serialize;
use tracing::info;

use crate::protocol::ServerMsg;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub model: String,
    pub cwd: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    /// First non-empty user message, truncated. `None` for empty sessions.
    pub first_user_message: Option<String>,
    /// True if this session is the one currently active on the server.
    pub active: bool,
}

const FIRST_MSG_TRUNC: usize = 80;

pub async fn list_sessions(State(state): State<AppState>) -> Response {
    let Some(store) = state.store.clone() else {
        return Json(Vec::<SessionSummary>::new()).into_response();
    };
    let cwd = state.current_cwd().await;
    let records = match store.list_recent(&cwd, 50).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")),
    };
    let active_id = state.current_session().await.id.to_string();
    let summaries: Vec<SessionSummary> = records
        .into_iter()
        .map(|r| summarize(&r, &active_id))
        .collect();
    Json(summaries).into_response()
}

pub async fn load_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — nothing to resume".to_string(),
        );
    };
    let record = match store.load(&SessionId::from(id.as_str())).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };

    // Rebuild a Session with the SAME harness pieces the server started with,
    // so the WsApprover / provider / policy all continue to route through the
    // live server plumbing. `tool_ctx` is rebuilt from the current shared cwd
    // (so a resumed session runs against wherever the user just switched to,
    // not the folder it was originally saved from).
    let mut resumed = Session::resume_from(
        record,
        state.harness_provider.clone(),
        state.registry.clone(),
        state.policy.clone(),
        state.approver.clone(),
        state.make_tool_ctx().await,
    );
    if let Some(s) = state.store.clone() {
        resumed = resumed.with_store(s);
    }

    // Swap under the write lock, then re-broadcast Ready so every connected
    // client rehydrates its transcript for the new session.
    let cfg = resumed.config().await;
    let mode = state.policy.lock().await.mode();
    let history = resumed.history().await;
    let session_id = resumed.id.to_string();

    {
        let mut guard = state.session.write().await;
        *guard = resumed;
    }
    info!(id = %session_id, "session resumed");

    let _ = state.events_tx.send(ServerMsg::Ready {
        session_id: session_id.clone(),
        model: cfg.model,
        mode,
        cwd: state.current_cwd().await.display().to_string(),
        history,
    });

    Json(serde_json::json!({ "id": session_id })).into_response()
}

/// Create a fresh session (no history) reusing the currently active
/// folder, model, and provider. Only the transcript resets — the workspace
/// the user picked, and the model they last swapped to, stay put.
pub async fn new_session(State(state): State<AppState>) -> Response {
    let prev = state.current_session().await;
    let prev_cfg = prev.config().await;
    let cwd = state.current_cwd().await;

    let mut fresh = Session::new(
        SessionConfig {
            model: prev_cfg.model.clone(),
            max_rounds: prev_cfg.max_rounds,
            temperature: prev_cfg.temperature,
            max_tokens: prev_cfg.max_tokens,
        },
        crate::system_prompt(&cwd),
        state.harness_provider.clone(),
        state.registry.clone(),
        state.policy.clone(),
        state.approver.clone(),
        state.make_tool_ctx().await,
    );
    if let Some(s) = state.store.clone() {
        fresh = fresh.with_store(s);
    }

    let cfg = fresh.config().await;
    let mode = state.policy.lock().await.mode();
    let history = fresh.history().await;
    let session_id = fresh.id.to_string();

    {
        let mut guard = state.session.write().await;
        *guard = fresh;
    }
    info!(id = %session_id, cwd = %cwd.display(), model = %cfg.model, "new session");

    let _ = state.events_tx.send(ServerMsg::Ready {
        session_id: session_id.clone(),
        model: cfg.model,
        mode,
        cwd: cwd.display().to_string(),
        history,
    });

    Json(serde_json::json!({ "id": session_id })).into_response()
}

fn summarize(r: &SessionRecord, active_id: &str) -> SessionSummary {
    let first = r
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.clone())
        .map(|s| truncate(&s, FIRST_MSG_TRUNC));
    let id = r.id.to_string();
    let active = id == active_id;
    SessionSummary {
        id,
        model: r.cfg.model.clone(),
        cwd: r.cwd.display().to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
        message_count: r.messages.iter().filter(|m| m.role != Role::System).count(),
        first_user_message: first,
        active,
    }
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim().replace(['\n', '\r'], " ");
    if trimmed.chars().count() <= max {
        return trimmed;
    }
    let head: String = trimmed.chars().take(max).collect();
    format!("{head}…")
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
