//! `/api/device-hub/*`: the only way to the device hub.
//!
//! The hub has routes that run shell commands and change device settings,
//! so this forwards a fixed list of read and stream routes and refuses the
//! rest with 404. Writes are limited to screenshot capture and stream
//! tuning. The caller's cookies and credentials never reach the hub; the
//! hub's own token is added here. Streams are passed through unbuffered and
//! marked so the compression layer leaves them alone.

use axum::body::Body;
use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as TMsg};

pub const PREFIX: &str = "/api/device-hub";

#[derive(Debug, PartialEq, Eq)]
pub enum Allowed {
    Read,
    Write,
    Socket,
}

/// One path segment that names a device: no traversal, no separators.
fn device_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'%'))
        && !s.contains("%2") // no encoded '/' or '.'
}

/// What a hub path may be used for, if anything.
pub fn classify(method: &Method, path: &str, upgrade: bool) -> Option<Allowed> {
    let segs: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    if upgrade {
        let ok = matches!(
            segs.as_slice(),
            ["api", "devices", "ws"]
                | ["vendor", "serve-sim", "helper", "ws"]
                | ["vendor", "serve-emu", "ws"]
        );
        return (ok && method == Method::GET).then_some(Allowed::Socket);
    }
    let read = match segs.as_slice() {
        ["api", "devices"] => true,
        ["vendor", "serve-sim", "helper", id, file] => {
            device_segment(id)
                && matches!(
                    *file,
                    "stream.mjpeg" | "stream.avcc" | "config" | "health" | "ax" | "foreground"
                )
        }
        ["vendor", "serve-emu", "api", "devices" | "accessibility"] => true,
        ["vendor", "serve-emu", "health"] => true,
        _ => false,
    };
    let write = matches!(
        segs.as_slice(),
        ["vendor", "serve-sim", "api", "screenshot"]
            | [
                "vendor",
                "serve-emu",
                "api",
                "screenshot" | "stream-mode" | "stream-settings"
            ]
    );
    match *method {
        Method::GET | Method::HEAD if read || write => Some(Allowed::Read),
        Method::POST if write => Some(Allowed::Write),
        _ => None,
    }
}

/// The query string without any `token` the caller tried to pass.
fn clean_query(q: Option<&str>) -> String {
    let kept: Vec<&str> = q
        .unwrap_or_default()
        .split('&')
        .filter(|kv| !kv.is_empty() && !kv.starts_with("token="))
        .collect();
    if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

pub async fn handle(req: Request) -> Response {
    let (mut parts, body) = req.into_parts();
    if let Some(denied) = crate::pairing::local_only(&crate::pairing::caller(&parts.extensions)) {
        return denied;
    }
    let Some(path) = parts.uri.path().strip_prefix(PREFIX).map(str::to_string) else {
        return not_found();
    };
    let ws = WebSocketUpgrade::from_request_parts(&mut parts, &())
        .await
        .ok();
    let Some(allowed) = classify(&parts.method, &path, ws.is_some()) else {
        return not_found();
    };
    let service = super::service();
    let Some(ep) = service.endpoint().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the device hub isn't running",
        )
            .into_response();
    };
    let target = format!("{path}{}", clean_query(parts.uri.query()));

    if allowed == Allowed::Socket {
        let Some(ws) = ws else { return not_found() };
        let url = format!("{}{target}", ep.origin.replacen("http", "ws", 1));
        return ws.on_upgrade(move |client| async move {
            if let Err(e) = pump(client, url, ep.token).await {
                tracing::debug!(error = %e, "device stream socket closed");
            }
        });
    }

    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut up = service
        .http()
        .request(method, format!("{}{target}", ep.origin))
        .bearer_auth(&ep.token);
    for name in [header::CONTENT_TYPE, header::ACCEPT, header::RANGE] {
        if let Some(v) = parts.headers.get(&name) {
            up = up.header(name.as_str(), v.as_bytes());
        }
    }
    if allowed == Allowed::Write {
        up = up.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let resp = match up.send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("device hub: {e}")).into_response(),
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_ENCODING,
    ] {
        if let Some(v) = resp
            .headers()
            .get(name.as_str())
            .and_then(|v| HeaderValue::from_bytes(v.as_bytes()).ok())
        {
            headers.insert(name, v);
        }
    }
    // A set Content-Encoding makes the compression layer pass the body
    // through; it would otherwise buffer streams that never end.
    headers
        .entry(header::CONTENT_ENCODING)
        .or_insert(HeaderValue::from_static("identity"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-transform"),
    );
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    let mut out = Response::new(Body::from_stream(resp.bytes_stream()));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    out
}

