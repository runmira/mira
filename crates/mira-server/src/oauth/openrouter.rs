//! OpenRouter "Sign in with OpenRouter" — PKCE OAuth flow.
//!
//! Flow:
//!
//! 1. Frontend calls `POST /api/auth/openrouter/start`. We generate a
//!    PKCE verifier + a flow_id, stash the verifier in `PendingFlowStore`
//!    keyed by flow_id, and return the fully-formed authorize URL for
//!    the UI to open in a new browser tab.
//! 2. Browser lands on `openrouter.ai/auth?...`, user grants access.
//! 3. OpenRouter redirects back to `GET /api/auth/openrouter/callback`
//!    on our loopback listener with `?code=...&state=<flow_id>`.
//! 4. We look up the verifier by flow_id, POST to
//!    `openrouter.ai/api/v1/auth/keys` with the code + verifier, and
//!    receive a `{ "key": "sk-or-..." }` response.
//! 5. The key is written into `~/.mira/mira.yaml` under
//!    `providers.openrouter.api_key`, the live provider is hot-swapped
//!    the same way `PUT /api/settings` does it, and the browser sees a
//!    tiny "you can close this tab" HTML page.
//!
//! Everything sensitive stays server-side: the verifier never leaves
//! this process and the returned key is written straight to disk (mode
//! 0600 on Unix — see `mira-config::save_global`) without ever passing
//! back through the browser.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use mira_config::{default_base_url_for, MiraConfig, ProviderConfig};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::state::AppState;

use super::pkce::{code_challenge_s256, gen_flow_id, gen_verifier};
use super::PendingFlow;

/// Reply to `POST /api/auth/openrouter/start`. The UI opens `authorize_url`
/// in a new tab and remembers `flow_id` so it can poll `/api/auth/status`
/// (or just wait for the callback-driven config refresh).
#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub authorize_url: String,
    pub flow_id: String,
}

