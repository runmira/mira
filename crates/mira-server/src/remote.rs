//! Remote access: make this computer's Mira reachable from the user's phone
//! at a stable `https://m-….runmira.dev`, with no networking setup.
//!
//! The runmira backend (supabase/functions/remote-access) owns the
//! Cloudflare side: it creates a tunnel and hostname for this (user,
//! computer) and hands back a connector token. This module runs Cloudflare's
//! connector, `cloudflared`, with that token. The connector dials out to
//! Cloudflare, which forwards requests to our loopback port — so nothing is
//! opened on the user's network, and every forwarded request still goes
//! through device pairing (`pairing.rs`): a hostname grants nothing.
//!
//! `cloudflared` is downloaded on first use: a pinned release, checked
//! against SHA-256s baked in here, cached in `~/.mira/bin`. The token is
//! kept owner-only in `~/.mira/remote.json` and passed to the connector in
//! its environment, not its arguments (which other local users can list).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::pairing::Caller;

const CLOUDFLARED_VERSION: &str = "2026.10.0";
/// Hostnames the backend hands out; anything else is refused.
const DOMAIN_SUFFIX: &str = ".runmira.dev";

/// (asset, sha256) for this platform's pinned `cloudflared`.
fn cloudflared_asset() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some((
            "cloudflared-darwin-arm64.tgz",
            "a2f79ff7b9420aa537d74af239f376da170bbabeb529aec416002adac6a72e70",
        )),
        ("macos", "x86_64") => Some((
            "cloudflared-darwin-amd64.tgz",
            "903845b81828c8cb3c5d13d816a2de71c06a3da5785469df8eb0e1b736d92f9f",
        )),
        ("linux", "x86_64") => Some((
            "cloudflared-linux-amd64",
            "d33ff2d14475178d2012c2c56beba87389ac5ded27649519f198a7d3134a99db",
        )),
        ("linux", "aarch64") => Some((
            "cloudflared-linux-arm64",
            "e6422b9d4f72d3194bc5a38676f13667c06666523217b842a877d72a80b5ac08",
        )),
        _ => None,
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct RemoteConfig {
    hostname: String,
    token: String,
    /// The local port the tunnel was pointed at when it was enabled.
    /// Cloudflare forwards there, so if Mira restarts on another port (the
    /// desktop app picks the next free one) the tunnel has to be re-pointed;
    /// `view` reports `port_stale` and the signed-in UI does it. 0 = unknown
    /// (saved before this field existed), treated as stale.
    #[serde(default)]
    port: u16,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Off,
    /// Downloading the connector or dialing Cloudflare.
    Starting,
    Connected,
    /// The connector exited or lost Cloudflare; retrying.
    Reconnecting,
}

