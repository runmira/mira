//! CLI OAuth commands: `mira login`, `mira logout`, `mira auth <sub>`.
//!
//! These are terminal-driven wrappers around [`mira_auth`] — the same
//! shared crate the `mira serve` browser flow uses. `login` spins up a
//! temporary loopback listener, opens the user's browser to the
//! provider's authorize URL, and blocks until the callback lands (or
//! a 5-minute timeout). Success writes the resulting key to
//! `~/.mira/mira.yaml` (and, for openai, the full token bundle to
//! `~/.mira/auth/openai.json`) so every subsequent `mira` /
//! `mira review` / `mira serve` invocation finds it without any
//! further steps.
//!
//! Refresh: an openai bundle within 10 minutes of expiry auto-refreshes
//! synchronously at CLI startup (`crate::main`) — cheap in the fresh
//! case, one HTTP round-trip in the near-expiry case. Users can force
//! a refresh with `mira auth refresh openai`.
//!
//! Headless flag: `--no-browser` (or `MIRA_NO_BROWSER=1`) skips the
//! browser open and just prints the authorize URL, useful when the CLI
//! is running over SSH without a graphical session.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use mira_auth::config as auth_config;
use mira_auth::loopback;
use mira_auth::pkce::{gen_flow_id, gen_verifier};
use mira_auth::providers::{openai, openrouter};
use mira_auth::store::{self, now_secs, TokenBundle};

/// How long the callback listener stays open before we give up. Matches
/// the server-side value in `mira-server/src/oauth/openai.rs` so both
/// surfaces have the same user-facing patience.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Providers `mira login` knows how to sign in with. Enum'd so clap
/// gives autocomplete + rejects typos at parse time, rather than
/// bouncing a lowercase-vs-uppercase string through the whole flow
/// before failing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    /// Sign in with OpenRouter — one PKCE round-trip, long-lived key.
    Openrouter,
    /// Sign in with ChatGPT (OpenAI) — PKCE + RFC 8693 token exchange,
    /// short-lived key with a refresh_token bundle.
    Openai,
}

impl Provider {
    fn key(&self) -> &'static str {
        match self {
            Self::Openrouter => openrouter::PROVIDER,
            Self::Openai => openai::PROVIDER,
        }
    }
}

// -------- `mira login` --------

#[derive(Args, Debug, Clone)]
pub struct LoginArgs {
    /// Which provider to sign in with.
    #[arg(value_enum)]
    pub provider: Provider,
    /// Print the authorize URL instead of opening a browser. Useful
    /// over SSH or in CI. Also honored via `MIRA_NO_BROWSER=1`.
    #[arg(long)]
    pub no_browser: bool,
}

pub async fn run_login(args: LoginArgs) -> Result<()> {
    let no_browser = args.no_browser || std::env::var_os("MIRA_NO_BROWSER").is_some();
    match args.provider {
        Provider::Openrouter => login_openrouter(no_browser).await,
        Provider::Openai => login_openai(no_browser).await,
    }
}

async fn login_openrouter(no_browser: bool) -> Result<()> {
    // OpenRouter accepts any caller-specified redirect, so an ephemeral
    // kernel-assigned port is fine.
    let (listener, port) = loopback::bind_any()
        .await
        .context("bind loopback port for openrouter")?;
    let verifier = gen_verifier();
    let state = gen_flow_id();
    let callback = format!("http://127.0.0.1:{port}/auth/callback");
    let url = openrouter::authorize_url(&callback, &verifier, &state);

    print_start_banner("OpenRouter", &url, no_browser);
    open_browser(&url, no_browser);

    let cb = loopback::await_callback(listener, &success_html("OpenRouter"), CALLBACK_TIMEOUT)
        .await
        .context("wait for openrouter callback")?;
    validate_callback(&cb, &state)?;
    let code = cb.code.ok_or_else(|| anyhow!("callback missing `code`"))?;

    eprintln!("· exchanging authorization code…");
    let api_key = openrouter::exchange_code(&code, &verifier)
        .await
        .context("openrouter token exchange")?;
    auth_config::write_provider_key(openrouter::PROVIDER, &api_key)
        .context("write openrouter key to mira.yaml")?;

    eprintln!("✓ signed in with OpenRouter. Key written to ~/.mira/mira.yaml.");
    Ok(())
}

