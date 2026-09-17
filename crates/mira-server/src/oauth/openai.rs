//! "Sign in with ChatGPT" — OpenAI OAuth PKCE + token-exchange flow.
//!
//! Two-step auth, mirroring the official Codex CLI
//! (`github.com/openai/codex`, `login/src/server.rs`):
//!
//! **Step A — Authorize + token exchange (standard OAuth code grant):**
//!
//! 1. `POST /api/auth/openai/start` → generate PKCE verifier + flow_id,
//!    stash server-side, return an `authorize_url` pointing at
//!    `https://auth.openai.com/oauth/authorize` with our loopback
//!    redirect and the mandatory Codex CLI flow params
//!    (`id_token_add_organizations=true`,
//!    `codex_cli_simplified_flow=true`).
//! 2. Browser lands there, user consents, OpenAI redirects to
//!    `GET /api/auth/openai/callback?code=…&state=<flow_id>`.
//! 3. Server posts `grant_type=authorization_code` to
//!    `https://auth.openai.com/oauth/token` and receives
//!    `{id_token, access_token, refresh_token}`.
//!
//! **Step B — Token exchange (RFC 8693 grant) for an API key:**
//!
//! 4. Server posts `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`
//!    with `subject_token=<access_token>` and
//!    `requested_token=openai-api-key` to the same `/oauth/token`
//!    endpoint. Response contains an `access_token` in API-key form
//!    (`sk-…`) that authenticates against the standard `api.openai.com/v1`
//!    endpoint.
//! 5. All four tokens (id, access, refresh, api-key) plus the expiry
//!    land in `~/.mira/auth/openai.json` (mode 0600) via
//!    [`super::store`]. The api-key is *also* mirrored into
//!    `~/.mira/mira.yaml` under `providers.openai.api_key` so the
//!    standard provider-build path picks it up unchanged.
//!
//! The refresh loop in [`super::refresh`] rotates all four values
//! periodically (default: 5-minute check, refresh when < 10 minutes
//! from expiry). Refresh reuses [`refresh_and_persist`] below.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::{default_base_url_for, MiraConfig, ProviderConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::state::AppState;

use super::pkce::{code_challenge_s256, gen_flow_id, gen_verifier};
use super::store::{self, now_secs, TokenBundle};

/// OpenAI-issued client ID for the Codex CLI's public OAuth app. Baked
/// into the official CLI (`codex-rs/login/src/auth/manager.rs`) — safe
/// to reuse in another PKCE client since no secret is involved.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Scopes required for the token-exchange step to return an API key.
const OAUTH_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";

/// Where OpenAI's OAuth issuer lives. Kept as a `const` (not env-
/// configurable) so users can't accidentally point at a phishing
/// endpoint — refresh from the real service or nothing.
const OAUTH_ISSUER: &str = "https://auth.openai.com";

