//! Sessions API — list live + persisted sessions, create/load/delete,
//! set titles, toggle background mode.
//!
//! ## Multi-session model
//!
//! Sessions the server has *loaded* live in `state.slots` as
//! [`SessionSlot`](crate::slot::SessionSlot) instances, each with its own
//! broadcast bus. Sessions the user has never opened this run still exist
//! as [`SessionRecord`]s on disk; loading one materializes a slot for it
//! via [`crate::slot::build_slot`].
//!
//! Endpoints:
//! - `GET    /api/sessions`                — summary list (live + persisted),
//!   with `running` and `attached`
//!   hints for the sidebar.
//! - `GET    /api/sessions/:id/history`    — persisted history for
//!   rehydrating a subagent panel
//!   on reload.
//! - `GET    /api/sessions/:id/preview`    — the few fields a sidebar
//!   hover card shows, without the transcript.
//! - `POST   /api/sessions/:id/load`       — ensure a slot exists for `id`
//!   and mark it active.
//! - `POST   /api/sessions/new`            — create a fresh slot.
//! - `DELETE /api/sessions/:id`            — abort turn, drop slot,
//!   delete record.
//! - `PATCH  /api/sessions/:id/title`      — manual rename.
//! - `POST   /api/sessions/:id/title/regenerate` — AI rename.
//! - `PUT    /api/sessions/:id/background` — swap background mode.
//! - `PUT    /api/sessions/:id/flags`      — pin / archive / restore.

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::RuntimeState;
use mira_core::SessionId;
use mira_harness::{SessionConfig, SessionRecord};
use mira_harness::persist::SessionListRecord;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::protocol::ServerMsg;
use crate::slot::BackgroundMode;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub id: String,
    pub model: String,
    pub cwd: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    /// Driver slug when an external agent drove turns here, for the sidebar
    /// badge. Absent for harness-only sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_driver: Option<String>,
    /// True when this session is the server's `active` pointer (HTTP
    /// handlers without a session_id target it).
    pub active: bool,
    /// True when a WS forwarder is currently subscribed to this slot's
    /// event stream. Sidebar renders a small "•" indicator.
    #[serde(default)]
    pub attached: bool,
    /// True when a turn task is currently in flight on this slot. The UI
    /// shows a running spinner so background sessions announce their
    /// state without an active client.
    #[serde(default)]
    pub running: bool,
    /// Background-mode policy for this slot when no client is attached.
    /// Only populated for live slots; persisted sessions default to the
    /// safe fallback when loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_mode: Option<BackgroundMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_status: Option<WorktreeMergeStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// Sidebar pin — the UI floats pinned sessions to the top of their
    /// project group. Persisted on the record, not client state.
    #[serde(default)]
    pub pinned: bool,
    /// True when the user archived this session; excluded from the
    /// default list (`?archived=true` returns only these).
    #[serde(default)]
    pub archived: bool,
    #[serde(skip_serializing_if = "SessionUsageView::is_empty")]
    pub usage: SessionUsageView,
    /// Made with "Fork from here": the chat it branched off (the sidebar
    /// nests it there) and the message it was taken at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_at: Option<String>,
    /// Waiting on the user (an approval, a question, a plan, a secret).
    /// Only known for chats loaded in memory.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_attention: bool,
    /// The chat that launched this one with `thread_launch`, so the sidebar
    /// can nest it there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_by: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct SessionUsageView {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_input_tokens: u64,
    pub rounds: u32,
}

