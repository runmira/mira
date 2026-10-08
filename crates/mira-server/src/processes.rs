//! The Processes pane: what the agent left running, and what's listening.
//!
//! `GET  /api/processes?session=`            — the session's background
//!       processes (`run_background`) plus every TCP port listening on
//!       this machine, each matched to the process that owns it.
//! `POST /api/processes?session=`            — start a command, as the
//!       agent's `run_background` would (`{ "command": "npm run dev" }`).
//! `GET  /api/processes/:id/output?session=&tail=` — recent output.
//! `POST /api/processes/:id/stop?session=`   — stop it (and its group).
//! `POST /api/processes/:id/restart?session=` — stop, then run it again.
//! `DELETE /api/processes/:id?session=`      — forget a finished one.
//! Everything that acts takes a JSON body, so another site can't fire it
//! with a plain form post.
//! `POST /api/ports/stop`                    — `{ "pid": 123 }`: stop
//!       whatever listens on a port. Only pids currently listening, and
//!       never this server.
//!
//! Ports come from `lsof`, so they show on macOS and Linux; elsewhere the
//! list is empty and the processes still work.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_tools::builtin::background::ProcessEntry;
use mira_tools::context::ToolProgressSink;
use serde::{Deserialize, Serialize};

use crate::slot::{SessionProgress, SessionSlot};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    tail: Option<usize>,
    #[serde(default)]
    owned_only: bool,
}

