use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mira_core::SessionId;
use tokio::fs;

use super::{SessionListRecord, SessionRecord, SessionStore, StoreError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// JSON-per-session file store.
///
/// Layout: `<root>/<session-id>.json`. Root defaults to
/// `~/.mira/sessions/`. Directory is created on first write.
pub struct FileStore {
    root: PathBuf,
    summaries: Arc<Mutex<HashMap<PathBuf, CachedSummary>>>,
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
        Ok(Self {
            root,
            summaries: Arc::new(Mutex::new(HashMap::new())),
        })
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
        let metadata = fs::metadata(&tmp).await?;
        let fingerprint = Fingerprint {
            len: metadata.len(),
            modified_ns: metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        };
        fs::rename(&tmp, &path).await?;
        let summary = SessionListRecord::from(record);
        // The transcript is authoritative; an unavailable index must not make a
        // successful checkpoint fail. Readers rebuild missing/stale sidecars.
        let _ =
            tokio::task::spawn_blocking(move || write_summary(&path, &summary, fingerprint)).await;
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
        let summaries = self.list_metadata(Some(cwd), false, true, limit).await?;
        self.load_listed(summaries).await
    }

    async fn list_all(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        self.load_listed(self.list_metadata(None, false, true, limit).await?)
            .await
    }

    async fn list_metadata(
        &self,
        cwd: Option<&Path>,
        archived: bool,
        include_children: bool,
        limit: usize,
    ) -> Result<Vec<SessionListRecord>, StoreError> {
        let root = self.root.clone();
        let cache = self.summaries.clone();
        let cwd = cwd.map(Path::to_path_buf);
        tokio::task::spawn_blocking(move || {
            let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
            let mut seen = std::collections::HashSet::new();
            let mut records = Vec::new();
            for entry in std::fs::read_dir(root)? {
                let path = entry?.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    continue;
                }
                seen.insert(path.clone());
                let Ok(fingerprint) = fingerprint(&path) else {
                    continue;
                };
                let summary = if let Some(cached) = cache
                    .get(&path)
                    .filter(|cached| cached.fingerprint == fingerprint)
                {
                    cached.summary.clone()
                } else {
                    let cached = std::fs::read(path.with_extension("summary"))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<CachedSummary>(&bytes).ok())
                        .filter(|cached| cached.version == 1 && cached.fingerprint == fingerprint);
                    let cached = match cached {
                        Some(cached) => cached,
                        None => {
                            let Ok(bytes) = std::fs::read(&path) else {
                                continue;
                            };
                            let Ok(record) = serde_json::from_slice::<SessionRecord>(&bytes) else {
                                continue;
                            };
                            // A concurrent checkpoint changed the file while reading:
                            // return its coherent snapshot, but don't cache it as current.
                            let summary = SessionListRecord::from(&record);
                            if super_fingerprint_matches(&path, &fingerprint) {
                                let cached = CachedSummary {
                                    version: 1,
                                    fingerprint: fingerprint.clone(),
                                    summary: summary.clone(),
                                };
                                let _ = persist_summary(&path, &cached);
                                cached
                            } else {
                                records.push(summary);
                                continue;
                            }
                        }
                    };
                    let summary = cached.summary.clone();
                    cache.insert(path.clone(), cached);
                    summary
                };
                records.push(summary);
            }
            cache.retain(|path, _| seen.contains(path));
            records.retain(|record| {
                record.archived_at.is_some() == archived
                    && cwd.as_ref().is_none_or(|cwd| &record.cwd == cwd)
                    && (include_children || record.parent_id.is_none())
            });
            records.sort_by_key(|record| {
                std::cmp::Reverse(if archived {
                    record.archived_at.unwrap_or(record.updated_at)
                } else {
                    record.updated_at
                })
            });
            records.truncate(limit);
            Ok(records)
        })
        .await
        .map_err(|error| StoreError::Io(std::io::Error::other(error)))?
    }

    async fn list_related(
        &self,
        parent: &SessionId,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        // The related-chat widget mounts with the conversation. Do not rescan
        // every transcript just to discover a handful of parent/child IDs.
        let mut summaries = self.list_metadata(None, false, true, usize::MAX).await?;
        summaries.extend(self.list_metadata(None, true, true, usize::MAX).await?);
        summaries.retain(|record| {
            record.parent_id.as_ref() == Some(parent)
                || record
                    .forked_from
                    .as_ref()
                    .is_some_and(|fork| &fork.session_id == parent)
        });
        summaries.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        summaries.truncate(limit);
        self.load_listed(summaries).await
    }

    async fn list_archived(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        self.load_listed(self.list_metadata(None, true, true, limit).await?)
            .await
    }

    /// Sidecar for the agent transcript. MUST live in the trait impl, not
    /// the inherent one: dynamic dispatch through `dyn SessionStore` only
    /// sees trait methods, and an inherent method with the same name
    /// silently shadows the default instead of overriding it. That exact
    /// mistake shipped once and every probe reported "no sidecar support".
    fn agent_log_path(&self, id: &SessionId) -> Option<PathBuf> {
        Some(self.root.join(format!("{id}.agent.jsonl")))
    }

    async fn queued_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        let mut entries = fs::read_dir(&self.root).await?;
        let mut ids = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".agent.queue") else {
                continue;
            };
            let Ok(bytes) = fs::read(&path).await else {
                continue;
            };
            let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
                continue;
            };
            if items
                .iter()
                .find(|i| i["delivered"] != true)
                .is_some_and(|i| i["dispatching"] != true && i["error"].is_null())
            {
                ids.push(SessionId::from(id));
            }
        }
        Ok(ids)
    }

    async fn delete(&self, id: &SessionId) -> Result<(), StoreError> {
        let path = self.path_for(id);
        match fs::remove_file(&path).await {
            Ok(()) => {}
            // Missing → treat as success. Users clicking "delete" twice on a
            // stale list shouldn't see a 404 spike back at them.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // Both sidecars belong to this session, missing or not.
        for sidecar in [
            self.agent_log_path(id),
            self.runtime_state_path(id),
            self.queue_state_path(id),
        ]
        .into_iter()
        .flatten()
        {
            match fs::remove_file(&sidecar).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let _ = fs::remove_file(path.with_extension("summary")).await;
        Ok(())
    }
}

impl FileStore {
    async fn load_listed(
        &self,
        summaries: Vec<SessionListRecord>,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let mut records = Vec::with_capacity(summaries.len());
        for summary in summaries {
            if let Ok(record) = self.load(&summary.id).await {
                records.push(record);
            }
        }
        Ok(records)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Fingerprint {
    len: u64,
    modified_ns: u128,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CachedSummary {
    version: u8,
    fingerprint: Fingerprint,
    summary: SessionListRecord,
}
fn fingerprint(path: &Path) -> std::io::Result<Fingerprint> {
    let metadata = std::fs::metadata(path)?;
    Ok(Fingerprint {
        len: metadata.len(),
        modified_ns: metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    })
}
fn super_fingerprint_matches(path: &Path, expected: &Fingerprint) -> bool {
    fingerprint(path).is_ok_and(|current| &current == expected)
}
fn write_summary(
    path: &Path,
    summary: &SessionListRecord,
    fingerprint: Fingerprint,
) -> std::io::Result<()> {
    if !super_fingerprint_matches(path, &fingerprint) {
        return Ok(());
    }
    persist_summary(
        path,
        &CachedSummary {
            version: 1,
            fingerprint,
            summary: summary.clone(),
        },
    )
}
fn persist_summary(path: &Path, summary: &CachedSummary) -> std::io::Result<()> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let destination = path.with_extension("summary");
    let tmp = path.with_extension(format!(
        "summary.tmp.{}.{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        std::fs::write(&tmp, serde_json::to_vec(summary)?)?;
        std::fs::rename(&tmp, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
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
            archived: Vec::new(),
            created_at: updated,
            updated_at: updated,
            title: None,
            turns: Vec::new(),
            usage: Default::default(),
            file_calls_done: Vec::new(),
            parent_id: None,
            tasks: Vec::new(),
            goal: None,
            previews: Default::default(),
            pinned: false,
            archived_at: None,
            agent: None,
            forked_from: None,
        }
    }

    #[tokio::test]
    async fn lightweight_listing_rebuilds_missing_corrupt_and_external_summaries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let mut legacy = record("legacy", "/repo", 1);
        legacy
            .messages
            .push(Message::assistant("large transcript ".repeat(10_000)));
        let path = store.path_for(&legacy.id);
        fs::write(&path, serde_json::to_vec(&legacy).unwrap())
            .await
            .unwrap();
        let first = store.list_metadata(None, false, false, 20).await.unwrap();
        assert_eq!(first[0].message_count, 2);
        assert_eq!(first[0].first_user_message.as_deref(), Some("hi"));
        let index = path.with_extension("summary");
        assert!(fs::metadata(&index).await.unwrap().len() < 2048);
        // Simulate an older CLI writing directly, outside this store/cache.
        legacy.title = Some("Updated by another process".into());
        legacy.pinned = true;
        legacy.updated_at = 2;
        fs::write(&path, serde_json::to_vec(&legacy).unwrap())
            .await
            .unwrap();
        let second = store.list_metadata(None, false, false, 20).await.unwrap();
        assert_eq!(second[0].title, legacy.title);
        assert!(second[0].pinned);
        fs::write(&index, b"broken index").await.unwrap();
        let reopened = FileStore::at(tmp.path()).unwrap();
        assert_eq!(
            reopened
                .list_metadata(None, false, false, 20)
                .await
                .unwrap()[0]
                .title,
            legacy.title
        );
        legacy.archived_at = Some(3);
        reopened.save(&legacy).await.unwrap();
        assert!(reopened
            .list_metadata(None, false, false, 20)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            reopened
                .list_metadata(None, true, false, 20)
                .await
                .unwrap()
                .len(),
            1
        );
        reopened.delete(&legacy.id).await.unwrap();
        assert!(!index.exists());
        assert!(reopened
            .list_metadata(None, true, false, 20)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn metadata_filters_children_before_limit_and_preserves_full_resume_history() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let mut child = record("child", "/repo", 3);
        child.parent_id = Some(SessionId::from("parent"));
        store.save(&child).await.unwrap();
        let mut parent = record("parent", "/repo", 2);
        parent.messages.push(Message::assistant("full response"));
        store.save(&parent).await.unwrap();
        store.save(&record("other", "/other", 4)).await.unwrap();
        let rows = store
            .list_metadata(Some(Path::new("/repo")), false, false, 1)
            .await
            .unwrap();
        assert_eq!(rows[0].id, parent.id);
        assert_eq!(
            store.list_recent(Path::new("/repo"), 1).await.unwrap()[0].id,
            child.id
        );
        assert_eq!(store.load(&parent.id).await.unwrap().messages.len(), 2);
    }

    #[tokio::test]
    async fn metadata_catalog_benchmark_large_history() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        for n in 0..300 {
            let mut item = record(&format!("chat-{n}"), "/repo", n);
            item.messages
                .push(Message::assistant("history payload ".repeat(4096)));
            store.save(&item).await.unwrap();
        }
        // A fresh process must use persisted summaries, not only a warm RAM cache.
        let reopened = FileStore::at(tmp.path()).unwrap();
        let started = std::time::Instant::now();
        let rows = reopened
            .list_metadata(None, false, false, 200)
            .await
            .unwrap();
        let summary_time = started.elapsed();
        assert_eq!(rows.len(), 200);
        let started = std::time::Instant::now();
        let mut bytes = 0;
        for n in 0..300 {
            let body = fs::read(store.path_for(&SessionId::from(format!("chat-{n}").as_str())))
                .await
                .unwrap();
            bytes += body.len();
            let _: SessionRecord = serde_json::from_slice(&body).unwrap();
        }
        eprintln!("300 chats, {bytes} transcript bytes: cold summaries {summary_time:?}, full transcript scan {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn relationships_include_old_forks_and_archived_delegates_but_exclude_unrelated() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let parent = record("parent", "/repo", 1);
        store.save(&parent).await.unwrap();
        let mut fork = record("fork", "/other-folder", 2);
        fork.forked_from = Some(super::super::ForkPoint {
            session_id: parent.id.clone(),
            at: "hi".into(),
        });
        store.save(&fork).await.unwrap();
        let mut child = record("child", "/repo", 3);
        child.parent_id = Some(parent.id.clone());
        child.archived_at = Some(4);
        store.save(&child).await.unwrap();
        store
            .save(&record("unrelated", "/repo", 100))
            .await
            .unwrap();
        let related = store.list_related(&parent.id, 20).await.unwrap();
        assert_eq!(
            related
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["child", "fork"]
        );
        assert_eq!(
            store.list_related(&parent.id, 1).await.unwrap()[0]
                .id
                .as_str(),
            "child"
        );
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
    async fn runtime_ledger_is_not_a_session_and_is_deleted_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let r = record("runtime_owner", "/repo/a", 42);
        store.save(&r).await.unwrap();
        let ledger = store.runtime_state_path(&r.id).unwrap();
        let transcript = store.agent_log_path(&r.id).unwrap();
        std::fs::write(&ledger, b"{\"requests\":[]}").unwrap();
        std::fs::write(&transcript, b"{}\n").unwrap();
        assert_eq!(store.list_all(10).await.unwrap().len(), 1);
        store.delete(&r.id).await.unwrap();
        assert!(!ledger.exists());
        assert!(!transcript.exists());
        assert!(store.list_all(10).await.unwrap().is_empty());
        store.delete(&r.id).await.unwrap();
    }

    #[tokio::test]
    async fn agent_approval_modes_persist_independently_per_chat() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        let mut a = record("chat_a", "/repo", 1);
        let b = record("chat_b", "/repo", 1);
        a.cfg.agent_approval_mode = mira_policy::Mode::Edit;
        store.save(&a).await.unwrap();
        store.save(&b).await.unwrap();
        assert_eq!(
            store.load(&a.id).await.unwrap().cfg.agent_approval_mode,
            mira_policy::Mode::Edit
        );
        assert_eq!(
            store.load(&b.id).await.unwrap().cfg.agent_approval_mode,
            mira_policy::Mode::Manual
        );
        let mut legacy = serde_json::to_value(&a.cfg).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("agent_approval_mode");
        let restored: crate::SessionConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.agent_approval_mode, mira_policy::Mode::Manual);
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

    #[tokio::test]
    async fn archived_sessions_are_hidden_until_asked_for() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::at(tmp.path()).unwrap();
        store.save(&record("s1", "/repo/a", 10)).await.unwrap();
        let mut gone = record("s2", "/repo/a", 20);
        gone.archived_at = Some(99);
        store.save(&gone).await.unwrap();

        // Default lists never surface archived sessions…
        assert_eq!(store.list_all(10).await.unwrap().len(), 1);
        assert_eq!(
            store
                .list_recent(&PathBuf::from("/repo/a"), 10)
                .await
                .unwrap()
                .len(),
            1
        );
        // …and the archived view returns only them, newest-archive first.
        let archived = store.list_archived(10).await.unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id.as_str(), "s2");
    }
    #[tokio::test]
    async fn queue_recovery_only_enumerates_dispatchable_outboxes() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::at(dir.path()).unwrap();
        for (id, body) in [
            (
                "pending",
                r#"[{"delivered":true},{"delivered":false,"dispatching":false,"error":null}]"#,
            ),
            ("uncertain", r#"[{"dispatching":true,"error":null}]"#),
            (
                "failed",
                r#"[{"dispatching":false,"error":"needs review"}]"#,
            ),
            ("receipt", r#"[{"delivered":true}]"#),
        ] {
            fs::write(store.queue_state_path(&SessionId::from(id)).unwrap(), body)
                .await
                .unwrap();
        }
        let ids = store.queued_sessions().await.unwrap();
        assert_eq!(ids, vec![SessionId::from("pending")]);
    }
}
