//! `mira service` — keep `mira serve` running in the background.
//!
//! Registers the web server with the OS service manager so it starts at
//! login and restarts if it crashes: a launchd agent on macOS, a systemd
//! user unit on Linux. The browser UI (and anything reaching this machine
//! over a tunnel or LAN) then works without a terminal or the desktop app
//! open.
//!
//! The service runs this same binary (at a path that survives upgrades)
//! with `serve --port … --host …`. Service managers start processes with a
//! minimal environment, so the installing shell's `PATH` is copied in —
//! external agents (`claude`, `codex`, …) and `git` resolve as they do in
//! your terminal. Other environment variables, API keys included, are not
//! copied: keep keys in `mira.yaml` (`mira init` / `mira login`), which
//! `serve` reads itself.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Args, Debug, Clone)]
pub struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Subcommand, Debug, Clone)]
enum ServiceCommand {
    /// Install and start the background service (replaces an existing one).
    Install(InstallArgs),
    /// Stop the service and remove it.
    Uninstall,
    /// Show whether the service is installed, running, and answering.
    Status,
    /// Restart the service (e.g. after changing settings by hand).
    Restart,
}

#[derive(Args, Debug, Clone)]
struct InstallArgs {
    /// TCP port for the web UI.
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Host to bind. Defaults to loopback; `0.0.0.0` exposes the UI to
    /// your network, so only use it on networks you trust.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

/// What `install` set up, so `status` knows where to probe.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct ServiceConfig {
    port: u16,
    host: String,
    exe: PathBuf,
}

const LAUNCHD_LABEL: &str = "dev.runmira.mira";

// System tools by absolute path, never via `PATH`, so a shadowing binary
// earlier on `PATH` can't stand in for them.
const LAUNCHCTL: &str = "/bin/launchctl";

fn systemctl_bin() -> &'static str {
    ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or("/usr/bin/systemctl")
}
const SYSTEMD_UNIT: &str = "mira.service";

pub async fn run(args: ServiceArgs) -> Result<()> {
    match args.command {
        ServiceCommand::Install(a) => install(a).await,
        ServiceCommand::Uninstall => uninstall(),
        ServiceCommand::Status => status().await,
        ServiceCommand::Restart => {
            if !restart()? {
                bail!("the service isn't installed — run `mira service install`");
            }
            println!("restarted");
            Ok(())
        }
    }
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn config_path() -> Result<PathBuf> {
    Ok(home()?.join(".mira").join("service.json"))
}

fn log_path() -> Result<PathBuf> {
    Ok(home()?.join(".mira").join("logs").join("service.log"))
}

fn load_config() -> Option<ServiceConfig> {
    let text = std::fs::read_to_string(config_path().ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// The path to run. Homebrew's resolved path is versioned
/// (`…/Cellar/mira/0.5.2/bin/mira`) and disappears on `brew upgrade`, so
/// use the prefix's stable `bin/mira` link instead.
pub(crate) fn stable_exe_path(exe: &Path) -> PathBuf {
    let s = exe.to_string_lossy();
    if let Some(i) = s.find("/Cellar/mira/") {
        return PathBuf::from(format!("{}/bin/mira", &s[..i]));
    }
    exe.to_path_buf()
}

fn current_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locate the mira binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    Ok(stable_exe_path(&exe))
}

async fn install(args: InstallArgs) -> Result<()> {
    let exe = current_exe()?;
    let home = home()?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    let cfg = ServiceConfig {
        port: args.port,
        host: args.host.clone(),
        exe: exe.clone(),
    };

    if cfg!(target_os = "macos") {
        let logs = log_path()?;
        std::fs::create_dir_all(logs.parent().unwrap()).context("create ~/.mira/logs")?;
        let plist = launchd_plist(&cfg, &home, &path_env, &logs);
        let target = launchd_plist_path(&home);
        std::fs::create_dir_all(target.parent().unwrap())
            .context("create ~/Library/LaunchAgents")?;
        // Replace a previous install: unload it first, or bootstrap fails.
        launchd_stop()?;
        std::fs::write(&target, plist).with_context(|| format!("write {}", target.display()))?;
        launchctl(&["bootstrap", &launchd_domain()?, &target.to_string_lossy()])
            .context("launchctl bootstrap")?;
    } else if cfg!(target_os = "linux") {
        ensure_systemd_user()?;
        let unit = systemd_unit(&cfg, &path_env);
        let target = systemd_unit_path(&home);
        std::fs::create_dir_all(target.parent().unwrap())
            .context("create ~/.config/systemd/user")?;
        std::fs::write(&target, unit).with_context(|| format!("write {}", target.display()))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", SYSTEMD_UNIT])?;
        // `restart`, not `start`: picks up a changed unit on reinstall.
        systemctl(&["restart", SYSTEMD_UNIT])?;
    } else {
        bail!("`mira service` supports macOS (launchd) and Linux (systemd)");
    }

    let cfg_path = config_path()?;
    std::fs::create_dir_all(cfg_path.parent().unwrap())?;
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg)?)?;

    println!(
        "installed: {} serve --port {} --host {}",
        exe.display(),
        cfg.port,
        cfg.host
    );
    println!("starts at login and restarts if it exits");
    match wait_until_up(&cfg).await {
        true => println!("running at {}", url(&cfg)),
        false => println!("not answering yet — check `mira service status`"),
    }
    if cfg.host != "127.0.0.1" && cfg.host != "localhost" {
        println!("note: bound to {} — reachable from your network", cfg.host);
    }
    if cfg!(target_os = "linux") {
        println!(
            "note: to keep it running while you're logged out: loginctl enable-linger {}",
            std::env::var("USER").unwrap_or_else(|_| "$USER".into())
        );
    }
    println!("note: only PATH was copied from this shell; keep API keys in mira.yaml");
    Ok(())
}

