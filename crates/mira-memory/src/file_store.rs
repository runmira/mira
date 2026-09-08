//! On-disk `MIRA.md` backend for [`MemoryStore`].
//!
//! Writes are atomic (temp-file + rename, same trick the session store uses).
//! Concurrent same-process writers are serialized per-scope via a Tokio mutex
//! so the read → mutate → write cycle inside `append`/`replace` can't lose
//! updates to interleaving. Cross-process racing (two `mira` invocations on
//! the same repo) is left uncovered here — vanishingly rare in practice and
//! would need `fs2`/`fd-lock` for advisory locks; we can add it later without
//! changing the surface.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::debug;

use crate::store::{MemoryError, MemoryScope, MemoryStore};

pub struct FileMemoryStore {
    user_path: PathBuf,
    project_path: PathBuf,
    /// Serializes read-modify-write for the user scope.
    user_lock: Arc<Mutex<()>>,
    /// Serializes read-modify-write for the project scope.
    project_lock: Arc<Mutex<()>>,
}

impl FileMemoryStore {
    /// Build a store rooted at the two conventional paths. Callers pull the
    /// paths from `mira-config` (`user_memory_path` and `<cwd>/.mira/MIRA.md`)
    /// — this crate stays free of path policy so tests can point it anywhere.
    pub fn new(user_path: impl Into<PathBuf>, project_path: impl Into<PathBuf>) -> Self {
        Self {
            user_path: user_path.into(),
            project_path: project_path.into(),
            user_lock: Arc::new(Mutex::new(())),
            project_lock: Arc::new(Mutex::new(())),
        }
    }

    fn lock_for(&self, scope: MemoryScope) -> Arc<Mutex<()>> {
        match scope {
            MemoryScope::User => self.user_lock.clone(),
            MemoryScope::Project => self.project_lock.clone(),
        }
    }
}

#[async_trait]
impl MemoryStore for FileMemoryStore {
    async fn read(&self, scope: MemoryScope) -> Result<String, MemoryError> {
        let path = self.path(scope);
        match fs::read_to_string(&path).await {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    async fn append(&self, scope: MemoryScope, text: &str) -> Result<u64, MemoryError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MemoryError::Empty);
        }
        let lock = self.lock_for(scope);
        let _guard = lock.lock().await;

        let path = self.path(scope);
        let existing = read_or_empty(&path).await?;
        let mut body = existing;
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&bullet_form(text));
        body.push('\n');
        atomic_write(&path, body.as_bytes()).await?;
        debug!(?scope, path = %path.display(), bytes = body.len(), "memory append");
        Ok(body.len() as u64)
    }

    async fn replace(
        &self,
        scope: MemoryScope,
        old: &str,
        new: &str,
    ) -> Result<u64, MemoryError> {
        if old.is_empty() {
            return Err(MemoryError::Empty);
        }
        let lock = self.lock_for(scope);
        let _guard = lock.lock().await;

        let path = self.path(scope);
        let existing = read_or_empty(&path).await?;
        let count = existing.matches(old).count();
        match count {
            0 => Err(MemoryError::NotFound(old.to_string())),
            1 => {
                let updated = existing.replacen(old, new, 1);
                atomic_write(&path, updated.as_bytes()).await?;
                debug!(?scope, path = %path.display(), bytes = updated.len(), "memory replace");
                Ok(updated.len() as u64)
            }
            n => Err(MemoryError::Ambiguous {
                needle: old.to_string(),
                count: n,
            }),
        }
    }

    async fn overwrite(&self, scope: MemoryScope, content: &str) -> Result<u64, MemoryError> {
        let lock = self.lock_for(scope);
        let _guard = lock.lock().await;

        let path = self.path(scope);
        atomic_write(&path, content.as_bytes()).await?;
        debug!(?scope, path = %path.display(), bytes = content.len(), "memory overwrite");
        Ok(content.len() as u64)
    }

    fn path(&self, scope: MemoryScope) -> PathBuf {
        match scope {
            MemoryScope::User => self.user_path.clone(),
            MemoryScope::Project => self.project_path.clone(),
        }
    }
}

