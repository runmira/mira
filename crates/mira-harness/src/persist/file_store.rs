use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mira_core::SessionId;
use tokio::fs;

use super::{SessionRecord, SessionStore, StoreError};

/// JSON-per-session file store.
///
/// Layout: `<root>/<session-id>.json`. Root defaults to
/// `~/.mira/sessions/`. Directory is created on first write.
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    /// Open the default store at `~/.mira/sessions/`.
    pub fn open_default() -> Result<Self, StoreError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(StoreError::NoHomeDir)?;
        Self::at(home.join(".mira").join("sessions"))
    }

    /// Open a store rooted at an arbitrary directory. Useful for tests.
    pub fn at(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path_for(&self, id: &SessionId) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
}

#[async_trait]
impl SessionStore for FileStore {
    async fn save(&self, record: &SessionRecord) -> Result<(), StoreError> {
        let path = self.path_for(&record.id);
        // Write to a sibling temp file then rename — atomic on POSIX, so a
        // crash mid-write can't leave a half-written session on disk.
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(record)?;
        fs::write(&tmp, &body).await?;
        fs::rename(&tmp, &path).await?;
        Ok(())
    }

    async fn load(&self, id: &SessionId) -> Result<SessionRecord, StoreError> {
        let path = self.path_for(id);
        let bytes = fs::read(&path)
            .await
            .map_err(|_| StoreError::NotFound(id.to_string()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn list_recent(
        &self,
        cwd: &Path,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let mut records = self.scan_all().await?;
        records.retain(|r| r.cwd == cwd);
        records.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        records.truncate(limit);
        Ok(records)
    }

    async fn list_all(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        let mut records = self.scan_all().await?;
        records.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        records.truncate(limit);
        Ok(records)
    }

    async fn delete(&self, id: &SessionId) -> Result<(), StoreError> {
        let path = self.path_for(id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            // Missing → treat as success. Users clicking "delete" twice on a
            // stale list shouldn't see a 404 spike back at them.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl FileStore {
    /// Load every record on disk. Malformed / unreadable files are silently
    /// skipped so one bad JSON doesn't break the whole sidebar.
    async fn scan_all(&self) -> Result<Vec<SessionRecord>, StoreError> {
        let mut records = Vec::new();
        let mut dir = fs::read_dir(&self.root).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(&path).await else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<SessionRecord>(&bytes) else {
                continue;
            };
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::Message;

    fn record(id: &str, cwd: &str, updated: u64) -> SessionRecord {
        SessionRecord {
            id: SessionId::from(id),
            cwd: PathBuf::from(cwd),
            cfg: crate::SessionConfig::new("test-model"),
            messages: vec![Message::user("hi")],
            created_at: updated,
            updated_at: updated,
            title: None,
            turns: Vec::new(),
            usage: Default::default(),
            parent_id: None,
            tasks: Vec::new(),
            goal: None,
        }
    }

    #[tokio::test]
    async fn round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let r = record("sess_1", "/repo/a", 42);
        store.save(&r).await.unwrap();
        let got = store.load(&r.id).await.unwrap();
        assert_eq!(got.id.as_str(), "sess_1");
        assert_eq!(got.messages.len(), 1);
    }

    #[tokio::test]
    async fn list_recent_by_cwd_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        store.save(&record("s1", "/repo/a", 10)).await.unwrap();
        store.save(&record("s2", "/repo/a", 30)).await.unwrap();
        store.save(&record("s3", "/repo/b", 40)).await.unwrap();
        let recent = store
            .list_recent(&PathBuf::from("/repo/a"), 10)
            .await
            .unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id.as_str(), "s2");
        assert_eq!(recent[1].id.as_str(), "s1");
    }
}