pub async fn start(State(state): State<AppState>) -> Response {
    let verifier = gen_verifier();
    let challenge = code_challenge_s256(&verifier);
    let flow_id = gen_flow_id();

    // Store server-side so we can validate the callback. Keyed by
    // flow_id (which is echoed back as `state`) so a concurrent second
    // flow doesn't stomp on the first.
    state.pending_oauth.lock().await.insert(
        flow_id.clone(),
        PendingFlow {
            verifier,
            provider: "openrouter".to_owned(),
            started_at: std::time::Instant::now(),
        },
    );

    // OpenRouter accepts `callback_url` as a query param on the
    // authorize endpoint. We point it at our loopback callback route
    // on this same server so no external URL registration is needed.
    // The `state` param carries our flow_id round-trip.
    let callback = format!(
        "http://127.0.0.1:{}/api/auth/openrouter/callback",
        state.local_port
    );
    let authorize_url = format!(
        "https://openrouter.ai/auth?callback_url={cb}&code_challenge={ch}&code_challenge_method=S256&state={st}",
        cb = urlencoding::encode(&callback),
        ch = urlencoding::encode(&challenge),
        st = urlencoding::encode(&flow_id),
    );

    Json(StartResponse {
        authorize_url,
        flow_id,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExchangeBody<'a> {
    code: &'a str,
    code_verifier: &'a str,
    code_challenge_method: &'a str,
}

#[derive(Debug, Deserialize)]
struct ExchangeResponse {
    key: String,
}

pub async fn callback(
    State(state): State<AppState>,
    Query(q): Query<CallbackQuery>,
) -> Response {
    // Provider-declined case: OpenRouter appends `error` / `error_description`
    // to the redirect when the user cancels or auth fails. Surface it to the
    // browser as a friendly page so the user knows to close the tab.
    if let Some(err) = q.error {
        let desc = q.error_description.as_deref().unwrap_or("no description");
        return html_page(&format!(
            "OpenRouter sign-in failed: {err}",
        ), &format!("Error: {err} — {desc}. You can close this tab."), false);
    }

    let Some(code) = q.code else {
        return html_page(
            "OpenRouter sign-in failed",
            "Missing `code` on the callback URL.",
            false,
        );
    };
    let Some(flow_id) = q.state else {
        return html_page(
            "OpenRouter sign-in failed",
            "Missing `state` on the callback URL — CSRF guard rejected the callback.",
            false,
        );
    };

    // Look up + consume the pending flow. `remove` (not `get`) so a
    // replayed callback URL can't be used a second time.
    let pending = state.pending_oauth.lock().await.remove(&flow_id);
    let Some(pending) = pending else {
        return html_page(
            "OpenRouter sign-in failed",
            "Unknown or expired sign-in session. Please try again.",
            false,
        );
    };
    if pending.provider != "openrouter" {
        return html_page(
            "OpenRouter sign-in failed",
            "Callback session mismatch. Please try again.",
            false,
        );
    }

    // Exchange the authorization code for an API key.
    let client = reqwest::Client::new();
    let resp = match client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&ExchangeBody {
            code: &code,
            code_verifier: &pending.verifier,
            code_challenge_method: "S256",
        })
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(%e, "openrouter oauth: token exchange request failed");
            return html_page(
                "OpenRouter sign-in failed",
                &format!("Couldn't reach openrouter.ai: {e}"),
                false,
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(%status, %body, "openrouter oauth: exchange returned non-2xx");
        return html_page(
            "OpenRouter sign-in failed",
            &format!("Token exchange failed ({status}): {body}"),
            false,
        );
    }
    let exchange: ExchangeResponse = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            warn!(%e, "openrouter oauth: bad json from exchange");
            return html_page(
                "OpenRouter sign-in failed",
                &format!("Bad response from token exchange: {e}"),
                false,
            );
        }
    };

    // Persist the key into ~/.mira/mira.yaml so the same
    // ProviderConfig::resolved_api_key path used for every other
    // provider just works. Also set default_provider so the UI
    // immediately shows OpenRouter as the active choice.
    let mut cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return html_page(
                "OpenRouter sign-in failed",
                &format!("Couldn't load config to persist key: {e}"),
                false,
            );
        }
    };
    let entry = cfg
        .providers
        .entry("openrouter".to_owned())
        .or_insert_with(|| ProviderConfig {
            base_url: default_base_url_for("openrouter").map(str::to_owned),
            ..Default::default()
        });
    entry.api_key = Some(exchange.key);
    if entry.base_url.is_none() {
        entry.base_url = default_base_url_for("openrouter").map(str::to_owned);
    }
    if cfg.default_provider.is_none() {
        cfg.default_provider = Some("openrouter".to_owned());
    }
    if let Err(e) = cfg.save_global() {
        return html_page(
            "OpenRouter sign-in failed",
            &format!("Got the key but couldn't save config: {e}"),
            false,
        );
    }

    // Hot-swap the live provider so the very next chat turn uses the
    // new key without a server restart. Same recipe as `put_settings`.
    let provider = super::super::settings::build_provider_from(&cfg);
    state.provider.set(provider);
    crate::models::invalidate();

    info!("openrouter oauth: sign-in complete; provider hot-swapped");
    html_page(
        "Signed in with OpenRouter",
        "You can close this tab and return to Mira.",
        true,
    )
}

/// Tiny self-contained HTML page rendered back to the browser at the
/// end of the round trip. No frameworks, no assets — the whole page
/// lives in this string. Success page auto-closes the tab after a
/// couple seconds when the browser allows it.
fn html_page(title: &str, body: &str, success: bool) -> Response {
    let color = if success { "#22c55e" } else { "#ef4444" };
    let auto_close = if success {
        "<script>setTimeout(() => window.close(), 1500);</script>"
    } else {
        ""
    };
    let html = format!(
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
    );
    let status = if success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Html(html)).into_response()
}
