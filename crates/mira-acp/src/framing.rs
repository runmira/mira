//! Newline-delimited JSON-RPC framing over a child process's stdio.
//!
//! The spec's rules, all of which are easy to get subtly wrong:
//!
//! * One message per line, delimited by `\n`, and messages **must not**
//!   contain embedded newlines. That means we serialize whole and split on
//!   lines — never stream a partial line into a parser.
//! * Notifications are the same shape as requests minus the `id` member.
//!   There is no separate channel, so a reader has to classify by id
//!   presence.
//! * **Id spaces are shared per connection, not per direction.** An agent
//!   may issue a request with id 5 while our own request 5 is still
//!   outstanding, so both directions have to be demultiplexed by id without
//!   colliding. This is the single most common way to deadlock a
//!   bidirectional JSON-RPC client.
//! * stdout carries *only* protocol. Anything an agent logs belongs on
//!   stderr, which we keep in a bounded ring for diagnostics rather than
//!   parsing.

use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Upper bound on a single line. A runaway agent that emits megabytes
/// without a newline would otherwise grow the buffer without limit.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// How much stderr to keep for diagnostics.
const STDERR_TAIL_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent wrote {0} bytes with no newline; refusing to buffer past the cap")]
    LineTooLong(usize),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// One classified inbound message.
#[derive(Clone, Debug, PartialEq)]
pub enum Inbound {
    /// Has an `id` and a `result`.
    Response { id: Value, result: Value },
    /// Has an `id` and an `error`.
    Error { id: Value, code: i64, message: String },
    /// Has an `id` and a `method` — the agent is calling *us*.
    Request { id: Value, method: String, params: Value },
    /// No `id` — an unsolicited update.
    Notification { method: String, params: Value },
}

impl Inbound {
    /// Classify a decoded JSON-RPC frame. Returns `None` for anything that
    /// isn't a well-formed frame — an array (batches are not part of v1),
    /// a bare value, or a frame missing the discriminator fields. Callers
    /// log-and-continue rather than failing, because real agents emit
    /// vendor extensions and occasional junk.
    pub fn classify(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        // A frame must name a method or carry an id; anything else is junk.
        let id = obj.get("id").cloned();
        let method = obj.get("method").and_then(Value::as_str).map(str::to_owned);

        if let Some(method) = method {
            let params = obj.get("params").cloned().unwrap_or(Value::Null);
            return Some(match id {
                Some(id) => Inbound::Request { id, method, params },
                None => Inbound::Notification { method, params },
            });
        }

        let id = id?;
        if let Some(err) = obj.get("error") {
            return Some(Inbound::Error {
                id,
                code: err.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string(),
            });
        }
        Some(Inbound::Response {
            id,
            result: obj.get("result").cloned().unwrap_or(Value::Null),
        })
    }

    pub fn id(&self) -> Option<&Value> {
        match self {
            Inbound::Response { id, .. } | Inbound::Error { id, .. } | Inbound::Request { id, .. } => {
                Some(id)
            }
            Inbound::Notification { .. } => None,
        }
    }
}

/// Serialize one outbound frame, enforcing the no-embedded-newline rule.
pub fn encode_frame(v: &Value) -> Result<String, FrameError> {
    let s = serde_json::to_string(v)?;
    if s.contains('\n') {
        // serde_json escapes newlines inside strings, so this should be
        // unreachable; treat it as a bug rather than shipping a frame the
        // agent would mis-parse.
        return Err(FrameError::Json(serde_json::Error::io(
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame contained an embedded newline",
            ),
        )));
    }
    Ok(format!("{s}\n"))
}

/// Bounded stderr tail, for "why did the agent die?" without unbounded
/// memory growth.
#[derive(Default)]
pub struct StderrTail {
    buf: VecDeque<u8>,
}

