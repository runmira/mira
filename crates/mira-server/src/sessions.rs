//! Sessions API — list recent sessions for the current cwd and resume one
//! into the live server without a restart.
//!
//! `GET    /api/sessions`           → summary list, newest first
//! `POST   /api/sessions/:id/load`  → swap the live session for the stored one
//! `DELETE /api/sessions/:id`       → remove a stored session; if it's the
//!                                    active one, start a fresh session in
//!                                    the same folder

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::RuntimeState;
use mira_core::{Role, SessionId};
use mira_harness::{Session, SessionConfig, SessionRecord};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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
    /// AI-generated short nickname. `None` until the title-generation task
    /// runs (after the first assistant reply).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// First non-empty user message, truncated. `None` for empty sessions.
    pub first_user_message: Option<String>,
    /// True if this session is the one currently active on the server.
    pub active: bool,
    /// Merge state of the session's worktree branch relative to `main`/
    /// `master` in the primary repo. Only populated for sessions whose cwd
    /// is a Mira-created worktree (`…/.mira/worktrees/<branch>`); `None`
    /// otherwise (regular session, deleted worktree, no git, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_status: Option<WorktreeMergeStatus>,
    /// Branch name of the worktree, when [`worktree_status`] is set. Shown
    /// as a tooltip on the merge indicator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
}

/// Where a worktree branch sits relative to its primary repo's base branch.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeMergeStatus {
    /// Ancestor of `main`/`master` — safe to prune.
    Merged,
    /// Not yet merged; carries commits the base branch doesn't have.
    Unmerged,
}

const FIRST_MSG_TRUNC: usize = 80;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// `?all=1` returns sessions across every cwd (used by the web sidebar
    /// to group by project). Omit or `all=0` for just the current cwd.
    #[serde(default)]
    pub all: bool,
}

pub async fn list_sessions(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let Some(store) = state.store.clone() else {
        return Json(Vec::<SessionSummary>::new()).into_response();
    };
    let records = if q.all {
        match store.list_all(200).await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")),
        }
    } else {
        let cwd = state.current_cwd().await;
        match store.list_recent(&cwd, 50).await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")),
        }
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

    // Adopt the session's original cwd — otherwise clicking a chat from a
    // different project silently runs it against whatever folder happens
    // to be active, which is worse than useless. Shared Arc means the
    // approver sees the same swap for diff previews.
    let record_cwd = record.cwd.clone();
    if record_cwd.is_dir() {
        {
            let mut guard = state.cwd.write().await;
            *guard = record_cwd.clone();
        }
        let mut s = RuntimeState::load().unwrap_or_default();
        s.last_cwd = Some(record_cwd.clone());
        if let Err(e) = s.save() {
            warn!(%e, "state.yaml: save failed after session load");
        }
    }

    // Rebuild a Session with the SAME harness pieces the server started with,
    // so the WsApprover / provider / policy all continue to route through the
    // live server plumbing. `tool_ctx` uses the (now updated) shared cwd —
    // which is the record's cwd from the swap above.
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
    let turns = resumed.turns().await;
    let usage = resumed.usage().await;
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
        turns,
        usage,
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
            reasoning_effort: prev_cfg.reasoning_effort.clone(),
        },
        crate::system_prompt(&cwd, &state.registry),
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
    let turns = fresh.turns().await;
    let usage = fresh.usage().await;
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
        turns,
        usage,
    });

    Json(serde_json::json!({ "id": session_id })).into_response()
}