impl SessionUsageView {
    fn is_empty(&self) -> bool {
        self.prompt_tokens == 0 && self.completion_tokens == 0 && self.rounds == 0
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeMergeStatus {
    Merged,
    Unmerged,
}

const FIRST_MSG_TRUNC: usize = 80;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_children: bool,
    #[serde(default)]
    pub all: bool,
    /// `?archived=true` returns only archived sessions (the sidebar's
    /// "Archived" view); the default list excludes them.
    #[serde(default)]
    pub archived: bool,
}

/// Sessions from the agents' own history dirs (currently Claude Code),
/// resumable in Mira via `acp_start` with `resume`. Read-only and best
/// effort: a missing dir lists nothing rather than failing.
/// The agent side of a Ready frame: sidecar transcript plus which driver it
/// belongs to. Shared by every Ready constructor so attach, cwd-switch and
/// the socket handshake replay identically — three call sites building the
/// same frame by hand is how one of them silently drops the transcript.
pub async fn read_agent_state(
    store: Option<&std::sync::Arc<dyn mira_harness::SessionStore>>,
    sess: &mira_harness::Session,
) -> (Vec<serde_json::Value>, Option<String>) {
    let Some(store) = store else {
        return (Vec::new(), None);
    };
    let sid = sess.id.clone();
    let transcript = store
        .agent_log_path(&sid)
        .map(|p| mira_acp::agent_sessions::read_lines(&p))
        .unwrap_or_default();
    // In-memory meta wins (just started, not yet checkpointed); the sidecar
    // is the fallback for slots rebuilt from disk.
    let driver = sess.agent_meta().await.map(|a| a.driver_kind).or_else(|| {
        transcript
            .iter()
            .rev()
            .find_map(|l| l.get("driver").and_then(|d| d.as_str()).map(str::to_string))
    });
    (transcript, driver)
}

/// Turns recorded for an agent session, oldest first, for the revert
/// picker. Read from the sidecar rather than any live process: revert
/// targets recorded history, which outlives processes. Empty when nothing
/// was recorded or persistence is off.
pub async fn list_agent_turns(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(sid) = q.get("session") else {
        return err(StatusCode::BAD_REQUEST, "missing ?session=<id>".to_string());
    };
    let turns = crate::acp_session::agent_turns(sid, state.store.as_ref());
    Json(turns).into_response()
}

#[derive(serde::Deserialize)]
pub struct RevertRequest {
    session_id: String,
    turn: u64,
}

/// Revert an agent session to before a turn ran: files restored, transcript
/// truncated, agent restarted fresh. See `revert_agent_turn` for the exact
/// contract and its honest limits.
pub async fn revert_agent_turn(
    State(state): State<AppState>,
    Json(req): Json<RevertRequest>,
) -> Response {
    let Some(slot) = state
        .slots
        .read()
        .await
        .get(&SessionId::from(req.session_id.as_str()))
        .cloned()
    else {
        return err(StatusCode::NOT_FOUND, "no such session".to_string());
    };
    match crate::acp_session::revert_agent_turn(&state, &slot, req.turn).await {
        Ok(note) => Json(serde_json::json!({ "ok": true, "note": note })).into_response(),
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

pub async fn list_external_agent_sessions() -> Response {
    // Capped: history accumulates without bound, and the panel shows the
    // most recent slice.
    let mut sessions = mira_acp::agent_sessions::list_claude_sessions();
    sessions.truncate(50);
    axum::Json(sessions).into_response()
}

pub async fn list_sessions(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let Some(store) = state.store.clone() else {
        // No persistence — return only live slots. Cheap loop; a normal
        // server has O(1) slots at any time.
        let live: Vec<SessionSummary> = summarize_live(&state).await;
        return Json(live).into_response();
    };
    let cwd = if q.all || q.archived {None} else {Some(state.current_cwd().await)};
    let records = match store.list_metadata(cwd.as_deref(), q.archived, q.include_children, if q.all || q.archived {200} else {50}).await {
        Ok(records) => records,
        Err(error) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {error}")),
    };
    let active_id = state.active.read().await.to_string();

    // Cache live-slot metadata so we can annotate persisted rows without a
    // fresh read of state.slots per iteration.
    let live_meta = live_slot_metadata(&state).await;

    // Persisted records → summaries.
    let mut summaries: Vec<SessionSummary> = records
        .into_iter()
        .filter(|r| q.include_children || r.parent_id.is_none())
        .map(|r| summarize_metadata(&r, &active_id, &live_meta))
        .collect();

    // Any live slot the user just started that hasn't yet been checkpointed
    // to disk (or is running under `--no-persist`) still needs to show up
    // in the sidebar. Add slots whose id isn't in the persisted list.
    //
    // Only slots that were never saved: a saved one missing from this list
    // was filtered out on purpose (another folder, archived, a subagent).
    // And never in the archived view — an unsaved chat can't be archived;
    // adding them there listed every open chat as "Untitled", dated 1970.
    let known_ids: std::collections::HashSet<String> =
        summaries.iter().map(|s| s.id.clone()).collect();
    let live_slots = if q.archived { Vec::new() } else { state.list_slots().await };
    for slot in live_slots {
        let id = slot.id.to_string();
        if known_ids.contains(&id) {
            continue;
        }
        let slot_cwd = slot.cwd.read().await.clone();
        if cwd.as_deref().is_some_and(|c| c != slot_cwd.as_path()) {
            continue;
        }
        let sess = slot.session.read().await.clone();
        if sess.is_subagent() && !q.include_children {
            continue;
        }
        if store.load(&slot.id).await.is_ok() {
            continue;
        }
        let now = mira_harness::persist::now_secs();
        let message_count = sess
            .transcript()
            .await
            .iter()
            .filter(|m| m.role == mira_core::Role::User)
            .count();
        summaries.push(SessionSummary {
            parent_id: None,
            id: id.clone(),
            model: sess.config().await.model,
            cwd: slot_cwd.display().to_string(),
            // Live and unsaved: it's happening now.
            created_at: now,
            updated_at: now,
            message_count,
            title: sess.title().await,
            first_user_message: None,
            active: id == active_id,
            attached: slot.is_attached(),
            running: slot.is_running().await,
            background_mode: Some(*slot.background_mode.read().await),
            worktree_status: None,
            worktree_branch: None,
            pinned: false,
            archived: false,
            agent_driver: slot
                .acp_launch
                .lock()
                .await
                .as_ref()
                .map(|p| p.driver_kind.clone()),
            usage: SessionUsageView::default(),
            forked_from: None,
            forked_at: None,
            needs_attention: false,
            launched_by: crate::agent_threads::launcher_of(&id),
        });
    }
    enrich_worktrees(&mut summaries).await;
    Json(summaries).into_response()
}

/// Metadata about currently-live slots, snapshotted in one pass so we don't
/// re-lock `state.slots` per record.
struct LiveMeta {
    attached: bool,
    running: bool,
    background_mode: BackgroundMode,
    /// The agent this live session runs on, if any — fresher than the
    /// record, which is only rewritten on checkpoint.
    agent_driver: Option<String>,
    needs_attention: bool,
}

async fn live_slot_metadata(state: &AppState) -> std::collections::HashMap<String, LiveMeta> {
    let mut out = std::collections::HashMap::new();
    for slot in state.list_slots().await {
        out.insert(
            slot.id.to_string(),
            LiveMeta {
                attached: slot.is_attached(),
                running: slot.is_running().await,
                background_mode: *slot.background_mode.read().await,
                needs_attention: slot.needs_attention().await,
                agent_driver: slot
                    .acp_launch
                    .lock()
                    .await
                    .as_ref()
                    .map(|p| p.driver_kind.clone()),
            },
        );
    }
    out
}

async fn summarize_live(state: &AppState) -> Vec<SessionSummary> {
    let active_id = state.active.read().await.to_string();
    let mut out = Vec::new();
    for slot in state.list_slots().await {
        let id = slot.id.to_string();
        let sess = slot.session.read().await;
        // The session's engine right now, from memory: the checkpoint may
        // predate the pick, and the row must badge correctly right now.
        let agent_driver = slot
            .acp_launch
            .lock()
            .await
            .as_ref()
            .map(|p| p.driver_kind.clone());
        out.push(SessionSummary {
            parent_id: None,
            id: id.clone(),
            model: sess.config().await.model,
            cwd: slot.cwd.read().await.display().to_string(),
            created_at: 0,
            updated_at: 0,
            message_count: 0,
            title: sess.title().await,
            first_user_message: None,
            active: id == active_id,
            attached: slot.is_attached(),
            running: slot.is_running().await,
            background_mode: Some(*slot.background_mode.read().await),
            worktree_status: None,
            worktree_branch: None,
            pinned: false,
            archived: false,
            agent_driver,
            usage: SessionUsageView::default(),
            forked_from: None,
            forked_at: None,
            needs_attention: false,
            launched_by: crate::agent_threads::launcher_of(&id),
        });
    }
    out
}

#[derive(Debug, Serialize)]
pub struct SessionHistoryView {
    pub id: String,
    pub model: String,
    pub cwd: String,
    pub title: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<mira_core::Message>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub previews: std::collections::HashMap<String, mira_tools::DiffPreview>,
    /// The agent transcript sidecar, oldest first. Raw persisted lines —
    /// the client replays them through its live frame handler, so replayed
    /// turns render exactly like live ones. Empty for harness-only sessions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_transcript: Vec<serde_json::Value>,
    /// Which agent that transcript belongs to, for badges and resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_driver: Option<String>,
}

pub async fn get_session_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — no history available".to_string(),
        );
    };
    let record = match store.load(&SessionId::from(id.as_str())).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };
    let agent_transcript = store
        .agent_log_path(&record.id)
        .map(|p| mira_acp::agent_sessions::read_lines(&p))
        .unwrap_or_default();
    let view = SessionHistoryView {
        id: record.id.to_string(),
        model: record.cfg.model.clone(),
        cwd: record.cwd.display().to_string(),
        title: record.title.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at,
        messages: record.messages,
        previews: record.previews,
        agent_driver: record.agent.as_ref().map(|a| a.driver_kind.clone()),
        agent_transcript,
    };
    Json(view).into_response()
}

