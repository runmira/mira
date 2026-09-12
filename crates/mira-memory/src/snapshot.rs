//! Live memory rendering for the request-time system prompt.
//!
//! The [`MemorySnapshot`] trait exists so the harness can pull a *fresh*
//! memory block on every round without knowing where memory lives. This is
//! the piece that unbakes memory from the persisted `system` message — the
//! trait implementation goes to disk each call, so a `/remember` (or any
//! agent-side memory edit) is visible on the very next model turn instead
//! of only affecting new sessions.
//!
//! When the harness supplies a [`MemoryQuery`], the render call scores +
//! packs entries against the current turn's context under a token budget
//! (see `retrieval.rs`). When no query is supplied, `render()` falls back
//! to the old dump-everything shape — used by tests and callers that
//! don't have a conversation to key off of.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::episodic::EpisodicStore;
use crate::loader::{load_memory_files, MemoryKind};
use crate::retrieval::{
    approx_tokens, parse_md_bullets, select_top_k, CandidateSource, MemoryQuery,
    RetrievalCandidate, DEFAULT_TOKEN_BUDGET,
};

/// Anything that can render the current memory block for injection into
/// the system prompt.
///
/// Returning `None` means "nothing to inject this round" — the harness
/// then sends the request with just the fixed system prefix + conversation.
///
/// `query` drives retrieval-based selection under a token budget when
/// supplied; passing `None` gives the legacy dump-everything shape.
#[async_trait]
pub trait MemorySnapshot: Send + Sync {
    async fn render(&self, query: Option<&MemoryQuery>) -> Option<String>;
}

/// Default N recent episodic entries to include in the prompt when an
/// [`EpisodicStore`] is attached AND we're in dump-everything mode.
/// Retrieval mode uses its own always-include floor
/// ([`crate::retrieval::RECENT_EPISODIC_FLOOR`]) which is independent.
pub const DEFAULT_EPISODIC_LIMIT: usize = 20;

/// Default snapshot that reads user + project `MIRA.md` from disk.
///
/// Cheap enough to call once per round: two small file reads + a string
/// format. If either file changes between rounds (user edits it, agent
/// tool appends to it, `/remember` slash command fires), the next
/// `render()` picks it up automatically.
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

    /// Override how many episodic entries render into the prompt in
    /// dump-everything mode. Defaults to [`DEFAULT_EPISODIC_LIMIT`].
    /// Retrieval mode ignores this — see [`crate::retrieval::RECENT_EPISODIC_FLOOR`].
    pub fn with_episodic_limit(mut self, limit: usize) -> Self {
        self.episodic_limit = limit;
        self
    }
}

#[async_trait]
impl MemorySnapshot for FileMemorySnapshot {
    async fn render(&self, query: Option<&MemoryQuery>) -> Option<String> {
        let files = load_memory_files(&self.user_path, &self.project_path);

        // Episodic tail size depends on the mode: retrieval wants a wider
        // pool to score against; dump mode uses the smaller explicit
        // limit so we don't inflate the un-retrieved prompt.
        let episodic_pool_size = if query.is_some() { 100 } else { self.episodic_limit };
        let episodic = match self.episodic.as_ref() {
            Some(s) => s.recent(episodic_pool_size).await.unwrap_or_default(),
            None => Vec::new(),
        };

        if files.is_empty() && episodic.is_empty() {
            return None;
        }

        match query {
            Some(q) => render_retrieval(&files, &episodic, q),
            None => Some(render_dump(&files, &episodic)),
        }
    }
}

/// Legacy shape — pastes each file wholesale + a tail of episodic
/// entries. Preserved so callers that pass `None` get the exact
/// behaviour they used to get.
fn render_dump(
    files: &[crate::loader::MemoryFile],
    episodic: &[crate::episodic::EpisodicEntry],
) -> String {
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
        for e in episodic {
            out.push_str(&format!("- {}\n", e.text.trim()));
        }
    }
    out.trim_end().to_string()
}

