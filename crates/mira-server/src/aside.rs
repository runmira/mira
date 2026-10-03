//! Ask aside: a side question about the session, answered while the main
//! agent keeps working.
//!
//! `POST /api/aside?session=` with
//! `{ "question": "…", "history": [{ "role": "user"|"assistant", "content": "…" }], "context": "…"? }`
//! streams NDJSON: `{"type":"delta","text":…}` chunks, then `{"type":"done"}`
//! or `{"type":"error","message":…}`.
//!
//! It's one model call on the session's model, with no tools: it can read
//! the conversation (condensed) and any `@path` files the question names,
//! and it never touches the session — the main agent doesn't see it, and
//! nothing it says lands in the transcript. `history` is the aside's own
//! earlier back-and-forth, kept by the pane. `context` is the pane's own
//! digest, used when the server has no transcript (an external agent's
//! session).

use std::path::{Path as FsPath, PathBuf};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use mira_ai::{ChatEvent, ChatRequest};
use mira_core::{Message, Role};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::state::AppState;

/// How much of the conversation the aside sees, newest kept. Roughly 20k
/// tokens: plenty to know what's going on, cheap enough to ask often.
const DIGEST_CHARS: usize = 80_000;
/// Per `@file` the question mentions.
const FILE_CHARS: usize = 40_000;

#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AsideTurn {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
pub struct AsideBody {
    question: String,
    #[serde(default)]
    history: Vec<AsideTurn>,
    #[serde(default)]
    context: Option<String>,
}

const SYSTEM: &str = "You are answering a side question (\"ask aside\") about a coding session \
that is still in progress. Another agent — the main agent — is working on the user's request \
right now; you are not that agent and you cannot act: no tools, no edits, no commands. You \
answer from the session digest below and any files the user attached.\n\n\
Be direct and brief: answer the question first, in a few sentences or a short list. Use \
markdown. Quote code only when it helps. If the digest doesn't say, say so plainly rather \
than guessing — and say what the user could ask the main agent instead. Refer to the main \
agent as \"the agent\".";

pub async fn ask(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
    Json(body): Json<AsideBody>,
) -> Response {
    let question = body.question.trim().to_owned();
    if question.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "ask something" })),
        )
            .into_response();
    }
    let slot = match q.session.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => match state.slot_str(id).await {
            Some(s) => s,
            None => state.active_slot().await,
        },
        None => state.active_slot().await,
    };
    let session = slot.session.read().await.clone();
    let cwd = slot.cwd.read().await.clone();
    let cfg = session.config().await;
    // A turn in flight may hold the history briefly; don't let the aside
    // wait on the agent — fall back to the pane's digest instead.
    let transcript = tokio::time::timeout(std::time::Duration::from_secs(2), session.transcript())
        .await
        .unwrap_or_default();
    let mut digest = digest(&transcript);
    if digest.trim().is_empty() {
        digest = body.context.clone().unwrap_or_default();
    }
    let digest = keep_tail(&digest, DIGEST_CHARS);
    let files = attached_files(&question, &cwd);

    let mut system = format!("{SYSTEM}\n\nWorking directory: {}\n", cwd.display());
    system.push_str("\n<session_digest>\n");
    system.push_str(if digest.trim().is_empty() {
        "(The session has no messages yet.)"
    } else {
        &digest
    });
    system.push_str("\n</session_digest>\n");
    for (path, text) in &files {
        system.push_str(&format!("\n<file path=\"{path}\">\n{text}\n</file>\n"));
    }

    let mut messages = vec![Message::system(system)];
    for t in body.history.iter().rev().take(20).rev() {
        match t.role.as_str() {
            "assistant" => messages.push(Message::assistant(t.content.clone())),
            _ => messages.push(Message::user(t.content.clone())),
        }
    }
    messages.push(Message::user(question));

    let req = ChatRequest {
        model: cfg.model.clone(),
        messages,
        tools: Vec::new(),
        temperature: Some(0.3),
        max_tokens: Some(4096),
        reasoning_effort: None,
        service_tier: None,
        response_format: None,
    };
    let provider = state.harness_provider.clone();
    let (tx, rx) = mpsc::channel::<String>(64);
    tokio::spawn(async move {
        let send = |v: serde_json::Value| {
            let tx = tx.clone();
            async move { tx.send(format!("{v}\n")).await.is_ok() }
        };
        let mut stream = match provider.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                send(serde_json::json!({"type":"error","message":e.to_string()})).await;
                return;
            }
        };
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(ChatEvent::TextDelta(t)) => {
                    if !send(serde_json::json!({"type":"delta","text":t})).await {
                        return; // the pane went away; stop paying for tokens
                    }
                }
                Ok(ChatEvent::Done(_)) => break,
                Ok(_) => {}
                Err(e) => {
                    send(serde_json::json!({"type":"error","message":e.to_string()})).await;
                    return;
                }
            }
        }
        send(serde_json::json!({"type":"done"})).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(Ok::<_, std::convert::Infallible>);
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap()
        .into_response()
}

