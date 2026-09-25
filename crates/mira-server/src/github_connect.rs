//! `/api/github/connect`: set a repository up for Mira's GitHub Action
//! from the web UI, with no files to edit (see `mira_cloud::setup`).
//!
//! - `GET` reports what would be connected and whether it already is.
//! - `POST {repo?}` connects it, using the provider, model and key from
//!   Settings and the `GITHUB_TOKEN` key.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_cloud::github::GitHub;
use mira_cloud::setup::{self, ConnectOptions};
use mira_config::MiraConfig;
use serde::{Deserialize, Serialize};

use crate::pull_requests::{parse_github_remote, resolve_github_token};
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct RepoQuery {
    /// `owner/name`; defaults to the current folder's GitHub remote.
    #[serde(default)]
    pub repo: Option<String>,
    /// A short-lived Runmira GitHub App token for the repository, from
    /// the connect flow. Without it, the `GITHUB_TOKEN` key is used.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Serialize)]
pub struct ConnectView {
    /// The repository this would connect.
    repo: Option<String>,
    has_token: bool,
    provider: Option<String>,
    model: Option<String>,
    /// Why connecting can't work yet (no key, no model, …).
    problem: Option<String>,
    /// The part of `problem` about Mira's own settings (provider, model,
    /// key), which matters whichever way GitHub is connected.
    settings_problem: Option<String>,
    /// Current state on GitHub, when it could be read.
    status: Option<setup::Status>,
}

pub async fn get_connect(State(state): State<AppState>, Query(q): Query<RepoQuery>) -> Response {
    let repo = repo_for(&state, q.repo).await;
    let cfg = MiraConfig::load(&state.current_cwd().await).unwrap_or_default();
    let token = resolve_github_token();
    let settings_problem = options(&cfg).err();
    let problem = match options(&cfg) {
        Err(e) => Some(e),
        Ok(_) if token.is_none() => {
            Some("add a GitHub token (repo and workflow scopes) under Search & keys".into())
        }
        Ok(_) if repo.is_none() => {
            Some("this folder has no GitHub remote; enter owner/name".into())
        }
        Ok(_) => None,
    };
    let status = match (&token, &repo) {
        (Some(t), Some((owner, name))) => match GitHub::new(api(), t) {
            Ok(gh) => setup::status(&gh, owner, name).await.ok(),
            Err(_) => None,
        },
        _ => None,
    };
    Json(ConnectView {
        repo: repo.map(|(o, n)| format!("{o}/{n}")),
        has_token: token.is_some(),
        provider: cfg
            .default_provider
            .as_deref()
            .map(mira_config::pretty_provider_name),
        model: cfg.default_model.clone(),
        problem,
        settings_problem,
        status,
    })
    .into_response()
}

pub async fn post_connect(State(state): State<AppState>, Json(q): Json<RepoQuery>) -> Response {
    let Some((owner, name)) = repo_for(&state, q.repo).await else {
        return err(
            StatusCode::BAD_REQUEST,
            "which repository? Enter owner/name".into(),
        );
    };
    let cfg = MiraConfig::load(&state.current_cwd().await).unwrap_or_default();
    let opts = match options(&cfg) {
        Ok(o) => o,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    let Some(token) = q
        .token
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(resolve_github_token)
    else {
        return err(
            StatusCode::BAD_REQUEST,
            "connect GitHub, or add a GitHub token (repo and workflow scopes) under Search & keys"
                .into(),
        );
    };
    let gh = match GitHub::new(api(), &token) {
        Ok(g) => g,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    match setup::connect(&gh, &owner, &name, &opts).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

async fn repo_for(state: &AppState, explicit: Option<String>) -> Option<(String, String)> {
    match explicit.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => r
            .trim_start_matches("https://github.com/")
            .trim_end_matches(".git")
            .split_once('/')
            .map(|(o, n)| (o.to_owned(), n.trim_end_matches('/').to_owned()))
            .filter(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/')),
        None => parse_github_remote(&state.current_cwd().await),
    }
}

/// Connect options from the default provider in Settings.
fn options(cfg: &MiraConfig) -> Result<ConnectOptions, String> {
    let provider = cfg
        .default_provider
        .clone()
        .ok_or("choose a provider in Settings first")?;
    let entry = cfg.providers.get(&provider).cloned().unwrap_or_default();
    ConnectOptions::new(
        &provider,
        cfg.default_model.as_deref().unwrap_or(""),
        entry.base_url.as_deref(),
        &entry.resolved_api_key().unwrap_or_default(),
    )
}

fn api() -> &'static str {
    "https://api.github.com"
}

fn err(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}
