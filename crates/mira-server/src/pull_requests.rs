//! Pull-request API — aggregates GitHub PRs across every project folder
//! Mira knows about, then exposes detail, comments, review, and merge
//! endpoints so the PullRequestPanel can drive them end-to-end.
//!
//! Authentication uses GH_TOKEN, GITHUB_TOKEN, or the saved github.com CLI
//! login. A rejected configured credential can recover once through that login;
//! missing scopes and resource failures are returned without replaying requests.
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

use crate::session_scope::ScopedState as State;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::extract::Path as AxumPath;
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
    let token = resolve_token().await;
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
    let Some(token) = resolve_token().await else {
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
    let Some(token) = resolve_token().await else {
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
    let Some(token) = resolve_token().await else {
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
    if !matches!(event.as_str(), "APPROVE" | "REQUEST_CHANGES" | "COMMENT") {
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
    let Some(token) = resolve_token().await else {
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
    let method = body
        .method
        .unwrap_or_else(|| "merge".to_string())
        .to_lowercase();
    if !matches!(method.as_str(), "merge" | "squash" | "rebase") {
        return err(
            StatusCode::BAD_REQUEST,
            format!("invalid merge method: {method}"),
        );
    }
    let Some(token) = resolve_token().await else {
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
pub(crate) fn parse_github_remote(cwd: &Path) -> Option<(String, String)> {
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
    let owner_repo = [
        "git@github.com:",
        "https://github.com/",
        "ssh://git@github.com/",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
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
    gh_request(client, reqwest::Method::GET, token, url, None).await
}

async fn gh_post<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<T, GhError> {
    gh_request(client, reqwest::Method::POST, token, url, Some(body)).await
}

async fn gh_put<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<T, GhError> {
    gh_request(client, reqwest::Method::PUT, token, url, Some(body)).await
}

async fn gh_request<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    method: reqwest::Method,
    token: &str,
    url: &str,
    body: Option<&Value>,
) -> Result<T, GhError> {
    let request = |token: &str| {
        let request = client
            .request(method.clone(), url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request.json(body)
        } else {
            request
        }
    };
    send_with_auth_retry(
        || request(token),
        |replacement| request(replacement),
        crate::github_auth::cli_token,
        token,
    )
    .await
}

/// Only 401 is safe to recover with another credential. A denied scope (403),
/// missing resource (404), network failure or rejected mutation is never replayed.
async fn send_with_auth_retry<T, F, G, H, Fut>(
    initial: F,
    replacement: G,
    fallback: H,
    original: &str,
) -> Result<T, GhError>
where
    T: serde::de::DeserializeOwned,
    F: FnOnce() -> reqwest::RequestBuilder,
    G: FnOnce(&str) -> reqwest::RequestBuilder,
    H: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    let response = initial()
        .send()
        .await
        .map_err(|e| GhError::network(e.to_string()))?;
    if response.status() != StatusCode::UNAUTHORIZED {
        return handle(response).await;
    }
    if let Some(token) = fallback().await.filter(|token| token != original) {
        let response = replacement(&token)
            .send()
            .await
            .map_err(|e| GhError::network(e.to_string()))?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return handle(response).await;
        }
    }
    Err(GhError {status:Some(401),message:"GitHub sign-in expired. Update the GitHub token in Settings → Search & keys, or run `gh auth login` on the machine running Mira.".into()})
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
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(200).collect());
    Err(GhError {
        status: Some(status.as_u16()),
        message: msg,
    })
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
        Self {
            status: None,
            message: m,
        }
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
            v.get("state")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
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
        url: v.get("html_url").and_then(Value::as_str).map(str::to_owned),
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

/// The hover card for a PR or issue mentioned in a transcript: one small
/// read, not the full detail view's six.
#[derive(Debug, Serialize, PartialEq)]
pub struct MentionCard {
    /// `pr` or `issue`.
    pub kind: &'static str,
    pub number: u64,
    pub title: String,
    /// `open`, `closed`, `merged` or `draft`.
    pub state: String,
    pub author: Option<String>,
    pub author_avatar: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changed_files: Option<u64>,
    /// ISO-8601.
    pub created_at: Option<String>,
    pub url: String,
}

fn card_from(kind: &'static str, v: &Value) -> MentionCard {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let merged = v.get("merged").and_then(Value::as_bool).unwrap_or(false)
        || v.get("merged_at").is_some_and(|m| !m.is_null());
    let draft = v.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let state = if merged {
        "merged".to_string()
    } else if draft && s("state").as_deref() == Some("open") {
        "draft".to_string()
    } else {
        s("state").unwrap_or_else(|| "open".into())
    };
    MentionCard {
        kind,
        number: n("number").unwrap_or(0),
        title: s("title").unwrap_or_default(),
        state,
        author: v
            .pointer("/user/login")
            .and_then(Value::as_str)
            .map(str::to_owned),
        author_avatar: v
            .pointer("/user/avatar_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        additions: n("additions"),
        deletions: n("deletions"),
        changed_files: n("changed_files"),
        created_at: s("created_at"),
        url: s("html_url").unwrap_or_default(),
    }
}

/// One GitHub REST read: with the stored token when there is one, else
/// through the `gh` CLI if it's signed in. `None` on a 404.
async fn github_read(path: &str) -> Result<Option<Value>, String> {
    if let Some(token) = resolve_token().await {
        let url = format!("{GITHUB_API}/{path}");
        return match gh_get::<Value>(&build_client(), &token, &url).await {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.status == Some(404) => Ok(None),
            Err(e) => Err(e.to_string()),
        };
    }
    Err(missing_token_msg())
}

/// One row of the composer's `#` picker.
#[derive(Debug, Serialize, PartialEq)]
pub struct IssueOption {
    /// `pr` or `issue`.
    pub kind: &'static str,
    pub number: u64,
    pub title: String,
    /// `open`, `closed`, `merged` or `draft`.
    pub state: String,
    pub author: Option<String>,
    pub labels: Vec<IssueLabel>,
    /// ISO-8601.
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct IssueLabel {
    pub name: String,
    /// Hex without `#`, as GitHub gives it.
    pub color: String,
}

#[derive(Debug, Serialize)]
pub struct IssueSearchView {
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub items: Vec<IssueOption>,
    /// Why there is nothing to pick from, in words for the picker.
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IssueSearchQuery {
    #[serde(default)]
    pub q: String,
}

/// An issue or PR from GitHub's issues API, which returns both.
fn issue_option(v: &Value) -> IssueOption {
    let is_pr = v.get("pull_request").is_some_and(|p| !p.is_null());
    let merged = v
        .pointer("/pull_request/merged_at")
        .is_some_and(|m| !m.is_null());
    let draft = v.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let state = v.get("state").and_then(Value::as_str).unwrap_or("open");
    IssueOption {
        kind: if is_pr { "pr" } else { "issue" },
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state: if merged {
            "merged".into()
        } else if draft && state == "open" {
            "draft".into()
        } else {
            state.into()
        },
        author: v
            .pointer("/user/login")
            .and_then(Value::as_str)
            .map(str::to_owned),
        labels: v
            .get("labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| {
                Some(IssueLabel {
                    name: l.get("name")?.as_str()?.to_string(),
                    color: l
                        .get("color")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .take(3)
            .collect(),
        updated_at: v
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// The GitHub read behind a `#` query: recent open issues and PRs when
/// empty, that number (then title matches) when numeric, a title search
/// otherwise.
fn issue_search_paths(owner: &str, repo: &str, q: &str) -> Vec<String> {
    let q = q.trim();
    let recent = format!("repos/{owner}/{repo}/issues?state=open&sort=updated&per_page=30");
    if q.is_empty() {
        return vec![recent];
    }
    let terms: String = url_encode(&format!("{q} repo:{owner}/{repo}"));
    let search = format!("search/issues?q={terms}&sort=updated&per_page=20");
    if q.chars().all(|c| c.is_ascii_digit()) {
        return vec![format!("repos/{owner}/{repo}/issues/{q}"), search];
    }
    vec![search]
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// GET /api/issues?q= — issues and PRs of the chat's GitHub repo, for the
/// composer's `#` picker.
pub async fn search_issues(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<IssueSearchQuery>,
) -> Response {
    let cwd = state.current_cwd().await;
    let remote = tokio::task::spawn_blocking(move || parse_github_remote(&cwd))
        .await
        .ok()
        .flatten();
    let Some((owner, repo)) = remote else {
        return Json(IssueSearchView {
            owner: None,
            repo: None,
            items: Vec::new(),
            error: Some("This chat's folder isn't a GitHub repository.".into()),
        })
        .into_response();
    };
    let mut items: Vec<IssueOption> = Vec::new();
    let mut error = None;
    for path in issue_search_paths(&owner, &repo, &query.q) {
        match github_read(&path).await {
            Ok(Some(v)) => {
                let found: Vec<IssueOption> = match v.get("items").or(Some(&v)) {
                    Some(Value::Array(list)) => list.iter().map(issue_option).collect(),
                    Some(one) if one.get("number").is_some() => vec![issue_option(one)],
                    _ => Vec::new(),
                };
                for item in found {
                    if !items.iter().any(|i| i.number == item.number) {
                        items.push(item);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    Json(IssueSearchView {
        owner: Some(owner),
        repo: Some(repo),
        error: items.is_empty().then_some(error).flatten(),
        items,
    })
    .into_response()
}

/// GET /api/prs/:owner/:repo/:number/card — a PR, or else an issue.
pub async fn mention_card(
    AxumPath((owner, repo, number)): AxumPath<(String, String, u64)>,
) -> Response {
    let ok = |c: &str| {
        !c.is_empty()
            && c.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    if !ok(&owner) || !ok(&repo) {
        return err(StatusCode::BAD_REQUEST, "bad repository".into());
    }
    let pr = github_read(&format!("repos/{owner}/{repo}/pulls/{number}")).await;
    let found = match pr {
        Ok(Some(v)) => Ok(Some(card_from("pr", &v))),
        Ok(None) => github_read(&format!("repos/{owner}/{repo}/issues/{number}"))
            .await
            .map(|v| v.map(|v| card_from("issue", &v))),
        Err(e) => Err(e),
    };
    match found {
        Ok(Some(card)) => Json(card).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not found".into()),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

async fn resolve_token() -> Option<String> {
    match resolve_github_token() {
        Some(token) => Some(token),
        None => crate::github_auth::cli_token().await,
    }
}

/// Public shim so other modules (review.rs) can grab the same token without
/// re-implementing the env/config lookup. Keeps the token-resolution rules
/// (env wins over yaml, yaml empty string means "unset") in one place.
pub fn resolve_github_token() -> Option<String> {
    // Match GitHub CLI precedence, including users who configure GH_TOKEN.
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
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
    "Connect GitHub using `gh auth login` on the machine running Mira, or add a GitHub token in Settings → Search & keys.".to_string()
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod mention_tests {
    use super::*;

    #[test]
    fn merged_draft_and_open_are_told_apart() {
        let pr = serde_json::json!({
            "number": 93, "title": "Browser: Mira manages its own Chromium", "state": "open",
            "draft": false, "merged": false, "user": {"login": "DamilolaDami", "avatar_url": "https://a/x.png"},
            "additions": 850, "deletions": 85, "changed_files": 12,
            "created_at": "2026-10-01T13:00:00Z", "html_url": "https://github.com/runmira/mira/pull/93"
        });
        let c = card_from("pr", &pr);
        assert_eq!(
            (c.state.as_str(), c.additions, c.deletions, c.changed_files),
            ("open", Some(850), Some(85), Some(12))
        );
        assert_eq!(c.author.as_deref(), Some("DamilolaDami"));
        let mut merged = pr.clone();
        merged["state"] = "closed".into();
        merged["merged_at"] = "2026-10-01T14:00:00Z".into();
        assert_eq!(card_from("pr", &merged).state, "merged");
        let mut draft = pr.clone();
        draft["draft"] = true.into();
        assert_eq!(card_from("pr", &draft).state, "draft");
        // An issue has no diff stats.
        let issue = serde_json::json!({"number": 70, "title": "Context inspector", "state": "closed", "html_url": "u"});
        let i = card_from("issue", &issue);
        assert_eq!(
            (i.kind, i.state.as_str(), i.additions),
            ("issue", "closed", None)
        );
    }
}

#[cfg(test)]
mod auth_recovery_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn issue_queries_pick_recent_by_number_or_search() {
        assert_eq!(
            issue_search_paths("runmira", "mira", " "),
            vec!["repos/runmira/mira/issues?state=open&sort=updated&per_page=30"]
        );
        assert_eq!(
            issue_search_paths("runmira", "mira", "205"),
            vec![
                "repos/runmira/mira/issues/205".to_string(),
                "search/issues?q=205+repo%3Arunmira%2Fmira&sort=updated&per_page=20".into(),
            ]
        );
        assert_eq!(
            issue_search_paths("runmira", "mira", "handoff budget"),
            vec!["search/issues?q=handoff+budget+repo%3Arunmira%2Fmira&sort=updated&per_page=20"]
        );
    }

    #[test]
    fn issue_rows_tell_prs_from_issues_and_their_state() {
        let issue = serde_json::json!({
            "number": 205, "title": "Context handoff", "state": "open",
            "user": {"login": "dami"},
            "labels": [{"name": "enhancement", "color": "a2eeef"}],
            "updated_at": "2026-10-15T10:00:00Z"
        });
        let row = issue_option(&issue);
        assert_eq!((row.kind, row.number, row.state.as_str()), ("issue", 205, "open"));
        assert_eq!(row.labels[0].name, "enhancement");
        let merged = serde_json::json!({
            "number": 207, "title": "x", "state": "closed",
            "pull_request": {"merged_at": "2026-10-15T10:54:08Z"}
        });
        assert_eq!(issue_option(&merged).kind, "pr");
        assert_eq!(issue_option(&merged).state, "merged");
        let draft = serde_json::json!({"number": 1, "state": "open", "draft": true, "pull_request": {"merged_at": null}});
        assert_eq!(issue_option(&draft).state, "draft");
    }

    #[tokio::test]
    async fn rejected_credentials_retry_once_but_scope_and_missing_resource_do_not() {
        for status in [401u16, 403, 404] {
            for method in [
                reqwest::Method::GET,
                reqwest::Method::POST,
                reqwest::Method::PUT,
            ] {
                let calls = Arc::new(AtomicUsize::new(0));
                let seen = calls.clone();
                let router = axum::Router::new().route(
                    "/resource",
                    axum::routing::any(
                        move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                            seen.fetch_add(1, Ordering::SeqCst);
                            async move {
                                if headers["authorization"] == "Bearer saved-login" {
                                    assert_eq!(body.as_ref(), b"{\"body\":\"same request\"}");
                                    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
                                } else {
                                    (
                                        StatusCode::from_u16(status).unwrap(),
                                        Json(serde_json::json!({"message":"rejected"})),
                                    )
                                }
                            }
                        },
                    ),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let url = format!("http://{}/resource", listener.local_addr().unwrap());
                let server =
                    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
                let client = build_client();
                let attempts = AtomicUsize::new(0);
                let build = |token: &str| {
                    client
                        .request(method.clone(), &url)
                        .bearer_auth(token)
                        .json(&serde_json::json!({"body":"same request"}))
                };
                let result: Result<Value, GhError> = send_with_auth_retry(
                    || build("expired"),
                    |token| build(token),
                    || {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        async { Some("saved-login".into()) }
                    },
                    "expired",
                )
                .await;
                if status == 401 {
                    assert_eq!(result.unwrap()["ok"], true);
                    assert_eq!(calls.load(Ordering::SeqCst), 2);
                    assert_eq!(attempts.load(Ordering::SeqCst), 1);
                } else {
                    assert_eq!(result.unwrap_err().status, Some(status));
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(attempts.load(Ordering::SeqCst), 0);
                }
                server.abort();
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires signed-in gh CLI and MIRA_TEST_GITHUB_PR=owner/repo/number; read-only"]
    async fn installed_cli_recovers_pr_read_from_an_expired_mira_token() {
        let target = std::env::var("MIRA_TEST_GITHUB_PR").expect("set owner/repo/number");
        let parts: Vec<_> = target.split('/').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[2].parse::<u64>().is_ok());
        let url = format!(
            "{GITHUB_API}/repos/{}/{}/pulls/{}",
            parts[0], parts[1], parts[2]
        );
        let value: Value = gh_get(&build_client(), "deliberately-invalid-test-token", &url)
            .await
            .unwrap();
        assert_eq!(value["number"].as_u64(), parts[2].parse::<u64>().ok());
    }
}
