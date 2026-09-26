//! Integrated terminal: real shells over a websocket, for the web UI's
//! terminal panel.
//!
//! `GET  /ws/terminal?id=&cols=&rows=` — attach to terminal `id`, or
//!       spawn a new shell in the session cwd when `id` is absent or
//!       unknown. Binary frames carry keystrokes (client → server) and
//!       output (server → client); text frames carry JSON control
//!       messages: `{"type":"resize","cols","rows"}` from the client,
//!       `{"type":"ready","id"}` and `{"type":"exit","code"}` from the
//!       server.
//! `GET    /api/terminals`     — list live terminals.
//! `DELETE /api/terminals/:id` — kill one.
//!
//! Shells outlive the websocket, so a page reload reattaches and replays
//! the recent output. A terminal is arbitrary code execution for whoever
//! can reach the server, so it's only enabled when the server is bound
//! to a loopback address, or when `MIRA_TERMINAL=1` opts in.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::warn;

use crate::state::AppState;

/// Output kept per terminal for replay on reattach.
const SCROLLBACK_BYTES: usize = 256 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Decide once, at startup, whether terminals are allowed.
pub fn configure(bind: std::net::SocketAddr) {
    let opt_in = std::env::var("MIRA_TERMINAL").is_ok_and(|v| v == "1" || v == "true");
    ENABLED.store(bind.ip().is_loopback() || opt_in, Ordering::Relaxed);
}

fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn disabled() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": "terminal is disabled when the server listens beyond localhost \
                      (set MIRA_TERMINAL=1 to allow it)"
        })),
    )
        .into_response()
}

struct Terminal {
    cwd: PathBuf,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    scrollback: Mutex<VecDeque<u8>>,
    output: broadcast::Sender<Vec<u8>>,
    exited: AtomicBool,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Terminal>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<Terminal>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn spawn(id: &str, cwd: PathBuf, cols: u16, rows: u16) -> anyhow::Result<Arc<Terminal>> {
    let pair = native_pty_system().openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::new_default_prog();
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let (output, _) = broadcast::channel(256);
    let term = Arc::new(Terminal {
        cwd,
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        child: Mutex::new(child),
        scrollback: Mutex::new(VecDeque::new()),
        output,
        exited: AtomicBool::new(false),
    });

    // Blocking PTY reads live on their own thread; output fans out to
    // every attached socket and into the replay buffer.
    let t = term.clone();
    let id = id.to_owned();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    {
                        let mut sb = t.scrollback.lock().unwrap();
                        sb.extend(&chunk);
                        let over = sb.len().saturating_sub(SCROLLBACK_BYTES);
                        sb.drain(..over);
                    }
                    let _ = t.output.send(chunk);
                }
            }
        }
        t.exited.store(true, Ordering::Relaxed);
        let _ = t.output.send(Vec::new()); // empty chunk = exited
        registry().lock().unwrap().remove(&id);
    });
    Ok(term)
}

#[derive(Deserialize)]
pub struct AttachQuery {
    id: Option<String>,
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
}

fn default_cols() -> u16 {
    80
}
fn default_rows() -> u16 {
    24
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { cols: u16, rows: u16 },
}

/// GET /ws/terminal
pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<AttachQuery>,
) -> Response {
    if !enabled() {
        return disabled();
    }
    let existing =
        q.id.as_ref()
            .and_then(|id| registry().lock().unwrap().get(id).cloned());
    let (id, term) = match existing {
        Some(t) => (q.id.clone().unwrap(), t),
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let cwd = state.current_cwd().await;
            match spawn(&id, cwd, q.cols.max(2), q.rows.max(2)) {
                Ok(t) => {
                    registry().lock().unwrap().insert(id.clone(), t.clone());
                    (id, t)
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("spawn shell: {e}"),
                    )
                        .into_response()
                }
            }
        }
    };
    ws.on_upgrade(move |socket| pump(socket, id, term))
}

async fn pump(socket: WebSocket, id: String, term: Arc<Terminal>) {
    let (mut tx, mut rx) = socket.split();
    // Subscribe before snapshotting scrollback so nothing falls between.
    let mut output = term.output.subscribe();
    let replay: Vec<u8> = term.scrollback.lock().unwrap().iter().copied().collect();
    let ready = serde_json::json!({ "type": "ready", "id": id }).to_string();
    if tx.send(Message::Text(ready)).await.is_err() {
        return;
    }
    if !replay.is_empty() && tx.send(Message::Binary(replay)).await.is_err() {
        return;
    }

    let out_term = term.clone();
    let mut send_task = tokio::spawn(async move {
        loop {
            match output.recv().await {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => {
                    if tx.send(Message::Binary(chunk)).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        if out_term.exited.load(Ordering::Relaxed) {
            let code = out_term
                .child
                .lock()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .map(|s| s.exit_code());
            let msg = serde_json::json!({ "type": "exit", "code": code }).to_string();
            let _ = tx.send(Message::Text(msg)).await;
        }
    });

    let in_term = term.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                Message::Binary(bytes) => {
                    let mut w = in_term.writer.lock().unwrap();
                    if w.write_all(&bytes).and_then(|_| w.flush()).is_err() {
                        break;
                    }
                }
                Message::Text(text) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Resize { cols, rows }) => {
                        let _ = in_term.master.lock().unwrap().resize(PtySize {
                            rows: rows.max(2),
                            cols: cols.max(2),
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    Err(e) => warn!(?e, "terminal: bad control message"),
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Either side ending closes the socket; the shell keeps running.
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// GET /api/terminals
pub async fn list() -> Response {
    if !enabled() {
        return disabled();
    }
    let terms: Vec<_> = registry()
        .lock()
        .unwrap()
        .iter()
        .map(|(id, t)| serde_json::json!({ "id": id, "cwd": t.cwd }))
        .collect();
    Json(serde_json::json!({ "terminals": terms })).into_response()
}

/// DELETE /api/terminals/:id
pub async fn kill(Path(id): Path<String>) -> Response {
    if !enabled() {
        return disabled();
    }
    match registry().lock().unwrap().remove(&id) {
        Some(t) => {
            let _ = t.child.lock().unwrap().kill();
            StatusCode::NO_CONTENT.into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn shell_runs_commands_and_keeps_output_for_replay() {
        let dir = std::env::temp_dir();
        let term = spawn("test", dir, 80, 24).unwrap();
        term.writer
            .lock()
            .unwrap()
            .write_all(b"echo mira-terminal-ok\nexit\n")
            .unwrap();
        for _ in 0..100 {
            if term.exited.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let out: Vec<u8> = term.scrollback.lock().unwrap().iter().copied().collect();
        assert!(String::from_utf8_lossy(&out).contains("mira-terminal-ok"));
        assert!(term.exited.load(Ordering::Relaxed));
    }
}
