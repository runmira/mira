//! Upload → work → diff → `git apply`, through each backend.
//!
//! The E2B case runs against an in-process fake of E2B's control plane
//! and envd (files + Connect `process.Process/Start` stream) that maps
//! the sandbox workspace onto a temp dir and runs commands locally.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use mira_compute::e2b::WORKSPACE_ROOT;
use mira_compute::{
    workspace, ComputeBackend, ComputeEvent, E2bBackend, E2bOptions, ExecRequest, LocalBackend,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

/// A small git project with one ignored file.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    std::fs::write(p.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(p.join(".env"), "SECRET=hunter2").unwrap();
    std::fs::create_dir(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/lib.rs"),
        "pub fn answer() -> u32 {\n    41\n}\n",
    )
    .unwrap();
    std::fs::write(p.join("README.md"), "hello\n").unwrap();
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    );
    dir
}

/// The workflow every backend must support.
async fn exercise(backend: &dyn ComputeBackend, local: &Path) {
    workspace::upload(backend, &workspace::pack(local).unwrap())
        .await
        .unwrap();

    // Ignored files never leave the machine.
    assert!(backend.read_file(".env").await.is_err());

    let src = String::from_utf8(backend.read_file("src/lib.rs").await.unwrap()).unwrap();
    backend
        .write_file("src/lib.rs", src.replace("41", "42").as_bytes())
        .await
        .unwrap();
    backend
        .write_file("docs/new/notes.md", b"new file\n")
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(64);
    let out = backend
        .exec(
            ExecRequest::new("rm README.md && echo removed && echo oops >&2 && pwd").cwd("src/.."),
            Some(tx),
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{out:?}");
    assert!(out.stdout.contains("removed"));
    assert!(out.stderr.contains("oops"));
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    assert!(
        events.contains(&ComputeEvent::Stdout("removed".into())),
        "{events:?}"
    );

    let failing = backend
        .exec(ExecRequest::new("exit 3"), None)
        .await
        .unwrap();
    assert_eq!(failing.exit_code, Some(3));

    let slow = backend
        .exec(
            ExecRequest::new("sleep 5").timeout(Duration::from_millis(500)),
            None,
        )
        .await
        .unwrap();
    assert!(slow.timed_out);

    let stat = workspace::diff_stat(backend).await.unwrap();
    assert!(stat.contains("src/lib.rs"), "{stat}");
    let patch = workspace::diff(backend).await.unwrap();

    // Apply to the untouched local checkout.
    let patch_file = local.join("../mira.patch");
    std::fs::write(&patch_file, &patch).unwrap();
    git(local, &["apply", patch_file.to_str().unwrap()]);
    assert!(std::fs::read_to_string(local.join("src/lib.rs"))
        .unwrap()
        .contains("42"));
    assert!(local.join("docs/new/notes.md").exists());
    assert!(!local.join("README.md").exists());
    assert_eq!(
        std::fs::read_to_string(local.join(".env")).unwrap(),
        "SECRET=hunter2"
    );

    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_scratch_roundtrip() {
    let proj = project();
    let backend = LocalBackend::scratch().unwrap();
    let root = backend.root().to_path_buf();
    exercise(&backend, proj.path()).await;
    assert!(!root.exists(), "scratch dir removed on shutdown");
}

// ---------- fake E2B ----------

#[derive(Clone)]
struct Fake {
    root: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn map(&self, remote: &str) -> PathBuf {
        let rel = remote
            .strip_prefix(WORKSPACE_ROOT)
            .unwrap_or(remote)
            .trim_start_matches('/');
        self.root.join(rel)
    }

    fn log(&self, s: impl Into<String>) {
        self.calls.lock().unwrap().push(s.into());
    }
}

async fn create(State(f): State<Fake>, headers: axum::http::HeaderMap) -> impl IntoResponse {
    if headers.get("X-API-Key").and_then(|v| v.to_str().ok()) != Some("test-key") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "bad key"})),
        );
    }
    f.log("create");
    (
        StatusCode::CREATED,
        Json(json!({"sandboxID": "sbx1", "envdAccessToken": "tok", "clientID": "c"})),
    )
}

async fn kill(State(f): State<Fake>) -> StatusCode {
    f.log("kill");
    StatusCode::NO_CONTENT
}

fn authed(headers: &axum::http::HeaderMap) -> bool {
    headers.get("X-Access-Token").and_then(|v| v.to_str().ok()) == Some("tok")
}