/// Relay a browser socket to the hub's, both ways, until either closes.
async fn pump(client: WebSocket, url: String, token: String) -> Result<(), String> {
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().map_err(|_| "bad token")?,
    );
    let (upstream, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| e.to_string())?;
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cl_tx, mut cl_rx) = client.split();
    // A close from either side is passed on so the other completes the
    // handshake; the hub's answer ends the relay.
    let to_hub = async {
        while let Some(Ok(m)) = cl_rx.next().await {
            let m = match m {
                AxMsg::Text(t) => TMsg::Text(t),
                AxMsg::Binary(b) => TMsg::Binary(b),
                AxMsg::Ping(p) => TMsg::Ping(p),
                AxMsg::Pong(p) => TMsg::Pong(p),
                AxMsg::Close(_) => TMsg::Close(None),
            };
            if up_tx.send(m).await.is_err() {
                break;
            }
        }
    };
    let to_client = async {
        while let Some(Ok(m)) = up_rx.next().await {
            let (m, last) = match m {
                TMsg::Text(t) => (AxMsg::Text(t), false),
                TMsg::Binary(b) => (AxMsg::Binary(b), false),
                TMsg::Ping(p) => (AxMsg::Ping(p), false),
                TMsg::Pong(p) => (AxMsg::Pong(p), false),
                TMsg::Close(_) => (AxMsg::Close(None), true),
                TMsg::Frame(_) => continue,
            };
            if cl_tx.send(m).await.is_err() || last {
                break;
            }
        }
        let _ = cl_tx.close().await;
    };
    tokio::select! {
        _ = to_client => {},
        // The browser went away without a close: give the hub a moment.
        _ = async { to_hub.await; tokio::time::sleep(std::time::Duration::from_secs(2)).await } => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(p: &str) -> Option<Allowed> {
        classify(&Method::GET, p, false)
    }

    #[test]
    fn allows_streams_and_reads() {
        assert_eq!(get("/api/devices"), Some(Allowed::Read));
        assert_eq!(
            get("/vendor/serve-sim/helper/91D0-AB/stream.avcc"),
            Some(Allowed::Read)
        );
        assert_eq!(
            get("/vendor/serve-sim/helper/91D0-AB/stream.mjpeg"),
            Some(Allowed::Read)
        );
        assert_eq!(get("/vendor/serve-emu/health"), Some(Allowed::Read));
        assert_eq!(
            classify(&Method::POST, "/vendor/serve-sim/api/screenshot", false),
            Some(Allowed::Write)
        );
        assert_eq!(
            classify(&Method::GET, "/vendor/serve-sim/helper/ws", true),
            Some(Allowed::Socket)
        );
    }

    #[test]
    fn refuses_everything_else() {
        for p in [
            "/vendor/serve-sim/api", // carries the exec token
            "/vendor/serve-sim/api/exec",
            "/vendor/serve-sim/exec",
            "/api/devices/create",
            "/api/devices/remove",
            "/vendor/serve-sim/helper/../api/stream.avcc",
            "/vendor/serve-sim/helper/%2e%2e/stream.avcc",
            "/vendor/serve-sim/helper/a/b/stream.avcc",
            "/vendor/serve-sim/helper/x/exec",
            "/",
            "",
        ] {
            assert_eq!(get(p), None, "{p}");
        }
        // Writes only where listed; reads never become writes.
        assert_eq!(classify(&Method::POST, "/api/devices", false), None);
        assert_eq!(
            classify(&Method::DELETE, "/vendor/serve-sim/api/screenshot", false),
            None
        );
        assert_eq!(classify(&Method::POST, "/api/devices/boot", false), None);
        // Sockets only on socket paths.
        assert_eq!(classify(&Method::GET, "/api/devices", true), None);
        assert_eq!(get("/vendor/serve-sim/helper/ws"), None);
    }

    #[test]
    fn strips_caller_tokens() {
        assert_eq!(clean_query(Some("device=a&token=x")), "?device=a");
        assert_eq!(clean_query(Some("token=x")), "");
        assert_eq!(clean_query(None), "");
    }
}
