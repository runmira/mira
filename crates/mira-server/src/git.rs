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

/// One branch entry for the worktree picker. Local + remote refs
/// (deduped on short name — a local `foo` hides `origin/foo`, but the
/// upstream tracking hint is preserved). `in_worktree` flags branches
/// that are already checked out somewhere so the UI can hide them
/// from the "switch to" list.
#[derive(Debug, Serialize)]
pub struct BranchEntry {
    /// Short name: `main`, `dami/fix-map-leaks`, or `origin/some-remote`
    /// when only a remote copy exists (`remote:` also flagged).
    pub name: String,
    /// `true` when this branch has no local ref — only a remote-tracking
    /// ref. Clicking such a branch in the UI creates a local branch
    /// tracking the remote at worktree-add time.
    pub is_remote: bool,
    /// `true` when this branch is currently checked out in some
    /// worktree (primary or linked). Used by the UI to filter the
    /// "switch to" list — you can't create a worktree on a branch
    /// that's already checked out elsewhere.
    pub in_worktree: bool,
    /// The upstream ref, if configured — surfaced as a subtle hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
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
    /// All branches (local + remote), deduped on short name. Sorted
    /// with the currently-checked-out branch first, then alphabetical.
    /// Empty when not in a repo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<BranchEntry>,
    /// Total lines added across all uncommitted changes (staged + unstaged vs HEAD).
    #[serde(default)]
    pub diff_added: u64,
    /// Total lines removed across all uncommitted changes (staged + unstaged vs HEAD).
    #[serde(default)]
    pub diff_removed: u64,
    /// Short subject of the most recent commit (`git log -1 --pretty=%s`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<String>,
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
            branches: vec![],
            diff_added: 0,
            diff_removed: 0,
            last_commit: None,
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
    let branches = list_branches(&cwd, &worktrees);
    let (diff_added, diff_removed) = diff_stats(&cwd);
    let last_commit = last_commit_subject(&cwd);

    Json(GitStatusView {
        in_repo: true,
        branch,
        dirty,
        ahead,
        behind,
        is_worktree,
        primary_project,
        worktrees,
        branches,
        diff_added,
        diff_removed,
        last_commit,
    })
    .into_response()
}

/// Diff stats scoped to only the files this session has written.
/// Returns zeroes when the session has no file guard or hasn't touched anything.
pub async fn session_diff(State(state): State<AppState>) -> Response {
    let cwd = state.current_cwd().await;
    let session = state.current_session().await;

    let written = match session.file_guard() {
        Some(g) => g.written_snapshot().await,
        None => {
            return Json(serde_json::json!({ "added": 0, "removed": 0, "files": [] }))
                .into_response()
        }
    };

    if written.is_empty() {
        return Json(serde_json::json!({ "added": 0, "removed": 0, "files": [] })).into_response();
    }

    // Collect relative paths (git diff requires paths relative to repo root).
    let file_args: Vec<String> = written
        .iter()
        .filter_map(|p| {
            p.strip_prefix(&cwd)
                .ok()
                .map(|r| r.to_string_lossy().into_owned())
        })
        .collect();

    if file_args.is_empty() {
        return Json(serde_json::json!({ "added": 0, "removed": 0, "files": [] })).into_response();
    }

    // `git diff HEAD --numstat -- file1 file2 …`
    let mut args = vec!["diff", "HEAD", "--numstat", "--"];
    let file_strs: Vec<&str> = file_args.iter().map(|s| s.as_str()).collect();
    args.extend_from_slice(&file_strs);

    let (added, removed) = match run(&cwd, &args) {
        Ok(out) => {
            let mut a = 0u64;
            let mut r = 0u64;
            for line in out.lines() {
                let mut parts = line.split('\t');
                if let (Some(ad), Some(rm)) = (parts.next(), parts.next()) {
                    a += ad.parse::<u64>().unwrap_or(0);
                    r += rm.parse::<u64>().unwrap_or(0);
                }
            }
            (a, r)
        }
        Err(_) => (0, 0),
    };

    let file_names: Vec<String> = written
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    Json(serde_json::json!({
        "added": added,
        "removed": removed,
        "files": file_names,
    }))
    .into_response()
}

