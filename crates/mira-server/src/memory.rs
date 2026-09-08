//! Per-repo and per-user memory files (`MIRA.md`).
//!
//! `GET  /api/memory`         → `{ user: {path, exists}, project: {path, exists} }`
//! `POST /api/memory/append`  → body: `{ scope: "user" | "project", text }`
//!                              appends a bullet to the appropriate file,
//!                              creating parents (and the file itself) as
//!                              needed. Returns the new byte count.
//!
//! The reader lives in `mira-memory::load_memory_files` so both the CLI and
//! the server pick it up through the shared crate. This module is the write
//! side — a tiny endpoint the frontend's `/remember` slash command hits.

use std::path::PathBuf;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_memory::MemoryScope;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AppendRequest {
    pub scope: Scope,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    Project,
}

impl From<Scope> for MemoryScope {
    fn from(s: Scope) -> Self {
        match s {
            Scope::User => MemoryScope::User,
            Scope::Project => MemoryScope::Project,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AppendView {
    pub path: String,
    pub bytes: u64,
}

pub async fn append_memory(
    State(state): State<AppState>,
    Json(req): Json<AppendRequest>,
) -> Response {
    let text = req.text.trim();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "text is empty".into());
    }
    // Route through the shared MemoryStore so this handler picks up the
    // same per-scope mutex that the memory tools use — no read-modify-
    // write race when `/remember` and a `memory_append` tool call collide.
    let store = state.current_memory().await;
    let scope: MemoryScope = req.scope.into();
    let bytes = match store.append(scope, text).await {
        Ok(b) => b,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("append memory: {e}"),
            )
        }
    };
    let path = store.path(scope);
    info!(scope = ?req.scope, path = %path.display(), bytes, "memory append");
    Json(AppendView {
        path: path.display().to_string(),
        bytes,
    })
    .into_response()
}

#[derive(Debug, Serialize)]
pub struct MemoryStatusView {
    pub user: FileStatus,
    pub project: FileStatus,
}

#[derive(Debug, Serialize)]
pub struct FileStatus {
    pub path: String,
    pub exists: bool,
    pub bytes: u64,
}

pub async fn get_memory(State(state): State<AppState>) -> Response {
    let cwd = state.current_cwd().await;
    let user_path = mira_config::user_memory_path();
    let project_path = cwd.join(".mira").join("MIRA.md");
    Json(MemoryStatusView {
        user: file_status(&user_path),
        project: file_status(&project_path),
    })
    .into_response()
}

fn file_status(p: &PathBuf) -> FileStatus {
    let meta = std::fs::metadata(p).ok();
    FileStatus {
        path: p.display().to_string(),
        exists: meta.is_some(),
        bytes: meta.map(|m| m.len()).unwrap_or(0),
    }
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
