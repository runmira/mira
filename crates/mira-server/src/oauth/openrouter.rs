//! Server axum handlers for the "Sign in with OpenRouter" PKCE flow.
//!
//! The pure OAuth mechanics (URL construction, code exchange, key
//! yaml-mirroring) live in [`mira_auth::providers::openrouter`] +
//! [`mira_auth::config`]. This file wires them into axum so the
//! browser can drive the flow, and adds the server-only bits:
//!
//! * Server-side PKCE verifier storage (the frontend triggers `start`
//!   and the browser triggers `callback` — the two sides don't share
//!   process state, so we stash the verifier in [`super::PendingFlow`]).
//! * Hot-swapping the live [`crate::provider::SwappableProvider`] the
//!   moment we have a fresh key, so the very next chat call uses it.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use mira_auth::config as auth_config;
use mira_auth::pkce::{gen_flow_id, gen_verifier};
use mira_auth::providers::openrouter;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::state::AppState;

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
    let flow_id = gen_flow_id();

    // Store server-side so we can validate the callback. Keyed by
    // flow_id (which is echoed back as `state`) so a concurrent second
    // flow doesn't stomp on the first.
    state.pending_oauth.lock().await.insert(
        flow_id.clone(),
        PendingFlow {
            verifier: verifier.clone(),
            provider: openrouter::PROVIDER.to_owned(),
            started_at: std::time::Instant::now(),
        },
    );

    // OpenRouter's PKCE flow accepts any caller-specified callback,
    // so we point it back at our own axum route on the main server
    // port. No temporary loopback needed here (unlike OpenAI's Codex
    // client, which allow-lists 1455/1457 only).
    let callback = format!(
        "http://127.0.0.1:{}/api/auth/openrouter/callback",
        state.local_port
    );
    let authorize_url = openrouter::authorize_url(&callback, &verifier, &flow_id);

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

pub async fn callback(State(state): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    // Provider-declined case: OpenRouter appends `error` /
    // `error_description` to the redirect when the user cancels or
    // auth fails. Surface it to the browser as a friendly page.
    if let Some(err) = q.error {
        let desc = q.error_description.as_deref().unwrap_or("no description");
        return html_page(
            &format!("OpenRouter sign-in failed: {err}"),
            &format!("Error: {err} — {desc}. You can close this tab."),
            false,
        );
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
    if pending.provider != openrouter::PROVIDER {
        return html_page(
            "OpenRouter sign-in failed",
            "Callback session mismatch. Please try again.",
            false,
        );
    }

    // Exchange, mirror into yaml, hot-swap the live provider.
    let key = match openrouter::exchange_code(&code, &pending.verifier).await {
        Ok(k) => k,
        Err(e) => {
            warn!(%e, "openrouter oauth: exchange failed");
            return html_page(
                "OpenRouter sign-in failed",
                &format!("Token exchange failed: {e}"),
                false,
            );
        }
    };
    if let Err(e) = auth_config::write_provider_key(openrouter::PROVIDER, &key) {
        return html_page(
            "OpenRouter sign-in failed",
            &format!("Got the key but couldn't save config: {e}"),
            false,
        );
    }

    // Hot-swap the live provider so the very next chat turn uses the
    // new key without a server restart. Same recipe as `put_settings`.
    match mira_config::MiraConfig::load_global() {
        Ok(cfg) => {
            let provider = super::super::settings::build_provider_from(&cfg);
            state.provider.set(provider);
            crate::models::invalidate();
        }
        Err(e) => warn!(%e, "openrouter oauth: post-write config reload failed"),
    }

    info!("openrouter oauth: sign-in complete; provider hot-swapped");
    html_page(
        "Signed in with OpenRouter",
        "You can close this tab and return to Mira.",
        true,
    )
}

/// Wrap [`mira_auth::loopback::html_page`] into an axum `Response`.
/// The loopback helper returns a plain string because it's also used
/// by hand-rolled HTTP responders that don't have axum in scope; here
/// we can lean on axum's `Html` extractor.
fn html_page(title: &str, body: &str, success: bool) -> Response {
    let html = mira_auth::loopback::html_page(title, body, success);
    let status = if success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Html(html)).into_response()
}
