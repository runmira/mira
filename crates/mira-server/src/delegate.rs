//! `delegate_task`: an agent hands a task to another engine — "have Codex
//! review what you just wrote", "ask Mira's own model to research this".
//!
//! The task runs in a new child chat (a subagent transcript: it has the
//! caller's chat as parent, so the sidebar leaves it out, but it's kept and
//! can be opened from the delegation card) in the same folder, on Mira's
//! own model or on an external agent. It's **read-only by construction**:
//! the child's background mode denies anything that would need approval,
//! so it can read, search and think, but not edit files or run commands.
//! A question it asks waits in its chat until the user opens it and
//! answers. Delegated chats can't delegate again.
//!
//! By default a task runs in the background (`mode: "async"`): the call
//! returns its id at once, `task_status` reads it, `task_cancel` stops it,
//! and when it finishes its result is queued into the parent chat — which
//! delivers it once the parent's own turn has ended — unless the parent
//! already read it with `task_status`. `mode: "wait"` blocks for the
//! answer instead, up to a wait budget that doesn't stop the task.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use mira_core::Role;
use serde::Serialize;

use crate::protocol::ServerMsg;
use crate::slot::{BackgroundMode, SessionSlot};
use crate::state::AppState;

/// Longest a delegated task may run before it's stopped.
const TASK_LIMIT: Duration = Duration::from_secs(60 * 60);
/// Default wait budget for `mode: "wait"`.
pub const WAIT_BUDGET: Duration = Duration::from_secs(10 * 60);
/// How much of a result the completion message quotes.
const RESULT_CHARS: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    /// Mira stopped while it ran.
    Interrupted,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        self != TaskStatus::Running
    }
}

/// One delegated task, as its parent chat and the card see it.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct DelegatedTask {
    pub id: String,
    pub parent_id: String,
    /// The helper's chat; open it to watch or answer its questions.
    pub child_id: String,
    pub engine: String,
    pub title: String,
    pub prompt: String,
    /// `read` (default) or `write`: a writing helper's approvals wait in
    /// its chat for the user.
    #[serde(default = "read_access")]
    pub access: String,
    /// The helper's own checkout, when it was given one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    pub status: TaskStatus,
    /// The answer, or why there isn't one.
    pub result: Option<String>,
    /// Unix ms.
    pub started_at: u64,
    pub finished_at: Option<u64>,
    /// The parent read the result (task_status) — no completion message.
    #[serde(skip)]
    acknowledged: bool,
    /// The queued completion message, if one was queued.
    #[serde(skip)]
    wake_id: Option<String>,
}

fn read_access() -> String {
    "read".into()
}

/// How a helper may work.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub title: Option<String>,
    /// May edit and run commands (each still needs the user's approval in
    /// its chat, unless the parent's policy already allows it).
    pub write: bool,
    /// Work in its own git worktree (a branch from the parent's HEAD), so
    /// its edits don't touch the parent's checkout.
    pub worktree: bool,
}

/// What's kept on disk: the wire shape plus the delivery bookkeeping the
/// wire leaves out.
#[derive(Serialize, serde::Deserialize)]
struct Stored {
    #[serde(flatten)]
    task: DelegatedTask,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default)]
    wake_id: Option<String>,
}

struct Entry {
    task: DelegatedTask,
    /// Stops the run.
    abort: Option<tokio::task::AbortHandle>,
    /// Fires when the task reaches a terminal state.
    done: tokio::sync::watch::Receiver<bool>,
}

fn tasks() -> &'static Mutex<HashMap<String, Entry>> {
    static T: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Parents whose saved tasks have been loaded this run.
fn loaded() -> &'static Mutex<std::collections::HashSet<String>> {
    static L: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// `<session>.tasks`, beside the chat's other state.
fn store_path(state: &AppState, parent: &str) -> Option<std::path::PathBuf> {
    state
        .store
        .as_ref()?
        .agent_log_path(&mira_core::SessionId::from(parent))
        .map(|p| p.with_extension("tasks"))
}

/// Bring `parent`'s saved tasks into memory, once. A task still running
/// when the last run ended was cut off: it comes back interrupted.
fn ensure_loaded(state: &AppState, parent: &str) {
    if !loaded()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(parent.to_string())
    {
        return;
    }
    let Some(path) = store_path(state, parent) else {
        return;
    };
    let saved: Vec<Stored> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut interrupted = false;
    let mut all = tasks().lock().unwrap_or_else(|e| e.into_inner());
    for s in saved {
        let mut task = s.task;
        task.acknowledged = s.acknowledged;
        task.wake_id = s.wake_id;
        if task.status == TaskStatus::Running {
            task.status = TaskStatus::Interrupted;
            task.result = Some("Mira stopped while this task was running.".into());
            task.finished_at = Some(now_ms());
            interrupted = true;
        }
        let (_, done) = tokio::sync::watch::channel(true);
        all.entry(task.id.clone()).or_insert(Entry {
            task,
            abort: None,
            done,
        });
    }
    drop(all);
    if interrupted {
        save(state, parent);
    }
}

