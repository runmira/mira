//! `mira -p "task"`: run one task without a UI and exit.
//!
//! Made for scripts, CI and other programs. Tools follow the normal
//! permission rules; anything that would ask a person is denied instead
//! (widen with `--mode` or `--allow`). Output goes to stdout in one of
//! three formats, and the exit code says whether the task finished:
//!
//! - `text` (default): the final answer only.
//! - `json`: one result object when the task ends.
//! - `stream-json`: one JSON object per line as things happen, ending
//!   with the same result object.
//!
//! Exit codes: 0 finished, 1 the model or provider failed, 2 bad usage.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use mira_core::ToolCall;
use mira_harness::{is_fatal_warning, Approver, HarnessEvent, Session, UsageTotals};
use mira_policy::Decision;
use serde_json::{json, Value};

#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    StreamJson,
}

/// Allows what the policy allows and denies everything that would need
/// a person, remembering each denial for the result.
#[derive(Default)]
pub struct HeadlessApprover {
    denied: Mutex<Vec<Value>>,
}

#[async_trait]
impl Approver for HeadlessApprover {
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool {
        if decision == Decision::Allow {
            return true;
        }
        if let Ok(mut d) = self.denied.lock() {
            d.push(json!({
                "tool": call.function.name,
                "input": serde_json::from_str::<Value>(&call.function.arguments)
                    .unwrap_or_else(|_| Value::String(call.function.arguments.clone())),
            }));
        }
        false
    }
}

impl HeadlessApprover {
    fn denials(&self) -> Vec<Value> {
        self.denied.lock().map(|d| d.clone()).unwrap_or_default()
    }
}

/// The prompt from `-p`, with piped stdin added as context: `cat log |
/// mira -p "why did this fail?"`. With no `-p` text, stdin is the prompt.
pub fn prompt(arg: &str, stdin: Option<String>) -> Option<String> {
    let arg = arg.trim();
    let stdin = stdin.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    match (arg.is_empty(), stdin) {
        (true, None) => None,
        (true, Some(s)) => Some(s),
        (false, None) => Some(arg.to_owned()),
        (false, Some(s)) => Some(format!("{arg}\n\n{s}")),
    }
}

/// Read piped stdin. With `-p` text, stdin is optional: if nothing
/// arrives within `grace`, it's ignored, so a caller that leaves stdin
/// open (some CI steps, `subprocess` without `stdin=DEVNULL`) doesn't
/// hang. Once input starts, it's read to the end.
pub fn read_stdin<R: std::io::Read + Send + 'static>(
    mut input: R,
    grace: Option<std::time::Duration>,
) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let first = match grace {
        Some(wait) => rx.recv_timeout(wait).ok()?,
        None => rx.recv().ok()?,
    };
    let mut all = first;
    for chunk in rx {
        all.extend(chunk);
    }
    Some(String::from_utf8_lossy(&all).into_owned())
}

pub struct Run<'a> {
    pub session: Session,
    pub approver: Arc<HeadlessApprover>,
    pub format: OutputFormat,
    pub model: &'a str,
    pub cwd: &'a std::path::Path,
    pub tools: Vec<String>,
}

