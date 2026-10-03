//! Browser control for the right-hand browser pane.
//!
//! `POST /api/browser/action` → body: one `BrowserAction`, verbatim.
//!                                 Returns `{ text, screenshot? }`.
//!
//! This is a thin HTTP face over `mira_browser::Browser`, which already
//! knows how to launch Chrome, speak CDP, and run every action the pane
//! needs (navigate, back/forward, reload, snapshot, screenshot, click,
//! type, key, scroll, evaluate, tab management). Reusing its action enum
//! means the pane and the agent's `browser` tool drive the *same* browser
//! with the same semantics instead of drifting apart.
//!
//! The `Browser` handle is cheap to construct and launches lazily on the
//! first action, so it lives on `AppState` unwrapped — no extra locking
//! here, and no Chrome process until someone actually asks for one.
//!
//! There is deliberately no "is it running?" endpoint: the crate keeps its
//! CDP session private, so the only honest answer the server can give is
//! the result of the last action. The pane renders that instead.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_browser::{normalize_url, BrowserAction, Outcome};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotOut {
    /// Base64 PNG, no data-URL prefix. The pane sets this as an <img> src.
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionOut {
    /// Human-readable result — extracted page text for `snapshot`, the
    /// loaded URL for `navigate`, and so on.
    pub text: String,
    /// Present for `screenshot`, and for actions whose result includes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ScreenshotOut>,
}

