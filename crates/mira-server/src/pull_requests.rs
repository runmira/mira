//! Pull-request API — aggregates GitHub PRs across every project folder
//! Mira knows about, then exposes detail, comments, review, and merge
//! endpoints so the PullRequestPanel can drive them end-to-end.
//!
//! Authentication uses the `GITHUB_TOKEN` third-party key (settings.rs
//! surfaces it in the well-known list). The token flows through the
//! standard `Authorization: Bearer …` header GitHub expects. When the
//! token is missing every endpoint returns 401 so the UI can nudge the
//! user to open Settings.
//!
//! Repos are derived by walking every distinct cwd in the session store
//! and parsing its `origin` remote for `owner/repo`. Non-GitHub remotes
//! and non-repo folders drop out silently.
//!
//! `GET  /api/prs`                                    → grouped open PRs
//! `GET  /api/prs/:owner/:repo/:number`               → detail + activity
//! `GET  /api/prs/:owner/:repo/:number/files`         → file-level diff
//! `POST /api/prs/:owner/:repo/:number/comments`      → post issue comment
//! `POST /api/prs/:owner/:repo/:number/reviews`       → submit review
//! `PUT  /api/prs/:owner/:repo/:number/merge`         → merge

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::MiraConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::state::AppState;

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "mira-server";

/* ---------- view models sent to the browser ---------- */

#[derive(Debug, Serialize)]
pub struct PullRequestListView {
    /// Present when a token is configured — used by the UI to classify
    /// PRs as "authored" vs "reviewing" vs "other".
    pub authenticated_user: Option<String>,
    pub repos: Vec<RepoGroup>,
    /// Per-repo fetch errors (bad remote, 404, auth). The UI shows these
    /// inline so a single flaky repo doesn't hide the healthy ones.
    pub errors: Vec<RepoError>,
}

#[derive(Debug, Serialize)]
pub struct RepoGroup {
    pub owner: String,
    pub repo: String,
    /// The project folder basename this repo came from — matches the
    /// sidebar's "Projects" grouping so the UI can co-locate them.
    pub project_label: String,
    /// Absolute path of the project folder — lets the UI show a full
    /// tooltip when multiple folders resolve to the same repo.
    pub cwd: String,
    pub prs: Vec<PullRequestSummary>,
}

