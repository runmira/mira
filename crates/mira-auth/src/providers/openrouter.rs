//! "Sign in with OpenRouter" — PKCE OAuth flow.
//!
//! OpenRouter's flow is the simple case: PKCE code exchange returns a
//! long-lived `sk-or-…` API key that gets written to `mira.yaml` and
//! never expires client-side. No refresh, no token bundle.
//!
//! Callers combine the two functions here with a
//! [`crate::loopback`] listener to run a full sign-in:
//!
//! ```no_run
//! # use mira_auth::{loopback, pkce, providers::openrouter, config};
//! # async fn go() -> anyhow::Result<()> {
//! let (listener, port) = loopback::bind_any().await?;
//! let verifier = pkce::gen_verifier();
//! let state = pkce::gen_flow_id();
//! let callback = format!("http://127.0.0.1:{port}/cb");
//! let url = openrouter::authorize_url(&callback, &verifier, &state);
//! // open `url` in a browser, then:
//! let html = loopback::html_page("Signed in", "Close this tab.", true);
//! let cb = loopback::await_callback(listener, &html, std::time::Duration::from_secs(300)).await?;
//! let key = openrouter::exchange_code(cb.code.as_deref().unwrap(), &verifier).await?;
//! config::write_provider_key(openrouter::PROVIDER, &key)?;
//! # Ok(()) }
//! ```

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::pkce::code_challenge_s256;

/// String key used in `~/.mira/mira.yaml` under `providers.<key>` and
/// (unused today, but reserved) in `~/.mira/auth/<key>.json`.
pub const PROVIDER: &str = "openrouter";

/// Build the authorize URL. `callback_url` is where OpenRouter redirects
/// the browser after the user consents — usually a loopback URL served
/// by [`crate::loopback`]. `state` is the CSRF guard we echo on the
/// callback.
pub fn authorize_url(callback_url: &str, verifier: &str, state: &str) -> String {
    let challenge = code_challenge_s256(verifier);
    format!(
        "https://openrouter.ai/auth?callback_url={cb}&code_challenge={ch}&code_challenge_method=S256&state={st}",
        cb = urlencoding::encode(callback_url),
        ch = urlencoding::encode(&challenge),
        st = urlencoding::encode(state),
    )
}

/// POST the authorization code + verifier to OpenRouter and receive
/// the resulting `sk-or-…` API key. The key is long-lived — callers
/// stash it in `mira.yaml` and never round-trip through this function
/// again unless the user re-signs-in.
pub async fn exchange_code(code: &str, verifier: &str) -> Result<String> {
    #[derive(Serialize)]
    struct Body<'a> {
        code: &'a str,
        code_verifier: &'a str,
        code_challenge_method: &'a str,
    }
    #[derive(Deserialize)]
    struct Resp {
        key: String,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&Body {
            code,
            code_verifier: verifier,
            code_challenge_method: "S256",
        })
        .send()
        .await
        .context("post openrouter.ai/api/v1/auth/keys")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("openrouter exchange {status}: {body}"));
    }
    let parsed: Resp = resp
        .json()
        .await
        .context("parse openrouter exchange response")?;
    Ok(parsed.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_encodes_callback_and_state() {
        let url = authorize_url("http://127.0.0.1:1234/cb", "v", "st ate");
        assert!(url.contains("callback_url=http%3A%2F%2F127.0.0.1%3A1234%2Fcb"));
        assert!(url.contains("code_challenge_method=S256"));
        // Space in state is encoded, not left raw.
        assert!(url.contains("state=st%20ate"));
    }
}
