//! Requests a chat is waiting on the user for — approvals, questions, plan
//! reviews, secrets — kept so they can be shown again.
//!
//! Each request goes out once, as a frame on the chat's channel. A window
//! looking at another chat at that moment never sees it, so the chat sat
//! waiting with nothing to answer. Every waiting request is held here
//! until it settles, and re-sent right after the chat's `Ready` whenever a
//! window opens it.

use std::sync::{Arc, Mutex};

use crate::protocol::ServerMsg;

type Notify = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Default)]
struct Inner {
    held: Mutex<Vec<(String, ServerMsg)>>,
    /// Told `true` when the chat starts waiting on the user, `false` when
    /// nothing is left — the sidebar's "waiting on you", a delegated
    /// task's parent.
    notify: Mutex<Vec<Notify>>,
}

#[derive(Clone, Default)]
pub struct LiveRequests(Arc<Inner>);

impl LiveRequests {
    /// Report waiting / not waiting to `notify` from now on.
    pub fn on_change(&self, notify: impl Fn(bool) + Send + Sync + 'static) {
        self.0
            .notify
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(notify));
    }

    fn changed(&self, before: bool, after: bool) {
        if before != after {
            // Called outside the lock: a listener may hold or read requests.
            let listeners = self.0.notify.lock().unwrap_or_else(|e| e.into_inner()).clone();
            for n in listeners {
                n(after);
            }
        }
    }

    /// Hold `frame` under `id` until the returned guard drops — however
    /// the wait ends (answered, cancelled, timed out, turn aborted).
    pub fn hold(&self, id: impl Into<String>, frame: ServerMsg) -> LiveGuard {
        let id = id.into();
        let was = {
            let mut held = self.0.held.lock().unwrap_or_else(|e| e.into_inner());
            let was = !held.is_empty();
            held.retain(|(k, _)| *k != id);
            held.push((id.clone(), frame));
            was
        };
        self.changed(was, true);
        LiveGuard {
            requests: self.clone(),
            id,
        }
    }

    /// The waiting requests, oldest first.
    pub fn frames(&self) -> Vec<ServerMsg> {
        self.0
            .held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, f)| f.clone())
            .collect()
    }

    /// Ids of the waiting requests.
    pub fn ids(&self) -> Vec<String> {
        self.0
            .held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.held.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

pub struct LiveGuard {
    requests: LiveRequests,
    id: String,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        let (was, now) = {
            let mut held = self.requests.0.held.lock().unwrap_or_else(|e| e.into_inner());
            let was = !held.is_empty();
            held.retain(|(k, _)| *k != self.id);
            (was, !held.is_empty())
        };
        self.requests.changed(was, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_held_until_its_guard_drops() {
        let live = LiveRequests::default();
        let a = live.hold("a", ServerMsg::Error { text: "a".into() });
        {
            let _b = live.hold("b", ServerMsg::Error { text: "b".into() });
            assert_eq!(live.frames().len(), 2);
        }
        assert_eq!(live.frames().len(), 1);
        drop(a);
        assert!(live.is_empty());
    }

    #[test]
    fn waiting_is_reported_on_the_first_request_and_after_the_last() {
        let live = LiveRequests::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        live.on_change(move |w| log.lock().unwrap().push(w));
        let a = live.hold("a", ServerMsg::Error { text: "a".into() });
        let b = live.hold("b", ServerMsg::Error { text: "b".into() });
        drop(a);
        drop(b);
        assert_eq!(*seen.lock().unwrap(), [true, false]);
    }
}