#[derive(Debug, Serialize)]
pub struct RepoError {
    pub owner: String,
    pub repo: String,
    pub cwd: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub author_avatar: Option<String>,
    pub head_ref: String,
    pub base_ref: String,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changed_files: Option<u64>,
    pub review_decision: Option<String>,
    pub comment_count: u64,
    /// GitHub logins of users whose review is currently requested.
    pub requested_reviewers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PullRequestDetailView {
    pub summary: PullRequestSummary,
    pub body: String,
    pub mergeable: Option<bool>,
    pub mergeable_state: Option<String>,
    pub merged: bool,
    /// `success` / `failure` / `pending` / `neutral` — rollup of check runs.
    pub check_status: Option<String>,
    pub checks: Vec<CheckRunView>,
    pub reviews: Vec<ReviewView>,
    pub comments: Vec<CommentView>,
    pub timeline: Vec<TimelineEventView>,
    pub commits: Vec<CommitView>,
}

#[derive(Debug, Serialize)]
pub struct CheckRunView {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewView {
    pub author: String,
    pub author_avatar: Option<String>,
    pub state: String,
    pub body: String,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommentView {
    pub author: String,
    pub author_avatar: Option<String>,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct TimelineEventView {
    pub kind: String,
    pub actor: Option<String>,
    pub message: String,
    pub at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommitView {
    pub sha: String,
    pub message: String,
    pub author: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PullRequestFilesView {
    pub files: Vec<FileChangeView>,
}

#[derive(Debug, Serialize)]
pub struct FileChangeView {
    pub filename: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
    pub raw_url: Option<String>,
}

/* ---------- request bodies ---------- */

#[derive(Debug, Deserialize)]
pub struct CommentBody {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ReviewBody {
    /// `APPROVE` | `REQUEST_CHANGES` | `COMMENT`
    pub event: String,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MergeBody {
    /// `merge` | `squash` | `rebase`. GitHub defaults to `merge` if omitted.
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub commit_title: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
}

/* ---------- handlers ---------- */

pub async fn list_pull_requests(State(state): State<AppState>) -> Response {
    let token = resolve_token();
    let repos = match discover_repos(&state).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("discover: {e}")),
    };

    let client = build_client();
    let authenticated_user = if let Some(t) = token.as_deref() {
        fetch_user(&client, t).await.ok()
    } else {
        None
    };

    let mut groups: Vec<RepoGroup> = Vec::new();
    let mut errors: Vec<RepoError> = Vec::new();
    for src in repos {
        if let Some(token) = token.as_deref() {
            match fetch_prs(&client, token, &src.owner, &src.repo).await {
                Ok(prs) => groups.push(RepoGroup {
                    owner: src.owner,
                    repo: src.repo,
                    project_label: src.label,
                    cwd: src.cwd,
                    prs,
                }),
                Err(e) => errors.push(RepoError {
                    owner: src.owner,
                    repo: src.repo,
                    cwd: src.cwd,
                    message: e,
                }),
            }
        } else {
            groups.push(RepoGroup {
                owner: src.owner,
                repo: src.repo,
                project_label: src.label,
                cwd: src.cwd,
                prs: Vec::new(),
            });
        }
    }

    // Ordering: repos with any open PRs first, then alphabetical. Keeps
    // the panel useful even when many folders are hooked up.
    groups.sort_by(|a, b| {
        (b.prs.is_empty().cmp(&a.prs.is_empty()))
            .then_with(|| a.project_label.cmp(&b.project_label))
    });

    Json(PullRequestListView {
        authenticated_user,
        repos: groups,
        errors,
    })
    .into_response()
}

pub async fn get_pull_request(
    State(_state): State<AppState>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
) -> Response {
    let Some(token) = resolve_token() else {
        return err(StatusCode::UNAUTHORIZED, missing_token_msg());
    };
    let client = build_client();
    let base = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls/{number}");

    let pr_val: Value = match gh_get(&client, &token, &base).await {
        Ok(v) => v,
        Err(e) => return proxied_error(&e),
    };
    let summary = summary_from(&pr_val);
    let body = pr_val
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mergeable = pr_val.get("mergeable").and_then(Value::as_bool);
    let mergeable_state = pr_val
        .get("mergeable_state")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let merged = pr_val
        .get("merged")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let head_sha = pr_val
        .get("head")
        .and_then(|h| h.get("sha"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Fan out the auxiliary lists. Each failure degrades to an empty list
    // rather than sinking the whole detail load — a missing check-runs
    // scope shouldn't make the PR itself un-viewable.
    let issue_comments_url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{number}/comments");
    let reviews_url = format!("{base}/reviews");
    let commits_url = format!("{base}/commits");
    let timeline_url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{number}/timeline");

    let comments: Vec<CommentView> = gh_get::<Vec<Value>>(&client, &token, &issue_comments_url)
        .await
        .map(|arr| arr.iter().map(comment_from).collect())
        .unwrap_or_default();
    let reviews: Vec<ReviewView> = gh_get::<Vec<Value>>(&client, &token, &reviews_url)
        .await
        .map(|arr| arr.iter().map(review_from).collect())
        .unwrap_or_default();
    let commits: Vec<CommitView> = gh_get::<Vec<Value>>(&client, &token, &commits_url)
        .await
        .map(|arr| arr.iter().map(commit_from).collect())
        .unwrap_or_default();
    let timeline: Vec<TimelineEventView> = gh_get::<Vec<Value>>(&client, &token, &timeline_url)
        .await
        .map(|arr| arr.iter().filter_map(timeline_from).collect())
        .unwrap_or_default();

    let (check_status, checks) = if head_sha.is_empty() {
        (None, Vec::new())
    } else {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/commits/{head_sha}/check-runs");
        match gh_get::<Value>(&client, &token, &url).await {
            Ok(v) => {
                let arr = v
                    .get("check_runs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let runs: Vec<CheckRunView> = arr.iter().map(check_run_from).collect();
                (Some(roll_up_checks(&runs)), runs)
            }
            Err(_) => (None, Vec::new()),
        }
    };

    let view = PullRequestDetailView {
        summary,
        body,
        mergeable,
        mergeable_state,
        merged,
        check_status,
        checks,
        reviews,
        comments,
        timeline,
        commits,
    };
    Json(view).into_response()
}

pub async fn get_pull_request_files(
    State(_state): State<AppState>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
) -> Response {
    let Some(token) = resolve_token() else {
        return err(StatusCode::UNAUTHORIZED, missing_token_msg());
    };
    let client = build_client();
    // GitHub caps at 30 files per page; we grab up to 100 which covers the
    // vast majority of PRs and keeps a single round trip.
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls/{number}/files?per_page=100");
    let arr: Vec<Value> = match gh_get(&client, &token, &url).await {
        Ok(v) => v,
        Err(e) => return proxied_error(&e),
    };
    let files: Vec<FileChangeView> = arr
        .iter()
        .map(|f| FileChangeView {
            filename: f
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            status: f
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("modified")
                .to_string(),
            additions: f.get("additions").and_then(Value::as_u64).unwrap_or(0),
            deletions: f.get("deletions").and_then(Value::as_u64).unwrap_or(0),
            patch: f.get("patch").and_then(Value::as_str).map(str::to_owned),
            raw_url: f.get("raw_url").and_then(Value::as_str).map(str::to_owned),
        })
        .collect();
    Json(PullRequestFilesView { files }).into_response()
}

pub async fn post_pull_request_comment(
    State(_state): State<AppState>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
    Json(body): Json<CommentBody>,
) -> Response {
    if body.body.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "comment body is empty".into());
    }
    let Some(token) = resolve_token() else {
        return err(StatusCode::UNAUTHORIZED, missing_token_msg());
    };
    let client = build_client();
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{number}/comments");
    let payload = serde_json::json!({ "body": body.body });
    match gh_post::<Value>(&client, &token, &url, &payload).await {
        Ok(v) => Json(comment_from(&v)).into_response(),
        Err(e) => proxied_error(&e),
    }
}

pub async fn post_pull_request_review(
    State(_state): State<AppState>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
    Json(body): Json<ReviewBody>,
) -> Response {
    let event = body.event.to_uppercase();
    if !matches!(
        event.as_str(),
        "APPROVE" | "REQUEST_CHANGES" | "COMMENT"
    ) {
        return err(
            StatusCode::BAD_REQUEST,
            format!("invalid review event: {}", body.event),
        );
    }
    let comment = body.body.unwrap_or_default();
    if event != "APPROVE" && comment.trim().is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "REQUEST_CHANGES / COMMENT reviews need a body".into(),
        );
    }
    let Some(token) = resolve_token() else {
        return err(StatusCode::UNAUTHORIZED, missing_token_msg());
    };
    let client = build_client();
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls/{number}/reviews");
    let payload = serde_json::json!({ "event": event, "body": comment });
    match gh_post::<Value>(&client, &token, &url, &payload).await {
        Ok(v) => Json(review_from(&v)).into_response(),
        Err(e) => proxied_error(&e),
    }
}

pub async fn merge_pull_request(
    State(_state): State<AppState>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
    Json(body): Json<MergeBody>,
) -> Response {
    let method = body.method.unwrap_or_else(|| "merge".to_string()).to_lowercase();
    if !matches!(method.as_str(), "merge" | "squash" | "rebase") {
        return err(
            StatusCode::BAD_REQUEST,
            format!("invalid merge method: {method}"),
        );
    }
    let Some(token) = resolve_token() else {
        return err(StatusCode::UNAUTHORIZED, missing_token_msg());
    };
    let client = build_client();
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls/{number}/merge");
    let mut payload = serde_json::json!({ "merge_method": method });
    if let Some(t) = body.commit_title {
        payload["commit_title"] = Value::String(t);
    }
    if let Some(m) = body.commit_message {
        payload["commit_message"] = Value::String(m);
    }
    match gh_put::<Value>(&client, &token, &url, &payload).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => proxied_error(&e),
    }
}

/* ---------- repo discovery ---------- */

struct RepoSource {
    owner: String,
    repo: String,
    label: String,
    cwd: String,
}

/// Enumerate every distinct project folder Mira knows about (the same set
/// the sidebar groups sessions by), resolve each to a `github.com` remote,
/// and dedupe by `owner/repo` so multi-worktree projects don't spawn N
/// duplicate rows.
async fn discover_repos(state: &AppState) -> Result<Vec<RepoSource>, String> {
    let Some(store) = state.store.clone() else {
        return Ok(Vec::new());
    };
    let records = store
        .list_all(500)
        .await
        .map_err(|e| format!("list_all: {e}"))?;
    // Preserve most-recent-first from the store ordering, but dedupe on
    // cwd so we don't kick off multiple parses for the same folder.
    let mut seen_cwd: BTreeSet<PathBuf> = BTreeSet::new();
    let mut cwds: Vec<PathBuf> = Vec::new();
    for r in records {
        if r.parent_id.is_some() {
            continue;
        }
        if seen_cwd.insert(r.cwd.clone()) {
            cwds.push(r.cwd);
        }
    }

    // Fold the currently active cwd in too, even if no session has been
    // saved yet (fresh install, first launch).
    let current = state.current_cwd().await;
    if seen_cwd.insert(current.clone()) {
        cwds.push(current);
    }

    let mut sources: HashMap<(String, String), RepoSource> = HashMap::new();
    for cwd in cwds {
        let Some((owner, repo)) = parse_github_remote(&cwd) else {
            continue;
        };
        let label = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.clone());
        // If we've already seen this owner/repo, keep the one whose folder
        // basename matches the repo — that's the "canonical" checkout.
        sources
            .entry((owner.clone(), repo.clone()))
            .and_modify(|existing| {
                if label == existing.repo && existing.label != existing.repo {
                    existing.label = label.clone();
                    existing.cwd = cwd.display().to_string();
                }
            })
            .or_insert(RepoSource {
                owner,
                repo,
                label,
                cwd: cwd.display().to_string(),
            });
    }

    let mut out: Vec<RepoSource> = sources.into_values().collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

/// Parse `git remote get-url origin` output into `(owner, repo)`. Supports
/// both `git@github.com:owner/repo(.git)` and `https://github.com/owner/repo(.git)`
/// forms. Anything else (bitbucket, gitlab, no remote, not a repo) returns
/// `None`.
fn parse_github_remote(cwd: &Path) -> Option<(String, String)> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim();
    let owner_repo = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("ssh://git@github.com/") {
        rest
    } else {
        return None;
    };
    let owner_repo = owner_repo.strip_suffix(".git").unwrap_or(owner_repo);
    let mut parts = owner_repo.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches('/').to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/* ---------- github REST helpers ---------- */

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn fetch_user(client: &reqwest::Client, token: &str) -> Result<String, String> {
    let v: Value = gh_get(client, token, &format!("{GITHUB_API}/user")).await?;
    v.get("login")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "no login field".to_string())
}

async fn fetch_prs(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<Vec<PullRequestSummary>, String> {
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls?state=open&per_page=50&sort=updated&direction=desc");
    let arr: Vec<Value> = gh_get(client, token, &url).await?;
    Ok(arr.iter().map(summary_from).collect())
}

async fn gh_get<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<T, GhError> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| GhError::network(e.to_string()))?;
    handle(resp).await
}

async fn gh_post<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<T, GhError> {
    let resp = client
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(body)
        .send()
        .await
        .map_err(|e| GhError::network(e.to_string()))?;
    handle(resp).await
}

async fn gh_put<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<T, GhError> {
    let resp = client
        .put(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(body)
        .send()
        .await
        .map_err(|e| GhError::network(e.to_string()))?;
    handle(resp).await
}

async fn handle<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T, GhError> {
    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<T>()
            .await
            .map_err(|e| GhError::network(format!("parse: {e}")));
    }
    let text = resp.text().await.unwrap_or_default();
    // GitHub returns `{"message": "..."}` for most errors.
    let msg = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| text.chars().take(200).collect());
    Err(GhError { status: Some(status.as_u16()), message: msg })
}

/// Fallible-only shim: `gh_get<Vec<X>>` above uses `Err(String)` in the
/// aggregate-list path so per-repo failures don't wreck the whole
/// response. `handle` returns a structured `GhError` so the detail
/// handlers can round-trip status codes; the coarser `fetch_prs` in the
/// list path stringifies the error before returning to keep its type
/// simple.
#[derive(Debug)]
struct GhError {
    status: Option<u16>,
    message: String,
}

impl GhError {
    fn network(m: String) -> Self {
        Self { status: None, message: m }
    }
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(code) => write!(f, "{code}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl From<GhError> for String {
    fn from(e: GhError) -> Self {
        e.to_string()
    }
}

fn proxied_error(e: &GhError) -> Response {
    let status = match e.status {
        Some(401) => StatusCode::UNAUTHORIZED,
        Some(403) => StatusCode::FORBIDDEN,
        Some(404) => StatusCode::NOT_FOUND,
        Some(422) => StatusCode::UNPROCESSABLE_ENTITY,
        Some(c) if c >= 500 => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_GATEWAY,
    };
    err(status, e.message.clone())
}

/* ---------- deserialization glue ---------- */

fn summary_from(v: &Value) -> PullRequestSummary {
    let user = v.get("user");
    let head_ref = v
        .get("head")
        .and_then(|h| h.get("ref"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_ref = v
        .get("base")
        .and_then(|b| b.get("ref"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let requested_reviewers = v
        .get("requested_reviewers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|u| u.get("login").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    PullRequestSummary {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        state: v
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("open")
            .to_string(),
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
        author: user
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        author_avatar: user
            .and_then(|u| u.get("avatar_url"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        head_ref,
        base_ref,
        html_url: v
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        created_at: v
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        updated_at: v
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        additions: v.get("additions").and_then(Value::as_u64),
        deletions: v.get("deletions").and_then(Value::as_u64),
        changed_files: v.get("changed_files").and_then(Value::as_u64),
        review_decision: None,
        comment_count: v.get("comments").and_then(Value::as_u64).unwrap_or(0),
        requested_reviewers,
    }
}

fn comment_from(v: &Value) -> CommentView {
    let user = v.get("user");
    CommentView {
        author: user
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        author_avatar: user
            .and_then(|u| u.get("avatar_url"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        body: v
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        created_at: v
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

fn review_from(v: &Value) -> ReviewView {
    let user = v.get("user");
    ReviewView {
        author: user
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        author_avatar: user
            .and_then(|u| u.get("avatar_url"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        state: v
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        body: v
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        submitted_at: v
            .get("submitted_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn commit_from(v: &Value) -> CommitView {
    let commit = v.get("commit");
    let author = commit
        .and_then(|c| c.get("author"))
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    CommitView {
        sha: v
            .get("sha")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(7)
            .collect(),
        message: commit
            .and_then(|c| c.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
        author,
    }
}

/// GitHub's issues timeline is a firehose of ~40 event kinds. Fold the
/// interesting ones into a small canonical shape (kind + actor + message)
/// so the UI can render an activity feed without switching per event.
fn timeline_from(v: &Value) -> Option<TimelineEventView> {
    let event = v.get("event").and_then(Value::as_str)?;
    let actor = v
        .get("actor")
        .and_then(|a| a.get("login"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let at = v
        .get("created_at")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (kind, message) = match event {
        "committed" => {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            ("commit".to_string(), msg)
        }
        "commented" => (
            "comment".to_string(),
            v.get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        "reviewed" => (
            "review".to_string(),
            v.get("state").and_then(Value::as_str).unwrap_or("").to_string(),
        ),
        "merged" => ("merged".to_string(), String::new()),
        "closed" => ("closed".to_string(), String::new()),
        "reopened" => ("reopened".to_string(), String::new()),
        "labeled" | "unlabeled" => {
            let name = v
                .get("label")
                .and_then(|l| l.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (event.to_string(), name)
        }
        "review_requested" => {
            let name = v
                .get("requested_reviewer")
                .and_then(|u| u.get("login"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default();
            ("review_requested".to_string(), name)
        }
        _ => return None,
    };
    Some(TimelineEventView {
        kind,
        actor,
        message,
        at,
    })
}

fn check_run_from(v: &Value) -> CheckRunView {
    CheckRunView {
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("queued")
            .to_string(),
        conclusion: v
            .get("conclusion")
            .and_then(Value::as_str)
            .map(str::to_owned),
        url: v
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// Reduce N check runs to one word for the summary pill. Failure trumps
/// pending, pending trumps success — mirrors the GitHub PR-page badge.
fn roll_up_checks(runs: &[CheckRunView]) -> String {
    if runs.is_empty() {
        return "none".to_string();
    }
    let mut has_failure = false;
    let mut has_pending = false;
    for r in runs {
        if r.status != "completed" {
            has_pending = true;
        }
        if let Some(c) = &r.conclusion {
            match c.as_str() {
                "failure" | "timed_out" | "cancelled" | "action_required" => has_failure = true,
                "success" | "neutral" | "skipped" => {}
                _ => {}
            }
        }
    }
    if has_failure {
        "failure".into()
    } else if has_pending {
        "pending".into()
    } else {
        "success".into()
    }
}

/* ---------- token + error plumbing ---------- */

fn resolve_token() -> Option<String> {
    resolve_github_token()
}

/// Public shim so other modules (review.rs) can grab the same token without
/// re-implementing the env/config lookup. Keeps the token-resolution rules
/// (env wins over yaml, yaml empty string means "unset") in one place.
pub fn resolve_github_token() -> Option<String> {
    if let Ok(v) = std::env::var("GITHUB_TOKEN") {
        if !v.is_empty() {
            return Some(v);
        }
    }
    let cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            warn!(%e, "prs: load_global failed");
            return None;
        }
    };
    cfg.keys
        .get("GITHUB_TOKEN")
        .filter(|s| !s.is_empty())
        .cloned()
}

fn missing_token_msg() -> String {
    "GITHUB_TOKEN is not set — add it in Settings → Keys.".to_string()
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
