//! E2B backend: tools run in an E2B Firecracker microVM.
//!
//! E2B has no Rust SDK, so this talks to its two HTTP surfaces directly:
//!
//! - **Control plane** (`https://api.e2b.app`, `X-API-Key`): create,
//!   pause, resume, kill a sandbox, extend its timeout.
//! - **envd**, the agent inside the sandbox (`https://49983-<id>.e2b.app`):
//!   `GET`/`POST /files` for file I/O, and the Connect-RPC
//!   `process.Process/Start` server stream for commands, whose stdout and
//!   stderr chunks feed the TUI's live tool tail.
//!
//! Written against E2B's public API and SDK behavior; the tests exercise
//! it against a local fake of both surfaces, not the live service.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use bytes::{Buf, BytesMut};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::{path, ComputeBackend, ComputeError, ComputeEvent, ExecOutput, ExecRequest, Result};

/// Port envd listens on inside every sandbox.
const ENVD_PORT: u16 = 49983;
/// Where the project lives inside the sandbox.
pub const WORKSPACE_ROOT: &str = "/home/user/workspace";
/// Extend the sandbox's lifetime at most this often.
const KEEPALIVE_EVERY: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct E2bOptions {
    pub api_key: String,
    /// Control-plane base URL.
    pub api_url: String,
    /// Sandbox domain; envd lives at `https://49983-<id>.<domain>`.
    pub domain: String,
    /// Sandbox template (image). `base` has bash, git, tar, Python, Node.
    pub template: String,
    /// Idle lifetime; refreshed while the session is active.
    pub timeout_secs: u64,
    /// Linux user commands run as.
    pub user: String,
    /// Override the envd URL (tests, self-hosted E2B).
    pub envd_url: Option<String>,
}

impl E2bOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: "https://api.e2b.app".into(),
            domain: "e2b.app".into(),
            template: "base".into(),
            timeout_secs: 3600,
            user: "user".into(),
            envd_url: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    #[serde(rename = "sandboxID")]
    sandbox_id: String,
    #[serde(default)]
    envd_access_token: Option<String>,
    #[serde(default)]
    domain: Option<String>,
}

pub struct E2bBackend {
    http: reqwest::Client,
    opts: E2bOptions,
    sandbox_id: String,
    envd_url: String,
    access_token: Option<String>,
    last_keepalive: Mutex<Instant>,
    closed: AtomicBool,
}

fn http_err(what: &str, e: reqwest::Error) -> ComputeError {
    ComputeError::Remote(format!("e2b {what}: {e}"))
}

async fn check(what: &str, resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 => ComputeError::Config(format!(
            "e2b {what}: unauthorized ({body}) — check E2B_API_KEY"
        )),
        404 => ComputeError::NotFound(format!("e2b {what}: not found {body}")),
        _ => ComputeError::Remote(format!("e2b {what}: HTTP {status}: {body}")),
    })
}

/// Connect-protocol envelope: flags byte, big-endian length, payload.
pub(crate) fn envelope(flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(flags);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Split complete envelopes off the front of `buf`.
pub(crate) fn take_envelopes(buf: &mut BytesMut) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    while buf.len() >= 5 {
        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if buf.len() < 5 + len {
            break;
        }
        let flags = buf[0];
        buf.advance(5);
        out.push((flags, buf.split_to(len).to_vec()));
    }
    out
}

/// Flag bit marking the final (trailer) envelope of a stream.
const END_STREAM: u8 = 0x02;

/// Accumulates one stream's bytes and emits whole lines to the sink.
struct LineTail {
    all: Vec<u8>,
    pending: Vec<u8>,
}

impl LineTail {
    fn new() -> Self {
        Self {
            all: Vec::new(),
            pending: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.all.extend_from_slice(chunk);
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(i) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=i).collect();
            lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
        }
        lines
    }

    fn finish(self) -> (String, Option<String>) {
        let rest =
            (!self.pending.is_empty()).then(|| String::from_utf8_lossy(&self.pending).into_owned());
        (String::from_utf8_lossy(&self.all).into_owned(), rest)
    }
}

