//! The device tools Mira downloads on the user's say-so: the device hub and
//! agent-device (npm packages), and Node to run them when the system has
//! none new enough.
//!
//! Each tool lives at `~/.mira/tools/<name>/<version>/`. An install builds a
//! `.staging-*` sibling, writes `.install-complete` into it and renames it
//! into place, so a version directory either is whole or doesn't exist: an
//! install killed halfway leaves only a staging directory, which the next
//! attempt removes. Versions are pinned in `tool_versions.json`; a new pin
//! installs the next time the tool is needed and never falls back.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const HUB: &str = "expo-device-hub";
pub const AGENT_DEVICE: &str = "agent-device";

const SENTINEL: &str = ".install-complete";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// agent-device declares `node >=22.12`; the hub runs on anything that does.
const MIN_NODE: (u64, u64) = (22, 12);

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "expo-device-hub")]
    hub: String,
    #[serde(rename = "agent-device")]
    agent_device: String,
    node: NodePin,
    /// Distinct registry requests a fresh install of each pinned tool
    /// makes, which install progress counts against.
    progress_units: std::collections::HashMap<String, u32>,
}

/// Install progress, 0…1, reported as it moves.
pub type Progress<'a> = &'a (dyn Fn(f32) + Send + Sync);

#[derive(Deserialize)]
struct NodePin {
    version: String,
    sha256: std::collections::HashMap<String, String>,
}

fn manifest() -> &'static Manifest {
    static M: OnceLock<Manifest> = OnceLock::new();
    M.get_or_init(|| {
        serde_json::from_str(include_str!("tool_versions.json")).expect("tool_versions.json")
    })
}

/// The pinned version of an npm tool.
pub fn pinned(tool: &str) -> &'static str {
    match tool {
        HUB => &manifest().hub,
        AGENT_DEVICE => &manifest().agent_device,
        _ => unreachable!("unknown device tool {tool}"),
    }
}

/// `~/.mira/tools`, or `MIRA_TOOLS_DIR` (tests).
pub fn tools_dir() -> PathBuf {
    std::env::var_os("MIRA_TOOLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".mira").join("tools"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// An installed npm tool.
#[derive(Clone, Debug)]
pub struct Tool {
    /// The script `node` runs.
    pub entry: PathBuf,
}

fn version_dir(tool: &str, version: &str) -> PathBuf {
    tools_dir().join(tool).join(version)
}

fn entry_for(tool: &str) -> &'static [&'static str] {
    match tool {
        HUB => &["dist", "server", "cli.mjs"],
        AGENT_DEVICE => &["bin", "agent-device.mjs"],
        _ => unreachable!(),
    }
}

fn tool_at(tool: &str, dir: &Path) -> Tool {
    let entry = entry_for(tool)
        .iter()
        .fold(dir.join("node_modules").join(tool), |p, part| p.join(part));
    Tool { entry }
}

fn is_complete(dir: &Path, version: &str) -> bool {
    std::fs::read_to_string(dir.join(SENTINEL)).is_ok_and(|v| v.trim() == version)
}

/// The pinned version of `tool` if it's fully installed. Reads only.
pub fn installed(tool: &str) -> Option<Tool> {
    let dir = version_dir(tool, pinned(tool));
    is_complete(&dir, pinned(tool)).then(|| tool_at(tool, &dir))
}

/// Installs run one at a time: two npm processes in one tree corrupt it,
/// and the hub and agent-device share Node's download.
fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// The pinned version of `tool`, installing it first if needed.
pub async fn ensure(tool: &str, node: &Node, progress: Progress<'_>) -> Result<Tool, String> {
    if let Some(t) = installed(tool) {
        return Ok(t);
    }
    let _guard = install_lock().lock().await;
    if let Some(t) = installed(tool) {
        return Ok(t);
    }
    let version = pinned(tool);
    let parent = tools_dir().join(tool);
    let target = parent.join(version);
    clear_partial(&parent, &target).await?;
    let staging = staging_dir(&parent).await?;
    let units = manifest().progress_units.get(tool).copied().unwrap_or(50);
    let result = npm_install(
        node,
        &staging,
        &format!("{tool}@{version}"),
        units,
        progress,
    )
    .await;
    let result = match result {
        Ok(()) if tool_at(tool, &staging).entry.is_file() => {
            progress(0.97);
            commit(&staging, &target, version)
                .await
                .inspect(|_| progress(1.0))
        }
        Ok(()) => Err(format!("npm finished but {tool} isn't where expected")),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(e);
    }
    prune_other_versions(&parent, version).await;
    Ok(tool_at(tool, &target))
}

/// Remove an incomplete version directory and leftover staging dirs.
async fn clear_partial(parent: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        tokio::fs::remove_dir_all(target)
            .await
            .map_err(|e| format!("removing an incomplete install: {e}"))?;
    }
    if let Ok(mut rd) = tokio::fs::read_dir(parent).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if e.file_name().to_string_lossy().starts_with(".staging-") {
                let _ = tokio::fs::remove_dir_all(e.path()).await;
            }
        }
    }
    Ok(())
}

