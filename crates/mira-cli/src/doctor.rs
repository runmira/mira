//! `mira doctor` — offline (and optionally online) diagnostics.
//!
//! Prints a checklist covering the pieces that most commonly trip
//! first-run setups: config parse, provider resolution, directory
//! permissions, skill discovery, MCP entries, memory files. With
//! `--ping`, also makes a live `list_models()` call to prove the
//! provider is actually reachable with the resolved key.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use mira_ai::build_chat_provider;
use mira_config::{MiraConfig, RuntimeState};

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Also probe the network — issue a `GET /models` against the
    /// resolved provider to prove the base URL + API key actually work.
    #[arg(long)]
    ping: bool,
}

pub async fn run(cli: &crate::Cli, args: DoctorArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;

    let mut report = Report::new();

    // ---- config files ----
    let global_path = mira_config::global_path();
    check_config_file(&mut report, "global config", &global_path);
    let local_path = cwd.join(".mira").join("config.yaml");
    check_config_file(&mut report, "project config", &local_path);

    let cfg = match MiraConfig::load(&cwd) {
        Ok(c) => {
            report.pass("merged config", "global + project merged cleanly");
            c
        }
        Err(e) => {
            report.fail("merged config", format!("{e:#}"));
            report.print();
            return Ok(());
        }
    };

    // ---- runtime state ----
    match RuntimeState::load() {
        Ok(_) => report.pass(
            "runtime state",
            mira_config::state_path().display().to_string(),
        ),
        Err(e) => report.warn("runtime state", format!("unreadable: {e:#}")),
    }

    // ---- writable directories ----
    check_writable(
        &mut report,
        "~/.mira",
        &parent_or(&mira_config::global_path()),
    );
    check_writable(&mut report, ".mira in cwd", &cwd.join(".mira"));

    // ---- provider resolution ----
    let settings = match crate::resolve_settings(cli, &cfg) {
        Ok(s) => {
            report.pass(
                "provider resolves",
                format!("{} · {} · model={}", s.provider_name, s.base_url, s.model),
            );
            Some(s)
        }
        Err(e) => {
            report.fail("provider resolves", format!("{e:#}"));
            None
        }
    };

    // ---- skills ----
    let shared = mira_config::shared_skills_dir();
    let user = mira_config::user_skills_dir();
    let project_dirs = mira_config::well_known_project_skills_dirs(&cwd);
    let reg = mira_skills::SkillRegistry::load_layered(&shared, &user, &project_dirs);
    let count = reg.names().len();
    if count > 0 {
        report.pass(
            "skills",
            format!("{count} loaded (bundled + user + project)"),
        );
    } else {
        report.warn("skills", "no skills loaded".to_string());
    }

    // ---- mcp servers ----
    if cfg.mcp_servers.is_empty() {
        report.pass("mcp servers", "none configured".to_string());
    } else {
        let names: Vec<&str> = cfg.mcp_servers.keys().map(String::as_str).collect();
        report.pass("mcp servers", names.join(", "));
    }

    // ---- memory files ----
    let user_mem = mira_config::user_memory_path();
    let project_mem = mira_config::project_memory_path(&cwd);
    for (label, p) in [
        ("user MIRA.md", &user_mem),
        ("project MIRA.md", &project_mem),
    ] {
        if p.exists() {
            let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            report.pass(label, format!("{} · {} bytes", p.display(), bytes));
        } else {
            report.info(label, format!("{} (not present)", p.display()));
        }
    }

    // ---- computer use / browser ----
    let computer_on = cli.computer || cfg.computer.enabled();
    if computer_on {
        for c in mira_computer::preflight().await {
            let label = format!("computer · {}", c.label);
            if c.ok {
                report.pass(label, c.detail);
            } else {
                report.fail(label, c.detail);
            }
        }
    } else {
        report.info(
            "computer use",
            "off (enable with --computer or `computer.enabled: true`)",
        );
    }
    let browser_on = cli.browser || cfg.browser.enabled();
    let explicit = cfg.browser.executable_path();
    match (
        browser_on,
        mira_browser::launch::find_executable(explicit.as_deref()),
    ) {
        (true, Ok(p)) => report.pass("browser", p.display().to_string()),
        (true, Err(e)) => report.fail("browser", e.to_string()),
        (false, Ok(p)) => report.info(
            "browser",
            format!("off (enable with --browser) · would use {}", p.display()),
        ),
        (false, Err(_)) => report.info("browser", "off (enable with --browser)"),
    }

    // ---- remote environments ----
    let names: Vec<String> = mira_compute::env::list(&cfg.compute)
        .into_iter()
        .map(|e| e.name)
        .collect();
    report.info("environments", names.join(", "));
    match crate::sandbox::startup_target(cli.sandbox.as_deref(), &cfg.compute) {
        None => report.info("start in", "local (this worktree)"),
        Some(name) => match mira_compute::EnvironmentSpec::resolve(&name, &cfg.compute) {
            Ok(spec) => report.pass("start in", format!("{name} ({})", spec.backend_name())),
            Err(e) => report.fail("start in", format!("{name}: {e}")),
        },
    }

    // ---- optional network probe ----
    if args.ping {
        if let Some(s) = &settings {
            let build = build_chat_provider(
                &s.provider_name,
                s.base_url.clone(),
                s.api_key.clone(),
                s.extra_headers.clone(),
                s.prompt_caching,
            );
            match build {
                Ok(p) => match p.list_models().await {
                    Ok(models) => report.pass(
                        "provider ping",
                        format!("{} model(s) returned", models.len()),
                    ),
                    Err(e) => report.fail("provider ping", format!("{e}")),
                },
                Err(e) => report.fail("provider build", format!("{e:#}")),
            }
        }
    }

    report.print();
    Ok(())
}

