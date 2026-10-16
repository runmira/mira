//! Global, ordered activity feed. Only active IDs are retained; completion
//! removes them. Snapshots repair reconnects and broadcast lag.
use crate::protocol::ServerMsg;
use std::{collections::BTreeSet, path::PathBuf, sync::Mutex, sync::OnceLock};
use tokio::sync::broadcast;
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActivitySnapshot {
    pub epoch: String,
    pub revision: u64,
    pub running: Vec<String>,
    /// Chats waiting on the user: an approval, question, plan or secret.
    pub waiting: Vec<String>,
}
struct State {
    revision: u64,
    running: BTreeSet<String>,
    waiting: BTreeSet<String>,
}
pub struct ActivityHub {
    epoch: String,
    state: Mutex<State>,
    tx: broadcast::Sender<ServerMsg>,
    /// Where the running set is kept on disk, so chats cut off by a
    /// restart can be found again (see [`crate::restart_recovery`]).
    persist: OnceLock<PathBuf>,
}
impl Default for ActivityHub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            state: Mutex::new(State {
                revision: 0,
                running: BTreeSet::new(),
                waiting: BTreeSet::new(),
            }),
            tx,
            persist: OnceLock::new(),
        }
    }
}
impl ActivityHub {
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMsg> {
        self.tx.subscribe()
    }
    pub fn snapshot(&self) -> ActivitySnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        ActivitySnapshot {
            epoch: self.epoch.clone(),
            revision: state.revision,
            running: state.running.iter().cloned().collect(),
            waiting: state.waiting.iter().cloned().collect(),
        }
    }
    pub fn set_waiting(&self, session_id: String, waiting: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let changed = if waiting {
            state.waiting.insert(session_id.clone())
        } else {
            state.waiting.remove(&session_id)
        };
        if !changed {
            return;
        }
        state.revision += 1;
        let _ = self.tx.send(ServerMsg::SessionWaiting {
            epoch: self.epoch.clone(),
            revision: state.revision,
            session_id,
            waiting,
        });
    }
    /// Keep the running set at `path` from now on.
    pub fn persist_to(&self, path: PathBuf) {
        if self.persist.set(path).is_ok() {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            self.save(&state.running);
        }
    }
    fn save(&self, running: &BTreeSet<String>) {
        let Some(path) = self.persist.get() else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        let ok = std::fs::write(&tmp, serde_json::to_vec(running).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = ok {
            tracing::warn!(%e, "session activity: could not save the running chats");
        }
    }
    pub fn set(&self, session_id: String, running: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let changed = if running {
            state.running.insert(session_id.clone())
        } else {
            state.running.remove(&session_id)
        };
        if !changed {
            return;
        }
        self.save(&state.running);
        state.revision += 1;
        // Send under the same lock as revision assignment to preserve ordering.
        let _ = self.tx.send(ServerMsg::SessionActivity {
            epoch: self.epoch.clone(),
            revision: state.revision,
            session_id,
            running,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_running_set_is_kept_on_disk_as_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("running.json");
        let hub = ActivityHub::default();
        hub.persist_to(path.clone());
        hub.set("a".into(), true);
        hub.set("b".into(), true);
        hub.set("a".into(), false);
        assert_eq!(crate::restart_recovery::previous(&path), ["b"]);
    }
    #[test]
    fn terminal_state_is_authoritative_and_completed_ids_do_not_accumulate() {
        let hub = ActivityHub::default();
        let mut rx = hub.subscribe();
        for n in 0..10000 {
            hub.set(n.to_string(), true);
            hub.set(n.to_string(), false);
        }
        assert!(hub.snapshot().running.is_empty());
        assert_eq!(hub.snapshot().revision, 20000);
        hub.set("s".into(), true);
        hub.set("s".into(), true);
        assert_eq!(hub.snapshot().revision, 20001);
        assert!(rx.try_recv().is_err()); // lag is repaired from the snapshot, not guessed.
    }
}
