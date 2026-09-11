//! Task tracking — session-scoped todo list the model maintains as it
//! works through multi-step problems.
//!
//! Shape follows the Claude Code Task tools (`TaskCreate`,
//! `TaskUpdate`, `TaskList`, `TaskGet`):
//!
//! - Server assigns monotonic ids (1, 2, 3, …).
//! - Deletion is soft — `status = Deleted` keeps history intact so
//!   the persisted transcript can replay coherently. `TaskStore::list`
//!   filters deleted items out; `snapshot_all` includes them (used
//!   by the checkpoint path so state resumes losslessly).
//! - No blocking / owner semantics yet — v1 keeps the surface small.
//! - Mutations are behind a `Mutex`; contention is negligible (one
//!   caller per model round, no parallelism inside a session).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// One task the model is tracking during the session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskItem {
    /// Monotonic id assigned by the store on create. Starts at 1;
    /// never reused within a session even after deletion.
    pub id: u32,
    /// Imperative-form title, e.g. "Run tests".
    pub subject: String,
    /// What needs to be done — a sentence or two of detail. Optional
    /// on the wire; empty when the caller passed nothing.
    #[serde(default)]
    pub description: String,
    /// Present-continuous form shown while the task is `InProgress`,
    /// e.g. "Running tests". `None` = UI falls back to `subject`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    pub status: TaskStatus,
    /// Milliseconds since Unix epoch. Stamped on create.
    pub created_at: u64,
    /// Milliseconds since Unix epoch. Stamped on every mutation.
    pub updated_at: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    /// Soft-deleted. `TaskStore::list` filters these; the checkpoint
    /// path keeps them so a resumed session sees the identical id
    /// space and can never accidentally reuse a taken id.
    Deleted,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Deleted => "deleted",
        }
    }
}

/// Session-scoped, in-memory store. Wrapped in `Arc` so tools + the
/// harness's checkpoint path share one view. Persistence is handled by
/// the caller — see `snapshot_all` / `restore`.
#[derive(Debug, Default)]
pub struct TaskStore {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    items: Vec<TaskItem>,
    /// Next id to hand out. Advances even when tasks get deleted so
    /// ids stay stable across a session lifetime.
    next_id: u32,
}

/// Args accepted by [`TaskStore::update`]. Every field optional; only
/// non-`None` fields overwrite.
#[derive(Debug, Default)]
pub struct TaskUpdate {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub status: Option<TaskStatus>,
}

impl TaskStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Rebuild a store from a persisted snapshot. Re-establishes the
    /// id counter so newly-created tasks continue past the largest id
    /// we saw.
    pub fn restore(items: Vec<TaskItem>) -> Arc<Self> {
        let next_id = items.iter().map(|t| t.id).max().unwrap_or(0);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                items,
                next_id,
            }),
        })
    }

    /// Non-deleted tasks in id order. This is what tools + the UI
    /// render.
    pub async fn list(&self) -> Vec<TaskItem> {
        self.inner
            .lock()
            .await
            .items
            .iter()
            .filter(|t| t.status != TaskStatus::Deleted)
            .cloned()
            .collect()
    }

    /// Every task, deleted included. Used by the checkpoint path so
    /// resume can reproduce the exact id state.
    pub async fn snapshot_all(&self) -> Vec<TaskItem> {
        self.inner.lock().await.items.clone()
    }

    pub async fn get(&self, id: u32) -> Option<TaskItem> {
        self.inner
            .lock()
            .await
            .items
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    /// Add a new task in `Pending`. Returns the assigned id.
    pub async fn create(
        &self,
        subject: String,
        description: String,
        active_form: Option<String>,
    ) -> TaskItem {
        let mut inner = self.inner.lock().await;
        inner.next_id = inner.next_id.saturating_add(1);
        let now = now_ms();
        let item = TaskItem {
            id: inner.next_id,
            subject,
            description,
            active_form,
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        inner.items.push(item.clone());
        item
    }

    /// Apply a partial update by id. Returns the updated task, or
    /// `None` when no task with that id exists.
    pub async fn update(&self, id: u32, patch: TaskUpdate) -> Option<TaskItem> {
        let mut inner = self.inner.lock().await;
        let t = inner.items.iter_mut().find(|t| t.id == id)?;
        if let Some(s) = patch.subject {
            t.subject = s;
        }
        if let Some(d) = patch.description {
            t.description = d;
        }
        if let Some(a) = patch.active_form {
            t.active_form = Some(a);
        }
        if let Some(s) = patch.status {
            t.status = s;
        }
        t.updated_at = now_ms();
        Some(t.clone())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_assigns_monotonic_ids() {
        let store = TaskStore::new();
        let a = store.create("first".into(), "".into(), None).await;
        let b = store.create("second".into(), "".into(), None).await;
        assert_eq!(a.id, 1);
        assert_eq!(b.id, 2);
        assert_eq!(a.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn update_mutates_and_bumps_updated_at() {
        let store = TaskStore::new();
        let a = store.create("subj".into(), "".into(), None).await;
        // Sleep so the ms clock ticks.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let updated = store
            .update(
                a.id,
                TaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert!(updated.updated_at >= a.updated_at);
    }

    #[tokio::test]
    async fn deleted_hidden_from_list_kept_in_snapshot() {
        let store = TaskStore::new();
        let a = store.create("kept".into(), "".into(), None).await;
        let b = store.create("gone".into(), "".into(), None).await;
        store
            .update(
                b.id,
                TaskUpdate {
                    status: Some(TaskStatus::Deleted),
                    ..Default::default()
                },
            )
            .await;
        let visible = store.list().await;
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, a.id);
        let snap = store.snapshot_all().await;
        assert_eq!(snap.len(), 2);
    }

    #[tokio::test]
    async fn restore_continues_id_sequence() {
        let store = TaskStore::new();
        let a = store.create("a".into(), "".into(), None).await;
        let b = store.create("b".into(), "".into(), None).await;
        let snap = store.snapshot_all().await;
        drop(store);
        let restored = TaskStore::restore(snap);
        // Even though `a` (id=1) got deleted mid-flight, next
        // created id continues past the max we ever saw.
        restored
            .update(
                a.id,
                TaskUpdate {
                    status: Some(TaskStatus::Deleted),
                    ..Default::default()
                },
            )
            .await;
        let c = restored.create("c".into(), "".into(), None).await;
        assert!(c.id > b.id, "new id {} should exceed prior max {}", c.id, b.id);
    }
}
