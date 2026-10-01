//! The bidirectional JSON-RPC connection to a spawned agent.
//!
//! Transport-agnostic on purpose: it takes an `AsyncRead`/`AsyncWrite`
//! pair rather than a `ChildProcess`, so the whole thing — id
//! demultiplexing, cancellation, dispatch — is testable against an
//! in-memory pipe with no agent binary present. [`spawn`] is the thin
//! adapter that supplies a real process's stdio.
//!
//! The problem this file exists to get right: **a JSON-RPC connection has a
//! single id space, not one per direction.** While our request 5 is
//! outstanding the agent may issue its own request 5, and its response to
//! our 5 may arrive while it is still sending updates. Every frame is
//! therefore classified against the pending-request map: a known id is a
//! response to us, an unknown id with a method is a call into us, and no
//! id at all is a notification. Get this wrong and you deadlock.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::framing::{encode_frame, pump_stdout, Inbound, StderrTail, Stdin};

/// Cap on in-flight requests, so a wedged agent can't make us allocate
/// without bound.
const MAX_PENDING: usize = 512;

#[derive(Debug, Error)]
pub enum ConnError {
    #[error("transport: {0}")]
    Transport(#[from] crate::framing::FrameError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent closed the connection")]
    Closed,
    #[error("agent returned an error: {code} {message}")]
    Remote { code: i64, message: String },
    #[error("too many in-flight requests ({MAX_PENDING})")]
    Overloaded,
    #[error("no handler registered for agent request {method}")]
    Unhandled { method: String },
    #[error("interrupted")]
    Cancelled,
    #[error("malformed payload: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// A response to one of our requests.
#[derive(Debug)]
pub enum CallOutcome {
    Result(Value),
    Error { code: i64, message: String },
    /// The agent never answered because the connection went away.
    Dropped,
}

/// Callbacks for the requests the agent makes *into* us, plus the
/// notification stream.
///
/// Implemented by whatever owns Mira's filesystem and terminal layers. The
/// connection knows nothing about either — it only knows how to route a
/// method name.
#[async_trait::async_trait]
pub trait AgentCallback: Send + Sync {
    /// Answer an agent-initiated request. Returning `Err` sends a JSON-RPC
    /// error back; the connection never drops a request on the floor,
    /// because an unanswered permission request blocks the agent forever.
    ///
    /// `id` is the agent's own request id and must be echoed in the reply —
    /// the connection does that. It is also the only handle a host has on an
    /// outstanding request, which is what lets `session/cancel` settle
    /// pending permissions later.
    async fn on_request(
        &self,
        id: &Value,
        method: &str,
        params: Value,
    ) -> Result<Value, ConnError>;

    /// An unsolicited notification, normally `session/update`.
    async fn on_notification(&self, method: &str, params: Value);

    /// Called once when the transport closes, so the owner can settle
    /// everything still pending.
    async fn on_closed(&self, _tail: String) {}
}

/// No-op callbacks, for tests and for a connection used purely to issue
/// requests.
pub struct NullCallbacks;

#[async_trait::async_trait]
impl AgentCallback for NullCallbacks {
    async fn on_request(
        &self,
        _id: &Value,
        method: &str,
        _p: Value,
    ) -> Result<Value, ConnError> {
        Err(ConnError::Unhandled {
            method: method.to_string(),
        })
    }
    async fn on_notification(&self, _m: &str, _p: Value) {}
}

struct Pending {
    tx: oneshot::Sender<CallOutcome>,
}

/// A live connection. Cloneable; all clones share one transport.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

struct Inner {
    stdin: Arc<Stdin>,
    pending: Mutex<HashMap<i64, Pending>>,
    next_id: std::sync::atomic::AtomicI64,
    /// Flipped when the transport dies, so waiters stop hanging. Held here
    /// to keep the channel alive for the life of the connection — dropping
    /// it would make every `dead_rx.changed()` await fail immediately.
    #[allow(dead_code)]
    dead: tokio::sync::watch::Sender<bool>,
    dead_rx: tokio::sync::watch::Receiver<bool>,
    /// Ring of recent stderr, for the "why did it die" path.
    stderr: Arc<Mutex<StderrTail>>,
}

impl Connection {
    /// Wire a connection to an already-split transport and start pumping.
    pub fn spawn_transport<R, W>(
        read: R,
        write: W,
        callbacks: Arc<dyn AgentCallback>,
    ) -> Connection
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let stdin = Arc::new(Stdin::new(Box::new(write)));
        let (dead, dead_rx) = tokio::sync::watch::channel(false);
        let dead_for_reader = dead.clone();
        let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();

        let conn = Connection {
            inner: Arc::new(Inner {
                stdin,
                pending: Mutex::new(HashMap::new()),
                next_id: std::sync::atomic::AtomicI64::new(1),
                dead,
                dead_rx: dead_rx.clone(),
                stderr: Arc::new(Mutex::new(StderrTail::default())),
            }),
        };

        // Reader: stdout is protocol, so nothing else may be read from it.
        // It owns the dead-flag flip and the on_closed callback; the
        // dispatcher below settles whatever was still in flight.
        let reader_stderr = conn.inner.stderr.clone();
        let reader_dead = dead_for_reader;
        let reader_cb = callbacks.clone();
        tokio::spawn(async move {
            let res = pump_stdout(tokio::io::BufReader::new(read), |frame| {
                let _ = tx.send(frame);
            })
            .await;
            if let Err(e) = res {
                tracing::warn!(error = %e, "acp: stdout pump ended with an error");
            }
            let tail = reader_stderr.lock().await.as_str();
            let _ = reader_dead.send(true);
            // Dropping the sender closes the channel, which ends the
            // dispatcher's loop.
            drop(tx);
            reader_cb.on_closed(tail).await;
        });

        // Dispatcher: the only place that decides "response to us" vs
        // "call into us" vs "notification".
        let dispatch_conn = conn.clone();
        let dispatch_cb = callbacks.clone();
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                dispatch_conn.dispatch(frame, dispatch_cb.clone()).await;
            }
            // Channel closed: fail everything still waiting so no caller
            // is left hanging on a dead agent.
            dispatch_conn.fail_all_pending().await;
        });

        conn
    }

    async fn dispatch(self: &Connection, frame: Inbound, cb: Arc<dyn AgentCallback>) {
        match frame {
            Inbound::Response { id, result } => {
                self.settle(&id, CallOutcome::Result(result)).await;
            }
            Inbound::Error {
                id,
                code,
                message,
            } => {
                self.settle(&id, CallOutcome::Error { code, message }).await;
            }
            Inbound::Request {
                id,
                method,
                params,
            } => {
                // Unknown id + method => the agent is calling us. Always
                // answer, even on error: an unanswered request wedges the
                // agent, and a permission request that never gets a reply
                // blocks its turn forever.
                let result = cb.on_request(&id, &method, params).await;
                let frame = match result {
                    Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
                    Err(e) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": {"code": -32603, "message": e.to_string()}
                    }),
                };
                if let Ok(f) = encode_frame(&frame) {
                    let _ = self.inner.stdin.write_frame(&f).await;
                }
            }
            Inbound::Notification { method, params } => {
                cb.on_notification(&method, params).await;
            }
        }
    }

    async fn settle(&self, id: &Value, outcome: CallOutcome) {
        // Only integer ids are ever issued by us, so anything else arriving
        // with a result is the agent echoing something and is ignored.
        let Some(n) = id.as_i64() else { return };
        let waiter = self.inner.pending.lock().await.remove(&n);
        if let Some(p) = waiter {
            let _ = p.tx.send(outcome);
        } else {
            tracing::debug!(id = n, "acp: response for an unknown request id");
        }
    }

    async fn fail_all_pending(&self) {
        let mut pending = self.inner.pending.lock().await;
        for (_, p) in pending.drain() {
            let _ = p.tx.send(CallOutcome::Dropped);
        }
    }

    /// True once the transport is gone.
    pub fn is_closed(&self) -> bool {
        *self.inner.dead_rx.borrow()
    }

    /// Recent stderr, for diagnostics.
    pub async fn stderr_tail(&self) -> String {
        self.inner.stderr.lock().await.as_str()
    }

    /// Attach a stderr reader so [`Self::stderr_tail`] has something in it.
    ///
    /// Deliberately **not** `async`: it spawns a task and returns, and an
    /// `async` version invited a call whose future was never awaited — which
    /// silently skipped the whole body and left the pipe undrained, letting a
    /// chatty agent block on a full stderr buffer.
    pub fn attach_stderr<R: AsyncRead + Unpin + Send + 'static>(&self, mut r: R) {
        let sink = self.inner.stderr.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match tokio::io::AsyncReadExt::read(&mut r, &mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => sink.lock().await.push(&buf[..n]),
                }
            }
        });
    }

    /// Issue a request and wait for its response.
    pub async fn request<M: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<M, ConnError> {
        match self.request_raw(method, params).await? {
            CallOutcome::Result(v) => Ok(serde_json::from_value(v)?),
            CallOutcome::Error { code, message } => Err(ConnError::Remote { code, message }),
            CallOutcome::Dropped => Err(ConnError::Closed),
        }
    }

    /// Like [`Self::request`] but hands back the raw outcome, so a caller
    /// that cares whether the agent answered before dying can tell.
    pub async fn request_raw(
        &self,
        method: &str,
        params: Value,
    ) -> Result<CallOutcome, ConnError> {
        if self.is_closed() {
            return Err(ConnError::Closed);
        }
        let id = self
            .inner
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            if pending.len() >= MAX_PENDING {
                return Err(ConnError::Overloaded);
            }
            pending.insert(id, Pending { tx });
        }

        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let encoded = match encode_frame(&frame) {
            Ok(f) => f,
            Err(e) => {
                self.inner.pending.lock().await.remove(&id);
                return Err(e.into());
            }
        };
        if let Err(e) = self.inner.stdin.write_frame(&encoded).await {
            self.inner.pending.lock().await.remove(&id);
            return Err(e.into());
        }

        // Stop waiting if the transport dies first, rather than hanging
        // forever on a receiver that will never fire.
        let mut closed = self.inner.dead_rx.clone();
        tokio::select! {
            got = rx => match got {
                Ok(outcome) => Ok(outcome),
                // Sender dropped without settling.
                Err(_) => Err(ConnError::Closed),
            },
            _ = closed.changed() => {
                self.inner.pending.lock().await.remove(&id);
                Err(ConnError::Closed)
            }
        }
    }

    /// Fire-and-forget notification (no id, no response expected).
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), ConnError> {
        let frame = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let encoded = encode_frame(&frame)?;
        self.inner.stdin.write_frame(&encoded).await?;
        Ok(())
    }

    /// Respond to an agent request we received the id of.
    pub async fn respond_ok(&self, id: &Value, result: Value) -> Result<(), ConnError> {
        let frame = json!({"jsonrpc": "2.0", "id": id, "result": result});
        let encoded = encode_frame(&frame)?;
        self.inner.stdin.write_frame(&encoded).await?;
        Ok(())
    }

    /// Respond with a JSON-RPC error.
    pub async fn respond_err(
        &self,
        id: &Value,
        code: i64,
        message: &str,
    ) -> Result<(), ConnError> {
        let frame = json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": message}
        });
        let encoded = encode_frame(&frame)?;
        self.inner.stdin.write_frame(&encoded).await?;
        Ok(())
    }
}