impl From<Outcome> for ActionOut {
    fn from(o: Outcome) -> Self {
        ActionOut {
            text: o.text,
            screenshot: o.screenshot.map(|s| ScreenshotOut {
                png_base64: s.png_base64,
                width: s.width,
                height: s.height,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ActionReq {
    /// The action object, parsed by the browser crate's own serde impls so
    /// the wire format can't drift from what the agent's tool accepts.
    action: Value,
}

pub async fn action(State(state): State<AppState>, Json(req): Json<ActionReq>) -> Response {
    let parsed = match BrowserAction::from_args(&req.action) {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    // Accept a bare host in the URL bar the same way the agent's tool does,
    // so "example.com" and "localhost:3000" both do the obvious thing.
    let action = match &parsed {
        BrowserAction::Navigate { url } => BrowserAction::Navigate {
            url: normalize_url(url),
        },
        other => other.clone(),
    };

    match state.browser.execute(&action).await {
        Ok(out) => Json(ActionOut::from(out)).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/* ------------------------------------------------------------------ */
/* Frame-embedding probe                                               */
/* ------------------------------------------------------------------ */

#[derive(Debug, Deserialize)]
pub struct EmbeddableQuery {
    url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddableOut {
    /// False when the site will refuse to load in an `<iframe>`, so the
    /// pane can skip straight to the agent browser instead of showing a
    /// blank frame and leaving the user to guess why.
    pub embeddable: bool,
    /// Human-readable explanation, shown verbatim in the pane.
    pub reason: String,
    /// The URL after redirects, which is what should actually be framed.
    pub final_url: String,
}

/// Ask the origin whether it will let us frame it.
///
/// A browser cannot answer this for us: reading `X-Frame-Options` requires
/// reading response headers, which is a cross-origin failure by definition.
/// So we ask the server we already talk to, which can do a plain request.
pub async fn embeddable(Query(q): Query<EmbeddableQuery>) -> Response {
    let target = normalize_url(&q.url);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        // We only read headers; a body would be wasted bandwidth and can be
        // enormous on some pages.
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    // Some origins only send framing headers on GET, so ask for that.
    let res = match client.get(&target).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let final_url = res.url().to_string();

    if let Some(xfo) = res
        .headers()
        .get("x-frame-options")
        .and_then(|v| v.to_str().ok())
    {
        // Every standard value blocks us: DENY outright, SAMEORIGIN only
        // permits a same-origin frame, and ALLOW-FROM is dead. The pane
        // always frames from a different origin, so any value means no.
        return Json(EmbeddableOut {
            embeddable: false,
            reason: format!(
                "This site sends X-Frame-Options: {xfo}, so it refuses to load in a frame."
            ),
            final_url,
        })
        .into_response();
    }

    if let Some(csp) = res
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(ancestors) = csp
            .split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix("frame-ancestors"))
        {
            // The pane is served from a random localhost port, so `self` and
            // any named host will not match us. Only a wildcard lets an
            // arbitrary origin frame the page.
            if !ancestors.split_whitespace().any(|a| a == "*") {
                return Json(EmbeddableOut {
                    embeddable: false,
                    reason: format!(
                        "This site's Content-Security-Policy sets frame-ancestors {ancestors}, so it refuses to load in a frame."
                    ),
                    final_url,
                })
                .into_response();
            }
        }
    }

    Json(EmbeddableOut {
        embeddable: true,
        reason: "This site can be embedded.".into(),
        final_url,
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Live view
// ---------------------------------------------------------------------------

/// `GET /api/browser/live` — the agent browser's current tab, streamed.
///
/// Server-sent events, one JSON [`mira_browser::LiveEvent`] each: screencast
/// frames, navigations, and `closed`. The latest frame is sent first, so a
/// pane opened mid-session shows the page at once. Opening the stream starts
/// the browser (or reconnects to the one already running).
pub async fn live(State(state): State<AppState>) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::StreamExt;

    let browser = state.browser.clone();
    let (rx, last) = browser.subscribe();
    {
        let browser = browser.clone();
        tokio::spawn(async move {
            if let Err(e) = browser.ensure_started().await {
                tracing::warn!(%e, "browser live view: could not start the browser");
            }
        });
    }
    let first = futures::stream::iter(last);
    let rest =
        tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
    let stream = first.chain(rest).map(|ev| {
        Ok::<_, std::convert::Infallible>(
            Event::default().data(serde_json::to_string(&ev).unwrap_or_default()),
        )
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

/// `POST /api/browser/input` — the watcher takes over: a click, scroll, key
/// or text, in the live view's CSS-pixel coordinates.
pub async fn input(
    State(state): State<AppState>,
    Json(input): Json<mira_browser::UserInput>,
) -> Response {
    match state.browser.input(&input).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// MCP: the same browser, for external agents
// ---------------------------------------------------------------------------

/// The secret in the MCP endpoint's path. Generated per process: the server
/// listens on loopback, but any page open in a local browser can reach
/// loopback too, and the browser tool acts in a logged-in profile — so the
/// endpoint must not be guessable.
pub fn mcp_token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
}

/// Who approves the actions an agent takes through Mira's tool server.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum McpGate {
    /// The agent asks before each tool call, and its asking already reaches
    /// Mira's approval card (Claude Code's permission gate).
    Agent,
    /// The agent may call tools without asking (OpenCode allows MCP tools
    /// by default), so Mira asks before anything that runs a command.
    Mira,
}

/// The URL of Mira's tool server for one chat: the browser the pane shows,
/// and background processes that land in that chat's Processes window.
pub fn agent_mcp_url(port: u16, session: &str, gate: McpGate) -> String {
    let gate = match gate {
        McpGate::Agent => "agent",
        McpGate::Mira => "mira",
    };
    format!(
        "http://127.0.0.1:{port}/mcp/{}?session={}&gate={gate}",
        mcp_token(),
        urlencoding_light(session),
    )
}

/// Session ids are `sess_<hex>`; anything else is escaped conservatively.
fn urlencoding_light(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32 & 0xff)
            }
        })
        .collect()
}

/// The MCP server entry an agent is launched with (`--mcp-config`), so
/// Claude Code drives Mira's browser — the one the pane shows — rather
/// than starting a browser of its own, and its background commands show
/// in Mira's Processes window.
pub fn agent_mcp_config(port: u16, session: &str) -> Value {
    serde_json::json!({
        "mcpServers": {
            "mira": {
                "type": "http",
                "url": agent_mcp_url(port, session, McpGate::Agent),
            }
        }
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct McpQuery {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    gate: Option<String>,
}

fn browser_tool_spec(state: &AppState) -> mira_ai::ToolSpec {
    use mira_tools::Tool as _;
    mira_tools::builtin::browser::BrowserTool::new(state.browser.clone()).spec()
}

/// `GET /mcp/:token` — no server-initiated stream is offered; the spec's
/// answer for that is 405.
pub async fn mcp_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

/// `POST /mcp/:token` — a minimal streamable-HTTP MCP server exposing one
/// tool, `browser`, with exactly the schema Mira's own model gets.
///
/// Calls arrive already approved: the agent's host (Mira) asked the user
/// through its permission gate before the agent was allowed to call.
pub async fn mcp(
    State(state): State<AppState>,
    axum::extract::Path(token): axum::extract::Path<String>,
    Query(q): Query<McpQuery>,
    Json(req): Json<Value>,
) -> Response {
    if token != mcp_token() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let reqs = match req {
        Value::Array(items) => items,
        one => vec![one],
    };
    let mut replies = Vec::new();
    for r in reqs {
        if let Some(reply) = mcp_one(&state, &q, r).await {
            replies.push(reply);
        }
    }
    match replies.len() {
        // Only notifications: acknowledged, nothing to say.
        0 => StatusCode::ACCEPTED.into_response(),
        1 => Json(replies.into_iter().next().unwrap()).into_response(),
        _ => Json(Value::Array(replies)).into_response(),
    }
}

/// The background-process tools, with exactly the specs Mira's own model
/// gets. They act on the chat's own process store, so what an agent starts
/// shows (and can be stopped) in that chat's Processes window.
fn background_tools() -> Vec<Box<dyn mira_tools::Tool>> {
    use mira_tools::builtin::background::{KillBackground, ReadOutput, RunBackground};
    vec![Box::new(RunBackground), Box::new(ReadOutput), Box::new(KillBackground)]
}

async fn mcp_one(state: &AppState, q: &McpQuery, req: Value) -> Option<Value> {
    use serde_json::json;
    let id = req.get("id").cloned()?; // a notification gets no reply
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let ok = |result: Value| json!({ "jsonrpc": "2.0", "id": id.clone(), "result": result });
    let err = |code: i64, message: String| json!({ "jsonrpc": "2.0", "id": id.clone(), "error": { "code": code, "message": message } });
    Some(match method {
        "initialize" => {
            let version = req["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2025-06-18");
            ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mira", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Mira's tools. `browser` is the browser the user watches live in \
                                 Mira's browser pane (a dedicated profile, never the user's own): use \
                                 it to open, click, type and screenshot pages. `run_background` starts \
                                 a long-running command (a dev server, a watcher) the user can see, \
                                 read and stop in Mira's Processes window; `read_output` and \
                                 `kill_background` read and stop it.",
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => {
            let mut specs = vec![browser_tool_spec(state)];
            if mcp_slot(state, q).await.is_some() {
                specs.extend(background_tools().iter().map(|t| t.spec()));
            }
            ok(json!({ "tools": specs.iter().map(|s| json!({
                "name": s.name,
                "description": s.description,
                "inputSchema": s.parameters,
            })).collect::<Vec<_>>() }))
        }
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            if name != "browser" {
                return Some(match call_background(state, q, name, &req["params"]["arguments"]).await {
                    Some(Ok((text, is_error))) => ok(json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": is_error,
                    })),
                    Some(Err(e)) => err(-32603, e),
                    None => err(-32602, format!("unknown tool `{name}`")),
                });
            }
            let args = req["params"]["arguments"].clone();
            let action = match BrowserAction::from_args(&args) {
                Ok(a) => a,
                Err(e) => {
                    return Some(ok(json!({
                        "content": [{ "type": "text", "text": e.to_string() }],
                        "isError": true,
                    })))
                }
            };
            match state.browser.execute(&action).await {
                Ok(out) => {
                    let mut content = vec![json!({ "type": "text", "text": out.text })];
                    if let Some(shot) = out.screenshot {
                        content.push(json!({
                            "type": "image",
                            "data": shot.png_base64,
                            "mimeType": "image/png",
                        }));
                    }
                    ok(json!({ "content": content, "isError": false }))
                }
                Err(e) => ok(json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                })),
            }
        }
        other => err(-32601, format!("method not found: {other}")),
    })
}

/// The chat a tool-server URL belongs to (`?session=`).
async fn mcp_slot(state: &AppState, q: &McpQuery) -> Option<std::sync::Arc<crate::slot::SessionSlot>> {
    state.slot_str(q.session.as_deref()?).await
}

/// Run one of the background-process tools for the URL's chat. `None` when
/// the tool isn't one of them (or there's no chat to run it in).
async fn call_background(
    state: &AppState,
    q: &McpQuery,
    name: &str,
    args: &Value,
) -> Option<Result<(String, bool), String>> {
    let tool = background_tools().into_iter().find(|t| t.spec().name == name)?;
    let slot = mcp_slot(state, q).await?;
    let call = mira_core::ToolCall {
        id: mira_core::ToolCallId::new(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name: name.to_string(),
            arguments: if args.is_null() { "{}".into() } else { args.to_string() },
        },
    };
    // An agent that doesn't ask before tool calls gets Mira's approval card
    // for anything that runs or stops a command (as Mira's own model does).
    if q.gate.as_deref() == Some("mira") && matches!(tool.action(), mira_tools::Action::Bash) {
        let allowed = slot
            .approver
            .approve(&call, mira_policy::Decision::Ask)
            .await;
        if !allowed {
            return Some(Ok((
                "The user denied this in Mira. Don't retry it as-is.".into(),
                true,
            )));
        }
    }
    let ctx = slot
        .make_tool_ctx(state.sandbox.clone())
        .await
        .with_bg_processes(slot.bg_processes.clone())
        .with_bg_progress(std::sync::Arc::new(crate::slot::SessionProgress {
            tx: slot.events_tx.clone(),
        }));
    Some(match tool.invoke(&call, &ctx).await {
        Ok(r) => Ok((r.content, r.is_error)),
        Err(e) => Ok((e.to_string(), true)),
    })
}
