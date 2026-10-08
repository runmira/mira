//! Thread tools for agents: launch a separate chat (optionally in its own git
//! worktree), then check on it, wait for it, and send it follow-ups.
//!
//! Unlike `delegate_task`, which runs a hidden read-only child and returns
//! one answer, a launched thread is an ordinary chat in the sidebar. It runs
//! on its own, under the launching chat's approval posture, and the user can
//! open it, watch it and step in. Approvals it needs wait for the user
//! (background mode `Park`) rather than being refused or granted silently.
//!
//! A chat may only read and message threads it launched, and a launched
//! thread can't launch more: parallel work fans out one level, not
//! recursively.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mira_core::{Role, SessionId};
use serde_json::{json, Value};

use crate::message_queue::QueuedInput;
use crate::protocol::ServerMsg;
use crate::slot::{BackgroundMode, SessionSlot};
use crate::state::AppState;

/// Threads one chat may have launched.
const MAX_THREADS: usize = 8;
/// Longest a `thread_wait` blocks.
const MAX_WAIT: Duration = Duration::from_secs(10 * 60);
/// How much of a thread's latest reply `thread_read` returns.
const REPLY_CHARS: usize = 6_000;

/// One launched thread, as its launcher sees it.
#[derive(Clone, Debug)]
struct Launched {
    id: String,
    title: String,
    engine: String,
    cwd: PathBuf,
    branch: Option<String>,
}

/// Launcher chat id → threads it launched. In memory: after a restart the
/// threads remain ordinary chats, but tools can no longer address them.
fn registry() -> &'static Mutex<HashMap<String, Vec<Launched>>> {
    static R: OnceLock<Mutex<HashMap<String, Vec<Launched>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn launched_by(parent: &str) -> Vec<Launched> {
    registry()
        .lock()
        .map(|r| r.get(parent).cloned().unwrap_or_default())
        .unwrap_or_default()
}

/// The chat that launched `id`, if it was launched this run.
pub fn launcher_of(id: &str) -> Option<String> {
    registry().lock().ok()?.iter().find_map(|(parent, threads)| {
        threads.iter().any(|t| t.id == id).then(|| parent.clone())
    })
}

fn is_launched_thread(id: &str) -> bool {
    registry()
        .lock()
        .map(|r| r.values().flatten().any(|t| t.id == id))
        .unwrap_or(false)
}

fn find(parent: &str, id: &str) -> Result<Launched, String> {
    launched_by(parent)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("no thread `{id}` was launched from this chat; see thread_list"))
}

/// Run git in `dir`, returning trimmed stdout.
async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// A worktree for `branch` beside the repository (`<repo>-worktrees/<branch>`),
/// reused when it already exists. A new branch starts from `base` (default:
/// the current HEAD).
async fn ensure_worktree(cwd: &Path, branch: &str, base: Option<&str>) -> Result<PathBuf, String> {
    git(cwd, &["check-ref-format", "--branch", branch])
        .await
        .map_err(|_| format!("`{branch}` isn't a valid branch name"))?;
    let root = PathBuf::from(
        git(cwd, &["rev-parse", "--show-toplevel"])
            .await
            .map_err(|_| "this chat's folder isn't a git repository, so it can't have worktrees".to_string())?,
    );
    let repo = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string();
    let parent = root.parent().ok_or("the repository has no parent folder")?;
    let path = parent
        .join(format!("{repo}-worktrees"))
        .join(branch.replace('/', "-"));
    if path.join(".git").exists() {
        return Ok(path);
    }
    let path_str = path.to_string_lossy().to_string();
    let exists = git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .await
        .is_ok();
    let result = if exists {
        git(&root, &["worktree", "add", &path_str, branch]).await
    } else {
        let base = base.unwrap_or("HEAD");
        git(&root, &["worktree", "add", "-b", branch, &path_str, base]).await
    };
    result.map_err(|e| format!("could not create the worktree: {e}"))?;
    Ok(path)
}