/// The handful of fields a sidebar hover card needs: when the chat was
/// last touched and how its last exchange ended.
///
/// A separate endpoint rather than a slice of `/history` on purpose —
/// that one carries every message and every diff preview in the chat,
/// which is hundreds of kilobytes for a long session and would be
/// fetched on every hover. This reads the same record and answers in
/// constant size.
#[derive(Debug, Serialize)]
pub struct SessionPreviewView {
    pub id: String,
    /// Last thing the *user* said, attachments stripped — what you were
    /// doing. `None` for a session that has only assistant turns.
    pub last_user: Option<String>,
    /// Last thing the agent said, or the last tool call it made when it
    /// said nothing.
    pub last_assistant: Option<String>,
    /// Tool name for the final turn when that turn was a tool call
    /// rather than prose, so the card can say "Ran cargo test" instead of
    /// showing nothing.
    pub last_tool: Option<String>,
    pub message_count: usize,
    pub updated_at: u64,
}

pub async fn get_session_preview(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(StatusCode::BAD_REQUEST, "persistence disabled".to_string());
    };
    let record = match store.load(&SessionId::from(id.as_str())).await {
        Ok(r) => r,
        // A live slot the app never persisted (a chat that hasn't been
        // flushed yet) has no record; the card simply shows less.
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };

    // Walk backwards and take the first of each — cheap regardless of
    // how long the chat is, and it never copies the whole transcript.
    let mut last_user = None;
    let mut last_assistant = None;
    let mut last_tool = None;
    for m in record.messages.iter().rev() {
        if last_assistant.is_none() && m.role == mira_core::Role::Assistant {
            let text = m.content.as_deref().unwrap_or("").trim();
            if !text.is_empty() {
                last_assistant = Some(text.chars().take(600).collect());
            }
        }
        if last_user.is_none() && m.role == mira_core::Role::User {
            let text = m.content.as_deref().unwrap_or("").trim();
            if !text.is_empty() {
                last_user = Some(text.chars().take(400).collect());
            }
        }
        if last_tool.is_none() {
            if let Some(call) = m.tool_calls.first() {
                last_tool = Some(call.function.name.clone());
            }
        }
        if last_user.is_some() && last_assistant.is_some() && last_tool.is_some() {
            break;
        }
    }

    // An agent's turns live in the sidecar, not the harness transcript.
    // When the agent spoke last, its exchange is what the card shows.
    let mut message_count = record.messages.len();
    if let Some(path) = store.agent_log_path(&record.id) {
        let lines = mira_acp::agent_sessions::read_lines(&path);
        message_count += lines.iter().filter(|l| l.get("user").is_some()).count();
        let last_user_line = lines.iter().rposition(|l| l.get("user").is_some());
        let agent_newer = last_user_line.is_some_and(|i| {
            let t = lines[i].get("t").and_then(|t| t.as_u64()).unwrap_or(0);
            last_user.is_none() || t / 1000 >= record.updated_at
        });
        if let (true, Some(i)) = (agent_newer, last_user_line) {
            last_user = lines[i]
                .pointer("/user/text")
                .and_then(|t| t.as_str())
                .map(|t| t.trim().chars().take(400).collect());
            let mut reply = String::new();
            let mut tool = None;
            for line in &lines[i + 1..] {
                let Some(frame) = line.get("frame") else { continue };
                match frame.get("type").and_then(|t| t.as_str()) {
                    Some("acp_text") => {
                        if let Some(t) = frame.get("text").and_then(|t| t.as_str()) {
                            reply.push_str(t);
                        }
                    }
                    Some("acp_tool_call") => {
                        tool = frame
                            .pointer("/call/title")
                            .or_else(|| frame.pointer("/call/name"))
                            .and_then(|t| t.as_str())
                            .map(|t| t.chars().take(80).collect());
                    }
                    _ => {}
                }
            }
            let reply = reply.trim();
            last_assistant = (!reply.is_empty()).then(|| reply.chars().take(600).collect());
            if tool.is_some() {
                last_tool = tool;
            }
        }
    }

    Json(SessionPreviewView {
        id: record.id.to_string(),
        last_user,
        last_assistant,
        last_tool,
        message_count,
        updated_at: record.updated_at,
    })
    .into_response()
}

