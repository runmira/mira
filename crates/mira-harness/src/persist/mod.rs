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
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
