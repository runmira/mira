//! Background-process tools: spawn a shell command without blocking the model,
//! stream its output live, and optionally wait for or kill it later.
//!
//! Three tools:
//!   - `run_background`  — spawn via `bash -lc`, return immediately with an id.
//!   - `read_output`     — return buffered output + running/exit status.
//!   - `kill_background` — stop the process, return final output.
//!
//! Output lines stream live as `ToolProgress` frames via the session-lifetime
//! progress sink so the UI updates continuously without the model polling.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::context::{ToolContext, ToolProgressSink};
use crate::tool::{spec, Action, Tool, ToolError};

// Maximum lines kept in the ring buffer per process.
const MAX_LINES: usize = 1_000;
// Maximum lines returned in a single `read_output` call.
const MAX_RETURN: usize = 200;

// ─── Process registry ────────────────────────────────────────────────────────

pub struct ProcessEntry {
    pub id: u32,
    pub command: String,
    /// The `run_background` tool-call id — `ToolProgress` frames are keyed on
    /// this so the frontend routes them to the right card.
    pub call_id: String,
    pub cwd: PathBuf,
    pub started_at: Instant,
    pub lines: Arc<Mutex<VecDeque<String>>>,
    /// `None` while the process is still running.
    pub exit_code: Arc<Mutex<Option<i32>>>,
    /// Fires the process kill signal.
    pub cancel: CancellationToken,
}

#[derive(Default)]
pub struct BackgroundProcessStore {
    processes: Mutex<HashMap<u32, Arc<ProcessEntry>>>,
    next_id: AtomicU32,
}

impl BackgroundProcessStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            processes: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        })
    }

    fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn insert(&self, entry: ProcessEntry) {
        let id = entry.id;
        self.processes.lock().await.insert(id, Arc::new(entry));
    }

    pub async fn get(&self, id: u32) -> Option<Arc<ProcessEntry>> {
        self.processes.lock().await.get(&id).cloned()
    }
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

fn require_store(ctx: &ToolContext) -> Result<Arc<BackgroundProcessStore>, ToolError> {
    ctx.bg_processes
        .clone()
        .ok_or_else(|| ToolError::Failed("background process store not available".into()))
}

async fn snapshot_lines(buf: &Arc<Mutex<VecDeque<String>>>, tail: Option<usize>) -> Vec<String> {
    let guard = buf.lock().await;
    let limit = tail.unwrap_or(MAX_RETURN).min(MAX_RETURN);
    if guard.len() <= limit {
        guard.iter().cloned().collect()
    } else {
        guard
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

fn fmt_elapsed(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

/// Spawn a Tokio task that reads `reader` line-by-line and feeds each line
/// into `buf` + emits it via `sink`.
fn spawn_drain(
    reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    buf: Arc<Mutex<VecDeque<String>>>,
    sink: Option<Arc<dyn ToolProgressSink>>,
    call_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(s) = &sink {
                s.emit(&call_id, &line);
            }
            let mut g = buf.lock().await;
            if g.len() >= MAX_LINES {
                g.pop_front();
            }
            g.push_back(line);
        }
    })
}

// ─── run_background ──────────────────────────────────────────────────────────

pub struct RunBackground;

#[derive(Deserialize)]
struct RunArgs {
    command: String,
}

