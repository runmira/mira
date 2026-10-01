//! Mira's own Chromium: Chrome for Testing, downloaded on first use.
//!
//! Driving whatever browser happens to be installed made the browser tool
//! as reliable as the user's machine: no Chrome at all, a Chrome too old or
//! too new for the protocol calls Mira makes, an enterprise policy that
//! blocks remote debugging, an Edge or Brave that behaves differently. A
//! browser Mira manages itself behaves the same everywhere.
//!
//! Chrome for Testing is Google's build for automation: a Stable Chromium
//! with no auto-update and no first-run prompts, published per platform
//! with a JSON manifest. It lives in `~/.mira/browser/chromium/<version>/`
//! next to Mira's browser profile. The version in use is recorded in
//! `current`; a newer Stable is fetched at most weekly, in the background,
//! and takes effect on the next launch, after which older versions are
//! removed.
//!
//! An installed browser is still used when this can't work — no build for
//! the platform (Linux on ARM), or the download fails — so the browser tool
//! never gets *less* available than it was.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::BrowserError;

const MANIFEST: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
/// How often to look for a newer Stable.
const UPDATE_EVERY: Duration = Duration::from_secs(7 * 24 * 3600);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// One install at a time per process; the final rename keeps a second
/// process from ever seeing a half-extracted browser.
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Chrome for Testing's name for this platform, when it publishes one.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("linux", "x86_64") => Some("linux64"),
        ("windows", "x86_64") => Some("win64"),
        ("windows", "x86") => Some("win32"),
        _ => None,
    }
}

/// The browser binary inside an extracted download.
fn exe_in(version_dir: &Path, platform: &str) -> PathBuf {
    let top = version_dir.join(format!("chrome-{platform}"));
    match platform {
        p if p.starts_with("mac") => top
            .join("Google Chrome for Testing.app")
            .join("Contents/MacOS/Google Chrome for Testing"),
        p if p.starts_with("win") => top.join("chrome.exe"),
        _ => top.join("chrome"),
    }
}

/// `~/.mira/browser/chromium`.
pub fn root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".mira")
            .join("browser")
            .join("chromium"),
    )
}

/// The managed browser, if one is installed: its version and binary.
pub fn installed() -> Option<(String, PathBuf)> {
    installed_in(&root()?, platform()?)
}

fn installed_in(root: &Path, platform: &str) -> Option<(String, PathBuf)> {
    let version = std::fs::read_to_string(root.join("current")).ok()?;
    let version = version.trim().to_string();
    let exe = exe_in(&root.join(&version), platform);
    exe.is_file().then_some((version, exe))
}

#[derive(Deserialize)]
struct Manifest {
    channels: std::collections::HashMap<String, Channel>,
}

#[derive(Deserialize)]
struct Channel {
    version: String,
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    #[serde(default)]
    chrome: Vec<Download>,
}

#[derive(Deserialize)]
struct Download {
    platform: String,
    url: String,
}

/// The current Stable for `platform`: its version and download URL.
fn stable_for(manifest: &str, platform: &str) -> Result<(String, String), BrowserError> {
    let m: Manifest = serde_json::from_str(manifest)
        .map_err(|e| BrowserError::Launch(format!("Chrome for Testing manifest: {e}")))?;
    let stable = m
        .channels
        .get("Stable")
        .ok_or_else(|| BrowserError::Launch("Chrome for Testing manifest has no Stable".into()))?;
    let url = stable
        .downloads
        .chrome
        .iter()
        .find(|d| d.platform == platform)
        .map(|d| d.url.clone())
        .ok_or_else(|| {
            BrowserError::Launch(format!("no Chrome for Testing build for {platform}"))
        })?;
    Ok((stable.version.clone(), url))
}

fn http() -> Result<reqwest::Client, BrowserError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| BrowserError::Launch(format!("http client: {e}")))
}