pub async fn delete_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — nothing to delete".to_string(),
        );
    };
    let sid = SessionId::from(id.as_str());
    if let Err(e) = store.delete(&sid).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("delete: {e}"));
    }

    // If the deleted row is the currently loaded session, roll a fresh one
    // in the same folder so the UI doesn't keep a dangling session_id.
    let active_id = state.current_session().await.id.to_string();
    if active_id == id {
        let prev_cfg = state.current_session().await.config().await;
        let cwd = state.current_cwd().await;
        let mut fresh = Session::new(
            SessionConfig {
                model: prev_cfg.model.clone(),
                max_rounds: prev_cfg.max_rounds,
                temperature: prev_cfg.temperature,
                max_tokens: prev_cfg.max_tokens,
                reasoning_effort: prev_cfg.reasoning_effort.clone(),
            },
            crate::system_prompt(&cwd, &state.registry),
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
        let turns = fresh.turns().await;
        let usage = fresh.usage().await;
        let session_id = fresh.id.to_string();
        {
            let mut guard = state.session.write().await;
            *guard = fresh;
        }
        let _ = state.events_tx.send(ServerMsg::Ready {
            session_id,
            model: cfg.model,
            mode,
            cwd: cwd.display().to_string(),
            history,
            turns,
            usage,
        });
    }

    info!(%id, "session deleted");
    Json(serde_json::json!({ "ok": true })).into_response()
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
    let (worktree_status, worktree_branch) = detect_worktree_status(&r.cwd);
    SessionSummary {
        id,
        model: r.cfg.model.clone(),
        cwd: r.cwd.display().to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
        message_count: r.messages.iter().filter(|m| m.role != Role::System).count(),
        title: r.title.clone(),
        first_user_message: first,
        active,
        worktree_status,
        worktree_branch,
    }
}

/// Recognise Mira-created worktrees by their canonical path shape
/// (`…/.mira/worktrees/<branch>`) and report whether their branch has been
/// merged into `main`/`master` in the primary repo. Anything else — regular
/// project cwd, deleted worktree dir, non-git folder — returns `(None, None)`.
///
/// Runs a small handful of `git` shell-outs. At ~200 sessions this adds a
/// perceptible pause to `GET /api/sessions?all=1`; if that becomes a problem
/// we can memoise per (primary_repo, branch).
fn detect_worktree_status(cwd: &std::path::Path) -> (Option<WorktreeMergeStatus>, Option<String>) {
    // Cheap prefilter: the path must contain the Mira worktrees folder.
    // Handles `some/.mira/worktrees/foo` and `foo/.mira/worktrees/bar`.
    if !cwd
        .components()
        .zip(cwd.components().skip(1))
        .zip(cwd.components().skip(2))
        .any(|((a, b), c)| {
            use std::path::Component::Normal;
            let (Normal(a), Normal(b), Normal(c)) = (a, b, c) else {
                return false;
            };
            a == std::ffi::OsStr::new(".mira")
                && b == std::ffi::OsStr::new("worktrees")
                && !c.is_empty()
        })
    {
        return (None, None);
    }
    if !cwd.is_dir() {
        // Worktree directory was deleted — nothing to report.
        return (None, None);
    }

    let branch = match git_output(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Some(b) if !b.is_empty() => b,
        _ => return (None, None),
    };

    // Primary repo lives at the parent of `--git-common-dir` (which points at
    // `<primary>/.git`).
    let common_dir = match git_output(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        Some(s) => std::path::PathBuf::from(s),
        None => return (None, Some(branch)),
    };
    let primary = match common_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => return (None, Some(branch)),
    };

    // Try main then master; whichever exists is the base. If neither does,
    // we can't compute a merge status, so leave it null.
    let base = ["main", "master"]
        .iter()
        .copied()
        .find(|b| branch_exists(&primary, b));
    let Some(base) = base else {
        return (None, Some(branch));
    };
    if branch == base {
        // On the base branch itself — a "worktree" of main isn't unusual;
        // don't badge it.
        return (None, Some(branch));
    }

    let status = if is_ancestor(&primary, &branch, base) {
        WorktreeMergeStatus::Merged
    } else {
        WorktreeMergeStatus::Unmerged
    };
    (Some(status), Some(branch))
}

fn git_output(cwd: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn branch_exists(cwd: &std::path::Path, name: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_ancestor(cwd: &std::path::Path, branch: &str, base: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(["merge-base", "--is-ancestor", branch, base])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