fn uninstall() -> Result<()> {
    let home = home()?;
    let mut removed = false;
    if cfg!(target_os = "macos") {
        let target = launchd_plist_path(&home);
        // A stop that fails leaves the files, so it isn't reported as gone.
        launchd_stop()?;
        if target.exists() {
            std::fs::remove_file(&target)?;
            removed = true;
        }
    } else if cfg!(target_os = "linux") {
        let target = systemd_unit_path(&home);
        if target.exists() {
            systemctl(&["disable", "--now", SYSTEMD_UNIT]).context("stop the service")?;
            std::fs::remove_file(&target)?;
            let _ = systemctl(&["daemon-reload"]);
            removed = true;
        }
    } else {
        bail!("`mira service` supports macOS (launchd) and Linux (systemd)");
    }
    if let Ok(p) = config_path() {
        let _ = std::fs::remove_file(p);
    }
    println!(
        "{}",
        if removed {
            "uninstalled"
        } else {
            "not installed"
        }
    );
    Ok(())
}

async fn status() -> Result<()> {
    let home = home()?;
    let installed = if cfg!(target_os = "macos") {
        launchd_plist_path(&home).exists()
    } else {
        systemd_unit_path(&home).exists()
    };
    if !installed {
        println!("not installed — run `mira service install`");
        return Ok(());
    }
    let state = if cfg!(target_os = "macos") {
        launchctl_output(&["print", &launchd_service()?])
            .ok()
            .and_then(|out| launchd_state(&out))
            .unwrap_or_else(|| "not loaded".into())
    } else {
        systemctl_output(&["is-active", SYSTEMD_UNIT])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".into())
    };
    println!("installed, {state}");
    if let Some(cfg) = load_config() {
        println!("binary: {}", cfg.exe.display());
        let up = probe(&cfg).await;
        println!(
            "{}: {}",
            url(&cfg),
            if up { "answering" } else { "not answering" }
        );
    }
    if cfg!(target_os = "macos") {
        println!("logs: {}", log_path()?.display());
    } else {
        println!("logs: journalctl --user -u mira");
    }
    Ok(())
}