async fn staging_dir(parent: &Path) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("preparing the install directory: {e}"))?;
    let dir = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(parent)
        .map_err(|e| format!("preparing the install directory: {e}"))?;
    Ok(dir.keep())
}

async fn commit(staging: &Path, target: &Path, version: &str) -> Result<(), String> {
    tokio::fs::write(staging.join(SENTINEL), version)
        .await
        .map_err(|e| format!("finishing the install: {e}"))?;
    tokio::fs::rename(staging, target)
        .await
        .map_err(|e| format!("finishing the install: {e}"))
}

async fn prune_other_versions(parent: &Path, keep: &str) {
    let Ok(mut rd) = tokio::fs::read_dir(parent).await else {
        return;
    };
    while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name();
        if name != keep && !name.to_string_lossy().starts_with('.') {
            let _ = tokio::fs::remove_dir_all(e.path()).await;
        }
    }
}

/// `npm install` with progress from its http log: each distinct registry
/// request it makes counts toward the pinned install's known total.
async fn npm_install(
    node: &Node,
    prefix: &Path,
    spec: &str,
    units: u32,
    progress: Progress<'_>,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut cmd = match std::env::var_os("MIRA_DEVICE_NPM") {
        // Tests stand in a script for npm.
        Some(fake) => tokio::process::Command::new(fake),
        None => {
            let npm = node
                .npm_cli()
                .ok_or("npm isn't installed next to Node; install Node from nodejs.org")?;
            let mut c = tokio::process::Command::new(&node.path);
            c.arg(npm);
            c
        }
    };
    cmd.args(["install", "--prefix"])
        .arg(prefix)
        .args([
            "--no-fund",
            "--no-audit",
            "--omit=dev",
            "--loglevel=http",
            spec,
        ])
        // Install scripts (native prebuilds) must find this Node first.
        .env("PATH", node.path_env())
        .env("npm_config_update_notifier", "false")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("running npm: {e}"))?;
    let stderr = child.stderr.take().ok_or("npm has no stderr")?;
    let run = async {
        let mut seen = std::collections::HashSet::new();
        let mut tail = std::collections::VecDeque::with_capacity(8);
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = registry_url(&line) {
                if seen.insert(url.to_string()) {
                    // Scripts run after the last request; leave room for them.
                    progress((seen.len() as f32 / units.max(1) as f32).min(0.9));
                }
            } else if !line.trim().is_empty() {
                if tail.len() == 8 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|e| format!("running npm: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "npm install {spec} failed: {}",
                tail.into_iter().collect::<Vec<_>>().join("\n")
            ))
        }
    };
    tokio::time::timeout(INSTALL_TIMEOUT, run)
        .await
        .map_err(|_| format!("installing {spec} timed out after 10 minutes"))?
}