/// Write `parent`'s tasks to disk.
fn save(state: &AppState, parent: &str) {
    let Some(path) = store_path(state, parent) else {
        return;
    };
    let stored: Vec<Stored> = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|e| e.task.parent_id == parent)
        .map(|e| Stored {
            task: e.task.clone(),
            acknowledged: e.task.acknowledged,
            wake_id: e.task.wake_id.clone(),
        })
        .collect();
    let tmp = path.with_extension("tasks.tmp");
    let ok = std::fs::write(&tmp, serde_json::to_vec(&stored).unwrap_or_default())
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = ok {
        tracing::warn!(%e, "delegated tasks: could not save");
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The tasks `parent` delegated, oldest first.
pub fn tasks_of(state: &AppState, parent: &str) -> Vec<DelegatedTask> {
    ensure_loaded(state, parent);
    let mut out: Vec<DelegatedTask> = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|e| e.task.parent_id == parent)
        .map(|e| e.task.clone())
        .collect();
    out.sort_by_key(|t| t.started_at);
    out
}

fn task_of(state: &AppState, parent: &str, id: &str) -> Result<DelegatedTask, String> {
    ensure_loaded(state, parent);
    tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .filter(|e| e.task.parent_id == parent)
        .map(|e| e.task.clone())
        .ok_or_else(|| format!("no delegated task `{id}` in this chat"))
}

/// Engines a task can go to: Mira's own model, or an external agent.
/// Driver kinds (`codex`) route at that kind's default instance; engine
/// registry ids (`codex-work`) route at the instance, with its own home,
/// key and environment. Anything else is rejected before a process
/// starts, so a typo cannot silently delegate to the ambient account.
pub fn engines() -> Vec<&'static str> {
    let mut out = vec!["mira"];
    out.extend([
        "claude-code",
        "codex",
        "opencode",
        "gemini",
        "grok",
        "cursor",
    ]);
    out.retain(|e| *e == "mira" || mira_acp::drivers::by_kind(e).is_some());
    out
}

/// Whether `engine` names something delegable: the static kinds above or
/// any external instance in the registry.
fn is_delegable(state: &AppState, engine: &str) -> bool {
    if engines().contains(&engine) {
        return true;
    }
    state
        .engines
        .current()
        .get(engine)
        .is_some_and(|i| !i.is_native())
}