impl E2bBackend {
    fn build_http() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| http_err("client", e))
    }

    fn from_response(http: reqwest::Client, opts: E2bOptions, r: CreateResponse) -> Self {
        let domain = r.domain.unwrap_or_else(|| opts.domain.clone());
        let envd_url = opts
            .envd_url
            .clone()
            .unwrap_or_else(|| format!("https://{ENVD_PORT}-{}.{domain}", r.sandbox_id));
        Self {
            http,
            sandbox_id: r.sandbox_id,
            envd_url,
            access_token: r.envd_access_token,
            last_keepalive: Mutex::new(Instant::now()),
            closed: AtomicBool::new(false),
            opts,
        }
    }

    /// Start a fresh sandbox from `opts.template`.
    pub async fn create(opts: E2bOptions) -> Result<Self> {
        if opts.api_key.is_empty() {
            return Err(ComputeError::Config("E2B API key is empty".into()));
        }
        let http = Self::build_http()?;
        let resp = http
            .post(format!("{}/sandboxes", opts.api_url))
            .header("X-API-Key", &opts.api_key)
            .json(&json!({
                "templateID": opts.template,
                "timeout": opts.timeout_secs,
                "secure": true,
                "metadata": { "client": "mira" },
            }))
            .send()
            .await
            .map_err(|e| http_err("create", e))?;
        let r: CreateResponse = check("create", resp)
            .await?
            .json()
            .await
            .map_err(|e| http_err("create response", e))?;
        Ok(Self::from_response(http, opts, r))
    }

    /// Resume a sandbox paused by [`ComputeBackend::checkpoint`].
    pub async fn resume(opts: E2bOptions, sandbox_id: &str) -> Result<Self> {
        let http = Self::build_http()?;
        let resp = http
            .post(format!("{}/sandboxes/{sandbox_id}/resume", opts.api_url))
            .header("X-API-Key", &opts.api_key)
            .json(&json!({ "timeout": opts.timeout_secs }))
            .send()
            .await
            .map_err(|e| http_err("resume", e))?;
        let r: CreateResponse = check("resume", resp)
            .await?
            .json()
            .await
            .map_err(|e| http_err("resume response", e))?;
        Ok(Self::from_response(http, opts, r))
    }

    pub fn sandbox_id(&self) -> &str {
        &self.sandbox_id
    }

    fn envd(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let basic =
            base64::engine::general_purpose::STANDARD.encode(format!("{}:", self.opts.user));
        let req = req.header("Authorization", format!("Basic {basic}"));
        match &self.access_token {
            Some(t) => req.header("X-Access-Token", t),
            None => req,
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            Err(ComputeError::Remote("the E2B sandbox was shut down".into()))
        } else {
            Ok(())
        }
    }

    /// Push the sandbox's expiry out while the session is in use.
    async fn keepalive(&self) {
        let mut last = self.last_keepalive.lock().await;
        if last.elapsed() < KEEPALIVE_EVERY {
            return;
        }
        *last = Instant::now();
        let res = self
            .http
            .post(format!(
                "{}/sandboxes/{}/timeout",
                self.opts.api_url, self.sandbox_id
            ))
            .header("X-API-Key", &self.opts.api_key)
            .json(&json!({ "timeout": self.opts.timeout_secs }))
            .send()
            .await;
        if let Err(e) = res {
            tracing::warn!(%e, "e2b: timeout refresh failed");
        }
    }

    async fn kill_process(&self, pid: u64) {
        let res = self
            .envd(
                self.http
                    .post(format!("{}/process.Process/SendSignal", self.envd_url)),
            )
            .header("Content-Type", "application/json")
            .header("Connect-Protocol-Version", "1")
            .body(json!({ "process": { "pid": pid }, "signal": "SIGNAL_SIGKILL" }).to_string())
            .send()
            .await;
        if let Err(e) = res {
            tracing::warn!(%e, pid, "e2b: kill after timeout failed");
        }
    }
}