#[derive(Debug, Serialize)]
pub struct ProcessView {
    id: u32,
    command: String,
    cwd: String,
    /// The tool card it came from (empty when started from the pane).
    call_id: String,
    pid: Option<u32>,
    running: bool,
    exit_code: Option<i32>,
    /// Stopped on request rather than exiting on its own.
    stopped: bool,
    elapsed_secs: u64,
    /// Ports its process group is listening on.
    ports: Vec<u16>,
    /// The last local URL it printed (`http://localhost:5173/`), if any —
    /// usually the address of the dev server it runs.
    url: Option<String>,
    /// The last line of output, for the collapsed row.
    last_line: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PortView {
    port: u16,
    pid: u32,
    /// The executable name `lsof` reports (`node`, `python3`).
    command: String,
    /// `*`, `127.0.0.1`, `[::1]` — where it listens.
    address: String,
    /// The background process that owns it, when the agent started it.
    process_id: Option<u32>,
    /// An installed app or OS service (`/Applications`, `/System`…), not
    /// something you're developing. The pane hides these by default.
    system: bool,
}

#[derive(Debug, Serialize)]
pub struct ListView {
    processes: Vec<ProcessView>,
    ports: Vec<PortView>,
    /// This server's own port, so the pane can leave it out.
    self_port: u16,
}

async fn slot_for(state: &AppState, q: &SessionQuery) -> Result<Arc<SessionSlot>,Response> {
    match q.session.as_deref() {
        Some(id) if !id.is_empty() => state.slot_str(id).await.ok_or_else(||err(StatusCode::NOT_FOUND,"chat is not loaded")),
        _ => Ok(state.active_slot().await),
    }
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

pub async fn list(State(state): State<AppState>, Query(q): Query<SessionQuery>) -> Response {
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    let entries = slot.bg_processes.list().await;
    let (ports, procs) = if q.owned_only {
        (Vec::new(),HashMap::new())
    } else {
        tokio::task::spawn_blocking(|| (listening_ports(), process_table())).await.unwrap_or_default()
    };

    let mut ports: Vec<PortView> = ports;
    let by_pid: HashMap<u32, u32> = entries
        .iter()
        .filter_map(|e| {
            let p = e.pid.load(Ordering::SeqCst);
            (p != 0).then_some((p, e.id))
        })
        .collect();
    for p in &mut ports {
        // The wrapper leads its own group, so a port whose process is in
        // that group (the `node` under `npm` under `bash`) is the agent's.
        let (pgid, exe) = procs
            .get(&p.pid)
            .map(|(g, e)| (*g, e.as_str()))
            .unwrap_or((p.pid, ""));
        p.process_id = by_pid.get(&pgid).or_else(|| by_pid.get(&p.pid)).copied();
        p.system = p.process_id.is_none() && is_system_exe(exe);
    }

    let mut processes = Vec::with_capacity(entries.len());
    for e in entries {
        processes.push(view(&e, &ports).await);
    }
    // Newest first: the one you just started is the one you care about.
    processes.reverse();
    Json(ListView {
        processes,
        ports,
        self_port: state.local_port,
    })
    .into_response()
}

async fn view(e: &Arc<ProcessEntry>, ports: &[PortView]) -> ProcessView {
    let exit_code = *e.exit_code.lock().await;
    let pid = e.pid.load(Ordering::SeqCst);
    let lines = e.tail(400).await;
    let mut own: Vec<u16> = ports
        .iter()
        .filter(|p| p.process_id == Some(e.id))
        .map(|p| p.port)
        .collect();
    own.sort_unstable();
    own.dedup();
    ProcessView {
        id: e.id,
        command: e.command.clone(),
        cwd: e.cwd.display().to_string(),
        call_id: e.call_id.clone(),
        pid: (pid != 0).then_some(pid),
        running: exit_code.is_none(),
        exit_code,
        stopped: e.stopped.load(Ordering::SeqCst),
        elapsed_secs: e.started_at.elapsed().as_secs(),
        ports: own,
        url: lines.iter().rev().find_map(|l| local_url(l)),
        last_line: lines
            .iter()
            .rev()
            .map(|l| strip_ansi(l))
            .find(|l| !l.trim().is_empty()),
    }
}

#[derive(Debug, Deserialize)]
pub struct StartBody {
    command: String,
}

pub async fn start(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
    Json(body): Json<StartBody>,
) -> Response {
    let command = body.command.trim().to_owned();
    if command.is_empty() {
        return err(StatusCode::BAD_REQUEST, "command is empty");
    }
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    let cwd = slot.cwd.read().await.clone();
    let id = slot
        .bg_processes
        .spawn(command, cwd, String::new(), None)
        .await;
    Json(serde_json::json!({ "id": id })).into_response()
}

pub async fn output(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<SessionQuery>,
) -> Response {
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    let Some(e) = slot.bg_processes.get(id).await else {
        return err(StatusCode::NOT_FOUND, format!("no process {id}"));
    };
    let lines: Vec<String> = e
        .tail(q.tail.unwrap_or(400).min(1000))
        .await
        .iter()
        .map(|l| strip_ansi(l))
        .collect();
    Json(serde_json::json!({ "lines": lines, "running": e.running().await })).into_response()
}

pub async fn stop(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<SessionQuery>,
    // A JSON body (even `{}`) so a cross-site form can't trigger it: that
    // content type needs a CORS preflight, which this server never grants.
    Json(_): Json<serde_json::Value>,
) -> Response {
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    let Some(e) = slot.bg_processes.get(id).await else {
        return err(StatusCode::NOT_FOUND, format!("no process {id}"));
    };
    e.stop();
    wait_exit(&e).await;
    Json(serde_json::json!({ "ok": true })).into_response()
}

pub async fn restart(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<SessionQuery>,
    // A JSON body (even `{}`) so a cross-site form can't trigger it: that
    // content type needs a CORS preflight, which this server never grants.
    Json(_): Json<serde_json::Value>,
) -> Response {
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    let Some(e) = slot.bg_processes.get(id).await else {
        return err(StatusCode::NOT_FOUND, format!("no process {id}"));
    };
    if e.running().await {
        e.stop();
        wait_exit(&e).await;
    }
    // Same tool card, so the agent's card keeps streaming the new run.
    let sink: Option<Arc<dyn ToolProgressSink>> = (!e.call_id.is_empty()).then(|| {
        Arc::new(SessionProgress {
            tx: slot.events_tx.clone(),
        }) as Arc<dyn ToolProgressSink>
    });
    let new_id = slot
        .bg_processes
        .spawn(e.command.clone(), e.cwd.clone(), e.call_id.clone(), sink)
        .await;
    slot.bg_processes.remove_finished(id).await;
    Json(serde_json::json!({ "id": new_id })).into_response()
}

pub async fn forget(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<SessionQuery>,
) -> Response {
    let slot = match slot_for(&state,&q).await {Ok(slot)=>slot,Err(response)=>return response};
    if slot.bg_processes.remove_finished(id).await {
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        err(StatusCode::CONFLICT, "still running — stop it first")
    }
}

#[derive(Debug, Deserialize)]
pub struct PortStopBody {
    pid: u32,
}

pub async fn stop_port(Json(body): Json<PortStopBody>) -> Response {
    let pid = body.pid;
    if pid == std::process::id() {
        return err(StatusCode::BAD_REQUEST, "that's Mira itself");
    }
    let listening = tokio::task::spawn_blocking(listening_ports)
        .await
        .unwrap_or_default();
    if !listening.iter().any(|p| p.pid == pid) {
        return err(StatusCode::NOT_FOUND, "nothing with that pid is listening");
    }
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    match status {
        Ok(s) if s.success() => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(_) => err(StatusCode::FORBIDDEN, "couldn't stop it (not yours?)"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Up to ~2s for a stopped process to actually exit, so a restart doesn't
/// race the old one for its port.
async fn wait_exit(e: &ProcessEntry) {
    for _ in 0..20 {
        if !e.running().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Every listening TCP socket, from `lsof -F` (pid, command, name).
fn listening_ports() -> Vec<PortView> {
    let Ok(out) = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-FpcnT"])
        .output()
    else {
        return Vec::new();
    };
    parse_lsof(&String::from_utf8_lossy(&out.stdout))
}

fn parse_lsof(text: &str) -> Vec<PortView> {
    let mut out: Vec<PortView> = Vec::new();
    let (mut pid, mut cmd) = (0u32, String::new());
    for line in text.lines() {
        let (tag, rest) = line.split_at(line.len().min(1));
        match tag {
            "p" => pid = rest.parse().unwrap_or(0),
            "c" => cmd = rest.to_owned(),
            "n" => {
                // `*:5173`, `127.0.0.1:8787`, `[::1]:3000`
                let Some((addr, port)) = rest.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.parse::<u16>() else {
                    continue;
                };
                // IPv4 and IPv6 sockets for one server: show it once.
                if out.iter().any(|p| p.port == port && p.pid == pid) {
                    continue;
                }
                out.push(PortView {
                    port,
                    pid,
                    command: cmd.clone(),
                    address: addr.to_owned(),
                    process_id: None,
                    system: false,
                });
            }
            _ => {}
        }
    }
    out.sort_by_key(|p| p.port);
    out
}

/// pid → (process group id, executable path). The group matches a port to
/// the wrapper that leads it; the path tells an installed app from a dev
/// server.
fn process_table() -> HashMap<u32, (u32, String)> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,pgid=,comm="])
        .output()
    else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let pgid = it.next()?.parse().ok()?;
            let exe = it.collect::<Vec<_>>().join(" ");
            Some((pid, (pgid, exe)))
        })
        .collect()
}

/// Installed apps and OS services: AirPlay on :5000/:7000, Spotify,
/// Raycast… They listen, but they're not what you're building.
fn is_system_exe(exe: &str) -> bool {
    [
        "/System/",
        "/Applications/",
        "/usr/libexec/",
        "/usr/sbin/",
        "/sbin/",
        "/Library/",
    ]
    .iter()
    .any(|p| exe.starts_with(p))
        // Anything inside an app bundle, wherever it lives (translocated
        // apps run from /private/var/…; Chrome for Testing from a cache).
        || exe.contains(".app/Contents/")
}

/// The first `http://localhost:PORT…`-style URL in a line, normalized to
/// `localhost` (a dev server's `0.0.0.0` isn't a page you can open).
fn local_url(line: &str) -> Option<String> {
    let line = strip_ansi(line);
    for scheme in ["http://", "https://"] {
        let mut from = 0;
        while let Some(i) = line[from..].find(scheme) {
            let start = from + i;
            let rest = &line[start + scheme.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == '>')
                .unwrap_or(rest.len());
            let hostport = &rest[..end];
            let host = hostport
                .split(['/', '?'])
                .next()
                .unwrap_or("")
                .rsplit_once(':')
                .map(|(h, p)| (h, p.parse::<u16>().is_ok()));
            if let Some((h, true)) = host {
                if matches!(h, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]" | "[::]") {
                    let url = format!("{scheme}{hostport}");
                    return Some(
                        url.replacen("0.0.0.0", "localhost", 1)
                            .replacen("127.0.0.1", "localhost", 1)
                            .replacen("[::1]", "localhost", 1)
                            .replacen("[::]", "localhost", 1),
                    );
                }
            }
            from = start + scheme.len();
        }
    }
    None
}

/// Drop ANSI color/cursor escapes so output reads as plain text.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lsof_and_dedups_v4_v6() {
        let text = "p100\ncnode\nn*:5173\nn[::1]:5173\np200\ncmira\nn127.0.0.1:8787\n";
        let ports = parse_lsof(text);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].port, 5173);
        assert_eq!(ports[0].command, "node");
        assert_eq!(ports[1].address, "127.0.0.1");
    }

    #[test]
    fn tells_apps_from_dev_servers() {
        assert!(is_system_exe(
            "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter"
        ));
        assert!(is_system_exe(
            "/Applications/Spotify.app/Contents/MacOS/Spotify"
        ));
        assert!(is_system_exe(
            "/Users/me/Applications/Raycast.app/Contents/MacOS/Raycast"
        ));
        assert!(is_system_exe(
            "/private/var/folders/x/T/AppTranslocation/1/d/Visual Studio Code.app/Contents/MacOS/Code Helper"
        ));
        assert!(!is_system_exe("/opt/homebrew/bin/node"));
        assert!(!is_system_exe("node"));
    }

    #[test]
    fn finds_dev_server_urls() {
        assert_eq!(
            local_url("  \u{1b}[32m➜\u{1b}[39m  Local:   http://localhost:5173/").as_deref(),
            Some("http://localhost:5173/")
        );
        assert_eq!(
            local_url("Listening on http://0.0.0.0:3000").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(local_url("see https://example.com:443/x"), None);
        assert_eq!(local_url("no url here"), None);
    }
}
