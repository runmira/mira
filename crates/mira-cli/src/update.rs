//! `mira update` — replace this binary with the latest release.
//!
//! Installs from `install.sh` (and anything else that's a plain binary on
//! disk) update in place: download the release tarball for this platform,
//! check it against the release's published SHA-256, make sure the new
//! binary runs, then swap it in atomically. Homebrew and cargo installs are
//! left to their own tools so they don't drift from what those track. A
//! running background service (`mira service`) is restarted onto the new
//! version.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;

use anyhow::{bail, Context, Result};
use clap::Args;
use sha2::{Digest, Sha256};

const REPO: &str = "runmira/mira";

#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Only report whether an update is available.
    #[arg(long)]
    check: bool,
    /// Install this release tag (e.g. `v0.5.1`) instead of the latest.
    #[arg(long, value_name = "TAG")]
    version: Option<String>,
    /// Reinstall even if already on that version.
    #[arg(long)]
    force: bool,
}

/// How this binary was installed, from where it lives.
#[derive(Debug, PartialEq)]
enum InstallKind {
    Homebrew,
    Cargo,
    Standalone,
}

fn install_kind(exe: &Path) -> InstallKind {
    let s = exe.to_string_lossy();
    if s.contains("/Cellar/") {
        InstallKind::Homebrew
    } else if s.contains("/.cargo/bin/")
        || s.contains("/target/debug/")
        || s.contains("/target/release/")
    {
        InstallKind::Cargo
    } else {
        InstallKind::Standalone
    }
}

/// `v0.5.2` / `0.5.2` → `[0, 5, 2]`; pre-release suffixes are ignored.
fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

/// The release asset for this machine, as `release.yml` names them.
fn asset_name() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => bail!("no Mira release builds for {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => bail!("no Mira release builds for {other}"),
    };
    Ok(format!("mira-{os}-{arch}.tar.gz"))
}

/// First field of a `shasum -a 256` line.
fn parse_sha256(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?.to_ascii_lowercase();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .context("build HTTP client")
}

async fn latest_tag(http: &reqwest::Client) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = http
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("look up the latest release")?
        .error_for_status()
        .context("look up the latest release")?
        .json()
        .await
        .context("read the latest release")?;
    Ok(release.tag_name)
}

async fn download(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let bytes = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url}"))?
        .bytes()
        .await
        .with_context(|| format!("download {url}"))?;
    Ok(bytes.to_vec())
}

pub async fn run(args: UpdateArgs) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().context("locate the mira binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);

    let http = client()?;
    let tag = match &args.version {
        Some(t) if t.starts_with('v') => t.clone(),
        Some(t) => format!("v{t}"),
        None => latest_tag(&http).await?,
    };
    let newer = is_newer(&tag, current);
    let same = parse_version(&tag) == parse_version(current);

    if args.check {
        if newer {
            println!(
                "update available: {current} → {}",
                tag.trim_start_matches('v')
            );
        } else {
            println!("up to date ({current})");
        }
        return Ok(());
    }
    if args.version.is_none() && !newer && !args.force {
        println!("up to date ({current})");
        return Ok(());
    }
    if same && !args.force {
        println!("already on {current} (use --force to reinstall)");
        return Ok(());
    }

    match install_kind(&exe) {
        InstallKind::Homebrew => {
            println!("installed with Homebrew — running `brew upgrade runmira/tap/mira`");
            let ok = Proc::new("brew")
                .args(["upgrade", "runmira/tap/mira"])
                .status()
                .context("run brew")?
                .success();
            if !ok {
                bail!("brew upgrade failed");
            }
            restart_service();
            return Ok(());
        }
        InstallKind::Cargo => bail!(
            "this mira was built with cargo ({}); update the checkout and rebuild, \
             or install a release with `curl -fsSL https://runmira.dev/install.sh | bash`",
            exe.display()
        ),
        InstallKind::Standalone => {}
    }

    let asset = asset_name()?;
    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    println!("downloading {asset} ({tag})");
    let tarball = download(&http, &format!("{base}/{asset}")).await?;
    let sums = download(&http, &format!("{base}/{asset}.sha256")).await?;

    let expected = parse_sha256(&String::from_utf8_lossy(&sums))
        .context("the release's .sha256 file is malformed")?;
    let actual = hex(&Sha256::digest(&tarball));
    if actual != expected {
        bail!("checksum mismatch for {asset}: expected {expected}, got {actual} — not installing");
    }

    let tmp = tempfile::tempdir().context("create a temp dir")?;
    let archive = tmp.path().join(&asset);
    std::fs::write(&archive, &tarball)?;
    let ok = Proc::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(tmp.path())
        .status()
        .context("run tar")?
        .success();
    let new_bin = tmp.path().join("mira");
    if !ok || !new_bin.is_file() {
        bail!("{asset} didn't contain a mira binary");
    }

    // Never swap in something that can't start.
    let version_out = Proc::new(&new_bin)
        .arg("--version")
        .output()
        .context("run the downloaded binary")?;
    if !version_out.status.success() {
        bail!("the downloaded binary doesn't run on this machine — not installing");
    }

    replace_binary(&new_bin, &exe)?;
    println!(
        "updated {current} → {} ({})",
        tag.trim_start_matches('v'),
        exe.display()
    );
    restart_service();
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Copy next to the target, then rename over it: atomic on one filesystem,
/// and safe while the old binary is running (it keeps its open inode).
fn replace_binary(new_bin: &Path, target: &Path) -> Result<()> {
    let dir = target.parent().context("binary has no parent directory")?;
    let staged: PathBuf = dir.join(format!(".mira-update-{}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        std::fs::copy(new_bin, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&staged, target)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&staged);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            bail!(
                "no permission to write {} — re-run with `sudo mira update`",
                dir.display()
            );
        }
        return Err(e).with_context(|| format!("replace {}", target.display()));
    }
    Ok(())
}

fn restart_service() {
    match crate::service::restart() {
        Ok(true) => println!("restarted the background service"),
        Ok(false) => {}
        Err(e) => println!("note: couldn't restart the background service: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("v0.5.10", "0.5.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.5.2", "0.5.2"));
        assert!(!is_newer("v0.5.1", "0.5.2"));
        assert!(!is_newer("v0.5.2-rc.1", "0.5.2"));
    }

    #[test]
    fn detects_install_method_from_path() {
        assert_eq!(
            install_kind(Path::new("/opt/homebrew/Cellar/mira/0.5.2/bin/mira")),
            InstallKind::Homebrew
        );
        assert_eq!(
            install_kind(Path::new("/Users/me/.cargo/bin/mira")),
            InstallKind::Cargo
        );
        assert_eq!(
            install_kind(Path::new("/usr/local/bin/mira")),
            InstallKind::Standalone
        );
        assert_eq!(
            install_kind(Path::new("/home/me/.local/bin/mira")),
            InstallKind::Standalone
        );
    }

    #[test]
    fn parses_shasum_output() {
        let line = format!("{}  mira-darwin-arm64.tar.gz\n", "AB".repeat(32));
        assert_eq!(parse_sha256(&line), Some("ab".repeat(32)));
        assert_eq!(parse_sha256("nothex  file"), None);
        assert_eq!(parse_sha256(""), None);
    }

    #[test]
    fn replaces_the_target_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let new_bin = dir.path().join("new");
        let target = dir.path().join("mira");
        std::fs::write(&new_bin, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_binary(&new_bin, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        // No staging file left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