pub async fn load_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let sid = SessionId::from(id.as_str());
    let slot = match state.ensure_slot(&sid).await {
        Ok(s) => s,
        Err(e) => {
            let status = if e.contains("persistence disabled") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::NOT_FOUND
            };
            return err(status, e);
        }
    };
    // Persist last_cwd so the next server restart lands on the same folder.
    let cwd = slot.cwd.read().await.clone();
    if cwd.is_dir() {
        let mut s = RuntimeState::load().unwrap_or_default();
        s.last_cwd = Some(cwd);
        if let Err(e) = s.save() {
            warn!(%e, "state.yaml: save failed after session load");
        }
    }
    state.set_active(slot.id.clone()).await;
    let ready = build_ready_for_slot(&slot, &state).await;
    let _ = slot.events_tx.send(ready);
    info!(id = %slot.id, "session loaded / re-attached");
    Json(serde_json::json!({ "id": slot.id.to_string() })).into_response()
}

pub async fn new_session(State(state): State<AppState>) -> Response {
    let prev_slot = state.active_slot().await;
    let prev = prev_slot.session.read().await.clone();
    let prev_cfg = prev.config().await;
    let cwd = prev_slot.cwd.read().await.clone();
    let cfg = SessionConfig {
        engine_instance: prev_cfg.engine_instance.clone(),
        agent_approval_mode: mira_policy::Mode::Manual,
        model: prev_cfg.model.clone(),
        max_rounds: prev_cfg.max_rounds,
        temperature: prev_cfg.temperature,
        max_tokens: prev_cfg.max_tokens,
        reasoning_effort: prev_cfg.reasoning_effort.clone(),
        service_tier: None,
        response_format: prev_cfg.response_format.clone(),
        compactor_model: prev_cfg.compactor_model.clone(),
        small_model: prev_cfg.small_model.clone(),
        context_window: prev_cfg.context_window,
    };
    let deps = state.slot_deps();
    let slot = crate::slot::build_slot(cwd, cfg, None, &deps).await;
    // A new chat inherits the previous chat's *setup*: the external
    // agent it ran (driver, config, permission mode) comes along, so
    // the first prompt on this session starts the same agent the same
    // way. Only the config moves — no process is spawned here, and the
    // resume cursor stays per-slot, so this agent begins a blank
    // conversation. `AcpStop` is what declines the inheritance.
    *slot.acp_launch.lock().await = prev_slot.acp_launch.lock().await.clone();
    let slot_id = slot.id.clone();
    state.insert_slot(slot.clone()).await;
    state.set_active(slot_id.clone()).await;
    let ready = build_ready_for_slot(&slot, &state).await;
    let _ = slot.events_tx.send(ready);
    info!(id = %slot_id, "new session slot");
    Json(serde_json::json!({ "id": slot_id.to_string() })).into_response()
}