/// Start `prompt` on `engine` in a child of `parent`.
///
/// `parent_call_id` is the `delegate_task` call id; every visible step the
/// child takes is re-broadcast on the parent's channel as a
/// `DelegateProgress` frame carrying it and the task id, so the card shows
/// live activity rather than a bare spinner.
pub async fn start(
    state: &AppState,
    parent: &Arc<SessionSlot>,
    prompt: &str,
    engine: Option<&str>,
    opts: Options,
    parent_call_id: &str,
) -> Result<DelegatedTask, String> {
    if parent.session.read().await.is_subagent() {
        return Err("a delegated task can't delegate again".into());
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("`prompt` is empty".into());
    }
    let engine = engine
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .unwrap_or("mira")
        .to_string();
    if !is_delegable(state, &engine) {
        // Name the static kinds in the error: registry ids are user
        // config, but the kinds are what a model author reaches for.
        return Err(format!(
            "unknown engine `{engine}` — use one of: {}",
            engines().join(", ")
        ));
    }

    let task_id = format!("task_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    // The child: same folder (or its own worktree) and model settings as
    // the parent, parented so it stays out of the sidebar. A read-only
    // helper is refused anything that needs approval; a writing one waits
    // in its chat for the user to approve.
    let parent_cwd = parent.cwd.read().await.clone();
    let worktree = if opts.worktree {
        let branch = format!("mira/{task_id}");
        Some(crate::agent_threads::ensure_worktree(&parent_cwd, &branch, None).await?)
    } else {
        None
    };
    let cwd = worktree.clone().unwrap_or(parent_cwd);
    let cfg = parent.session.read().await.config().await;
    let child = crate::slot::build_slot(cwd, cfg, None, &state.slot_deps()).await;
    {
        let s = child.session.read().await.clone();
        *child.session.write().await = s.with_parent_id(parent.id.clone());
    }
    *child.background_mode.write().await = if opts.write {
        BackgroundMode::Park
    } else {
        BackgroundMode::Deny
    };
    state.insert_slot(child.clone()).await;

    let title = opts
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let first = prompt.lines().next().unwrap_or(prompt);
            if first.chars().count() > 80 {
                format!("{}…", first.chars().take(80).collect::<String>())
            } else {
                first.to_string()
            }
        });
    let task = DelegatedTask {
        id: task_id,
        parent_id: parent.id.to_string(),
        child_id: child.id.to_string(),
        engine: engine.clone(),
        title,
        prompt: prompt.to_string(),
        access: if opts.write { "write" } else { "read" }.into(),
        worktree: worktree.as_ref().map(|p| p.display().to_string()),
        status: TaskStatus::Running,
        result: None,
        started_at: now_ms(),
        finished_at: None,
        acknowledged: false,
        wake_id: None,
    };
    let (done_tx, done_rx) = tokio::sync::watch::channel(false);
    tasks().lock().unwrap_or_else(|e| e.into_inner()).insert(
        task.id.clone(),
        Entry {
            task: task.clone(),
            abort: None,
            done: done_rx,
        },
    );
    save(state, &task.parent_id);
    publish(parent, &task);
    watch_questions(state, &task, &child);

    let brief = if opts.write {
        format!(
            "{prompt}\n\n(This is a delegated task. Make the changes it needs; actions that need \
             approval wait for the user. Finish with a summary of what you changed.)"
        )
    } else {
        format!(
            "{prompt}\n\n(This is a delegated, read-only task: you can read and search, but \
             edits and commands will be refused. Answer with your findings.)"
        )
    };
    let run = {
        let (state, parent, child) = (state.clone(), parent.clone(), child.clone());
        let (task_id, call_id) = (task.id.clone(), parent_call_id.to_string());
        tokio::spawn(async move {
            let tap = Tap {
                parent: parent.clone(),
                call_id,
                task_id: task_id.clone(),
            };
            let result = tokio::time::timeout(TASK_LIMIT, async {
                if engine == "mira" {
                    run_on_mira(&child, &brief, &tap).await
                } else {
                    run_on_agent(&state, &child, &engine, &brief, &tap).await
                }
            })
            .await
            .unwrap_or_else(|_| {
                Err(format!(
                    "the task took longer than {} minutes",
                    TASK_LIMIT.as_secs() / 60
                ))
            });
            let status = if result.is_ok() {
                TaskStatus::Completed
            } else {
                TaskStatus::Failed
            };
            let text = match result {
                Ok(t) | Err(t) => t,
            };
            settle(&state, &task_id, status, text, &child).await;
            let _ = done_tx.send(true);
        })
    };
    if let Some(e) = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&task.id)
    {
        e.abort = Some(run.abort_handle());
    }
    Ok(task)
}

/// Run a task and wait for it: the old blocking `delegate_task`. Kept for
/// callers that need the answer in the same turn.
pub async fn wait(
    state: &AppState,
    parent: &str,
    task_id: &str,
    budget: Duration,
) -> Result<DelegatedTask, String> {
    ensure_loaded(state, parent);
    let mut done = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(task_id)
        .filter(|e| e.task.parent_id == parent)
        .map(|e| e.done.clone())
        .ok_or_else(|| format!("no delegated task `{task_id}` in this chat"))?;
    let _ = tokio::time::timeout(budget, done.wait_for(|d| *d)).await;
    acknowledge_if_done(state, parent, task_id)
}

/// The task's current state; reading a finished one counts as receiving
/// its result, so no completion message follows.
pub fn status(state: &AppState, parent: &str, task_id: &str) -> Result<DelegatedTask, String> {
    acknowledge_if_done(state, parent, task_id)
}

fn acknowledge_if_done(
    state: &AppState,
    parent: &str,
    task_id: &str,
) -> Result<DelegatedTask, String> {
    let task = task_of(state, parent, task_id)?;
    if task.status.is_terminal() {
        let wake = {
            let mut all = tasks().lock().unwrap_or_else(|e| e.into_inner());
            all.get_mut(task_id).and_then(|e| {
                e.task.acknowledged = true;
                e.task.wake_id.take()
            })
        };
        save(state, parent);
        // Already queued: take it back out, the parent has the result.
        if let Some(id) = wake {
            let (state, parent) = (state.clone(), parent.to_string());
            tokio::spawn(async move {
                if let Some(slot) = state.slot_str(&parent).await {
                    let _ = slot.message_queue.remove(&id).await;
                }
            });
        }
    }
    Ok(task)
}

/// Stop a running task.
pub async fn cancel(state: &AppState, parent: &str, task_id: &str) -> Result<DelegatedTask, String> {
    let task = task_of(state, parent, task_id)?;
    if task.status.is_terminal() {
        return Ok(task);
    }
    let abort = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(task_id)
        .and_then(|e| e.abort.take());
    if let Some(a) = abort {
        a.abort();
    }
    let child = state.slot_str(&task.child_id).await;
    if let Some(child) = &child {
        settle(state, task_id, TaskStatus::Cancelled, "Stopped.".into(), child).await;
    }
    if let Some(e) = tasks().lock().unwrap_or_else(|e| e.into_inner()).get_mut(task_id) {
        e.task.acknowledged = true;
    }
    save(state, parent);
    task_of(state, parent, task_id)
}

