//! `POST /api/undo` — revert the last N file writes made by the current
//! session's agent turn.
//!
//! Sits directly on top of `mira_tools::FileGuard`, which the harness wires
//! into every `Session` at construction time. Missing guard = feature was
//! disabled during session init; we surface that as a clean 400 rather
//! than a 500 so the frontend can tell the user.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_tools::AppliedUndo;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::protocol::ServerMsg;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct UndoRequest {
    /// How many operations to revert (newest first). Defaults to 1.
    #[serde(default = "default_count")]
    pub count: usize,
}

fn default_count() -> usize {
    1
}

#[derive(Debug, Serialize)]
pub struct UndoView {
    pub applied: Vec<AppliedUndo>,
}

pub async fn apply_undo(State(state): State<AppState>, Json(req): Json<UndoRequest>) -> Response {
    let session = state.current_session().await;
    let Some(guard) = session.file_guard() else {
        return err(
            StatusCode::BAD_REQUEST,
            "undo is not available for this session".into(),
        );
    };
    match guard.undo(req.count).await {
        Ok(applied) => {
            info!(count = applied.len(), "undo applied");
            // Surface the revert in the transcript so the user has an
            // in-line record of what changed. Warning is the closest
            // existing channel for out-of-band notices; if we grow a
            // dedicated "info" variant later this moves there.
            let summary = if applied.is_empty() {
                "nothing to undo".to_owned()
            } else {
                let paths: Vec<String> = applied.iter().map(|a| a.path.clone()).collect();
                format!(
                    "[undo] reverted {} write{}: {}",
                    applied.len(),
                    if applied.len() == 1 { "" } else { "s" },
                    paths.join(", ")
                )
            };
            let _ = state.events_tx.send(ServerMsg::Warning { text: summary });
            Json(UndoView { applied }).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
