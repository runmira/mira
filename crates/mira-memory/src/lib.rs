//! Agent-managed memory for Mira.
//!
//! Two files back the "long-term" side today:
//!
//! - `~/.mira/MIRA.md` — user-global, human-curated (plus `/remember`).
//! - `<cwd>/.mira/MIRA.md` — project-scoped, human-curated (plus `/remember`).
//!
//! A third store (`<cwd>/.mira/episodic.jsonl`) lands in a follow-up patch —
//! it's the *agent-written* stream of durable facts pulled out of past sessions.
//!
//! [`MemoryStore`] is the trait every backend implements; [`FileMemoryStore`]
//! is the on-disk one. Tests and future backends (SQLite, vector-backed, …)
//! slot in behind the trait without touching callers.

pub mod episodic;
pub mod file_store;
pub mod loader;
pub mod snapshot;
pub mod store;

pub use episodic::{
    project_episodic_path, EpisodicEntry, EpisodicSource, EpisodicStore, FileEpisodicStore,
};
pub use file_store::FileMemoryStore;
pub use loader::{load_memory_files, MemoryFile, MemoryKind, MEMORY_MAX_BYTES};
pub use snapshot::{FileMemorySnapshot, MemorySnapshot};
pub use store::{search, MemoryError, MemoryMatch, MemoryScope, MemoryStore};
