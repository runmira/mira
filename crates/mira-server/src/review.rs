//! `POST /api/review` — kicks off a two-stage review and streams progress
//! frames over the existing WS broadcast.
//!
//! The response body is only a small acknowledgement (`{ run_id }`); the
//! real payload arrives asynchronously as `review_progress` / `review_result`
//! / `review_error` frames on the shared events channel, tagged with the
//! same `run_id` so the frontend can correlate multiple overlapping runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_review::{Progress, ProgressSink};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;

use crate::protocol::ServerMsg;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ReviewRequest {
    /// Git range (e.g. `main...HEAD`). Only consulted when neither `diff`
    /// nor `pr` is provided.
    #[serde(default)]
    pub range: Option<String>,
    /// Raw unified diff. Highest precedence — skips shell-out entirely.
    #[serde(default)]
    pub diff: Option<String>,
    /// GitHub PR number. Fetches the diff via `gh pr diff <N>` when
    /// `owner`/`repo` are missing; via the REST API (`application/vnd.github.v3.diff`)
    /// when they're present and a `GITHUB_TOKEN` is configured. The REST
    /// path is what the PullRequestPanel uses so "Review with Mira" works
    /// on any repo Mira knows about, regardless of the session's cwd.
    #[serde(default)]
    pub pr: Option<u32>,
    /// Repo owner for the REST diff-fetch path. Ignored unless `pr` is set.
    #[serde(default)]
    pub owner: Option<String>,
    /// Repo name for the REST diff-fetch path. Ignored unless `pr` is set.
    #[serde(default)]
    pub repo: Option<String>,
    /// If true, skip stage 2 (hostile re-verify). Faster, noisier. Should
    /// be set for cross-repo reviews since stage-2 opens local files that
    /// don't exist for a remote PR.
    #[serde(default)]
    pub no_verify: bool,
}

#[derive(Debug, Serialize)]
pub struct ReviewAck {
    pub run_id: String,
}

pub async fn start_review(
    State(state): State<AppState>,
    Json(req): Json<ReviewRequest>,
) -> Response {
    let cwd = state.current_cwd().await;
    let diff = match collect_diff_async(&req, &cwd).await {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("collect diff: {e}")),
    };
    if diff.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "no diff to review".into());
    }

    let run_id = Uuid::new_v4().to_string();
    let events_tx = state.events_tx.clone();
    let provider = state.harness_provider.clone();
    let model = state.current_session().await.config().await.model;
    let verify = !req.no_verify;

    // Announce, then hand off to a background task so the HTTP request
    // returns immediately — the browser subscribes to the broadcast and
    // sees the rest as it happens.
    let _ = events_tx.send(ServerMsg::ReviewStarted {
        run_id: run_id.clone(),
    });
    let run_id_bg = run_id.clone();
    let cwd_bg = cwd.clone();
    tokio::spawn(async move {
        let sink = BroadcastSink {
            run_id: run_id_bg.clone(),
            tx: events_tx.clone(),
        };
        match mira_review::review(&*provider, &model, &diff, &cwd_bg, verify, &sink).await {
            Ok(findings) => {
                info!(run_id = %run_id_bg, count = findings.len(), "review finished");
                let _ = events_tx.send(ServerMsg::ReviewResult {
                    run_id: run_id_bg,
                    findings,
                });
            }
            Err(e) => {
                warn!(run_id = %run_id_bg, %e, "review failed");
                let _ = events_tx.send(ServerMsg::ReviewError {
                    run_id: run_id_bg,
                    text: format!("{e:#}"),
                });
            }
        }
    });

    Json(ReviewAck { run_id }).into_response()
}

/* ---------- progress fan-out ---------- */

struct BroadcastSink {
    run_id: String,
    tx: broadcast::Sender<ServerMsg>,
}

#[async_trait]
impl ProgressSink for BroadcastSink {
    async fn emit(&self, event: Progress) {
        let _ = self.tx.send(ServerMsg::ReviewProgress {
            run_id: self.run_id.clone(),
            event,
        });
    }
}

/* ---------- diff sourcing (mirrors CLI logic) ---------- */

