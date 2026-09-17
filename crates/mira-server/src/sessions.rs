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
//!                                            with `running` and `attached`
//!                                            hints for the sidebar.
//! - `GET    /api/sessions/:id/history`    — persisted history for
//!                                            rehydrating a subagent panel
//!                                            on reload.
//! - `POST   /api/sessions/:id/load`       — ensure a slot exists for `id`
//!                                            and mark it active.
//! - `POST   /api/sessions/new`            — create a fresh slot.
//! - `DELETE /api/sessions/:id`            — abort turn, drop slot,
//!                                            delete record.
//! - `PATCH  /api/sessions/:id/title`      — manual rename.
//! - `POST   /api/sessions/:id/title/regenerate` — AI rename.
//! - `PUT    /api/sessions/:id/background` — swap background mode.

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::RuntimeState;
use mira_core::{Role, SessionId};
use mira_harness::{SessionConfig, SessionRecord};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::protocol::ServerMsg;
use crate::slot::BackgroundMode;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub model: String,
    pub cwd: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub first_user_message: Option<String>,
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
    #[serde(skip_serializing_if = "SessionUsageView::is_empty")]
    pub usage: SessionUsageView,
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
    pub all: bool,
}

pub async fn list_sessions(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let Some(store) = state.store.clone() else {
        // No persistence — return only live slots. Cheap loop; a normal
        // server has O(1) slots at any time.
        let live: Vec<SessionSummary> = summarize_live(&state).await;
        return Json(live).into_response();
    };
    let records = if q.all {
        match store.list_all(200).await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")),
        }
    } else {
        let cwd = state.current_cwd().await;
        match store.list_recent(&cwd, 50).await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")),
        }
    };
    let active_id = state.active.read().await.to_string();

    // Cache live-slot metadata so we can annotate persisted rows without a
    // fresh read of state.slots per iteration.
    let live_meta = live_slot_metadata(&state).await;

    // Persisted records → summaries.
    let mut summaries: Vec<SessionSummary> = records
        .into_iter()
        .filter(|r| r.parent_id.is_none())
        .map(|r| summarize_record(&r, &active_id, &live_meta))
        .collect();

    // Any live slot the user just started that hasn't yet been checkpointed
    // to disk (or is running under `--no-persist`) still needs to show up
    // in the sidebar. Add slots whose id isn't in the persisted list.
    let known_ids: std::collections::HashSet<String> =
        summaries.iter().map(|s| s.id.clone()).collect();
    for slot in state.list_slots().await {
        let id = slot.id.to_string();
        if known_ids.contains(&id) {
            continue;
        }
        summaries.push(SessionSummary {
            id: id.clone(),
            model: slot.session.read().await.config().await.model,
            cwd: slot.cwd.read().await.display().to_string(),
            created_at: 0,
            updated_at: 0,
            message_count: 0,
            title: None,
            first_user_message: None,
            active: id == active_id,
            attached: slot.is_attached(),
            running: slot.is_running().await,
            background_mode: Some(*slot.background_mode.read().await),
            worktree_status: None,
            worktree_branch: None,
            usage: SessionUsageView::default(),
        });
    }
    Json(summaries).into_response()
}

/// Metadata about currently-live slots, snapshotted in one pass so we don't
/// re-lock `state.slots` per record.
struct LiveMeta {
    attached: bool,
    running: bool,
    background_mode: BackgroundMode,
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
        out.push(SessionSummary {
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
            usage: SessionUsageView::default(),
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
    let view = SessionHistoryView {
        id: record.id.to_string(),
        model: record.cfg.model.clone(),
        cwd: record.cwd.display().to_string(),
        title: record.title.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at,
        messages: record.messages,
        previews: record.previews,
    };
    Json(view).into_response()
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
    let prev = state.current_session().await;
    let prev_cfg = prev.config().await;
    let cwd = state.current_cwd().await;
    let cfg = SessionConfig {
        model: prev_cfg.model.clone(),
        max_rounds: prev_cfg.max_rounds,
        temperature: prev_cfg.temperature,
        max_tokens: prev_cfg.max_tokens,
        reasoning_effort: prev_cfg.reasoning_effort.clone(),
        response_format: prev_cfg.response_format.clone(),
        compactor_model: prev_cfg.compactor_model.clone(),
    };
    let deps = state.slot_deps();
    let slot = crate::slot::build_slot(cwd, cfg, None, &deps).await;
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
            return err(StatusCode::INTERNAL_SERVER_ERROR, format!("delete: {e}"));
        }
    }

