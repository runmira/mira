//! Global, ordered activity feed. Only active IDs are retained; completion
//! removes them. Snapshots repair reconnects and broadcast lag.
use crate::protocol::ServerMsg;
use std::{collections::BTreeSet, sync::Mutex};
use tokio::sync::broadcast;
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActivitySnapshot {
    pub epoch: String,
    pub revision: u64,
    pub running: Vec<String>,
}
struct State {
    revision: u64,
    running: BTreeSet<String>,
}
pub struct ActivityHub {
    epoch: String,
    state: Mutex<State>,
    tx: broadcast::Sender<ServerMsg>,
}
impl Default for ActivityHub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            state: Mutex::new(State {
                revision: 0,
                running: BTreeSet::new(),
            }),
            tx,
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
