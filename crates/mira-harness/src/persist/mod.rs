//! Session persistence.
//!
//! [`SessionStore`] is the trait; [`file_store::FileStore`] is the default
//! implementation that keeps sessions as pretty-printed JSON files under
//! `~/.mira/sessions/`. Sessions save automatically after each round when
//! a store is attached via [`crate::Session::with_store`].
//!
//! The trait exists so tests and future backends (Postgres, S3, etc.) can
//! slot in without touching the harness.

pub mod file_store;

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use mira_ai::TokenUsage;
use mira_core::{Message, SessionId, ToolCallId};
use mira_tools::DiffPreview;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::session::SessionConfig;

pub use file_store::FileStore;

/// What the user did with the sidebar's "Settled" shelf, in epoch seconds.
/// A chat also settles on its own (its PR merged or closed), so both
/// directions are recorded: `settled_at` holds it there until the chat
/// moves again, `unsettled_at` keeps it out until something newer happens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettleMarks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsettled_at: Option<u64>,
}

impl SettleMarks {
    pub fn is_empty(&self) -> bool {
        self.settled_at.is_none() && self.unsettled_at.is_none()
    }

    /// The user's choice, stamped now: settling clears an earlier
    /// un-settle and the other way round.
    pub fn set(settled: bool) -> Self {
        let now = Some(now_secs());
        if settled {
            Self {
                settled_at: now,
                unsettled_at: None,
            }
        } else {
            Self {
                settled_at: None,
                unsettled_at: now,
            }
        }
    }
}

/// A serialised session on disk.
///
/// `created_at` and `updated_at` are seconds since the Unix epoch. Kept as
/// plain integers to avoid dragging in a datetime crate for a value only
/// used for "most recent" sorting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub cwd: PathBuf,
    pub cfg: SessionConfig,
    pub messages: Vec<Message>,
    /// Messages compaction replaced, kept so the transcript can still
    /// show them (the model only sees `messages`). Empty for sessions
    /// never compacted or written before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived: Vec<Message>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Human-readable nickname generated after the first assistant reply.
    /// `None` means "not yet generated"; the UI falls back to the first
    /// user message. Skipped in the wire format when missing so we stay
    /// backwards-compatible with sessions written before this landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Per-turn wall-clock timing. Same ordering as user messages appear in
    /// `messages`. Used by the UI to render "Worked for Xs" chips even
    /// after a reload. Defaults to empty for records written before this
    /// field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnMeta>,
    /// Aggregate token accounting across every round in the session. Zeroed
    /// for sessions written before this field existed (or when the provider
    /// doesn't report usage).
    #[serde(default, skip_serializing_if = "UsageTotals::is_zero")]
    pub usage: UsageTotals,
    /// Read, write and edit calls that completed successfully, so after a
    /// resume they still supersede earlier reads of the same file (see
    /// `history::stub_superseded_reads`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_calls_done: Vec<ToolCallId>,
    /// Set when this session was spawned as a subagent by another session.
    /// Points at the parent's id so the sidebar can hide it from the
    /// primary chat list (subagents aren't user-facing conversations)
    /// and `delete` can cascade from the parent. Absent (`None`) for
    /// top-level chats; skipped from the wire format for backwards
    /// compatibility with sessions written before this landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<SessionId>,
    /// Session-scoped task list — every `TaskItem` including
    /// soft-deleted ones (so id sequence resumes exactly). Empty for
    /// sessions written before the field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<mira_tools::TaskItem>,
    /// Standing `/goal`, if any. Kept in the same record so resume
    /// picks up an in-flight autonomous run instead of forgetting it.
    /// Terminal statuses (Met / Impossible / …) are preserved too —
    /// the UI shows a "last goal" chip until the user clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<crate::goal::Goal>,
    /// Diff previews for `edit_file` / `write_file` calls, keyed by
    /// the tool call id. Captured at dispatch time (just before the
    /// tool runs) so the "before" file state matches what the user
    /// saw live. On reload the frontend attaches these to the
    /// matching tool entry so the transcript renders the same diff
    /// it did during the live session, instead of falling back to the
    /// arg-only `ReconstructedPreview`. Empty for legacy records.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub previews: HashMap<String, DiffPreview>,
    /// Sidebar pin. Pinned sessions float to the top of their project
    /// group. Absent (false) for records written before this existed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Web-sidebar archive: `Some(epoch secs)` when the user archived
    /// the session — hidden from the default sidebar list and the CLI
    /// resume picker until restored. `None` = live. (Deliberately
    /// distinct from `archived: Vec<Message>` above, which holds
    /// compaction-replaced messages.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<u64>,
    /// Web-sidebar "Settled": when the user put the chat away as done, or
    /// took it back out. Empty for chats the user never touched.
    #[serde(default, skip_serializing_if = "SettleMarks::is_empty")]
    pub settle: SettleMarks,
    /// An external agent drove (or drives) turns in this session. The agent's
    /// own transcript lives in the `<id>.agent.jsonl` sidecar, not in
    /// `messages` — see below. Absent for harness-only sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentSessionMeta>,
    /// Set on a chat made with "Fork from here": where it branched off.
    /// The sidebar nests it under that chat. Unlike `parent_id` (subagents)
    /// it's a full, user-facing chat, and deleting the original leaves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkPoint>,
}