impl Drop for E2bBackend {
    /// Best effort: if the caller never called `shutdown`, kill the VM so
    /// it doesn't bill until its timeout. E2B reaps it at the timeout
    /// regardless.
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::Relaxed) {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let http = self.http.clone();
        let url = format!("{}/sandboxes/{}", self.opts.api_url, self.sandbox_id);
        let key = self.opts.api_key.clone();
        rt.spawn(async move {
            let _ = http.delete(url).header("X-API-Key", key).send().await;
        });
    }
}

#[async_trait]
impl ComputeBackend for E2bBackend {
    fn name(&self) -> &'static str {
        "e2b"
    }

    fn workspace_root(&self) -> String {
        WORKSPACE_ROOT.to_owned()
    }

    async fn exec(
        &self,
        req: ExecRequest,
        sink: Option<mpsc::Sender<ComputeEvent>>,
    ) -> Result<ExecOutput> {
        self.ensure_open()?;
        self.keepalive().await;
        let body = json!({
            "process": {
                "cmd": "/bin/bash",
                "args": ["-l", "-c", req.command],
                "envs": {},
                "cwd": path::join(WORKSPACE_ROOT, &req.cwd),
            }
        });
        let resp = self
            .envd(
                self.http
                    .post(format!("{}/process.Process/Start", self.envd_url)),
            )
            .header("Content-Type", "application/connect+json")
            .header("Connect-Protocol-Version", "1")
            .header("Keepalive-Ping-Interval", "50")
            .body(envelope(0, body.to_string().as_bytes()))
            .send()
            .await
            .map_err(|e| http_err("exec", e))?;
        let resp = check("exec", resp).await?;

        let mut stdout = LineTail::new();
        let mut stderr = LineTail::new();
        let mut exit_code = None;
        let mut pid = None;
        let mut stream_error = None;
        let mut buf = BytesMut::new();
        let mut chunks = resp.bytes_stream();
        let b64 = base64::engine::general_purpose::STANDARD;

        let read = async {
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.map_err(|e| http_err("exec stream", e))?;
                buf.extend_from_slice(&chunk);
                for (flags, payload) in take_envelopes(&mut buf) {
                    let v: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
                    if flags & END_STREAM != 0 {
                        if let Some(err) = v.get("error") {
                            stream_error = Some(
                                err.get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown error")
                                    .to_owned(),
                            );
                        }
                        continue;
                    }
                    let event = &v["event"];
                    if let Some(p) = event["start"]["pid"].as_u64() {
                        pid = Some(p);
                    }
                    let data = &event["data"];
                    for (field, tail, is_err) in [
                        ("stdout", &mut stdout, false),
                        ("stderr", &mut stderr, true),
                    ] {
                        let Some(s) = data[field].as_str() else {
                            continue;
                        };
                        let bytes = b64.decode(s).unwrap_or_default();
                        for line in tail.push(&bytes) {
                            if let Some(tx) = &sink {
                                let ev = if is_err {
                                    ComputeEvent::Stderr(line)
                                } else {
                                    ComputeEvent::Stdout(line)
                                };
                                let _ = tx.send(ev).await;
                            }
                        }
                    }
                    let end = &event["end"];
                    if !end.is_null() {
                        exit_code = Some(end["exitCode"].as_i64().unwrap_or(0) as i32);
                    }
                }
            }
            Ok::<_, ComputeError>(())
        };
        let timed_out = match tokio::time::timeout(req.timeout, read).await {
            Ok(res) => {
                res?;
                false
            }
            Err(_) => true,
        };
        if timed_out {
            if let Some(p) = pid {
                self.kill_process(p).await;
            }
            exit_code = None;
        } else if let (None, Some(msg)) = (exit_code, stream_error) {
            return Err(ComputeError::Remote(format!("e2b exec: {msg}")));
        }

        let flush = |tail: LineTail, is_err: bool| {
            let (all, rest) = tail.finish();
            (
                all,
                rest.map(|l| {
                    if is_err {
                        ComputeEvent::Stderr(l)
                    } else {
                        ComputeEvent::Stdout(l)
                    }
                }),
            )
        };
        let (stdout, rest_out) = flush(stdout, false);
        let (stderr, rest_err) = flush(stderr, true);
        if let Some(tx) = &sink {
            for ev in [rest_out, rest_err].into_iter().flatten() {
                let _ = tx.send(ev).await;
            }
        }
        Ok(ExecOutput {
            stdout,
            stderr,
            exit_code,
            timed_out,
        })
    }

    async fn read_file(&self, rel: &str) -> Result<Vec<u8>> {
        self.ensure_open()?;
        let abs = path::join(WORKSPACE_ROOT, rel);
        let resp = self
            .envd(self.http.get(format!("{}/files", self.envd_url)))
            .query(&[
                ("path", abs.as_str()),
                ("username", self.opts.user.as_str()),
            ])
            .send()
            .await
            .map_err(|e| http_err("read", e))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ComputeError::NotFound(format!("no such file: {rel}")));
        }
        let bytes = check("read", resp)
            .await?
            .bytes()
            .await
            .map_err(|e| http_err("read body", e))?;
        Ok(bytes.to_vec())
    }

    async fn write_file(&self, rel: &str, data: &[u8]) -> Result<()> {
        self.ensure_open()?;
        if rel.is_empty() {
            return Err(ComputeError::InvalidPath(
                "cannot write the workspace root".into(),
            ));
        }
        let abs = path::join(WORKSPACE_ROOT, rel);
        let name = rel.rsplit('/').next().unwrap_or(rel).to_owned();
        let form = reqwest::multipart::Form::new().part(
            "file",
            reqwest::multipart::Part::bytes(data.to_vec()).file_name(name),
        );
        // envd creates missing parent directories.
        let resp = self
            .envd(self.http.post(format!("{}/files", self.envd_url)))
            .query(&[
                ("path", abs.as_str()),
                ("username", self.opts.user.as_str()),
            ])
            .multipart(form)
            .send()
            .await
            .map_err(|e| http_err("write", e))?;
        check("write", resp).await?;
        Ok(())
    }

    /// Pause the VM (memory + filesystem are kept) and return the id to
    /// pass to [`E2bBackend::resume`]. The backend is unusable after.
    async fn checkpoint(&self) -> Result<String> {
        self.pause().await?;
        Ok(self.sandbox_id.clone())
    }

    async fn pause(&self) -> Result<()> {
        self.ensure_open()?;
        let resp = self
            .http
            .post(format!(
                "{}/sandboxes/{}/pause",
                self.opts.api_url, self.sandbox_id
            ))
            .header("X-API-Key", &self.opts.api_key)
            .send()
            .await
            .map_err(|e| http_err("pause", e))?;
        check("pause", resp).await?;
        // Paused, not killed: Drop must not delete it.
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        let resp = self
            .http
            .delete(format!(
                "{}/sandboxes/{}",
                self.opts.api_url, self.sandbox_id
            ))
            .header("X-API-Key", &self.opts.api_key)
            .send()
            .await
            .map_err(|e| http_err("kill", e))?;
        if resp.status() != reqwest::StatusCode::NOT_FOUND {
            check("kill", resp).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_round_trip_across_chunk_boundaries() {
        let a = envelope(0, br#"{"a":1}"#);
        let b = envelope(END_STREAM, br#"{}"#);
        let all: Vec<u8> = [a, b].concat();
        let mut buf = BytesMut::new();
        let mut got = Vec::new();
        for byte in all {
            buf.extend_from_slice(&[byte]);
            got.extend(take_envelopes(&mut buf));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (0, br#"{"a":1}"#.to_vec()));
        assert_eq!(got[1].0, END_STREAM);
        assert!(buf.is_empty());
    }

    #[test]
    fn line_tail_splits_partial_lines() {
        let mut t = LineTail::new();
        assert!(t.push(b"hel").is_empty());
        assert_eq!(t.push(b"lo\nwor"), vec!["hello".to_owned()]);
        let (all, rest) = t.finish();
        assert_eq!(all, "hello\nwor");
        assert_eq!(rest.as_deref(), Some("wor"));
    }
}