/// The registry URL in an `npm http` log line, whether fetched or cached.
fn registry_url(line: &str) -> Option<&str> {
    if !line.starts_with("npm http") {
        return None;
    }
    let start = line.find("https://")?;
    line[start..].split_whitespace().next()
}

// ---- Node ---------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub version: String,
    /// Downloaded by Mira rather than found on the system.
    pub managed: bool,
}

impl Node {
    /// npm's entry script, run with this Node (never a shell shim). Looked
    /// for beside both the path found and the binary it links to: Homebrew
    /// keeps npm under `/opt/homebrew/lib`, not under the Cellar binary.
    fn npm_cli(&self) -> Option<PathBuf> {
        let real = std::fs::canonicalize(&self.path).ok();
        let beside = |node: &Path| {
            let bin = node.parent()?;
            let lib = bin.parent()?.join("lib/node_modules/npm/bin/npm-cli.js");
            let linked = std::fs::canonicalize(bin.join("npm")).ok();
            [Some(lib), linked]
                .into_iter()
                .flatten()
                .find(|p| p.is_file() && p.ends_with("npm-cli.js"))
        };
        beside(&self.path).or_else(|| real.as_deref().and_then(beside))
    }

    /// `PATH` with this Node's directory first.
    pub fn path_env(&self) -> std::ffi::OsString {
        let dir = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let rest = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&rest)))
            .unwrap_or(rest)
    }

    pub fn describe(&self) -> String {
        format!(
            "{} ({})",
            self.version,
            if self.managed { "downloaded" } else { "system" }
        )
    }
}

fn parse_version(v: &str) -> Option<(u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn new_enough(v: &str) -> bool {
    parse_version(v).is_some_and(|v| v >= MIN_NODE)
}

async fn node_version(path: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Where Node usually lives. Apps opened from the Dock get a bare PATH, so
/// version managers' directories are checked as well.
fn node_candidates() -> Vec<PathBuf> {
    let home = home();
    let mut out: Vec<PathBuf> = std::env::var_os("MIRA_NODE")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(paths) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&paths).map(|d| d.join("node")));
    }
    out.extend(
        [
            "/opt/homebrew/bin/node",
            "/usr/local/bin/node",
            "/usr/bin/node",
        ]
        .map(PathBuf::from),
    );
    out.push(home.join(".volta/bin/node"));
    for managed in [
        ".nvm/versions/node",
        ".local/share/fnm/node-versions",
        "Library/Application Support/fnm/node-versions",
    ] {
        if let Ok(rd) = std::fs::read_dir(home.join(managed)) {
            let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            // Newest first, by the version in the directory name.
            dirs.sort_by_key(|d| {
                std::cmp::Reverse(
                    d.file_name()
                        .and_then(|n| parse_version(&n.to_string_lossy()))
                        .unwrap_or((0, 0)),
                )
            });
            for d in dirs {
                out.push(d.join("bin/node"));
                out.push(d.join("installation/bin/node"));
            }
        }
    }
    out
}

/// A system Node new enough to run the tools, if any.
/// `MIRA_DEVICE_SYSTEM_NODE=0` skips it in favor of Mira's own download.
pub async fn system_node() -> Option<Node> {
    if std::env::var_os("MIRA_DEVICE_SYSTEM_NODE").is_some_and(|v| v == "0") {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    for path in node_candidates() {
        if !path.is_file() || !seen.insert(std::fs::canonicalize(&path).unwrap_or(path.clone())) {
            continue;
        }
        if let Some(version) = node_version(&path).await {
            let node = Node {
                path,
                version,
                managed: false,
            };
            // A Node without npm can't install the tools; keep looking.
            if new_enough(&node.version) && node.npm_cli().is_some() {
                return Some(node);
            }
        }
    }
    None
}

fn node_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        _ => None,
    }
}