/// Queue `text` on `slot` and start delivery.
async fn send(state: &AppState, slot: &Arc<SessionSlot>, text: &str) -> Result<(), String> {
    let (engine, _, _) = slot.selection.snapshot();
    slot.message_queue
        .enqueue(QueuedInput {
            id: format!("thread-{}", uuid::Uuid::new_v4().simple()),
            text: text.to_string(),
            images: Vec::new(),
            engine,
            dispatching: false,
            delivered: false,
            fingerprint: String::new(),
            error: None,
            recovery: None,
        })
        .await?;
    crate::message_queue::publish(slot).await;
    crate::message_queue::wake(state, slot);
    Ok(())
}

/// `thread_launch`: a new chat, running `message` on `engine`.
pub async fn launch(state: &AppState, parent: &Arc<SessionSlot>, args: &Value) -> Result<String, String> {
    let parent_id = parent.id.to_string();
    if parent.session.read().await.is_subagent() || is_launched_thread(&parent_id) {
        return Err("a launched or delegated thread can't launch more threads".into());
    }
    if launched_by(&parent_id).len() >= MAX_THREADS {
        return Err(format!("this chat already launched {MAX_THREADS} threads"));
    }
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or("give the thread a `title`")?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .ok_or("put the task in `message`")?;
    let engine = args
        .get("engine")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|e| !e.is_empty());
    if let Some(e) = engine {
        if !crate::delegate::is_engine(state, e) {
            return Err(format!(
                "unknown engine `{e}` — use one of: {}",
                crate::delegate::engines().join(", ")
            ));
        }
    }

    let parent_cwd = parent.cwd.read().await.clone();
    let worktree = args.get("worktree").filter(|w| w.is_object());
    let branch = worktree
        .and_then(|w| w.get("branch"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty());
    let base = worktree
        .and_then(|w| w.get("base"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty());
    if worktree.is_some() && branch.is_none() {
        return Err("`worktree` needs a `branch`".into());
    }
    let cwd = match branch {
        Some(b) => ensure_worktree(&parent_cwd, b, base).await?,
        None => parent_cwd,
    };

    // Same setup as the launcher: its model settings and approval posture.
    let cfg = parent.session.read().await.config().await;
    let slot = crate::slot::build_slot(cwd.clone(), cfg, None, &state.slot_deps()).await;
    *slot.background_mode.write().await = BackgroundMode::Park;
    state.insert_slot(slot.clone()).await;
    slot.session.read().await.set_title(title).await;

    // The engine: one named, else whatever the launcher runs on.
    let engine_label = match engine {
        Some("mira") => "mira".to_string(),
        Some(e) => {
            let params = crate::delegate::agent_launch_params(state, e)?;
            let label = params.driver_kind.clone();
            crate::session_engine::select_agent(state, &slot, params).await;
            label
        }
        None => match parent.acp_launch.lock().await.clone() {
            Some(params) => {
                let label = params.driver_kind.clone();
                crate::session_engine::select_agent(state, &slot, params).await;
                label
            }
            None => "mira".to_string(),
        },
    };

    let id = slot.id.to_string();
    if let Ok(mut r) = registry().lock() {
        r.entry(parent_id).or_default().push(Launched {
            id: id.clone(),
            title: title.to_string(),
            engine: engine_label.clone(),
            cwd: cwd.clone(),
            branch: branch.map(str::to_string),
        });
    }
    send(state, &slot, message).await?;
    // Sidebars refresh on a title update, so the new chat appears at once.
    state
        .broadcast_all(ServerMsg::SessionTitleUpdated {
            session_id: id.clone(),
            title: title.to_string(),
        })
        .await;

    let place = match branch {
        Some(b) => format!("in worktree {} on branch {b}", cwd.display()),
        None => format!("in {}", cwd.display()),
    };
    Ok(format!(
        "Launched thread {id} (\"{title}\") on {engine_label} {place}. It runs on its own and \
         shows in the user's sidebar; approvals it needs wait for the user. Check on it with \
         thread_read or block until it finishes with thread_wait."
    ))
}

/// Whether `slot` has work in flight or waiting to be delivered.
async fn busy(slot: &SessionSlot) -> bool {
    if slot.is_foreground_running().await {
        return true;
    }
    slot.message_queue
        .snapshot()
        .await
        .iter()
        .any(|i| !i.delivered && i.error.is_none() && i.recovery.is_none())
}

/// The last thing the thread said, from whichever engine ran it last.
async fn latest_reply(state: &AppState, slot: &SessionSlot) -> Option<String> {
    let agent = slot.acp_launch.lock().await.is_some();
    if agent {
        let path = state.store.as_ref()?.agent_log_path(&slot.id)?;
        let lines = mira_acp::agent_sessions::read_lines(&path);
        let start = lines.iter().rposition(|l| l.get("user").is_some()).unwrap_or(0);
        let text = crate::session_engine::agent_lines_to_text(&lines[start..]);
        let reply: Vec<&str> = text
            .split("\n\n")
            .filter(|p| p.starts_with("Agent: "))
            .map(|p| p.trim_start_matches("Agent: "))
            .collect();
        return (!reply.is_empty()).then(|| reply.join("\n\n"));
    }
    slot.session
        .read()
        .await
        .transcript()
        .await
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && m.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .and_then(|m| m.content.clone())
}

fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        format!("[…]\n{}", s.chars().skip(n - max).collect::<String>())
    }
}

