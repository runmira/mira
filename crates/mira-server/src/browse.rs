//! Directory listing for the folder-picker modal.
//!
//! `GET /api/browse?path=/some/dir&show_hidden=false&include_files=false`
//!
//! Returns the canonicalized path, its parent (or `null` at the root), and a
//! sorted list of entries. Files are excluded by default — the picker is for
//! choosing a project folder. Hidden entries (leading `.`) are also excluded
//! by default. Bounded to 500 entries so hitting `/` doesn't hang the UI.
//!
//! This runs on loopback and speaks to the local filesystem with the user's
//! own uid — no additional auth. Locking down the traversal to a whitelist
//! would be a future addition once multi-user hosting is on the table.

use std::path::{Path, PathBuf};

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default)]
    pub include_files: bool,
}

#[derive(Debug, Serialize)]
pub struct BrowseView {
    pub path: String,
    pub parent: Option<String>,
    pub home: Option<String>,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

pub async fn browse(Query(q): Query<BrowseQuery>) -> Response {
    let raw = q.path.unwrap_or_else(default_start);
    let expanded = shellexpand::tilde(&raw).to_string();
    let path = match std::fs::canonicalize(&expanded) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("cannot resolve `{expanded}`: {e}") })),
            )
                .into_response();
        }
    };
    if !path.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("not a directory: {}", path.display()) })),
        )
            .into_response();
    }

    let read = match std::fs::read_dir(&path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": format!("read_dir failed: {e}") })),
            )
                .into_response();
        }
    };

    let mut entries: Vec<Entry> = Vec::new();
    let mut truncated = false;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !q.show_hidden && name.starts_with('.') {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let is_dir = file_type.is_dir() || (file_type.is_symlink() && entry.path().is_dir());
        if !is_dir && !q.include_files {
            continue;
        }
        entries.push(Entry {
            name,
            path: entry.path().display().to_string(),
            is_dir,
        });
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Json(BrowseView {
        path: path.display().to_string(),
        parent: path.parent().map(|p| p.display().to_string()),
        home: std::env::var("HOME").ok(),
        entries,
        truncated,
    })
    .into_response()
}

fn default_start() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_owned())
}

/// Used elsewhere as a small guard: strip a trailing slash but not the root.
#[allow(dead_code)]
pub(crate) fn tidy(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        PathBuf::from("/")
    } else {
        PathBuf::from(trimmed)
    }
}
