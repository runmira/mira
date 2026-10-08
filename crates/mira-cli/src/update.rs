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

fn install_kind(exe: &Path, cargo_home: Option<&Path>) -> InstallKind {
    let s = exe.to_string_lossy();
    // `cargo install` (any --root) records what it installed next to bin/.
    // Those records list every tool installed into that root, so only count
    // them if they list a `mira` binary.
    let cargo_records = exe
        .parent()
        .and_then(Path::parent)
        .is_some_and(cargo_installed_mira);
    if s.contains("/Cellar/") {
        InstallKind::Homebrew
    } else if cargo_records
        || cargo_home.is_some_and(|h| exe.starts_with(h.join("bin")))
        || s.contains("/.cargo/bin/")
        || s.contains("/target/debug/")
        || s.contains("/target/release/")
    {
        InstallKind::Cargo
    } else {
        InstallKind::Standalone
    }
}

/// Do cargo's install records in `root` list a `mira` binary?
fn cargo_installed_mira(root: &Path) -> bool {
    // .crates2.json: { "installs": { "<pkg> <ver> (<src>)": { "bins": [..] } } }
    if let Ok(text) = std::fs::read_to_string(root.join(".crates2.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let listed = v["installs"].as_object().is_some_and(|installs| {
                installs.values().any(|i| {
                    i["bins"]
                        .as_array()
                        .is_some_and(|bins| bins.iter().any(|b| b == "mira"))
                })
            });
            if listed {
                return true;
            }
        }
    }
    // .crates.toml: `"<pkg> <ver> (<src>)" = ["bin", …]` under [v1].
    std::fs::read_to_string(root.join(".crates.toml")).is_ok_and(|text| {
        text.lines().any(|line| {
            line.rsplit_once('=').is_some_and(|(_, bins)| {
                bins.trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .any(|b| b.trim().trim_matches('"') == "mira")
            })
        })
    })
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

    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from);
    let kind = install_kind(&exe, cargo_home.as_deref());
    if kind == InstallKind::Homebrew && args.version.is_some() && !args.check {
        bail!(
            "Homebrew installs follow the tap's formula, so --version isn't supported; \
             `brew upgrade runmira/tap/mira` installs the latest"
        );
    }

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

    match kind {
        InstallKind::Homebrew => {
            let verb = if args.force { "reinstall" } else { "upgrade" };
            println!("installed with Homebrew — running `brew {verb} runmira/tap/mira`");
            let ok = Proc::new("brew")
                .args([verb, "runmira/tap/mira"])
                .status()
                .context("run brew")?
                .success();
            if !ok {
                bail!("brew {verb} failed");
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
    // Fail on permissions before downloading anything.
    let staged = Staged::new(&exe)?;
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

    extract_binary(&tarball, &staged.path).with_context(|| format!("unpack {asset}"))?;

    // Never swap in something that can't start. Run from the staging file
    // beside the target, not /tmp, which may be mounted noexec.
    let version_out = Proc::new(&staged.path)
        .arg("--version")
        .output()
        .context("run the downloaded binary")?;
    if !version_out.status.success() {
        bail!("the downloaded binary doesn't run on this machine — not installing");
    }

    staged.install(&exe)?;
    println!(
        "updated {current} → {} ({})",
        tag.trim_start_matches('v'),
        exe.display()
    );
    restart_service();
    Ok(())
}

/// Pull the `mira` binary out of a release tarball, in-process. No `tar`
/// from `PATH`: under `sudo mira update` that lookup would run whatever
/// `tar` sits first on the user's `PATH` as root. Only a regular file
/// named `mira` is taken; links and everything else are ignored.
fn extract_binary(tarball: &[u8], dest: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tarball));
    for entry in archive.entries().context("read the archive")? {
        let mut entry = entry.context("read the archive")?;
        // Release tarballs store `./mira`; a lone `mira` is fine too.
        // Anything nested, absolute, or with `..` is not the binary.
        let is_mira = entry
            .path()
            .ok()
            .map(|p| {
                use std::path::Component;
                let parts: Vec<_> = p.components().filter(|c| *c != Component::CurDir).collect();
                matches!(parts.as_slice(), [Component::Normal(n)] if *n == "mira")
            })
            .unwrap_or(false);
        if is_mira && entry.header().entry_type().is_file() {
            let mut out = std::fs::File::create(dest).context("write the binary")?;
            std::io::copy(&mut entry, &mut out).context("write the binary")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
                    .context("make the binary executable")?;
            }
            return Ok(());
        }
    }
    bail!("no mira binary in the archive")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The new binary, staged beside the one it replaces: same filesystem, so
