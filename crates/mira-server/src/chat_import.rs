//! Bringing chats from other coding agents into Mira (`/api/import`).
//!
//! A scan lists Claude Code and Codex history grouped by project. Importing
//! a chat makes it a Mira chat on that agent: the conversation is written
//! as the agent transcript Mira replays (so it reads like any agent chat),
//! the session is marked as running on that agent, and the agent's own
//! session id is recorded as the resume point — the next message continues
//! the real session, with everything the agent remembers, instead of
//! starting over.
//!
//! Chats Mira itself ran (they land in the same history folders) are left
//! out, and each chat is imported once: a ledger in `~/.mira` maps it to
//! the Mira chat it became.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_acp::history_import::{self as hist, HistorySession, Role, Source};
use mira_core::{Message, SessionId};
use mira_harness::persist::{AgentSessionMeta, SessionRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::protocol::ServerMsg;
use crate::state::AppState;

const SOURCES: [Source; 2] = [Source::ClaudeCode, Source::Codex];
/// A scan is reused this long, so the picker and the import that follows
/// read the same list without scanning twice.
const SCAN_TTL: Duration = Duration::from_secs(120);

fn mira_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".mira"))
}

/* ---------------------------------------------------------------- */
/* Ledger: which outside chat became which Mira chat                 */
/* ---------------------------------------------------------------- */

fn ledger_key(source: Source, id: &str) -> String {
    format!("{}:{id}", source.driver_kind())
}

fn read_ledger(home: &Path) -> HashMap<String, String> {
    std::fs::read(home.join("imported-chats.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_ledger(home: &Path, ledger: &HashMap<String, String>) {
    let _ = std::fs::create_dir_all(home);
    let tmp = home.join("imported-chats.json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(ledger).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, home.join("imported-chats.json"));
    }
}

/* ---------------------------------------------------------------- */
/* Scan                                                              */
/* ---------------------------------------------------------------- */

static SCAN: Mutex<Option<(Instant, Vec<HistorySession>)>> = Mutex::new(None);

/// Every outside chat, newest first, minus the ones Mira ran itself.
fn scan_all(fresh: bool) -> Vec<HistorySession> {
    if !fresh {
        if let Some((at, list)) = SCAN.lock().ok().and_then(|g| g.clone()) {
            if at.elapsed() < SCAN_TTL {
                return list;
            }
        }
    }
    let own: HashSet<String> = mira_home()
        .and_then(|m| m.parent().map(Path::to_path_buf))
        .map(|home| mira_acp::agent_sessions::agent_session_ids_in(&home))
        .unwrap_or_default();
    let mut all: Vec<HistorySession> = SOURCES
        .iter()
        .filter_map(|&s| hist::source_home(s).map(|h| (s, h)))
        .flat_map(|(s, h)| hist::scan(s, &h))
        .filter(|c| !own.contains(&c.id) && !c.cwd.is_empty())
        // Mira's own agent worktrees are its sandboxes, not projects.
        .filter(|c| !c.cwd.contains("/.mira/"))
        .collect();
    all.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    if let Ok(mut g) = SCAN.lock() {
        *g = Some((Instant::now(), all.clone()));
    }
    all
}

#[derive(Serialize)]
struct SourceView {
    id: Source,
    label: &'static str,
    /// The agent's history folder exists on this computer.
    found: bool,
    chats: usize,
}

#[derive(Serialize)]
struct ChatView {
    source: Source,
    id: String,
    title: String,
    model: Option<String>,
    messages: usize,
    /// Unix milliseconds.
    updated_at: Option<i64>,
    /// Already brought into Mira (the Mira chat's id).
    imported_as: Option<String>,
    /// Run by a program, not a person (see `HistorySession::automated`).
    automated: bool,
}

#[derive(Serialize)]
struct ProjectView {
    path: String,
    /// The folder's name.
    name: String,
    /// `owner/name` from the git remote, when there is one.
    repo: Option<String>,
    is_git: bool,
    /// The folder still exists.
    exists: bool,
    last_active: Option<i64>,
    chats: Vec<ChatView>,
}

#[derive(Serialize)]
struct ScanView {
    sources: Vec<SourceView>,
    projects: Vec<ProjectView>,
}

/// `owner/name` from a git remote URL (GitHub, GitLab, any `host:owner/name`).
fn repo_from_remote(url: &str) -> Option<String> {
    let s = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = s.rsplit_once(':').map(|(_, p)| p).unwrap_or(s);
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    (parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty())
        .then(|| format!("{}/{}", parts[1], parts[0]))
}

fn git_info(path: &Path) -> (bool, Option<String>) {
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let is_git = run(&["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true");
    let repo = is_git
        .then(|| run(&["remote", "get-url", "origin"]))
        .flatten()
        .and_then(|u| repo_from_remote(&u));
    (is_git, repo)
}

fn scan_view(fresh: bool) -> ScanView {
    let all = scan_all(fresh);
    let ledger = mira_home().map(|h| read_ledger(&h)).unwrap_or_default();
    let sources = SOURCES
        .iter()
        .map(|&s| SourceView {
            id: s,
            label: s.label(),
            found: hist::source_home(s).is_some_and(|h| h.is_dir()),
            chats: all.iter().filter(|c| c.source == s).count(),
        })
        .collect();
    let mut by_path: BTreeMap<String, Vec<&HistorySession>> = BTreeMap::new();
    for c in &all {
        by_path.entry(c.cwd.clone()).or_default().push(c);
    }
    let mut projects: Vec<ProjectView> = by_path
        .into_iter()
        .map(|(path, chats)| {
            let p = Path::new(&path);
            let exists = p.is_dir();
            let (is_git, repo) = if exists { git_info(p) } else { (false, None) };
            ProjectView {
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone()),
                repo,
                is_git,
                exists,
                last_active: chats.iter().filter_map(|c| c.updated_at).max(),
                chats: chats
                    .into_iter()
                    .map(|c| ChatView {
                        source: c.source,
                        id: c.id.clone(),
                        title: c.title.clone(),
                        model: c.model.clone(),
                        messages: c.message_count,
                        updated_at: c.updated_at,
                        imported_as: ledger.get(&ledger_key(c.source, &c.id)).cloned(),
                        automated: c.automated,
                    })
                    .collect(),
                path,
            }
        })
        .collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.last_active));
    ScanView { sources, projects }
}

