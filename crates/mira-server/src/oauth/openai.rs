//! Server axum handler for "Sign in with ChatGPT" (OpenAI OAuth
//! PKCE + RFC 8693 token exchange).
//!
//! Pure OAuth mechanics live in [`mira_auth::providers::openai`] +
//! [`mira_auth::loopback`]. This module wires them into axum so the
//! browser can drive the flow, and adds the server-only bits:
//!
//! * One-shot temporary loopback listener on the Codex-registered
//!   ports (1455/1457). Bound before the authorize URL is built so a
//!   busy port fails cleanly instead of bouncing the browser into a
//!   dead socket.
//! * Persisting the resulting [`TokenBundle`], mirroring the API key
//!   into `mira.yaml`, and hot-swapping the live
//!   [`crate::provider::SwappableProvider`] so the next chat call
//!   uses the fresh key.
//! * A [`refresh_and_persist`] entry point the background refresh
//!   loop (`super::refresh`) calls when the stored bundle is near
//!   expiry.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_auth::config as auth_config;
use mira_auth::loopback;
use mira_auth::pkce::{gen_flow_id, gen_verifier};
use mira_auth::providers::openai;
use mira_auth::store::{self, TokenBundle};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::state::AppState;

/// How long the loopback listener stays open waiting for the browser
/// callback before it gives up and releases the port. 5 minutes is
/// generous — the OAuth code itself expires in 10.
const CALLBACK_LISTENER_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Reply to `POST /api/auth/openai/start`. Frontend opens `authorize_url`
/// in a new tab; the OAuth callback lands on our temporary loopback
/// listener (not the main mira-server port) because Codex's OAuth
/// client registration only allows ports 1455/1457.
#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub authorize_url: String,
    pub flow_id: String,
}

pub async fn start(State(state): State<AppState>) -> Response {
    let verifier = gen_verifier();
    let flow_id = gen_flow_id();

    // Bind the callback listener FIRST so we know the exact port
    // before we build the authorize URL. If both allowed ports are
    // busy the whole start fails cleanly instead of constructing a
    // URL the browser will bounce back into a dead socket.
    let (listener, port) = match loopback::bind_first_free(openai::CALLBACK_PORTS).await {
        Ok(v) => v,
        Err(e) => {
            warn!(%e, "openai oauth: couldn't bind callback listener");
            return error_json(
                StatusCode::CONFLICT,
                format!(
                    "Couldn't reserve loopback port for sign-in: {e}. \
                     If the Codex CLI or another mira sign-in is running, \
                     wait a moment and try again."
                ),
            );
        }
    };

    // Exactly `localhost` (not `127.0.0.1`) — that's what's on the
    // Codex OAuth client's redirect URI allow-list. Wrong hostname
    // yields `error_code: unknown_error` at auth.openai.com.
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let authorize_url = openai::authorize_url(&redirect_uri, &verifier, &flow_id);

    // Spawn the one-shot server. It owns the exchange + persist + hot-
    // swap chain and shuts itself down on completion (or after
    // CALLBACK_LISTENER_TIMEOUT). The main mira-server never sees the
    // callback — it lands on the temp listener directly.
    let temp_state = LoopbackState {
        expected_flow_id: flow_id.clone(),
        verifier,
        redirect_uri: redirect_uri.clone(),
        app: state.clone(),
    };
    tokio::spawn(run_loopback_listener(listener, temp_state));

    Json(StartResponse {
        authorize_url,
        flow_id,
    })
    .into_response()
}

/// Small state bag threaded into the one-shot callback handler. Held
/// by-value (not Arc'd) since only one handler will ever read it.
#[derive(Clone)]
struct LoopbackState {
    expected_flow_id: String,
    verifier: String,
    redirect_uri: String,
    app: AppState,
}

async fn run_loopback_listener(listener: TcpListener, state: LoopbackState) {
    // Wait once for the browser callback, then either persist a bundle
    // or render a friendly error page. Both success and failure paths
    // use `loopback::html_page` for the response body.
    let cb = match loopback::await_callback(
        listener,
        &loopback::html_page(
            "Signed in with ChatGPT",
            "You can close this tab and return to Mira.",
            true,
        ),
        CALLBACK_LISTENER_TIMEOUT,
    )
    .await
    {
        Ok(cb) => cb,
        Err(e) => {
            warn!(%e, "openai oauth: callback listener timed out or failed");
            return;
        }
    };

    if let Err(msg) = handle_callback(&state, cb).await {
        warn!(%msg, "openai oauth: callback handling failed");
    }
}

async fn handle_callback(
    state: &LoopbackState,
    cb: mira_auth::loopback::Callback,
) -> Result<(), String> {
    if let Some(err) = cb.error {
        let desc = cb.error_description.as_deref().unwrap_or("no description");
        return Err(format!("OpenAI reported: {err} — {desc}"));
    }
    let code = cb
        .code
        .ok_or_else(|| "missing `code` on callback".to_owned())?;
    let flow_id = cb
        .state
        .ok_or_else(|| "missing `state` on callback (CSRF guard rejected)".to_owned())?;
    if flow_id != state.expected_flow_id {
        return Err("state mismatch — CSRF guard rejected the callback".to_owned());
    }

    let bundle = openai::exchange_code(&code, &state.verifier, &state.redirect_uri)
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;
    persist_bundle_and_hotswap(&state.app, &bundle)
        .await
        .map_err(|e| format!("Got the token but couldn't save: {e}"))?;
    info!("openai oauth: sign-in complete; provider hot-swapped");
    Ok(())
}

fn error_json(status: StatusCode, msg: String) -> Response {
    #[derive(Serialize)]
    struct E {
        error: String,
    }
    (status, Json(E { error: msg })).into_response()
}

/// Refresh path — called by [`super::refresh`] when the stored bundle
/// is near expiry. Loads the current bundle, hands it to the pure
/// [`openai::refresh`] adapter, and re-persists + hot-swaps.
pub async fn refresh_and_persist(state: &AppState) -> anyhow::Result<()> {
    let Some(current) = store::load(openai::PROVIDER)? else {
        anyhow::bail!("no stored openai bundle to refresh");
    };
    let bundle = openai::refresh(&current).await?;
    persist_bundle_and_hotswap(state, &bundle).await?;
    Ok(())
}

/// Save the bundle to the auth store, mirror the api-key into yaml so
/// the standard provider-build path picks it up, and hot-swap the live
/// [`crate::provider::SwappableProvider`].
async fn persist_bundle_and_hotswap(state: &AppState, bundle: &TokenBundle) -> anyhow::Result<()> {
    store::save(openai::PROVIDER, bundle)?;
    auth_config::write_provider_key(openai::PROVIDER, &bundle.api_key)?;

    let cfg = mira_config::MiraConfig::load_global()?;
    let provider = crate::settings::build_provider_from(&cfg);
    state.provider.set(provider);
    crate::models::invalidate();
    Ok(())
}