/// Retrieval-based render — score everything, pack under budget,
/// re-group by source for a coherent read.
fn render_retrieval(
    files: &[crate::loader::MemoryFile],
    episodic: &[crate::episodic::EpisodicEntry],
    query: &MemoryQuery,
) -> Option<String> {
    let mut candidates: Vec<RetrievalCandidate> = Vec::new();
    for m in files {
        let source = match m.kind {
            MemoryKind::User => CandidateSource::UserMd,
            MemoryKind::Project => CandidateSource::ProjectMd,
        };
        candidates.extend(parse_md_bullets(&m.content, source));
    }
    for e in episodic {
        candidates.push(RetrievalCandidate::new_episodic(e));
    }

    if candidates.is_empty() {
        return None;
    }

    let selected = select_top_k(candidates, query);
    if selected.is_empty() {
        return None;
    }

    // Render grouped: User → Project → Episodic. `select_top_k` already
    // returns entries in this source order; we just walk and cluster.
    let mut out = String::new();
    let mut prev_source: Option<CandidateSource> = None;
    for c in &selected {
        if Some(c.source) != prev_source {
            if !out.is_empty() {
                out.push('\n');
            }
            let header = match c.source {
                CandidateSource::UserMd => "User memory (from ~/.mira/MIRA.md)",
                CandidateSource::ProjectMd => "Project memory (from .mira/MIRA.md)",
                CandidateSource::Episodic => "Learned across sessions (from .mira/episodic.jsonl)",
            };
            out.push_str(&format!("## {header}\n\n"));
            prev_source = Some(c.source);
        }
        out.push_str("- ");
        out.push_str(c.text.trim());
        out.push('\n');
    }

    // Debug marker for operators: the block trails with a one-liner
    // showing how much of the budget was consumed. Useful when tuning.
    let used = selected
        .iter()
        .map(|c| approx_tokens(&c.text) + 1)
        .sum::<usize>();
    let budget = query.budget();
    out.push_str(&format!(
        "\n<!-- memory: {} / {} entries · ~{} / {} tokens -->\n",
        selected.len(),
        budget_display(budget),
        used,
        budget,
    ));

    Some(out.trim_end().to_string())
}

fn budget_display(budget: usize) -> String {
    if budget == DEFAULT_TOKEN_BUDGET {
        format!("{budget} (default)")
    } else {
        budget.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn q(context: &str, budget: usize) -> MemoryQuery {
        MemoryQuery {
            context: context.into(),
            token_budget: Some(budget),
            now_secs: 1_700_000_000,
        }
    }

    #[tokio::test]
    async fn empty_dirs_render_none() {
        let tmp = tempdir().unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), tmp.path().join("p.md"));
        assert!(s.render(None).await.is_none());
        assert!(s.render(Some(&q("hello", 1000))).await.is_none());
    }

    #[tokio::test]
    async fn dump_mode_renders_labeled_sections() {
        let tmp = tempdir().unwrap();
        let u = tmp.path().join("u.md");
        let p = tmp.path().join("p.md");
        fs::write(&u, "- global thing").unwrap();
        fs::write(&p, "- repo thing").unwrap();
        let s = FileMemorySnapshot::new(u, p);
        let rendered = s.render(None).await.unwrap();
        assert!(rendered.contains("User memory"));
        assert!(rendered.contains("- global thing"));
        assert!(rendered.contains("Project memory"));
        assert!(rendered.contains("- repo thing"));
        // Dump mode has no budget marker.
        assert!(!rendered.contains("<!-- memory:"));
        // User section appears before the project section.
        let user_pos = rendered.find("User memory").unwrap();
        let proj_pos = rendered.find("Project memory").unwrap();
        assert!(user_pos < proj_pos);
    }

    #[tokio::test]
    async fn retrieval_mode_scores_and_filters() {
        // Two project bullets — only one is relevant to the query. With
        // a tight budget, only the relevant one should render.
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("p.md");
        fs::write(
            &p,
            "- we use pnpm not npm\n- dark mode toggle sits in header\n",
        )
        .unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), p);
        // Budget wide enough for one bullet + header + budget marker.
        let rendered = s
            .render(Some(&q("pnpm install failed", 20)))
            .await
            .unwrap();
        assert!(rendered.contains("pnpm"));
        assert!(rendered.contains("<!-- memory:"));
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
        let rendered = s.render(None).await.unwrap();
        assert!(rendered.contains("Learned across sessions"));
        assert!(rendered.contains("- we use pnpm not npm"));
    }

    #[tokio::test]
    async fn snapshot_renders_none_when_everything_empty() {
        let tmp = tempdir().unwrap();
        let epi = Arc::new(crate::FileEpisodicStore::new(tmp.path().join("ep.jsonl")));
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), tmp.path().join("p.md"))
            .with_episodic(epi);
        assert!(s.render(None).await.is_none());
    }

    #[tokio::test]
    async fn render_picks_up_updates_between_calls() {
        // Whole point of the snapshot: file edits between rounds are seen.
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("p.md");
        fs::write(&p, "- first").unwrap();
        let s = FileMemorySnapshot::new(tmp.path().join("u.md"), p.clone());
        let a = s.render(None).await.unwrap();
        assert!(a.contains("- first"));
        assert!(!a.contains("- second"));
        fs::write(&p, "- first\n- second").unwrap();
        let b = s.render(None).await.unwrap();
        assert!(b.contains("- first"));
        assert!(b.contains("- second"));
    }
}