/// The Codex CLI's public OAuth client is registered against these
/// exact loopback ports at OpenAI's Hydra allow-list — any other port
/// yields `error_code: unknown_error` at the authorize step. Since
/// we're reusing that client, we have to bind our callback listener on
/// one of these two, not on the main mira-server port. Kept in sync
/// with `codex-rs/login/src/server.rs:60-62`.
const CODEX_CALLBACK_PORT: u16 = 1455;
const CODEX_CALLBACK_PORT_FALLBACK: u16 = 1457;

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
    let challenge = code_challenge_s256(&verifier);
    let flow_id = gen_flow_id();

    // Bind the callback listener FIRST so we know the exact port
    // before we build the authorize URL. If both allowed ports are
    // busy (previous sign-in still cleaning up, or a stale codex
    // process holding one), the whole start fails cleanly instead of
    // constructing a URL the browser will bounce back into a dead
    // socket.
    let (listener, port) = match bind_callback_listener().await {
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
    // Mandatory extras per Codex CLI's own request:
    //   id_token_add_organizations=true  → response id_token includes
    //     the ChatGPT account/org id we need for token exchange.
    //   codex_cli_simplified_flow=true    → server-side flag that gates
    //     the simplified UX + skips the "which app is asking" splash
    //     for known-good clients.
    let authorize_url = format!(
        "{iss}/oauth/authorize\
         ?response_type=code\
         &client_id={cid}\
         &redirect_uri={ru}\
         &scope={scope}\
         &code_challenge={ch}\
         &code_challenge_method=S256\
         &state={st}\
         &id_token_add_organizations=true\
         &codex_cli_simplified_flow=true",
        iss = OAUTH_ISSUER,
        cid = urlencoding::encode(CLIENT_ID),
        ru = urlencoding::encode(&redirect_uri),
        scope = urlencoding::encode(OAUTH_SCOPES),
        ch = urlencoding::encode(&challenge),
        st = urlencoding::encode(&flow_id),
    );

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

/// Try [`CODEX_CALLBACK_PORT`] first, [`CODEX_CALLBACK_PORT_FALLBACK`]
/// second. Both belong to Codex's OAuth client allow-list; anything
/// else yields the opaque authorize-time error.
async fn bind_callback_listener() -> anyhow::Result<(TcpListener, u16)> {
    for port in [CODEX_CALLBACK_PORT, CODEX_CALLBACK_PORT_FALLBACK] {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
        match TcpListener::bind(addr).await {
            Ok(l) => return Ok((l, port)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("both 1455 and 1457 are in use")
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

/// The temp listener's event loop. Accepts one connection, replies
/// with a success/error page, then shuts down. Also shuts down on
/// [`CALLBACK_LISTENER_TIMEOUT`] so a user who abandons the tab
/// doesn't leak the port.
async fn run_loopback_listener(listener: TcpListener, state: LoopbackState) {
    let handle = accept_and_handle(listener, state);
    match tokio::time::timeout(CALLBACK_LISTENER_TIMEOUT, handle).await {
        Ok(()) => {}
        Err(_) => warn!("openai oauth: callback listener timed out waiting for browser"),
    }
}

async fn accept_and_handle(listener: TcpListener, state: LoopbackState) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut socket, _peer) = match listener.accept().await {
        Ok(v) => v,
        Err(e) => {
            warn!(%e, "openai oauth: accept failed");
            return;
        }
    };

    // Read the HTTP request line + headers. A browser callback is
    // small — under 4KB — so a single read is enough in practice.
    // We don't need to speak full HTTP/1.1: parse the request line,
    // extract the query string, respond, close.
    let mut buf = vec![0u8; 8192];
    let n = match socket.read(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            warn!(%e, "openai oauth: read failed");
            return;
        }
    };
    buf.truncate(n);
    let request = String::from_utf8_lossy(&buf);
    let query = parse_callback_query(&request);

    let response_body = match handle_callback(&state, query).await {
        Ok(()) => html_page(
            "Signed in with ChatGPT",
            "You can close this tab and return to Mira.",
            true,
        ),
        Err(msg) => html_page("ChatGPT sign-in failed", &msg, false),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        status = "200 OK",
        len = response_body.len(),
        body = response_body,
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// Best-effort HTTP query parser. Only pulls out the `code`, `state`,
/// `error`, `error_description` fields the OAuth callback carries.
struct CallbackFields {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn parse_callback_query(request: &str) -> CallbackFields {
    let mut fields = CallbackFields {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    // Request line: `GET /auth/callback?code=...&state=... HTTP/1.1\r\n`
    let Some(line) = request.lines().next() else {
        return fields;
    };
    let Some(path_start) = line.find(' ') else {
        return fields;
    };
    let after = &line[path_start + 1..];
    let path_end = after.find(' ').unwrap_or(after.len());
    let path = &after[..path_end];
    let Some(qi) = path.find('?') else {
        return fields;
    };
    let query = &path[qi + 1..];
    for pair in query.split('&') {
        let Some(eq) = pair.find('=') else { continue };
        let k = &pair[..eq];
        let v = urlencoding::decode(&pair[eq + 1..])
            .map(|s| s.into_owned())
            .unwrap_or_default();
        match k {
            "code" => fields.code = Some(v),
            "state" => fields.state = Some(v),
            "error" => fields.error = Some(v),
            "error_description" => fields.error_description = Some(v),
            _ => {}
        }
    }
    fields
}

/// Run the full callback flow — verify state, exchange, token-swap,
/// persist, hot-swap. Returns `Err(msg)` on any failure with a
/// human-readable string suitable for the browser page.
async fn handle_callback(state: &LoopbackState, q: CallbackFields) -> Result<(), String> {
    if let Some(err) = q.error {
        let desc = q.error_description.as_deref().unwrap_or("no description");
        return Err(format!("OpenAI reported: {err} — {desc}"));
    }
    let code = q.code.ok_or_else(|| "missing `code` on callback".to_owned())?;
    let flow_id = q
        .state
        .ok_or_else(|| "missing `state` on callback (CSRF guard rejected)".to_owned())?;
    if flow_id != state.expected_flow_id {
        return Err("state mismatch — CSRF guard rejected the callback".to_owned());
    }

    let bundle = exchange_code_for_bundle(&code, &state.verifier, &state.redirect_uri)
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

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    #[serde(default)]
    id_token: String,
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    /// Seconds until `access_token` expiry. OpenAI returns this on
    /// both the initial exchange and refresh.
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ExchangeApiKeyResponse {
    /// The short-lived OpenAI API key. Returned in the `access_token`
    /// field per RFC 8693 — the field name is generic but the token
    /// itself is the `sk-…` API key our provider adapter uses.
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// Refresh path — called by [`super::refresh`] when the stored bundle
/// is near expiry. Reads the current bundle, uses its refresh_token to
/// mint a new access_token, runs the token-exchange for a new api-key,
/// persists, and hot-swaps the live provider.
pub async fn refresh_and_persist(state: &AppState) -> anyhow::Result<()> {
    let Some(current) = store::load("openai")? else {
        anyhow::bail!("no stored openai bundle to refresh");
    };

    // Step 1: refresh the access + id tokens.
    let refreshed = refresh_access_token(&current.refresh_token).await?;

    // Step 2: token-exchange for a fresh API key. Refresh sometimes
    // returns an empty id_token (only access + refresh) — in that case
    // reuse the previous id_token since it still identifies the same
    // ChatGPT account.
    let id_for_exchange = if refreshed.id_token.is_empty() {
        current.id_token.as_str()
    } else {
        refreshed.id_token.as_str()
    };
    let api_key = token_exchange_for_api_key(id_for_exchange).await?;

    let bundle = TokenBundle {
        api_key: api_key.access_token,
        // OpenAI's refresh flow returns a new refresh_token; if it
        // didn't (shouldn't happen but be defensive), reuse the current
        // one rather than losing the ability to refresh again.
        refresh_token: if refreshed.refresh_token.is_empty() {
            current.refresh_token
        } else {
            refreshed.refresh_token
        },
        access_token: refreshed.access_token,
        id_token: if refreshed.id_token.is_empty() {
            current.id_token
        } else {
            refreshed.id_token
        },
        // Prefer the api-key's expiry since that's what we actually
        // use for API calls. Fall back to the access_token expiry
        // when the server didn't send one on this endpoint.
        expires_at: now_secs()
            + if api_key.expires_in > 0 {
                api_key.expires_in
            } else {
                refreshed.expires_in.max(3600)
            },
        obtained_at: now_secs(),
    };

    persist_bundle_and_hotswap(state, &bundle).await?;
    Ok(())
}

// ---- Wire helpers ------------------------------------------------------

async fn exchange_code_for_bundle(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<TokenBundle> {
    let mut form: HashMap<&str, &str> = HashMap::new();
    form.insert("grant_type", "authorization_code");
    form.insert("code", code);
    form.insert("redirect_uri", redirect_uri);
    form.insert("client_id", CLIENT_ID);
    form.insert("code_verifier", verifier);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{OAUTH_ISSUER}/oauth/token"))
        .form(&form)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("code exchange {status}: {body}");
    }
    let tokens: OAuthTokenResponse = resp.json().await?;

    // Now the RFC 8693 step: exchange the id_token (which carries the
    // ChatGPT account/org info) for an API key.
    let api = token_exchange_for_api_key(&tokens.id_token).await?;

    Ok(TokenBundle {
        api_key: api.access_token,
        refresh_token: tokens.refresh_token,
        access_token: tokens.access_token,
        id_token: tokens.id_token,
        expires_at: now_secs()
            + if api.expires_in > 0 {
                api.expires_in
            } else {
                tokens.expires_in.max(3600)
            },
        obtained_at: now_secs(),
    })
}

async fn refresh_access_token(refresh_token: &str) -> anyhow::Result<OAuthTokenResponse> {
    let mut form: HashMap<&str, &str> = HashMap::new();
    form.insert("grant_type", "refresh_token");
    form.insert("refresh_token", refresh_token);
    form.insert("client_id", CLIENT_ID);
    form.insert("scope", OAUTH_SCOPES);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{OAUTH_ISSUER}/oauth/token"))
        .form(&form)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("refresh {status}: {body}");
    }
    Ok(resp.json().await?)
}

async fn token_exchange_for_api_key(id_token: &str) -> anyhow::Result<ExchangeApiKeyResponse> {
    let mut form: HashMap<&str, &str> = HashMap::new();
    form.insert(
        "grant_type",
        "urn:ietf:params:oauth:grant-type:token-exchange",
    );
    form.insert("client_id", CLIENT_ID);
    // Codex CLI passes the id_token — not the access_token — as the
    // subject. The id_token carries the ChatGPT account/org info
    // (thanks to `id_token_add_organizations=true` on the authorize
    // request) that OpenAI's token-exchange endpoint uses to mint the
    // API key. Passing the access_token here yields a `token_expired`
    // 401 even when the access_token was just issued seconds ago.
    // See codex-rs/login/src/server.rs:1153-1160.
    form.insert("subject_token", id_token);
    form.insert(
        "subject_token_type",
        "urn:ietf:params:oauth:token-type:id_token",
    );
    // Non-standard field OpenAI uses to select which token flavor
    // they mint on the response — value is documented in Codex CLI.
    form.insert("requested_token", "openai-api-key");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{OAUTH_ISSUER}/oauth/token"))
        .form(&form)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token-exchange {status}: {body}");
    }
    Ok(resp.json().await?)
}

/// Save the bundle to the auth store, mirror the api-key into yaml so
/// the standard provider-build path picks it up, and hot-swap the live
/// [`crate::provider::SwappableProvider`].
async fn persist_bundle_and_hotswap(state: &AppState, bundle: &TokenBundle) -> anyhow::Result<()> {
    store::save("openai", bundle)?;

    let mut cfg = MiraConfig::load_global()?;
    let entry = cfg
        .providers
        .entry("openai".to_owned())
        .or_insert_with(|| ProviderConfig {
            base_url: default_base_url_for("openai").map(str::to_owned),
            ..Default::default()
        });
    entry.api_key = Some(bundle.api_key.clone());
    if entry.base_url.is_none() {
        entry.base_url = default_base_url_for("openai").map(str::to_owned);
    }
    if cfg.default_provider.is_none() {
        cfg.default_provider = Some("openai".to_owned());
    }
    cfg.save_global()?;

    let provider = crate::settings::build_provider_from(&cfg);
    state.provider.set(provider);
    crate::models::invalidate();
    Ok(())
}

// ---- Success/error page ------------------------------------------------

/// Raw HTML body — the loopback listener writes this directly into a
/// hand-rolled HTTP/1.1 response (no axum on that path), so we return
/// the string, not an axum `Response`.
fn html_page(title: &str, body: &str, success: bool) -> String {
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
