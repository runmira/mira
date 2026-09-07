//! Git status + worktree API.
//!
//! `GET  /api/git/status`     → { in_repo, branch, dirty, ahead, behind,
//!                                worktrees: [{ path, branch, is_current }] }
//! `POST /api/git/worktree`   → body: { branch, base? } — creates
//!                              `.mira/worktrees/<branch>` from `base`
//!                              (defaults to current HEAD) and returns its path
//!
//! The composer's worktree chip polls `/api/git/status` and drives the
//! session-cwd swap through the existing `PUT /api/cwd` endpoint. Keeping
//! worktree switching layered on top of `put_cwd` means the fresh-session +
//! system-prompt rebuild logic isn't duplicated here.

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
    pub is_current: bool,
}

#[derive(Debug, Serialize)]
pub struct GitStatusView {
    pub in_repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub dirty: bool,
    pub ahead: u32,
    pub behind: u32,
    /// `true` when the current cwd is a linked worktree (as opposed to the
    /// primary git dir). Used by the UI so the toggle can say "back to main".
    pub is_worktree: bool,
    /// Basename of the primary worktree — the project as the user thinks of
    /// it (`mira`), independent of which worktree folder we're currently in
    /// (`diff-tes` etc.). The UI prefers this over the cwd's basename when
    /// `is_worktree` is true so switching branches doesn't visually change
    /// the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_project: Option<String>,
    pub worktrees: Vec<WorktreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorktree {
    /// Branch name for the new worktree. Created off `base` if it doesn't
    /// exist yet; checked out if it does.
    pub branch: String,
    /// Base ref to branch from. Defaults to `HEAD`.
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateWorktreeView {
    pub path: String,
    pub branch: String,
}

pub async fn get_status(State(state): State<AppState>) -> Response {
    let cwd = state.current_cwd().await;

    if !is_git_repo(&cwd) {
        return Json(GitStatusView {
            in_repo: false,
            branch: None,
            dirty: false,
            ahead: 0,
            behind: 0,
            is_worktree: false,
            primary_project: None,
            worktrees: vec![],
        })
        .into_response();
    }

    let branch = current_branch(&cwd);
    let (dirty, ahead, behind) = porcelain_summary(&cwd);
    let worktrees = list_worktrees(&cwd);
    let common_dir = git_common_dir(&cwd);
    let git_dir = git_dir(&cwd);
    // `common-dir` == `git-dir` for the primary worktree; they diverge for
    // linked worktrees (git-dir points inside `<primary>/.git/worktrees/<name>`).
    let is_worktree = match (git_dir, common_dir) {
        (Some(g), Some(c)) => g != c,
        _ => false,
    };
    let primary_project = primary_worktree(&cwd)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));

    Json(GitStatusView {
        in_repo: true,
        branch,
        dirty,
        ahead,
        behind,
        is_worktree,
        primary_project,
        worktrees,
    })
    .into_response()
}

pub async fn create_worktree(
    State(state): State<AppState>,
    Json(req): Json<CreateWorktree>,
) -> Response {
    let cwd = state.current_cwd().await;
    if !is_git_repo(&cwd) {
        return err(StatusCode::BAD_REQUEST, "not a git repo".into());
    }
    let branch = req.branch.trim();
    if branch.is_empty() || branch.contains(char::is_whitespace) {
        return err(StatusCode::BAD_REQUEST, "invalid branch name".into());
    }

    // Anchor worktrees under `<primary>/.mira/worktrees/<sanitized-branch>`
    // so switching back to "main" is just PUT /api/cwd to the primary tree.
    let primary = match primary_worktree(&cwd) {
        Some(p) => p,
        None => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot locate primary worktree".into(),
            )
        }
    };
    let slug = branch.replace('/', "-");
    let target = primary.join(".mira").join("worktrees").join(&slug);
    if let Some(parent) = target.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}"));
        }
    }

    // If the branch already exists, `git worktree add <path> <branch>` checks
    // it out; if not, `-b <branch>` creates it off `base`.
    let branch_exists = branch_exists(&cwd, branch);
    let base = req.base.as_deref().unwrap_or("HEAD");
    let mut cmd = Command::new("git");
    cmd.current_dir(&cwd).arg("worktree").arg("add");
    if branch_exists {
        cmd.arg(&target).arg(branch);
    } else {
        cmd.arg("-b").arg(branch).arg(&target).arg(base);
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git worktree add: {e}"),
            )
        }
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git worktree add failed: {stderr}"),
        );
    }

    Json(CreateWorktreeView {
        path: target.display().to_string(),
        branch: branch.to_owned(),
    })
    .into_response()
}