pub async fn delete_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let sid = SessionId::from(id.as_str());
    let live_slot = state.slot(&sid).await;
    let _dispatch = match &live_slot {
        Some(s) => Some(s.engine.dispatch_lock.lock().await),
        None => None,
    };
    let _start = match &live_slot {
        Some(s) => Some(s.engine.start_lock.lock().await),
        None => None,
    };
    if let Some(slot) = &live_slot {
        slot.engine
            .retired
            .store(true, std::sync::atomic::Ordering::SeqCst);
        crate::acp_session::stop_agent(slot).await;
        slot.runtime_requests.close().await;
        slot.message_queue.close().await;
        state.session_activity.set(slot.id.to_string(),false);
    }

    // Cascade: any persisted subagent transcripts belong to this parent.
    if let Some(store) = state.store.clone() {
        if let Ok(all) = store.list_all(1000).await {
            for child in all {
                if child
                    .parent_id
                    .as_ref()
                    .map(|p| p.to_string() == id)
                    .unwrap_or(false)
                {
                    if let Err(e) = store.delete(&child.id).await {
                        warn!(child = %child.id, %e, "cascade delete failed");
                    }
                }
            }
        }
        if let Err(e) = store.delete(&sid).await {
            if let Some(slot) = &live_slot {
                slot.runtime_requests.reopen();
                slot.message_queue.reopen();
                slot.engine
                    .retired
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
            return err(StatusCode::INTERNAL_SERVER_ERROR, format!("delete: {e}"));
        }
    }

    // Tear down the live slot if any. Aborts the in-flight turn so it
    // stops pumping tokens; the AttachGuard on any WS forwarder is
    // dropped when the socket next closes.
    // A deleted chat's agent loses its tool-server token too.
    crate::browser::revoke_mcp_grants(&sid.to_string());
    if let Some(slot) = state.remove_slot(&sid).await {
        if let Some(h) = slot.turn.lock().await.take() {
            h.abort();
        }
        let _ = slot.session.read().await.cancel().await;
        // The slot owned a third-party agent process; deleting the
        // session takes the process down with it instead of leaving an
        // orphaned CLI running with no transcript attached.
        crate::acp_session::stop_agent(&slot).await;
        // Release its remote environments; pending changes are saved as a
        // patch under ~/.mira/sandbox, never applied.
        let envs = slot.environments.clone();
        tokio::spawn(async move {
            if let Err(e) = envs.finish().await {
                warn!(%e, "releasing the session's environments failed");
            }
        });
    }

    // If we just deleted the active session, promote *some* remaining
    // slot to active. Prefer an existing live slot; if none, spin up a
    // fresh one in the deleted session's folder so the HTTP surface has
    // an active target.
    let mut promoted: Option<String> = None;
    let active_now = state.active.read().await.clone();
    if active_now.to_string() == id {
        let remaining = state.list_slots().await;
        if let Some(next) = remaining.into_iter().next() {
            promoted = Some(next.id.to_string());
            state.set_active(next.id.clone()).await;
        } else {
            let prev_cfg = state.current_session().await.config().await;
            let cwd = state.current_cwd().await;
            let cfg = SessionConfig {
                engine_instance: prev_cfg.engine_instance.clone(),
                agent_approval_mode: mira_policy::Mode::Manual,
                model: prev_cfg.model,
                max_rounds: prev_cfg.max_rounds,
                temperature: prev_cfg.temperature,
                max_tokens: prev_cfg.max_tokens,
                reasoning_effort: prev_cfg.reasoning_effort.clone(),
                service_tier: None,
                response_format: prev_cfg.response_format.clone(),
                compactor_model: prev_cfg.compactor_model.clone(),
                small_model: prev_cfg.small_model.clone(),
                context_window: prev_cfg.context_window,
            };
            let deps = state.slot_deps();
            let fresh = crate::slot::build_slot(cwd, cfg, None, &deps).await;
            let fid = fresh.id.clone();
            state.insert_slot(fresh.clone()).await;
            state.set_active(fid.clone()).await;
            promoted = Some(fid.to_string());
            let ready = build_ready_for_slot(&fresh, &state).await;
            let _ = fresh.events_tx.send(ready);
        }
    }

    info!(%id, promoted = ?promoted, "session deleted");
    Json(serde_json::json!({ "ok": true, "promoted": promoted })).into_response()
}

/// Pin / archive / restore a session. Each field is optional so the UI
/// can flip one flag without knowing the other; the record is the source
/// of truth, so this survives restarts and CLI use.
///
/// 404s for sessions with no persisted record yet — a brand-new slot
/// that has never been checkpointed can't be meaningfully pinned, and
/// "archive a session that doesn't exist on disk" is a stale-UI race.
#[derive(Debug, Deserialize)]
pub struct SetFlagsRequest {
    pub pinned: Option<bool>,
    /// `true` archives (stamped with the current time), `false` restores.
    pub archived: Option<bool>,
}