/// the final rename is atomic (and safe while the old binary is running —
/// it keeps its open inode). Removed again unless installed.
struct Staged {
    path: PathBuf,
    installed: bool,
}

impl Staged {
    fn new(target: &Path) -> Result<Self> {
        let dir = target.parent().context("binary has no parent directory")?;
        let path = dir.join(format!(".mira-update-{}", std::process::id()));
        if let Err(e) = std::fs::File::create(&path) {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                bail!(
                    "no permission to write {} — re-run with `sudo mira update`",
                    dir.display()
                );
            }
            return Err(e).with_context(|| format!("write to {}", dir.display()));
        }
        Ok(Self {
            path,
            installed: false,
        })
    }

    fn install(mut self, target: &Path) -> Result<()> {
        std::fs::rename(&self.path, target)
            .with_context(|| format!("replace {}", target.display()))?;
        self.installed = true;
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.installed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn restart_service() {
    // The service belongs to the user, not root; under sudo, launchctl and
    // systemctl --user would look in root's session.
    if std::env::var_os("SUDO_USER").is_some() {
        println!(
            "run `mira service restart` (without sudo) to move the service onto the new version"
        );
        return;
    }
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
            install_kind(Path::new("/opt/homebrew/Cellar/mira/0.5.2/bin/mira"), None),
            InstallKind::Homebrew
        );
        assert_eq!(
            install_kind(Path::new("/Users/me/.cargo/bin/mira"), None),
            InstallKind::Cargo
        );
        assert_eq!(
            install_kind(Path::new("/usr/local/bin/mira"), None),
            InstallKind::Standalone
        );
        assert_eq!(
            install_kind(Path::new("/home/me/.local/bin/mira"), None),
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

    fn tarball(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extracts_only_the_mira_binary() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let tgz = tarball(&[
            ("README", tar::EntryType::Regular, b"hi"),
            ("nested/mira", tar::EntryType::Regular, b"wrong"),
            ("mira", tar::EntryType::Regular, b"binary"),
        ]);
        extract_binary(&tgz, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"binary");
        // The release workflow's layout.
        let tgz = tarball(&[
            ("./LICENSE", tar::EntryType::Regular, b"l"),
            ("./mira", tar::EntryType::Regular, b"release"),
        ]);
        extract_binary(&tgz, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"release");
    }

    #[test]
    fn refuses_a_symlinked_or_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let link = tarball(&[("mira", tar::EntryType::Symlink, b"")]);
        assert!(extract_binary(&link, &dest).is_err());
        let none = tarball(&[("other", tar::EntryType::Regular, b"x")]);
        assert!(extract_binary(&none, &dest).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn staged_binary_replaces_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("mira");
        std::fs::write(&target, b"old").unwrap();
        let staged = Staged::new(&target).unwrap();
        assert_eq!(staged.path.parent(), Some(dir.path()));
        std::fs::write(&staged.path, b"new").unwrap();
        staged.install(&target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn abandoned_staging_is_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("mira");
        std::fs::write(&target, b"old").unwrap();
        drop(Staged::new(&target).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn cargo_installs_are_found_by_their_records() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("bin")).unwrap();
        let exe = root.path().join("bin/mira");
        assert_eq!(install_kind(&exe, None), InstallKind::Standalone);

        // Records for *other* tools in the same root don't make mira a cargo build.
        std::fs::write(
            root.path().join(".crates.toml"),
            "[v1]\n\"ripgrep 14.1.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"rg\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join(".crates2.json"),
            r#"{"installs":{"ripgrep 14.1.0 (registry)":{"bins":["rg"]}}}"#,
        )
        .unwrap();
        assert_eq!(install_kind(&exe, None), InstallKind::Standalone);

        // Either record listing mira does.
        std::fs::write(
            root.path().join(".crates.toml"),
            "[v1]\n\"mira-cli 0.5.2 (path+file:///src/mira/crates/mira-cli)\" = [\"mira\"]\n",
        )
        .unwrap();
        assert_eq!(install_kind(&exe, None), InstallKind::Cargo);
        std::fs::remove_file(root.path().join(".crates.toml")).unwrap();
        std::fs::write(
            root.path().join(".crates2.json"),
            r#"{"installs":{"mira-cli 0.5.2 (path)":{"bins":["mira"]}}}"#,
        )
        .unwrap();
        assert_eq!(install_kind(&exe, None), InstallKind::Cargo);
        // A custom CARGO_HOME, without records.
        assert_eq!(
            install_kind(
                Path::new("/opt/cargo/bin/mira"),
                Some(Path::new("/opt/cargo"))
            ),
            InstallKind::Cargo
        );
    }
}
