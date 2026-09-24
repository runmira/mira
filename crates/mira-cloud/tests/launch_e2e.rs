//! Full system, minus the real services: `Launch` against a fake E2B that
//! runs commands on this machine, uploading the real `mira` binary, which
//! then runs the worker against a bare git repo, a fake GitHub, and a fake
//! OpenAI-compatible model (which also plays the goal evaluator).
//!
//! Needs a built binary:
//!
//! ```sh
//! cargo build -p mira-cli && cargo test -p mira-cloud --test launch_e2e -- --ignored
//! ```
//! (`MIRA_BIN=/path/to/mira` overrides the binary.)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Multipart, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use base64::Engine as _;
use mira_cloud::launcher::{Launch, MiraInstall};
use mira_cloud::spec::{GitIdentity, Limits, ModelSpec, RepoSpec, Secrets, TaskSpec};
use mira_compute::e2b::WORKSPACE_ROOT;
use mira_compute::env::{BackendSpec, EnvironmentSpec};
use mira_compute::E2bOptions;
use serde_json::{json, Value};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn mira_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MIRA_BIN") {
        return Some(PathBuf::from(p));
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/mira");
    p.exists().then_some(p)
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

// ---------- fake E2B ----------

#[derive(Clone)]
struct E2b {
    root: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
}

impl E2b {
    fn map(&self, p: &str) -> PathBuf {
        self.root.join(
            p.strip_prefix(WORKSPACE_ROOT)
                .unwrap_or(p)
                .trim_start_matches('/'),
        )
    }
}

async fn e2b_create(State(f): State<E2b>, Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
    f.calls
        .lock()
        .unwrap()
        .push(format!("create timeout={}", b["timeout"]));
    (
        StatusCode::CREATED,
        Json(json!({"sandboxID": "sbx9", "envdAccessToken": "tok"})),
    )
}
async fn e2b_kill(State(f): State<E2b>) -> StatusCode {
    f.calls.lock().unwrap().push("kill".into());
    StatusCode::NO_CONTENT
}
async fn e2b_get() -> Json<Value> {
    Json(json!({"sandboxID": "sbx9"}))
}
async fn e2b_read(State(f): State<E2b>, Query(q): Query<HashMap<String, String>>) -> Response {
    match std::fs::read(f.map(&q["path"])) {
        Ok(b) => b.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn e2b_write(
    State(f): State<E2b>,
    Query(q): Query<HashMap<String, String>>,
    mut form: Multipart,
) -> StatusCode {
    while let Some(field) = form.next_field().await.unwrap() {
        let p = f.map(&q["path"]);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, field.bytes().await.unwrap()).unwrap();
    }
    StatusCode::OK
}
fn frame(flags: u8, v: Value) -> Vec<u8> {
    let p = v.to_string().into_bytes();
    let mut out = vec![flags];
    out.extend_from_slice(&(p.len() as u32).to_be_bytes());
    out.extend(p);
    out
}
async fn e2b_start(State(f): State<E2b>, body: Bytes) -> Body {
    let req: Value = serde_json::from_slice(&body[5..]).unwrap();
    let p = &req["process"];
    let args: Vec<String> = p["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    let out = tokio::process::Command::new(p["cmd"].as_str().unwrap())
        .args(&args)
        .current_dir(f.map(p["cwd"].as_str().unwrap()))
        .output()
        .await
        .unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut bytes = frame(0, json!({"event": {"start": {"pid": 1}}}));
    bytes.extend(frame(
        0,
        json!({"event": {"data": {"stdout": b64.encode(&out.stdout)}}}),
    ));
    bytes.extend(frame(
        0,
        json!({"event": {"data": {"stderr": b64.encode(&out.stderr)}}}),
    ));
    bytes.extend(frame(
        0,
        json!({"event": {"end": {"exitCode": out.status.code().unwrap_or(-1)}}}),
    ));
    bytes.extend(frame(2, json!({})));
    Body::from(bytes)
}

// ---------- fake model (OpenAI-compatible SSE) ----------

#[derive(Clone, Default)]
struct Model {
    evals: Arc<Mutex<usize>>,
}

async fn chat(State(m): State<Model>, Json(req): Json<Value>) -> Response {
    let msgs = req["messages"].as_array().unwrap();
    let first = msgs[0]["content"].to_string();
    let chunk = |delta: Value, finish: Option<&str>| {
        format!(
            "data: {}\n\n",
            json!({"id": "x", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        )
    };
    let body = if first.contains("verdict") {
        *m.evals.lock().unwrap() += 1;
        chunk(
            json!({"content": json!({"verdict": "met", "reason": "file written"}).to_string()}),
            Some("stop"),
        )
    } else if msgs.last().unwrap()["role"] == "tool" {
        chunk(
            json!({"content": "Wrote CLOUD.md and checked it."}),
            Some("stop"),
        )
    } else {
        let args = json!({"path": "CLOUD.md", "content": "made in the cloud\n"}).to_string();
        chunk(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "write_file", "arguments": args}}]}),
            Some("tool_calls"),
        )
    } + "data: [DONE]\n\n";
    ([("content-type", "text/event-stream")], body).into_response()
}

// ---------- fake GitHub ----------

#[derive(Clone, Default)]
struct Gh {
    calls: Arc<Mutex<Vec<String>>>,
}
async fn gh_create(State(g): State<Gh>) -> (StatusCode, Json<Value>) {
    g.calls.lock().unwrap().push("create".into());
    (
        StatusCode::CREATED,
        Json(
            json!({"number": 1, "html_url": "https://github.test/o/r/pull/1", "node_id": "PR_1", "draft": true}),
        ),
    )
}
async fn gh_ok(State(g): State<Gh>, h: HeaderMap) -> Json<Value> {
    assert_eq!(h.get("authorization").unwrap(), "Bearer gh");
    g.calls.lock().unwrap().push("call".into());
    Json(json!({}))
}
async fn gh_graphql(State(g): State<Gh>) -> Json<Value> {
    g.calls.lock().unwrap().push("ready".into());
    Json(json!({"data": {}}))
}