/// The managed browser, installing it first if needed. Errors leave the
/// caller free to fall back to an installed browser.
pub async fn ensure() -> Result<PathBuf, BrowserError> {
    let (Some(root), Some(platform)) = (root(), platform()) else {
        return Err(BrowserError::NotFound(format!(
            "Chrome for Testing has no build for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
    };
    if let Some((_, exe)) = installed_in(&root, platform) {
        spawn_update_check(root, platform);
        return Ok(exe);
    }
    install(&root, platform).await.map(|(_, exe)| exe)
}

/// Fetch the current Stable, unless it's the one already in use. Returns
/// its version and binary.
async fn install(root: &Path, platform: &str) -> Result<(String, PathBuf), BrowserError> {
    let _one_at_a_time = INSTALL.lock().await;
    std::fs::create_dir_all(root)
        .map_err(|e| BrowserError::Launch(format!("create browser dir: {e}")))?;
    let client = http()?;
    let manifest = client
        .get(MANIFEST)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| BrowserError::Launch(format!("fetch Chrome for Testing manifest: {e}")))?
        .text()
        .await
        .map_err(|e| BrowserError::Launch(format!("read manifest: {e}")))?;
    let (version, url) = stable_for(&manifest, platform)?;
    let _ = std::fs::write(root.join("checked"), "");

    let dest = root.join(&version);
    let exe = exe_in(&dest, platform);
    if !exe.is_file() {
        download_and_extract(&client, &url, root, &dest).await?;
        if !exe.is_file() {
            return Err(BrowserError::Launch(format!(
                "downloaded Chrome for Testing {version} but {} is missing",
                exe.display()
            )));
        }
    }
    std::fs::write(root.join("current"), &version)
        .map_err(|e| BrowserError::Launch(format!("record browser version: {e}")))?;
    tracing::info!(%version, path = %exe.display(), "managed browser ready");
    Ok((version, exe))
}

async fn download_and_extract(
    client: &reqwest::Client,
    url: &str,
    root: &Path,
    dest: &Path,
) -> Result<(), BrowserError> {
    let io = |what: &str, e: std::io::Error| BrowserError::Launch(format!("{what}: {e}"));
    std::fs::create_dir_all(root).map_err(|e| io("create browser dir", e))?;
    let zip_path = root.join(format!("download-{}.zip", std::process::id()));
    let staging = root.join(format!("staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);

    let resp = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| BrowserError::Launch(format!("download browser: {e}")))?;
    let total = resp.content_length();
    tracing::info!(
        url,
        mb = total.map(|t| t / 1_000_000),
        "downloading Mira's browser (Chrome for Testing)"
    );
    let mut file = tokio::fs::File::create(&zip_path)
        .await
        .map_err(|e| io("create download file", e))?;
    let mut stream = resp.bytes_stream();
    let (mut got, mut logged) = (0u64, 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| BrowserError::Launch(format!("download browser: {e}")))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| io("write download", e))?;
        got += chunk.len() as u64;
        if let Some(t) = total.filter(|t| *t > 0) {
            let pct = got * 100 / t;
            if pct >= logged + 20 {
                logged = pct;
                tracing::info!(pct, "downloading Mira's browser");
            }
        }
    }
    file.flush().await.map_err(|e| io("write download", e))?;
    drop(file);

    let (zip_bg, staging_bg) = (zip_path.clone(), staging.clone());
    let extracted = tokio::task::spawn_blocking(move || extract(&zip_bg, &staging_bg))
        .await
        .map_err(|e| BrowserError::Launch(format!("extract browser: {e}")))?;
    let _ = std::fs::remove_file(&zip_path);
    if let Err(e) = extracted {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // Atomic from any reader's point of view: the version directory either
    // isn't there yet or is complete.
    let _ = std::fs::remove_dir_all(dest);
    std::fs::rename(&staging, dest).map_err(|e| io("install browser", e))
}

/// Unpack `zip` into `into`. macOS uses `ditto`, which keeps the app
/// bundle's symlinks, permissions and extended attributes exactly — the
/// bundle doesn't launch without them.
fn extract(zip: &Path, into: &Path) -> Result<(), BrowserError> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(zip)
            .arg(into)
            .output()
            .map_err(|e| BrowserError::Launch(format!("run ditto: {e}")))?;
        if !out.status.success() {
            return Err(BrowserError::Launch(format!(
                "unpack browser: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        return Ok(());
    }
    let file = std::fs::File::open(zip)
        .map_err(|e| BrowserError::Launch(format!("open download: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| BrowserError::Launch(format!("read download: {e}")))?;
    archive
        .extract(into)
        .map_err(|e| BrowserError::Launch(format!("unpack browser: {e}")))
}

/// At most weekly, fetch a newer Stable in the background. It's used from
/// the next launch; the running browser is left alone.
fn spawn_update_check(root: PathBuf, platform: &'static str) {
    let due = std::fs::metadata(root.join("checked"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_none_or(|age| age >= UPDATE_EVERY);
    if !due {
        return;
    }
    tokio::spawn(async move {
        match install(&root, platform).await {
            Ok((version, _)) => prune(&root, &version),
            Err(e) => tracing::debug!(%e, "managed browser update check failed"),
        }
    });
}

/// Remove every version but `keep`, and leftovers of interrupted installs.
fn prune(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let is_version = name.chars().next().is_some_and(|c| c.is_ascii_digit());
        let leftover = name.starts_with("staging-") || name.starts_with("download-");
        if (is_version && name != keep) || leftover {
            let p = e.path();
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "timestamp": "2026-09-30T10:00:00.000Z",
        "channels": {
            "Stable": {
                "channel": "Stable", "version": "141.0.7390.54", "revision": "1",
                "downloads": {
                    "chrome": [
                        {"platform": "linux64", "url": "https://x/linux64/chrome-linux64.zip"},
                        {"platform": "mac-arm64", "url": "https://x/mac-arm64/chrome-mac-arm64.zip"}
                    ],
                    "chromedriver": []
                }
            },
            "Beta": {"channel": "Beta", "version": "142.0.0.0", "revision": "2", "downloads": {"chrome": []}}
        }
    }"#;

    #[test]
    fn the_stable_build_for_the_platform_is_picked() {
        let (v, url) = stable_for(SAMPLE, "mac-arm64").unwrap();
        assert_eq!(v, "141.0.7390.54");
        assert!(url.ends_with("chrome-mac-arm64.zip"));
        assert!(
            stable_for(SAMPLE, "win64").is_err(),
            "no build listed, no guess"
        );
    }

    #[test]
    fn binaries_are_found_where_each_platform_puts_them() {
        let d = Path::new("/v");
        assert_eq!(
            exe_in(d, "mac-arm64"),
            Path::new("/v/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        );
        assert_eq!(exe_in(d, "linux64"), Path::new("/v/chrome-linux64/chrome"));
        assert_eq!(exe_in(d, "win64"), Path::new("/v/chrome-win64/chrome.exe"));
    }

    #[test]
    fn only_a_complete_recorded_install_counts() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        assert!(installed_in(r, "linux64").is_none());
        std::fs::write(r.join("current"), "141.0.1\n").unwrap();
        assert!(
            installed_in(r, "linux64").is_none(),
            "recorded but not extracted"
        );
        let exe = exe_in(&r.join("141.0.1"), "linux64");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert_eq!(
            installed_in(r, "linux64").map(|(v, _)| v).as_deref(),
            Some("141.0.1")
        );
    }

    #[test]
    fn pruning_keeps_only_the_current_version() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        for d in ["140.0.1", "141.0.1", "staging-9"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        std::fs::write(r.join("download-9.zip"), "").unwrap();
        std::fs::write(r.join("current"), "141.0.1").unwrap();
        prune(r, "141.0.1");
        let mut left: Vec<_> = std::fs::read_dir(r)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["141.0.1", "current"]);
    }

    /// Downloads ~150 MB; run by hand: `cargo test -p mira-browser -- --ignored managed`.
    #[tokio::test]
    #[ignore]
    async fn managed_installs_and_launches() {
        let exe = ensure().await.expect("install");
        let out = std::process::Command::new(&exe)
            .arg("--version")
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("Chrome"));
    }
}
