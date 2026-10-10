//! `mira doctor` — offline (and optionally online) diagnostics.
//!
//! Prints a checklist covering the pieces that most commonly trip
//! first-run setups: config parse, provider resolution, directory
//! permissions, skill discovery, MCP entries, memory files. With
//! `--ping`, also makes a live `list_models()` call to prove the
//! provider is actually reachable with the resolved key.
//!
//! `--bundle` also packs the results with version, config, logs and
//! (optionally) a session into a zip for a bug report; `--crashes` walks
//! through saved crash reports one at a time.

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
    /// Also write a zip for a bug report: version, platform, these checks,
    /// config and recent logs, with keys and tokens removed. Nothing is
    /// sent; you look it over and attach it yourself.
    #[arg(long)]
    bundle: bool,
    /// Include a chat in the bundle: a session id, or `last` for the most
    /// recent one. It holds your conversation, so it's left out by default.
    #[arg(long, value_name = "ID", requires = "bundle")]
    session: Option<String>,
    /// Where to write the bundle. Default: ~/Downloads (or this folder).
    #[arg(long, value_name = "PATH", requires = "bundle")]
    out: Option<PathBuf>,
    /// Review saved crash reports (see `diagnostics.crash_reports`) and
    /// choose, one by one, whether to open a GitHub issue with each.
    #[arg(long, conflicts_with_all = ["bundle", "ping"])]
    crashes: bool,
}

pub async fn run(cli: &crate::Cli, args: DoctorArgs) -> Result<()> {
    if args.crashes {
        return review_crashes();
    }
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
            // A broken config is exactly when a bundle helps most.
            if args.bundle {
                write_bundle(&args, &cwd, &MiraConfig::default(), &report)?;
            }
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

    // ---- plugins + mcp servers ----
    // Actually connect, so a broken server shows up here rather than as a
    // missing tool mid-session.
    let ext = mira_server::extensions::Extensions::new(Some(cwd.clone()));
    ext.reload().await;
    let plugins = ext.plugins().installed().unwrap_or_default();
    let on = plugins.iter().filter(|(p, _, _)| p.enabled).count();
    report.pass(
        "plugins",
        format!("{} installed, {on} enabled", plugins.len()),
    );
    ext.mcp()
        .wait_settled(ext.mcp().options().connect_timeout)
        .await;
    let servers = ext.mcp().servers();
    if servers.is_empty() {
        report.pass("mcp servers", "none configured".to_string());
    }
    for s in servers {
        let label = format!("mcp · {} ({})", s.name, s.scope.label());
        match &s.status {
            mira_mcp::Status::Connected => {
                report.pass(label, format!("connected · {} tools", s.tools.len()))
            }
            mira_mcp::Status::Failed { message } => report.fail(label, message.clone()),
            other => report.warn(label, other.label()),
        }
    }
    for notice in ext.notices() {
        report.warn("mcp config", notice);
    }
    ext.mcp().shutdown();

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

    // ---- command sandbox ----
    match mira_sandbox::detect_backend() {
        mira_sandbox::SandboxBackend::ProcessLevel => report.warn(
            "sandbox",
            "none: commands run unconfined (install bubblewrap, or use Linux 5.13+ for Landlock)",
        ),
        b => report.pass("sandbox", b.name()),
    }
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
    if args.bundle {
        write_bundle(&args, &cwd, &cfg, &report)?;
    }
    Ok(())
}