fn managed_node_path() -> PathBuf {
    let v = &manifest().node.version;
    version_dir("node", v).join("bin").join("node")
}

/// Mira's own Node if it's installed. Reads only.
pub fn installed_node() -> Option<Node> {
    let v = &manifest().node.version;
    is_complete(&version_dir("node", v), v).then(|| Node {
        path: managed_node_path(),
        version: format!("v{v}"),
        managed: true,
    })
}

/// A Node to run the tools with: the system's when new enough, otherwise
/// Mira's pinned download, fetched and checksum-verified on first use.
pub async fn ensure_node(progress: Progress<'_>) -> Result<Node, String> {
    if let Some(n) = installed_node() {
        return Ok(n);
    }
    if let Some(n) = system_node().await {
        return Ok(n);
    }
    let _guard = install_lock().lock().await;
    if let Some(n) = installed_node() {
        return Ok(n);
    }
    let platform =
        node_platform().ok_or("Devices need Node 22.12 or newer; install it from nodejs.org")?;
    let pin = &manifest().node;
    let expected = pin
        .sha256
        .get(platform)
        .ok_or("no Node build for this platform")?;
    let name = format!("node-v{}-{platform}", pin.version);
    let url = format!("https://nodejs.org/dist/v{}/{name}.tar.gz", pin.version);
    let bytes = download(&url, progress).await?;
    let actual = hex(&Sha256::digest(&bytes));
    if &actual != expected {
        return Err(format!(
            "Node download didn't match its checksum ({actual})"
        ));
    }
    let parent = tools_dir().join("node");
    let target = parent.join(&pin.version);
    clear_partial(&parent, &target).await?;
    let staging = staging_dir(&parent).await?;
    let unpack_to = staging.clone();
    let unpacked = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
        let mut archive = tar::Archive::new(gz);
        archive.set_preserve_permissions(true);
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            // Drop the `node-vX-platform/` top directory.
            let rel: PathBuf = path.components().skip(1).collect();
            if rel.as_os_str().is_empty() {
                continue;
            }
            entry
                .unpack(unpack_to.join(rel))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r.map_err(|e| format!("unpacking Node: {e}")));
    let result = match unpacked {
        Ok(()) => commit(&staging, &target, &pin.version).await,
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(e);
    }
    prune_other_versions(&parent, &pin.version).await;
    Ok(Node {
        path: managed_node_path(),
        version: format!("v{}", pin.version),
        managed: true,
    })
}

