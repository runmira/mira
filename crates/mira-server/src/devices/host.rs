//! The device hub as a supervised child process, and the state the Device
//! pane follows.
//!
//! The hub binds loopback with `--require-token`; the token it prints at
//! startup is read from its stdout and only ever sent by Mira's proxy, so
//! nothing else on this machine can drive it either. If the hub exits while
//! it's enabled it is restarted, up to a few times a minute. A hub left over
//! from a Mira that didn't shut down cleanly is found through its pidfile
//! and stopped before a new one starts.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{watch, Mutex};

use super::toolchain::{self, Node, AGENT_DEVICE, HUB};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const BOOT_TIMEOUT: Duration = Duration::from_secs(240);
const MAX_RESTARTS_PER_MINUTE: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HubStatus {
    #[default]
    Off,
    Installing,
    Starting,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTools {
    #[default]
    Absent,
    Installing,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Platform {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Platforms {
    pub ios: Platform,
    pub android: Platform,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    pub hub: String,
    pub agent_device: String,
    pub node: Option<String>,
    pub hub_installed: bool,
    pub agent_device_installed: bool,
}

/// What `GET /api/devices/state` returns and `/api/devices/events` streams.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    pub hub: HubStatus,
    pub hub_detail: Option<String>,
    /// 0…1 while the hub (and Node, if needed) downloads.
    pub hub_progress: Option<f32>,
    pub hub_enabled: bool,
    pub agent_access: bool,
    pub agent_tools: AgentTools,
    pub agent_detail: Option<String>,
    /// 0…1 while agent-device downloads.
    pub agent_progress: Option<f32>,
    pub platforms: Platforms,
    pub versions: Versions,
}

/// A running hub.
#[derive(Clone)]
pub struct HubEndpoint {
    pub origin: String,
    pub token: String,
}

struct Running {
    endpoint: HubEndpoint,
    /// Dropping it kills the hub (`kill_on_drop`).
    stop: tokio::sync::oneshot::Sender<()>,
}

pub struct DeviceService {
    state: watch::Sender<DeviceState>,
    hub: Mutex<Option<Running>>,
    /// Serializes start/stop so toggling quickly can't start two hubs.
    lifecycle: Mutex<()>,
    node: Mutex<Option<Node>>,
    agent_tool: Mutex<Option<toolchain::Tool>>,
    http: reqwest::Client,
}

pub fn service() -> &'static Arc<DeviceService> {
    static S: OnceLock<Arc<DeviceService>> = OnceLock::new();
    S.get_or_init(|| {
        let cfg = mira_config::MiraConfig::load_global()
            .unwrap_or_default()
            .devices;
        let state = DeviceState {
            hub_enabled: cfg.hub_enabled(),
            agent_access: cfg.agent_access(),
            versions: versions(None),
            ..Default::default()
        };
        Arc::new(DeviceService {
            state: watch::channel(state).0,
            hub: Mutex::new(None),
            lifecycle: Mutex::new(()),
            node: Mutex::new(None),
            agent_tool: Mutex::new(None),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        })
    })
}

fn versions(node: Option<&Node>) -> Versions {
    Versions {
        hub: toolchain::pinned(HUB).into(),
        agent_device: toolchain::pinned(AGENT_DEVICE).into(),
        node: node
            .cloned()
            .or_else(toolchain::installed_node)
            .map(|n| n.describe()),
        hub_installed: toolchain::installed(HUB).is_some(),
        agent_device_installed: toolchain::installed(AGENT_DEVICE).is_some(),
    }
}

impl DeviceService {
    pub fn snapshot(&self) -> DeviceState {
        self.state.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<DeviceState> {
        self.state.subscribe()
    }

    fn update(&self, f: impl FnOnce(&mut DeviceState)) {
        self.state.send_if_modified(|s| {
            let before = s.clone();
            f(s);
            *s != before
        });
    }

    /// Bring the hub up at server start when the user left it on. Also
    /// fills in platform support so the pane can say what's missing.
    pub fn resume(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.refresh_platforms().await;
            if this.snapshot().hub_enabled {
                this.start().await;
            }
        });
    }

    pub async fn refresh_platforms(&self) {
        let platforms = platform_support().await;
        self.update(|s| s.platforms = platforms);
    }

    /// Apply a settings change, save it and start or stop what it needs.
    pub async fn set(
        self: &Arc<Self>,
        hub_enabled: Option<bool>,
        agent_access: Option<bool>,
    ) -> Result<DeviceState, String> {
        let mut cfg = mira_config::MiraConfig::load_global().map_err(|e| e.to_string())?;
        if let Some(on) = hub_enabled {
            cfg.devices.hub_enabled = Some(on);
            if !on {
                cfg.devices.agent_access = Some(false);
            }
        }
        if let Some(on) = agent_access {
            cfg.devices.agent_access = Some(on);
        }
        cfg.save_global().map_err(|e| e.to_string())?;
        let (hub_on, agent_on) = (cfg.devices.hub_enabled(), cfg.devices.agent_access());
        let was = self.snapshot();
        self.update(|s| {
            s.hub_enabled = hub_on;
            s.agent_access = agent_on;
        });
        if hub_on && (!was.hub_enabled || was.hub == HubStatus::Failed) {
            let this = self.clone();
            tokio::spawn(async move { this.start().await });
        } else if !hub_on && was.hub_enabled {
            self.stop().await;
        } else if hub_on && agent_on && !was.agent_access {
            let this = self.clone();
            tokio::spawn(async move { this.ensure_agent_tools().await });
        } else if !agent_on && was.agent_access {
            self.stop_agent_daemon().await;
        }
        Ok(self.snapshot())
    }

    /// Install what's missing and start the hub. Idempotent.
    pub async fn start(self: &Arc<Self>) {
        let _l = self.lifecycle.lock().await;
        if self.hub.lock().await.is_some() || !self.snapshot().hub_enabled {
            return;
        }
        match self.start_inner().await {
            Ok(()) => {
                if self.snapshot().agent_access {
                    let this = self.clone();
                    tokio::spawn(async move { this.ensure_agent_tools().await });
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "device hub failed to start");
                self.update(|s| {
                    s.hub = HubStatus::Failed;
                    s.hub_detail = Some(e);
                });
            }
        }
    }

    async fn node(&self, progress: toolchain::Progress<'_>) -> Result<Node, String> {
        let mut slot = self.node.lock().await;
        if let Some(n) = slot.as_ref() {
            return Ok(n.clone());
        }
        let n = toolchain::ensure_node(progress).await?;
        *slot = Some(n.clone());
        Ok(n)
    }

    /// Whether getting Node means downloading it, so one progress bar can
    /// cover that and the install after it.
    async fn node_download_ahead(&self) -> bool {
        self.node.lock().await.is_none()
            && toolchain::installed_node().is_none()
            && toolchain::system_node().await.is_none()
    }

    /// Progress at 1% steps, so the event stream isn't flooded.
    fn set_progress(&self, agent: bool, value: Option<f32>) {
        let value = value.map(|v| (v.clamp(0.0, 1.0) * 100.0).round() / 100.0);
        self.update(|s| {
            let slot = if agent {
                &mut s.agent_progress
            } else {
                &mut s.hub_progress
            };
            // Never step backwards within one install.
            *slot = match (*slot, value) {
                (Some(old), Some(new)) if new < old => Some(old),
                (_, v) => v,
            };
        });
    }

    async fn start_inner(self: &Arc<Self>) -> Result<(), String> {
        let needs_install = toolchain::installed(HUB).is_none();
        self.update(|s| {
            s.hub = if needs_install {
                HubStatus::Installing
            } else {
                HubStatus::Starting
            };
            s.hub_detail = needs_install.then(|| "Installing device hub…".into());
        });
        // Node's download, when there is one, fills the first third.
        let node_share = if needs_install && self.node_download_ahead().await {
            0.35
        } else {
            0.0
        };
        if needs_install {
            self.set_progress(false, Some(0.0));
        }
        if node_share > 0.0 {
            self.update(|s| s.hub_detail = Some("Downloading Node…".into()));
        }
        let node = self
            .node(&|f| self.set_progress(false, Some(f * node_share)))
            .await
            .inspect_err(|_| self.set_progress(false, None))?;
        if needs_install {
            self.update(|s| s.hub_detail = Some("Installing device hub…".into()));
        }
        let tool = toolchain::ensure(HUB, &node, &|f| {
            self.set_progress(false, Some(node_share + f * (1.0 - node_share)))
        })
        .await
        .inspect_err(|_| self.set_progress(false, None))?;
        self.update(|s| {
            s.hub = HubStatus::Starting;
            s.hub_detail = None;
            s.hub_progress = None;
            s.versions = versions(Some(&node));
        });
        self.refresh_platforms().await;
        reap_stale_hub().await;
        let (endpoint, child) = spawn_hub(&node, &tool).await?;
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        *self.hub.lock().await = Some(Running {
            endpoint,
            stop: stop_tx,
        });
        self.update(|s| s.hub = HubStatus::Ready);
        let this = self.clone();
        tokio::spawn(async move { this.supervise(child, stop_rx).await });
        Ok(())
    }

    /// Wait for the hub to exit; restart it if it wasn't asked to stop.
    async fn supervise(
        self: Arc<Self>,
        mut child: tokio::process::Child,
        mut stop: tokio::sync::oneshot::Receiver<()>,
    ) {
        let mut restarts: Vec<Instant> = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stop => {
                    let _ = child.kill().await;
                    remove_pidfile();
                    return;
                }
                status = child.wait() => {
                    remove_pidfile();
                    tracing::warn!(?status, "device hub exited");
                    self.hub.lock().await.take();
                    if !self.snapshot().hub_enabled {
                        return;
                    }
                    restarts.retain(|t| t.elapsed() < Duration::from_secs(60));
                    if restarts.len() >= MAX_RESTARTS_PER_MINUTE {
                        self.update(|s| {
                            s.hub = HubStatus::Failed;
                            s.hub_detail = Some(format!("The device hub keeps exiting ({status:?}). Turn it off and on to retry."));
                        });
                        return;
                    }
                    restarts.push(Instant::now());
                    self.update(|s| s.hub = HubStatus::Starting);
                    let respawned = async {
                        let node = self.node(&|_| {}).await?;
                        let tool = toolchain::installed(HUB).ok_or("the device hub install is missing")?;
                        spawn_hub(&node, &tool).await
                    }
                    .await;
                    match respawned {
                        Ok((endpoint, new_child)) => {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            *self.hub.lock().await = Some(Running { endpoint, stop: tx });
                            self.update(|s| s.hub = HubStatus::Ready);
                            child = new_child;
                            stop = rx;
                        }
                        Err(e) => {
                            self.update(|s| {
                                s.hub = HubStatus::Failed;
                                s.hub_detail = Some(e);
                            });
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Stop the hub and agent tools. Simulators and emulators keep running.
    pub async fn stop(&self) {
        let _l = self.lifecycle.lock().await;
        if let Some(r) = self.hub.lock().await.take() {
            let _ = r.stop.send(());
        }
        self.stop_agent_daemon().await;
        self.update(|s| {
            s.hub = HubStatus::Off;
            s.hub_detail = None;
        });
    }

    pub async fn endpoint(&self) -> Option<HubEndpoint> {
        self.hub.lock().await.as_ref().map(|r| r.endpoint.clone())
    }

    // ---- agent tools ----------------------------------------------------

    pub async fn ensure_agent_tools(self: &Arc<Self>) {
        if self.agent_tool.lock().await.is_some() {
            return;
        }
        let needs_install = toolchain::installed(AGENT_DEVICE).is_none();
        if needs_install {
            self.update(|s| {
                s.agent_tools = AgentTools::Installing;
                s.agent_detail = Some("Installing agent tools…".into());
                s.agent_progress = Some(0.0);
            });
        }
        let result = async {
            let node = self.node(&|_| {}).await?;
            toolchain::ensure(AGENT_DEVICE, &node, &|f| self.set_progress(true, Some(f))).await
        }
        .await;
        self.set_progress(true, None);
        match result {
            Ok(tool) => {
                *self.agent_tool.lock().await = Some(tool);
                let node = self.node.lock().await.clone();
                self.update(|s| {
                    s.agent_tools = AgentTools::Ready;
                    s.agent_detail = None;
                    s.versions = versions(node.as_ref());
                });
            }
            Err(e) => self.update(|s| {
                s.agent_tools = AgentTools::Failed;
                s.agent_detail = Some(e);
            }),
        }
    }

    /// agent-device and the Node to run it with, once installed.
    pub async fn agent_tool(&self) -> Option<(Node, toolchain::Tool)> {
        let tool = self.agent_tool.lock().await.clone()?;
        let node = self.node.lock().await.clone()?;
        Some((node, tool))
    }

    async fn stop_agent_daemon(&self) {
        let Some((node, tool)) = self.agent_tool().await else {
            return;
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new(&node.path)
                .arg(&tool.entry)
                .args(["daemon", "stop", "--state-dir"])
                .arg(agent_state_dir())
                .env("AGENT_DEVICE_NO_UPDATE_NOTIFIER", "1")
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await;
    }

    // ---- talking to the hub --------------------------------------------

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn hub_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let ep = self
            .endpoint()
            .await
            .ok_or("The device hub isn't running. Turn it on in the Device pane.")?;
        let mut req = self
            .http
            .request(method, format!("{}{path}", ep.origin))
            .bearer_auth(&ep.token)
            .timeout(timeout);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| format!("device hub: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() || v.get("ok") == Some(&Value::Bool(false)) {
            return Err(v
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("device hub answered {status}")));
        }
        Ok(v)
    }

    /// Devices from the hub, with stream URLs for the booted ones.
    pub async fn list(&self) -> Result<Value, String> {
        let raw = self
            .hub_json(
                reqwest::Method::GET,
                "/api/devices",
                None,
                Duration::from_secs(30),
            )
            .await?;
        let mut devices = Vec::new();
        for d in raw["simulators"]
            .as_array()
            .into_iter()
            .chain(raw["emulators"].as_array())
            .flatten()
        {
            if d["physical"].as_bool() == Some(true) || d["supported"].as_bool() == Some(false) {
                continue;
            }
            let platform = d["platform"].as_str().unwrap_or("ios");
            let id = d["id"].as_str().unwrap_or_default();
            let booted = d["booted"].as_bool().unwrap_or(false);
            devices.push(json!({
                "platform": platform,
                "id": id,
                "name": d["name"],
                "os": d["version"],
                "booted": booted,
                "frame": d["deviceFrame"],
                "stream": booted.then(|| stream_urls(platform, id)),
            }));
        }
        let state = self.snapshot();
        let mut notes = Vec::new();
        for (name, p, missing) in [
            ("ios", &state.platforms.ios, "Xcode is installed, but no iOS Simulator is available. Install a runtime in Xcode Settings → Components."),
            ("android", &state.platforms.android, "The Android SDK is installed, but no virtual device exists. Create one in Android Studio → Device Manager."),
        ] {
            if let Some(r) = &p.reason {
                notes.push(r.clone());
            } else if p.available && !devices.iter().any(|d| d["platform"] == name) {
                notes.push(missing.to_string());
            }
        }
        Ok(json!({ "devices": devices, "notes": notes }))
    }

    /// Boot a device (no-op when running) and attach its stream. Android
    /// AVDs change id on boot, so the returned id is the one to use.
    pub async fn boot(&self, platform: &str, id: &str) -> Result<Value, String> {
        let r = self
            .hub_json(
                reqwest::Method::POST,
                "/api/devices/boot",
                Some(json!({ "platform": platform, "id": id })),
                BOOT_TIMEOUT,
            )
            .await;
        // Already booted reads as an error on some hub versions; the
        // attach below is what matters.
        let booted_id = match &r {
            Ok(v) => v["serial"]
                .as_str()
                .or(v["id"].as_str())
                .unwrap_or(id)
                .to_string(),
            Err(e) if e.contains("Booted") || e.contains("already") => id.to_string(),
            Err(e) => return Err(e.clone()),
        };
        if platform == "ios" {
            self.hub_json(
                reqwest::Method::POST,
                "/vendor/serve-sim/grid/api/start",
                Some(json!({ "udid": booted_id })),
                BOOT_TIMEOUT,
            )
            .await?;
        }
        Ok(json!({ "id": booted_id, "stream": stream_urls(platform, &booted_id) }))
    }

    pub async fn shutdown(&self, platform: &str, id: &str) -> Result<(), String> {
        let (path, body) = if platform == "ios" {
            ("/vendor/serve-sim/grid/api/shutdown", json!({ "udid": id }))
        } else {
            (
                "/api/devices/shutdown",
                json!({ "platform": platform, "id": id }),
            )
        };
        self.hub_json(
            reqwest::Method::POST,
            path,
            Some(body),
            Duration::from_secs(60),
        )
        .await
        .map(|_| ())
    }

    pub async fn screenshot(&self, platform: &str, id: &str) -> Result<Vec<u8>, String> {
        let ep = self
            .endpoint()
            .await
            .ok_or("The device hub isn't running.")?;
        let prefix = if platform == "ios" {
            "/vendor/serve-sim"
        } else {
            "/vendor/serve-emu"
        };
        let resp = self
            .http
            .post(format!("{}{prefix}/api/screenshot", ep.origin))
            .query(&[("device", id)])
            .bearer_auth(&ep.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("screenshot: {e}"))?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }
}

/// Where the pane connects for a booted device, through Mira's proxy.
pub fn stream_urls(platform: &str, id: &str) -> Value {
    let id = encode(id);
    if platform == "ios" {
        let base = "/api/device-hub/vendor/serve-sim";
        json!({
            "video": format!("{base}/helper/{id}/stream.avcc"),
            "mjpeg": format!("{base}/helper/{id}/stream.mjpeg"),
            "config": format!("{base}/helper/{id}/config"),
            "input": format!("{base}/helper/ws?device={id}"),
        })
    } else {
        json!({ "ws": format!("/api/device-hub/vendor/serve-emu/ws?device={id}") })
    }
}

/// Percent-encode everything but unreserved characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---- process management ---------------------------------------------------

fn pidfile() -> PathBuf {
    toolchain::tools_dir().join("device-hub.pid")
}

fn remove_pidfile() {
    let _ = std::fs::remove_file(pidfile());
}

pub fn agent_state_dir() -> PathBuf {
    toolchain::tools_dir().join("agent-device-state")
}

/// Stop a hub a previous Mira left running. Only kills the pid if it is
/// still a device hub, so a reused pid is left alone.
async fn reap_stale_hub() {
    let Ok(pid) = std::fs::read_to_string(pidfile()) else {
        return;
    };
    let pid = pid.trim();
    if pid.parse::<u32>().is_err() {
        remove_pidfile();
        return;
    }
    let cmd = tokio::process::Command::new("ps")
        .args(["-o", "command=", "-p", pid])
        .output()
        .await;
    if cmd.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(HUB)) {
        let _ = tokio::process::Command::new("kill").arg(pid).status().await;
    }
    remove_pidfile();
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("reserving a port for the device hub: {e}"))
}

async fn spawn_hub(
    node: &Node,
    tool: &toolchain::Tool,
) -> Result<(HubEndpoint, tokio::process::Child), String> {
    let port = free_port()?;
    let mut child = tokio::process::Command::new(&node.path)
        .arg(&tool.entry)
        .args([
            "--port",
            &port.to_string(),
            "--host",
            "127.0.0.1",
            "--require-token",
        ])
        .args(["--hide-sidebar", "--hide-boot-device"])
        .env("PATH", node.path_env())
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("starting the device hub: {e}"))?;
    if let Some(pid) = child.id() {
        let _ = std::fs::write(pidfile(), pid.to_string());
    }
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        if let Some(s) = stderr {
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!(target: "device_hub", "{l}");
            }
        }
    });
    let stdout = child.stdout.take().ok_or("device hub has no stdout")?;
    let (token_tx, token_rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let mut tx = Some(token_tx);
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            match (tx.is_some(), token_in(&l)) {
                (true, Some(t)) => {
                    let _ = tx.take().map(|tx| tx.send(t));
                }
                // Never log the line carrying the token.
                (_, Some(_)) => {}
                _ => tracing::debug!(target: "device_hub", "{l}"),
            }
        }
    });
    let token = tokio::time::timeout(READY_TIMEOUT, token_rx)
        .await
        .map_err(|_| "the device hub didn't start within a minute".to_string())?
        .map_err(|_| "the device hub exited while starting".to_string())?;
    let origin = format!("http://127.0.0.1:{port}");
    wait_ready(&origin).await?;
    Ok((HubEndpoint { origin, token }, child))
}

