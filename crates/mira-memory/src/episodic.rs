//! Cross-session episodic memory.
//!
//! `<cwd>/.mira/episodic.jsonl` — one [`EpisodicEntry`] per line. This is the
//! *agent-written* stream: things the model or the harness noticed during a
//! session and thought worth carrying into future sessions ("we use vitest
//! not jest here", "the build script needs `SKIP_LINT=1`"). The user's
//! curated `MIRA.md` is a separate surface — the two never share a file so
//! consolidation and manual pruning can happen independently.
//!
//! JSONL because:
//! - Line-sized appends are effectively atomic (mutex here for correctness).
//! - Entries carry provenance (timestamp, source, session id) natively.
//! - Trivial to tail-read N most-recent entries for prompt rendering.
//! - Consolidation later just rewrites the file with fewer / merged lines.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::warn;

use crate::store::MemoryError;

/// Where an entry came from. Kept as an enum (not free text) so consolidation
/// passes and review UIs can filter cleanly — e.g. "show me only what the
/// model auto-wrote this week".
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodicSource {
    /// Post-round auto-extraction pass.
    Auto,
    /// The agent called `memory_remember` mid-conversation.
    Tool,
    /// A future UI-driven "pin this" or CLI import.
    Manual,
}

/// One line of episodic memory.
///
/// `timestamp` is seconds since Unix epoch — same convention the session
/// store uses; keeps everything comparable without a datetime dependency.
/// `session_id` is optional so the auto-extractor and tool paths, which do
/// know the current session, can attach it, while a plain CLI import can
/// leave it unset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpisodicEntry {
    pub text: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub source: EpisodicSource,
}

impl EpisodicEntry {
    /// Construct a fresh entry with the wall-clock now. Callers who want
    /// deterministic timestamps (tests, backfill) can build the struct
    /// directly.
    pub fn now(text: impl Into<String>, source: EpisodicSource) -> Self {
        Self {
            text: text.into(),
            timestamp: now_secs(),
            session_id: None,
            source,
        }
    }

    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
pub trait EpisodicStore: Send + Sync {
    /// Append one entry. Serialised to JSON on a single line — never
    /// pretty-printed, so tail-parsing stays simple. Empty text is
    /// rejected (nothing durable to remember).
    async fn append(&self, entry: EpisodicEntry) -> Result<(), MemoryError>;

    /// Most-recent-first slice, capped at `limit`. Malformed lines are
    /// logged and skipped so one bad write doesn't take out the whole
    /// history.
    async fn recent(&self, limit: usize) -> Result<Vec<EpisodicEntry>, MemoryError>;

    /// Atomically replace the file's entire contents with `entries`, in
    /// the order given. Used by the consolidation path — the naive
    /// alternative (truncate + repeated `append`) would be racy against
    /// any concurrent reader. Writes to a temp file next to the target
    /// and renames on success.
    async fn overwrite_all(&self, entries: Vec<EpisodicEntry>) -> Result<(), MemoryError>;

    /// On-disk path — for tools that quote it back to the model / user.
    fn path(&self) -> PathBuf;
}

/// JSONL-file-backed store. Only in-process concurrency is protected;
/// simultaneous writes from separate processes would still be safe *per
/// line* on POSIX (O_APPEND) up to `PIPE_BUF`, but the mutex covers the
/// full round-trip and is cheap enough not to matter.
pub struct FileEpisodicStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl FileEpisodicStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }
}

#[async_trait]
impl EpisodicStore for FileEpisodicStore {
    async fn append(&self, entry: EpisodicEntry) -> Result<(), MemoryError> {
        if entry.text.trim().is_empty() {
            return Err(MemoryError::Empty);
        }
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        f.write_all(line.as_bytes()).await?;
        Ok(())
    }

