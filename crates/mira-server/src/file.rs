//! `GET /api/file?path=/abs/or/tilde` — read a single file as text.
//!
//! Used by the composer's "Attach file" flow: the browser reads the file
//! contents so it can inline them into the next user message. Capped at
//! 200 KB to keep the request path snappy and prevent a stray click from
//! stuffing a 5 MB log into the model's context. Non-UTF-8 files are
//! rejected — the composer only supports text attachments.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::state::AppState;

/// Cap for attaching a file to a message: anything bigger fails cleanly
/// rather than truncating silently — the user picked the wrong file, and a
/// 5 MB log would swamp the model's context.
const MAX_BYTES: u64 = 200 * 1024;
/// Cap for showing a file in the viewer (`?purpose=view`): nothing goes to
/// the model, so only rendering cost matters.
const MAX_VIEW_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
    /// `view` for the file viewer (larger cap); absent for attachments.
    #[serde(default)]
    pub purpose: Option<String>,
}

fn human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

#[derive(Debug, Serialize)]
pub struct FileView {
    pub path: String,
    pub bytes: u64,
    pub content: String,
    /// The line a `path:42` reference pointed at, 1-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Split a trailing `:line` or `:line:col` off a reference (`src/a.rs:42`),
/// the way compilers, grep and models write locations. Only when the whole
/// reference doesn't name a file itself, so a file literally called
/// `notes:2` still opens.
fn split_location(reference: &str, exists: impl Fn(&str) -> bool) -> (String, Option<u32>) {
    if exists(reference) {
        return (reference.to_string(), None);
    }
    let mut parts = reference.rsplitn(3, ':').collect::<Vec<_>>();
    parts.reverse();
    let num = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    match parts.as_slice() {
        [path, line, col] if num(line).is_some() && num(col).is_some() => {
            (path.to_string(), num(line))
        }
        [rest @ .., last] if num(last).is_some() && !rest.is_empty() => (rest.join(":"), num(last)),
        _ => (reference.to_string(), None),
    }
}

pub async fn read_file(State(state): State<AppState>, Query(q): Query<FileQuery>) -> Response {
    // Relative references are the chat's — `src/lib.rs` means the file in
    // the folder the chat works in, not wherever the server was started
    // (the desktop app starts it in the home directory).
    let base = state.current_cwd().await;
    let resolve = |p: &str| {
        let expanded = std::path::PathBuf::from(shellexpand::tilde(p).to_string());
        if expanded.is_absolute() {
            expanded
        } else {
            base.join(expanded)
        }
    };
    let (reference, line) = split_location(q.path.trim(), |p| resolve(p).is_file());
    let expanded = resolve(&reference).display().to_string();
    let path = match std::fs::canonicalize(&expanded) {
        Ok(p) => p,
        Err(e) => {
            return err(
                StatusCode::BAD_REQUEST,
                format!("cannot resolve `{expanded}`: {e}"),
            )
        }
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("stat: {e}")),
    };
    if !meta.is_file() {
        return err(
            StatusCode::BAD_REQUEST,
            format!("not a regular file: {}", path.display()),
        );
    }
    let viewing = q.purpose.as_deref() == Some("view");
    let cap = if viewing { MAX_VIEW_BYTES } else { MAX_BYTES };
    if meta.len() > cap {
        let msg = if viewing {
            format!(
                "This file is {} — too large to preview here (the limit is {}). Open it in your editor.",
                human(meta.len()),
                human(cap)
            )
        } else {
            format!(
                "This file is {} — attachments are limited to {}. Attach a smaller file, or paste the part that matters.",
                human(meta.len()),
                human(cap)
            )
        };
        return err(StatusCode::PAYLOAD_TOO_LARGE, msg);
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return err(StatusCode::FORBIDDEN, format!("read: {e}")),
    };
    let content = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return err(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "file is not valid UTF-8 text".to_string(),
            )
        }
    };

    debug!(path = %path.display(), bytes = meta.len(), "file read");

    Json(FileView {
        path: path.display().to_string(),
        bytes: meta.len(),
        content,
        line,
    })
    .into_response()
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_are_split_off_references() {
        let none = |_: &str| false;
        assert_eq!(
            split_location("src/lib.rs:42", none),
            ("src/lib.rs".into(), Some(42))
        );
        assert_eq!(
            split_location("src/lib.rs:42:7", none),
            ("src/lib.rs".into(), Some(42))
        );
        assert_eq!(
            split_location("src/lib.rs", none),
            ("src/lib.rs".into(), None)
        );
        assert_eq!(
            split_location("C:/x/a.rs:9", none),
            ("C:/x/a.rs".into(), Some(9))
        );
        assert_eq!(split_location("a.rs:0", none), ("a.rs:0".into(), None));
        // A file whose name really ends in `:2` is taken as it is.
        assert_eq!(
            split_location("notes:2", |p| p == "notes:2"),
            ("notes:2".into(), None)
        );
    }
}