pub async fn set_session_flags(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SetFlagsRequest>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — cannot pin or archive".to_string(),
        );
    };
    let sid = SessionId::from(id.as_str());
    let mut record = match store.load(&sid).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };
    if let Some(pinned) = body.pinned {
        record.pinned = pinned;
    }
    if let Some(archived) = body.archived {
        record.archived_at = if archived {
            Some(mira_harness::persist::now_secs())
        } else {
            None
        };
    }
    if let Err(e) = store.save(&record).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}"));
    }
    // Mirror onto the live slot (if one exists) — checkpoints run after
    // every turn and would otherwise overwrite the record with the
    // session's stale in-memory flags.
    for slot in state.list_slots().await {
        if slot.id.to_string() == id {
            slot.session
                .read()
                .await
                .set_sidebar_flags(record.pinned, record.archived_at)
                .await;
        }
    }
    info!(%id, pinned = record.pinned, archived = record.archived_at.is_some(), "session flags updated");
    Json(serde_json::json!({
        "id": id,
        "pinned": record.pinned,
        "archived": record.archived_at.is_some(),
    }))
    .into_response()
}

/// Build a Ready frame for `slot` from its live session state — the same
/// frame the WebSocket sends, so every path reports the session's engine.
async fn build_ready_for_slot(slot: &crate::slot::SessionSlot, state: &AppState) -> ServerMsg {
    crate::ws::build_ready(slot, state).await
}

fn summarize_record(r: &SessionRecord, active_id: &str, live_meta: &std::collections::HashMap<String, LiveMeta>) -> SessionSummary {
    let mut summary = summarize_metadata(&SessionListRecord::from(r), active_id, live_meta);
    (summary.worktree_status, summary.worktree_branch) = cached_worktree_status(&r.cwd);
    summary
}

fn summarize_metadata(
    r: &SessionListRecord,
    active_id: &str,
    live_meta: &std::collections::HashMap<String, LiveMeta>,
) -> SessionSummary {
    let first = r.first_user_message.as_deref().map(|s| truncate(s, FIRST_MSG_TRUNC));
    let id = r.id.to_string();
    let live = live_meta.get(&id);
    SessionSummary {
        parent_id: r.parent_id.as_ref().map(|id| id.to_string()),        id: id.clone(),
        model: r.model.clone(),
        cwd: r.cwd.display().to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
        message_count: r.message_count,
        title: r.title.clone(),
        first_user_message: first,
        active: id == active_id,
        attached: live.map(|m| m.attached).unwrap_or(false),
        running: live.map(|m| m.running).unwrap_or(false),
        background_mode: live.map(|m| m.background_mode),
        worktree_status: None,
        worktree_branch: None,
        pinned: r.pinned,
        archived: r.archived_at.is_some(),
        // Badge the engine the session runs on now: an agent it has left
        // for a provider no longer counts.
        agent_driver: match live {
            Some(m) => m.agent_driver.clone(),
            None => r.agent_driver.clone(),
        },
        usage: SessionUsageView {
            prompt_tokens: r.usage.prompt_tokens,
            completion_tokens: r.usage.completion_tokens,
            cached_input_tokens: r.usage.cached_input_tokens,
            rounds: r.usage.rounds,
        },
        forked_from: r.forked_from.as_ref().map(|f| f.session_id.to_string()),
        forked_at: r.forked_from.as_ref().map(|f| f.at.clone()),
        needs_attention: live.is_some_and(|m| m.needs_attention),
        launched_by: crate::agent_threads::launcher_of(&id),
    }
}

#[derive(Debug, Deserialize)]
pub struct ForkBody {
    /// The user message to fork at, as edit & resend identifies it: its
    /// text, and which match counting from the latest.
    text: String,
    #[serde(default)]
    occurrence: usize,
}

/// `POST /api/sessions/:id/fork` — "Fork from here": a new chat with this
/// one's history through the given message's turn, nested under it in the
/// sidebar. The files on disk are left as they are. Answers `{ "id": … }`.
pub async fn fork_session(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<ForkBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "forking needs saved chats (persistence is off)".to_string(),
        );
    };
    let src = SessionId::from(id.as_str());
    // A live chat may be a round ahead of its last save.
    if let Some(slot) = state.slot(&src).await {
        slot.session.read().await.clone().save_now().await;
    }
    let record = match store.load(&src).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };
    let fork = match record.fork_at(SessionId::new(), &body.text, body.occurrence) {
        Ok(f) => f,
        Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
    };
    if let Err(e) = store.save(&fork).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}"));
    }
    Json(serde_json::json!({ "id": fork.id.to_string() })).into_response()
}

