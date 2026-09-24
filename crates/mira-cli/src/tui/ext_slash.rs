//! `/mcp`, `/plugin`, and custom slash commands in the TUI.
//!
//! Anything slow (sign-in, installs, marketplace fetches) runs in the
//! background and reports through the same channel `/remote-env` uses,
//! so the UI never freezes.

use mira_mcp::Status;

use super::state::TuiState;
use super::{EnvUpdate, TuiConfig};

fn words(rest: &str) -> (String, String) {
    let rest = rest.trim();
    match rest.split_once(char::is_whitespace) {
        Some((a, b)) => (a.to_ascii_lowercase(), b.trim().to_owned()),
        None => (rest.to_ascii_lowercase(), String::new()),
    }
}

fn marker(s: &Status) -> &'static str {
    match s {
        Status::Connected => "●",
        Status::Connecting => "◌",
        Status::NeedsAuth | Status::NeedsApproval => "◆",
        Status::Disabled | Status::Rejected => "○",
        Status::Failed { .. } => "✗",
    }
}

/// Open a URL in the default browser (best effort).
pub(crate) fn open_browser(url: &str) {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub(crate) async fn run_mcp_slash(rest: &str, state: &mut TuiState, cfg: &TuiConfig) {
    let ext = &cfg.extensions;
    let mcp = ext.mcp();
    let (sub, arg) = words(rest);
    let need_name = |state: &mut TuiState, usage: &str| {
        state.push_warning(format!("usage: /mcp {usage} <server>"));
    };
    match sub.as_str() {
        "" | "list" | "status" => {
            let servers = mcp.servers();
            if servers.is_empty() {
                state.push_info(
                    "no MCP servers · add one with `mira mcp add`, the Plugins page in `mira serve`, \
                     or a plugin (/plugin)",
                );
                return;
            }
            for s in &servers {
                let detail = match &s.status {
                    Status::Connected => format!(
                        "connected · {} tool{}{}",
                        s.tools.len(),
                        if s.tools.len() == 1 { "" } else { "s" },
                        if s.prompts.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} prompts", s.prompts.len())
                        }
                    ),
                    other => other.label(),
                };
                state.push_info(format!(
                    "  {} {:<24} {:<34} {} · {}",
                    marker(&s.status),
                    s.name,
                    detail,
                    s.scope.label(),
                    s.transport
                ));
            }
            state.push_info(
                "/mcp tools|reconnect|login|logout|enable|disable|approve|reject <server>",
            );
        }
        "tools" => {
            let Some(s) = mcp.server(&arg) else {
                return need_name(state, "tools");
            };
            if s.tools.is_empty() {
                state.push_info(format!("`{}` has no tools ({})", s.name, s.status.label()));
            }
            for t in &s.tools {
                let first = t.description.lines().next().unwrap_or("");
                let short: String = first.chars().take(90).collect();
                state.push_info(format!("  {}  {short}", t.name));
            }
        }
        "reconnect" | "enable" | "disable" | "approve" | "reject" => {
            if arg.is_empty() {
                return need_name(state, &sub);
            }
            let r = match sub.as_str() {
                "reconnect" => mcp.reconnect(&arg),
                "enable" => mcp.set_enabled(&arg, true),
                "disable" => mcp.set_enabled(&arg, false),
                "approve" => mcp.set_approved(&arg, true),
                _ => mcp.set_approved(&arg, false),
            };
            match r {
                Ok(()) => state.flash = Some(format!("mcp {sub} → {arg}")),
                Err(e) => state.push_warning(e),
            }
        }
        "login" => {
            if arg.is_empty() {
                return need_name(state, "login");
            }
            let mcp = mcp.clone();
            let tx = cfg.env_tx.clone();
            state.push_info(format!("signing in to `{arg}` — finish in your browser…"));
            tokio::spawn(async move {
                let tx2 = tx.clone();
                let r = mcp
                    .sign_in_with_loopback(&arg, move |url| {
                        open_browser(url);
                        let _ = tx2.send(EnvUpdate::Info(format!("sign-in page: {url}")));
                    })
                    .await;
                let _ = tx.send(match r {
                    Ok(()) => EnvUpdate::Info(format!("signed in to `{arg}`; connecting…")),
                    Err(e) => EnvUpdate::Warning(format!("sign-in to `{arg}` failed: {e}")),
                });
            });
        }
        "logout" => {
            if arg.is_empty() {
                return need_name(state, "logout");
            }
            match mcp.sign_out(&arg).await {
                Ok(()) => state.flash = Some(format!("signed out of {arg}")),
                Err(e) => state.push_warning(e),
            }
        }
        other => state.push_warning(format!(
            "unknown `/mcp {other}` — try /mcp, /mcp tools <server>, /mcp login <server>"
        )),
    }
}