/// Where a forked chat branched off its original.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkPoint {
    pub session_id: SessionId,
    /// The message the fork was taken at (its opening words), for the
    /// "forked from …" label.
    pub at: String,
}

/// Which external agent a session belongs to, for badges, replay and resume.
///
/// Deliberately metadata only: the transcript itself is the sidecar file.
/// Keeping them separate means listing (sidebar) never pays for content
/// (transcript), and deleting the record can cascade to the sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionMeta {
    /// Driver slug, e.g. `"claude-code"`.
    pub driver_kind: String,
    /// Engine instance that drove it, e.g. `"codex-work"`. Absent on
    /// records written before instances were routed: those always ran
    /// the default instance, whose id is its driver kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// The agent's running model, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether the agent is still this session's engine. `false` once the
    /// user switched back to a provider: the sidecar (and this record of
    /// who ran) stays for replay, but the session no longer routes to the
    /// agent. Defaults to `true` so records written before the flag
    /// existed keep meaning "agent session".
    #[serde(default = "default_true")]
    pub active: bool,
    /// The launch settings the agent ran with, secrets removed — what lets
    /// a session reloaded from disk (or after a restart) route its next
    /// prompt to the same agent, configured the same way, without the
    /// client re-sending anything. Opaque to the harness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// Why a chat couldn't be forked at a message.
#[derive(Debug, Error, PartialEq)]
pub enum ForkError {
    #[error("that message isn't in this chat's live history (it may have been compacted)")]
    MessageNotFound,
    #[error("forking chats run by an external agent isn't supported yet")]
    AgentChat,
}