/// Multi-line notes render as a single bullet: first line prefixed `- `,
/// continuation lines indented two spaces. Matches the existing
/// `/remember` HTTP endpoint's formatting so both paths produce
/// identically-shaped files.
fn bullet_form(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                format!("- {l}")
            } else {
                format!("  {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn read_or_empty(path: &Path) -> Result<String, MemoryError> {
    match fs::read_to_string(path).await {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// Temp-file + rename. Rename is atomic on POSIX and on Windows for same-vol
/// same-name replacements, so a crash mid-write can't leave a torn file.
async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, bytes).await?;
    fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in(dir: &Path) -> FileMemoryStore {
        FileMemoryStore::new(dir.join("user.md"), dir.join("project.md"))
    }

    #[tokio::test]
    async fn read_missing_returns_empty() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        assert_eq!(s.read(MemoryScope::User).await.unwrap(), "");
    }

    #[tokio::test]
    async fn append_creates_and_grows() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(MemoryScope::Project, "first").await.unwrap();
        s.append(MemoryScope::Project, "second").await.unwrap();
        let body = s.read(MemoryScope::Project).await.unwrap();
        assert_eq!(body, "- first\n- second\n");
    }

    #[tokio::test]
    async fn append_multiline_indents_continuations() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(MemoryScope::User, "line one\nline two")
            .await
            .unwrap();
        let body = s.read(MemoryScope::User).await.unwrap();
        assert_eq!(body, "- line one\n  line two\n");
    }

    #[tokio::test]
    async fn append_rejects_empty() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        let err = s.append(MemoryScope::User, "   ").await.unwrap_err();
        assert!(matches!(err, MemoryError::Empty));
    }

    #[tokio::test]
    async fn replace_unique_hit_rewrites_file() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(MemoryScope::Project, "old fact").await.unwrap();
        s.replace(MemoryScope::Project, "old fact", "new fact")
            .await
            .unwrap();
        let body = s.read(MemoryScope::Project).await.unwrap();
        assert_eq!(body, "- new fact\n");
    }

    #[tokio::test]
    async fn replace_missing_errors() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(MemoryScope::Project, "something").await.unwrap();
        let err = s
            .replace(MemoryScope::Project, "nope", "irrelevant")
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn replace_ambiguous_errors() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.overwrite(MemoryScope::Project, "dup\ndup\n").await.unwrap();
        let err = s
            .replace(MemoryScope::Project, "dup", "unique")
            .await
            .unwrap_err();
        match err {
            MemoryError::Ambiguous { count, .. } => assert_eq!(count, 2),
            e => panic!("unexpected error: {e:?}"),
        }
    }

    #[tokio::test]
    async fn concurrent_appends_do_not_lose_updates() {
        // Two parallel appends against the same scope must both land — the
        // per-scope mutex serializes the read-modify-write cycle. Without
        // it, whichever writer finishes last would clobber the other.
        let tmp = tempdir().unwrap();
        let s = Arc::new(store_in(tmp.path()));
        let a = {
            let s = s.clone();
            tokio::spawn(async move { s.append(MemoryScope::Project, "A").await })
        };
        let b = {
            let s = s.clone();
            tokio::spawn(async move { s.append(MemoryScope::Project, "B").await })
        };
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();
        let body = s.read(MemoryScope::Project).await.unwrap();
        assert!(body.contains("- A\n"));
        assert!(body.contains("- B\n"));
        assert_eq!(body.lines().count(), 2);
    }

    #[tokio::test]
    async fn overwrite_replaces_everything() {
        let tmp = tempdir().unwrap();
        let s = store_in(tmp.path());
        s.append(MemoryScope::User, "keep").await.unwrap();
        s.overwrite(MemoryScope::User, "fresh\n").await.unwrap();
        assert_eq!(s.read(MemoryScope::User).await.unwrap(), "fresh\n");
    }
}
