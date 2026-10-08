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

use crate::session_changes::{ChangedFile, FileStatus};
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

/// What the active chat changed, by anyone — its tools, an external agent
/// or a shell command — that a commit would still carry. See
/// [`crate::session_changes`].
async fn session_files(state: &AppState) -> (PathBuf, Vec<ChangedFile>) {
    let slot = state.active_slot().await;
    let cwd = slot.cwd.read().await.clone();
    let session = slot.id.to_string();
    let written: Vec<PathBuf> = match slot.session.read().await.file_guard() {
        Some(g) => g.written_snapshot().await.into_iter().collect(),
        None => Vec::new(),
    };
    let cwd_bg = cwd.clone();
    let files = tokio::task::spawn_blocking(move || {
        crate::session_changes::changed_files(&cwd_bg, &session, &written)
    })
    .await
    .unwrap_or_default();
    (cwd, files)
}

/// GET /api/git/session-diff
/// Totals and per-file line counts for what this chat changed.
pub async fn session_diff(State(state): State<AppState>) -> Response {
    let (_, files) = session_files(&state).await;
    let added: u64 = files.iter().map(|f| f.added).sum();
    let removed: u64 = files.iter().map(|f| f.removed).sum();
    let untracked = files
        .iter()
        .filter(|f| f.status == FileStatus::Added)
        .count();
    Json(serde_json::json!({
        "added": added,
        "removed": removed,
        "uncommitted": files.len(),
        "untracked": untracked,
        "files": files,
    }))
    .into_response()
}