#[async_trait]
impl Tool for RunBackground {
    fn spec(&self) -> ToolSpec {
        spec(
            "run_background",
            "Spawn a shell command in the background without blocking the model. \
             Returns immediately with a numeric `id`. Output streams live and is \
             buffered; use `read_output` to inspect it and `kill_background` to \
             stop it. Ideal for dev servers, build watchers, or long compiles that \
             the model needs to start but not wait for.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run (executed via bash -lc)."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: RunArgs = call.parse_arguments()?;
        let store = require_store(ctx)?;

        let id = store.alloc_id();
        let lines: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let exit_code: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
        let cancel = CancellationToken::new();
        let call_id_str = call.id.to_string();

        store
            .insert(ProcessEntry {
                id,
                command: args.command.clone(),
                call_id: call_id_str.clone(),
                cwd: ctx.cwd.clone(),
                started_at: Instant::now(),
                lines: lines.clone(),
                exit_code: exit_code.clone(),
                cancel: cancel.clone(),
            })
            .await;

        let sink: Option<Arc<dyn ToolProgressSink>> = ctx.bg_progress.clone();
        let call_id = call_id_str;
        let command_str = args.command.clone();
        let cwd = ctx.cwd.clone();

        tokio::spawn(async move {
            let mut child = match Command::new("bash")
                .args(["-lc", &command_str])
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("(failed to spawn: {e})");
                    if let Some(s) = &sink {
                        s.emit(&call_id, &msg);
                    }
                    lines.lock().await.push_back(msg);
                    *exit_code.lock().await = Some(1);
                    return;
                }
            };

            let h_out = child
                .stdout
                .take()
                .map(|r| spawn_drain(r, lines.clone(), sink.clone(), call_id.clone()));
            let h_err = child
                .stderr
                .take()
                .map(|r| spawn_drain(r, lines.clone(), sink.clone(), call_id.clone()));

            // Wait for both pipes to drain OR a kill signal.
            let join_drains = async move {
                if let Some(h) = h_out {
                    let _ = h.await;
                }
                if let Some(h) = h_err {
                    let _ = h.await;
                }
            };

            tokio::select! {
                _ = join_drains => {}
                _ = cancel.cancelled() => {
                    let _ = child.kill().await;
                }
            }

            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
            *exit_code.lock().await = Some(code);
        });

        Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "id={id}\ncommand={cmd}\nProcess started. Output streams live. Use read_output({id}) to inspect or kill_background({id}) to stop.",
                cmd = args.command,
            ),
        ))
    }
}

// ─── read_output ─────────────────────────────────────────────────────────────

pub struct ReadOutput;

#[derive(Deserialize)]
struct ReadArgs {
    id: u32,
    #[serde(default)]
    tail: Option<usize>,
}

#[async_trait]
impl Tool for ReadOutput {
    fn spec(&self) -> ToolSpec {
        spec(
            "read_output",
            "Read buffered output from a background process started with \
             `run_background`. Shows whether it is still running or has exited. \
             Use `tail` to limit how many lines to return (max 200).",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Process id returned by run_background."
                    },
                    "tail": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Return only the last N lines. Default: all (max 200)."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ReadArgs = call.parse_arguments()?;
        let store = require_store(ctx)?;

        let entry = store
            .get(args.id)
            .await
            .ok_or_else(|| ToolError::Failed(format!("no background process id={}", args.id)))?;

        let lines = snapshot_lines(&entry.lines, args.tail).await;
        let exit_code = *entry.exit_code.lock().await;
        let running = exit_code.is_none();
        let elapsed = fmt_elapsed(entry.started_at.elapsed());

        let mut body = format!(
            "id={id}\nrunning={running}\nelapsed={elapsed}\n",
            id = entry.id,
        );
        if let Some(code) = exit_code {
            body.push_str(&format!("exit_code={code}\n"));
        }
        body.push_str(&format!("--- output ({} lines) ---\n", lines.len()));
        for line in &lines {
            body.push_str(line);
            body.push('\n');
        }

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

// ─── kill_background ─────────────────────────────────────────────────────────

pub struct KillBackground;

#[derive(Deserialize)]
struct KillArgs {
    id: u32,
}

#[async_trait]
impl Tool for KillBackground {
    fn spec(&self) -> ToolSpec {
        spec(
            "kill_background",
            "Stop a background process and return its final output. Safe to call \
             even if the process already exited — returns the buffered output either way.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Process id returned by run_background."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: KillArgs = call.parse_arguments()?;
        let store = require_store(ctx)?;

        let entry = store
            .get(args.id)
            .await
            .ok_or_else(|| ToolError::Failed(format!("no background process id={}", args.id)))?;

        let was_running = entry.exit_code.lock().await.is_none();
        if was_running {
            entry.cancel.cancel();
            // Brief wait for the drain tasks to flush any final output.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        let lines = snapshot_lines(&entry.lines, None).await;
        let exit_code = *entry.exit_code.lock().await;
        let elapsed = fmt_elapsed(entry.started_at.elapsed());

        let mut body = format!(
            "id={id}\nkilled={was_running}\nelapsed={elapsed}\n",
            id = entry.id,
        );
        if let Some(code) = exit_code {
            body.push_str(&format!("exit_code={code}\n"));
        }
        body.push_str(&format!("--- output ({} lines) ---\n", lines.len()));
        for line in &lines {
            body.push_str(line);
            body.push('\n');
        }

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
