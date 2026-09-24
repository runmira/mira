//! Minimal Chrome DevTools Protocol client.
//!
//! One WebSocket to the browser endpoint, flat sessions
//! (`Target.attachToTarget { flatten: true }`) for pages. Commands are
//! matched to replies by `id`; events are ignored — the driver polls page
//! state instead, which is simpler and robust enough for agent pacing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::BrowserError;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, BrowserError>>>>>;
type Sink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// Per-command ceiling. Navigation waits are layered on top by polling,
/// so a single command should never legitimately take this long.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Cdp {
    sink: Mutex<Sink>,
    pending: Pending,
    next_id: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for Cdp {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Cdp {
    pub async fn connect(ws_url: &str) -> Result<Self, BrowserError> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| BrowserError::Protocol(format!("connect {ws_url}: {e}")))?;
        let (sink, mut stream) = ws.split();
        let pending: Pending = Arc::default();
        let reader_pending = pending.clone();
        let reader = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let text = match msg {
                    Ok(Message::Text(t)) => t,
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(id) = v.get("id").and_then(Value::as_u64) else {
                    continue; // an event
                };
                if let Some(tx) = reader_pending.lock().await.remove(&id) {
                    let res = match v.get("error") {
                        Some(err) => Err(BrowserError::Protocol(
                            err.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown CDP error")
                                .to_owned(),
                        )),
                        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = tx.send(res);
                }
            }
            // Connection gone: fail everything still waiting.
            for (_, tx) in reader_pending.lock().await.drain() {
                let _ = tx.send(Err(BrowserError::Closed));
            }
        });
        Ok(Self {
            sink: Mutex::new(sink),
            pending,
            next_id: AtomicU64::new(1),
            reader,
        })
    }

    /// Send `method` with `params`, on the page `session` when given.
    pub async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, BrowserError> {
        if self.reader.is_finished() {
            return Err(BrowserError::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let sent = self
            .sink
            .lock()
            .await
            .send(Message::Text(msg.to_string()))
            .await;
        if let Err(e) = sent {
            self.pending.lock().await.remove(&id);
            return Err(BrowserError::Protocol(format!("send {method}: {e}")));
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(BrowserError::Closed),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(BrowserError::Protocol(format!("{method} timed out")))
            }
        }
    }
}