impl SessionRecord {
    /// A new chat with this one's history through the turn opened by the
    /// `occurrence`-th most recent user message whose text is `text` (the
    /// same key edit & resend uses): that message and every reply to it,
    /// nothing after. Usage starts from zero, a running goal is dropped,
    /// and it isn't pinned or archived.
    pub fn fork_at(
        &self,
        new_id: SessionId,
        text: &str,
        occurrence: usize,
    ) -> Result<SessionRecord, ForkError> {
        if self.agent.as_ref().is_some_and(|a| a.active) {
            return Err(ForkError::AgentChat);
        }
        let is_prompt =
            |m: &Message| m.role == mira_core::Role::User && !crate::history::is_summary(m);
        let idx = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, m)| {
                m.role == mira_core::Role::User
                    && m.content.as_deref().map(crate::history::strip_hook_context) == Some(text)
            })
            .nth(occurrence)
            .map(|(i, _)| i)
            .ok_or(ForkError::MessageNotFound)?;
        // Through the end of that turn: up to the next prompt.
        let end = self.messages[idx + 1..]
            .iter()
            .position(is_prompt)
            .map_or(self.messages.len(), |p| idx + 1 + p);
        let messages: Vec<Message> = self.messages[..end].to_vec();

        let prompts = messages.iter().filter(|m| is_prompt(m)).count();
        let mut turns = self.turns.clone();
        turns.truncate(prompts);
        let kept_calls: std::collections::HashSet<String> = messages
            .iter()
            .flat_map(|m| m.tool_calls.iter().map(|c| c.id.to_string()))
            .collect();
        let previews = self
            .previews
            .iter()
            .filter(|(k, _)| kept_calls.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let at: String = text.trim().chars().take(80).collect();
        let base = self
            .title
            .clone()
            .or_else(|| self.first_user_message().map(str::to_string))
            .unwrap_or_else(|| "Chat".into());
        let now = now_secs();
        Ok(SessionRecord {
            id: new_id,
            cwd: self.cwd.clone(),
            cfg: self.cfg.clone(),
            messages,
            archived: self.archived.clone(),
            created_at: now,
            updated_at: now,
            title: Some(format!(
                "{} (fork)",
                base.chars().take(60).collect::<String>()
            )),
            turns,
            usage: UsageTotals::default(),
            file_calls_done: self.file_calls_done.clone(),
            parent_id: None,
            tasks: self.tasks.clone(),
            goal: None,
            previews,
            pinned: false,
            archived_at: None,
            settle: SettleMarks::default(),
            agent: None,
            forked_from: Some(ForkPoint {
                session_id: self.id.clone(),
                at,
            }),
        })
    }

    /// Every message of the conversation in order, as a person would
    /// read it: what compaction replaced, then the live history. System
    /// messages and compaction summaries are left out.
    pub fn conversation(&self) -> impl Iterator<Item = &Message> {
        self.archived
            .iter()
            .chain(self.messages.iter())
            .filter(|m| m.role != mira_core::Role::System && !crate::history::is_summary(m))
    }

    /// The first thing the user said (for titles and session lists).
    pub fn first_user_message(&self) -> Option<&str> {
        self.conversation()
            .find(|m| m.role == mira_core::Role::User)
            .and_then(|m| m.content.as_deref())
            .map(crate::history::strip_hook_context)
    }
}

/// Lightweight sidebar metadata. Never contains transcript, images, tool results, or diffs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionListRecord {
    pub id: SessionId,
    pub cwd: PathBuf,
    pub model: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    pub message_count: usize,
    pub usage: UsageTotals,
    pub parent_id: Option<SessionId>,
    pub pinned: bool,
    pub archived_at: Option<u64>,
    #[serde(default)]
    pub settle: SettleMarks,
    pub agent_driver: Option<String>,
    pub forked_from: Option<ForkPoint>,
}
impl From<&SessionRecord> for SessionListRecord {
    fn from(record: &SessionRecord) -> Self {
        Self {
            id: record.id.clone(),
            cwd: record.cwd.clone(),
            model: record.cfg.model.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
            title: record.title.clone(),
            first_user_message: record
                .first_user_message()
                .map(|text| text.chars().take(512).collect()),
            message_count: record.conversation().count(),
            usage: record.usage,
            parent_id: record.parent_id.clone(),
            pinned: record.pinned,
            archived_at: record.archived_at,
            settle: record.settle,
            agent_driver: record
                .agent
                .as_ref()
                .filter(|agent| agent.active)
                .map(|agent| agent.driver_kind.clone()),
            forked_from: record.forked_from.clone(),
        }
    }
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

/// Running token totals for a whole session. Grows monotonically; individual
/// rounds arrive as [`mira_ai::TokenUsage`] events which we fold in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    /// Prompt tokens written to the prompt cache (priced at a premium).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub cache_write_tokens: u64,
    /// Number of provider rounds we've seen a usage report for. May be
    /// smaller than the total number of turns if the provider omits usage on
    /// some responses.
    #[serde(default)]
    pub rounds: u32,
}

