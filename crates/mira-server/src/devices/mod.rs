//! iOS Simulators and Android Emulators: the Device pane and agents' device
//! tools. See `docs/devices.md` for the HTTP contract.
//!
//! - [`toolchain`]: downloads and pins the device hub, agent-device and Node.
//! - [`host`]: runs and supervises the hub; the state the pane follows.
//! - [`proxy`]: `/api/device-hub/*`, the allowlisted way to the hub.
//! - [`actions`]: typed device settings, run here rather than in the hub.
//! - [`tools`]: the `device_*` tools agents call.

mod actions;
mod host;
mod proxy;
mod toolchain;
mod tools;

use std::convert::Infallible;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Extension, Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;

pub use host::service;
pub use tools::{available, call, tool_specs};

use crate::pairing::{local_only, Caller};

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/api/devices/state", get(state))
        .route("/api/devices/events", get(events))
        .route("/api/devices/settings", post(settings))
        .route("/api/devices/list", get(list))
        .route("/api/devices/boot", post(boot))
        .route("/api/devices/shutdown", post(shutdown))
        .route("/api/devices/screenshot", post(screenshot))
        .route("/api/devices/action", post(action))
        .route("/api/devices/tool-versions", get(tool_versions))
        .route("/api/device-hub/*rest", any(proxy::handle))
}

/// Start the hub at server start if the user left it on.
pub fn resume() {
    service().resume();
}

fn err(status: StatusCode, e: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": e.to_string() }))).into_response()
}

async fn state() -> Response {
    Json(service().snapshot()).into_response()
}

async fn events() -> Response {
    let rx = service().subscribe();
    let stream = tokio_stream::wrappers::WatchStream::new(rx).map(|s| {
        Ok::<_, Infallible>(Event::default().data(serde_json::to_string(&s).unwrap_or_default()))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    hub_enabled: Option<bool>,
    agent_access: Option<bool>,
}

async fn settings(Extension(caller): Extension<Caller>, Json(s): Json<Settings>) -> Response {
    if let Some(denied) = local_only(&caller) {
        return denied;
    }
    match service().set(s.hub_enabled, s.agent_access).await {
        Ok(state) => Json(state).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn list() -> Response {
    let svc = service();
    svc.refresh_platforms().await;
    match svc.list().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

#[derive(Deserialize)]
struct Target {
    platform: String,
    id: String,
}

/// The refusal for a malformed target, like [`local_only`].
fn check(t: &Target) -> Option<Response> {
    let ok_platform = matches!(t.platform.as_str(), "ios" | "android");
    let ok_id = !t.id.is_empty()
        && t.id.len() < 128
        && t.id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
    (!(ok_platform && ok_id && !t.id.starts_with('-')))
        .then(|| err(StatusCode::BAD_REQUEST, "unknown device"))
}

async fn boot(Extension(caller): Extension<Caller>, Json(t): Json<Target>) -> Response {
    if let Some(denied) = local_only(&caller) {
        return denied;
    }
    if let Some(r) = check(&t) {
        return r;
    }
    match service().boot(&t.platform, &t.id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

async fn shutdown(Extension(caller): Extension<Caller>, Json(t): Json<Target>) -> Response {
    if let Some(denied) = local_only(&caller) {
        return denied;
    }
    if let Some(r) = check(&t) {
        return r;
    }
    match service().shutdown(&t.platform, &t.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

async fn screenshot(Extension(caller): Extension<Caller>, Json(t): Json<Target>) -> Response {
    if let Some(denied) = local_only(&caller) {
        return denied;
    }
    if let Some(r) = check(&t) {
        return r;
    }
    match service().screenshot(&t.platform, &t.id).await {
        Ok(png) => ([(header::CONTENT_TYPE, "image/png")], Body::from(png)).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

#[derive(Deserialize)]
struct ActionReq {
    platform: String,
    id: String,
    #[serde(flatten)]
    action: actions::Action,
}

async fn action(Extension(caller): Extension<Caller>, Json(a): Json<ActionReq>) -> Response {
    if let Some(denied) = local_only(&caller) {
        return denied;
    }
    let t = Target {
        platform: a.platform,
        id: a.id,
    };
    if let Some(r) = check(&t) {
        return r;
    }
    match actions::apply(&t.platform, &t.id, a.action).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn tool_versions() -> Response {
    Json(service().snapshot().versions).into_response()
}