/// Record a task's end, stop what's left of its child, tell the card, and
/// queue the result into the parent unless it was already read.
async fn settle(
    state: &AppState,
    task_id: &str,
    status: TaskStatus,
    result: String,
    child: &Arc<SessionSlot>,
) {
    let task = {
        let mut all = tasks().lock().unwrap_or_else(|e| e.into_inner());
        let Some(e) = all.get_mut(task_id) else {
            return;
        };
        if e.task.status.is_terminal() {
            return;
        }
        e.task.status = status;
        e.task.result = Some(result);
        e.task.finished_at = Some(now_ms());
        e.abort = None;
        e.task.clone()
    };
    save(state, &task.parent_id);
    // Nothing of the child keeps running; its chat stays to be opened.
    crate::acp_session::stop_agent(child).await;
    if let Some(h) = child.turn.lock().await.take() {
        h.abort();
    }
    // Answer anything it was still waiting on, so nothing stays parked.
    crate::approver::drain_pending_as_denied(&child.pending).await;
    if child.attached.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        state.remove_slot(&child.id).await;
    }
    let Some(parent) = state.slot_str(&task.parent_id).await else {
        return;
    };
    publish(&parent, &task);
    if status == TaskStatus::Cancelled {
        return;
    }
    // A `wait` call that's still blocked hands the result over itself;
    // give it the first look before queuing a message.
    tokio::task::yield_now().await;
    let unread = tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(task_id)
        .is_some_and(|e| !e.task.acknowledged);
    if !unread {
        return;
    }
    let wake_id = format!("delegated-{task_id}");
    let (engine, _, _) = parent.selection.snapshot();
    let item = crate::message_queue::QueuedInput {
        id: wake_id.clone(),
        text: completion_message(&task),
        images: Vec::new(),
        engine,
        dispatching: false,
        delivered: false,
        fingerprint: String::new(),
        error: None,
        recovery: None,
    };
    if parent.message_queue.enqueue(item).await.is_ok() {
        if let Some(e) = tasks().lock().unwrap_or_else(|e| e.into_inner()).get_mut(task_id) {
            e.task.wake_id = Some(wake_id);
        }
        save(state, &task.parent_id);
        crate::message_queue::wake(state, &parent);
    }
}

fn completion_message(task: &DelegatedTask) -> String {
    let result = task.result.as_deref().unwrap_or("").trim();
    let result = if result.chars().count() > RESULT_CHARS {
        format!(
            "{}…\n\n(Truncated; task_status with taskId {} has the full result.)",
            result.chars().take(RESULT_CHARS).collect::<String>(),
            task.id
        )
    } else {
        result.to_string()
    };
    let outcome = match task.status {
        TaskStatus::Completed => "finished",
        _ => "failed",
    };
    format!(
        "[Delegated task {} \"{}\" on {} {outcome}.]\n\n{result}",
        task.id, task.title, task.engine
    )
}

/// When the helper starts waiting on a question, tell the parent agent —
/// a message queued into the parent chat (delivered once its own turn has
/// ended) — so it can answer with `task_respond` or ask the user. Taken
/// back out if the question is answered first.
fn watch_questions(state: &AppState, task: &DelegatedTask, child: &Arc<SessionSlot>) {
    let (state, task_id, parent_id) = (state.clone(), task.id.clone(), task.parent_id.clone());
    let title = task.title.clone();
    let child_w = Arc::downgrade(child);
    let current: Arc<Mutex<Option<String>>> = Default::default();
    child.live_requests.on_change(move |waiting| {
        let (state, task_id, parent_id, title) =
            (state.clone(), task_id.clone(), parent_id.clone(), title.clone());
        let (child_w, current) = (child_w.clone(), current.clone());
        tokio::spawn(async move {
            let Some(parent) = state.slot_str(&parent_id).await else {
                return;
            };
            let previous = current.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(id) = previous {
                let _ = parent.message_queue.remove(&id).await;
            }
            if !waiting {
                return;
            }
            let Some(child) = child_w.upgrade() else {
                return;
            };
            let questions = pending_questions(&child);
            if questions.is_empty() {
                return;
            }
            let id = format!("delegated-q-{task_id}-{}", uuid::Uuid::new_v4().simple());
            let mut text = format!(
                "[Delegated task {task_id} \"{title}\" is waiting on a question.]\n"
            );
            for (request_id, proposal) in &questions {
                for q in &proposal.questions {
                    let options: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                    text.push_str(&format!("\n- {}", q.question));
                    if !options.is_empty() {
                        text.push_str(&format!(" (options: {})", options.join(", ")));
                    }
                }
                text.push_str(&format!(" — requestId {request_id}"));
            }
            text.push_str(&format!(
                "\n\nAnswer it with task_respond (taskId {task_id}) when the answer follows from \
                 the task or the user's instructions; otherwise ask the user, then answer it. If \
                 it's no longer pending, it was answered."
            ));
            let (engine, _, _) = parent.selection.snapshot();
            let item = crate::message_queue::QueuedInput {
                id: id.clone(),
                text,
                images: Vec::new(),
                engine,
                dispatching: false,
                delivered: false,
                fingerprint: String::new(),
                error: None,
                recovery: None,
            };
            if parent.message_queue.enqueue(item).await.is_ok() {
                *current.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
                crate::message_queue::wake(&state, &parent);
            }
        });
    });
}