async fn collect_diff_async(req: &ReviewRequest, cwd: &Path) -> anyhow::Result<String> {
    if let Some(d) = &req.diff {
        return Ok(d.clone());
    }
    if let Some(pr) = req.pr {
        // Prefer the REST fetch when `owner`/`repo` are supplied — the
        // PullRequestPanel takes this path so it works even when the
        // session isn't parked in that repo's local checkout. Fall back
        // to `gh pr diff` when the frontend only knows the PR number and
        // trusts that the user is already on the right cwd.
        if let (Some(owner), Some(repo)) = (req.owner.as_deref(), req.repo.as_deref()) {
            if let Some(token) = crate::pull_requests::resolve_github_token() {
                return fetch_pr_diff_rest(&token, owner, repo, pr).await;
            }
        }
        return run_capture(cwd, &["gh", "pr", "diff", &pr.to_string()]);
    }
    // Check upfront so a missing repo produces a readable one-liner instead
    // of git's ~2KB `--no-index` usage dump.
    ensure_git_repo(cwd)?;
    let range = req
        .range
        .clone()
        .unwrap_or_else(|| default_range(cwd).unwrap_or_else(|| "HEAD".to_string()));
    // `-U15` widens the surrounding-context window from git's 3-line default.
    // The extra context is what lets the reviewer see the guard clauses,
    // helper calls, and type declarations that live 5-10 lines away from the
    // changed lines — without it the model has to guess about invariants.
    run_capture(cwd, &["git", "diff", "-U15", &range])
}

/// Fetch a PR's raw unified diff via GitHub REST. The `application/vnd.github.v3.diff`
/// Accept header flips the response body from JSON to a straight `.diff` payload,
/// which is exactly what `mira_review::review` wants.
async fn fetch_pr_diff_rest(
    token: &str,
    owner: &str,
    repo: &str,
    number: u32,
) -> anyhow::Result<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let resp = reqwest::Client::builder()
        .user_agent("mira-server")
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github.v3.diff")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("github {status}: {}", body.chars().take(200).collect::<String>());
    }
    Ok(resp.text().await?)
}

fn ensure_git_repo(cwd: &Path) -> anyhow::Result<()> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            // Surface git's own reason (e.g. "dubious ownership", "not a git
            // repository") — the plain "not a git repository" fallback masked
            // safe.directory failures on cloned/synced repos.
            let stderr = summarize_stderr(String::from_utf8_lossy(&o.stderr).trim());
            let reason = if stderr.is_empty() {
                "not a git repository".to_owned()
            } else {
                stderr
            };
            anyhow::bail!("git check failed in {}: {reason}", cwd.display())
        }
        Err(e) => anyhow::bail!("git not runnable in {}: {e}", cwd.display()),
    }
}

/// Pick a sensible default git-diff argument.
///
/// - On a feature branch that diverges from `main`/`master`: `<base>...HEAD`
///   so we review the branch's commits.
/// - Otherwise (on the base branch itself, or no base found): `None`, and the
///   caller falls back to `HEAD` — i.e. working tree vs HEAD, which catches
///   uncommitted work. Without this, reviewing on `main` with local mods
///   would diff `main...HEAD` (empty) and report "no diff to review".
fn default_range(cwd: &Path) -> Option<String> {
    for base in ["main", "master"] {
        if !branch_exists(cwd, base) {
            continue;
        }
        if is_current_branch(cwd, base) {
            // On the base branch itself — `<base>...HEAD` is trivially empty.
            return None;
        }
        return Some(format!("{base}...HEAD"));
    }
    None
}

fn branch_exists(cwd: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_current_branch(cwd: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == name)
        .unwrap_or(false)
}

fn run_capture(cwd: &Path, argv: &[&str]) -> anyhow::Result<String> {
    let out = Command::new(argv[0])
        .current_dir(cwd)
        .args(&argv[1..])
        .output()
        .map_err(|e| anyhow::anyhow!("spawn {}: {e}", argv.join(" ")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "{} exited {}: {}",
            argv.join(" "),
            out.status,
            summarize_stderr(stderr.trim()),
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Keep error output scannable in a JSON error field / UI panel: first
/// meaningful line + byte cap. Git's `--no-index` usage dump is 2KB alone.
fn summarize_stderr(s: &str) -> String {
    const LIMIT: usize = 500;
    let first = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first.len() >= LIMIT {
        format!("{}…", &first[..LIMIT])
    } else {
        first.to_owned()
    }
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

// Silence rustc's unused-import warning on Arc when the module has few uses.
#[allow(dead_code)]
fn _keep_arc(_: Arc<()>) {}
#[allow(dead_code)]
fn _keep_pathbuf(_: PathBuf) {}