impl UsageTotals {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    /// The totals as one [`TokenUsage`], for pricing (saturates at `u32`).
    pub fn as_token_usage(&self) -> TokenUsage {
        let c = |n: u64| n.min(u32::MAX as u64) as u32;
        TokenUsage {
            prompt_tokens: c(self.prompt_tokens),
            completion_tokens: c(self.completion_tokens),
            cached_input_tokens: c(self.cached_input_tokens),
            cache_write_tokens: c(self.cache_write_tokens),
        }
    }

    /// Fold one provider-reported round into the running totals.
    pub fn add_round(&mut self, u: TokenUsage) {
        self.prompt_tokens += u.prompt_tokens as u64;
        self.completion_tokens += u.completion_tokens as u64;
        self.cached_input_tokens += u.cached_input_tokens as u64;
        self.cache_write_tokens += u.cache_write_tokens as u64;
        self.rounds = self.rounds.saturating_add(1);
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// One user→assistant round-trip's wall-clock timing. `ended_at == None`
/// means the turn is still in flight (or the process died mid-turn).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TurnMeta {
    /// Milliseconds since Unix epoch.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Tokens this turn used, summed over its provider rounds. Zero for
    /// turns recorded before per-turn usage existed.
    #[serde(default, skip_serializing_if = "UsageTotals::is_zero")]
    pub usage: UsageTotals,
    /// Model the turn's first round ran on — per-turn, because the
    /// session's model can change between turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error("session not found: {0}")]
    NotFound(String),

    #[error("no home directory")]
    NoHomeDir,
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, record: &SessionRecord) -> Result<(), StoreError>;
    async fn load(&self, id: &SessionId) -> Result<SessionRecord, StoreError>;
    /// Sessions in `cwd`, newest first.
    async fn list_recent(
        &self,
        cwd: &std::path::Path,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, StoreError>;
    /// Every session across all cwds, newest first. Used by the web
    /// sidebar to group chats by project — the CLI resume flow still uses
    /// `list_recent` scoped to the current folder. Archived sessions
    /// (see `SessionRecord::archived_at`) are excluded.
    async fn list_all(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError>;
    /// List only sidebar metadata. Other stores retain the full-record fallback.
    async fn list_metadata(
        &self,
        cwd: Option<&std::path::Path>,
        archived: bool,
        include_children: bool,
        limit: usize,
    ) -> Result<Vec<SessionListRecord>, StoreError> {
        let records = if archived {
            self.list_archived(usize::MAX).await?
        } else if let Some(cwd) = cwd {
            self.list_recent(cwd, usize::MAX).await?
        } else {
            self.list_all(usize::MAX).await?
        };
        Ok(records
            .iter()
            .filter(|record| include_children || record.parent_id.is_none())
            .take(limit)
            .map(SessionListRecord::from)
            .collect())
    }

    /// Direct child chats (forks and delegated agents), across all ages and folders.
    async fn list_related(
        &self,
        parent: &SessionId,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let mut records = self.list_all(usize::MAX).await?;
        records.retain(|record| {
            record.parent_id.as_ref() == Some(parent)
                || record
                    .forked_from
                    .as_ref()
                    .is_some_and(|fork| &fork.session_id == parent)
        });
        records.truncate(limit);
        Ok(records)
    }
    /// Sessions the user archived from the web sidebar, newest first.
    /// Kept separate from `list_recent`/`list_all` so "archived" stays a
    /// deliberate ask — those callers never want them mixed in.
    async fn list_archived(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError>;
    /// Permanently remove a stored session. Idempotent on `NotFound` — a
    /// double-click on the sidebar delete menu shouldn't 404.
    async fn delete(&self, id: &SessionId) -> Result<(), StoreError>;
    /// Provider runtime ledger sidecar. Its extension deliberately does
    /// not match session JSON so listing chats never parses request ledgers.
    /// Stores may enumerate pending input outboxes without loading transcripts.
    async fn queued_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        Ok(Vec::new())
    }

    fn queue_state_path(&self, id: &SessionId) -> Option<std::path::PathBuf> {
        self.agent_log_path(id).map(|p| p.with_extension("queue"))
    }

    fn runtime_state_path(&self, id: &SessionId) -> Option<std::path::PathBuf> {
        self.agent_log_path(id).map(|p| p.with_extension("runtime"))
    }

    /// Path of the agent-transcript sidecar for a session, if this store
    /// keeps one. `None` means agent turns are not persisted by this store.
    fn agent_log_path(&self, _id: &SessionId) -> Option<std::path::PathBuf> {
        None
    }
}

/// Seconds since the Unix epoch. Public so `goal.rs` can stamp
/// `Goal::created_at` without duplicating the shim.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Millisecond epoch — used for turn timing (a fast turn can be under a
/// second, so `now_secs` doesn't have the resolution we need).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod fork_tests {
    use super::*;

    fn record(messages: Vec<Message>) -> SessionRecord {
        SessionRecord {
            id: SessionId::from("sess_src"),
            cwd: PathBuf::from("/tmp"),
            cfg: SessionConfig::new("m"),
            messages,
            archived: Vec::new(),
            created_at: 1,
            updated_at: 2,
            title: Some("Fix the parser".into()),
            turns: vec![
                TurnMeta::default(),
                TurnMeta::default(),
                TurnMeta::default(),
            ],
            usage: UsageTotals {
                prompt_tokens: 9,
                ..Default::default()
            },
            file_calls_done: Vec::new(),
            parent_id: None,
            tasks: Vec::new(),
            goal: None,
            previews: HashMap::new(),
            pinned: true,
            archived_at: None,
            settle: SettleMarks::default(),
            agent: None,
            forked_from: None,
        }
    }

    #[test]
    fn keeps_history_through_the_chosen_turn() {
        let src = record(vec![
            Message::system("sys"),
            Message::user("one"),
            Message::assistant("a1"),
            Message::user("two"),
            Message::assistant("a2"),
            Message::user("three"),
            Message::assistant("a3"),
        ]);
        let f = src.fork_at(SessionId::from("sess_new"), "two", 0).unwrap();
        let texts: Vec<_> = f
            .messages
            .iter()
            .filter_map(|m| m.content.clone())
            .collect();
        assert_eq!(texts, ["sys", "one", "a1", "two", "a2"]);
        assert_eq!(f.turns.len(), 2);
        assert_eq!(f.title.as_deref(), Some("Fix the parser (fork)"));
        assert_eq!(
            f.forked_from.as_ref().unwrap().session_id,
            SessionId::from("sess_src")
        );
        assert_eq!(f.forked_from.as_ref().unwrap().at, "two");
        assert!(f.usage.is_zero() && !f.pinned && f.parent_id.is_none());
    }

    #[test]
    fn the_last_turn_keeps_everything() {
        let src = record(vec![Message::user("one"), Message::assistant("a1")]);
        let f = src.fork_at(SessionId::from("n"), "one", 0).unwrap();
        assert_eq!(f.messages.len(), 2);
    }

    #[test]
    fn counts_repeated_messages_from_the_latest() {
        let src = record(vec![
            Message::user("again"),
            Message::assistant("first"),
            Message::user("again"),
            Message::assistant("second"),
        ]);
        let f = src.fork_at(SessionId::from("n"), "again", 1).unwrap();
        assert_eq!(f.messages.len(), 2);
        assert_eq!(f.messages[1].content.as_deref(), Some("first"));
    }

    #[test]
    fn refuses_what_it_cant_fork() {
        let src = record(vec![Message::user("one")]);
        assert_eq!(
            src.fork_at(SessionId::from("n"), "missing", 0).unwrap_err(),
            ForkError::MessageNotFound
        );
        let mut agent = record(vec![Message::user("one")]);
        agent.agent = Some(AgentSessionMeta {
            driver_kind: "claude-code".into(),
            instance: None,
            model: None,
            active: true,
            launch: None,
        });
        assert_eq!(
            agent.fork_at(SessionId::from("n"), "one", 0).unwrap_err(),
            ForkError::AgentChat
        );
    }
}