/// The questions a helper's chat is waiting on: (request id, proposal).
fn pending_questions(child: &SessionSlot) -> Vec<(String, mira_tools::prompt::AskUserProposal)> {
    child
        .live_requests
        .frames()
        .into_iter()
        .filter_map(|f| match f {
            ServerMsg::AskUserRequest { prompt_id, proposal } => Some((prompt_id, proposal)),
            _ => None,
        })
        .collect()
}

/// What a task's helper is waiting on, for `task_requests`.
pub async fn requests(state: &AppState, parent: &str, task_id: &str) -> Result<String, String> {
    let task = task_of(state, parent, task_id)?;
    let Some(child) = state.slot_str(&task.child_id).await.filter(|_| !task.status.is_terminal())
    else {
        return Ok(format!("Task {task_id} isn't running, so it isn't waiting on anything."));
    };
    let questions = pending_questions(&child);
    let approvals = child
        .live_requests
        .frames()
        .iter()
        .filter(|f| matches!(f, ServerMsg::ApprovalRequest { .. }))
        .count();
    if questions.is_empty() && approvals == 0 {
        return Ok(format!("Task {task_id} isn't waiting on anything."));
    }
    let mut out = String::new();
    for (id, proposal) in &questions {
        out.push_str(&format!("Question requestId {id}:\n"));
        for (i, q) in proposal.questions.iter().enumerate() {
            out.push_str(&format!("  {}. {}", i + 1, q.question));
            if !q.options.is_empty() {
                let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                out.push_str(&format!(" [options: {}]", labels.join(" | ")));
            }
            if q.multi_select {
                out.push_str(" (multi-select)");
            }
            out.push('\n');
        }
    }
    if approvals > 0 {
        out.push_str(&format!(
            "{approvals} action(s) wait for the user's approval in the task's chat; only the user can approve them.\n"
        ));
    }
    Ok(out)
}

/// Answer a question the helper asked, as the parent agent.
pub async fn respond(
    state: &AppState,
    parent: &str,
    task_id: &str,
    request_id: &str,
    answers: Vec<mira_tools::prompt::AskUserAnswer>,
) -> Result<(), String> {
    use mira_tools::prompt::{AskUserResponse, PromptResponse};
    let task = task_of(state, parent, task_id)?;
    let child = state
        .slot_str(&task.child_id)
        .await
        .filter(|_| !task.status.is_terminal())
        .ok_or_else(|| format!("task {task_id} isn't running"))?;
    let Some((_, proposal)) = pending_questions(&child).into_iter().find(|(id, _)| id == request_id)
    else {
        return Err(format!("no pending question `{request_id}` in task {task_id}; see task_requests"));
    };
    if answers.len() != proposal.questions.len() {
        return Err(format!(
            "give one answer per question ({} question(s), {} answer(s))",
            proposal.questions.len(),
            answers.len()
        ));
    }
    let response = PromptResponse::AskUser(AskUserResponse {
        answers,
        cancelled: false,
    });
    if !crate::interactive::resolve(&child.prompt_pending, request_id, response.clone()).await {
        return Err("that question was just answered or closed".into());
    }
    // Close the card in any window showing the helper's chat.
    let _ = child.events_tx.send(ServerMsg::PromptResolved {
        prompt_id: request_id.to_string(),
        response,
    });
    Ok(())
}

/// Tell the parent's windows (the card) about a task's state.
fn publish(parent: &SessionSlot, task: &DelegatedTask) {
    let _ = parent.events_tx.send(ServerMsg::DelegateUpdate { task: task.clone() });
}

/// What a task's tool result says about it.
pub fn describe(task: &DelegatedTask) -> String {
    match task.status {
        TaskStatus::Running => format!(
            "Delegated task {} started on {} (it runs in its own chat, read-only). Its result \
             will be delivered to this chat when it finishes; task_status with taskId {} reads \
             it sooner, task_cancel stops it.",
            task.id, task.engine, task.id
        ),
        TaskStatus::Completed => task.result.clone().unwrap_or_default(),
        TaskStatus::Failed => format!("The task failed: {}", task.result.as_deref().unwrap_or("")),
        TaskStatus::Cancelled => format!("Task {} was stopped.", task.id),
        TaskStatus::Interrupted => format!(
            "Task {} was cut off when Mira stopped; delegate it again if it's still needed.",
            task.id
        ),
    }
}

