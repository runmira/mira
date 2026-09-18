//! "Sign in with ChatGPT" — OpenAI OAuth PKCE + RFC 8693 token
//! exchange. See the top-level [`crate`] docs for the flow overview;
//! this module exposes the pure HTTP steps.
//!
//! Structure:
//!
//! * [`authorize_url`] — builds the `auth.openai.com/oauth/authorize`
//!   URL with the mandatory Codex CLI flow params.
//! * [`exchange_code`] — POST the auth-code back, then POST the
//!   id_token in an RFC 8693 token-exchange to receive the actual
//!   `sk-…` API key. Returns a full [`TokenBundle`].
//! * [`refresh`] — swap a near-expiry bundle for a fresh one using
//!   its `refresh_token`. Same two-POST shape as [`exchange_code`].

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::pkce::code_challenge_s256;
use crate::store::{now_secs, TokenBundle};

/// String key used in `~/.mira/mira.yaml` and `~/.mira/auth/<key>.json`.
pub const PROVIDER: &str = "openai";

/// OpenAI-issued client ID for the Codex CLI's public OAuth app. Baked
/// into the official CLI (`codex-rs/login/src/auth/manager.rs`) — safe
/// to reuse in another PKCE client since no secret is involved.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Scopes required for the token-exchange step to return an API key.
pub const OAUTH_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";

/// Where OpenAI's OAuth issuer lives. Kept as a `const` (not env-
/// configurable) so users can't accidentally point at a phishing
/// endpoint — refresh from the real service or nothing.
pub const OAUTH_ISSUER: &str = "https://auth.openai.com";

/// The Codex CLI's public OAuth client is registered against these
/// exact loopback ports at OpenAI's Hydra allow-list — any other port
/// yields `error_code: unknown_error` at the authorize step. Since
/// we're reusing that client, callers have to bind their callback
/// listener on one of these. Kept in sync with
/// `codex-rs/login/src/server.rs:60-62`.
pub const CALLBACK_PORTS: &[u16] = &[1455, 1457];

/// Build the authorize URL. `redirect_uri` must be `http://localhost:<port>/...`
/// with `<port>` from [`CALLBACK_PORTS`] — literally `localhost`, not
/// `127.0.0.1`, per OpenAI's Hydra allow-list.
pub fn authorize_url(redirect_uri: &str, verifier: &str, state: &str) -> String {
    let challenge = code_challenge_s256(verifier);
    // Mandatory extras per Codex CLI's own request:
    //   id_token_add_organizations=true  → response id_token includes
    //     the ChatGPT account/org id we need for token exchange.
    //   codex_cli_simplified_flow=true    → server-side flag that gates
    //     the simplified UX + skips the "which app is asking" splash
    //     for known-good clients.
    format!(
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
        ru = urlencoding::encode(redirect_uri),
        scope = urlencoding::encode(OAUTH_SCOPES),
        ch = urlencoding::encode(&challenge),
        st = urlencoding::encode(state),
    )
}

/// Full sign-in exchange: POST the auth-code for tokens, then RFC 8693
/// the id_token for an `sk-…` API key. Returns a bundle callers can
/// hand to [`crate::store::save`] verbatim.
pub async fn exchange_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenBundle> {
    let tokens = post_authorization_code(code, verifier, redirect_uri).await?;
    // The RFC 8693 step: exchange the id_token (which carries the
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

/// Refresh path: use `current.refresh_token` to mint a new access +
/// id token, then RFC 8693 for a new api-key. Returns the fresh bundle.
///
/// Fields the refresh endpoint sometimes omits (empty `id_token`,
/// empty `refresh_token`) fall back to the values in `current` so the
/// caller doesn't lose the ability to refresh again.
pub async fn refresh(current: &TokenBundle) -> Result<TokenBundle> {
    let refreshed = post_refresh(&current.refresh_token).await?;

    let id_for_exchange = if refreshed.id_token.is_empty() {
        current.id_token.as_str()
    } else {
        refreshed.id_token.as_str()
    };
    let api = token_exchange_for_api_key(id_for_exchange).await?;

    Ok(TokenBundle {
        api_key: api.access_token,
        refresh_token: if refreshed.refresh_token.is_empty() {
            current.refresh_token.clone()
        } else {
            refreshed.refresh_token
        },
        access_token: refreshed.access_token,
        id_token: if refreshed.id_token.is_empty() {
            current.id_token.clone()
        } else {
            refreshed.id_token
        },
        expires_at: now_secs()
            + if api.expires_in > 0 {
                api.expires_in
            } else {
                refreshed.expires_in.max(3600)
            },
        obtained_at: now_secs(),
    })
}

// ---- Wire helpers ------------------------------------------------------

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

async fn post_authorization_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
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
        .await
        .context("post oauth/token (authorization_code)")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("code exchange {status}: {body}"));
    }
    resp.json().await.context("parse oauth/token response")
}

async fn post_refresh(refresh_token: &str) -> Result<OAuthTokenResponse> {
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
        .await
        .context("post oauth/token (refresh_token)")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("refresh {status}: {body}"));
    }
    resp.json().await.context("parse refresh response")
}

async fn token_exchange_for_api_key(id_token: &str) -> Result<ExchangeApiKeyResponse> {
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
        .await
        .context("post oauth/token (token-exchange)")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token-exchange {status}: {body}"));
    }
    resp.json().await.context("parse token-exchange response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_codex_flow_params() {
        let url = authorize_url("http://localhost:1455/auth/callback", "v", "st");
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
    }

    #[test]
    fn callback_ports_match_codex_allowlist() {
        // Regression guard: OpenAI's Hydra client only allows these.
        assert_eq!(CALLBACK_PORTS, &[1455, 1457]);
    }
}