pub(crate) async fn run_plugin_slash(rest: &str, state: &mut TuiState, cfg: &TuiConfig) {
    let ext = cfg.extensions.clone();
    let pm = ext.plugins().clone();
    let (sub, arg) = words(rest);
    match sub.as_str() {
        "" | "list" | "installed" => {
            match pm.installed() {
                Ok(list) if list.is_empty() => {
                    state.push_info("no plugins installed · /plugin browse to see what's available")
                }
                Ok(list) => {
                    for (p, _, c) in list {
                        let mut parts = Vec::new();
                        for (n, what) in [
                            (c.commands.len(), "commands"),
                            (c.agents.len(), "agents"),
                            (c.skills.len(), "skills"),
                            (c.mcp_servers.len(), "MCP servers"),
                        ] {
                            if n > 0 {
                                parts.push(format!("{n} {what}"));
                            }
                        }
                        state.push_info(format!(
                            "  {} {:<36} {:<12} {}",
                            if p.enabled { "●" } else { "○" },
                            p.id(),
                            p.version.chars().take(12).collect::<String>(),
                            parts.join(" · ")
                        ));
                    }
                }
                Err(e) => state.push_warning(format!("{e:#}")),
            }
            if let Ok(ms) = pm.marketplaces() {
                if ms.is_empty() {
                    state.push_info(
                        "no marketplaces · try /plugin marketplace add anthropics/claude-plugins-official",
                    );
                } else {
                    let names: Vec<String> = ms
                        .iter()
                        .map(|m| format!("{} ({})", m.name, m.plugin_count))
                        .collect();
                    state.push_info(format!("marketplaces: {}", names.join(" · ")));
                }
            }
            state.push_info(
                "/plugin browse [query] · install|uninstall|enable|disable <plugin> · \
                 marketplace add|update|remove",
            );
        }
        "browse" | "search" | "discover" => {
            let q = arg.to_ascii_lowercase();
            match pm.catalog() {
                Ok(all) => {
                    let hits: Vec<_> = all
                        .into_iter()
                        .filter(|c| {
                            q.is_empty()
                                || c.name.to_ascii_lowercase().contains(&q)
                                || c.description
                                    .as_deref()
                                    .unwrap_or("")
                                    .to_ascii_lowercase()
                                    .contains(&q)
                        })
                        .collect();
                    if hits.is_empty() {
                        state.push_info("no matching plugins");
                    }
                    for c in hits.iter().take(30) {
                        let desc: String = c
                            .description
                            .as_deref()
                            .unwrap_or("")
                            .chars()
                            .take(70)
                            .collect();
                        state.push_info(format!(
                            "  {} {:<40} {desc}",
                            if c.installed { "●" } else { " " },
                            c.id
                        ));
                    }
                    if hits.len() > 30 {
                        state.push_info(format!(
                            "…and {} more · narrow with /plugin browse <query>",
                            hits.len() - 30
                        ));
                    }
                }
                Err(e) => state.push_warning(format!("{e:#}")),
            }
        }
        "install" | "uninstall" | "enable" | "disable" | "update" => {
            if arg.is_empty() {
                state.push_warning(format!("usage: /plugin {sub} <plugin>[@marketplace]"));
                return;
            }
            let tx = cfg.env_tx.clone();
            state.push_info(format!("{sub} {arg}…"));
            tokio::spawn(async move {
                let r = match sub.as_str() {
                    "install" | "update" => pm
                        .install(&arg)
                        .await
                        .map(|p| format!("installed {} ({})", p.id(), p.version)),
                    "uninstall" => pm
                        .uninstall(&arg)
                        .await
                        .map(|_| format!("uninstalled {arg}")),
                    "enable" => pm
                        .set_enabled(&arg, true)
                        .await
                        .map(|_| format!("enabled {arg}")),
                    _ => pm
                        .set_enabled(&arg, false)
                        .await
                        .map(|_| format!("disabled {arg}")),
                };
                match r {
                    Ok(msg) => {
                        ext.reload().await;
                        let _ = tx.send(EnvUpdate::Info(format!(
                            "{msg} · commands, skills and MCP servers are live; \
                             plugin agents load in new sessions"
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(EnvUpdate::Warning(format!("{e:#}")));
                    }
                }
            });
        }
        "marketplace" | "marketplaces" | "market" => {
            let (op, target) = words(&arg);
            let tx = cfg.env_tx.clone();
            match op.as_str() {
                "" | "list" => match pm.marketplaces() {
                    Ok(ms) if ms.is_empty() => state.push_info(
                        "no marketplaces · /plugin marketplace add anthropics/claude-plugins-official",
                    ),
                    Ok(ms) => {
                        for m in ms {
                            state.push_info(format!(
                                "  {:<28} {:>4} plugins  {}{}",
                                m.name,
                                m.plugin_count,
                                m.source,
                                m.error.map(|e| format!("  ✗ {e}")).unwrap_or_default()
                            ));
                        }
                    }
                    Err(e) => state.push_warning(format!("{e:#}")),
                },
                "add" | "update" | "remove" | "rm" => {
                    if target.is_empty() && op != "update" {
                        state.push_warning(format!("usage: /plugin marketplace {op} <source|name>"));
                        return;
                    }
                    state.push_info(format!("marketplace {op} {target}…"));
                    tokio::spawn(async move {
                        let r = match op.as_str() {
                            "add" => pm
                                .add_marketplace(&target)
                                .await
                                .map(|n| format!("added marketplace `{n}` · /plugin browse")),
                            "update" if target.is_empty() => {
                                let names: Vec<String> = pm
                                    .marketplaces()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|m| m.name)
                                    .collect();
                                let mut last = Ok(());
                                for n in &names {
                                    if let Err(e) = pm.update_marketplace(n).await {
                                        last = Err(e);
                                    }
                                }
                                last.map(|_| format!("updated {} marketplaces", names.len()))
                            }
                            "update" => pm
                                .update_marketplace(&target)
                                .await
                                .map(|_| format!("updated `{target}`")),
                            _ => pm
                                .remove_marketplace(&target)
                                .await
                                .map(|_| format!("removed `{target}` and its plugins")),
                        };
                        match r {
                            Ok(msg) => {
                                ext.reload().await;
                                let _ = tx.send(EnvUpdate::Info(msg));
                            }
                            Err(e) => {
                                let _ = tx.send(EnvUpdate::Warning(format!("{e:#}")));
                            }
                        }
                    });
                }
                other => state.push_warning(format!(
                    "unknown `/plugin marketplace {other}` — add, list, update, remove"
                )),
            }
        }
        other => state.push_warning(format!(
            "unknown `/plugin {other}` — try /plugin, /plugin browse, /plugin install <name>"
        )),
    }
}

/// Custom command or MCP prompt. `None`: not one of ours.
pub(crate) async fn custom_command(
    name: &str,
    rest: &str,
    state: &mut TuiState,
    cfg: &TuiConfig,
) -> Option<Option<String>> {
    match cfg.extensions.expand(name, rest).await? {
        Ok(text) => {
            state.flash = Some(format!("/{name}"));
            Some(Some(text))
        }
        Err(e) => {
            state.push_warning(e);
            Some(None)
        }
    }
}

/// Palette rows for custom commands and MCP prompts.
pub(crate) fn palette_items(filter: &str, cfg: &TuiConfig) -> Vec<(String, String)> {
    cfg.extensions
        .commands()
        .into_iter()
        .filter(|c| c.name.to_ascii_lowercase().contains(filter))
        .map(|c| {
            let hint = c.argument_hint.map(|h| format!(" {h}")).unwrap_or_default();
            (
                format!("/{}", c.name),
                format!("{}{hint} · {}", c.source, c.description),
            )
        })
        .collect()
}