/* ---------- git shell-outs ---------- */

fn is_git_repo(cwd: &Path) -> bool {
    run(cwd, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

fn current_branch(cwd: &Path) -> Option<String> {
    let out = run(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        // Detached HEAD — surface the short sha instead.
        run(cwd, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned())
    } else {
        Some(trimmed.to_owned())
    }
}

/// Parses `git status --porcelain=v2 --branch` for the dirty flag + ahead/behind.
/// v2 puts ahead/behind on the `# branch.ab +N -N` header line, saving us a
/// second `rev-list` invocation.
fn porcelain_summary(cwd: &Path) -> (bool, u32, u32) {
    let Ok(out) = run(cwd, &["status", "--porcelain=v2", "--branch"]) else {
        return (false, 0, 0);
    };
    let mut dirty = false;
    let mut ahead = 0u32;
    let mut behind = 0u32;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: `+N -N`
            let mut it = rest.split_whitespace();
            if let Some(a) = it.next() {
                ahead = a.trim_start_matches('+').parse().unwrap_or(0);
            }
            if let Some(b) = it.next() {
                behind = b.trim_start_matches('-').parse().unwrap_or(0);
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            dirty = true;
        }
    }
    (dirty, ahead, behind)
}

fn git_common_dir(cwd: &Path) -> Option<PathBuf> {
    let s = run(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    Some(PathBuf::from(s.trim()))
}

fn git_dir(cwd: &Path) -> Option<PathBuf> {
    let s = run(cwd, &["rev-parse", "--path-format=absolute", "--git-dir"]).ok()?;
    Some(PathBuf::from(s.trim()))
}

/// The primary worktree is the parent of `--git-common-dir` (which points at
/// `<primary>/.git`).
fn primary_worktree(cwd: &Path) -> Option<PathBuf> {
    let common = git_common_dir(cwd)?;
    common.parent().map(|p| p.to_path_buf())
}

fn branch_exists(cwd: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `git worktree list --porcelain` groups per worktree with blank-line
/// separators: `worktree <path>`, `HEAD <sha>`, `branch refs/heads/<name>`
/// (or `detached`). We just want path + short branch name + whether it's
/// currently checked out.
fn list_worktrees(cwd: &Path) -> Vec<WorktreeEntry> {
    let Ok(out) = run(cwd, &["worktree", "list", "--porcelain"]) else {
        return vec![];
    };
    let current = std::fs::canonicalize(cwd).ok();
    let mut result = Vec::new();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            // Flush any previous record (worktree entries are separated by
            // blank lines, but we may hit a new `worktree` header without one).
            if let Some(p_prev) = path.take() {
                push_worktree(&mut result, p_prev, branch.take(), current.as_deref());
            }
            path = Some(p.to_owned());
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = Some(b.trim_start_matches("refs/heads/").to_owned());
        } else if line == "detached" {
            branch = None;
        }
        // `HEAD <sha>` lines are ignored — we don't display them.
    }
    if let Some(p) = path {
        push_worktree(&mut result, p, branch, current.as_deref());
    }
    result
}

fn push_worktree(
    out: &mut Vec<WorktreeEntry>,
    path: String,
    branch: Option<String>,
    current: Option<&Path>,
) {
    let is_current = current
        .and_then(|c| std::fs::canonicalize(&path).ok().map(|p| p == c))
        .unwrap_or(false);
    out.push(WorktreeEntry {
        path,
        branch,
        is_current,
    });
}

fn run(cwd: &Path, args: &[&str]) -> Result<String, std::io::Error> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