/// GET /api/git/session-changes
/// Full unified diffs for every file this chat changed that still differs
/// from HEAD — the data behind the "Review changes" panel.
pub async fn session_changes(State(state): State<AppState>) -> Response {
    let (cwd, files) = session_files(&state).await;
    let out = tokio::task::spawn_blocking(move || {
        files
            .into_iter()
            .map(|f| {
                let diff = if f.status == FileStatus::Added {
                    // `--no-index` exits 1 when the files differ, so read
                    // stdout directly instead of going through `run`.
                    Command::new("git")
                        .current_dir(&cwd)
                        .args(["diff", "--no-index", "--", "/dev/null", &f.path])
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                        .unwrap_or_default()
                } else {
                    run(&cwd, &["diff", "--relative", "HEAD", "--", &f.path]).unwrap_or_default()
                };
                serde_json::json!({
                    "path": f.path,
                    "status": f.status,
                    "added": f.added,
                    "removed": f.removed,
                    "diff": diff,
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    Json(serde_json::json!({ "files": out })).into_response()
}

#[derive(Deserialize)]
pub struct RevertFileRequest {
    pub path: String,
}

/// POST /api/git/revert-file { path }
/// Throw away this chat's changes to one file: restore it from HEAD, or
/// delete it if the chat created it. Only files the chat changed are
/// accepted.
pub async fn revert_file(
    State(state): State<AppState>,
    Json(req): Json<RevertFileRequest>,
) -> Response {
    let (cwd, files) = session_files(&state).await;
    let Some(file) = files.iter().find(|f| f.path == req.path) else {
        return err(
            StatusCode::BAD_REQUEST,
            "not a file this session changed".into(),
        );
    };
    let result = if file.status == FileStatus::Added {
        std::fs::remove_file(cwd.join(&file.path))
    } else {
        run(
            &cwd,
            &[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                &file.path,
            ],
        )
        .map(|_| ())
    };
    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct CheckpointRequest {
    /// The message, as shown in the transcript.
    pub text: String,
    /// Which match of `text`, counted from the latest.
    #[serde(default)]
    pub occurrence: usize,
}

#[derive(Deserialize)]
pub struct UndoRestoreRequest {
    pub undo: String,
}

/// Run a checkpoint operation for the active chat, off the runtime.
async fn with_checkpoints<T: Serialize + Send + 'static>(
    state: &AppState,
    op: impl FnOnce(&Path, &str) -> Result<T, String> + Send + 'static,
) -> Response {
    let slot = state.active_slot().await;
    let cwd = slot.cwd.read().await.clone();
    let session = slot.id.to_string();
    match tokio::task::spawn_blocking(move || op(&cwd, &session)).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /api/checkpoints/preview { text, occurrence }
/// What restoring the files to before that message would change.
pub async fn checkpoint_preview(
    State(state): State<AppState>,
    Json(req): Json<CheckpointRequest>,
) -> Response {
    with_checkpoints(&state, move |cwd, session| {
        crate::checkpoints::preview(cwd, session, &req.text, req.occurrence)
            .map(|changes| serde_json::json!({ "changes": changes }))
    })
    .await
}

/// POST /api/checkpoints/restore { text, occurrence }
/// Put the files back the way they were before that message.
pub async fn checkpoint_restore(
    State(state): State<AppState>,
    Json(req): Json<CheckpointRequest>,
) -> Response {
    with_checkpoints(&state, move |cwd, session| {
        crate::checkpoints::restore(cwd, session, &req.text, req.occurrence)
    })
    .await
}

/// POST /api/checkpoints/undo { undo }
/// Undo a restore.
pub async fn checkpoint_undo(
    State(state): State<AppState>,
    Json(req): Json<UndoRestoreRequest>,
) -> Response {
    with_checkpoints(&state, move |cwd, session| {
        crate::checkpoints::undo(cwd, session, &req.undo)
    })
    .await
}

/// A commit mentioned in a transcript, for its hover card.
#[derive(Debug, Serialize, PartialEq)]
pub struct CommitCard {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub body: String,
    pub author: String,
    /// Unix seconds.
    pub date: i64,
    pub files: Vec<CommitFile>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CommitFile {
    pub path: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// Separates the header fields of `git show --format`; can't appear in them.
const SEP: char = '\u{1f}';

fn parse_commit(out: &str) -> Option<CommitCard> {
    let (head, stat) = out.split_once("\u{1e}").unwrap_or((out, ""));
    let mut f = head.split(SEP);
    let (sha, short, author, date, subject, body) = (
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next().unwrap_or(""),
    );
    let files = stat
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            let (a, r, path) = (p.next()?, p.next()?, p.next()?);
            Some(CommitFile {
                path: path.to_string(),
                added: a.parse().ok(),
                removed: r.parse().ok(),
            })
        })
        .collect();
    Some(CommitCard {
        sha: sha.trim().to_string(),
        short: short.trim().to_string(),
        subject: subject.to_string(),
        body: body.trim().to_string(),
        author: author.to_string(),
        date: date.trim().parse().unwrap_or(0),
        files,
    })
}

/// GET /api/git/commit/:sha — a commit in the chat's repo: message, author,
/// date and files changed.
pub async fn commit_card(
    State(state): State<AppState>,
    axum::extract::Path(sha): axum::extract::Path<String>,
) -> Response {
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return err(StatusCode::BAD_REQUEST, "not a commit hash".into());
    }
    let cwd = state.current_cwd().await;
    let format = format!("--format=%H{SEP}%h{SEP}%an{SEP}%at{SEP}%s{SEP}%b%x1e");
    let out = tokio::task::spawn_blocking(move || {
        run(
            &cwd,
            &[
                "show",
                "--no-color",
                "--numstat",
                &format,
                &format!("{sha}^{{commit}}"),
                "--",
            ],
        )
    })
    .await;
    match out {
        Ok(Ok(text)) => match parse_commit(&text) {
            Some(card) => Json(card).into_response(),
            None => err(StatusCode::NOT_FOUND, "no such commit".into()),
        },
        _ => err(
            StatusCode::NOT_FOUND,
            "no such commit in this repository".into(),
        ),
    }
}

#[derive(Deserialize)]
pub struct ApplyPatchRequest {
    pub patch: String,
    /// Only check whether it applies.
    #[serde(default)]
    pub check: bool,
}

/// Run `git apply` on a patch in `cwd`, feeding it on stdin.
fn git_apply(cwd: &Path, patch: &str, check: bool) -> Result<(), String> {
    use std::io::Write;
    let mut args = vec!["apply", "--whitespace=nowarn", "--recount"];
    if check {
        args.push("--check");
    }
    let mut child = Command::new("git")
        .current_dir(cwd)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut text = patch.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .trim_start_matches("error: ")
            .to_string())
    }
}

/// POST /api/git/apply { patch, check } — apply a diff a reply proposed,
/// in the chat's folder. `git apply` refuses paths outside the repository
/// and applies all of the patch or none of it.
pub async fn apply_patch(
    State(state): State<AppState>,
    Json(req): Json<ApplyPatchRequest>,
) -> Response {
    if req.patch.len() > 2_000_000 {
        return err(StatusCode::PAYLOAD_TOO_LARGE, "patch too large".into());
    }
    let cwd = state.current_cwd().await;
    let check = req.check;
    let res = tokio::task::spawn_blocking(move || git_apply(&cwd, &req.patch, check)).await;
    match res {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => err(StatusCode::CONFLICT, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Where a name is defined.
#[derive(Debug, Serialize, PartialEq)]
pub struct SymbolHit {
    pub path: String,
    pub line: u32,
    /// The defining line, trimmed.
    pub preview: String,
}

/// Definitions of `name` in the repo at `cwd`, best first: the common
/// declaration forms across Rust, TS/JS, Python, Go, Swift, Kotlin, Java.
fn find_definitions(cwd: &Path, name: &str) -> Vec<SymbolHit> {
    let n = regex_escape(name);
    let pattern = format!(
        r"(^|[^[:alnum:]_])(fn|function|def|class|struct|enum|trait|interface|type|impl|const|let|var|static|mod|func|object|protocol)[[:space:]]+{n}([^[:alnum:]_]|$)"
    );
    let Ok(out) = run(
        cwd,
        &["grep", "-n", "-I", "-E", "--full-name", "-e", &pattern],
    ) else {
        return Vec::new();
    };
    let mut hits: Vec<SymbolHit> = out
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, ':');
            let (path, line, text) = (p.next()?, p.next()?.parse().ok()?, p.next()?);
            // A declaration inside a string or a comment isn't one.
            let at = text.find(name)?;
            let before = &text[..at];
            if before.matches('"').count() % 2 == 1
                || before.contains("//")
                || before.trim_start().starts_with('#')
                || before.trim_start().starts_with('*')
            {
                return None;
            }
            Some(SymbolHit {
                path: path.to_string(),
                line,
                preview: text.trim().chars().take(160).collect(),
            })
        })
        .collect();
    // Source before tests and docs; declarations before `let`/`const`.
    let rank = |h: &SymbolHit| {
        let test = h.path.contains("test") || h.path.ends_with(".md");
        let weak = h.preview.starts_with("let ") || h.preview.starts_with("var ");
        (test, weak, h.path.len())
    };
    hits.sort_by_key(rank);
    hits.truncate(8);
    hits
}

fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if r"\.^$|?*+()[]{}".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

#[derive(Deserialize)]
pub struct SymbolQuery {
    pub name: String,
}

/// GET /api/symbol?name=… — where a name mentioned in a reply is defined.
pub async fn find_symbol(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SymbolQuery>,
) -> Response {
    let name = q.name.trim().trim_end_matches("()").to_string();
    let ok = (2..=80).contains(&name.len())
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !ok {
        return Json(Vec::<SymbolHit>::new()).into_response();
    }
    let cwd = state.current_cwd().await;
    let hits = tokio::task::spawn_blocking(move || find_definitions(&cwd, &name))
        .await
        .unwrap_or_default();
    Json(hits).into_response()
}

/// A branch mentioned in a transcript, for its chip and hover card.
#[derive(Debug, Serialize, PartialEq)]
pub struct BranchCard {
    pub name: String,
    /// `local` or `remote` (only on `origin`).
    pub location: &'static str,
    /// Checked out in the chat's folder right now.
    pub current: bool,
    pub base: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub last_subject: String,
    pub last_short: String,
    pub last_author: String,
    /// Unix seconds.
    pub last_date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<serde_json::Value>,
}

fn rev_exists(cwd: &Path, rev: &str) -> bool {
    run(
        cwd,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .is_ok()
}

/// The branch others are measured against: origin's default, else main or
/// master.
fn base_branch(cwd: &Path) -> Option<String> {
    if let Ok(head) = run(
        cwd,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let head = head.trim().trim_start_matches("origin/").to_string();
        if !head.is_empty() {
            return Some(head);
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| rev_exists(cwd, &format!("refs/heads/{b}")))
        .map(str::to_string)
}

fn branch_card(cwd: &Path, name: &str) -> Option<BranchCard> {
    let name = name.trim_start_matches("origin/");
    let (rev, location) = if rev_exists(cwd, &format!("refs/heads/{name}")) {
        (format!("refs/heads/{name}"), "local")
    } else if rev_exists(cwd, &format!("refs/remotes/origin/{name}")) {
        (format!("refs/remotes/origin/{name}"), "remote")
    } else {
        return None;
    };
    let current = run(cwd, &["branch", "--show-current"]).is_ok_and(|b| b.trim() == name);
    let base = base_branch(cwd);
    let (mut ahead, mut behind) = (0, 0);
    if let Some(b) = base.as_deref().filter(|b| *b != name) {
        let base_rev = if rev_exists(cwd, &format!("refs/heads/{b}")) {
            format!("refs/heads/{b}")
        } else {
            format!("refs/remotes/origin/{b}")
        };
        if let Ok(out) = run(
            cwd,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{base_rev}...{rev}"),
            ],
        ) {
            let mut it = out.split_whitespace();
            behind = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            ahead = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    let log = run(
        cwd,
        &[
            "log",
            "-1",
            &format!("--format=%h{SEP}%s{SEP}%an{SEP}%at"),
            &rev,
        ],
    )
    .ok()?;
    let mut f = log.trim_end().split(SEP);
    Some(BranchCard {
        name: name.to_string(),
        location,
        current,
        base,
        ahead,
        behind,
        last_short: f.next()?.to_string(),
        last_subject: f.next()?.to_string(),
        last_author: f.next()?.to_string(),
        last_date: f.next()?.parse().unwrap_or(0),
        pr: None,
    })
}

#[derive(Deserialize)]
pub struct BranchQuery {
    pub name: String,
    /// Also look up the branch's PR (slower: a GitHub call).
    #[serde(default)]
    pub pr: bool,
}

/// GET /api/git/branch?name=…[&pr=true] — a branch in the chat's repo:
/// last commit, ahead/behind its base, and optionally its PR. 404 when no
/// such branch exists, which is how the client tells a branch name from an
/// ordinary word.
pub async fn branch_info(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<BranchQuery>,
) -> Response {
    let name = q.name.trim().to_string();
    let ok = (1..=200).contains(&name.len())
        && !name.starts_with('-')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "/-_.".contains(c));
    if !ok {
        return err(StatusCode::NOT_FOUND, "not a branch".into());
    }
    let cwd = state.current_cwd().await;
    let want_pr = q.pr;
    let card = tokio::task::spawn_blocking(move || {
        let mut card = branch_card(&cwd, &name)?;
        if want_pr {
            // Best effort, through the signed-in gh CLI.
            card.pr = Command::new("gh")
                .current_dir(&cwd)
                .args([
                    "pr",
                    "view",
                    &card.name,
                    "--json",
                    "number,title,state,isDraft,url",
                ])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_slice(&o.stdout).ok());
        }
        Some(card)
    })
    .await
    .ok()
    .flatten();
    match card {
        Some(c) => Json(c).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such branch".into()),
    }
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

    // Only this chat's files — the panel offers to commit what the chat
    // changed, not whatever else is dirty in the repo. New files go in only
    // when asked for.
    let (_, files) = session_files(&state).await;
    let paths: Vec<&str> = files
        .iter()
        .filter(|f| req.include_unstaged || f.status != FileStatus::Added)
        .map(|f| f.path.as_str())
        .collect();
    if paths.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "nothing from this session to commit".into(),
        );
    }
    let mut add = vec!["add", "-A", "--"];
    add.extend(&paths);
    if let Err(e) = run(&cwd, &add) {
        return err(StatusCode::BAD_REQUEST, format!("git add: {e}"));
    }

    let message = if req.message.trim().is_empty() {
        let names: Vec<&str> = paths
            .iter()
            .map(|p| p.rsplit('/').next().unwrap_or(p))
            .take(3)
            .collect();
        let more = paths.len().saturating_sub(names.len());
        if more > 0 {
            format!("update {} and {more} more", names.join(", "))
        } else {
            format!("update {}", names.join(", "))
        }
    } else {
        req.message.trim().to_string()
    };

    // `--only`: commit exactly these paths, leaving anything else the
    // user had staged where it was.
    let commit_out = Command::new("git")
        .current_dir(&cwd)
        .args(["commit", "-m", &message, "--only", "--"])
        .args(&paths)
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

#[cfg(test)]
mod commit_tests {
    use super::*;

    #[test]
    fn a_real_commit_is_described() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let g = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .current_dir(p)
                .args([
                    "-c",
                    "user.name=Ada",
                    "-c",
                    "user.email=a@x",
                    "-c",
                    "commit.gpgsign=false"
                ])
                .args(args)
                .status()
                .unwrap()
                .success())
        };
        g(&["init", "-q", "--template="]);
        std::fs::write(p.join("a.rs"), "one\ntwo\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "Add a\n\nWhy it exists."]);
        let format = format!("--format=%H{SEP}%h{SEP}%an{SEP}%at{SEP}%s{SEP}%b%x1e");
        let out = run(
            p,
            &[
                "show",
                "--no-color",
                "--numstat",
                &format,
                "HEAD^{commit}",
                "--",
            ],
        )
        .unwrap();
        let c = parse_commit(&out).unwrap();
        assert_eq!(
            (c.subject.as_str(), c.body.as_str(), c.author.as_str()),
            ("Add a", "Why it exists.", "Ada")
        );
        assert_eq!(
            c.files,
            vec![CommitFile {
                path: "a.rs".into(),
                added: Some(2),
                removed: Some(0)
            }]
        );
        assert!(c.sha.starts_with(&c.short) && c.date > 0);
    }

    #[test]
    fn a_patch_checks_then_applies_all_or_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        assert!(std::process::Command::new("git")
            .current_dir(p)
            .args(["init", "-q", "--template="])
            .status()
            .unwrap()
            .success());
        std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        let patch = "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
        git_apply(p, patch, true).expect("applies");
        assert_eq!(
            std::fs::read_to_string(p.join("a.txt")).unwrap(),
            "one\ntwo\n",
            "check changes nothing"
        );
        git_apply(p, patch, false).expect("applied");
        assert_eq!(
            std::fs::read_to_string(p.join("a.txt")).unwrap(),
            "one\nTWO\n"
        );
        // Applying again no longer fits, and says why.
        assert!(git_apply(p, patch, true).is_err());
        // Paths outside the folder are refused.
        let escape = "--- a/../x.txt\n+++ b/../x.txt\n@@ -0,0 +1 @@\n+x\n";
        assert!(git_apply(p, escape, false).is_err());
    }

    #[test]
    fn definitions_are_found_and_ranked() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        assert!(std::process::Command::new("git")
            .current_dir(p)
            .args(["init", "-q", "--template="])
            .status()
            .unwrap()
            .success());
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("tests")).unwrap();
        std::fs::write(
            p.join("src/a.rs"),
            "use x;\n\npub fn spawn_turn(s: u8) {}\nfn spawn_turns() {}\n",
        )
        .unwrap();
        std::fs::write(p.join("tests/t.rs"), "fn spawn_turn() {}\n").unwrap();
        std::fs::write(p.join("src/b.ts"), "export function spawnTurn() {}\n").unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(p)
            .args(["add", "-A"])
            .status()
            .unwrap()
            .success());
        let hits = find_definitions(p, "spawn_turn");
        let got: Vec<_> = hits.iter().map(|h| (h.path.as_str(), h.line)).collect();
        assert_eq!(
            got,
            [("src/a.rs", 3), ("tests/t.rs", 1)],
            "exact name only, source first"
        );
        assert_eq!(find_definitions(p, "spawnTurn")[0].path, "src/b.ts");
        assert!(find_definitions(p, "nothing_here").is_empty());
        // Mentions in strings and comments aren't definitions.
        std::fs::write(
            p.join("src/c.rs"),
            "let s = \"fn quoted_name() {}\";\n// fn commented_name()\n",
        )
        .unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(p)
            .args(["add", "-A"])
            .status()
            .unwrap()
            .success());
        assert!(find_definitions(p, "quoted_name").is_empty());
        assert!(find_definitions(p, "commented_name").is_empty());
    }

    #[test]
    fn branches_are_described_and_words_are_not() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let g = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .current_dir(p)
                    .args([
                        "-c",
                        "user.name=Ada",
                        "-c",
                        "user.email=a@x",
                        "-c",
                        "commit.gpgsign=false"
                    ])
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "git {args:?}"
            )
        };
        g(&["init", "-q", "--template=", "-b", "main"]);
        std::fs::write(p.join("a"), "1").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "base"]);
        g(&["checkout", "-q", "-b", "feat/cli-oauth"]);
        std::fs::write(p.join("a"), "2").unwrap();
        g(&["commit", "-qam", "Add OAuth"]);
        std::fs::write(p.join("a"), "3").unwrap();
        g(&["commit", "-qam", "Refresh tokens"]);
        let c = branch_card(p, "feat/cli-oauth").expect("a branch");
        assert_eq!(
            (c.ahead, c.behind, c.current, c.location),
            (2, 0, true, "local")
        );
        assert_eq!(c.base.as_deref(), Some("main"));
        assert_eq!(c.last_subject, "Refresh tokens");
        assert!(
            branch_card(p, "origin/feat/cli-oauth").is_some(),
            "origin/ prefix accepted"
        );
        assert!(
            branch_card(p, "explanation").is_none(),
            "a word isn't a branch"
        );
    }
}

#[derive(Deserialize)]
pub struct TurnFileDiffRequest { pub text: String, #[serde(default)] pub occurrence: usize, pub path: String }
pub async fn turn_file_diff(
    State(state): State<AppState>, axum::extract::Path(id): axum::extract::Path<String>, Json(req): Json<TurnFileDiffRequest>,
) -> Response {
    let Some(slot) = state.slot_str(&id).await else { return err(StatusCode::NOT_FOUND, "chat is not loaded".into()); };
    let cwd = slot.cwd.read().await.clone();
    match tokio::task::spawn_blocking(move || crate::checkpoints::turn_file_diff(&cwd, &id, &req.text, req.occurrence, &req.path)).await {
        Ok(Ok(diff)) => Json(diff).into_response(),
        Ok(Err(error)) => err(StatusCode::BAD_REQUEST, error),
        Err(error) => err(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}
