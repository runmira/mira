//! Live memory rendering for the request-time system prompt.
//!
//! The [`MemorySnapshot`] trait exists so the harness can pull a *fresh*
//! memory block on every round without knowing where memory lives. This is
//! the piece that unbakes memory from the persisted `system` message — the
//! trait implementation goes to disk each call, so a `/remember` (or any
//! agent-side memory edit) is visible on the very next model turn instead
//! of only affecting new sessions.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::episodic::EpisodicStore;
use crate::loader::{load_memory_files, MemoryKind};

/// Anything that can render the current memory block for injection into
/// the system prompt.
///
/// Returning `None` means "nothing to inject this round" — the harness
/// then sends the request with just the fixed system prefix + conversation.
#[async_trait]
pub trait MemorySnapshot: Send + Sync {
    async fn render(&self) -> Option<String>;
}

/// Default snapshot that reads user + project `MIRA.md` from disk.
///
/// Cheap enough to call once per round: two small file reads + a string
/// format. If either file changes between rounds (user edits it, agent
/// tool appends to it, `/remember` slash command fires), the next
/// `render()` picks it up automatically.
/// Default N recent episodic entries to include in the prompt when an
/// [`EpisodicStore`] is attached. Chosen small enough to fit comfortably
/// inside the second system message without pushing into the conversation's
/// working budget on long sessions.
pub const DEFAULT_EPISODIC_LIMIT: usize = 20;

pub struct FileMemorySnapshot {
    user_path: PathBuf,
    project_path: PathBuf,
    episodic: Option<Arc<dyn EpisodicStore>>,
    episodic_limit: usize,
}

impl FileMemorySnapshot {
    pub fn new(user_path: PathBuf, project_path: PathBuf) -> Self {
        Self {
            user_path,
            project_path,
            episodic: None,
            episodic_limit: DEFAULT_EPISODIC_LIMIT,
        }
    }

    /// Attach an episodic store so its most-recent entries render into a
    /// "Learned across sessions" section. Off by default so tests and any
    /// caller that doesn't want the cross-session surface can skip it.
    pub fn with_episodic(mut self, store: Arc<dyn EpisodicStore>) -> Self {
        self.episodic = Some(store);
        self
    }

    /// Override how many episodic entries render into the prompt. Defaults
    /// to [`DEFAULT_EPISODIC_LIMIT`].
    pub fn with_episodic_limit(mut self, limit: usize) -> Self {
        self.episodic_limit = limit;
        self
    }
}

#[async_trait]
impl MemorySnapshot for FileMemorySnapshot {
    async fn render(&self) -> Option<String> {
        let files = load_memory_files(&self.user_path, &self.project_path);
        let episodic = match self.episodic.as_ref() {
            Some(s) => s.recent(self.episodic_limit).await.unwrap_or_default(),
            None => Vec::new(),
        };
        if files.is_empty() && episodic.is_empty() {
            return None;
        }
        let mut out = String::new();
        for m in files {
            let header = match m.kind {
                MemoryKind::User => "User memory (from ~/.mira/MIRA.md)",
                MemoryKind::Project => "Project memory (from .mira/MIRA.md)",
            };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("## {header}\n\n{}\n", m.content.trim()));
        }
        if !episodic.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("## Learned across sessions (from .mira/episodic.jsonl)\n\n");
            // Most-recent last is easier for the model to weight — it reads
            // top-to-bottom, so older context comes first, freshest last.
            for e in &episodic {
                out.push_str(&format!("- {}\n", e.text.trim()));
            }
        }
        Some(out.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn empty_dirs_render_none() {
        let tmp = tempdir().unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), tmp.path().join("p.md"));
        assert!(s.render().await.is_none());
    }

    #[tokio::test]
    async fn user_and_project_render_labeled_sections() {
        let tmp = tempdir().unwrap();
        let u = tmp.path().join("u.md");
        let p = tmp.path().join("p.md");
        fs::write(&u, "- global thing").unwrap();
        fs::write(&p, "- repo thing").unwrap();
        let s = FileMemorySnapshot::new(u, p);
        let rendered = s.render().await.unwrap();
        assert!(rendered.contains("User memory"));
        assert!(rendered.contains("- global thing"));
        assert!(rendered.contains("Project memory"));
        assert!(rendered.contains("- repo thing"));
        // User section appears before the project section.
        let user_pos = rendered.find("User memory").unwrap();
        let proj_pos = rendered.find("Project memory").unwrap();
        assert!(user_pos < proj_pos);
    }

    #[tokio::test]
    async fn episodic_entries_render_in_dedicated_section() {
        let tmp = tempdir().unwrap();
        let epi = Arc::new(crate::FileEpisodicStore::new(tmp.path().join("ep.jsonl")));
        epi.append(crate::EpisodicEntry::now(
            "we use pnpm not npm",
            crate::EpisodicSource::Tool,
        ))
        .await
        .unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), tmp.path().join("p.md"))
            .with_episodic(epi);
        let rendered = s.render().await.unwrap();
        assert!(rendered.contains("Learned across sessions"));
        assert!(rendered.contains("- we use pnpm not npm"));
    }

    #[tokio::test]
    async fn snapshot_renders_none_when_everything_empty() {
        let tmp = tempdir().unwrap();
        let epi = Arc::new(crate::FileEpisodicStore::new(tmp.path().join("ep.jsonl")));
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), tmp.path().join("p.md"))
            .with_episodic(epi);
        assert!(s.render().await.is_none());
    }

    #[tokio::test]
    async fn render_picks_up_updates_between_calls() {
        // Whole point of the snapshot: file edits between rounds are seen.
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("p.md");
        fs::write(&p, "- first").unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), p.clone());
        let a = s.render().await.unwrap();
        assert!(a.contains("- first"));
        assert!(!a.contains("- second"));
        fs::write(&p, "- first\n- second").unwrap();
        let b = s.render().await.unwrap();
        assert!(b.contains("- first"));
        assert!(b.contains("- second"));
    }
}