/// Where the child's steps go: the parent's channel, tagged with the
/// parent's call id and the task id.
struct Tap {
    parent: Arc<SessionSlot>,
    call_id: String,
    task_id: String,
}

/// Re-broadcast one child step on the parent's channel. Fire-and-forget:
/// with no client attached (a background session) the send just drops.
fn emit_progress(tap: &Tap, kind: &str, text: &str) {
    let _ = tap.parent.events_tx.send(ServerMsg::DelegateProgress {
        call_id: tap.call_id.clone(),
        task_id: Some(tap.task_id.clone()),
        kind: kind.to_owned(),
        text: text.to_owned(),
    });
}

async fn run_on_mira(child: &Arc<SessionSlot>, task: &str, tap: &Tap) -> Result<String, String> {
    let sess = child.session.read().await.clone();
    let mut stream = sess.send_with_images(task.to_string(), Vec::new()).await;
    // Each harness event the child produces is inspected for a visible step
    // (a tool call, a warning) and forwarded to the parent's card. Token and
    // usage frames are skipped — the card shows steps, not a livestream.
    while let Some(evt) = stream.next().await {
        if let Some((kind, text)) = harness_activity(&evt) {
            emit_progress(tap, &kind, &text);
        }
    }
    let transcript = sess.transcript().await;
    transcript
        .iter()
        .rev()
        .find(|m| {
            m.role == Role::Assistant && m.content.as_deref().is_some_and(|c| !c.trim().is_empty())
        })
        .and_then(|m| m.content.clone())
        .ok_or_else(|| "Mira's model finished without an answer (is a provider set up?)".into())
}