#[tokio::test]
#[ignore = "needs a built mira binary (cargo build -p mira-cli)"]
async fn launch_runs_the_real_worker_to_a_ready_pr() {
    let Some(bin) = mira_bin() else {
        panic!("no mira binary; run `cargo build -p mira-cli` or set MIRA_BIN");
    };

    // Remote repo.
    let remote = tempfile::tempdir().unwrap();
    let seed = remote.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hi\n").unwrap();
    git(&seed, &["add", "-A"]);
    git(
        &seed,
        &[
            "-c",
            "user.name=s",
            "-c",
            "user.email=s@s",
            "commit",
            "-qm",
            "init",
        ],
    );
    git(
        remote.path(),
        &["clone", "-q", "--bare", "seed", "remote.git"],
    );

    let sandbox = tempfile::tempdir().unwrap();
    let e2b = E2b {
        root: sandbox.path().to_path_buf(),
        calls: Arc::default(),
    };
    let e2b_url = serve(
        Router::new()
            .route("/sandboxes", post(e2b_create))
            .route("/sandboxes/:id", delete(e2b_kill).get(e2b_get))
            .route("/files", get(e2b_read).post(e2b_write))
            .route("/process.Process/Start", post(e2b_start))
            .layer(axum::extract::DefaultBodyLimit::disable())
            .with_state(e2b.clone()),
    )
    .await;
    let model = Model::default();
    let model_url = serve(
        Router::new()
            .route("/chat/completions", post(chat))
            .with_state(model.clone()),
    )
    .await;
    let gh = Gh::default();
    let gh_url = serve(
        Router::new()
            .route("/repos/:o/:r/pulls", post(gh_create))
            .route("/repos/:o/:r/pulls/:n", patch(gh_ok))
            .route("/repos/:o/:r/issues/:n/comments", post(gh_ok))
            .route("/graphql", post(gh_graphql))
            .with_state(gh.clone()),
    )
    .await;

    let mut opts = E2bOptions::new("e2b-key");
    opts.api_url = e2b_url.clone();
    opts.envd_url = Some(e2b_url);
    let launch = Launch {
        spec: TaskSpec {
            id: "t-e2e001".into(),
            prompt: "Add a CLOUD.md file".into(),
            repo: RepoSpec {
                owner: "o".into(),
                name: "r".into(),
                clone_url: format!("file://{}", remote.path().join("remote.git").display()),
                api_url: gh_url,
            },
            base_branch: "main".into(),
            branch: "mira/add-cloud-md-t-e2e001".into(),
            workdir: Default::default(),
            git_identity: GitIdentity {
                name: "Dami".into(),
                email: "d@example.com".into(),
            },
            model: ModelSpec {
                provider: "fake".into(),
                base_url: model_url,
                model: "fake-model".into(),
                evaluator_model: None,
                prompt_caching: false,
            },
            limits: Limits {
                max_runtime_secs: 600,
                max_iterations: 3,
                budget_usd: None,
            },
            setup: None,
            sandbox_id: None,
            e2b_api_url: None,
        },
        secrets: Secrets {
            model_api_key: "model-key".into(),
            github_token: "gh".into(),
            e2b_api_key: Some("e2b-key".into()),
        },
        environment: EnvironmentSpec {
            name: "e2b".into(),
            description: None,
            backend: BackendSpec::E2b(opts),
            env: Default::default(),
            setup: None,
        },
        install: MiraInstall::Upload(bin),
        worker_command: None,
    };
    let record = launch
        .run(Arc::new(|m| eprintln!("launch: {m}")))
        .await
        .unwrap();
    assert_eq!(record.sandbox_id, "sbx9");
    assert_eq!(record.envd_access_token.as_deref(), Some("tok"));
    // The launcher must leave the sandbox running. With an instant fake
    // model the worker may already be done (and have deleted its own
    // sandbox); a kill is only acceptable once the log says so.
    if e2b.calls.lock().unwrap().contains(&"kill".to_owned()) {
        let log =
            std::fs::read_to_string(sandbox.path().join(".mira-task/task.log")).unwrap_or_default();
        assert!(
            log.contains("finished:"),
            "sandbox killed before the worker finished:\n{log}"
        );
    }
    assert!(
        e2b.calls.lock().unwrap()[0].contains("timeout=720"),
        "task budget + grace"
    );

    // The detached worker delivers on its own.
    let mut delivered = false;
    for _ in 0..120 {
        if e2b.calls.lock().unwrap().contains(&"kill".to_owned()) {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let log =
        std::fs::read_to_string(sandbox.path().join(".mira-task/task.log")).unwrap_or_default();
    assert!(delivered, "worker never finished; log:\n{log}");
    assert!(log.contains("finished: done"), "{log}");
    assert!(
        !sandbox.path().join(".mira-task/secrets.json").exists(),
        "secrets file consumed"
    );
    assert_eq!(
        git(
            &remote.path().join("remote.git"),
            &["show", "mira/add-cloud-md-t-e2e001:CLOUD.md"]
        ),
        "made in the cloud\n"
    );
    let calls = gh.calls.lock().unwrap().clone();
    assert_eq!(calls.first().map(String::as_str), Some("create"));
    assert_eq!(calls.last().map(String::as_str), Some("ready"), "{calls:?}");
    assert!(*model.evals.lock().unwrap() >= 1);
}
