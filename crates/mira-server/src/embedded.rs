//! Embedded frontend assets.
//!
//! Whatever's in `crates/mira-server/frontend/dist/` at compile time is
//! baked into the binary here. The build script guarantees the directory
//! exists (so this macro never panics on a cold clone), but it may be
//! empty until `npm run build` is run. [`has_frontend`] lets the router
//! decide whether to serve embedded assets or fall through to the
//! placeholder page.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};

static FRONTEND: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/frontend/dist");

pub fn has_frontend() -> bool {
    FRONTEND.get_file("index.html").is_some()
}

/// Serve a file from the embedded tree, or return the SPA entry (`index.html`)
/// on any miss so client-side routing works. Returns 404 only if there is no
/// embedded frontend at all — in which case the caller should fall back to
/// the placeholder page.
pub async fn serve(path: &str) -> Response {
    let path = path.trim_start_matches('/');
    if let Some(file) = FRONTEND.get_file(path) {
        return respond(path, file.contents());
    }
    if let Some(index) = FRONTEND.get_file("index.html") {
        return respond("index.html", index.contents());
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn respond(path: &str, bytes: &'static [u8]) -> Response {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    // Long cache for hashed assets under /assets/; short for entry html.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    (StatusCode::OK, headers, bytes).into_response()
}
