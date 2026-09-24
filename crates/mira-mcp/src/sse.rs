//! Client for the older HTTP+SSE MCP transport (protocol 2024-11-05),
//! still used by many hosted servers and by `type: "sse"` entries in
//! Claude Code configs.
//!
//! The client opens a long-lived `GET` event stream. The server's first
//! event, `endpoint`, names the URL to `POST` messages to; replies and
//! server-initiated messages come back as `message` events on the stream.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::RoleClient;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Supplies a bearer token per request (refreshing as needed), or `None`.
pub type TokenFn = Arc<dyn Fn() -> BoxFuture<'static, Option<String>> + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum SseConnectError {
    /// 401 on the event stream; carries the `WWW-Authenticate` header.
    #[error("authorization required")]
    AuthRequired(String),
    #[error("{0}")]
    Failed(String),
}

/// The transport handed to rmcp: a sink for outgoing messages and a
/// stream of incoming ones. Dropping it stops the background tasks.
pub struct SseTransport {
    pub sink: mpsc::Sender<TxJsonRpcMessage<RoleClient>>,
    pub stream: mpsc::Receiver<RxJsonRpcMessage<RoleClient>>,
    _stop: tokio_util::sync::DropGuard,
}

pub async fn connect(
    url: &str,
    headers: &BTreeMap<String, String>,
    token: Option<TokenFn>,
    timeout: Duration,
) -> Result<SseTransport, SseConnectError> {
    let base = url::Url::parse(url).map_err(|e| SseConnectError::Failed(format!("{url}: {e}")))?;
    let mut header_map = HeaderMap::new();
    for (k, v) in headers {
        let name = HeaderName::try_from(k.as_str())
            .map_err(|e| SseConnectError::Failed(format!("header `{k}`: {e}")))?;
        let value = HeaderValue::try_from(v.as_str())
            .map_err(|e| SseConnectError::Failed(format!("header `{k}`: {e}")))?;
        header_map.insert(name, value);
    }
    let client = reqwest::Client::new();

    let mut get = client
        .get(base.clone())
        .headers(header_map.clone())
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if let Some(t) = &token {
        if let Some(bearer) = t().await {
            get = get.bearer_auth(bearer);
        }
    }
    let resp = tokio::time::timeout(timeout, get.send())
        .await
        .map_err(|_| SseConnectError::Failed(format!("no response from {url}")))?
        .map_err(|e| SseConnectError::Failed(e.to_string()))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let challenge = resp
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        return Err(SseConnectError::AuthRequired(challenge));
    }
    if !resp.status().is_success() {
        return Err(SseConnectError::Failed(format!(
            "{url} answered {}",
            resp.status()
        )));
    }

    let mut events = resp.bytes_stream().eventsource();
    // The first `endpoint` event says where to POST.
    let endpoint = tokio::time::timeout(timeout, async {
        while let Some(ev) = events.next().await {
            match ev {
                Ok(ev) if ev.event == "endpoint" => return Some(ev.data),
                Ok(_) => continue,
                Err(e) => {
                    warn!(%e, "mcp sse: bad event before endpoint");
                    return None;
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| SseConnectError::Failed(format!("{url} sent no `endpoint` event")))?;
    let post_url = base
        .join(endpoint.trim())
        .map_err(|e| SseConnectError::Failed(format!("endpoint `{endpoint}`: {e}")))?;

    let stop = CancellationToken::new();
    let (in_tx, in_rx) = mpsc::channel::<RxJsonRpcMessage<RoleClient>>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<TxJsonRpcMessage<RoleClient>>(64);

    // Reader: SSE `message` events → incoming messages. Ends (closing the
    // transport) when the stream does.
    let reader_stop = stop.clone();
    let mut reader_tx = in_tx.clone();
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                _ = reader_stop.cancelled() => break,
                ev = events.next() => ev,
            };
            let Some(ev) = ev else { break };
            let ev = match ev {
                Ok(ev) => ev,
                Err(e) => {
                    warn!(%e, "mcp sse: stream error");
                    break;
                }
            };
            if !ev.event.is_empty() && ev.event != "message" {
                continue;
            }
            match serde_json::from_str::<RxJsonRpcMessage<RoleClient>>(&ev.data) {
                Ok(msg) => {
                    if reader_tx.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(e) => debug!(%e, "mcp sse: ignoring unparsable message"),
            }
        }
    });

    // Writer: outgoing messages → POST to the endpoint.
    let writer_stop = stop.clone();
    tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                _ = writer_stop.cancelled() => break,
                msg = out_rx.next() => msg,
            };
            let Some(msg) = msg else { break };
            let mut post = client
                .post(post_url.clone())
                .headers(header_map.clone())
                .json(&msg);
            if let Some(t) = &token {
                if let Some(bearer) = t().await {
                    post = post.bearer_auth(bearer);
                }
            }
            match post.send().await {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => warn!(status = %r.status(), "mcp sse: POST rejected"),
                Err(e) => warn!(%e, "mcp sse: POST failed"),
            }
        }
    });
    drop(in_tx);

    Ok(SseTransport {
        sink: out_tx,
        stream: in_rx,
        _stop: stop.drop_guard(),
    })
}

impl rmcp::transport::Transport<RoleClient> for SseTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let mut sink = self.sink.clone();
        async move {
            sink.send(item)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))
        }
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.stream.next()
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.sink.close_channel();
        self.stream.close();
        std::future::ready(Ok(()))
    }
}