    // Tear down the live slot if any. Aborts the in-flight turn so it
    // stops pumping tokens; the AttachGuard on any WS forwarder is
    // dropped when the socket next closes.
    if let Some(slot) = state.remove_slot(&sid).await {
        if let Some(h) = slot.turn.lock().await.take() {
            h.abort();
        }
        let _ = slot.session.read().await.cancel().await;
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
                model: prev_cfg.model,
                max_rounds: prev_cfg.max_rounds,
                temperature: prev_cfg.temperature,
                max_tokens: prev_cfg.max_tokens,
                reasoning_effort: prev_cfg.reasoning_effort.clone(),
                response_format: prev_cfg.response_format.clone(),
                compactor_model: prev_cfg.compactor_model.clone(),
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

/// Build a Ready frame for `slot` from its live session state.
async fn build_ready_for_slot(
    slot: &crate::slot::SessionSlot,
    state: &AppState,
) -> ServerMsg {
    let sess = slot.session.read().await.clone();
    let cfg = sess.config().await;
    let mode = state.policy.lock().await.mode();
    let history = sess.history().await;
    let turns = sess.turns().await;
    let usage = sess.usage().await;
    let tasks = sess.tasks().await;
    let goal = sess.goal().await;
    let previews = sess.previews().await;
    ServerMsg::Ready {
        session_id: sess.id.to_string(),
        model: cfg.model,
        mode,
        cwd: slot.cwd.read().await.display().to_string(),
        history,
        turns,
        usage,
        tasks,
        goal,
        previews,
    }
}

fn summarize_record(
    r: &SessionRecord,
    active_id: &str,
    live_meta: &std::collections::HashMap<String, LiveMeta>,
) -> SessionSummary {
    let first = r
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.clone())
        .map(|s| truncate(&s, FIRST_MSG_TRUNC));
    let id = r.id.to_string();
    let (worktree_status, worktree_branch) = detect_worktree_status(&r.cwd);
    let live = live_meta.get(&id);
    SessionSummary {
        id: id.clone(),
        model: r.cfg.model.clone(),
        cwd: r.cwd.display().to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
        message_count: r.messages.iter().filter(|m| m.role != Role::System).count(),
        title: r.title.clone(),
        first_user_message: first,
        active: id == active_id,
        attached: live.map(|m| m.attached).unwrap_or(false),
        running: live.map(|m| m.running).unwrap_or(false),
        background_mode: live.map(|m| m.background_mode),
        worktree_status,
        worktree_branch,
        usage: SessionUsageView {
            prompt_tokens: r.usage.prompt_tokens,
            completion_tokens: r.usage.completion_tokens,
            cached_input_tokens: r.usage.cached_input_tokens,
            rounds: r.usage.rounds,
        },
    }
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

    let user_msg = record
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.clone());
    let assistant_msg = record
        .messages
        .iter()
        .find(|m| {
            m.role == Role::Assistant
                && m.content
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
        })
        .and_then(|m| m.content.clone());
    let (Some(user), Some(assistant)) = (user_msg, assistant_msg) else {
        return err(
            StatusCode::BAD_REQUEST,
            "session doesn't have enough context yet (needs a user message + assistant reply)"
                .to_string(),
        );
    };

    let provider = state.harness_provider.clone();
    let model = record.cfg.model.clone();
    info!(session = %id, %model, "regenerate title: calling extractor");
    let title = match crate::title::generate(&*provider, &model, &user, &assistant).await {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => {
            let fallback = crate::title::heuristic_from_user_message(&user);
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
