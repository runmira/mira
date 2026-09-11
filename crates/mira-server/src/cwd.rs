//! Working-directory API — read the current cwd or swap it for a different
//! folder without restarting the server.
//!
//! `GET /api/cwd`   → `{ path, home }`
//! `PUT /api/cwd`   → body: `{ path }` — validates the path exists and is a
//!                    directory, canonicalizes, swaps the shared cwd, and
//!                    starts a fresh session in the new folder.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::RuntimeState;
use mira_harness::{Session, SessionConfig};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::protocol::ServerMsg;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct CwdView {
    pub path: String,
    pub home: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CwdUpdate {
    pub path: String,
}

pub async fn get_cwd(State(state): State<AppState>) -> Response {
    Json(CwdView {
        path: state.current_cwd().await.display().to_string(),
        home: std::env::var("HOME").ok(),
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

    // Swap the shared cwd handle first; the approver reads through the same
    // Arc so preview computation immediately targets the new folder.
    {
        let mut guard = state.cwd.write().await;
        *guard = path.clone();
    }
    // Rebuild the memory store now that the project path has moved.
    // Fresh instance = fresh per-scope mutex, but that's fine — the
    // previous folder's writers finished when the session ended.
    state.rebuild_memory_for_cwd(&path).await;
    // Persist the pick so the next `mira serve` restart lands here rather
    // than the launch shell's cwd. Failures aren't worth propagating.
    persist_cwd(&path);

    // Start a fresh session in the new folder — resuming an old chat that
    // referenced files in the old folder would just be confusing.
    let prev_cfg = state.current_session().await.config().await;
    let mut fresh = Session::new(
        SessionConfig {
            model: prev_cfg.model,
            max_rounds: prev_cfg.max_rounds,
            temperature: prev_cfg.temperature,
            max_tokens: prev_cfg.max_tokens,
            reasoning_effort: prev_cfg.reasoning_effort.clone(),
            response_format: prev_cfg.response_format.clone(),
        },
        crate::system_prompt(&path, &state.registry),
        state.harness_provider.clone(),
        state.registry.clone(),
        state.policy.clone(),
        state.approver.clone(),
        state.make_tool_ctx().await,
    );
    if let Some(s) = state.store.clone() {
        fresh = fresh.with_store(s);
    }
    // Re-point the memory snapshot at the new project. User memory is
    // unchanged; project memory now resolves against the new cwd. Uses
    // the shared episodic handle so the snapshot reader and
    // `memory_remember` writer hit the same in-memory mutex.
    fresh = fresh.with_memory_snapshot(crate::make_memory_snapshot_with(
        &path,
        state.current_episodic().await,
    ));

    let cfg = fresh.config().await;
    let mode = state.policy.lock().await.mode();
    let history = fresh.history().await;
    let turns = fresh.turns().await;
    let usage = fresh.usage().await;
    let tasks = fresh.tasks().await;
    let goal = fresh.goal().await;
    let session_id = fresh.id.to_string();

    {
        let mut guard = state.session.write().await;
        *guard = fresh;
    }
    info!(cwd = %path.display(), "cwd changed");

    let _ = state.events_tx.send(ServerMsg::Ready {
        session_id,
        model: cfg.model,
        mode,
        cwd: path.display().to_string(),
        history,
        turns,
        usage,
        tasks,
        goal,
    });

    Json(CwdView {
        path: path.display().to_string(),
        home: std::env::var("HOME").ok(),
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
