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