fn write_bundle(args: &DoctorArgs, cwd: &Path, cfg: &MiraConfig, report: &Report) -> Result<()> {
    let session_id = match args.session.as_deref() {
        Some("last") => Some(last_session().context("no saved sessions to include")?),
        Some(id) if mira_diagnostics::bundle::is_session_id(id) => Some(id.to_string()),
        Some(id) => anyhow::bail!("`{id}` isn't a session id"),
        None => None,
    };
    let mut opts = mira_diagnostics::BundleOptions::new(
        mira_diagnostics::Redactor::from_env_and_config(cfg),
        "cli",
    );
    opts.cwd = Some(cwd.to_path_buf());
    opts.session_id = session_id.clone();
    opts.extra.push(mira_diagnostics::BundleFile::new(
        "doctor.txt",
        report.render(),
    ));
    let bundle = mira_diagnostics::Bundle::collect(opts);
    if let Some(id) = &session_id {
        if !bundle.files.iter().any(|f| f.name.starts_with("session/")) {
            anyhow::bail!("no saved session `{id}`");
        }
    }

    let path = match &args.out {
        Some(p) if p.is_dir() => p.join(mira_diagnostics::Bundle::file_name()),
        Some(p) => p.clone(),
        None => default_bundle_dir(cwd).join(mira_diagnostics::Bundle::file_name()),
    };
    let zip = bundle.to_zip().context("build zip")?;
    mira_config::write_private(&path, &zip).with_context(|| format!("write {}", path.display()))?;

    println!("\nDiagnostics bundle (keys and tokens removed):");
    for f in &bundle.files {
        let size = mira_diagnostics::bundle::human(f.contents.len() as u64);
        match &f.note {
            Some(note) => println!("  {:<44} {size:>8}  ({note})", f.name),
            None => println!("  {:<44} {size:>8}", f.name),
        }
    }
    println!("\nWrote {}", path.display());
    println!("Nothing was sent. Look through it, then attach it to an issue:");
    println!("  {}", mira_diagnostics::ISSUES_URL);
    if session_id.is_none() {
        println!("(To include a chat, add `--session last` or `--session <id>`.)");
    }
    Ok(())
}

/// ~/Downloads when it exists, so the zip doesn't land inside a repo.
fn default_bundle_dir(cwd: &Path) -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Downloads"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| cwd.to_path_buf())
}

fn last_session() -> Option<String> {
    let dir = mira_diagnostics::mira_dir().join("sessions");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?.to_string();
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, id))
        })
        .max()
        .map(|(_, id)| id)
}

/// Show each unreviewed crash report in full and ask about it. Answering
/// marks it reviewed, so it isn't brought up again.
fn review_crashes() -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};

    let on = MiraConfig::load_global()
        .map(|c| c.diagnostics.crash_reports())
        .unwrap_or(false);
    let pending = mira_diagnostics::crash::pending();
    if pending.is_empty() {
        println!("No new crash reports.");
        if !on {
            println!("Crash reports are off. Turn them on with:");
            println!("  mira config set diagnostics.crash_reports true");
        }
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        for r in &pending {
            println!("{}", r.path.display());
        }
        return Ok(());
    }
    let stdin = std::io::stdin();
    for (i, r) in pending.iter().enumerate() {
        println!(
            "── crash report {} of {} · {}\n",
            i + 1,
            pending.len(),
            r.path.display()
        );
        println!("{}", r.contents.trim_end());
        print!("\nThis is everything that would go in the issue. Open a GitHub issue with it? [y/N/q] ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        stdin.lock().read_line(&mut answer)?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => {
                crate::tui::ext_slash::open_browser(&r.issue_url());
                println!("Opened in your browser. Nothing is sent until you submit it there.");
            }
            "q" | "quit" => return Ok(()),
            _ => {}
        }
        r.mark_reviewed()
            .with_context(|| format!("mark {} reviewed", r.path.display()))?;
        println!();
    }
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
        print!("{}", self.render());
    }

    fn render(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let width = self.rows.iter().map(|r| r.label.len()).max().unwrap_or(0);
        for r in &self.rows {
            let mark = match r.level {
                Level::Pass => "OK  ",
                Level::Warn => "WARN",
                Level::Fail => "FAIL",
                Level::Info => "    ",
            };
            let _ = writeln!(
                out,
                "[{mark}] {:<width$}  {}",
                r.label,
                r.detail,
                width = width
            );
        }
        let (fails, warns) = self
            .rows
            .iter()
            .fold((0usize, 0usize), |(f, w), r| match r.level {
                Level::Fail => (f + 1, w),
                Level::Warn => (f, w + 1),
                _ => (f, w),
            });
        let _ = if fails > 0 {
            writeln!(out, "\n{fails} check(s) failed, {warns} warning(s).")
        } else if warns > 0 {
            writeln!(out, "\nAll critical checks passed. {warns} warning(s).")
        } else {
            writeln!(out, "\nAll checks passed.")
        };
        out
    }
}