/// Run the task; returns the process exit code.
pub async fn run(r: Run<'_>, prompt: &str) -> anyhow::Result<i32> {
    let started = Instant::now();
    let mut out = Out::new(r.format);
    out.event(json!({
        "type": "system",
        "subtype": "init",
        "session_id": r.session.id.to_string(),
        "model": r.model,
        "cwd": r.cwd,
        "tools": r.tools,
    }));

    // Text of the model's latest round: the answer once the task ends.
    let mut last_round = String::new();
    let mut fresh_round = false;
    let mut turns = 0u32;
    let mut failure: Option<String> = None;
    let mut totals = UsageTotals::default();

    let mut events = r.session.send(prompt).await;
    while let Some(evt) = events.next().await {
        match evt {
            HarnessEvent::Token(t) => {
                if fresh_round {
                    last_round.clear();
                    fresh_round = false;
                }
                last_round.push_str(&t);
                out.event(json!({"type": "text", "text": t}));
            }
            HarnessEvent::ToolStart(call) => {
                out.event(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.function.name,
                    "input": serde_json::from_str::<Value>(&call.function.arguments)
                        .unwrap_or(Value::String(call.function.arguments.clone())),
                }));
            }
            HarnessEvent::ToolEnd(res) => {
                out.event(json!({
                    "type": "tool_result",
                    "id": res.call_id,
                    "is_error": res.is_error,
                    "content": res.content,
                }));
            }
            HarnessEvent::TurnComplete => {
                turns += 1;
                fresh_round = true;
            }
            HarnessEvent::Warning(w) => {
                if is_fatal_warning(&w) {
                    failure = Some(w.clone());
                }
                out.warn(&w);
            }
            HarnessEvent::Usage { totals: t, .. } => totals = t,
            HarnessEvent::Compacted { messages_removed } => {
                out.event(json!({"type": "compacted", "messages_removed": messages_removed}));
            }
            HarnessEvent::Done => break,
            // Live tool output, diff previews, memory and goal frames
            // don't change the result.
            _ => {}
        }
    }

    let denials = r.approver.denials();
    let is_error = failure.is_some();
    let result = json!({
        "type": "result",
        "subtype": if is_error { "error" } else { "success" },
        "is_error": is_error,
        "error": failure,
        "result": last_round.trim(),
        "session_id": r.session.id.to_string(),
        "num_turns": turns,
        "duration_ms": started.elapsed().as_millis() as u64,
        "usage": {
            "input_tokens": totals.prompt_tokens,
            "output_tokens": totals.completion_tokens,
            "cached_input_tokens": totals.cached_input_tokens,
        },
        "total_cost_usd": mira_ai::cost_usd(r.model, totals.as_token_usage()),
        "permission_denials": denials,
    });
    out.finish(result, last_round.trim());
    Ok(if is_error { 1 } else { 0 })
}

/// Writes events in the chosen format. Text mode prints only the answer
/// (and failures to stderr); stream-json prints every event.
struct Out {
    format: OutputFormat,
    stdout: std::io::Stdout,
}

impl Out {
    fn new(format: OutputFormat) -> Self {
        Self {
            format,
            stdout: std::io::stdout(),
        }
    }

    fn line(&mut self, v: &Value) {
        let mut lock = self.stdout.lock();
        let _ = writeln!(lock, "{v}");
        let _ = lock.flush();
    }

    fn event(&mut self, v: Value) {
        if self.format == OutputFormat::StreamJson {
            self.line(&v);
        }
    }

    fn warn(&mut self, w: &str) {
        match self.format {
            OutputFormat::StreamJson => self.line(&json!({"type": "warning", "message": w})),
            _ => eprintln!("mira: {w}"),
        }
    }

    fn finish(&mut self, result: Value, text: &str) {
        match self.format {
            OutputFormat::Text => {
                if !text.is_empty() {
                    let mut lock = self.stdout.lock();
                    let _ = writeln!(lock, "{text}");
                }
            }
            OutputFormat::Json | OutputFormat::StreamJson => self.line(&result),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_combines_arg_and_stdin() {
        assert_eq!(prompt("", None), None);
        assert_eq!(prompt("  ", Some(" \n".into())), None);
        assert_eq!(prompt("fix it", None).as_deref(), Some("fix it"));
        assert_eq!(
            prompt("", Some("from stdin\n".into())).as_deref(),
            Some("from stdin")
        );
        assert_eq!(
            prompt("why?", Some("log line".into())).as_deref(),
            Some("why?\n\nlog line")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stdin_is_read_to_the_end_or_skipped_when_silent() {
        use std::time::Duration;
        let piped = std::io::Cursor::new(b"line one\nline two\n".to_vec());
        assert_eq!(
            read_stdin(piped, Some(Duration::from_millis(500))).as_deref(),
            Some("line one\nline two\n")
        );
        // An open pipe nobody writes to: skipped after the grace period.
        let (reader, _keep_open) = std::os::unix::net::UnixStream::pair().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(read_stdin(reader, Some(Duration::from_millis(100))), None);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn approver_allows_only_what_the_policy_allows() {
        let a = HeadlessApprover::default();
        let call = ToolCall {
            id: mira_core::ToolCallId::from("1"),
            kind: mira_core::message::ToolCallKind::Function,
            function: mira_core::message::ToolCallFunction {
                name: "bash".into(),
                arguments: r#"{"command":"rm -rf /"}"#.into(),
            },
        };
        assert!(a.approve(&call, Decision::Allow).await);
        assert!(!a.approve(&call, Decision::Ask).await);
        assert!(!a.approve(&call, Decision::Deny).await);
        let d = a.denials();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["tool"], "bash");
        assert_eq!(d[0]["input"]["command"], "rm -rf /");
    }
}