/// The conversation as compact text: what the user asked, what the agent
/// said, and one line per tool call (with a peek at its result).
fn digest(transcript: &[Message]) -> String {
    let mut out = String::new();
    for m in transcript {
        match m.role {
            Role::System => {}
            Role::User => {
                let raw = m.content.as_deref().unwrap_or("");
                let text = if mira_harness::history::is_summary(m) {
                    format!("[Earlier conversation, summarized]\n{raw}")
                } else {
                    mira_harness::history::strip_hook_context(raw).to_owned()
                };
                out.push_str(&format!("\n## User\n{}\n", clip(text.trim(), 4000)));
            }
            Role::Assistant => {
                if let Some(t) = m.content.as_deref().filter(|t| !t.trim().is_empty()) {
                    out.push_str(&format!("\n## Agent\n{}\n", clip(t.trim(), 4000)));
                }
                for c in &m.tool_calls {
                    out.push_str(&format!(
                        "- tool {}({})\n",
                        c.function.name,
                        clip(&c.function.arguments, 300)
                    ));
                }
            }
            Role::Tool => {
                if let Some(t) = m.content.as_deref() {
                    let one = t.lines().take(6).collect::<Vec<_>>().join(" ⏎ ");
                    out.push_str(&format!("  → {}\n", clip(&one, 300)));
                }
            }
        }
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// The last `max` chars, cut at a line so the digest starts cleanly.
fn keep_tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_owned();
    }
    let tail: String = s.chars().skip(n - max).collect();
    match tail.find('\n') {
        Some(i) => format!("[…earlier messages omitted…]{}", &tail[i..]),
        None => tail,
    }
}

/// `@path` tokens in the question that name files inside `cwd`.
fn attached_files(question: &str, cwd: &FsPath) -> Vec<(String, String)> {
    let Ok(root) = cwd.canonicalize() else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for tok in question.split_whitespace() {
        let Some(raw) = tok.strip_prefix('@') else {
            continue;
        };
        let rel = raw.trim_end_matches([',', '.', '?', '!', ':', ';', ')']);
        if rel.is_empty() || out.iter().any(|(p, _)| p == rel) || out.len() >= 5 {
            continue;
        }
        let path: PathBuf = root.join(rel);
        let Ok(real) = path.canonicalize() else {
            continue;
        };
        // Inside the project only — `@../../.ssh/id_rsa` reads nothing.
        if !real.starts_with(&root) || !real.is_file() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&real) {
            out.push((rel.to_owned(), clip(&text, FILE_CHARS)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_condenses_tool_calls() {
        let mut call = Message::assistant("Looking at it.");
        call.tool_calls = vec![mira_core::ToolCall {
            id: "c1".into(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction {
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            },
        }];
        let d = digest(&[
            Message::system("sys"),
            Message::user("fix the bug"),
            call,
            Message::tool("c1".into(), "fn main() {}\nline2"),
        ]);
        assert!(d.contains("## User\nfix the bug"));
        assert!(d.contains("- tool read_file({\"path\":\"src/main.rs\"})"));
        assert!(d.contains("→ fn main() {} ⏎ line2"));
        assert!(!d.contains("sys"));
    }

    #[test]
    fn attaches_only_files_inside_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let got = attached_files("what is in @a.txt? and @../etc/passwd", dir.path());
        assert_eq!(got, vec![("a.txt".to_owned(), "hello".to_owned())]);
    }

    #[test]
    fn keeps_the_newest_text() {
        let s = format!("{}\nnewest line", "x".repeat(100));
        let t = keep_tail(&s, 20);
        assert!(t.ends_with("newest line"));
        assert!(t.starts_with("[…earlier"));
    }
}