async fn run_on_agent(
    state: &AppState,
    child: &Arc<SessionSlot>,
    engine: &str,
    task: &str,
    tap: &Tap,
) -> Result<String, String> {
    let params = agent_launch_params(state, engine)?;
    let driver = params.driver_kind.clone();
    let mut rx = child.events_tx.subscribe();
    let handle = crate::acp_session::start_agent(state, child, &params, None).await?;
    let agent = handle
        .agent()
        .await
        .ok_or_else(|| format!("{driver} stopped before it started"))?;
    let mut prompted = {
        let agent = agent.clone();
        let task = task.to_string();
        tokio::spawn(async move { agent.prompt_text(&task).await })
    };
    let mut prompt_done = false;
    let mut answer_messages = Vec::new();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(frame @ (ServerMsg::AcpText { .. } | ServerMsg::AcpTextSnapshot { .. })) => collect_agent_answer(&mut answer_messages, &frame),
                // The agent's own tool calls, forwarded as card activity.
                Ok(ServerMsg::AcpToolCall { call }) => {
                    let (k, text) = acp_activity(&call);
                    if !text.is_empty() {
                        emit_progress(tap, &k, &text);
                    }
                }
                Ok(ServerMsg::AcpTurnEnd { .. }) => break,
                Ok(ServerMsg::Error { text }) => return Err(text),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            // An ACP agent's turn ends when its prompt returns (a native
            // agent's returns at once and ends with a turn-end event).
            r = &mut prompted, if !prompt_done => {
                prompt_done = true;
                match r {
                    Ok(Ok(Some(_))) => break,
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => return Err(e),
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
    }
    // Text already sent before the end was noticed.
    while let Ok(m) = rx.try_recv() {
        collect_agent_answer(&mut answer_messages, &m);
    }
    let answer = answer_messages
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string();
    if answer.is_empty() {
        Err(format!("{driver} finished without an answer"))
    } else {
        Ok(answer)
    }
}

/// Launch parameters for an external engine. An instance id resolves to its
/// own config (home, key, env); a bare driver kind falls back to that kind's
/// default instance. Either way the agent inherits the instance's credential
/// boundary rather than ambient — work must not bill a stranger's account.
pub(crate) fn agent_launch_params(
    state: &AppState,
    engine: &str,
) -> Result<crate::acp_session::AcpLaunchParams, String> {
    Ok(
        match state
            .engines
            .current()
            .get(engine)
            .filter(|i| !i.is_native())
        {
            Some(inst) => {
                let kind = inst.driver.to_string();
                let cfg = state
                    .engines
                    .current()
                    .external_driver_config(engine)
                    .unwrap_or_default();
                crate::acp_session::AcpLaunchParams::for_instance(engine, kind, cfg)
            }
            None => {
                let resolved = crate::acp_session::resolve_start_params(None, None, Some(engine))?;
                crate::acp_session::AcpLaunchParams::for_instance(
                    resolved.instance,
                    resolved.kind,
                    resolved.cfg,
                )
            }
        },
    )
}

/// Whether `engine` can run a task: `mira` or an external agent.
pub(crate) fn is_engine(state: &AppState, engine: &str) -> bool {
    is_delegable(state, engine)
}

/* ---------- activity summarizers ---------- */

/// A visible step from one of the child's own harness events: a tool call it
/// dispatched, or a warning it raised. `None` for events with nothing to
/// show (`Token`, `Usage`, `Done`, …).
fn harness_activity(evt: &mira_harness::HarnessEvent) -> Option<(String, String)> {
    use mira_harness::HarnessEvent as E;
    match evt {
        E::ToolStart(call) => {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
            let (kind, mut text) = tool_activity(&call.function.name, &args);
            if let Some(target) = tool_target(&call.function.name, &args) {
                text.push(' ');
                text.push_str(&target);
            }
            Some((kind.to_string(), text))
        }
        E::Warning(w) => Some(("note".into(), w.clone())),
        _ => None,
    }
}

/// The coarse bucket and verb for one of Mira's own tools. Kept in the same
/// vocabulary as the frontend's `categoryFor` / `infoFor` so a delegated step
/// reads exactly like a step Mira ran itself.
fn tool_activity(name: &str, _args: &serde_json::Value) -> (&'static str, String) {
    let pair = match name {
        "read_file" => ("read", "Read"),
        "write_file" => ("write", "Wrote"),
        "edit_file" => ("edit", "Edited"),
        "grep" => ("search", "Searched for"),
        "glob" => ("search", "Found files matching"),
        "find_symbol" => ("search", "Found"),
        "find_references" => ("search", "Found references to"),
        "find_callers" => ("search", "Found callers of"),
        "web_search" => ("search", "Searched the web for"),
        "web_fetch" => ("fetch", "Fetched"),
        "bash" => ("run", "Ran"),
        "memory_read" | "memory_search" => ("read", "Recalled"),
        "memory_append" | "memory_edit" | "memory_remember" => ("write", "Remembered"),
        "task_list" => ("read", "Listed tasks"),
        "task_get" => ("read", "Read task"),
        _ => ("other", "Used"),
    };
    (pair.0, pair.1.to_string())
}

/// The human-readable target of a tool call — the path, pattern, command or
/// query its row would show. Empty string when the tool has no natural one.
fn tool_target(name: &str, args: &serde_json::Value) -> Option<String> {
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let short_path = |p: String| match p.rsplit_once('/') {
        Some((_, base)) if p.matches('/').count() >= 2 => format!("…/{base}"),
        _ => p,
    };
    let val = match name {
        "read_file" | "write_file" | "edit_file" => s(&args["path"]).map(short_path),
        "grep" | "glob" | "find_symbol" | "find_references" | "find_callers" | "web_search" => {
            s(&args["pattern"])
                .or_else(|| s(&args["query"]))
                .or_else(|| s(&args["name"]))
        }
        "bash" => s(&args["command"]).map(|c| {
            let one = c.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.chars().count() > 60 {
                format!("{}…", one.chars().take(60).collect::<String>())
            } else {
                one
            }
        }),
        "web_fetch" => s(&args["url"]),
        "memory_read" | "memory_append" | "memory_edit" | "memory_remember" | "memory_search" => {
            s(&args["path"]).or_else(|| s(&args["query"]))
        }
        _ => None,
    };
    val.filter(|v| !v.is_empty())
}

/// One of an external agent's tool calls, as a step for the card. Uses the
/// agent's own `title` when it gave one (`Edit src/main.rs`), else the kind.
fn acp_activity(call: &mira_acp::events::ToolCallState) -> (String, String) {
    use mira_acp::events::AcpToolKind;
    let title = call.title.trim();
    if title.is_empty() {
        return ("other".into(), String::new());
    }
    let kind = match call.kind {
        Some(AcpToolKind::Read) => "read",
        Some(AcpToolKind::Edit) => "edit",
        Some(AcpToolKind::Delete) | Some(AcpToolKind::Move) => "write",
        Some(AcpToolKind::Search) => "search",
        Some(AcpToolKind::Execute) => "run",
        Some(AcpToolKind::Fetch) => "fetch",
        _ => "other",
    };
    (kind.into(), title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mira_is_always_an_engine_and_unknown_ones_are_not() {
        let e = engines();
        assert_eq!(e[0], "mira");
        assert!(e
            .iter()
            .all(|k| *k == "mira" || mira_acp::drivers::by_kind(k).is_some()));
        assert!(!e.contains(&"nonsense"));
    }

    #[test]
    fn a_read_step_names_its_file_in_the_card_vocabulary() {
        let call = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new(
                "read_file",
                r#"{"path":"crates/mira-auth/src/loopback.rs"}"#,
            ),
        };
        let (kind, text) =
            harness_activity(&mira_harness::HarnessEvent::ToolStart(call)).expect("a step");
        assert_eq!(kind, "read");
        assert!(text.starts_with("Read "), "{text}");
        // Long paths get the same `…/tail` shortening the tool rows use.
        assert!(text.ends_with("loopback.rs"), "{text}");
    }

    #[test]
    fn a_search_step_quotes_its_pattern_and_a_bash_step_its_command() {
        let grep = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new("grep", r#"{"pattern":"bind_any"}"#),
        };
        let (kind, text) = harness_activity(&mira_harness::HarnessEvent::ToolStart(grep)).unwrap();
        assert_eq!(kind, "search");
        assert_eq!(text, "Searched for bind_any");

        let bash = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new("bash", r#"{"command":"cargo test"}"#),
        };
        let (kind, text) = harness_activity(&mira_harness::HarnessEvent::ToolStart(bash)).unwrap();
        assert_eq!(kind, "run");
        assert_eq!(text, "Ran cargo test");
    }

    fn task(status: TaskStatus, result: &str) -> DelegatedTask {
        DelegatedTask {
            id: "task_abc".into(),
            parent_id: "p".into(),
            child_id: "c".into(),
            engine: "codex".into(),
            title: "Review the diff".into(),
            prompt: "Review the diff".into(),
            access: "read".into(),
            worktree: None,
            status,
            result: Some(result.into()),
            started_at: 0,
            finished_at: None,
            acknowledged: false,
            wake_id: None,
        }
    }

    #[test]
    fn saved_tasks_keep_their_delivery_bookkeeping() {
        let mut t = task(TaskStatus::Completed, "done");
        t.wake_id = Some("delegated-task_abc".into());
        let stored = Stored {
            task: t.clone(),
            acknowledged: true,
            wake_id: t.wake_id.clone(),
        };
        let back: Stored = serde_json::from_slice(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(back.acknowledged);
        assert_eq!(back.wake_id.as_deref(), Some("delegated-task_abc"));
        assert_eq!(back.task.status, TaskStatus::Completed);
        // The wire shape (what the card gets) leaves the bookkeeping out.
        let wire = serde_json::to_value(&t).unwrap();
        assert!(wire.get("acknowledged").is_none() && wire.get("wake_id").is_none());
    }

    #[test]
    fn a_started_task_points_at_status_and_cancel() {
        let d = describe(&task(TaskStatus::Running, ""));
        assert!(d.contains("task_abc") && d.contains("task_status") && d.contains("task_cancel"), "{d}");
    }

    #[test]
    fn the_completion_message_names_the_task_and_truncates_long_results() {
        let m = completion_message(&task(TaskStatus::Completed, "looks good"));
        assert!(m.starts_with("[Delegated task task_abc \"Review the diff\" on codex finished.]"), "{m}");
        assert!(m.ends_with("looks good"));
        let long = "x".repeat(RESULT_CHARS + 10);
        let m = completion_message(&task(TaskStatus::Failed, &long));
        assert!(m.contains("failed.]") && m.contains("Truncated; task_status"), "{m}");
    }

    #[test]
    fn token_frames_are_not_steps() {
        assert!(harness_activity(&mira_harness::HarnessEvent::Token("hi".into())).is_none());
        assert!(harness_activity(&mira_harness::HarnessEvent::Done).is_none());
    }
}

/// Reconcile native messages for delegated answers just as the chat does.
fn collect_agent_answer(messages: &mut Vec<(Option<String>, String)>, frame: &ServerMsg) {
    let (id, text, snapshot) = match frame {
        ServerMsg::AcpText { message_id, text } => (message_id.clone(), text, false),
        ServerMsg::AcpTextSnapshot { message_id, text } => (Some(message_id.clone()), text, true),
        _ => return,
    };
    let existing = if id.is_some() {
        messages.iter_mut().find(|(key, _)| *key == id)
    } else {
        messages.last_mut().filter(|(key, _)| key.is_none())
    };
    if let Some((_, content)) = existing {
        if snapshot {
            *content = text.clone();
        } else {
            content.push_str(text);
        }
    } else {
        messages.push((id, text.clone()));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn partial_delegated_answers_are_replaced_without_duplicates() {
        let mut messages = Vec::new();
        collect_agent_answer(
            &mut messages,
            &ServerMsg::AcpText {
                message_id: Some("one".into()),
                text: "partial".into(),
            },
        );
        let complete = ServerMsg::AcpTextSnapshot {
            message_id: "one".into(),
            text: "full answer".into(),
        };
        collect_agent_answer(&mut messages, &complete);
        collect_agent_answer(&mut messages, &complete);
        assert_eq!(messages, vec![(Some("one".into()), "full answer".into())]);
    }
}