#[derive(Deserialize)]
pub struct ScanQuery {
    #[serde(default)]
    pub fresh: bool,
}

/// GET /api/import/scan[?fresh=true]
pub async fn scan(axum::extract::Query(q): axum::extract::Query<ScanQuery>) -> Response {
    match tokio::task::spawn_blocking(move || scan_view(q.fresh)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/* ---------------------------------------------------------------- */
/* Import                                                            */
/* ---------------------------------------------------------------- */

#[derive(Deserialize)]
pub struct ChatRef {
    pub source: String,
    pub id: String,
}

#[derive(Deserialize)]
pub struct ImportRequest {
    pub chats: Vec<ChatRef>,
}

#[derive(Serialize, Default)]
struct ImportResult {
    imported: Vec<String>,
    /// Already in Mira.
    skipped: usize,
    failed: Vec<String>,
}

/// The agent transcript lines that replay a chat: a note saying where it
/// came from, then the conversation as user prompts and agent answers.
fn transcript_lines(chat: &HistorySession) -> Vec<serde_json::Value> {
    let kind = chat.source.driver_kind();
    let frame = |t: i64, msg: ServerMsg| json!({ "t": t, "driver": kind, "frame": serde_json::to_value(msg).unwrap_or_default() });
    let mut t = chat.started_at.unwrap_or(0);
    let shown = chat.messages.len();
    let note = if chat.message_count > shown {
        format!(
            "Imported from {} — the latest {shown} of {} messages. The agent still has the whole conversation.",
            chat.source.label(),
            chat.message_count
        )
    } else {
        format!("Imported from {}", chat.source.label())
    };
    let mut out = vec![frame(t, ServerMsg::Warning { text: note })];
    let mut open_turn = false;
    for m in &chat.messages {
        t = m.at.unwrap_or(t);
        match m.role {
            Role::User => {
                if open_turn {
                    out.push(frame(t, ServerMsg::acp_turn_end("end_turn")));
                }
                out.push(
                    json!({ "t": t, "driver": kind, "user": { "text": m.text, "images": 0 } }),
                );
                open_turn = true;
            }
            Role::Assistant => out.push(frame(
                t,
                ServerMsg::AcpText {
                    text: m.text.clone(),
                },
            )),
        }
    }
    if open_turn {
        out.push(frame(t, ServerMsg::acp_turn_end("end_turn")));
    }
    out
}

async fn import_one(
    state: &AppState,
    chat: &HistorySession,
    model: &str,
) -> Result<String, String> {
    let store = state
        .store
        .as_ref()
        .ok_or("chats aren't saved on this server")?;
    let id = SessionId::new();
    let cwd = PathBuf::from(&chat.cwd);
    let secs = |ms: Option<i64>| ms.map(|m| (m / 1000).max(0) as u64);
    let now = mira_harness::persist::now_ms() / 1000;
    let record = SessionRecord {
        id: id.clone(),
        cwd: cwd.clone(),
        cfg: mira_harness::SessionConfig::new(model),
        // The system prompt Mira would start the chat with, so switching it
        // to a provider later works like any chat.
        messages: vec![Message::system(crate::system_prompt(
            &cwd,
            &state.base_registry,
        ))],
        archived: vec![],
        created_at: secs(chat.started_at).unwrap_or(now),
        updated_at: secs(chat.updated_at).unwrap_or(now),
        title: Some(chat.title.clone()),
        turns: vec![],
        usage: Default::default(),
        parent_id: None,
        tasks: vec![],
        goal: None,
        previews: Default::default(),
        pinned: false,
        archived_at: None,
        agent: Some(AgentSessionMeta {
            driver_kind: chat.source.driver_kind().to_string(),
            model: chat.model.clone(),
            active: true,
            launch: None,
        }),
    };
    let log = store.agent_log_path(&id).ok_or("no transcript storage")?;
    for line in transcript_lines(chat) {
        mira_acp::agent_sessions::append_line_to(&log, &line);
    }
    // The resume point: the next message continues the real session.
    if let Some(home) = mira_home().and_then(|m| m.parent().map(Path::to_path_buf)) {
        mira_acp::agent_sessions::record_in(
            &home,
            id.as_str(),
            chat.source.driver_kind(),
            &chat.id,
        );
    }
    store.save(&record).await.map_err(|e| e.to_string())?;
    Ok(id.to_string())
}

/// POST /api/import { chats: [{source, id}] }
pub async fn import(State(state): State<AppState>, Json(req): Json<ImportRequest>) -> Response {
    let Some(home) = mira_home() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "no home directory" })),
        )
            .into_response();
    };
    let all = tokio::task::spawn_blocking(|| scan_all(false))
        .await
        .unwrap_or_default();
    let by_key: HashMap<String, &HistorySession> = all
        .iter()
        .map(|c| (ledger_key(c.source, &c.id), c))
        .collect();
    let mut ledger = read_ledger(&home);
    // The provider model the chats get if they're ever switched off the
    // agent: the selected one, else the current chat's.
    let model = match state.selection.snapshot().1 {
        Some(m) => m,
        None => state.current_session().await.config().await.model,
    };
    let mut result = ImportResult::default();
    for r in req.chats.iter().take(500) {
        let Some(source) = Source::parse(&r.source) else {
            result.failed.push(r.id.clone());
            continue;
        };
        let key = ledger_key(source, &r.id);
        if ledger.contains_key(&key) {
            result.skipped += 1;
            continue;
        }
        let Some(found) = by_key.get(&key) else {
            result.failed.push(r.id.clone());
            continue;
        };
        let path = found.path.clone();
        let Some(chat) = tokio::task::spawn_blocking(move || hist::read_session(source, &path))
            .await
            .ok()
            .flatten()
        else {
            result.failed.push(r.id.clone());
            continue;
        };
        match import_one(&state, &chat, &model).await {
            Ok(mira_id) => {
                ledger.insert(key, mira_id.clone());
                result.imported.push(mira_id);
            }
            Err(e) => {
                tracing::warn!(%e, chat = %r.id, "import: chat failed");
                result.failed.push(r.id.clone());
            }
        }
    }
    write_ledger(&home, &ledger);
    Json(result).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_acp::history_import::HistoryMessage;

    #[test]
    fn remotes_name_their_repo() {
        assert_eq!(
            repo_from_remote("https://github.com/runmira/mira.git").as_deref(),
            Some("runmira/mira")
        );
        assert_eq!(
            repo_from_remote("git@github.com:runmira/mira.git").as_deref(),
            Some("runmira/mira")
        );
        assert_eq!(
            repo_from_remote("ssh://git@gitlab.com/team/app").as_deref(),
            Some("team/app")
        );
        assert_eq!(repo_from_remote("nonsense"), None);
    }

    #[test]
    fn a_chat_replays_as_agent_turns() {
        let m = |role, text: &str, at| HistoryMessage {
            role,
            text: text.into(),
            at: Some(at),
        };
        let chat = HistorySession {
            source: Source::ClaudeCode,
            id: "abc".into(),
            cwd: "/w".into(),
            title: "t".into(),
            model: None,
            started_at: Some(1000),
            updated_at: Some(4000),
            message_count: 3,
            automated: false,
            messages: vec![
                m(Role::User, "first", 1000),
                m(Role::Assistant, "answer", 2000),
                m(Role::User, "second", 3000),
            ],
            path: PathBuf::new(),
        };
        let lines = transcript_lines(&chat);
        let kinds: Vec<String> = lines
            .iter()
            .map(|l| {
                if l.get("user").is_some() {
                    "user".to_string()
                } else {
                    l["frame"]["type"].as_str().unwrap_or("?").to_string()
                }
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "warning",
                "user",
                "acp_text",
                "acp_turn_end",
                "user",
                "acp_turn_end"
            ]
        );
        assert!(lines.iter().all(|l| l["driver"] == "claude-code"));
        assert_eq!(lines[1]["user"]["text"], "first");
    }
}