fn check_config_file(report: &mut Report, label: &str, path: &Path) {
    if !path.exists() {
        report.info(label, format!("{} (not present)", path.display()));
        return;
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            report.fail(label, format!("{}: {e}", path.display()));
            return;
        }
    };
    match serde_yaml::from_str::<MiraConfig>(&raw) {
        Ok(_) => report.pass(label, path.display().to_string()),
        Err(e) => report.fail(label, format!("{}: {e}", path.display())),
    }
}

fn check_writable(report: &mut Report, label: &str, dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        report.fail(label, format!("mkdir {}: {e}", dir.display()));
        return;
    }
    let probe = dir.join(".mira-doctor-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            report.pass(label, format!("{} (writable)", dir.display()));
        }
        Err(e) => report.fail(label, format!("{}: {e}", dir.display())),
    }
}

fn parent_or(p: &Path) -> PathBuf {
    p.parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

enum Level {
    Pass,
    Warn,
    Fail,
    Info,
}

struct Row {
    level: Level,
    label: String,
    detail: String,
}

struct Report {
    rows: Vec<Row>,
}

impl Report {
    fn new() -> Self {
        Self { rows: Vec::new() }
    }

    fn pass(&mut self, label: impl Into<String>, detail: impl Into<String>) {
        self.rows.push(Row {
            level: Level::Pass,
            label: label.into(),
            detail: detail.into(),
        });
    }
    fn warn(&mut self, label: impl Into<String>, detail: impl Into<String>) {
        self.rows.push(Row {
            level: Level::Warn,
            label: label.into(),
            detail: detail.into(),
        });
    }
    fn fail(&mut self, label: impl Into<String>, detail: impl Into<String>) {
        self.rows.push(Row {
            level: Level::Fail,
            label: label.into(),
            detail: detail.into(),
        });
    }
    fn info(&mut self, label: impl Into<String>, detail: impl Into<String>) {
        self.rows.push(Row {
            level: Level::Info,
            label: label.into(),
            detail: detail.into(),
        });
    }

    fn print(&self) {
        let width = self.rows.iter().map(|r| r.label.len()).max().unwrap_or(0);
        for r in &self.rows {
            let mark = match r.level {
                Level::Pass => "OK  ",
                Level::Warn => "WARN",
                Level::Fail => "FAIL",
                Level::Info => "    ",
            };
            println!("[{mark}] {:<width$}  {}", r.label, r.detail, width = width);
        }
        let (fails, warns) = self
            .rows
            .iter()
            .fold((0usize, 0usize), |(f, w), r| match r.level {
                Level::Fail => (f + 1, w),
                Level::Warn => (f, w + 1),
                _ => (f, w),
            });
        if fails > 0 {
            println!("\n{fails} check(s) failed, {warns} warning(s).");
        } else if warns > 0 {
            println!("\nAll critical checks passed. {warns} warning(s).");
        } else {
            println!("\nAll checks passed.");
        }
    }
}
