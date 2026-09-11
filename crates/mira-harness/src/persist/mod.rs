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

use std::path::PathBuf;

use async_trait::async_trait;
use mira_ai::TokenUsage;
use mira_core::{Message, SessionId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::session::SessionConfig;

pub use file_store::FileStore;

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

    /// Fold one provider-reported round into the running totals.
    pub fn add_round(&mut self, u: TokenUsage) {
        self.prompt_tokens += u.prompt_tokens as u64;
        self.completion_tokens += u.completion_tokens as u64;
        self.cached_input_tokens += u.cached_input_tokens as u64;
        self.rounds = self.rounds.saturating_add(1);
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// One user→assistant round-trip's wall-clock timing. `ended_at == None`
/// means the turn is still in flight (or the process died mid-turn).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnMeta {
    /// Milliseconds since Unix epoch.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
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
    /// `list_recent` scoped to the current folder.
    async fn list_all(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError>;
    /// Permanently remove a stored session. Idempotent on `NotFound` — a
    /// double-click on the sidebar delete menu shouldn't 404.
    async fn delete(&self, id: &SessionId) -> Result<(), StoreError>;
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Millisecond epoch — used for turn timing (a fast turn can be under a
/// second, so `now_secs` doesn't have the resolution we need).
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
