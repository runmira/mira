//! Sequenced session events and bounded reconnect replay. The raw channel
//! remains available to persistence/sinks; sockets receive explicit ownership.
use crate::protocol::ServerMsg;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StreamCursor {
    pub epoch: String,
    pub sequence: u64,
}
struct Retained {
    sequence: u64,
    bytes: usize,
    frame: ServerMsg,
}
struct Journal {
    session: String,
    cursor: StreamCursor,
    bytes: usize,
    frames: VecDeque<Retained>,
}
#[derive(Clone)]
pub struct SessionEvents {
    raw: broadcast::Sender<ServerMsg>,
    live: broadcast::Sender<ServerMsg>,
    journal: Arc<Mutex<Journal>>,
}
const MAX_FRAMES: usize = 2048;
const MAX_BYTES: usize = 4 * 1024 * 1024;
impl SessionEvents {
    pub fn new(session: &str) -> Self {
        let (raw, _) = broadcast::channel(256);
        Self::with_sender(session, raw)
    }
    fn with_sender(session: &str, raw: broadcast::Sender<ServerMsg>) -> Self {
        let (live, _) = broadcast::channel(256);
        Self {
            raw,
            live,
            journal: Arc::new(Mutex::new(Journal {
                session: session.to_owned(),
                cursor: StreamCursor {
                    epoch: uuid::Uuid::new_v4().to_string(),
                    sequence: 0,
                },
                bytes: 0,
                frames: VecDeque::new(),
            })),
        }
    }
    pub fn bind_session(&self, session: &str) {
        let mut journal = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        journal.session = session.into();
    }
    pub fn cursor(&self) -> StreamCursor {
        self.journal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cursor
            .clone()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMsg> {
        self.raw.subscribe()
    }
    pub fn subscribe_stream(&self) -> broadcast::Receiver<ServerMsg> {
        self.live.subscribe()
    }
    pub fn receiver_count(&self) -> usize {
        self.raw.receiver_count() + self.live.receiver_count()
    }
    pub fn send(
        &self,
        frame: ServerMsg,
    ) -> Result<usize, broadcast::error::SendError<Box<ServerMsg>>> {
        let mut journal = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        // Snapshots are connection control, not replayable mutations.
        if matches!(frame, ServerMsg::Ready { .. }) {
            let listeners = self.receiver_count();
            let _ = self.live.send(frame.clone());
            let _ = self.raw.send(frame.clone());
            return if listeners > 0 {
                Ok(listeners)
            } else {
                Err(broadcast::error::SendError(Box::new(frame)))
            };
        }
        journal.cursor.sequence += 1;
        let sequence = journal.cursor.sequence;
        let stamped = ServerMsg::SessionStream {
            session_id: journal.session.clone(),
            epoch: journal.cursor.epoch.clone(),
            sequence,
            frame: Box::new(frame.clone()),
        };
        let bytes = serde_json::to_vec(&stamped)
            .map(|v| v.len())
            .unwrap_or(MAX_BYTES + 1);
        journal.bytes += bytes;
        journal.frames.push_back(Retained {
            sequence,
            bytes,
            frame: stamped.clone(),
        });
        while journal.frames.len() > MAX_FRAMES || journal.bytes > MAX_BYTES {
            if let Some(old) = journal.frames.pop_front() {
                journal.bytes -= old.bytes;
            } else {
                break;
            }
        }
        let listeners = self.receiver_count();
        let _ = self.live.send(stamped);
        let _ = self.raw.send(frame.clone());
        if listeners > 0 {
            Ok(listeners)
        } else {
            Err(broadcast::error::SendError(Box::new(frame)))
        }
    }
    /// None means wrong epoch, future cursor, or a gap that expired.
    pub fn replay(&self, after: &StreamCursor) -> Option<Vec<ServerMsg>> {
        let journal = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        if after.epoch != journal.cursor.epoch || after.sequence > journal.cursor.sequence {
            return None;
        }
        if after.sequence < journal.cursor.sequence
            && journal
                .frames
                .front()
                .is_none_or(|f| f.sequence > after.sequence + 1)
        {
            return None;
        }
        Some(
            journal
                .frames
                .iter()
                .filter(|f| f.sequence > after.sequence)
                .map(|f| f.frame.clone())
                .collect(),
        )
    }
}
impl From<broadcast::Sender<ServerMsg>> for SessionEvents {
    fn from(sender: broadcast::Sender<ServerMsg>) -> Self {
        Self::with_sender("legacy", sender)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_has_stable_scope_and_cursor_even_without_attached_clients() {
        let a = SessionEvents::new("a");
        let b = SessionEvents::new("b");
        let cursor = a.cursor();
        let _ = a.send(ServerMsg::Token { text: "one".into() });
        let _ = a.send(ServerMsg::Token { text: "two".into() });
        let replay = a.replay(&cursor).unwrap();
        assert_eq!(replay.len(), 2);
        assert!(
            matches!(&replay[0], ServerMsg::SessionStream {session_id, sequence: 1, ..} if session_id == "a")
        );
        assert!(b.replay(&cursor).is_none());
        assert!(a.replay(&a.cursor()).unwrap().is_empty());
    }
    #[test]
    fn expired_oversized_and_future_cursors_require_a_snapshot() {
        let a = SessionEvents::new("a");
        let cursor = a.cursor();
        for _ in 0..MAX_FRAMES + 1 {
            let _ = a.send(ServerMsg::Done);
        }
        assert!(a.replay(&cursor).is_none());
        let current = a.cursor();
        let _ = a.send(ServerMsg::Token {
            text: "x".repeat(MAX_BYTES + 1),
        });
        assert!(a.replay(&current).is_none());
        let mut future = a.cursor();
        future.sequence += 1;
        assert!(a.replay(&future).is_none());
    }
}
