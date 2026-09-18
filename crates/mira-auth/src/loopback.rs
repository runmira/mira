//! One-shot HTTP loopback listener for OAuth callbacks.
//!
//! OAuth PKCE flows redirect the browser back to a URL controlled by
//! the client — for a CLI or a background service, that means opening
//! a temporary `127.0.0.1:<port>` listener, waiting for one request,
//! serving a small "you can close this tab" HTML page, and shutting
//! down. This module owns that dance so every provider adapter and the
//! CLI login command share the same primitive.
//!
//! Deliberately hand-rolls a minimal HTTP/1.1 responder — pulling in
//! hyper/axum just to serve one 3-line response would double the
//! CLI's dep tree. The parser only extracts the query string on the
//! request line; nothing else in the request matters.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::warn;

/// The OAuth callback query, as pulled off the browser's redirect URL.
/// All four fields are optional because a hostile / broken redirect
/// might omit any of them; callers decide which ones are load-bearing.
#[derive(Debug, Clone, Default)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl Callback {
    /// True when the redirect carried neither a `code` nor an `error` —
    /// which usually means the browser hit `/` or `/favicon.ico` before
    /// the real callback fired. Callers should skip and keep waiting.
    pub fn is_noise(&self) -> bool {
        self.code.is_none() && self.error.is_none()
    }
}

/// Try each port in `ports` in order; return the first one that binds
/// cleanly. Fails only when *every* candidate is in use — a real
/// signal, since the caller usually can't recover without user action
/// (kill the other process, retry later).
pub async fn bind_first_free(ports: &[u16]) -> Result<(TcpListener, u16)> {
    for &port in ports {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
        match TcpListener::bind(addr).await {
            Ok(l) => return Ok((l, port)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(anyhow!(
        "no free port in {:?} — is another sign-in already in progress?",
        ports
    ))
}

/// Bind any free port on `127.0.0.1` (kernel-assigned). Preferred when
/// the provider's OAuth client accepts a variable redirect port
/// (OpenRouter). Providers with a fixed allow-list (OpenAI/Codex) must
/// use [`bind_first_free`] with their specific ports instead.
pub async fn bind_any() -> Result<(TcpListener, u16)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind 127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

/// Wait for one HTTP callback on `listener`, respond with `response_html`,
/// then return the parsed query. Requests whose query has neither `code`
/// nor `error` (favicon probes, root pings) get a `204 No Content` and
/// we keep waiting — a browser tab that reloads the URL a few times
/// shouldn't wedge the flow.
///
/// The whole listener is bounded by `timeout`; if it fires with no
/// meaningful callback, this returns `Err`.
pub async fn await_callback(
    listener: TcpListener,
    response_html: &str,
    timeout: Duration,
) -> Result<Callback> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!(
                "timed out after {}s waiting for browser callback",
                timeout.as_secs()
            ));
        }
        let (mut socket, _peer) =
            match tokio::time::timeout(remaining, listener.accept()).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    warn!(%e, "loopback accept failed");
                    continue;
                }
                Err(_) => {
                    return Err(anyhow!(
                        "timed out after {}s waiting for browser callback",
                        timeout.as_secs()
                    ))
                }
            };

        // Read one HTTP request. Browsers send small requests (<4KB
        // headers), so a single read is enough in practice.
        let mut buf = vec![0u8; 8192];
        let n = match socket.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                warn!(%e, "loopback read failed");
                continue;
            }
        };
        buf.truncate(n);
        let request = String::from_utf8_lossy(&buf);
        let cb = parse_callback_query(&request);

        if cb.is_noise() {
            // 204 the noise and keep waiting — favicon probes shouldn't
            // exhaust the callback window.
            let _ = socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await;
            let _ = socket.shutdown().await;
            continue;
        }

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            status = if cb.error.is_some() { "400 Bad Request" } else { "200 OK" },
            len = response_html.len(),
            body = response_html,
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
        return Ok(cb);
    }
}

/// Best-effort query parser. Only pulls out the four OAuth fields we
/// care about; anything else in the request is ignored.
fn parse_callback_query(request: &str) -> Callback {
    let mut cb = Callback::default();
    let Some(line) = request.lines().next() else {
        return cb;
    };
    let Some(path_start) = line.find(' ') else {
        return cb;
    };
    let after = &line[path_start + 1..];
    let path_end = after.find(' ').unwrap_or(after.len());
    let path = &after[..path_end];
    let Some(qi) = path.find('?') else {
        return cb;
    };
    let query = &path[qi + 1..];
    for pair in query.split('&') {
        let Some(eq) = pair.find('=') else { continue };
        let k = &pair[..eq];
        let v = urlencoding::decode(&pair[eq + 1..])
            .map(|s| s.into_owned())
            .unwrap_or_default();
        match k {
            "code" => cb.code = Some(v),
            "state" => cb.state = Some(v),
            "error" => cb.error = Some(v),
            "error_description" => cb.error_description = Some(v),
            _ => {}
        }
    }
    cb
}

/// Small HTML page rendered back to the browser at the end of the
/// round trip. No frameworks, no assets — the whole page lives in this
/// string. Success page auto-closes the tab after 1.5s when the browser
/// allows it (Chromium-based browsers usually do; Firefox blocks JS
/// window.close on user-initiated tabs, so falls back to the message).
pub fn html_page(title: &str, body: &str, success: bool) -> String {
    let color = if success { "#22c55e" } else { "#ef4444" };
    let auto_close = if success {
        "<script>setTimeout(() => window.close(), 1500);</script>"
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Mira — {title}</title>
    <style>
      body {{ background: #0b0b0b; color: #eaeaea; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 0; height: 100vh; display: flex; align-items: center; justify-content: center; }}
      .card {{ text-align: center; padding: 24px 32px; border: 1px solid #222; border-radius: 12px; background: #131313; max-width: 480px; }}
      .dot {{ display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: {color}; margin-right: 8px; vertical-align: middle; }}
      h1 {{ font-size: 16px; margin: 0 0 12px 0; font-weight: 600; }}
      p {{ font-size: 14px; margin: 0; color: #b0b0b0; }}
    </style>
  </head>
  <body>
    <div class="card">
      <h1><span class="dot"></span>{title}</h1>
      <p>{body}</p>
    </div>
    {auto_close}
  </body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_openai_callback() {
        let req = "GET /auth/callback?code=abc123&state=fl0w HTTP/1.1\r\nHost: localhost:1455\r\n\r\n";
        let cb = parse_callback_query(req);
        assert_eq!(cb.code.as_deref(), Some("abc123"));
        assert_eq!(cb.state.as_deref(), Some("fl0w"));
        assert!(cb.error.is_none());
    }

    #[test]
    fn parses_error_callback_with_description() {
        let req = "GET /cb?error=access_denied&error_description=user%20said%20no HTTP/1.1\r\n\r\n";
        let cb = parse_callback_query(req);
        assert_eq!(cb.error.as_deref(), Some("access_denied"));
        assert_eq!(cb.error_description.as_deref(), Some("user said no"));
        assert!(cb.code.is_none());
    }

    #[test]
    fn favicon_and_root_are_noise() {
        let favicon = parse_callback_query("GET /favicon.ico HTTP/1.1\r\n\r\n");
        assert!(favicon.is_noise());
        let root = parse_callback_query("GET / HTTP/1.1\r\n\r\n");
        assert!(root.is_noise());
    }
}
