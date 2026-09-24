//! The worker, end to end: bare git repo as the remote, an in-process
//! fake of the GitHub API, and a scripted model that also plays the goal
//! evaluator. Checks the branch, the commits, and the PR lifecycle.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{patch, post};
use axum::{Json, Router};
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
use mira_cloud::spec::{GitIdentity, Limits, ModelSpec, RepoSpec, Secrets, TaskSpec};
use mira_cloud::worker::{TaskStatus, Worker};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Role, ToolCall};
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

/// Bare "GitHub" repo with one commit on `main`.
fn remote() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
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
    git(dir.path(), &["clone", "-q", "--bare", "seed", "remote.git"]);
    dir
}

/// Plays both roles: the coding model and the goal evaluator.
struct Scripted {
    main_turns: Mutex<usize>,
    evals: Mutex<usize>,
}

fn call(args: Value) -> ToolCall {
    ToolCall {
        id: format!("c{}", rand_id()).into(),
        kind: ToolCallKind::Function,
        function: ToolCallFunction {
            name: "write_file".into(),
            arguments: args.to_string(),
        },
    }
}

fn rand_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[async_trait]
impl ChatProvider for Scripted {
    async fn stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let is_eval = req
            .messages
            .first()
            .and_then(|m| m.content.as_deref())
            .is_some_and(|c| c.contains("\"verdict\""));
        let events = if is_eval {
            let mut n = self.evals.lock().unwrap();
            *n += 1;
            let verdict = if *n == 1 {
                json!({"verdict": "not_met", "reason": "hello.txt still says v1"})
            } else {
                json!({"verdict": "met", "reason": "hello.txt says v2"})
            };
            vec![
                ChatEvent::TextDelta(verdict.to_string()),
                ChatEvent::Done(FinishReason::Stop),
            ]
        } else if req.messages.last().is_some_and(|m| m.role == Role::Tool) {
            vec![
                ChatEvent::TextDelta("Updated hello.txt and read it back to verify.".into()),
                ChatEvent::Done(FinishReason::Stop),
            ]
        } else {
            let mut n = self.main_turns.lock().unwrap();
            *n += 1;
            vec![
                ChatEvent::ToolCalls(vec![call(
                    json!({"path": "hello.txt", "content": format!("v{n}\n")}),
                )]),
                ChatEvent::Done(FinishReason::ToolCalls),
            ]
        };
        Ok(stream::iter(events.into_iter().map(Ok)).boxed())
    }
}