struct Inner {
    config: Option<RemoteConfig>,
    status: Status,
    error: Option<String>,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub struct Remote {
    dir: PathBuf,
    port: u16,
    machine_id: String,
    inner: Mutex<Inner>,
}

impl Remote {
    /// Load state from `dir` (`~/.mira`) and, if remote access was on,
    /// start the connector again.
    pub fn start(dir: PathBuf, port: u16) -> Arc<Self> {
        let machine_id = load_or_create_machine_id(&dir.join("machine-id"));
        let config = std::fs::read_to_string(dir.join("remote.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<RemoteConfig>(&t).ok());
        let remote = Arc::new(Self {
            dir,
            port,
            machine_id,
            inner: Mutex::new(Inner {
                config: None,
                status: Status::Off,
                error: None,
                task: None,
            }),
        });
        if let Some(cfg) = config {
            remote.run(cfg);
        }
        remote
    }

    pub fn default_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".mira")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_status(&self, status: Status, error: Option<String>) {
        let mut inner = self.lock();
        inner.status = status;
        if error.is_some() || status == Status::Connected {
            inner.error = error;
        }
    }

    /// (Re)start the connector for `cfg`.
    fn run(self: &Arc<Self>, cfg: RemoteConfig) {
        let me = self.clone();
        let mut inner = self.lock();
        if let Some(t) = inner.task.take() {
            t.abort();
        }
        inner.config = Some(cfg.clone());
        inner.status = Status::Starting;
        inner.error = None;
        inner.task = Some(tokio::spawn(async move { me.supervise(cfg).await }));
    }

    async fn supervise(self: Arc<Self>, cfg: RemoteConfig) {
        let mut backoff = Duration::from_secs(2);
        loop {
            match self.connect_once(&cfg).await {
                Ok(()) => {}
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "remote access connector");
                    self.set_status(Status::Reconnecting, Some(format!("{e:#}")));
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
            self.set_status(Status::Reconnecting, None);
        }
    }

    /// Run the connector until it exits.
    async fn connect_once(&self, cfg: &RemoteConfig) -> Result<()> {
        let bin = ensure_cloudflared(&self.dir.join("bin")).await?;
        let mut child = tokio::process::Command::new(&bin)
            .args(["tunnel", "--no-autoupdate", "run"])
            .env("TUNNEL_TOKEN", &cfg.token)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("start cloudflared")?;
        let stderr = child.stderr.take().context("cloudflared stderr")?;
        let mut lines = BufReader::new(stderr).lines();
        let mut last_error = None;
        while let Ok(Some(line)) = lines.next_line().await {
            if line.contains("Registered tunnel connection") {
                self.set_status(Status::Connected, None);
            } else if line.contains(" ERR ") {
                last_error = Some(
                    line.split(" ERR ")
                        .nth(1)
                        .unwrap_or(&line)
                        .trim()
                        .to_string(),
                );
            }
        }
        let status = child.wait().await.context("wait for cloudflared")?;
        bail!(
            "cloudflared exited ({status}){}",
            last_error.map(|e| format!(": {e}")).unwrap_or_default()
        )
    }

    fn enable(self: &Arc<Self>, cfg: RemoteConfig) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&cfg)?;
        std::fs::create_dir_all(&self.dir)?;
        mira_config::write_private(&self.dir.join("remote.json"), &bytes)
            .context("save remote access settings")?;
        self.run(cfg);
        Ok(())
    }

    fn disable(&self) {
        let mut inner = self.lock();
        if let Some(t) = inner.task.take() {
            t.abort();
        }
        inner.config = None;
        inner.status = Status::Off;
        inner.error = None;
        let _ = std::fs::remove_file(self.dir.join("remote.json"));
    }

    #[cfg(test)]
    fn disable_task_for_test(&self) {
        if let Some(t) = self.lock().task.take() {
            t.abort();
        }
    }

    fn view(&self) -> serde_json::Value {
        let inner = self.lock();
        serde_json::json!({
            "machine_id": self.machine_id,
            "port": self.port,
            "enabled": inner.config.is_some(),
            "hostname": inner.config.as_ref().map(|c| c.hostname.clone()),
            "port_stale": inner.config.as_ref().is_some_and(|c| c.port != self.port),
            "status": inner.status,
            "error": inner.error,
            "supported": cloudflared_asset().is_some(),
        })
    }
}

fn load_or_create_machine_id(path: &Path) -> String {
    if let Ok(id) = std::fs::read_to_string(path) {
        let id = id.trim().to_string();
        if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id.to_ascii_lowercase();
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, &id);
    id
}

/// The pinned `cloudflared`, downloading and verifying it on first use.
async fn ensure_cloudflared(bin_dir: &Path) -> Result<PathBuf> {
    let target = bin_dir.join(format!("cloudflared-{CLOUDFLARED_VERSION}"));
    if target.is_file() {
        return Ok(target);
    }
    let (asset, sha256) =
        cloudflared_asset().context("remote access isn't available on this platform")?;
    let url = format!(
        "https://github.com/cloudflare/cloudflared/releases/download/{CLOUDFLARED_VERSION}/{asset}"
    );
    tracing::info!(%url, "downloading cloudflared");
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .context("download cloudflared")?
        .bytes()
        .await
        .context("download cloudflared")?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != sha256 {
        bail!("cloudflared download didn't match its checksum; not running it");
    }
    let binary = if asset.ends_with(".tgz") {
        extract_cloudflared(&bytes)?
    } else {
        bytes.to_vec()
    };
    std::fs::create_dir_all(bin_dir).context("create ~/.mira/bin")?;
    let staged = bin_dir.join(format!(".cloudflared-{}", std::process::id()));
    std::fs::write(&staged, binary).context("save cloudflared")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&staged, &target).context("install cloudflared")?;
    Ok(target)
}