/// GET /api/git/branch-pr
/// Returns the PR for the current branch using `gh pr view`.
/// Returns 404 when no PR exists for the branch.
pub async fn branch_pr(State(state): State<AppState>) -> Response {
    let cwd = state.current_cwd().await;
    if !is_git_repo(&cwd) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not a git repo"})),
        )
            .into_response();
    }
    let out = Command::new("gh")
        .current_dir(&cwd)
        .args([
            "pr",
            "view",
            "--json",
            "number,title,state,url,isDraft,reviewDecision",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            match serde_json::from_str::<serde_json::Value>(text.trim()) {
                Ok(v) => Json(v).into_response(),
                Err(_) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no pr"})),
                )
                    .into_response(),
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no pr"})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub message: String,
    #[serde(default)]
    pub include_unstaged: bool,
    #[serde(default)]
    pub push_after: bool,
}

/// POST /api/git/commit
/// Commits staged (+ optionally all) changes. If message is blank, generates
/// one from the list of changed files.
pub async fn commit(State(state): State<AppState>, Json(req): Json<CommitRequest>) -> Response {
    let cwd = state.current_cwd().await;
    if !is_git_repo(&cwd) {
        return err(StatusCode::BAD_REQUEST, "not a git repo".into());
    }

    // Always stage modifications to tracked files; with the toggle also
    // stage new/untracked files (-A vs -u).
    Command::new("git")
        .current_dir(&cwd)
        .args(["add", if req.include_unstaged { "-A" } else { "-u" }])
        .output()
        .ok();

    let message = if req.message.trim().is_empty() {
        // Auto-generate from staged files
        let files = run(&cwd, &["diff", "--staged", "--name-only"]).unwrap_or_default();
        let names: Vec<&str> = files
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .take(3)
            .collect();
        if names.is_empty() {
            "chore: session changes".to_string()
        } else {
            format!("update {}", names.join(", "))
        }
    } else {
        req.message.trim().to_string()
    };

    let commit_out = Command::new("git")
        .current_dir(&cwd)
        .args(["commit", "-m", &message])
        .output();

    match commit_out {
        Ok(o) if o.status.success() => {
            if req.push_after {
                let push_out = Command::new("git")
                    .current_dir(&cwd)
                    .args(["push", "--set-upstream", "origin", "HEAD"])
                    .output();
                match push_out {
                    Ok(po) if po.status.success() => {
                        Json(serde_json::json!({"ok": true, "pushed": true})).into_response()
                    }
                    Ok(po) => err(
                        StatusCode::BAD_REQUEST,
                        String::from_utf8_lossy(&po.stderr).to_string(),
                    ),
                    Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                }
            } else {
                Json(serde_json::json!({"ok": true, "pushed": false})).into_response()
            }
        }
        Ok(o) => err(
            StatusCode::BAD_REQUEST,
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn push(State(state): State<AppState>) -> Response {
    let cwd = state.current_cwd().await;
    if !is_git_repo(&cwd) {
        return err(StatusCode::BAD_REQUEST, "not a git repo".into());
    }
    let out = Command::new("git")
        .current_dir(&cwd)
        .args(["push", "--set-upstream", "origin", "HEAD"])
        .output();
    match out {
        Ok(o) if o.status.success() => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(o) => err(
            StatusCode::BAD_REQUEST,
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
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

    // Three shapes for `git worktree add`, decided by whether a matching
    // local branch already exists and — if not — whether a remote-tracking
    // ref exists we can use as the start point:
    //   1. local `<branch>` exists         → `git worktree add <path> <branch>`
    //   2. only `<remote>/<branch>` exists → `git worktree add -b <branch>
    //                                        <path> <remote>/<branch>`
    //      (creates a local tracking branch; the branch picker for a fresh
    //      remote is the common case that used to require dropping to a
    //      terminal to `git fetch && git checkout -b`.)
    //   3. brand-new branch                → `git worktree add -b <branch>
    //                                        <path> <base>` (base defaults
    //                                        to HEAD)
    let branch_exists_locally = branch_exists(&cwd, branch);
    let remote_ref = if branch_exists_locally {
        None
    } else {
        remote_branch_ref(&cwd, branch)
    };
    let base = req.base.as_deref().unwrap_or("HEAD");
    let mut cmd = Command::new("git");
    cmd.current_dir(&cwd).arg("worktree").arg("add");
    if branch_exists_locally {
        cmd.arg(&target).arg(branch);
    } else if let Some(remote) = &remote_ref {
        cmd.arg("-b").arg(branch).arg(&target).arg(remote);
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

/// If exactly one remote has a branch with this short name, return the
/// full remote-tracking short-ref (`origin/foo`). Ambiguous names
/// (present on `origin` and `upstream`) return `None` — the user
/// should qualify. Missing remote returns `None` too, which the
/// caller treats as "create a fresh branch off base."
fn remote_branch_ref(cwd: &Path, branch: &str) -> Option<String> {
    // `for-each-ref` on remotes only, filtering the short name via
    // `--format`. Cheaper than parsing full refnames ourselves.
    let out = run(
        cwd,
        &["for-each-ref", "--format=%(refname:short)", "refs/remotes/"],
    )
    .ok()?;
    let matches: Vec<String> = out
        .lines()
        .filter_map(|l| {
            let short = l.trim();
            if short.is_empty() || short.ends_with("/HEAD") {
                return None;
            }
            // `origin/foo` → strip `origin/`, compare to `foo`.
            let (_remote, name) = short.split_once('/')?;
            (name == branch).then(|| short.to_owned())
        })
        .collect();
    if matches.len() == 1 {
        Some(matches.into_iter().next().unwrap())
    } else {
        // Zero (branch doesn't exist upstream) or ambiguous (multiple
        // remotes have it) → let the caller fall through to the "fresh
        // branch off base" path.
        None
    }
}

/// Enumerate all branches (local + remote), dedupe on short name so a
/// local `foo` hides `origin/foo` from the picker (but still records
/// the upstream), and flag branches already checked out in one of the
/// listed worktrees.
///
/// Uses `git for-each-ref --format='%(refname)|…'` so we get the
/// full ref (`refs/heads/foo`, `refs/remotes/origin/foo`) and can
/// classify unambiguously — a heuristic on the short name would
/// misclassify branches like `dami/fix-foo` if a remote were ever
/// called `dami`. `refs/remotes/*/HEAD` symbolic refs are dropped.
fn list_branches(cwd: &Path, worktrees: &[WorktreeEntry]) -> Vec<BranchEntry> {
    let Ok(out) = run(
        cwd,
        &[
            "for-each-ref",
            "--format=%(refname)|%(refname:short)|%(upstream:short)",
            "refs/heads/",
            "refs/remotes/",
        ],
    ) else {
        return vec![];
    };

    let checked_out: std::collections::HashSet<String> =
        worktrees.iter().filter_map(|w| w.branch.clone()).collect();

    // Collect locals in a map keyed by name so remotes can dedupe
    // against them in one pass without a second lookup. Remotes we
    // stash separately and merge at the end.
    let mut locals: std::collections::BTreeMap<String, BranchEntry> =
        std::collections::BTreeMap::new();
    let mut remotes: Vec<BranchEntry> = Vec::new();

    for line in out.lines() {
        let mut parts = line.splitn(3, '|');
        let (Some(full), Some(short), upstream) =
            (parts.next(), parts.next(), parts.next().unwrap_or(""))
        else {
            continue;
        };
        let full = full.trim();
        let short = short.trim();
        let upstream = upstream.trim();
        if full.is_empty() || short.is_empty() {
            continue;
        }
        // Drop `refs/remotes/<remote>/HEAD` — symbolic ref, not a real
        // branch. Points at whatever the remote's default branch is,
        // which is already in the list under its own name. Gate on the
        // FULL refname: `%(refname:short)` for a symbolic ref can be
        // just `origin` on some git versions (they strip the `/HEAD`
        // suffix), which would slip past a short-name check and get
        // pushed as a branch literally named `HEAD` — then
        // `git worktree add -b HEAD` fails because HEAD isn't a valid
        // branch name.
        if full.ends_with("/HEAD") {
            continue;
        }

        if let Some(rest) = full.strip_prefix("refs/heads/") {
            // Belt-and-suspenders: `HEAD` is never a valid local
            // branch. Should be unreachable via `refs/heads/HEAD`, but
            // filtering here means any future code path can't spawn
            // the same "HEAD as a branch" surprise.
            if rest == "HEAD" {
                continue;
            }
            locals.entry(rest.to_owned()).or_insert(BranchEntry {
                name: rest.to_owned(),
                is_remote: false,
                in_worktree: checked_out.contains(rest),
                upstream: if upstream.is_empty() {
                    None
                } else {
                    Some(upstream.to_owned())
                },
            });
        } else if let Some(rest) = full.strip_prefix("refs/remotes/") {
            // `rest` is `<remote>/<branch>` — strip the remote prefix
            // to get the branch name a user would type at
            // `git checkout`.
            let (_remote, name) = match rest.split_once('/') {
                Some(t) => t,
                None => continue,
            };
            if name == "HEAD" {
                continue;
            }
            remotes.push(BranchEntry {
                name: name.to_owned(),
                is_remote: true,
                in_worktree: false,
                upstream: Some(short.to_owned()),
            });
        }
    }

    // Merge remotes on top of locals: if a local of the same short
    // name exists, drop the remote entry (the local wins the picker
    // slot). If two remotes ship the same branch (rare — usually
    // origin vs upstream fork), keep only the first; `remote_branch_ref`
    // will return None on ambiguity at create-time and fall back
    // safely to the base-branch path.
    let mut out_vec: Vec<BranchEntry> = locals.into_values().collect();
    let mut seen_remote: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in remotes {
        if out_vec.iter().any(|b| b.name == r.name) {
            continue;
        }
        if !seen_remote.insert(r.name.clone()) {
            continue;
        }
        out_vec.push(r);
    }

    // Local first, then remote; alphabetical within each group so the
    // picker is deterministic across reloads.
    out_vec.sort_by(|a, b| {
        a.is_remote
            .cmp(&b.is_remote)
            .then_with(|| a.name.cmp(&b.name))
    });
    out_vec
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

fn diff_stats(cwd: &Path) -> (u64, u64) {
    let Ok(out) = run(cwd, &["diff", "HEAD", "--numstat"]) else {
        return (0, 0);
    };
    let mut added = 0u64;
    let mut removed = 0u64;
    for line in out.lines() {
        let mut parts = line.split('\t');
        if let (Some(a), Some(r)) = (parts.next(), parts.next()) {
            added += a.parse::<u64>().unwrap_or(0);
            removed += r.parse::<u64>().unwrap_or(0);
        }
    }
    (added, removed)
}

fn last_commit_subject(cwd: &Path) -> Option<String> {
    run(cwd, &["log", "-1", "--pretty=format:%s"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn run(cwd: &Path, args: &[&str]) -> Result<String, std::io::Error> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
