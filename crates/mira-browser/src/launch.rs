//! Finding and starting a Chromium-family browser with a DevTools port.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::{BrowserError, BrowserOptions};

/// How long to wait for the "DevTools listening on …" banner.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Locate a browser: explicit option, then `MIRA_BROWSER`, then the
/// usual install locations for Chrome, Chromium, Edge and Brave.
pub fn find_executable(explicit: Option<&Path>) -> Result<PathBuf, BrowserError> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p.to_owned())
        } else {
            Err(BrowserError::NotFound(format!(
                "configured browser `{}` does not exist",
                p.display()
            )))
        };
    }
    if let Some(p) = std::env::var_os("MIRA_BROWSER").map(PathBuf::from) {
        if p.is_file() {
            return Ok(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Chromium.app/Contents/MacOS/Chromium",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
        ] {
            candidates.push(Path::new("/Applications").join(app));
            if let Some(home) = std::env::var_os("HOME") {
                candidates.push(PathBuf::from(home).join("Applications").join(app));
            }
        }
    } else if cfg!(windows) {
        for base in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Some(b) = std::env::var_os(base).map(PathBuf::from) {
                candidates.push(b.join(r"Google\Chrome\Application\chrome.exe"));
                candidates.push(b.join(r"Microsoft\Edge\Application\msedge.exe"));
                candidates.push(b.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
            }
        }
    } else {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ] {
            if let Some(p) = mira_computer::backend::which(name) {
                candidates.push(p);
            }
        }
        // Playwright-managed Chromium (common on dev boxes and CI).
        if let Some(root) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH").map(PathBuf::from) {
            if let Ok(entries) = std::fs::read_dir(&root) {
                let mut dirs: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("chromium-"))
                    })
                    .collect();
                dirs.sort();
                for d in dirs.into_iter().rev() {
                    candidates.push(d.join("chrome-linux/chrome"));
                    candidates.push(d.join("chrome-linux64/chrome"));
                }
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        BrowserError::NotFound(
            "no Chrome/Chromium/Edge/Brave install found; install one or set \
             `browser.executable` in mira.yaml (or MIRA_BROWSER)"
                .into(),
        )
    })
}

/// Chrome refuses to start its sandbox as root (typical in containers).
fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        })
        .unwrap_or(false)
}

pub struct Launched {
    pub child: Child,
    pub ws_url: String,
}

pub async fn launch(opts: &BrowserOptions) -> Result<Launched, BrowserError> {
    let exe = find_executable(opts.executable.as_deref())?;
    std::fs::create_dir_all(&opts.profile_dir).map_err(|e| {
        BrowserError::Launch(format!(
            "create profile dir {}: {e}",
            opts.profile_dir.display()
        ))
    })?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", opts.profile_dir.display()))
        .arg(format!(
            "--window-size={},{}",
            opts.window_size.0, opts.window_size.1
        ))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-features=Translate,MediaRouter",
            "--disable-popup-blocking",
        ]);
    if opts.headless {
        cmd.arg("--headless=new");
    }
    if running_as_root() {
        cmd.arg("--no-sandbox");
    }
    cmd.arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| BrowserError::Launch(format!("spawn {}: {e}", exe.display())))?;
    let stderr = child.stderr.take().expect("piped stderr");
    let mut lines = BufReader::new(stderr).lines();

    let banner = async {
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.split("DevTools listening on ").nth(1) {
                return Ok(url.trim().to_owned());
            }
            tail.push(line);
            if tail.len() > 20 {
                tail.remove(0);
            }
        }
        Err(BrowserError::Launch(format!(
            "browser exited before opening DevTools. If another Mira browser is \
             already using {}, close it first. Output:\n{}",
            opts.profile_dir.display(),
            tail.join("\n")
        )))
    };
    let ws_url = tokio::time::timeout(STARTUP_TIMEOUT, banner)
        .await
        .map_err(|_| BrowserError::Launch("timed out waiting for DevTools".into()))??;

    // Keep draining stderr so a chatty browser can't block on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

    Ok(Launched { child, ws_url })
}

/// The DevTools endpoint of a browser already running on `profile`, from
/// the `DevToolsActivePort` file Chrome writes when started with
/// `--remote-debugging-port`.
///
/// This is how Mira gets its browser back after Mira itself exits without
/// closing it (a crash, a force quit, a dev restart): the Chrome it
/// launched keeps running and keeps the profile locked, and before this
/// every later launch failed on that lock until a reboot. Reconnecting
/// keeps its tabs and logins too.
pub fn active_devtools_url(profile: &Path) -> Option<String> {
    let text = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let mut lines = text.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    path.starts_with("/devtools/browser/")
        .then(|| format!("ws://127.0.0.1:{port}{path}"))
}

/// Clear what a browser left behind on `profile` so a fresh one can start.
///
/// Chrome's profile lock (`SingletonLock`) is a symlink to `host-pid`.
/// A lock whose process is gone is stale and is simply removed. A lock
/// whose process is alive but unreachable over DevTools is a Mira browser
/// that has stopped answering; the profile is Mira's own (never the
/// user's), so that process is asked to quit, then removed. Locks held by
/// another host (a shared home directory) are left alone.
pub async fn clear_stale_lock(profile: &Path) {
    let lock = profile.join("SingletonLock");
    let Ok(target) = std::fs::read_link(&lock) else {
        return;
    };
    let target = target.to_string_lossy().to_string();
    let Some((host, pid)) = target.rsplit_once('-') else {
        return;
    };
    let Ok(pid) = pid.parse::<u32>() else { return };
    if !is_this_host(host) {
        tracing::warn!(lock = %target, "browser profile is locked by another host; leaving it");
        return;
    }
    if process_alive(pid) {
        tracing::warn!(
            pid,
            "stopping an unresponsive Mira browser that holds the profile"
        );
        terminate(pid).await;
    }
    for f in [
        "SingletonLock",
        "SingletonSocket",
        "SingletonCookie",
        "DevToolsActivePort",
    ] {
        let _ = std::fs::remove_file(profile.join(f));
    }
}

fn is_this_host(host: &str) -> bool {
    let ours = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    ours.is_empty() || ours == host || ours.split('.').next() == host.split('.').next()
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
async fn terminate(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
    for _ in 0..30 {
        if !process_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
}

#[cfg(not(unix))]
async fn terminate(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_devtools_port_file_becomes_a_websocket_url() {
        let dir = std::env::temp_dir().join(format!("mira-devtools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("DevToolsActivePort"),
            "52341\n/devtools/browser/abc-123\n",
        )
        .unwrap();
        assert_eq!(
            active_devtools_url(&dir).as_deref(),
            Some("ws://127.0.0.1:52341/devtools/browser/abc-123")
        );
        std::fs::write(dir.join("DevToolsActivePort"), "garbage").unwrap();
        assert_eq!(active_devtools_url(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_lock_left_by_a_dead_process_is_cleared() {
        let dir = std::env::temp_dir().join(format!("mira-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let host = std::process::Command::new("hostname").output().unwrap();
        let host = String::from_utf8_lossy(&host.stdout).trim().to_string();
        // pid 1 is alive; use an absurd pid that cannot be.
        #[cfg(unix)]
        std::os::unix::fs::symlink(format!("{host}-999999"), dir.join("SingletonLock")).unwrap();
        std::fs::write(dir.join("DevToolsActivePort"), "1\n/devtools/browser/x\n").unwrap();
        clear_stale_lock(&dir).await;
        assert!(std::fs::symlink_metadata(dir.join("SingletonLock")).is_err());
        assert!(!dir.join("DevToolsActivePort").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