fn extract_cloudflared(tgz: &[u8]) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let parts: Vec<_> = path
            .components()
            .filter(|c| *c != std::path::Component::CurDir)
            .collect();
        let is_bin =
            matches!(parts.as_slice(), [std::path::Component::Normal(n)] if *n == "cloudflared");
        if is_bin && entry.header().entry_type().is_file() {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut out)?;
            return Ok(out);
        }
    }
    bail!("cloudflared archive has no cloudflared binary")
}

// ---- endpoints ----------------------------------------------------------------

fn local_only(req: &Request) -> Option<Response> {
    let local = matches!(req.extensions().get::<Caller>(), Some(Caller::Local));
    (!local).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "only from the computer running Mira" })),
        )
            .into_response()
    })
}

/// `GET /api/remote` — whether remote access is on, and how it's doing.
pub async fn status(State(remote): State<Arc<Remote>>) -> Response {
    Json(remote.view()).into_response()
}

#[derive(Deserialize)]
struct EnableBody {
    hostname: String,
    token: String,
}

/// `POST /api/remote/enable` — start the connector with what the runmira
/// backend returned (local only).
pub async fn enable(State(remote): State<Arc<Remote>>, req: Request) -> Response {
    if let Some(r) = local_only(&req) {
        return r;
    }
    let body = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Ok(body) = serde_json::from_slice::<EnableBody>(&body) else {
        return (StatusCode::BAD_REQUEST, "expected { hostname, token }").into_response();
    };
    if !valid_hostname(&body.hostname) || body.token.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "not a Mira remote access hostname").into_response();
    }
    match remote.enable(RemoteConfig {
        hostname: body.hostname,
        token: body.token.trim().to_string(),
        // The UI asked the backend to point the tunnel at this port.
        port: remote.port,
    }) {
        Ok(()) => Json(remote.view()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{e:#}") })),
        )
            .into_response(),
    }
}

/// `POST /api/remote/disable` — stop the connector (local only). The
/// caller then asks the backend to delete the tunnel.
pub async fn disable(State(remote): State<Arc<Remote>>, req: Request) -> Response {
    if let Some(r) = local_only(&req) {
        return r;
    }
    remote.disable();
    Json(remote.view()).into_response()
}

fn valid_hostname(h: &str) -> bool {
    h.strip_suffix(DOMAIN_SUFFIX).is_some_and(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_must_be_ours() {
        assert!(valid_hostname("m-abc123.runmira.dev"));
        assert!(!valid_hostname("runmira.dev"));
        assert!(!valid_hostname("evil.example"));
        assert!(!valid_hostname("m-abc.runmira.dev.evil.example"));
        assert!(!valid_hostname("a.b.runmira.dev"));
        assert!(!valid_hostname("UPPER.runmira.dev"));
    }

    #[test]
    fn a_restart_on_another_port_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let saved = RemoteConfig {
            hostname: "m-abc.runmira.dev".into(),
            token: "t".into(),
            port: 8787,
        };
        std::fs::write(
            dir.path().join("remote.json"),
            serde_json::to_vec(&saved).unwrap(),
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let same = Remote::start(dir.path().to_path_buf(), 8787);
            assert_eq!(same.view()["port_stale"], false);
            same.disable_task_for_test();
            let moved = Remote::start(dir.path().to_path_buf(), 8790);
            assert_eq!(moved.view()["port_stale"], true);
            moved.disable_task_for_test();
        });
        // Saved before the port was recorded: unknown, so re-point.
        let old: RemoteConfig =
            serde_json::from_str(r#"{"hostname":"m-abc.runmira.dev","token":"t"}"#).unwrap();
        assert_eq!(old.port, 0);
    }

    #[test]
    fn machine_id_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        let a = load_or_create_machine_id(&path);
        assert_eq!(a.len(), 32);
        assert_eq!(load_or_create_machine_id(&path), a);
    }

    #[test]
    fn every_supported_platform_is_pinned() {
        // The platforms Mira ships for (release.yml) all have a connector.
        if matches!(std::env::consts::OS, "macos" | "linux")
            && matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
        {
            let (_, sha) = cloudflared_asset().unwrap();
            assert_eq!(sha.len(), 64);
        }
    }

    #[test]
    fn extracts_the_binary_from_the_release_layout() {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Regular);
        builder
            .append_data(&mut header, "cloudflared", &b"bin"[..])
            .unwrap();
        let tgz = builder.into_inner().unwrap().finish().unwrap();
        assert_eq!(extract_cloudflared(&tgz).unwrap(), b"bin");
    }
}
