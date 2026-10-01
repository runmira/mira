//! The context inspector (`/api/context`): what fills the active chat's
//! context window, and taking a tool result out of it.
//!
//! Only for chats Mira's own models run. An external agent (Claude Code,
//! Codex) keeps its own context in its own process; Mira can't see inside
//! it, so these answer with a short explanation instead of a guess.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::state::AppState;

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

const AGENT_CONTEXT: &str = "This chat runs on an external agent, which manages its own context.";

/// GET /api/context
pub async fn breakdown(State(state): State<AppState>) -> Response {
    let slot = state.active_slot().await;
    if slot.acp_launch.lock().await.is_some() {
        return err(StatusCode::CONFLICT, AGENT_CONTEXT);
    }
    let session = slot.session.read().await.clone();
    let cfg = session.config().await;
    let window = mira_harness::history::context_window_with(&cfg.model, cfg.context_window) as u64;
    let b = session.context_breakdown().await;
    Json(serde_json::json!({
        "window": window,
        "compact_at": mira_harness::history::auto_compact_at(window),
        "breakdown": b,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct DropRequest {
    pub call_id: String,
}

/// POST /api/context/drop { call_id }
pub async fn drop_result(State(state): State<AppState>, Json(req): Json<DropRequest>) -> Response {
    let slot = state.active_slot().await;
    if slot.acp_launch.lock().await.is_some() {
        return err(StatusCode::CONFLICT, AGENT_CONTEXT);
    }
    let session = slot.session.read().await.clone();
    match session.drop_tool_result(&req.call_id).await {
        Ok(d) => Json(d).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}