/// Shared folders are probed once, with bounded Git concurrency off the async
/// executor. The short cache is independent of frequently updated chat metadata.
async fn enrich_worktrees(summaries: &mut [SessionSummary]) {
    use std::sync::{Arc, OnceLock};
    static LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let limit = LIMIT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)));
    let folders: std::collections::HashSet<_> = summaries.iter().map(|summary| std::path::PathBuf::from(&summary.cwd)).collect();
    let mut jobs = tokio::task::JoinSet::new();
    for folder in folders {
        let limit = limit.clone();
        jobs.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            tokio::task::spawn_blocking(move || {
                let metadata = cached_worktree_status(&folder); (folder, metadata)
            }).await.ok()
        });
    }
    let mut metadata = std::collections::HashMap::new();
    while let Some(result) = jobs.join_next().await {
        if let Ok(Some((folder, value))) = result {metadata.insert(folder, value);}
    }
    for summary in summaries {
        if let Some((status, branch)) = metadata.get(std::path::Path::new(&summary.cwd)) {
            summary.worktree_status = *status; summary.worktree_branch = branch.clone();
        }
    }
}
fn cached_worktree_status(cwd: &std::path::Path) -> (Option<WorktreeMergeStatus>, Option<String>) {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Cache = std::collections::HashMap<std::path::PathBuf, (Instant, (Option<WorktreeMergeStatus>, Option<String>))>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::new()));
    if let Some((at, value)) = cache.lock().unwrap_or_else(|error| error.into_inner()).get(cwd) {
        if at.elapsed() < Duration::from_secs(15) {return value.clone();}
    }
    let value = detect_worktree_status(cwd);
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(15));
    if cache.len() < 1024 {cache.insert(cwd.to_path_buf(), (Instant::now(), value.clone()));}
    value
}

fn detect_worktree_status(cwd: &std::path::Path) -> (Option<WorktreeMergeStatus>, Option<String>) {
    if !cwd
        .components()
        .zip(cwd.components().skip(1))
        .zip(cwd.components().skip(2))
        .any(|((a, b), c)| {
            use std::path::Component::Normal;
            let (Normal(a), Normal(b), Normal(c)) = (a, b, c) else {
                return false;
            };
            a == std::ffi::OsStr::new(".mira")
                && b == std::ffi::OsStr::new("worktrees")
                && !c.is_empty()
        })
    {
        return (None, None);
    }
    if !cwd.is_dir() {
        return (None, None);
    }

    let branch = match git_output(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Some(b) if !b.is_empty() => b,
        _ => return (None, None),
    };

    let common_dir = match git_output(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        Some(s) => std::path::PathBuf::from(s),
        None => return (None, Some(branch)),
    };
    let primary = match common_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => return (None, Some(branch)),
    };

    let base = ["main", "master"]
        .iter()
        .copied()
        .find(|b| branch_exists(&primary, b));
    let Some(base) = base else {
        return (None, Some(branch));
    };
    if branch == base {
        return (None, Some(branch));
    }

    let status = if is_ancestor(&primary, &branch, base) {
        WorktreeMergeStatus::Merged
    } else {
        WorktreeMergeStatus::Unmerged
    };
    (Some(status), Some(branch))
}

fn git_output(cwd: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn branch_exists(cwd: &std::path::Path, name: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_ancestor(cwd: &std::path::Path, branch: &str, base: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(["merge-base", "--is-ancestor", branch, base])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim().replace(['\n', '\r'], " ");
    if trimmed.chars().count() <= max {
        return trimmed;
    }
    let head: String = trimmed.chars().take(max).collect();
    format!("{head}…")
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

// ---------- rename endpoints ----------

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct RenameResponse {
    pub id: String,
    pub title: String,
}

pub async fn set_session_title(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<RenameRequest>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — nothing to rename".to_string(),
        );
    };
    let title = req.title.trim().to_string();
    let sid = SessionId::from(id.as_str());
    let mut record = match store.load(&sid).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };
    record.title = if title.is_empty() {
        None
    } else {
        Some(title.clone())
    };
    if let Err(e) = store.save(&record).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}"));
    }

    if let Some(slot) = state.slot(&sid).await {
        if !title.is_empty() {
            slot.session.read().await.set_title(&title).await;
        }
    }
    // Fan out so any watching sidebar refreshes regardless of which
    // session it's currently focused on.
    state
        .broadcast_all(ServerMsg::SessionTitleUpdated {
            session_id: id.clone(),
            title: title.clone(),
        })
        .await;

    Json(RenameResponse { id, title }).into_response()
}

pub async fn regenerate_session_title(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return err(
            StatusCode::BAD_REQUEST,
            "persistence disabled — nothing to rename".to_string(),
        );
    };
    let sid = SessionId::from(id.as_str());
    let mut record = match store.load(&sid).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::NOT_FOUND, format!("load: {e}")),
    };

    let user_msg = record.first_user_message().map(str::to_owned);
    let users: Vec<String> = user_msg.into_iter().collect();
    if users.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "session doesn't have a user message yet".to_string(),
        );
    }

    let provider = state.harness_provider.clone();
    let model = record.cfg.background_model(None);
    info!(session = %id, %model, "regenerate title: calling extractor");
    let mut generated = crate::title::generate(&*provider, &model, &users).await;
    if generated.is_err() && model != record.cfg.model {
        generated = crate::title::generate(&*provider, &record.cfg.model, &users).await;
    }
    let title = match generated {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => {
            let fallback = crate::title::heuristic_from_user_message(&users[0]);
            if fallback.is_empty() {
                warn!(session = %id, "regenerate title: extractor empty, no heuristic fallback");
                return err(
                    StatusCode::BAD_GATEWAY,
                    "extractor returned an empty title".to_string(),
                );
            }
            warn!(session = %id, %fallback, "regenerate title: extractor empty, using heuristic");
            fallback
        }
        Err(e) => {
            warn!(session = %id, %e, "regenerate title: generate failed");
            return err(StatusCode::BAD_GATEWAY, format!("generate: {e}"));
        }
    };
    info!(session = %id, %title, "regenerate title: generated");

    record.title = Some(title.clone());
    if let Err(e) = store.save(&record).await {
        warn!(session = %id, %e, "regenerate title: save failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}"));
    }

    if let Some(slot) = state.slot(&sid).await {
        slot.session.read().await.set_title(&title).await;
    }
    state
        .broadcast_all(ServerMsg::SessionTitleUpdated {
            session_id: id.clone(),
            title: title.clone(),
        })
        .await;

    info!(session = %id, %title, "regenerate title: applied");
    Json(RenameResponse { id, title }).into_response()
}

