//! Byte-capped Markdown loader for the system-prompt injection path.
//!
//! Historically lived in `mira-config::load_memory_files`; moved here so all
//! memory reads and writes flow through one crate. The loader stays a pure
//! function (no store handle) because the system-prompt builder wants a
//! *snapshot* of what's on disk at a given moment, not a live handle.

use std::path::{Path, PathBuf};

/// One loaded memory file. Kept as a struct (rather than just `String`) so
/// the caller can label the section in the prompt — the model reads better
/// when it knows a chunk is "user-level" vs "project-level".
#[derive(Clone, Debug)]
pub struct MemoryFile {
    pub kind: MemoryKind,
    pub path: PathBuf,
    pub content: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// `~/.mira/MIRA.md` — user's global preferences and conventions.
    User,
    /// `<cwd>/.mira/MIRA.md` — repo-specific rules and context.
    Project,
}

/// Cap per file. A runaway MIRA.md shouldn't blow the model's context; the
/// surplus is elided with a marker so the user notices in-band.
pub const MEMORY_MAX_BYTES: usize = 32 * 1024;

/// Load user + project memory files. Missing files are skipped silently — a
/// fresh repo with no MIRA.md is not an error. Files past
/// [`MEMORY_MAX_BYTES`] are truncated with a marker rather than rejected so
/// a badly-sized file doesn't break the whole session.
///
/// Ordering: user memory first, project memory second. Later content carries
/// more weight in typical LLM behavior, so project-specific rules override
/// user-global preferences when they conflict.
pub fn load_memory_files(user_path: &Path, project_path: &Path) -> Vec<MemoryFile> {
    let mut out = Vec::new();
    if let Some(m) = read_memory(MemoryKind::User, user_path) {
        out.push(m);
    }
    if let Some(m) = read_memory(MemoryKind::Project, project_path) {
        out.push(m);
    }
    out
}

fn read_memory(kind: MemoryKind, path: &Path) -> Option<MemoryFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    let content = if raw.len() > MEMORY_MAX_BYTES {
        let cut = floor_char_boundary(&raw, MEMORY_MAX_BYTES);
        format!(
            "{}\n\n… [memory file truncated at {MEMORY_MAX_BYTES} bytes; the rest was skipped]",
            &raw[..cut]
        )
    } else {
        raw
    };
    // Empty (or whitespace-only) files are placeholders the user hasn't
    // filled in — skipping keeps a stray empty section out of the prompt.
    if content.trim().is_empty() {
        return None;
    }
    Some(MemoryFile {
        kind,
        path: path.to_path_buf(),
        content,
    })
}

/// UTF-8-safe truncation: back off to the previous char boundary so we never
/// slice mid-codepoint. `str::floor_char_boundary` is nightly-only, hence
/// the hand-rolled version.
fn floor_char_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_files_yield_empty() {
        let tmp = tempdir().unwrap();
        let out = load_memory_files(&tmp.path().join("u.md"), &tmp.path().join("p.md"));
        assert!(out.is_empty());
    }

    #[test]
    fn user_first_then_project() {
        let tmp = tempdir().unwrap();
        let u = tmp.path().join("u.md");
        let p = tmp.path().join("p.md");
        fs::write(&u, "USER").unwrap();
        fs::write(&p, "PROJ").unwrap();
        let out = load_memory_files(&u, &p);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, MemoryKind::User);
        assert_eq!(out[1].kind, MemoryKind::Project);
    }

    #[test]
    fn oversize_is_truncated_with_marker() {
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("p.md");
        fs::write(&p, "x".repeat(MEMORY_MAX_BYTES + 100)).unwrap();
        let out = load_memory_files(&tmp.path().join("missing.md"), &p);
        assert_eq!(out.len(), 1);
        assert!(out[0].content.contains("truncated"));
        assert!(out[0].content.len() <= MEMORY_MAX_BYTES + 200);
    }

    #[test]
    fn empty_file_is_skipped() {
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("p.md");
        fs::write(&p, "   \n\n").unwrap();
        let out = load_memory_files(&tmp.path().join("missing.md"), &p);
        assert!(out.is_empty());
    }
}