impl StderrTail {
    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend(chunk);
        while self.buf.len() > STDERR_TAIL_BYTES {
            self.buf.pop_front();
        }
    }

    pub fn as_str(&self) -> String {
        // `VecDeque::as_slices` can return two disjoint pieces once it wraps.
        let (a, b) = self.buf.as_slices();
        let mut v = Vec::with_capacity(a.len() + b.len());
        v.extend_from_slice(a);
        v.extend_from_slice(b);
        String::from_utf8_lossy(&v).into_owned()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// The agent's stdin, guarded by a mutex.
///
/// Writes from several tasks (a request plus an interleaved cancel
/// notification) must not interleave mid-line, so the handle is shared
/// behind a lock and every write is a whole frame.
pub struct Stdin {
    inner: Mutex<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>,
}

impl Stdin {
    pub fn new(w: Box<dyn tokio::io::AsyncWrite + Send + Unpin>) -> Self {
        Self {
            inner: Mutex::new(w),
        }
    }

    /// Write one already-encoded frame. Serializes the whole thing under
    /// the lock, so concurrent senders can't split a line.
    pub async fn write_frame(&self, frame: &str) -> Result<(), FrameError> {
        let mut guard = self.inner.lock().await;
        guard.write_all(frame.as_bytes()).await?;
        guard.flush().await?;
        Ok(())
    }

    /// Convenience: encode a value and write it.
    pub async fn send(&self, v: &Value) -> Result<(), FrameError> {
        let frame = encode_frame(v)?;
        self.write_frame(&frame).await
    }
}

/// Read lines from the agent's stdout, handing each classified frame to
/// `on_message`.
///
/// Runs until stdout closes. Never parses stderr — that is a log channel.
pub async fn pump_stdout<R, F>(
    mut reader: BufReader<R>,
    mut on_message: F,
) -> Result<(), FrameError>
where
    R: tokio::io::AsyncRead + Unpin,
    F: FnMut(Inbound),
{
    let mut owned = String::new();
    loop {
        owned.clear();
        let n = read_line_capped(&mut reader, &mut owned).await?;
        if n == 0 {
            return Ok(());
        }
        let trimmed = owned.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => {
                if let Some(frame) = Inbound::classify(&v) {
                    on_message(frame);
                } else {
                    // Batches, bare values, and malformed frames land here.
                    // A v1 agent shouldn't send any of them, but a hard error
                    // would kill a session over one stray line.
                    tracing::debug!(
                        line = %trimmed.chars().take(200).collect::<String>(),
                        "acp: ignoring unclassifiable frame"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    line = %trimmed.chars().take(200).collect::<String>(),
                    "acp: ignoring non-JSON stdout line"
                );
            }
        }
    }
}

/// Reads one newline-terminated frame into `out`, refusing to grow past
/// `MAX_LINE_BYTES`.
///
/// Hand-rolled rather than using `AsyncBufReadExt::lines`, because `lines()`
/// has no size cap and a runaway agent that emits megabytes without a
/// newline would grow the buffer until it took the process down.
async fn read_line_capped<R>(
    reader: &mut BufReader<R>,
    out: &mut String,
) -> Result<usize, FrameError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut total = 0usize;
    loop {
        let available = match reader.fill_buf().await {
            Ok(b) => b,
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if available.is_empty() {
            break;
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(idx) => {
                out.push_str(&String::from_utf8_lossy(&available[..idx]));
                let read = idx + 1;
                reader.consume(read);
                total += read;
                return Ok(total.max(1));
            }
            None => {
                let len = available.len();
                out.push_str(&String::from_utf8_lossy(available));
                reader.consume(len);
                total += len;
                if out.len() > MAX_LINE_BYTES {
                    return Err(FrameError::LineTooLong(out.len()));
                }
            }
        }
    }
    Ok(total)
}

/// Shared handle types for a live agent process.
pub type SharedStdin = Arc<Stdin>;
pub type SharedStderr = Arc<Mutex<StderrTail>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Value {
        v
    }

    #[test]
    fn a_response_is_classified_by_id_and_result() {
        let f = Inbound::classify(&obj(serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "result": {"stopReason": "end_turn"}
        })))
        .unwrap();
        assert_eq!(f.id(), Some(&serde_json::json!(4)));
        assert!(matches!(f, Inbound::Response { .. }));
    }

    #[test]
    fn a_request_from_the_agent_is_not_mistaken_for_a_response() {
        // Same `id` shape, but a `method` means the agent is calling us.
        let f = Inbound::classify(&obj(serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "fs/read_text_file",
            "params": {"path": "/x"}
        })))
        .unwrap();
        match f {
            Inbound::Request { method, .. } => assert_eq!(method, "fs/read_text_file"),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[test]
    fn a_notification_is_identified_by_absence_of_an_id() {
        let f = Inbound::classify(&obj(serde_json::json!({
            "jsonrpc": "2.0", "method": "session/update", "params": {}
        })))
        .unwrap();
        assert!(matches!(f, Inbound::Notification { .. }));
        assert_eq!(f.id(), None);
    }

    #[test]
    fn a_null_id_is_still_a_request_not_a_notification() {
        // The spec discourages null ids but doesn't forbid them, and
        // classifying one as a notification would strand a pending response.
        let f = Inbound::classify(&obj(serde_json::json!({
            "jsonrpc": "2.0", "id": null, "method": "ping"
        })))
        .unwrap();
        assert!(matches!(f, Inbound::Request { .. }));
    }

    #[test]
    fn errors_carry_code_and_message() {
        let f = Inbound::classify(&obj(serde_json::json!({
            "jsonrpc": "2.0", "id": 2,
            "error": {"code": -32000, "message": "Authentication required"}
        })))
        .unwrap();
        match f {
            Inbound::Error { code, message, .. } => {
                assert_eq!(code, -32000);
                assert_eq!(message, "Authentication required");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn junk_is_rejected_rather_than_crashing_the_pump() {
        // Batches aren't part of v1, and a bare value isn't a frame.
        assert!(Inbound::classify(&serde_json::json!([{"jsonrpc": "2.0"}])).is_none());
        assert!(Inbound::classify(&serde_json::json!("hello")).is_none());
        assert!(Inbound::classify(&serde_json::json!({"jsonrpc": "2.0"})).is_none());
    }

    #[test]
    fn encoding_terminates_with_exactly_one_newline() {
        let v = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        let s = encode_frame(&v).unwrap();
        assert!(s.ends_with('\n'));
        assert_eq!(s.matches('\n').count(), 1);
    }

    #[test]
    fn embedded_newlines_in_strings_are_escaped_not_emitted_raw() {
        // A multi-line message body is routine; the frame must stay one line.
        let v = serde_json::json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": {"text": "line one\nline two\n"}
        });
        let s = encode_frame(&v).unwrap();
        assert_eq!(s.matches('\n').count(), 1, "frame broke across lines: {s:?}");
        // And it round-trips.
        let back: Value = serde_json::from_str(s.trim_end()).unwrap();
        assert_eq!(back["params"]["text"], "line one\nline two\n");
    }

    #[tokio::test]
    async fn the_pump_handles_multiple_frames_on_one_stream() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"fs/write_text_file\",\"params\":{}}\n",
        );
        let reader = BufReader::new(std::io::Cursor::new(input.as_bytes()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        pump_stdout(reader, move |f| {
            sink.lock().expect("sink poisoned").push(f);
        })
        .await
        .unwrap();
        let seen = seen.lock().expect("sink poisoned");
        assert_eq!(seen.len(), 3);
        assert!(matches!(seen[0], Inbound::Response { .. }));
        assert!(matches!(seen[1], Inbound::Notification { .. }));
        assert!(matches!(seen[2], Inbound::Request { .. }));
    }

    #[tokio::test]
    async fn the_pump_skips_junk_without_dropping_later_frames() {
        // One bad line must not cost us the rest of the stream — this is the
        // whole reason the pump classifies rather than erroring.
        let input = concat!(
            "not json at all\n",
            "[1,2,3]\n",
            "{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"ok\":true}}\n",
        );
        let reader = BufReader::new(std::io::Cursor::new(input.as_bytes()));
        let count = Arc::new(std::sync::Mutex::new(0usize));
        let sink = count.clone();
        pump_stdout(reader, move |_| {
            *sink.lock().expect("sink poisoned") += 1;
        })
        .await
        .unwrap();
        assert_eq!(*count.lock().expect("sink poisoned"), 1);
    }

    #[tokio::test]
    async fn a_line_with_no_trailing_newline_still_yields_its_frame() {
        // Agents do exit without a final newline; dropping the last frame
        // would lose the turn's stop reason.
        let input = "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"stopReason\":\"cancelled\"}}";
        let reader = BufReader::new(std::io::Cursor::new(input.as_bytes()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        pump_stdout(reader, move |f| sink.lock().expect("sink poisoned").push(f))
            .await
            .unwrap();
        assert_eq!(seen.lock().expect("sink poisoned").len(), 1);
    }

    #[test]
    fn the_stderr_tail_is_bounded_and_keeps_the_most_recent_bytes() {
        let mut t = StderrTail::default();
        t.push(b"old junk");
        t.push(&vec![b'x'; STDERR_TAIL_BYTES * 2]);
        let s = t.as_str();
        assert!(s.len() <= STDERR_TAIL_BYTES + 8);
        assert!(!s.contains("old junk"), "tail should have evicted old output");
    }
}