async fn login_openai(no_browser: bool) -> Result<()> {
    // Codex's OAuth client only allows these loopback ports.
    let (listener, port) = loopback::bind_first_free(openai::CALLBACK_PORTS)
        .await
        .with_context(|| {
            format!(
                "reserve one of ports {:?} for openai callback — is the Codex CLI or another `mira login` running?",
                openai::CALLBACK_PORTS
            )
        })?;

    let verifier = gen_verifier();
    let state = gen_flow_id();
    // Literally `localhost` — the Codex client's redirect allow-list
    // doesn't accept `127.0.0.1` even though it resolves to the same
    // socket. Matches `oauth/openai.rs` on the server side.
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let url = openai::authorize_url(&redirect_uri, &verifier, &state);

    print_start_banner("ChatGPT (OpenAI)", &url, no_browser);
    open_browser(&url, no_browser);

    let cb = loopback::await_callback(listener, &success_html("ChatGPT"), CALLBACK_TIMEOUT)
        .await
        .context("wait for openai callback")?;
    validate_callback(&cb, &state)?;
    let code = cb.code.ok_or_else(|| anyhow!("callback missing `code`"))?;

    eprintln!("· exchanging authorization code + minting API key…");
    let bundle = match openai::exchange_code(&code, &verifier, &redirect_uri).await {
        Ok(b) => b,
        Err(e) => {
            // Consumer ChatGPT accounts (Free / Plus) don't have an
            // "API organization" attached to them until the user
            // creates one — OpenAI's token-exchange endpoint then
            // refuses the id_token with `missing organization_id`.
            // Codex CLI hides this behind an extra browser step; we
            // don't do that step yet (see mira#3), so turn the raw
            // 401 into an actionable message instead.
            let msg = e.to_string();
            if msg.contains("missing organization_id") || msg.contains("invalid_subject_token") {
                bail!(
                    "OpenAI rejected the token exchange because your ChatGPT \
                     account isn't attached to an API organization yet.\n\n\
                     Fix it in ~30 seconds:\n  \
                     1. Open https://platform.openai.com/settings/organization\n  \
                     2. Create an organization (free — one click).\n  \
                     3. Re-run `mira login openai`.\n\n\
                     If you'd rather skip the OpenAI-specific setup, \
                     `mira login openrouter` works with any account and \
                     gives you access to GPT / Claude / Gemini / everything \
                     through one key."
                );
            }
            return Err(e).context("openai token exchange");
        }
    };
    store::save(openai::PROVIDER, &bundle).context("save openai token bundle")?;
    auth_config::write_provider_key(openai::PROVIDER, &bundle.api_key)
        .context("mirror openai key into mira.yaml")?;

    let ttl_min = bundle.expires_at.saturating_sub(now_secs()) / 60;
    eprintln!(
        "✓ signed in with ChatGPT. Key valid for ~{ttl_min}m; a refresh_token was saved to ~/.mira/auth/openai.json for automatic renewal."
    );
    Ok(())
}

fn validate_callback(cb: &loopback::Callback, expected_state: &str) -> Result<()> {
    if let Some(err) = &cb.error {
        let desc = cb.error_description.as_deref().unwrap_or("no description");
        bail!("provider reported: {err} — {desc}");
    }
    let got = cb
        .state
        .as_deref()
        .ok_or_else(|| anyhow!("callback missing `state`"))?;
    if got != expected_state {
        bail!("state mismatch — CSRF guard rejected the callback (got {got:?})");
    }
    Ok(())
}

// -------- `mira logout` --------

#[derive(Args, Debug, Clone)]
pub struct LogoutArgs {
    /// Which provider's credentials to forget.
    #[arg(value_enum)]
    pub provider: Provider,
}

pub async fn run_logout(args: LogoutArgs) -> Result<()> {
    let cleared_yaml = auth_config::clear_provider_key(args.provider.key())
        .with_context(|| format!("clear {} key from mira.yaml", args.provider.key()))?;
    let cleared_bundle = match args.provider {
        Provider::Openai => store::delete(openai::PROVIDER).context("delete openai bundle")?,
        Provider::Openrouter => false, // no bundle file
    };

    match (cleared_yaml, cleared_bundle) {
        (false, false) => println!("(already signed out of {})", args.provider.key()),
        (yaml, bundle) => {
            let mut parts: Vec<&str> = Vec::new();
            if yaml {
                parts.push("mira.yaml key");
            }
            if bundle {
                parts.push("token bundle");
            }
            println!(
                "✓ signed out of {}: cleared {}.",
                args.provider.key(),
                parts.join(" + ")
            );
        }
    }
    Ok(())
}

// -------- `mira auth <sub>` --------

#[derive(Args, Debug, Clone)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub sub: AuthSub,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSub {
    /// Per-provider: signed in? key mirrored? bundle exists? expires when?
    Status,
    /// Force-refresh a provider's stored token bundle now. Useful for
    /// diagnosing a suspected expiry issue without waiting for the
    /// startup auto-refresh.
    Refresh {
        #[arg(value_enum)]
        provider: Provider,
    },
}

pub async fn run_auth(args: AuthArgs) -> Result<()> {
    match args.sub {
        AuthSub::Status => run_auth_status(),
        AuthSub::Refresh { provider } => run_auth_refresh(provider).await,
    }
}

