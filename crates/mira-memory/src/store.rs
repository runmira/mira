use std::path::PathBuf;

use async_trait::async_trait;
use thiserror::Error;

/// Which memory file a call targets.
///
/// `User` = `~/.mira/MIRA.md`. `Project` = `<cwd>/.mira/MIRA.md`.
/// Auto-extraction lands in the *episodic* store (JSONL), which is a
/// separate surface — not addressed by this enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    User,
    Project,
}

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// `replace` couldn't find `old` in the file.
    #[error("string not found in memory: {0:?}")]
    NotFound(String),

    /// `replace` found `old` more than once — caller must widen the anchor.
    #[error("string appears {count} times in memory; anchor must be unique: {needle:?}")]
    Ambiguous { needle: String, count: usize },

    #[error("memory text is empty")]
    Empty,
}

/// One line-level hit produced by [`search`]. `line_no` is 1-based, which
/// matches how humans read files and how our other tools report locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryMatch {
    pub line_no: usize,
    pub line: String,
}

/// Backend-agnostic memory storage.
///
/// The trait is intentionally small: `read` and three write shapes
/// (`append` for the common bullet-add path, `replace` for targeted edits,
/// `overwrite` as the escape hatch). Anything more elaborate — search,
/// dedup, retrieval scoring — is layered on top of `read`.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Whole file contents. Missing files come back as `""` — a fresh repo
    /// with no MIRA.md is not an error.
    async fn read(&self, scope: MemoryScope) -> Result<String, MemoryError>;

    /// Append `text` as a Markdown bullet. Multi-line notes render as one
    /// bullet with the continuation lines indented, matching the existing
    /// `/remember` endpoint so both paths produce identical formatting.
    /// Returns the file's new byte count.
    async fn append(&self, scope: MemoryScope, text: &str) -> Result<u64, MemoryError>;

    /// Replace exactly one occurrence of `old` with `new`. Errors out if
    /// `old` is missing or appears more than once — same rule the `Edit`
    /// file tool uses, and for the same reason (silent multi-replace is a
    /// footgun). Returns the new byte count.
    async fn replace(
        &self,
        scope: MemoryScope,
        old: &str,
        new: &str,
    ) -> Result<u64, MemoryError>;

    /// Blow away the file's contents and rewrite from scratch. Meant for
    /// consolidation passes; day-to-day edits should go through `replace`.
    /// Returns the new byte count.
    async fn overwrite(&self, scope: MemoryScope, content: &str) -> Result<u64, MemoryError>;

    /// Resolved on-disk path for a scope. Kept on the trait so tool
    /// responses can quote it back to the model / user without having to
    /// re-derive it.
    fn path(&self, scope: MemoryScope) -> PathBuf;
}

/// Case-insensitive substring line search over a scope's contents.
///
/// Lives outside the trait so backends don't have to implement it — every
/// store can supply `read`, and search is a pure function of that. If a
/// future backend has a native index (embeddings, FTS), it can shadow this
/// with its own method.
pub async fn search(
    store: &dyn MemoryStore,
    scope: MemoryScope,
    needle: &str,
) -> Result<Vec<MemoryMatch>, MemoryError> {
    if needle.is_empty() {
        return Err(MemoryError::Empty);
    }
    let body = store.read(scope).await?;
    let needle_lc = needle.to_lowercase();
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if line.to_lowercase().contains(&needle_lc) {
            out.push(MemoryMatch {
                line_no: i + 1,
                line: line.to_string(),
            });
        }
    }
    Ok(out)
}