// ---------- background-mode endpoint ----------

#[derive(Debug, Deserialize)]
pub struct BackgroundModeUpdate {
    pub mode: BackgroundMode,
}

/// `PUT /api/sessions/:id/background` — set the slot's background mode.
/// Errors if the slot isn't loaded (background mode only makes sense for
/// live slots; a persisted-but-not-loaded session has no channels to gate).
pub async fn set_background_mode_http(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<BackgroundModeUpdate>,
) -> Response {
    let sid = SessionId::from(id.as_str());
    let Some(slot) = state.slot(&sid).await else {
        return err(
            StatusCode::NOT_FOUND,
            format!("session `{id}` is not loaded; open it before changing background mode"),
        );
    };
    {
        let mut guard = slot.background_mode.write().await;
        *guard = body.mode;
    }
    state
        .broadcast_all(ServerMsg::BackgroundModeChanged {
            session_id: id.clone(),
            mode: body.mode,
        })
        .await;
    Json(serde_json::json!({ "id": id, "mode": body.mode })).into_response()
}

#[derive(Deserialize)]
pub struct RelationshipQuery { #[serde(default)] pub limit: Option<usize> }
/// Direct parents are loaded by ID even when they have fallen out of the recent-chat list.
pub async fn relationships(State(state): State<AppState>, AxumPath(id): AxumPath<String>, Query(query): Query<RelationshipQuery>) -> Response {
    let Some(store) = state.store.clone() else { return Json(serde_json::json!({"current": null, "parent": null, "children": [], "has_more": false})).into_response(); };
    let sid = SessionId::from(id.as_str());
    let record = match store.load(&sid).await {
        Ok(record) => record,
        Err(mira_harness::StoreError::NotFound(_)) => return Json(serde_json::json!({"current": null, "parent": null, "children": [], "has_more": false})).into_response(),
        Err(error) => return err(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let parent_id = record.parent_id.clone().or_else(|| record.forked_from.as_ref().map(|fork| fork.session_id.clone()));
    let parent = match parent_id { Some(id) => store.load(&id).await.ok(), None => None };
    let limit = query.limit.unwrap_or(20).clamp(20, 200);
    let mut children = match store.list_related(&sid, limit + 1).await { Ok(records) => records, Err(error) => return err(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()) };
    let has_more = children.len() > limit; children.truncate(limit);
    let active = state.active.read().await.to_string(); let live = live_slot_metadata(&state).await;
    Json(serde_json::json!({ "current": summarize_record(&record, &active, &live), "parent": parent.as_ref().map(|record| summarize_record(record, &active, &live)), "children": children.iter().map(|record| summarize_record(record, &active, &live)).collect::<Vec<_>>(), "has_more": has_more })).into_response()
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn sidebar_metadata_preserves_counts_titles_engine_and_unicode_preview() {
        let record = SessionListRecord {
            id: SessionId::from("saved-chat"), cwd: "/project".into(), model: "model".into(),
            created_at: 1, updated_at: 2, title: Some("Saved chat".into()),
            first_user_message: Some("你好\nworld".into()), message_count: 123,
            usage: Default::default(), parent_id: None, pinned: true, archived_at: None,
            agent_driver: Some("codex".into()), forked_from: None,
        };
        let summary = summarize_metadata(&record, "saved-chat", &Default::default());
        assert_eq!(summary.message_count, 123); assert_eq!(summary.title, record.title);
        assert_eq!(summary.first_user_message.as_deref(), Some("你好 world"));
        assert_eq!(summary.agent_driver.as_deref(), Some("codex"));
        assert!(summary.active && summary.pinned); assert!(summary.worktree_status.is_none());
    }
    #[tokio::test]
    async fn shared_non_worktree_folder_enrichment_preserves_all_rows() {
        let record = SessionListRecord {
            id: SessionId::from("saved-chat"), cwd: "/project".into(), model: "model".into(),
            created_at: 1, updated_at: 2, title: None, first_user_message: None, message_count: 0,
            usage: Default::default(), parent_id: None, pinned: false, archived_at: None,
            agent_driver: None, forked_from: None,
        };
        let mut rows = (0..100).map(|_| summarize_metadata(&record, "", &Default::default())).collect::<Vec<_>>();
        enrich_worktrees(&mut rows).await;
        assert_eq!(rows.len(), 100); assert!(rows.iter().all(|row| row.worktree_status.is_none() && row.worktree_branch.is_none()));
    }
}
