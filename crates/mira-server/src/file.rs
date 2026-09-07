//! `GET /api/file?path=/abs/or/tilde` — read a single file as text.
//!
//! Used by the composer's "Attach file" flow: the browser reads the file
//! contents so it can inline them into the next user message. Capped at
//! 200 KB to keep the request path snappy and prevent a stray click from
//! stuffing a 5 MB log into the model's context. Non-UTF-8 files are
//! rejected — the composer only supports text attachments.

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Hard cap on what we'll read. Anything bigger fails cleanly rather than
/// truncating silently — the user picked the wrong file.
const MAX_BYTES: u64 = 200 * 1024;

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct FileView {
    pub path: String,
    pub bytes: u64,
    pub content: String,
}

pub async fn read_file(Query(q): Query<FileQuery>) -> Response {
    let expanded = shellexpand::tilde(&q.path).to_string();
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
    if meta.len() > MAX_BYTES {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file is {} bytes; cap is {} bytes (paste a subset by hand for now)",
                meta.len(),
                MAX_BYTES
            ),
        );
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
    })
    .into_response()
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
