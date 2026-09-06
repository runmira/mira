//! `mira serve` — a local HTTP/WebSocket front-end.
//!
//! Boots even without a configured provider: the UI can open the settings
//! panel, save credentials to `~/.mira/mira.yaml`, and the server hot-swaps
//! the real provider in without a restart. If the user did pre-configure
//! (env vars, existing yaml, CLI flags), we honour it — same resolution as
//! interactive chat.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_ai::{ChatProvider, NullProvider};
use mira_harness::{FileStore, SessionConfig, SessionStore};
use mira_policy::{Policy, PolicyConfig};
use mira_sandbox::Sandbox;
use mira_server::ServerConfig;
use mira_tools::{builtin, Registry};
use tokio::sync::Mutex;

use crate::config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct ServeArgs {
    /// TCP port to bind.
    #[arg(long, default_value_t = 8787)]
    pub port: u16,

    /// Host to bind. Defaults to loopback — pass `0.0.0.0` to listen on all
    /// interfaces (only do this on trusted networks).
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Serve a built frontend from this directory instead of the placeholder
    /// page. In development, point Vite's dev server at the API instead.
    #[arg(long, value_name = "DIR")]
    pub static_dir: Option<PathBuf>,

    /// Open the URL in the system browser after startup. (Best effort — no-op
    /// on headless machines.)
    #[arg(long)]
    pub open: bool,
}

pub async fn run(cli: &super::Cli, args: ServeArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;

    // Reuse the CLI's resolver, but don't hard-fail if the user hasn't set
    // credentials yet — the settings UI is the fix for that.
    let resolved = super::resolve_settings(cli, &cfg).ok();

    let provider = build_initial_provider(&resolved);

    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_default(&mut registry);
    let registry = Arc::new(registry);

    let mode = resolved.as_ref().map(|s| s.mode).unwrap_or_default();
    let policy = Policy::from_config(&PolicyConfig {
        mode,
        allow: cfg.permissions.allow.clone(),
        ask: cfg.permissions.ask.clone(),
        deny: cfg.permissions.deny.clone(),
    })
    .context("compile policy")?;
    let policy = Arc::new(Mutex::new(policy));

    let store: Option<Arc<dyn SessionStore>> = if cli.no_persist {
        None
    } else {
        match FileStore::open_default() {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                eprintln!("warning: persistence disabled ({e})");
                None
            }
        }
    };

    let resume = super::resume_target(cli.resume.as_deref(), store.as_deref(), &cwd).await?;

    let model = resolved
        .as_ref()
        .map(|s| s.model.clone())
        .or_else(|| cfg.default_model.clone())
        .unwrap_or_else(|| "unconfigured".to_owned());
    let mut sess_cfg = SessionConfig::new(model.clone());
    sess_cfg.max_tokens = resolved.as_ref().and_then(|s| s.max_tokens);
    sess_cfg.temperature = resolved.as_ref().and_then(|s| s.temperature);

    let host: IpAddr = args
        .host
        .parse()
        .with_context(|| format!("invalid --host `{}`", args.host))?;
    let bind = SocketAddr::from((host, args.port));

    let url = format!(
        "http://{}:{}",
        if host.is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            host.to_string()
        },
        args.port
    );

    eprintln!("mira serve: {url}");
    eprintln!(
        "            cwd={} model={} mode={}",
        cwd.display(),
        model,
        mode.as_str()
    );
    if resolved.is_none() {
        eprintln!("            (no provider configured — open the UI Settings panel to add one)");
    }
    if args.static_dir.is_none() && !mira_server::has_embedded_frontend() {
        eprintln!(
            "            (placeholder UI — build the React frontend and pass --static-dir dist/)"
        );
    }

    if args.open {
        try_open(&url);
    }

    mira_server::run(ServerConfig {
        cfg: sess_cfg,
        provider,
        registry,
        policy,
        sandbox,
        cwd,
        store,
        resume,
        bind,
        static_dir: args.static_dir,
    })
    .await
}

fn build_initial_provider(resolved: &Option<super::ResolvedSettings>) -> Arc<dyn ChatProvider> {
    let Some(s) = resolved else {
        return Arc::new(NullProvider::default());
    };
    match OpenAiCompatible::new(OpenAiConfig {
        base_url: s.base_url.clone(),
        api_key: s.api_key.clone(),
        extra_headers: s.extra_headers.clone(),
    }) {
        Ok(p) => Arc::new(p),
        Err(e) => Arc::new(NullProvider::new(format!("provider build failed: {e}"))),
    }
}

fn try_open(url: &str) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(url).status();
}
