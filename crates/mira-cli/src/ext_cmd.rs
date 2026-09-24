//! `mira mcp …` and `mira plugin …`, shaped like `claude mcp` and
//! `claude plugin`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use mira_config::{McpHttpConfig, McpRemoteKind, McpServerConfig, McpStdioConfig, MiraConfig};
use mira_mcp::{McpState, Status};
use mira_server::extensions::Extensions;

#[derive(Args, Debug, Clone)]
pub struct McpArgs {
    #[command(subcommand)]
    pub cmd: McpCmd,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum ScopeArg {
    /// This project only, private to you (default).
    Local,
    /// Shared with the project in `.mcp.json`.
    Project,
    /// Every project (`~/.mira/mira.yaml`).
    User,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum TransportArg {
    Stdio,
    Http,
    Sse,
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpCmd {
    /// Add a server: `mira mcp add github -- npx -y @modelcontextprotocol/server-github`
    /// or `mira mcp add --transport http linear https://mcp.linear.app/mcp`.
    Add {
        name: String,
        /// Command (stdio) or URL (http/sse).
        command_or_url: String,
        /// Arguments for a stdio command.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        #[arg(short, long, value_enum, default_value_t = ScopeArg::Local)]
        scope: ScopeArg,
        /// Defaults to `http` for URLs, `stdio` otherwise.
        #[arg(short, long, value_enum)]
        transport: Option<TransportArg>,
        /// Environment variable for a stdio server (`KEY=value`).
        #[arg(short, long = "env")]
        env: Vec<String>,
        /// Header for an http/sse server (`Name: value`).
        #[arg(short = 'H', long = "header")]
        header: Vec<String>,
    },
    /// Add a server from its JSON definition (`.mcp.json` entry shape).
    AddJson {
        name: String,
        json: String,
        #[arg(short, long, value_enum, default_value_t = ScopeArg::Local)]
        scope: ScopeArg,
    },
    /// List servers with their live status.
    List,
    /// Show one server: status, tools, prompts, resources.
    Get { name: String },
    /// Remove a server.
    Remove {
        name: String,
        /// Which definition to remove when the name exists in several.
        #[arg(short, long, value_enum)]
        scope: Option<ScopeArg>,
    },
    /// Sign in to a remote server with OAuth (opens your browser).
    Login { name: String },
    /// Forget a remote server's sign-in.
    Logout { name: String },
    /// Turn a server on.
    Enable { name: String },
    /// Turn a server off without removing it.
    Disable { name: String },
    /// Allow a project server (`.mcp.json`) to run.
    Approve { name: String },
    /// Refuse a project server.
    Reject { name: String },
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().context("read cwd")
}

async fn extensions() -> Result<Extensions> {
    let ext = Extensions::new(Some(cwd()?));
    ext.reload().await;
    Ok(ext)
}

fn parse_pairs(items: &[String], sep: char, what: &str) -> Result<BTreeMap<String, String>> {
    items
        .iter()
        .map(|i| {
            let (k, v) = i
                .split_once(sep)
                .ok_or_else(|| anyhow!("{what} `{i}` should look like KEY{sep}value"))?;
            Ok((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect()
}

/// Write (or with `None`, delete) a server in a scope's file. Returns
/// whether anything was there to delete.
fn write(
    scope: ScopeArg,
    project: &Path,
    name: &str,
    cfg: Option<McpServerConfig>,
) -> Result<bool> {
    let existed;
    match scope {
        ScopeArg::User => {
            let mut c = MiraConfig::load_global()?;
            existed = c.mcp_servers.contains_key(name);
            match cfg {
                Some(cfg) => {
                    c.mcp_servers.insert(name.to_owned(), cfg);
                }
                None => {
                    c.mcp_servers.remove(name);
                }
            }
            c.save_global()?;
        }
        ScopeArg::Local => {
            let path = mira_mcp::paths::state_file();
            let mut st = McpState::load(&path)?;
            existed = st.local_servers(Some(project)).contains_key(name);
            st.set_local_server(project, name, cfg);
            st.save(&path)?;
        }
        ScopeArg::Project => {
            let path = project.join(".mcp.json");
            let mut doc: serde_json::Value = match std::fs::read_to_string(&path) {
                Ok(t) => {
                    serde_json::from_str(&t).with_context(|| format!("parse {}", path.display()))?
                }
                Err(_) => serde_json::json!({}),
            };
            let servers = doc
                .as_object_mut()
                .ok_or_else(|| anyhow!("{} isn't a JSON object", path.display()))?
                .entry("mcpServers")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or_else(|| anyhow!("`mcpServers` isn't an object"))?;
            existed = servers.contains_key(name);
            match cfg {
                Some(cfg) => {
                    servers.insert(name.to_owned(), serde_json::to_value(cfg)?);
                }
                None => {
                    servers.remove(name);
                }
            }
            std::fs::write(&path, serde_json::to_string_pretty(&doc)? + "\n")?;
        }
    }
    Ok(existed)
}

fn scope_label(s: ScopeArg) -> &'static str {
    match s {
        ScopeArg::Local => "local (this project, private)",
        ScopeArg::Project => "project (.mcp.json)",
        ScopeArg::User => "user (~/.mira/mira.yaml)",
    }
}

fn marker(s: &Status) -> &'static str {
    match s {
        Status::Connected => "✓",
        Status::Connecting => "…",
        Status::NeedsAuth | Status::NeedsApproval => "!",
        Status::Disabled | Status::Rejected => "-",
        Status::Failed { .. } => "✗",
    }
}

pub async fn run_mcp(args: McpArgs) -> Result<()> {
    let project = cwd()?;
    match args.cmd {
        McpCmd::Add {
            name,
            command_or_url,
            args,
            scope,
            transport,
            env,
            header,
        } => {
            let is_url =
                command_or_url.starts_with("http://") || command_or_url.starts_with("https://");
            let transport = transport.unwrap_or(if is_url {
                TransportArg::Http
            } else {
                TransportArg::Stdio
            });
            let cfg = match transport {
                TransportArg::Stdio => {
                    if !header.is_empty() {
                        bail!("--header is for http/sse servers");
                    }
                    McpServerConfig::Stdio(McpStdioConfig {
                        kind: None,
                        command: command_or_url,
                        args,
                        env: parse_pairs(&env, '=', "env")?,
                        cwd: None,
                    })
                }
                TransportArg::Http | TransportArg::Sse => {
                    if !args.is_empty() || !env.is_empty() {
                        bail!("arguments and --env are for stdio servers");
                    }
                    McpServerConfig::Http(McpHttpConfig {
                        kind: (transport == TransportArg::Sse).then_some(McpRemoteKind::Sse),
                        url: command_or_url,
                        headers: parse_pairs(&header, ':', "header")?,
                        auth: None,
                        oauth: None,
                    })
                }
            };
            write(scope, &project, &name, Some(cfg))?;
            if scope == ScopeArg::Project {
                approve_own(&name).await;
            }
            println!("Added `{name}` to {}.", scope_label(scope));
            check(&name).await
        }
        McpCmd::AddJson { name, json, scope } => {
            let v: serde_json::Value = serde_json::from_str(&json).context("parse JSON")?;
            let cfg = mira_mcp::spec::config_from_json(&v).map_err(|e| anyhow!(e))?;
            write(scope, &project, &name, Some(cfg))?;
            if scope == ScopeArg::Project {
                approve_own(&name).await;
            }
            println!("Added `{name}` to {}.", scope_label(scope));
            check(&name).await
        }
        McpCmd::List => {
            let ext = extensions().await?;
            println!("Checking MCP server health…\n");
            ext.mcp()
                .wait_settled(ext.mcp().options().connect_timeout)
                .await;
            let servers = ext.mcp().servers();
            if servers.is_empty() {
                println!("No MCP servers configured. Add one with `mira mcp add`.");
            }
            for s in servers {
                println!(
                    "{} {}: {} ({}, {}) - {}",
                    marker(&s.status),
                    s.name,
                    s.target,
                    s.transport,
                    s.scope.label(),
                    match &s.status {
                        Status::Connected => format!("connected, {} tools", s.tools.len()),
                        other => other.label(),
                    }
                );
            }
            for n in ext.notices() {
                println!("\n{n}");
            }
            ext.mcp().shutdown();
            Ok(())
        }
        McpCmd::Get { name } => {
            let ext = extensions().await?;
            ext.mcp()
                .wait_settled(ext.mcp().options().connect_timeout)
                .await;
            let s = ext
                .mcp()
                .server(&name)
                .ok_or_else(|| anyhow!("no MCP server `{name}`"))?;
            println!("{}:", s.name);
            println!("  Scope: {}", s.scope.label());
            if let Some(src) = &s.source {
                println!("  Defined in: {src}");
            }
            println!("  Type: {}", s.transport);
            println!(
                "  {}: {}",
                if s.transport == "stdio" {
                    "Command"
                } else {
                    "URL"
                },
                s.target
            );
            println!("  Status: {}", s.status.label());
            if let (Some(n), Some(v)) = (&s.server_name, &s.server_version) {
                println!("  Server: {n} {v}");
            }
            if s.can_sign_in {
                println!("  Signed in: {}", if s.signed_in { "yes" } else { "no" });
            }
            if let Some(log) = &s.log_path {
                println!("  Log: {log}");
            }
            if !s.tools.is_empty() {
                println!("  Tools:");
                for t in &s.tools {
                    let d: String = t
                        .description
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(80)
                        .collect();
                    println!("    {}  {d}", t.name);
                }
            }
            if !s.prompts.is_empty() {
                println!("  Prompts (slash commands):");
                for p in &s.prompts {
                    println!("    /{}", mira_mcp::tool_name(&s.name, &p.name));
                }
            }
            if !s.resources.is_empty() {
                println!("  Resources: {}", s.resources.len());
            }
            ext.mcp().shutdown();
            Ok(())
        }
        McpCmd::Remove { name, scope } => {
            let scopes = match scope {
                Some(s) => vec![s],
                None => vec![ScopeArg::Local, ScopeArg::Project, ScopeArg::User],
            };
            for s in scopes {
                if write(s, &project, &name, None)? {
                    println!("Removed `{name}` from {}.", scope_label(s));
                    return Ok(());
                }
            }
            bail!(
                "no MCP server `{name}` in {}",
                match scope {
                    Some(s) => scope_label(s).to_owned(),
                    None =>
                        "any scope you can edit (plugin servers: `mira mcp disable`)".to_owned(),
                }
            )
        }
        McpCmd::Login { name } => {
            let ext = extensions().await?;
            ext.mcp()
                .wait_settled(ext.mcp().options().connect_timeout)
                .await;
            ext.mcp()
                .sign_in_with_loopback(&name, |url| {
                    println!("Opening your browser to sign in to `{name}`…\nIf it doesn't open, visit:\n  {url}\n");
                    crate::tui::open_browser(url);
                })
                .await
                .map_err(|e| anyhow!(e))?;
            ext.mcp().wait_settled(Duration::from_secs(30)).await;
            let status = ext
                .mcp()
                .server(&name)
                .map(|s| s.status.label())
                .unwrap_or_default();
            println!("Signed in to `{name}` ({status}).");
            ext.mcp().shutdown();
            Ok(())
        }
        McpCmd::Logout { name } => {
            let ext = extensions().await?;
            ext.mcp().sign_out(&name).await.map_err(|e| anyhow!(e))?;
            println!("Signed out of `{name}`.");
            ext.mcp().shutdown();
            Ok(())
        }
        McpCmd::Enable { name } => toggle(&name, "Enabled", |m, n| m.set_enabled(n, true)).await,
        McpCmd::Disable { name } => toggle(&name, "Disabled", |m, n| m.set_enabled(n, false)).await,
        McpCmd::Approve { name } => toggle(&name, "Approved", |m, n| m.set_approved(n, true)).await,
        McpCmd::Reject { name } => toggle(&name, "Rejected", |m, n| m.set_approved(n, false)).await,
    }
}

/// Mark a project server the user just added themselves as approved.
async fn approve_own(name: &str) {
    if let Ok(ext) = extensions().await {
        let _ = ext.mcp().set_approved(name, true);
        ext.mcp().shutdown();
    }
}

/// Connect once and report, so a typo shows up right away.
async fn check(name: &str) -> Result<()> {
    let ext = extensions().await?;
    ext.mcp()
        .wait_settled(ext.mcp().options().connect_timeout)
        .await;
    if let Some(s) = ext.mcp().server(name) {
        match &s.status {
            Status::Connected => println!("✓ Connected: {} tools.", s.tools.len()),
            Status::NeedsAuth => println!("! Needs sign-in: run `mira mcp login {name}`."),
            other => println!("{} {}", marker(other), other.label()),
        }
    }
    ext.mcp().shutdown();
    Ok(())
}

async fn toggle(
    name: &str,
    done: &str,
    f: impl FnOnce(&mira_mcp::McpManager, &str) -> Result<(), String>,
) -> Result<()> {
    let ext = extensions().await?;
    let r = f(ext.mcp(), name);
    ext.mcp().shutdown();
    r.map_err(|e| anyhow!(e))?;
    println!("{done} `{name}`.");
    Ok(())
}

#[derive(Args, Debug, Clone)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub cmd: PluginCmd,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PluginCmd {
    /// Installed plugins.
    List,
    /// Plugins available in your marketplaces.
    Browse {
        query: Option<String>,
    },
    /// A plugin's details and components.
    Info {
        plugin: String,
    },
    /// Install a plugin: `name` or `name@marketplace`.
    Install {
        plugin: String,
    },
    /// Reinstall a plugin from its marketplace's current version.
    Update {
        plugin: String,
    },
    Uninstall {
        plugin: String,
    },
    Enable {
        plugin: String,
    },
    Disable {
        plugin: String,
    },
    /// Manage marketplaces.
    Marketplace {
        #[command(subcommand)]
        cmd: MarketplaceCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MarketplaceCmd {
    /// Add one: `owner/repo`, a git URL, a URL to marketplace.json, or a
    /// local path.
    Add {
        source: String,
    },
    List,
    /// Fetch the latest catalog (all marketplaces when no name is given).
    Update {
        name: Option<String>,
    },
    /// Remove a marketplace and uninstall its plugins.
    Remove {
        name: String,
    },
}

pub async fn run_plugin(args: PluginArgs) -> Result<()> {
    let pm = mira_plugins::PluginManager::new(mira_plugins::PluginManager::default_root());
    match args.cmd {
        PluginCmd::List => {
            let list = pm.installed()?;
            if list.is_empty() {
                println!("No plugins installed. Browse with `mira plugin browse`.");
            }
            for (p, m, c) in list {
                println!(
                    "{} {} {}{}",
                    if p.enabled { "●" } else { "○" },
                    p.id(),
                    p.version,
                    m.and_then(|m| m.description)
                        .map(|d| format!("\n    {d}"))
                        .unwrap_or_default()
                );
                let mut parts = Vec::new();
                for (items, what) in [
                    (c.commands.len(), "commands"),
                    (c.agents.len(), "agents"),
                    (c.skills.len(), "skills"),
                    (c.mcp_servers.len(), "MCP servers"),
                    (c.hooks.len(), "hooks (not run yet)"),
                ] {
                    if items > 0 {
                        parts.push(format!("{items} {what}"));
                    }
                }
                if !parts.is_empty() {
                    println!("    {}", parts.join(" · "));
                }
                for pr in c.problems {
                    println!("    ! {pr}");
                }
            }
        }
        PluginCmd::Browse { query } => {
            let q = query.unwrap_or_default().to_ascii_lowercase();
            let all = pm.catalog()?;
            if all.is_empty() {
                println!(
                    "No marketplaces yet. Add Anthropic's with:\n  mira plugin marketplace add anthropics/claude-plugins-official"
                );
            }
            for c in all.iter().filter(|c| {
                q.is_empty()
                    || c.name.to_ascii_lowercase().contains(&q)
                    || c.description
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&q)
            }) {
                println!(
                    "{} {}{}\n    {}",
                    if c.installed { "●" } else { " " },
                    c.id,
                    c.category
                        .as_deref()
                        .map(|x| format!("  [{x}]"))
                        .unwrap_or_default(),
                    c.description.as_deref().unwrap_or("")
                );
            }
        }
        PluginCmd::Info { plugin } => {
            let d = pm.detail(&plugin)?;
            println!("{}", d.entry.id);
            if let Some(desc) = &d.entry.description {
                println!("  {desc}");
            }
            if let Some(a) = &d.entry.author {
                println!("  Author: {a}");
            }
            println!("  Source: {}", d.entry.source);
            println!(
                "  Installed: {}",
                match (&d.entry.installed_version, d.entry.enabled) {
                    (Some(v), true) => format!("{v} (enabled)"),
                    (Some(v), false) => format!("{v} (disabled)"),
                    _ => "no".into(),
                }
            );
            if let Some(c) = &d.components {
                let show = |label: &str, items: &[String]| {
                    if !items.is_empty() {
                        println!("  {label}: {}", items.join(", "));
                    }
                };
                show(
                    "Commands",
                    &d.command_names
                        .iter()
                        .map(|c| format!("/{c}"))
                        .collect::<Vec<_>>(),
                );
                show("Agents", &d.agent_names);
                show("Skills", &c.skills);
                show("MCP servers", &c.mcp_server_names);
                show("Hooks (not run yet)", &c.hooks);
                show("LSP servers (not run yet)", &c.lsp_servers);
            } else {
                println!("  Components: known after install");
            }
        }
        PluginCmd::Install { plugin } | PluginCmd::Update { plugin } => {
            let p = pm.install(&plugin).await?;
            println!("Installed {} ({}).", p.id(), p.version);
        }
        PluginCmd::Uninstall { plugin } => {
            pm.uninstall(&plugin).await?;
            println!("Uninstalled {plugin}.");
        }
        PluginCmd::Enable { plugin } => {
            pm.set_enabled(&plugin, true).await?;
            println!("Enabled {plugin}.");
        }
        PluginCmd::Disable { plugin } => {
            pm.set_enabled(&plugin, false).await?;
            println!("Disabled {plugin}.");
        }
        PluginCmd::Marketplace { cmd } => {
            match cmd {
                MarketplaceCmd::Add { source } => {
                    let name = pm.add_marketplace(&source).await?;
                    let count = pm
                        .catalog()?
                        .iter()
                        .filter(|c| c.marketplace == name)
                        .count();
                    println!("Added marketplace `{name}` ({count} plugins). Browse with `mira plugin browse`.");
                }
                MarketplaceCmd::List => {
                    let ms = pm.marketplaces()?;
                    if ms.is_empty() {
                        println!("No marketplaces. Add one with `mira plugin marketplace add <owner/repo>`.");
                    }
                    for m in ms {
                        println!(
                            "{} ({} plugins) — {}{}",
                            m.name,
                            m.plugin_count,
                            m.source,
                            m.error.map(|e| format!("\n    ! {e}")).unwrap_or_default()
                        );
                    }
                }
                MarketplaceCmd::Update { name } => {
                    let names = match name {
                        Some(n) => vec![n],
                        None => pm.marketplaces()?.into_iter().map(|m| m.name).collect(),
                    };
                    for n in names {
                        pm.update_marketplace(&n).await?;
                        println!("Updated `{n}`.");
                    }
                }
                MarketplaceCmd::Remove { name } => {
                    pm.remove_marketplace(&name).await?;
                    println!("Removed `{name}` and its plugins.");
                }
            }
        }
    }
    Ok(())
}