async fn describe(state: &AppState, t: &Launched) -> Value {
    let Some(slot) = state.slot(&SessionId::from(t.id.as_str())).await else {
        return json!({ "id": t.id, "title": t.title, "status": "closed" });
    };
    let queue = slot.message_queue.snapshot().await;
    let failed: Vec<String> = queue.iter().filter_map(|i| i.error.clone()).collect();
    let status = if !failed.is_empty() {
        "needs_attention"
    } else if busy(&slot).await {
        "running"
    } else {
        "idle"
    };
    json!({
        "id": t.id,
        "title": t.title,
        "engine": t.engine,
        "folder": t.cwd,
        "branch": t.branch,
        "status": status,
        "errors": failed,
        "latest_reply": latest_reply(state, &slot).await.map(|r| tail(&r, REPLY_CHARS)),
    })
}

/// `thread_list`: the threads this chat launched.
pub async fn list(state: &AppState, parent: &SessionSlot) -> Result<String, String> {
    let mut out = Vec::new();
    for t in launched_by(&parent.id.to_string()) {
        let mut d = describe(state, &t).await;
        if let Some(o) = d.as_object_mut() {
            o.remove("latest_reply");
        }
        out.push(d);
    }
    if out.is_empty() {
        return Ok("This chat hasn't launched any threads.".into());
    }
    Ok(serde_json::to_string_pretty(&out).unwrap_or_default())
}

fn thread_id(args: &Value) -> Result<&str, String> {
    args.get("thread_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "give `thread_id` (from thread_launch or thread_list)".into())
}

/// `thread_read`: status and latest reply.
pub async fn read(state: &AppState, parent: &SessionSlot, args: &Value) -> Result<String, String> {
    let t = find(&parent.id.to_string(), thread_id(args)?)?;
    Ok(serde_json::to_string_pretty(&describe(state, &t).await).unwrap_or_default())
}

/// `thread_wait`: block until the thread is idle (or the timeout), then read it.
pub async fn wait(state: &AppState, parent: &SessionSlot, args: &Value) -> Result<String, String> {
    let t = find(&parent.id.to_string(), thread_id(args)?)?;
    let secs = args
        .get("timeout")
        .and_then(Value::as_f64)
        .unwrap_or(300.0)
        .clamp(1.0, MAX_WAIT.as_secs_f64());
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(secs);
    // A just-queued message may not have started yet: give delivery a beat.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut finished = false;
    while tokio::time::Instant::now() < deadline {
        match state.slot(&SessionId::from(t.id.as_str())).await {
            Some(slot) if busy(&slot).await => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    slot.engine.activity_changed.notified(),
                )
                .await;
            }
            _ => {
                finished = true;
                break;
            }
        }
    }
    let mut d = describe(state, &t).await;
    if !finished {
        d["note"] = json!(format!("still running after {secs:.0}s; wait again or read it later"));
    }
    Ok(serde_json::to_string_pretty(&d).unwrap_or_default())
}