    async fn recent(&self, limit: usize) -> Result<Vec<EpisodicEntry>, MemoryError> {
        let body = match fs::read_to_string(&self.path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<EpisodicEntry> = Vec::new();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<EpisodicEntry>(trimmed) {
                Ok(e) => out.push(e),
                Err(e) => warn!(?e, line = %trimmed, "episodic: skip malformed entry"),
            }
        }
        // Most-recent last on disk (append-only), so tail from the end.
        let start = out.len().saturating_sub(limit);
        Ok(out.split_off(start))
    }

    async fn overwrite_all(&self, entries: Vec<EpisodicEntry>) -> Result<(), MemoryError> {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        // Temp-file + rename for atomicity. A crash between the two
        // leaves the original file intact; a crash after the rename
        // leaves the new file intact. Never a truncated JSONL.
        let tmp_path = tmp_path_for(&self.path);
        {
            let mut f = fs::File::create(&tmp_path).await?;
            for entry in &entries {
                let mut line = serde_json::to_string(entry)?;
                line.push('\n');
                f.write_all(line.as_bytes()).await?;
            }
            f.flush().await?;
        }
        fs::rename(&tmp_path, &self.path).await?;
        Ok(())
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(".tmp");
    match path.parent() {
        Some(p) => p.join(file_name),
        None => PathBuf::from(file_name),
    }
}

/// `<cwd>/.mira/episodic.jsonl` — the conventional path.
pub fn project_episodic_path(cwd: &Path) -> PathBuf {
    cwd.join(".mira").join("episodic.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in(dir: &Path) -> FileEpisodicStore {
        FileEpisodicStore::new(dir.join("episodic.jsonl"))
    }

    #[tokio::test]
    async fn recent_on_missing_file_returns_empty() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        assert!(s.recent(10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn append_then_recent_roundtrips() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(EpisodicEntry::now("first", EpisodicSource::Auto))
            .await
            .unwrap();
        s.append(EpisodicEntry::now("second", EpisodicSource::Tool))
            .await
            .unwrap();
        let recent = s.recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "first");
        assert_eq!(recent[1].text, "second");
        assert_eq!(recent[1].source, EpisodicSource::Tool);
    }

    #[tokio::test]
    async fn recent_respects_limit_and_returns_tail() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        for i in 0..5 {
            s.append(EpisodicEntry::now(format!("entry-{i}"), EpisodicSource::Auto))
                .await
                .unwrap();
        }
        let recent = s.recent(2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "entry-3");
        assert_eq!(recent[1].text, "entry-4");
    }

    #[tokio::test]
    async fn empty_text_is_rejected() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        let err = s
            .append(EpisodicEntry::now("   ", EpisodicSource::Tool))
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::Empty));
    }

    #[tokio::test]
    async fn malformed_lines_are_skipped_not_fatal() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("episodic.jsonl");
        std::fs::write(
            &path,
            "not json at all\n\
             {\"text\":\"good\",\"timestamp\":1,\"source\":\"auto\"}\n",
        )
        .unwrap();
        let s = FileEpisodicStore::new(path);
        let recent = s.recent(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].text, "good");
    }

    #[tokio::test]
    async fn concurrent_appends_all_land() {
        // Two parallel appends: mutex serializes them, both lines end up on
        // disk. Without the mutex, an interleaving could split a JSON line.
        let tmp = tempdir().unwrap();
        let s = Arc::new(store_in(tmp.path()));
        let a = {
            let s = s.clone();
            tokio::spawn(async move {
                s.append(EpisodicEntry::now("A", EpisodicSource::Tool))
                    .await
            })
        };
        let b = {
            let s = s.clone();
            tokio::spawn(async move {
                s.append(EpisodicEntry::now("B", EpisodicSource::Tool))
                    .await
            })
        };
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();
        let recent = s.recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        let texts: Vec<_> = recent.iter().map(|e| e.text.as_str()).collect();
        assert!(texts.contains(&"A"));
        assert!(texts.contains(&"B"));
    }
}