async fn read_file(
    State(f): State<Fake>,
    headers: axum::http::HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> axum::response::Response {
    if !authed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match std::fs::read(f.map(&q["path"])) {
        Ok(b) => b.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn write_file(
    State(f): State<Fake>,
    headers: axum::http::HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    mut form: Multipart,
) -> StatusCode {
    if !authed(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    while let Some(field) = form.next_field().await.unwrap() {
        if field.name() == Some("file") {
            let p = f.map(&q["path"]);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, field.bytes().await.unwrap()).unwrap();
        }
    }
    StatusCode::OK
}

fn frame(flags: u8, v: Value) -> Vec<u8> {
    let payload = v.to_string().into_bytes();
    let mut out = vec![flags];
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend(payload);
    out
}

async fn start(State(f): State<Fake>, headers: axum::http::HeaderMap, body: Bytes) -> Body {
    assert!(authed(&headers));
    assert_eq!(
        headers.get("content-type").unwrap(),
        "application/connect+json"
    );
    let req: Value = serde_json::from_slice(&body[5..]).unwrap();
    let p = &req["process"];
    let args: Vec<String> = p["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    let cwd = f.map(p["cwd"].as_str().unwrap());
    let cmd = p["cmd"].as_str().unwrap().to_owned();
    f.log(format!("exec {}", args.last().unwrap()));
    let b64 = base64::engine::general_purpose::STANDARD;
    // Stream: start, then output chunks as the process produces them,
    // then end + trailer.
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, std::io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(frame(0, json!({"event": {"start": {"pid": 7}}}))))
            .await;
        let out = tokio::process::Command::new(&cmd)
            .args(&args)
            .current_dir(cwd)
            .output()
            .await
            .unwrap();
        for (field, data) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
            if !data.is_empty() {
                let ev = json!({"event": {"data": {field: b64.encode(data)}}});
                let _ = tx.send(Ok(frame(0, ev))).await;
            }
        }
        let code = out.status.code().unwrap_or(-1);
        let _ = tx
            .send(Ok(frame(
                0,
                json!({"event": {"end": {"exitCode": code, "exited": true}}}),
            )))
            .await;
        let _ = tx.send(Ok(frame(2, json!({})))).await;
    });
    Body::from_stream(tokio_stream_from(rx))
}

fn tokio_stream_from(
    mut rx: mpsc::Receiver<Result<Vec<u8>, std::io::Error>>,
) -> impl futures::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    async_stream(move |tx| async move {
        while let Some(item) = rx.recv().await {
            if tx.send(item).await.is_err() {
                break;
            }
        }
    })
}

/// Tiny channel-backed stream helper (avoids an async-stream dep).
fn async_stream<F, Fut, T>(f: F) -> impl futures::Stream<Item = T>
where
    F: FnOnce(mpsc::Sender<T>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
    T: Send + 'static,
{
    let (tx, mut rx) = mpsc::channel(16);
    tokio::spawn(f(tx));
    futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

async fn signal(State(f): State<Fake>) -> Json<Value> {
    f.log("signal");
    Json(json!({}))
}

async fn noop() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[tokio::test]
async fn e2b_roundtrip_against_fake() {
    let proj = project();
    let root = tempfile::tempdir().unwrap();
    let fake = Fake {
        root: root.path().to_path_buf(),
        calls: Arc::default(),
    };
    let app = Router::new()
        .route("/sandboxes", post(create))
        .route("/sandboxes/:id", delete(kill))
        .route("/sandboxes/:id/timeout", post(noop))
        .route("/files", get(read_file).post(write_file))
        .route("/process.Process/Start", post(start))
        .route("/process.Process/SendSignal", post(signal))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut opts = E2bOptions::new("wrong-key");
    opts.api_url = url.clone();
    opts.envd_url = Some(url.clone());
    assert!(
        E2bBackend::create(opts.clone()).await.is_err(),
        "bad key rejected"
    );

    opts.api_key = "test-key".into();
    let backend = E2bBackend::create(opts).await.unwrap();
    assert_eq!(backend.sandbox_id(), "sbx1");
    assert_eq!(backend.workspace_root(), WORKSPACE_ROOT);
    exercise(&backend, proj.path()).await;

    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls.first().map(String::as_str), Some("create"));
    assert!(
        calls.contains(&"signal".to_owned()),
        "timeout kills the process: {calls:?}"
    );
    assert_eq!(calls.last().map(String::as_str), Some("kill"));
}
