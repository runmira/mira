//! "Report a problem" in the web UI.
//!
//! `GET /api/diagnostics?session=<id>` → every file the bundle would hold,
//! in full, so the user can read it first.
//! `GET /api/diagnostics/bundle?session=<id>` → the same files as a zip.
//! `POST /api/diagnostics/save?session=<id>` → writes the zip to
//! ~/Downloads and returns its path. For the desktop app, whose webview
//! doesn't download files; only from the computer running Mira.
//!
//! Both rebuild from disk; the zip may carry a few more log lines than the
//! preview did, redacted the same way.

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use axum::Json;
use mira_diagnostics::{Bundle, BundleFile, BundleOptions, Redactor};
use serde::{Deserialize, Serialize};

use crate::pairing::Caller;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct DiagnosticsQuery {
    /// Include this chat. Off unless the user ticks it in the dialog.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Serialize)]
struct Preview {
    files: Vec<BundleFile>,
    total_bytes: usize,
    file_name: String,
    crash_reports_on: bool,
    issue_url: String,
}

pub async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let bundle = match collect(&state, &headers, q).await {
        Ok(b) => b,
        Err((status, message)) => return error(status, message),
    };
    let crash_reports_on = mira_config::MiraConfig::load_global()
        .map(|c| c.diagnostics.crash_reports())
        .unwrap_or(false);
    Json(Preview {
        total_bytes: bundle.total_bytes(),
        file_name: Bundle::file_name(),
        crash_reports_on,
        issue_url: issue_url(),
        files: bundle.files,
    })
    .into_response()
}

pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let bundle = match collect(&state, &headers, q).await {
        Ok(b) => b,
        Err((status, message)) => return error(status, message),
    };
    match bundle.to_zip() {
        Ok(zip) => (
            [
                (header::CONTENT_TYPE, "application/zip".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", Bundle::file_name()),
                ),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            zip,
        )
            .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("build zip: {e}")),
    }
}

pub async fn save(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    headers: HeaderMap,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    if !matches!(caller.as_deref(), Some(Caller::Local)) {
        return error(
            StatusCode::FORBIDDEN,
            "only from the computer running Mira".into(),
        );
    }
    let bundle = match collect(&state, &headers, q).await {
        Ok(b) => b,
        Err((status, message)) => return error(status, message),
    };
    let saved = tokio::task::spawn_blocking(move || -> std::io::Result<std::path::PathBuf> {
        let dir = std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Downloads"))
            .filter(|d| d.is_dir())
            .unwrap_or_else(|| mira_diagnostics::mira_dir().join("diagnostics"));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(Bundle::file_name());
        mira_config::write_private(&path, &bundle.to_zip()?)?;
        Ok(path)
    })
    .await;
    match saved {
        Ok(Ok(path)) => Json(serde_json::json!({
            "path": path.display().to_string(),
            // The `/api/editors/open` entry that shows a file in its folder.
            "reveal_with": match std::env::consts::OS {
                "macos" => "finder",
                "windows" => "explorer",
                _ => "files",
            },
        }))
        .into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}")),
    }
}

async fn collect(
    state: &AppState,
    headers: &HeaderMap,
    q: DiagnosticsQuery,
) -> Result<Bundle, (StatusCode, String)> {
    let session_id = match q.session.filter(|s| !s.is_empty()) {
        Some(id) if mira_diagnostics::bundle::is_session_id(&id) => Some(id),
        Some(_) => return Err((StatusCode::BAD_REQUEST, "bad session id".into())),
        None => None,
    };
    let cwd = state.current_cwd().await;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tokio::task::spawn_blocking(move || {
        let cfg = mira_config::MiraConfig::load(&cwd).unwrap_or_default();
        let mut opts = BundleOptions::new(Redactor::from_env_and_config(&cfg), "web");
        opts.cwd = Some(cwd);
        opts.session_id = session_id;
        opts.extra.push(BundleFile::new(
            "client.json",
            serde_json::json!({ "user_agent": user_agent }).to_string(),
        ));
        Bundle::collect(opts)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("collect: {e}")))
}

fn issue_url() -> String {
    mira_diagnostics::issue_url(
        "",
        &format!(
            "**What happened?**\n\n\n**What did you expect?**\n\n\n**Steps to reproduce**\n\n\n\
             Mira {} · {} {}\n\n<!-- Drag the diagnostics zip here to attach it. -->\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
    )
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}
