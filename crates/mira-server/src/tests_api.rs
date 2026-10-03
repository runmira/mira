//! The Tests pane: find the project's test command and run it.
//!
//! `GET  /api/tests/detect?session=` — the commands this project's files
//!       suggest (`cargo test`, `pnpm test`, `pytest -q`, `go test ./...`),
//!       best first.
//! `POST /api/tests/run?session=`    — `{ "command": "…" }`. Streams NDJSON:
//!       `{"type":"line","text":…}` per output line, then
//!       `{"type":"exit","code":…}`. Closing the request stops the run (and
//!       everything it started).
//!
//! The pane parses the output itself (per framework), so this stays a
//! plain runner. POST + JSON so another site can't start a command.

use std::path::Path as FsPath;
use std::process::Stdio;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    command: String,
    /// What it is, for the picker: `Cargo`, `Vitest (pnpm)`, `pytest`.
    label: String,
}

async fn cwd_for(state: &AppState, q: &SessionQuery) -> std::path::PathBuf {
    if let Some(id) = q.session.as_deref().filter(|s| !s.is_empty()) {
        if let Some(slot) = state.slot_str(id).await {
            return slot.cwd.read().await.clone();
        }
    }
    state.current_cwd().await
}

pub async fn detect(State(state): State<AppState>, Query(q): Query<SessionQuery>) -> Response {
    let cwd = cwd_for(&state, &q).await;
    let found = tokio::task::spawn_blocking(move || suggestions(&cwd))
        .await
        .unwrap_or_default();
    Json(serde_json::json!({ "suggestions": found })).into_response()
}

fn suggestions(dir: &FsPath) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let has = |f: &str| dir.join(f).exists();
    let mut push = |command: &str, label: &str| {
        out.push(Suggestion {
            command: command.into(),
            label: label.into(),
        })
    };

    if has("package.json") {
        let pkg: serde_json::Value = std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let pm = if has("bun.lockb") || has("bun.lock") {
            "bun"
        } else if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else {
            "npm"
        };
        let dep = |name: &str| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|k| pkg.get(k).and_then(|d| d.get(name)).is_some())
        };
        let runner = if dep("vitest") {
            "Vitest"
        } else if dep("jest") {
            "Jest"
        } else if dep("@playwright/test") {
            "Playwright"
        } else {
            "tests"
        };
        let script = pkg
            .get("scripts")
            .and_then(|s| s.get("test"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        // npm init's placeholder isn't a test suite.
        if !script.is_empty() && !script.contains("no test specified") {
            let cmd = if pm == "npm" { "npm test" } else { &format!("{pm} test") as &str };
            push(cmd, &format!("{runner} ({pm})"));
        } else if dep("vitest") {
            push("npx vitest run", "Vitest");
        } else if dep("jest") {
            push("npx jest", "Jest");
        }
    }
    if has("Cargo.toml") {
        push("cargo test", "Cargo");
    }
    if has("go.mod") {
        push("go test ./...", "Go");
    }
    if has("pytest.ini")
        || has("conftest.py")
        || std::fs::read_to_string(dir.join("pyproject.toml"))
            .map(|s| s.contains("pytest"))
            .unwrap_or(false)
        || (has("tests") && (has("pyproject.toml") || has("setup.py")))
    {
        push("python -m pytest -q", "pytest");
    }
    if has("Gemfile") && has("spec") {
        push("bundle exec rspec", "RSpec");
    }
    if has("mix.exs") {
        push("mix test", "ExUnit");
    }
    if has("Package.swift") {
        push("swift test", "Swift");
    }
    out
}

#[derive(Debug, Deserialize)]
pub struct RunBody {
    command: String,
}

pub async fn run(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
    Json(body): Json<RunBody>,
) -> Response {
    let command = body.command.trim().to_owned();
    if command.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "command is empty" })),
        )
            .into_response();
    }
    let cwd = cwd_for(&state, &q).await;
    let (tx, rx) = mpsc::channel::<String>(256);

    tokio::spawn(async move {
        let line = |v: serde_json::Value| format!("{v}\n");
        let mut cmd = Command::new("bash");
        cmd.args(["-lc", &command])
            .current_dir(&cwd)
            // Run once and print plainly: watch modes and colors both key
            // off these.
            .env("CI", "1")
            .env("FORCE_COLOR", "0")
            .env("NO_COLOR", "1")
            .env("CARGO_TERM_COLOR", "never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(line(serde_json::json!({"type":"line","text":format!("couldn't start: {e}")})))
                    .await;
                let _ = tx.send(line(serde_json::json!({"type":"exit","code":-1}))).await;
                return;
            }
        };
        let pgid = child.id();
        let (ltx, mut lrx) = mpsc::channel::<String>(256);
        for pipe in [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let ltx = ltx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if ltx.send(l).await.is_err() {
                        break;
                    }
                }
            });
        }
        drop(ltx);

        loop {
            tokio::select! {
                next = lrx.recv() => match next {
                    Some(l) => {
                        if tx.send(line(serde_json::json!({"type":"line","text":l}))).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                // The pane closed the request (Stop, or went away).
                _ = tx.closed() => break,
            }
        }

        if tx.is_closed() {
            #[cfg(unix)]
            if let Some(p) = pgid {
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", "--", &format!("-{p}")])
                    .status();
            }
            let _ = child.kill().await;
            return;
        }
        let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
        let _ = tx.send(line(serde_json::json!({"type":"exit","code":code}))).await;
    });

    let stream = futures::StreamExt::map(tokio_stream::wrappers::ReceiverStream::new(rx), |s| {
        Ok::<_, std::convert::Infallible>(s)
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap()
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_by_project_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest"},"devDependencies":{"vitest":"1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let s = suggestions(dir.path());
        assert_eq!(s[0].command, "pnpm test");
        assert_eq!(s[0].label, "Vitest (pnpm)");
        assert_eq!(s[1].command, "cargo test");
    }

    #[test]
    fn ignores_npm_placeholder_script() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        assert!(suggestions(dir.path()).is_empty());
    }
}
