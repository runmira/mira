//! `mira mcp-bridge <url>` — Mira's tool server as a stdio MCP server.
//!
//! Some agents only take MCP servers they launch themselves and talk to over
//! stdin/stdout. For those, Mira hands over this command instead of its HTTP
//! URL: each JSON-RPC line read from stdin is POSTed to the URL, and the
//! reply — a JSON body, or the `data:` events of an SSE stream — goes back to
//! stdout one message per line. A 202/204 (a notification's acknowledgement)
//! writes nothing. The `mcp-session-id` and the negotiated protocol version
//! are replayed on every later request, as streamable HTTP expects.
//!
//! The bearer token comes from `MIRA_MCP_TOKEN` (set by Mira in the agent's
//! MCP server entry), never argv, which process listings show.
//!
//! stdout carries protocol only; anything else goes to stderr.

use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[derive(Args, Debug, Clone)]
pub struct McpBridgeArgs {
    /// Mira's tool-server URL (`http://127.0.0.1:<port>/mcp/<token>?session=…`).
    pub url: String,
}

#[derive(Default)]
struct Negotiated {
    session_id: Option<String>,
    protocol_version: Option<String>,
}

pub async fn run(args: McpBridgeArgs) -> Result<()> {
    let client = reqwest::Client::builder().build()?;
    let auth = std::env::var(mira_acp::session::MIRA_MCP_TOKEN_ENV)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .map(|t| format!("Bearer {}", t.trim()));
    let out = Arc::new(Mutex::new(tokio::io::stdout()));
    let negotiated = Arc::new(Mutex::new(Negotiated::default()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tasks = tokio::task::JoinSet::new();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_line(&out, &rpc_error(&Value::Null, -32700, &format!("parse error: {e}"))).await;
                continue;
            }
        };
        let is_initialize = msg.get("method").and_then(Value::as_str) == Some("initialize");
        // `initialize` must settle the session before anything else goes out;
        // everything after runs concurrently, so one slow tool call (a page
        // loading) doesn't hold up the rest.
        let job = relay(
            client.clone(),
            args.url.clone(),
            auth.clone(),
            msg,
            negotiated.clone(),
            out.clone(),
        );
        if is_initialize {
            job.await;
        } else {
            tasks.spawn(job);
        }
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn relay(
    client: reqwest::Client,
    url: String,
    auth: Option<String>,
    msg: Value,
    negotiated: Arc<Mutex<Negotiated>>,
    out: Arc<Mutex<tokio::io::Stdout>>,
) {
    let id = msg.get("id").cloned();
    let mut req = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(msg.to_string());
    if let Some(a) = &auth {
        req = req.header("authorization", a);
    }
    {
        let n = negotiated.lock().await;
        if let Some(s) = &n.session_id {
            req = req.header("mcp-session-id", s);
        }
        if let Some(v) = &n.protocol_version {
            req = req.header("mcp-protocol-version", v);
        }
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            if let Some(id) = id {
                write_line(&out, &rpc_error(&id, -32603, &format!("Mira isn't reachable: {e}"))).await;
            }
            return;
        }
    };
    if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        negotiated.lock().await.session_id = Some(s.to_string());
    }
    let status = resp.status();
    if status.as_u16() == 202 || status.as_u16() == 204 {
        return;
    }
    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/event-stream"));
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            if let Some(id) = id {
                write_line(&out, &rpc_error(&id, -32603, &format!("reading Mira's reply: {e}"))).await;
            }
            return;
        }
    };
    if !status.is_success() && !is_sse && serde_json::from_str::<Value>(&body).is_err() {
        if let Some(id) = id {
            let msg = format!("Mira answered HTTP {}: {}", status.as_u16(), body.trim());
            write_line(&out, &rpc_error(&id, -32603, &msg)).await;
        }
        return;
    }
    let replies: Vec<Value> = if is_sse {
        sse_messages(&body)
    } else {
        match serde_json::from_str::<Value>(&body) {
            Ok(Value::Array(items)) => items,
            Ok(v) => vec![v],
            Err(_) => Vec::new(),
        }
    };
    for r in replies {
        if let Some(v) = r
            .get("result")
            .and_then(|res| res.get("protocolVersion"))
            .and_then(Value::as_str)
        {
            negotiated.lock().await.protocol_version = Some(v.to_string());
        }
        write_line(&out, &r).await;
    }
}

/// The JSON messages in an SSE body: each event's `data:` lines, joined.
fn sse_messages(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    for line in body.lines().chain(std::iter::once("")) {
        if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.trim_start());
        } else if line.trim().is_empty() && !data.is_empty() {
            if let Ok(v) = serde_json::from_str(&data) {
                out.push(v);
            }
            data.clear();
        }
    }
    out
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn write_line(out: &Arc<Mutex<tokio::io::Stdout>>, v: &Value) {
    let mut o = out.lock().await;
    let _ = o.write_all(format!("{v}\n").as_bytes()).await;
    let _ = o.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_messages_out_of_an_sse_stream() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":{}}\n\n";
        let msgs = sse_messages(body);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["id"], 1);
        assert_eq!(msgs[1]["id"], 2);
    }

    #[test]
    fn errors_answer_the_request_they_belong_to() {
        let e = rpc_error(&json!(7), -32603, "down");
        assert_eq!(e["id"], 7);
        assert_eq!(e["error"]["message"], "down");
    }
}