/// The session token from the hub's startup link (`…/?token=…`).
fn token_in(line: &str) -> Option<String> {
    let rest = &line[line.find("token=")? + "token=".len()..];
    let t: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
        .collect();
    (!t.is_empty()).then_some(t)
}

async fn wait_ready(origin: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if client
            .get(format!("{origin}/readyz"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err("the device hub didn't become ready within a minute".into())
}

// ---- platform support -------------------------------------------------------

async fn platform_support() -> Platforms {
    Platforms {
        ios: ios_support().await,
        android: android_support(),
    }
}

fn unavailable(reason: impl Into<String>) -> Platform {
    Platform {
        available: false,
        reason: Some(reason.into()),
    }
}

async fn ios_support() -> Platform {
    if !cfg!(target_os = "macos") {
        return unavailable("iOS Simulators need macOS with Xcode.");
    }
    let ok = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new("/usr/bin/xcrun")
            .args(["simctl", "help"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .is_ok_and(|s| s.is_ok_and(|s| s.success()));
    if ok {
        Platform {
            available: true,
            reason: None,
        }
    } else {
        unavailable("Xcode isn't set up: install Xcode from the App Store, open it once, then run `sudo xcode-select -s /Applications/Xcode.app`.")
    }
}

fn android_support() -> Platform {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let explicit =
        std::env::var_os("ANDROID_HOME").or_else(|| std::env::var_os("ANDROID_SDK_ROOT"));
    let root = explicit
        .map(PathBuf::from)
        .into_iter()
        .chain([home.join("Library/Android/sdk"), home.join("Android/Sdk")])
        .find(|p| p.is_dir());
    let Some(root) = root else {
        return unavailable("Android SDK not found. Install it with Android Studio, or set ANDROID_HOME to your SDK directory.");
    };
    let shown = root.display();
    if !root.join("platform-tools/adb").is_file() {
        return unavailable(format!("Android SDK Platform-Tools are missing from {shown}. Install them in Android Studio's SDK Manager."));
    }
    if !root.join("emulator/emulator").is_file() {
        return unavailable(format!(
            "Android Emulator is missing from {shown}. Install it in Android Studio's SDK Manager."
        ));
    }
    if !root.join("cmdline-tools/latest/bin/avdmanager").is_file() {
        return unavailable(if root.join("tools/bin/avdmanager").is_file() {
            format!("The Android SDK command-line tools in {shown} are too old. Install Android SDK Command-line Tools (latest) in Android Studio's SDK Manager → SDK Tools.")
        } else {
            format!("Android SDK Command-line Tools (latest) are missing from {shown}. Install them in Android Studio's SDK Manager → SDK Tools.")
        });
    }
    Platform {
        available: true,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_from_startup_link() {
        assert_eq!(
            token_in("  Local:   http://localhost:3499/?token=FyCj-T_A.E~K").as_deref(),
            Some("FyCj-T_A.E~K")
        );
        assert_eq!(token_in("Expo Device Hub ready"), None);
        assert_eq!(token_in("token="), None);
    }

    #[test]
    fn stream_urls_stay_under_the_proxy() {
        let ios = stream_urls("ios", "91D0-ABC");
        for k in ["video", "mjpeg", "config", "input"] {
            assert!(
                ios[k]
                    .as_str()
                    .unwrap()
                    .starts_with("/api/device-hub/vendor/serve-sim/"),
                "{k}"
            );
        }
        assert_eq!(
            stream_urls("android", "emulator-5554")["ws"],
            "/api/device-hub/vendor/serve-emu/ws?device=emulator-5554"
        );
        // Ids are encoded, so one can't climb out of the helper path.
        assert!(stream_urls("ios", "../x")["video"]
            .as_str()
            .unwrap()
            .contains("..%2Fx"));
    }
}