fn run_auth_status() -> Result<()> {
    let cfg = mira_config::MiraConfig::load_global().context("load ~/.mira/mira.yaml")?;
    let default = cfg.default_provider.as_deref().unwrap_or("(none)");
    println!("default provider: {default}");
    println!();

    for provider in [Provider::Openrouter, Provider::Openai] {
        let key_in_yaml = cfg
            .providers
            .get(provider.key())
            .and_then(|p| p.api_key.as_deref())
            .is_some();
        let bundle = match provider {
            Provider::Openai => store::load(openai::PROVIDER).ok().flatten(),
            Provider::Openrouter => None,
        };

        let status = match (key_in_yaml, &bundle) {
            (false, None) => "not signed in".to_owned(),
            (true, None) => "key in yaml (no OAuth bundle)".to_owned(),
            (true, Some(b)) => format!(
                "signed in · bundle expires in {}",
                format_ttl(b.expires_at.saturating_sub(now_secs()))
            ),
            (false, Some(_)) => "bundle present but yaml key missing — try re-login".to_owned(),
        };
        println!("· {} — {status}", provider.key());
    }
    Ok(())
}

async fn run_auth_refresh(provider: Provider) -> Result<()> {
    match provider {
        Provider::Openrouter => {
            bail!("openrouter tokens don't expire — nothing to refresh. Use `mira login openrouter` to rotate.")
        }
        Provider::Openai => {
            let Some(current) = store::load(openai::PROVIDER)? else {
                bail!("no openai bundle on disk — run `mira login openai` first")
            };
            eprintln!("· refreshing openai token…");
            let fresh = openai::refresh(&current).await.context("openai refresh")?;
            store::save(openai::PROVIDER, &fresh)?;
            auth_config::write_provider_key(openai::PROVIDER, &fresh.api_key)?;
            let ttl = fresh.expires_at.saturating_sub(now_secs());
            println!(
                "✓ openai bundle refreshed. New key valid for ~{}.",
                format_ttl(ttl)
            );
            Ok(())
        }
    }
}

// -------- shared helpers --------

/// Best-effort browser open. If it fails or `no_browser` is set, we've
/// already printed the URL so the user can paste it manually.
fn open_browser(url: &str, no_browser: bool) {
    if no_browser {
        return;
    }
    #[cfg(target_os = "macos")]
    let cmd = ("open", &[url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = ("xdg-open", &[url]);
    #[cfg(target_os = "windows")]
    let cmd = ("cmd", &["/C", "start", "", url]);

    let (bin, args) = cmd;
    let spawn = std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if spawn.is_err() {
        eprintln!(
            "(couldn't launch a browser — open the URL above manually, or re-run with --no-browser to suppress this attempt.)"
        );
    }
}

fn print_start_banner(label: &str, url: &str, no_browser: bool) {
    eprintln!();
    eprintln!("→ Signing in with {label}");
    if no_browser {
        eprintln!("  Open this URL in your browser (--no-browser was set):");
    } else {
        eprintln!("  Opening your browser… If nothing happens, paste this URL manually:");
    }
    eprintln!();
    eprintln!("  {url}");
    eprintln!();
    eprintln!("  Waiting for the callback (Ctrl+C to abort)…");
}

fn success_html(provider: &str) -> String {
    loopback::html_page(
        &format!("Signed in with {provider}"),
        "You can close this tab and return to the terminal.",
        true,
    )
}

fn format_ttl(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

// -------- startup auto-refresh --------

/// Called from `main()` on every CLI entry that will hit a provider.
/// If a stored openai bundle is within 10 minutes of expiry, refresh
/// it synchronously so the first chat call doesn't 401. Silently
/// no-ops when there's no bundle (user pasted an api_key manually) or
/// the bundle is still fresh — the fast path is a single JSON read.
///
/// Refresh failures are logged as warnings but don't fail the CLI —
/// the user's stored key is still valid until it actually expires.
pub async fn auto_refresh_if_needed() {
    let bundle = match store::load(openai::PROVIDER) {
        Ok(Some(b)) => b,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(%e, "auth: couldn't read openai bundle at startup");
            return;
        }
    };
    // 10-minute buffer — matches the server's REFRESH_LEEWAY.
    if !bundle.is_near_expiry(10 * 60) {
        return;
    }
    tracing::debug!(
        expires_at = bundle.expires_at,
        "auth: openai bundle near expiry, refreshing synchronously"
    );
    match openai::refresh(&bundle).await {
        Ok(fresh) => {
            if let Err(e) = store::save(openai::PROVIDER, &fresh) {
                tracing::warn!(%e, "auth: couldn't persist refreshed openai bundle");
                return;
            }
            if let Err(e) = auth_config::write_provider_key(openai::PROVIDER, &fresh.api_key) {
                tracing::warn!(%e, "auth: couldn't mirror refreshed openai key into mira.yaml");
            }
        }
        Err(e) => {
            tracing::warn!(%e, "auth: openai refresh failed at startup; using stale key");
        }
    }
    // Suppress unused import warning when TokenBundle isn't referenced
    // in tests; the load() call above depends on it transitively.
    let _phantom: Option<TokenBundle> = None;
}