async fn download(url: &str, progress: Progress<'_>) -> Result<Vec<u8>, String> {
    use futures::StreamExt;
    let resp = reqwest::Client::builder()
        .timeout(INSTALL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("downloading Node: {e}"))?;
    let total = resp.content_length().unwrap_or(0);
    let mut bytes = Vec::with_capacity(total as usize);
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk.map_err(|e| format!("downloading Node: {e}"))?);
        if total > 0 {
            progress(bytes.len() as f32 / total as f32);
        }
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_version_floor() {
        assert!(new_enough("v22.12.0"));
        assert!(new_enough("v24.1.0"));
        assert!(!new_enough("v22.11.9"));
        assert!(!new_enough("v20.19.0"));
        assert!(!new_enough("garbage"));
    }

    /// A stand-in for npm: creates the package's entry file, or fails when
    /// `FAIL` exists next to it.
    fn fake_npm(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("npm");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nset -e\n[ -e '{fail}' ] && exit 3\nprefix=$3\n\
                 mkdir -p \"$prefix/node_modules/expo-device-hub/dist/server\"\n\
                 echo ok > \"$prefix/node_modules/expo-device-hub/dist/server/cli.mjs\"\n",
                fail = dir.join("FAIL").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[tokio::test]
    async fn staged_install_is_all_or_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = tmp.path().join("tools");
        // Env is process-wide; this is the only test that sets these.
        std::env::set_var("MIRA_TOOLS_DIR", &tools);
        std::env::set_var("MIRA_DEVICE_NPM", fake_npm(tmp.path()));
        let node = Node {
            path: "/nonexistent/node".into(),
            version: "v22.12.0".into(),
            managed: false,
        };
        let version = pinned(HUB);

        // Leftovers from an install killed halfway: a staging dir and a
        // version dir without the sentinel.
        let parent = tools.join(HUB);
        std::fs::create_dir_all(parent.join(".staging-old")).unwrap();
        std::fs::create_dir_all(parent.join(version).join("node_modules")).unwrap();
        assert!(installed(HUB).is_none(), "no sentinel, not installed");

        // A failing npm leaves nothing behind that looks installed.
        std::fs::write(tmp.path().join("FAIL"), "").unwrap();
        assert!(ensure(HUB, &node, &|_| {}).await.is_err());
        assert!(installed(HUB).is_none());
        assert!(!parent.join(version).exists());
        assert!(
            std::fs::read_dir(&parent).unwrap().count() == 0,
            "staging cleaned up"
        );

        // Then a good install lands whole, and an old version is pruned.
        std::fs::remove_file(tmp.path().join("FAIL")).unwrap();
        std::fs::create_dir_all(parent.join("0.0.1")).unwrap();
        let tool = ensure(HUB, &node, &|_| {}).await.unwrap();
        assert!(tool.entry.is_file());
        assert_eq!(
            std::fs::read_to_string(parent.join(version).join(SENTINEL)).unwrap(),
            version
        );
        assert!(!parent.join("0.0.1").exists());
        assert!(installed(HUB).is_some());

        std::env::remove_var("MIRA_TOOLS_DIR");
        std::env::remove_var("MIRA_DEVICE_NPM");
    }

    #[test]
    fn finds_npm_beside_a_linked_node() {
        use std::os::unix::fs::symlink;
        // Homebrew's layout: bin/node links into the Cellar, npm lives in
        // the prefix's lib.
        let t = tempfile::tempdir().unwrap();
        let cellar = t.path().join("Cellar/node/25/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::write(cellar.join("node"), "").unwrap();
        let npm = t.path().join("lib/node_modules/npm/bin");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(npm.join("npm-cli.js"), "").unwrap();
        std::fs::create_dir_all(t.path().join("bin")).unwrap();
        symlink(cellar.join("node"), t.path().join("bin/node")).unwrap();
        let node = Node {
            path: t.path().join("bin/node"),
            version: "v25.0.0".into(),
            managed: false,
        };
        assert!(node.npm_cli().is_some_and(|p| p.ends_with("npm-cli.js")));
        let bare = Node {
            path: cellar.join("node"),
            version: "v25.0.0".into(),
            managed: false,
        };
        assert!(bare.npm_cli().is_none());
    }

    #[test]
    fn counts_registry_requests_from_npm_http_lines() {
        let fetch = "npm http fetch GET 200 https://registry.npmjs.org/ws/-/ws-8.18.0.tgz 34ms (cache miss)";
        let cached =
            "npm http cache ws@https://registry.npmjs.org/ws/-/ws-8.18.0.tgz 0ms (cache hit)";
        assert_eq!(
            registry_url(fetch),
            Some("https://registry.npmjs.org/ws/-/ws-8.18.0.tgz")
        );
        assert_eq!(
            registry_url(cached),
            registry_url(fetch),
            "a cache hit counts once"
        );
        assert_eq!(registry_url("npm warn deprecated x"), None);
    }

    #[test]
    fn manifest_pins_every_platform() {
        let m = manifest();
        assert!(!m.hub.is_empty() && !m.agent_device.is_empty());
        assert!(m.progress_units[HUB] > 0 && m.progress_units[AGENT_DEVICE] > 0);
        for p in ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"] {
            assert_eq!(m.node.sha256[p].len(), 64, "{p}");
        }
    }
}