/// `thread_send`: a follow-up message, queued behind anything in flight.
pub async fn send_to(state: &AppState, parent: &SessionSlot, args: &Value) -> Result<String, String> {
    let t = find(&parent.id.to_string(), thread_id(args)?)?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .ok_or("`message` is empty")?;
    let slot = state
        .ensure_slot(&SessionId::from(t.id.as_str()))
        .await
        .map_err(|e| format!("could not open thread {}: {e}", t.id))?;
    send(state, &slot, message).await?;
    Ok(format!("Queued for thread {} (\"{}\").", t.id, t.title))
}

/// The thread tools' MCP definitions.
pub fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "thread_launch",
            "description": "Start a separate chat that works on its own, in parallel — for \
                            independent work like a second feature or one PR of a stack. Give it a \
                            `title` and the whole task in `message` (it doesn't see this \
                            conversation). With `worktree: {branch, base?}` it runs in its own git \
                            worktree on that branch, so its edits don't touch this checkout. \
                            `engine` is `mira` or an installed agent; default: the same engine as \
                            this chat. It shows in the user's sidebar. Use delegate_task instead \
                            for a quick read-only answer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "message": { "type": "string", "description": "The whole task" },
                    "engine": { "type": "string", "enum": crate::delegate::engines() },
                    "worktree": {
                        "type": "object",
                        "properties": {
                            "branch": { "type": "string", "description": "New or existing branch" },
                            "base": { "type": "string", "description": "Where a new branch starts (default: HEAD)" }
                        },
                        "required": ["branch"],
                        "additionalProperties": false
                    }
                },
                "required": ["title", "message"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "thread_list",
            "description": "The threads this chat launched, with their status.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "thread_read",
            "description": "A launched thread's status (running, idle, needs_attention) and its latest reply.",
            "inputSchema": {
                "type": "object",
                "properties": { "thread_id": { "type": "string" } },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "thread_wait",
            "description": "Wait until a launched thread finishes its current work, then return \
                            its status and latest reply. `timeout` in seconds (default 300, max 600).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "timeout": { "type": "number" }
                },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "thread_send",
            "description": "Send a launched thread a follow-up message (queued behind its current work).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "message": { "type": "string" }
                },
                "required": ["thread_id", "message"],
                "additionalProperties": false
            }
        }),
    ]
}

/// Route a thread tool call, or `None` when `name` isn't one.
pub async fn call(state: &AppState, slot: &Arc<SessionSlot>, name: &str, args: &Value) -> Option<Result<String, String>> {
    Some(match name {
        "thread_launch" => launch(state, slot, args).await,
        "thread_list" => list(state, slot).await,
        "thread_read" => read(state, slot, args).await,
        "thread_wait" => wait(state, slot, args).await,
        "thread_send" => send_to(state, slot, args).await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launched_threads_are_scoped_to_their_launcher() {
        registry().lock().unwrap().insert(
            "parent-a".into(),
            vec![Launched {
                id: "child-1".into(),
                title: "t".into(),
                engine: "codex".into(),
                cwd: PathBuf::from("/tmp"),
                branch: None,
            }],
        );
        assert!(find("parent-a", "child-1").is_ok());
        assert!(find("parent-b", "child-1").is_err());
        assert!(is_launched_thread("child-1"));
        assert!(!is_launched_thread("parent-a"));
    }

    #[tokio::test]
    async fn a_worktree_is_created_beside_the_repo_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("app");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"],
        ] {
            git(&repo, &args).await.unwrap();
        }
        let wt = ensure_worktree(&repo, "feature/ui", None).await.unwrap();
        // git reports the resolved path (`/private/var/…` on macOS).
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(wt, root.join("app-worktrees").join("feature-ui"));
        assert_eq!(git(&wt, &["branch", "--show-current"]).await.unwrap(), "feature/ui");
        assert_eq!(ensure_worktree(&repo, "feature/ui", None).await.unwrap(), wt);
        assert!(ensure_worktree(&repo, "bad..name", None).await.is_err());
    }
}
