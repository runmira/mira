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
use mira_ai::{build_chat_provider, ChatProvider, NullProvider};
use mira_harness::{FileStore, SessionConfig, SessionStore};
use mira_policy::{Policy, PolicyConfig};
use mira_sandbox::Sandbox;
use mira_server::mcp::{McpBootStatus, McpToolInfo};
use mira_server::ServerConfig;
use mira_tools::{builtin, Registry};
use tokio::sync::Mutex;

use mira_config::RuntimeState;

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
    let launch_cwd = std::env::current_dir().context("failed to read cwd")?;
    // Prefer the folder the user last picked over the process's launch
    // directory — restarting mira shouldn't yank them back to whatever
    // shell they happened to type the command from.
    let state = RuntimeState::load().unwrap_or_default();
    let cwd = state
        .last_cwd
        .as_ref()
        .filter(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| launch_cwd.clone());
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    mira_config::export_keys_to_env(&cfg);

    // Reuse the CLI's resolver, but don't hard-fail if the user hasn't set
    // credentials yet — the settings UI is the fix for that.
    let resolved = super::resolve_settings(cli, &cfg).ok();

    let provider = build_initial_provider(&resolved);

    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_core(&mut registry);
    if cfg.memory.tools_enabled() {
        builtin::register_memory(&mut registry);
    }
    // Skills: bundled + user + project, three tiers overriding by name.
    // The RwLock lets the server hot-swap the project tier on cwd change
    // (see the `put_cwd` handler) without re-registering the tool.
    let skills_registry = mira_skills::SkillRegistry::load_layered(
        &mira_config::user_skills_dir(),
        &mira_config::project_skills_dir(&cwd),
    );
    let skills_handle: mira_tools::builtin::skill::SkillHandle = std::sync::Arc::new(
        tokio::sync::RwLock::new(std::sync::Arc::new(skills_registry)),
    );
    builtin::register_skills(&mut registry, skills_handle.clone());
    // `memory_consolidate` — dedup/merge a MIRA.md via a cheap model.
    // Uses the extractor-model config knob (same fallback path as the
    // background auto-extractor), or the initial session model when no
    // cheap tier is configured. Gated behind `memory.tools_enabled`.
    if cfg.memory.tools_enabled() {
        let fallback = resolved
            .as_ref()
            .map(|s| s.model.clone())
            .or_else(|| cfg.default_model.clone())
            .unwrap_or_else(|| "unconfigured".to_owned());
        let consolidate_model = cfg
            .memory
            .extractor_model()
            .map(str::to_owned)
            .unwrap_or(fallback);
        builtin::register_consolidate(&mut registry, provider.clone(), consolidate_model);
    }
    // Same MCP wiring as the CLI entrypoint (see main.rs): one broken
    // server must not stop `mira serve` from booting — the user needs the
    // Settings UI reachable to fix it.
    //
    // In addition to registering tools we capture per-server outcome into
    // `mcp_boot` so the Plugins UI (`GET /api/mcp`) can render live status
    // (connected + tool list, or the connect error) without re-attempting
    // to connect on every request.
    let mut mcp_boot: Vec<McpBootStatus> = Vec::with_capacity(cfg.mcp_servers.len());
    for (name, server_cfg) in &cfg.mcp_servers {
        match mira_tools::connect_mcp(name, server_cfg).await {
            Ok(conn) => {
                let count = conn.tools.len();
                // Snapshot each tool's spec BEFORE moving the Arc into the
                // registry — the spec pass is cheap and gives the UI the
                // namespaced name + description without touching the live
                // MCP service.
                let infos: Vec<McpToolInfo> = conn
                    .tools
                    .iter()
                    .map(|t| {
                        let spec = t.spec();
                        McpToolInfo {
                            name: spec.name,
                            description: spec.description,
                        }
                    })
                    .collect();
                for tool in conn.tools {
                    registry.register_arc(tool);
                }
                mcp_boot.push(McpBootStatus {
                    name: name.clone(),
                    config: server_cfg.clone(),
                    tools: infos,
                    error: None,
                });
                eprintln!(
                    "mcp `{name}`: {count} tool{} registered",
                    if count == 1 { "" } else { "s" }
                );
            }
            Err(e) => {
                let msg = format!("{e:#}");
                eprintln!("warning: mcp `{name}` disabled ({msg})");
                mcp_boot.push(McpBootStatus {
                    name: name.clone(),
                    config: server_cfg.clone(),
                    tools: Vec::new(),
                    error: Some(msg),
                });
            }
        }
    }
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

    // Same "last wins" logic as cwd: prefer the model the user last picked
    // in the chip over the yaml default so a restart doesn't yank them off
    // whichever model they were happily using.
    let model = state
        .last_model
        .clone()
        .or_else(|| resolved.as_ref().map(|s| s.model.clone()))
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
        memory_runtime: cfg.memory.clone(),
        mcp_boot,
        skills: skills_handle,
    })
    .await
}

fn build_initial_provider(resolved: &Option<super::ResolvedSettings>) -> Arc<dyn ChatProvider> {
    let Some(s) = resolved else {
        return Arc::new(NullProvider::default());
    };
    match build_chat_provider(
        &s.provider_name,
        s.base_url.clone(),
        s.api_key.clone(),
        s.extra_headers.clone(),
        s.prompt_caching,
    ) {
        Ok(p) => p,
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