#[derive(Clone, Default)]
struct FakeGh {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

fn authed(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer gh-token")
}

async fn create_pr(
    State(f): State<FakeGh>,
    h: HeaderMap,
    Json(b): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert!(authed(&h));
    f.calls.lock().unwrap().push(("create".into(), b));
    (
        StatusCode::CREATED,
        Json(
            json!({"number": 7, "html_url": "https://github.test/o/r/pull/7", "node_id": "PR_7", "draft": true, "state": "open"}),
        ),
    )
}

async fn update_pr(State(f): State<FakeGh>, h: HeaderMap, Json(b): Json<Value>) -> Json<Value> {
    assert!(authed(&h));
    f.calls.lock().unwrap().push(("update".into(), b));
    Json(json!({}))
}

async fn comment(
    State(f): State<FakeGh>,
    h: HeaderMap,
    Json(b): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert!(authed(&h));
    f.calls.lock().unwrap().push(("comment".into(), b));
    (StatusCode::CREATED, Json(json!({})))
}

async fn graphql(State(f): State<FakeGh>, h: HeaderMap, Json(b): Json<Value>) -> Json<Value> {
    assert!(authed(&h));
    f.calls.lock().unwrap().push(("graphql".into(), b));
    Json(json!({"data": {"markPullRequestReadyForReview": {"pullRequest": {"isDraft": false}}}}))
}

#[tokio::test]
async fn delivers_a_ready_pr_with_the_work() {
    let remote = remote();
    let remote_url = format!("file://{}", remote.path().join("remote.git").display());
    let work = tempfile::tempdir().unwrap();

    let fake = FakeGh::default();
    let app = Router::new()
        .route("/repos/:o/:r/pulls", post(create_pr))
        .route("/repos/:o/:r/pulls/:n", patch(update_pr))
        .route("/repos/:o/:r/issues/:n/comments", post(comment))
        .route("/graphql", post(graphql))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let spec = TaskSpec {
        id: "t-test01".into(),
        prompt: "Make hello.txt say v2".into(),
        repo: RepoSpec {
            owner: "o".into(),
            name: "r".into(),
            clone_url: remote_url,
            api_url: api,
        },
        base_branch: "main".into(),
        branch: "mira/make-hello-t-test01".into(),
        workdir: work.path().join("repo"),
        git_identity: GitIdentity {
            name: "Dami".into(),
            email: "dami@example.com".into(),
        },
        model: ModelSpec {
            provider: "fake".into(),
            base_url: String::new(),
            model: "fake-model".into(),
            evaluator_model: None,
            prompt_caching: false,
        },
        limits: Limits {
            max_runtime_secs: 600,
            max_iterations: 5,
            budget_usd: None,
        },
        setup: Some("echo preparing > .setup-ran".into()),
        sandbox_id: None,
        e2b_api_url: None,
    };
    let log_lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let l = log_lines.clone();
    let outcome = Worker {
        spec,
        secrets: Secrets {
            model_api_key: "unused".into(),
            github_token: "gh-token".into(),
            e2b_api_key: None,
        },
        provider: Arc::new(Scripted {
            main_turns: Mutex::new(0),
            evals: Mutex::new(0),
        }),
        log: Arc::new(move |s| l.lock().unwrap().push(s)),
    }
    .run()
    .await
    .unwrap();

    let log = log_lines.lock().unwrap().join("\n");
    assert_eq!(outcome.status, TaskStatus::Done, "{log}");
    assert_eq!(outcome.iterations, 2, "{log}");

    // The branch landed on the remote with the final content, authored as
    // the user, including the setup script's output.
    let bare = remote.path().join("remote.git");
    let branch = "mira/make-hello-t-test01";
    assert_eq!(
        git(&bare, &["show", &format!("{branch}:hello.txt")]),
        "v2\n"
    );
    assert_eq!(
        git(&bare, &["show", &format!("{branch}:.setup-ran")]),
        "preparing\n"
    );
    let log_out = git(&bare, &["log", "--format=%an <%ae>|%s", branch]);
    assert!(
        log_out
            .lines()
            .all(|l| l.starts_with("Dami <dami@example.com>|") || l.contains("init")),
        "{log_out}"
    );
    assert!(
        log_out.contains("Start Mira task: Make hello.txt say v2"),
        "{log_out}"
    );
    assert!(log_out.contains("Mira: iteration 1"), "{log_out}");

    // PR lifecycle: draft created first, progress updates, summary, ready.
    let calls = fake.calls.lock().unwrap().clone();
    let kinds: Vec<&str> = calls.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds.first(), Some(&"create"), "{kinds:?}");
    assert_eq!(calls[0].1["draft"], true);
    assert_eq!(calls[0].1["head"], branch);
    assert_eq!(calls[0].1["base"], "main");
    assert!(kinds.contains(&"update"));
    let summary = calls.iter().find(|(k, _)| k == "comment").unwrap().1["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        summary.contains("## Summary") && summary.contains("hello.txt"),
        "{summary}"
    );
    assert!(summary.contains("**Status:** done"), "{summary}");
    assert_eq!(
        kinds.last(),
        Some(&"graphql"),
        "marked ready last: {kinds:?}"
    );
    let final_body = calls.iter().rev().find(|(k, _)| k == "update").unwrap().1["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        final_body.contains("status: **done**") && final_body.contains("Iteration 1"),
        "{final_body}"
    );
}