/// Build the params object for a request from key/value pairs.
pub fn params(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Helper for building typed params.
pub fn p<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    #[derive(Default)]
    struct Recorder {
        requests: StdMutex<Vec<(String, Value)>>,
        notifications: StdMutex<Vec<(String, Value)>>,
        closed_tail: StdMutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl AgentCallback for Recorder {
        async fn on_request(
            &self,
            _id: &Value,
            method: &str,
            params: Value,
        ) -> Result<Value, ConnError> {
            self.requests
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            Ok(json!({"handled": method}))
        }
        async fn on_notification(&self, method: &str, params: Value) {
            self.notifications
                .lock()
                .unwrap()
                .push((method.to_string(), params));
        }
        async fn on_closed(&self, tail: String) {
            *self.closed_tail.lock().unwrap() = Some(tail);
        }
    }

    /// A connection whose "agent" is a script we drive from the test.
    struct Harness {
        conn: Connection,
        agent_out: tokio::io::DuplexStream,
        /// One long-lived buffered reader. Constructing a `BufReader` per
        /// read would let it over-read and then discard the remainder,
        /// silently swallowing later frames.
        agent_in: tokio::io::BufReader<tokio::io::DuplexStream>,
        rec: Arc<Recorder>,
    }

    fn harness() -> Harness {
        let (to_agent, agent_in) = tokio::io::duplex(64 * 1024);
        let (agent_out, from_agent) = tokio::io::duplex(64 * 1024);
        let rec = Arc::new(Recorder::default());
        let conn = Connection::spawn_transport(
            from_agent,
            to_agent,
            rec.clone() as Arc<dyn AgentCallback>,
        );
        Harness {
            conn,
            agent_out,
            agent_in: tokio::io::BufReader::new(agent_in),
            rec,
        }
    }

    /// Read one line the agent is meant to have received.
    async fn read_agent_line(h: &mut Harness) -> Value {
        let mut line = String::new();
        h.agent_in.read_line(&mut line).await.expect("read");
        serde_json::from_str(line.trim()).expect("agent got non-JSON")
    }

    async fn agent_say(h: &mut Harness, v: Value) {
        let mut s = serde_json::to_string(&v).unwrap();
        s.push('\n');
        h.agent_out.write_all(s.as_bytes()).await.unwrap();
        h.agent_out.flush().await.unwrap();
    }

    #[tokio::test]
    async fn a_request_gets_its_own_response() {
        let mut h = harness();
        let waiter = tokio::spawn({
            let conn = h.conn.clone();
            async move { conn.request::<Value>("initialize", json!({"protocolVersion": 1})).await }
        });

        let seen = read_agent_line(&mut h).await;
        assert_eq!(seen["method"], "initialize");
        let id = seen["id"].clone();

        // The agent answers with a *different* id first. That must not be
        // mistaken for our answer.
        agent_say(&mut h, json!({"jsonrpc":"2.0","id": 9999, "result": {"wrong": true}})).await;
        agent_say(&mut h, json!({"jsonrpc":"2.0","id": id, "result": {"ok": true}})).await;

        let got: Value = waiter.await.unwrap().unwrap();
        assert_eq!(got, json!({"ok": true}));
    }

    #[tokio::test]
    async fn an_agent_request_using_our_own_id_number_is_still_dispatched_as_a_request() {
        // The sharp edge: shared id space. Our request 1 is in flight and
        // the agent issues its own request 1. If this were treated as a
        // response, our request would resolve with the wrong value.
        let mut h = harness();
        let waiter = tokio::spawn({
            let conn = h.conn.clone();
            async move { conn.request::<Value>("session/new", json!({})).await }
        });

        let ours = read_agent_line(&mut h).await;
        let our_id = ours["id"].clone();
        assert_eq!(our_id, 1);

        // Agent calls us with the *same* id.
        agent_say(
            &mut h,
            json!({
                "jsonrpc":"2.0","id": our_id, "method":"fs/read_text_file",
                "params":{"path":"/x"}
            }),
        )
        .await;

        // The agent sees our answer to its request.
        let reply = read_agent_line(&mut h).await;
        assert_eq!(reply["id"], our_id);
        assert_eq!(reply["result"]["handled"], "fs/read_text_file");

        // And our own request still resolves from its own response.
        agent_say(&mut h, json!({"jsonrpc":"2.0","id": our_id, "result": {"ours": true}})).await;
        let got: Value = waiter.await.unwrap().unwrap();
        assert_eq!(got, json!({"ours": true}));
    }

    #[tokio::test]
    async fn a_remote_error_becomes_a_conn_error_not_a_panic() {
        let mut h = harness();
        let waiter = tokio::spawn({
            let conn = h.conn.clone();
            async move { conn.request::<Value>("authenticate", json!({})).await }
        });
        let seen = read_agent_line(&mut h).await;
        agent_say(
            &mut h,
            json!({
                "jsonrpc":"2.0","id": seen["id"],
                "error":{"code":-32000,"message":"Authentication required"}
            }),
        )
        .await;
        let err = waiter.await.unwrap().unwrap_err();
        assert!(matches!(err, ConnError::Remote { code: -32000, .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn notifications_reach_the_callback_without_an_id() {
        let mut h = harness();
        agent_say(
            &mut h,
            json!({
                "jsonrpc":"2.0","method":"session/update",
                "params":{"sessionId":"s","update":{"sessionUpdate":"plan","entries":[]}}
            }),
        )
        .await;
        // Give the dispatcher a moment.
        for _ in 0..50 {
            if !h.rec.notifications.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let notes = h.rec.notifications.lock().unwrap().clone();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "session/update");
    }

    #[tokio::test]
    async fn every_agent_request_gets_an_answer_even_when_the_handler_fails() {
        // An unanswered permission request wedges the agent's turn forever,
        // so the connection always replies — errors included.
        struct Failing;
        #[async_trait::async_trait]
        impl AgentCallback for Failing {
            async fn on_request(
                &self,
                _id: &Value,
                _m: &str,
                _p: Value,
            ) -> Result<Value, ConnError> {
                Err(ConnError::Unhandled { method: "boom".into() })
            }
            async fn on_notification(&self, _m: &str, _p: Value) {}
        }
        let (to_agent, mut agent_in) = tokio::io::duplex(64 * 1024);
        let (mut agent_out, from_agent) = tokio::io::duplex(64 * 1024);
        Connection::spawn_transport(
            from_agent,
            to_agent,
            Arc::new(Failing) as Arc<dyn AgentCallback>,
        );
        let s = serde_json::to_string(&json!({
            "jsonrpc":"2.0","id": 7, "method":"session/request_permission", "params":{}
        }))
        .unwrap()
            + "\n";
        agent_out.write_all(s.as_bytes()).await.unwrap();
        agent_out.flush().await.unwrap();

        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::io::BufReader::new(&mut agent_in).read_line(&mut line),
        )
        .await
        .expect("agent was never answered")
        .unwrap();
        let reply: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(reply["id"], 7);
        assert!(reply.get("error").is_some(), "expected an error reply: {reply}");
    }

    #[tokio::test]
    async fn pending_requests_fail_when_the_agent_disappears() {
        let (to_agent, agent_in) = tokio::io::duplex(64 * 1024);
        let (agent_out, from_agent) = tokio::io::duplex(64 * 1024);
        let conn =
            Connection::spawn_transport(from_agent, to_agent, Arc::new(NullCallbacks) as Arc<dyn AgentCallback>);

        let waiter = tokio::spawn({
            let conn = conn.clone();
            async move { conn.request::<Value>("session/prompt", json!({})).await }
        });
        // Let the request reach the pipe, then drop the agent's end.
        drop(agent_in);
        drop(agent_out);

        let res = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("caller hung on a dead agent")
            .unwrap();
        assert!(res.is_err(), "expected an error, got {res:?}");
    }

    #[tokio::test]
    async fn a_request_after_close_fails_fast_rather_than_hanging() {
        let (to_agent, agent_in) = tokio::io::duplex(1024);
        let (agent_out, from_agent) = tokio::io::duplex(1024);
        let conn =
            Connection::spawn_transport(from_agent, to_agent, Arc::new(NullCallbacks) as Arc<dyn AgentCallback>);
        drop(agent_in);
        drop(agent_out);
        for _ in 0..50 {
            if conn.is_closed() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(conn.is_closed());
        let res = conn.request::<Value>("initialize", json!({})).await;
        assert!(matches!(res, Err(ConnError::Closed)), "got {res:?}");
    }

    #[tokio::test]
    async fn notifications_carry_no_id() {
        let mut h = harness();
        h.conn
            .notify("session/cancel", json!({"sessionId": "s"}))
            .await
            .unwrap();
        let seen = read_agent_line(&mut h).await;
        assert_eq!(seen["method"], "session/cancel");
        assert!(
            seen.get("id").is_none(),
            "a notification must not carry an id: {seen}"
        );
    }

    #[tokio::test]
    async fn concurrent_requests_get_distinct_ids() {
        let mut h = harness();
        let mut waiters = Vec::new();
        for _ in 0..4 {
            waiters.push(tokio::spawn({
                let conn = h.conn.clone();
                async move { conn.request::<Value>("ping", json!({})).await }
            }));
        }
        let mut ids = std::collections::HashSet::new();
        let mut by_id = HashMap::new();
        for _ in 0..4 {
            let v = read_agent_line(&mut h).await;
            ids.insert(v["id"].as_i64().unwrap());
            by_id.insert(v["id"].clone(), v);
        }
        assert_eq!(ids.len(), 4, "ids must not collide: {ids:?}");

        for (id, _) in by_id {
            agent_say(&mut h, json!({"jsonrpc":"2.0","id": id, "result": {"pong": true}})).await;
        }
        for w in waiters {
            let got: Value = w.await.unwrap().unwrap();
            assert_eq!(got, json!({"pong": true}));
        }
    }
}