/// Restart the service if it's installed. `Ok(false)` when it isn't; used
/// by `mira update` to move a running service onto the new binary.
pub fn restart() -> Result<bool> {
    let home = home()?;
    if cfg!(target_os = "macos") {
        if !launchd_plist_path(&home).exists() {
            return Ok(false);
        }
        launchctl(&["kickstart", "-k", &launchd_service()?])?;
        Ok(true)
    } else if cfg!(target_os = "linux") {
        if !systemd_unit_path(&home).exists() {
            return Ok(false);
        }
        systemctl(&["restart", SYSTEMD_UNIT])?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn url(cfg: &ServiceConfig) -> String {
    let host = match cfg.host.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        h => h,
    };
    // IPv6 literals need brackets in a URL.
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("http://{host}:{}", cfg.port)
}

async fn probe(cfg: &ServiceConfig) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build();
    match client {
        Ok(c) => c
            .get(format!("{}/api/version", url(cfg)))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false),
        Err(_) => false,
    }
}

async fn wait_until_up(cfg: &ServiceConfig) -> bool {
    for _ in 0..30 {
        if probe(cfg).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}

// ---- launchd ------------------------------------------------------------

fn launchd_plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"))
}

fn uid() -> Result<String> {
    let out = Proc::new("/usr/bin/id")
        .arg("-u")
        .output()
        .context("run `id -u`")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn launchd_domain() -> Result<String> {
    Ok(format!("gui/{}", uid()?))
}

fn launchd_service() -> Result<String> {
    Ok(format!("{}/{LAUNCHD_LABEL}", launchd_domain()?))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn launchd_plist(cfg: &ServiceConfig, home: &Path, path_env: &str, log: &Path) -> String {
    let args = [
        cfg.exe.to_string_lossy().into_owned(),
        "serve".into(),
        "--port".into(),
        cfg.port.to_string(),
        "--host".into(),
        cfg.host.clone(),
    ];
    let args = args
        .iter()
        .map(|a| format!("    <string>{}</string>", xml_escape(a)))
        .collect::<Vec<_>>()
        .join("\n");
    let log = xml_escape(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}
  </array>
  <key>WorkingDirectory</key>
  <string>{home}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        home = xml_escape(&home.to_string_lossy()),
        path = xml_escape(path_env),
    )
}

/// `state = running` / `pid = 123` out of `launchctl print`.
fn launchd_state(print: &str) -> Option<String> {
    let field = |name: &str| {
        print.lines().find_map(|l| {
            let l = l.trim();
            l.strip_prefix(name)
                .and_then(|r| r.trim_start().strip_prefix('='))
                .map(|v| v.trim().to_string())
        })
    };
    let state = field("state")?;
    Some(match field("pid") {
        Some(pid) => format!("{state} (pid {pid})"),
        None => state,
    })
}

/// Unload the agent if it's loaded. Not being loaded is fine; failing to
/// unload one that is, is an error.
fn launchd_stop() -> Result<()> {
    let service = launchd_service()?;
    if launchctl_output(&["print", &service]).is_ok() {
        launchctl(&["bootout", &service]).context("stop the service")?;
    }
    Ok(())
}

fn launchctl(args: &[&str]) -> Result<()> {
    let out = Proc::new(LAUNCHCTL)
        .args(args)
        .output()
        .context("run launchctl")?;
    if !out.status.success() {
        bail!(
            "launchctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn launchctl_output(args: &[&str]) -> Result<String> {
    let out = Proc::new(LAUNCHCTL)
        .args(args)
        .output()
        .context("run launchctl")?;
    if !out.status.success() {
        bail!("launchctl {}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---- systemd ------------------------------------------------------------

fn systemd_unit_path(home: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    base.join("systemd/user").join(SYSTEMD_UNIT)
}

/// Quote one word for a unit file: `"` and `\` escaped, `%` doubled
/// (systemd specifiers).
fn systemd_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

fn systemd_unit(cfg: &ServiceConfig, path_env: &str) -> String {
    let exec = [
        systemd_quote(&cfg.exe.to_string_lossy()),
        "serve".into(),
        "--port".into(),
        cfg.port.to_string(),
        "--host".into(),
        systemd_quote(&cfg.host),
    ]
    .join(" ");
    format!(
        "[Unit]\n\
         Description=Mira web server\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         WorkingDirectory=%h\n\
         Environment={path}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        path = systemd_quote(&format!("PATH={path_env}")),
    )
}

fn ensure_systemd_user() -> Result<()> {
    match Proc::new(systemctl_bin())
        .args(["--user", "is-system-running"])
        .output()
    {
        // Any answer ("running", "degraded", …) means a user manager exists.
        Ok(out) if !out.stdout.is_empty() => Ok(()),
        _ => bail!(
            "no systemd user session found (`systemctl --user` failed). \
             `mira service` needs systemd; run `mira serve` under your own supervisor instead"
        ),
    }
}

fn systemctl(args: &[&str]) -> Result<()> {
    let out = Proc::new(systemctl_bin())
        .arg("--user")
        .args(args)
        .output()
        .context("run systemctl")?;
    if !out.status.success() {
        bail!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn systemctl_output(args: &[&str]) -> Result<String> {
    let out = Proc::new(systemctl_bin())
        .arg("--user")
        .args(args)
        .output()
        .context("run systemctl")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(exe: &str) -> ServiceConfig {
        ServiceConfig {
            port: 8787,
            host: "127.0.0.1".into(),
            exe: PathBuf::from(exe),
        }
    }

    #[test]
    fn urls_bracket_ipv6_hosts() {
        let mut c = cfg("/bin/mira");
        assert_eq!(url(&c), "http://127.0.0.1:8787");
        c.host = "::1".into();
        assert_eq!(url(&c), "http://[::1]:8787");
        c.host = "::".into();
        assert_eq!(url(&c), "http://[::1]:8787");
        c.host = "0.0.0.0".into();
        assert_eq!(url(&c), "http://127.0.0.1:8787");
    }

    #[test]
    fn homebrew_paths_point_at_the_stable_link() {
        assert_eq!(
            stable_exe_path(Path::new("/opt/homebrew/Cellar/mira/0.5.2/bin/mira")),
            PathBuf::from("/opt/homebrew/bin/mira")
        );
        assert_eq!(
            stable_exe_path(Path::new("/usr/local/Cellar/mira/0.5.2/bin/mira")),
            PathBuf::from("/usr/local/bin/mira")
        );
        assert_eq!(
            stable_exe_path(Path::new("/home/me/.local/bin/mira")),
            PathBuf::from("/home/me/.local/bin/mira")
        );
    }

    #[test]
    fn plist_runs_serve_and_escapes_values() {
        let p = launchd_plist(
            &cfg("/Users/a&b/bin/mira"),
            Path::new("/Users/a&b"),
            "/usr/bin:/opt/<x>",
            Path::new("/Users/a&b/.mira/logs/service.log"),
        );
        assert!(p.contains("<string>/Users/a&amp;b/bin/mira</string>"));
        assert!(p.contains(
            "<string>serve</string>\n    <string>--port</string>\n    <string>8787</string>"
        ));
        assert!(p.contains("<string>/usr/bin:/opt/&lt;x&gt;</string>"));
        assert!(p.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(!p.contains("a&b"));
    }

    /// launchd silently ignores a malformed plist, so have macOS parse it.
    #[cfg(target_os = "macos")]
    #[test]
    fn plist_is_valid_for_launchd() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.plist");
        let p = launchd_plist(
            &cfg("/opt/homebrew/bin/mira"),
            Path::new("/Users/me"),
            "/opt/homebrew/bin:/usr/bin:/bin",
            Path::new("/Users/me/.mira/logs/service.log"),
        );
        std::fs::write(&file, p).unwrap();
        let out = Proc::new("plutil")
            .arg("-lint")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    #[test]
    fn unit_quotes_exec_and_escapes_specifiers() {
        let u = systemd_unit(&cfg("/home/me/my bin/mira"), "/usr/bin:/odd%dir");
        assert!(u.contains(
            "ExecStart=\"/home/me/my bin/mira\" serve --port 8787 --host \"127.0.0.1\"\n"
        ));
        assert!(u.contains("Environment=\"PATH=/usr/bin:/odd%%dir\"\n"));
        assert!(u.contains("Restart=on-failure"));
        assert!(u.contains("WantedBy=default.target"));
    }

    #[test]
    fn reads_state_and_pid_from_launchctl_print() {
        let out = "dev.runmira.mira = {\n\tactive count = 1\n\tstate = running\n\tpid = 4242\n}";
        assert_eq!(launchd_state(out).as_deref(), Some("running (pid 4242)"));
        assert_eq!(
            launchd_state("x = {\n\tstate = not running\n}").as_deref(),
            Some("not running")
        );
    }
}
