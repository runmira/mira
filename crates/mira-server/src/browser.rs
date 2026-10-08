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

/// What a tool-server token grants: one chat's tools, under one gate.
#[derive(Clone, Debug, PartialEq)]
pub struct McpGrant {
    pub session: String,
    pub gate: McpGate,
}

/// A token unused this long stops working (an agent left idle overnight
/// gets a fresh one when it restarts).
const GRANT_IDLE: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

struct GrantEntry {
    grant: McpGrant,
    last_used: std::time::Instant,
}

/// Live tokens. Each agent session gets its own, sent as a bearer header
/// (never in the URL, which ends up in agent configs and logs): the server
/// listens on loopback, but any page in a local browser can reach loopback
/// too, and the browser tool acts in a logged-in profile.
fn grants() -> &'static std::sync::Mutex<std::collections::HashMap<String, GrantEntry>> {
    static G: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, GrantEntry>>> =
        std::sync::OnceLock::new();
    G.get_or_init(Default::default)
}

/// The token for agents in `session` under `gate`: the chat's existing one
/// while it's live, otherwise a new one. Stable across agent restarts on
/// purpose — an agent (re)started in the chat, or one still holding the
/// token from before, keeps working; revoking on every stop handed agents
/// tokens that were dead on arrival (401). Tokens end when the chat is
/// deleted or after `GRANT_IDLE` unused.
pub fn issue_mcp_grant(session: &str, gate: McpGate) -> String {
    if let Ok(mut g) = grants().lock() {
        g.retain(|_, e| e.last_used.elapsed() < GRANT_IDLE);
        if let Some((token, e)) = g
            .iter_mut()
            .find(|(_, e)| e.grant.session == session && e.grant.gate == gate)
        {
            e.last_used = std::time::Instant::now();
            return token.clone();
        }
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    if let Ok(mut g) = grants().lock() {
        g.insert(
            token.clone(),
            GrantEntry {
                grant: McpGrant {
                    session: session.to_string(),
                    gate,
                },
                last_used: std::time::Instant::now(),
            },
        );
    }
    token
}

/// Revoke a chat's tokens (the chat was deleted).
pub fn revoke_mcp_grants(session: &str) {
    if let Ok(mut g) = grants().lock() {
        g.retain(|_, e| e.grant.session != session);
    }
}

/// The grant behind a token, if it's live; using it keeps it alive.
fn grant_for(token: &str) -> Option<McpGrant> {
    let mut g = grants().lock().ok()?;
    g.retain(|_, e| e.last_used.elapsed() < GRANT_IDLE);
    let e = g.get_mut(token)?;
    e.last_used = std::time::Instant::now();
    Some(e.grant.clone())
}

/// The tool server's address.
pub fn agent_mcp_endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// Mira's tool server for one agent session: the browser the pane shows,
/// and background processes that land in that chat's Processes window.
pub fn agent_mcp(port: u16, session: &str, gate: McpGate) -> mira_acp::session::MiraMcp {
    mira_acp::session::MiraMcp {
        url: agent_mcp_endpoint(port),
        token: issue_mcp_grant(session, gate),
    }
}

/// The MCP server entry an agent is launched with (`--mcp-config`), so
/// Claude Code drives Mira's browser — the one the pane shows — rather
/// than starting a browser of its own, and its background commands show
/// in Mira's Processes window.
pub fn agent_mcp_config(port: u16, session: &str) -> Value {
    let m = agent_mcp(port, session, McpGate::Agent);
    serde_json::json!({
        "mcpServers": {
            "mira": {
                "type": "http",
                "url": m.url,
                "headers": { "Authorization": m.authorization() },
            }
        }
    })
}

/// `GET /mcp` — no server-initiated stream is offered; the spec's answer
/// for that is 405.
pub async fn mcp_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

/// `POST /mcp` — Mira's tool server for external agents (streamable HTTP,
/// JSON replies): single-purpose browser tools on the pane's browser, and
/// background processes in the chat's Processes window.
///
/// `Authorization: Bearer <token>` decides everything: which chat, and who
/// approves. No token, or a revoked or idle one, is a 401.
pub async fn mcp(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<Value>,
) -> Response {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    let Some(q) = grant_for(token) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let profile = ToolProfile::from_headers(&headers);
    let reqs = match req {
        Value::Array(items) => items,
        one => vec![one],
    };
    let mut replies = Vec::new();
    for r in reqs {
        if let Some(reply) = mcp_one(&state, &q, profile, r).await {
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

/// Mira tools that only look — they never change a page, a file or a
/// process — so agents that gate each tool call (Claude Code) can run them
/// without asking.
pub const READ_ONLY_AGENT_TOOLS: &[&str] = &[
    "browser_snapshot",
    "browser_screenshot",
    "browser_tabs",
    "browser_wait",
    "read_output",
];

/// Claude Code's `--allowedTools` value for Mira's read-only tools (its MCP
/// tools are named `mcp__<server>__<tool>`).
pub fn claude_allowed_tools() -> String {
    READ_ONLY_AGENT_TOOLS
        .iter()
        .map(|t| format!("mcp__mira__{t}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One single-purpose browser tool for agents: its name, the browser action
/// it runs, what it's for, and its own arguments (JSON Schema properties).
struct BrowserVerb {
    name: &'static str,
    action: &'static str,
    description: &'static str,
    properties: fn() -> Value,
    required: &'static [&'static str],
}

/// The agent-facing browser: one tool per verb, the way models handle
/// browsers best (one multipurpose tool with an `action` switch is what they
/// fumble). Each maps onto an action of the same browser Mira's own model
/// and the browser pane use.
const BROWSER_VERBS: &[BrowserVerb] = &[
    BrowserVerb {
        name: "browser_open",
        action: "navigate",
        description: "Open a URL in Mira's browser — the one the user watches live in Mira's \
                      browser pane. Starts the browser if needed. Then read the page with \
                      browser_snapshot.",
        properties: || serde_json::json!({ "url": { "type": "string", "description": "Full URL, e.g. http://localhost:5173" } }),
        required: &["url"],
    },
    BrowserVerb {
        name: "browser_snapshot",
        action: "snapshot",
        description: "Read the current page: title, URL, readable text, and its interactive \
                      elements, each with a `ref` to pass to browser_click / browser_type. \
                      Prefer this to screenshots for reading.",
        properties: || serde_json::json!({}),
        required: &[],
    },
    BrowserVerb {
        name: "browser_screenshot",
        action: "screenshot",
        description: "A PNG screenshot of the visible page, for checking layout and visuals.",
        properties: || serde_json::json!({}),
        required: &[],
    },
    BrowserVerb {
        name: "browser_click",
        action: "click",
        description: "Click an element: by `ref` from browser_snapshot (best), a CSS \
                      `selector`, or a viewport `coordinate`.",
        properties: || {
            serde_json::json!({
                "ref": { "type": "string", "description": "Element ref from browser_snapshot" },
                "selector": { "type": "string" },
                "coordinate": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                "double": { "type": "boolean", "description": "Double-click" }
            })
        },
        required: &[],
    },
    BrowserVerb {
        name: "browser_type",
        action: "type",
        description: "Type text into a field (by `ref` or `selector`; otherwise the focused \
                      element). `clear` empties it first; `submit` presses Enter after.",
        properties: || {
            serde_json::json!({
                "text": { "type": "string" },
                "ref": { "type": "string" },
                "selector": { "type": "string" },
                "clear": { "type": "boolean" },
                "submit": { "type": "boolean" }
            })
        },
        required: &["text"],
    },
    BrowserVerb {
        name: "browser_press",
        action: "key",
        description: "Press a key or chord: Enter, Escape, Tab, ArrowDown, Meta+A…",
        properties: || serde_json::json!({ "key": { "type": "string" } }),
        required: &["key"],
    },
    BrowserVerb {
        name: "browser_scroll",
        action: "scroll",
        description: "Scroll the page (or the element with `ref`): `direction` up, down, left \
                      or right; `amount` in screen-steps.",
        properties: || {
            serde_json::json!({
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                "amount": { "type": "integer", "minimum": 1 },
                "ref": { "type": "string" }
            })
        },
        required: &[],
    },
    BrowserVerb {
        name: "browser_evaluate",
        action: "evaluate",
        description: "Run a JavaScript expression in the page and get its JSON value back.",
        properties: || serde_json::json!({ "expression": { "type": "string" } }),
        required: &["expression"],
    },
    BrowserVerb {
        name: "browser_wait",
        action: "wait",
        description: "Wait a moment (seconds) for the page to settle — after a navigation or \
                      an action that loads.",
        properties: || serde_json::json!({ "duration": { "type": "number", "minimum": 0, "maximum": 30 } }),
        required: &[],
    },
    BrowserVerb {
        name: "browser_back",
        action: "back",
        description: "Go back in history.",
        properties: || serde_json::json!({}),
        required: &[],
    },
    BrowserVerb {
        name: "browser_reload",
        action: "reload",
        description: "Reload the page.",
        properties: || serde_json::json!({}),
        required: &[],
    },
    BrowserVerb {
        name: "browser_resize",
        action: "set_viewport",
        description: "Set the page's viewport (CSS pixels) to check responsive layouts; \
                      `mobile` adds touch and mobile semantics. 0×0 restores the real size.",
        properties: || {
            serde_json::json!({
                "width": { "type": "integer", "minimum": 0 },
                "height": { "type": "integer", "minimum": 0 },
                "mobile": { "type": "boolean" }
            })
        },
        required: &["width", "height"],
    },
    BrowserVerb {
        name: "browser_tabs",
        action: "list_tabs",
        description: "List the open tabs (index, title, URL).",
        properties: || serde_json::json!({}),
        required: &[],
    },
    BrowserVerb {
        name: "browser_switch_tab",
        action: "switch_tab",
        description: "Switch to the tab at `index` (from browser_tabs).",
        properties: || serde_json::json!({ "index": { "type": "integer", "minimum": 0 } }),
        required: &["index"],
    },
    BrowserVerb {
        name: "browser_hover",
        action: "hover",
        description: "Move the pointer over an element (by `ref` or `selector`) without \
                      clicking: opens hover menus and tooltips.",
        properties: || serde_json::json!({ "ref": { "type": "string" }, "selector": { "type": "string" } }),
        required: &[],
    },
    BrowserVerb {
        name: "browser_select",
        action: "select",
        description: "Choose options in a <select> dropdown (by `ref` or `selector`). `values` \
                      match an option's value or its visible label.",
        properties: || {
            serde_json::json!({
                "values": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "ref": { "type": "string" },
                "selector": { "type": "string" }
            })
        },
        required: &["values"],
    },
    BrowserVerb {
        name: "browser_upload",
        action: "upload",
        description: "Attach local files to an <input type=file> (by `ref` or `selector`). \
                      `files` are absolute paths.",
        properties: || {
            serde_json::json!({
                "files": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "ref": { "type": "string" },
                "selector": { "type": "string" }
            })
        },
        required: &["files"],
    },
    BrowserVerb {
        name: "browser_drag",
        action: "drag",
        description: "Drag one element onto another: `from_ref`/`from_selector` to \
                      `to_ref`/`to_selector`.",
        properties: || {
            serde_json::json!({
                "from_ref": { "type": "string" },
                "from_selector": { "type": "string" },
                "to_ref": { "type": "string" },
                "to_selector": { "type": "string" }
            })
        },
        required: &[],
    },
    BrowserVerb {
        name: "browser_dialog",
        action: "dialog",
        description:
            "Answer the page's open alert / confirm / prompt dialog. While one is \
                      open, other browser tools report it instead of acting. `text` fills a prompt.",
        properties: || {
            serde_json::json!({
                "accept": { "type": "boolean" },
                "text": { "type": "string" }
            })
        },
        required: &["accept"],
    },
    BrowserVerb {
        name: "browser_wait_for",
        action: "wait_for",
        description: "Wait until `text` or a `selector` appears on the page (or, with `gone`, \
                      disappears). `timeout` in seconds, default 10, max 30. Prefer this to \
                      browser_wait after an action that loads something.",
        properties: || {
            serde_json::json!({
                "text": { "type": "string" },
                "selector": { "type": "string" },
                "gone": { "type": "boolean" },
                "timeout": { "type": "number" }
            })
        },
        required: &[],
    },
    BrowserVerb {
        name: "browser_color_scheme",
        action: "color_scheme",
        description: "Make the page prefer `light` or `dark` mode, or `auto` to follow the \
                      system, to check both themes.",
        properties: || serde_json::json!({ "scheme": { "type": "string", "enum": ["light", "dark", "auto"] } }),
        required: &["scheme"],
    },
];

/// Device sizes for `browser_devices`: (id, label, width, height, mobile).
const DEVICES: &[(&str, &str, u32, u32, bool)] = &[
    ("iphone", "iPhone 15", 393, 852, true),
    ("pixel", "Pixel 8", 412, 915, true),
    ("ipad", "iPad mini", 744, 1133, true),
    ("laptop", "Laptop", 1280, 800, false),
    ("desktop", "Desktop", 1440, 900, false),
];

/// Mira's own tools beyond the browser verbs, for agents: a page at several
/// device sizes at once, and handing a task to another engine.
fn extra_agent_tools() -> Vec<Value> {
    use serde_json::json;
    vec![
        json!({
            "name": "browser_devices",
            "description": "Screenshot one page at several device sizes at once (iPhone, Pixel, \
                            iPad, laptop, desktop) to check a responsive layout. Uses the browser \
                            in Mira's pane and restores its size afterwards.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Open this first (otherwise the current page)" },
                    "devices": {
                        "type": "array",
                        "items": { "type": "string", "enum": DEVICES.iter().map(|d| d.0).collect::<Vec<_>>() },
                        "description": "Default: iphone, ipad, laptop"
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "delegate_task",
            "description": "Hand a task to another engine and get its answer: a second opinion, \
                            a review, research. `engine` is `mira` (Mira's own model) or an \
                            installed agent. Read-only: it can read and search this project, not \
                            edit or run commands. Takes up to 10 minutes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The whole task; it doesn't see this conversation" },
                    "engine": { "type": "string", "enum": crate::delegate::engines() }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "browser_record_start",
            "description": "Start recording the browser the user watches (an animated GIF), e.g. to show a flow \
                            working. Do the steps, then call browser_record_stop. Up to 3 minutes.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "browser_record_stop",
            "description": "Stop the browser recording: saves the GIF, shows it in the chat when it's small \
                            enough, and returns its path.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "request_secret",
            "description": "Ask the user privately for a secret you need (an API key, a token, a \
                            webhook signing secret). They type it into a private input; you never \
                            see the value. You get back a file path to read it from in commands, \
                            e.g. export NAME=\"$(cat path)\". Never ask for secrets in chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Environment-variable name, e.g. STRIPE_SECRET_KEY" },
                    "reason": { "type": "string", "description": "What it's for, shown to the user" },
                    "dotenv": { "type": "string", "description": "Also set it in this project .env file, e.g. .env.local" }
                },
                "required": ["name", "reason"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "html_render",
            "description": "Show the user a self-contained HTML page in the chat — a chart, table, \
                            diagram or mockup that says more than prose. Inline all CSS and JS (CDN \
                            scripts are fine). It renders sandboxed above your reply, so don't \
                            restate it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "html": { "type": "string", "description": "A complete HTML document, at most 2 MB" }
                },
                "required": ["title", "html"],
                "additionalProperties": false
            }
        }),
    ]
}

/// `browser_devices`: the page at each size, then the real size back.
async fn devices_result(state: &AppState, args: &Value) -> Value {
    use serde_json::json;
    let picked: Vec<&str> = args
        .get("devices")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .filter(|v: &Vec<&str>| !v.is_empty())
        .unwrap_or_else(|| vec!["iphone", "ipad", "laptop"]);
    let fail = |e: String| json!({ "content": [{ "type": "text", "text": e }], "isError": true });
    if let Some(url) = args
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.trim().is_empty())
    {
        if let Err(e) = state
            .browser
            .execute(&BrowserAction::Navigate {
                url: url.to_string(),
            })
            .await
        {
            return fail(e.to_string());
        }
    }
    let mut content = Vec::new();
    for id in picked {
        let Some(&(_, label, w, h, mobile)) = DEVICES.iter().find(|d| d.0 == id) else {
            content.push(json!({ "type": "text", "text": format!("{id}: unknown device") }));
            continue;
        };
        let sized = state
            .browser
            .execute(&BrowserAction::SetViewport {
                width: w,
                height: h,
                mobile,
            })
            .await;
        if let Err(e) = sized {
            content.push(json!({ "type": "text", "text": format!("{label}: {e}") }));
            continue;
        }
        let _ = state
            .browser
            .execute(&BrowserAction::Wait {
                duration: Some(0.6),
            })
            .await;
        match state.browser.execute(&BrowserAction::Screenshot).await {
            Ok(out) => {
                content.push(json!({ "type": "text", "text": format!("{label} — {w}×{h}") }));
                if let Some(shot) = out.screenshot {
                    content.push(json!({ "type": "image", "data": shot.png_base64, "mimeType": "image/png" }));
                }
            }
            Err(e) => content.push(json!({ "type": "text", "text": format!("{label}: {e}") })),
        }
    }
    let _ = state
        .browser
        .execute(&BrowserAction::SetViewport {
            width: 0,
            height: 0,
            mobile: false,
        })
        .await;
    json!({ "content": content, "isError": false })
}

/// The browser action a single-purpose tool call means.
fn browser_verb_action(name: &str, args: &Value) -> Option<Result<BrowserAction, String>> {
    let verb = BROWSER_VERBS.iter().find(|v| v.name == name)?;
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("action".into(), Value::String(verb.action.into()));
    Some(BrowserAction::from_args(&Value::Object(obj)).map_err(|e| e.to_string()))
}

/// Run a browser action and shape the MCP result (text, plus the
/// screenshot when there is one).
async fn browser_result(state: &AppState, action: &BrowserAction) -> Value {
    use serde_json::json;
    match state.browser.execute(action).await {
        Ok(out) => {
            let mut content = vec![json!({ "type": "text", "text": out.text })];
            if let Some(shot) = out.screenshot {
                content.push(json!({
                    "type": "image",
                    "data": shot.png_base64,
                    "mimeType": "image/png",
                }));
            }
            json!({ "content": content, "isError": false })
        }
        Err(e) => json!({
            "content": [{ "type": "text", "text": e.to_string() }],
            "isError": true,
        }),
    }
}

// A deliberate allowlist: never accidentally publish privileged registry tools.
const SESSION_MCP_TOOLS: &[&str] = &[
    "ask_user",
    "plan",
    "read_file",
    "write_file",
    "edit_file",
    "grep",
    "glob",
    "bash",
];

/// Which tools an agent is shown. Codex has its own shell, patch, search and
/// plan tools; listing Mira's copies beside them only splits its attention,
/// and it ignores them anyway. It keeps what it can't do itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolProfile {
    All,
    Codex,
}

impl ToolProfile {
    fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let codex = headers
            .get(mira_acp::session::MIRA_TOOL_PROFILE_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.trim()
                    .eq_ignore_ascii_case(mira_acp::session::CODEX_TOOL_PROFILE)
            });
        if codex {
            Self::Codex
        } else {
            Self::All
        }
    }

    /// Listed in `tools/list`. Calls stay accepted either way, so a thread
    /// resumed from before the profile existed keeps working.
    fn lists(self, session_tool: &str) -> bool {
        match self {
            Self::All => true,
            Self::Codex => session_tool == "ask_user",
        }
    }

    fn instructions(self) -> &'static str {
        match self {
            Self::All => "Mira's tools. The browser_* tools drive the browser the user \
                          watches live in Mira's browser pane (a dedicated profile, never the \
                          user's own): browser_open a URL, browser_snapshot to read it (with \
                          element refs), then browser_click / browser_type / browser_press; \
                          browser_screenshot to check visuals. run_background starts a \
                          long-running command (a dev server, a watcher) the user can see, read \
                          and stop in Mira's Processes window; read_output and kill_background \
                          read and stop it. ask_user displays a question form and waits for the user; \
                          plan displays a step-by-step plan card and waits for review. Prefer ask_user \
                          over guessing when a request could be shaped several ways. Only questions \
                          and plans sent through those tools reach the user. delegate_task hands a \
                          self-contained task to another engine and returns its answer. \
                          request_secret collects an API key or token privately (never ask for \
                          one in chat); html_render shows a self-contained HTML page in the chat. \
                          device_* tools drive iOS Simulators and Android Emulators; \
                          browser_record_start/stop records the browser as a GIF. \
                          Call a tool when you need its effect; never claim to have shown a form, \
                          run a command, or delegated without calling the tool first. Text \
                          descriptions of tool actions are invisible to the user.",
            Self::Codex => "Mira's tools, for what your own tools can't do. The browser_* tools \
                            drive the browser the user watches live in Mira's browser pane: \
                            browser_open a URL, browser_snapshot to read it (with element refs), \
                            then browser_click / browser_type / browser_press; browser_screenshot \
                            to check visuals. run_background starts a long-running command (a dev \
                            server, a watcher) the user can see and stop in Mira's Processes \
                            window; read_output and kill_background read and stop it. ask_user \
                            shows a question form and waits for the answer, for when \
                            request_user_input isn't available. request_secret collects an API key \
                            or token privately. html_render shows a self-contained HTML page (a \
                            chart, table, mockup) in the chat. device_* tools drive iOS Simulators \
                            and Android Emulators; browser_record_start/stop records the browser. delegate_task hands a \
                            self-contained task to another engine. Never claim to have shown a \
                            form or run a command without calling the tool.",
        }
    }
}

async fn session_tool_decision(
    state: &AppState,
    slot: &crate::slot::SessionSlot,
    tool: &dyn mira_tools::Tool,
    call: &mira_core::ToolCall,
) -> mira_policy::Decision {
    // Clone rules, then apply this chat's current mode without changing another
    // chat's policy. Explicit denies still win, including multi-target edits.
    let mut policy = state.policy.lock().await.clone();
    policy.set_mode(slot.session.read().await.config().await.agent_approval_mode);
    let mut decision = mira_policy::Decision::Allow;
    for target in tool.policy_targets(call) {
        match policy.evaluate(&mira_policy::Request {
            action: tool.action(),
            target: &target,
        }) {
            mira_policy::Decision::Deny => {
                decision = mira_policy::Decision::Deny;
                break;
            }
            mira_policy::Decision::Ask => decision = mira_policy::Decision::Ask,
            mira_policy::Decision::Allow => {}
        }
    }
    decision
}

/// Execute the chat's own tools, with its live approval posture and environment.
async fn session_tool_result(
    state: &AppState,
    grant: &McpGrant,
    name: &str,
    args: &Value,
) -> Value {
    use serde_json::json;
    let error = |message: &str| json!({ "content": [{ "type": "text", "text": message }], "isError": true });
    let Some(slot) = mcp_slot(state, grant).await else {
        return error("This chat has closed.");
    };
    let Some(tool) = slot.registry.get(name) else {
        return error(&format!("The {name} tool is unavailable."));
    };
    let call = mira_core::ToolCall {
        id: mira_core::ToolCallId::new(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name: name.into(),
            arguments: args.to_string(),
        },
    };
    let _ = slot
        .events_tx
        .send(crate::protocol::ServerMsg::ToolStart { call: call.clone() });
    let decision = session_tool_decision(state, &slot, tool.as_ref(), &call).await;
    let allowed = slot.approver.approve(&call, decision).await;
    let ctx = slot
        .make_tool_ctx(state.sandbox.clone())
        .await
        .with_compute_slot(slot.environments.slot())
        .with_bg_processes(slot.bg_processes.clone())
        .with_bg_progress(std::sync::Arc::new(crate::slot::SessionProgress {
            tx: slot.events_tx.clone(),
        }));
    let invoked = if allowed {
        tool.invoke(&call, &ctx).await
    } else {
        Ok(mira_core::ToolResult {
            call_id: call.id.clone(),
            content:
                "This tool call was denied by Mira's policy or the user. Do not retry it unchanged."
                    .into(),
            is_error: true,
            data: None,
            images: Vec::new(),
        })
    };
    match invoked {
        Ok(result) => {
            let mut content = vec![json!({ "type": "text", "text": result.content })];
            content.extend(result.images.iter().map(|image| json!({ "type": "image", "data": image.data, "mimeType": image.media_type })));
            let response = json!({ "content": content, "isError": result.is_error });
            let _ = slot
                .events_tx
                .send(crate::protocol::ServerMsg::ToolEnd { result });
            response
        }
        Err(e) => {
            let result = mira_core::ToolResult {
                call_id: call.id,
                content: e.to_string(),
                is_error: true,
                data: None,
                images: Vec::new(),
            };
            let _ = slot
                .events_tx
                .send(crate::protocol::ServerMsg::ToolEnd { result });
            error(&e.to_string())
        }
    }
}

/// The background-process tools, with exactly the specs Mira's own model
/// gets. They act on the chat's own process store, so what an agent starts
/// shows (and can be stopped) in that chat's Processes window.
fn background_tools() -> Vec<Box<dyn mira_tools::Tool>> {
    use mira_tools::builtin::background::{KillBackground, ReadOutput, RunBackground};
    vec![
        Box::new(RunBackground),
        Box::new(ReadOutput),
        Box::new(KillBackground),
    ]
}

async fn mcp_one(
    state: &AppState,
    q: &McpGrant,
    profile: ToolProfile,
    req: Value,
) -> Option<Value> {
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
                "instructions": profile.instructions(),
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => {
            // Single-purpose browser tools for agents; the multipurpose
            // `browser` stays callable for sessions that listed it before.
            let mut tools: Vec<Value> = BROWSER_VERBS
                .iter()
                .map(|v| {
                    json!({
                        "name": v.name,
                        "description": v.description,
                        "inputSchema": {
                            "type": "object",
                            "properties": (v.properties)(),
                            "required": v.required,
                            "additionalProperties": false,
                        },
                    })
                })
                .collect();
            tools.push(extra_agent_tools()[0].clone());
            if let Some(slot) = mcp_slot(state, q).await {
                for &name in SESSION_MCP_TOOLS.iter().filter(|n| profile.lists(n)) {
                    if let Some(tool) = slot.registry.get(name) {
                        let s = tool.spec();
                        tools.push(json!({ "name": s.name, "description": s.description, "inputSchema": s.parameters }));
                    }
                }
                tools.extend(extra_agent_tools().into_iter().skip(1));
                tools.extend(crate::devices::tool_specs());
                // Delegated chats run read-only and can't fan out.
                if !slot.session.read().await.is_subagent() {
                    tools.extend(crate::agent_threads::tool_specs());
                }
                tools.extend(background_tools().iter().map(|t| {
                    let s = t.spec();
                    json!({ "name": s.name, "description": s.description, "inputSchema": s.parameters })
                }));
            }
            ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = &req["params"]["arguments"];
            // Show the pane the agent is about to drive, as it would be
            // for Mira's own browser tool.
            if name == "browser" || name.starts_with("browser_") {
                if let Some(slot) = mcp_slot(state, q).await {
                    let _ = slot
                        .events_tx
                        .send(crate::protocol::ServerMsg::BrowserActive);
                }
            }
            if name == "browser_devices" {
                return Some(ok(devices_result(state, args).await));
            }
            if SESSION_MCP_TOOLS.contains(&name) {
                return Some(ok(session_tool_result(state, q, name, args).await));
            }
            if name == "delegate_task" {
                return Some(ok(delegate_result(state, q, args).await));
            }
            if name.starts_with("device_") {
                return Some(ok(match crate::devices::call(name, args).await {
                    Ok(content) => json!({ "content": content, "isError": false }),
                    Err(e) => {
                        json!({ "content": [{ "type": "text", "text": e }], "isError": true })
                    }
                }));
            }
            if name.starts_with("thread_") {
                let text = |t: String, err: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": err });
                let Some(slot) = mcp_slot(state, q).await else {
                    return Some(ok(text("This chat has closed.".into(), true)));
                };
                if let Some(out) = crate::agent_threads::call(state, &slot, name, args).await {
                    return Some(ok(match out {
                        Ok(t) => text(t, false),
                        Err(e) => text(e, true),
                    }));
                }
            }
            if name == "browser_record_start" || name == "browser_record_stop" {
                let text = |t: String, err: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": err });
                let Some(slot) = mcp_slot(state, q).await else {
                    return Some(ok(text("This chat has closed.".into(), true)));
                };
                let out = if name == "browser_record_start" {
                    crate::browser_recording::start(state, &slot).await
                } else {
                    crate::browser_recording::stop(state, &slot).await
                };
                return Some(ok(match out {
                    Ok(t) => text(t, false),
                    Err(e) => text(e, true),
                }));
            }
            if name == "request_secret" || name == "html_render" {
                let text = |t: String, err: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": err });
                let Some(slot) = mcp_slot(state, q).await else {
                    return Some(ok(text("This chat has closed.".into(), true)));
                };
                let out = if name == "request_secret" {
                    crate::agent_secrets::request_secret(state, &slot, args).await
                } else {
                    crate::agent_secrets::html_render(&slot, args)
                };
                return Some(ok(match out {
                    Ok(t) => text(t, false),
                    Err(e) => text(e, true),
                }));
            }
            if let Some(action) = browser_verb_action(name, &req["params"]["arguments"]) {
                return Some(match action {
                    Ok(a) => ok(browser_result(state, &a).await),
                    Err(e) => ok(json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true,
                    })),
                });
            }
            if name != "browser" {
                return Some(
                    match call_background(state, q, name, &req["params"]["arguments"]).await {
                        Some(Ok((text, is_error))) => ok(json!({
                            "content": [{ "type": "text", "text": text }],
                            "isError": is_error,
                        })),
                        Some(Err(e)) => err(-32603, e),
                        None => err(-32602, format!("unknown tool `{name}`")),
                    },
                );
            }
            let args = req["params"]["arguments"].clone();
            match BrowserAction::from_args(&args) {
                Ok(a) => ok(browser_result(state, &a).await),
                Err(e) => ok(json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                })),
            }
        }
        other => err(-32601, format!("method not found: {other}")),
    })
}

/// `delegate_task`: run the task on another engine (asking first when the
/// agent doesn't gate its own tool calls — it starts another model).
async fn delegate_result(state: &AppState, q: &McpGrant, args: &Value) -> Value {
    use serde_json::json;
    let text = |t: String, err: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": err });
    let Some(slot) = mcp_slot(state, q).await else {
        return text("this chat isn't open in Mira".into(), true);
    };
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or("");
    let engine = args.get("engine").and_then(Value::as_str);
    let call = mira_core::ToolCall {
        id: mira_core::ToolCallId::new(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction::new("delegate_task", args.to_string()),
    };
    if q.gate == McpGate::Mira
        && !slot
            .approver
            .approve(&call, mira_policy::Decision::Ask)
            .await
    {
        return text(
            "The user declined handing this task off. Don't retry it as-is.".into(),
            true,
        );
    }
    let call_id = call.id.to_string();
    match crate::delegate::delegate(state, &slot, prompt, engine, &call_id).await {
        Ok(answer) => text(answer, false),
        Err(e) => text(e, true),
    }
}

/// The chat a token belongs to.
async fn mcp_slot(
    state: &AppState,
    q: &McpGrant,
) -> Option<std::sync::Arc<crate::slot::SessionSlot>> {
    state.slot_str(&q.session).await
}

/// Run one of the background-process tools for the URL's chat. `None` when
/// the tool isn't one of them (or there's no chat to run it in).
async fn call_background(
    state: &AppState,
    q: &McpGrant,
    name: &str,
    args: &Value,
) -> Option<Result<(String, bool), String>> {
    let tool = background_tools()
        .into_iter()
        .find(|t| t.spec().name == name)?;
    let slot = mcp_slot(state, q).await?;
    let call = mira_core::ToolCall {
        id: mira_core::ToolCallId::new(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name: name.to_string(),
            arguments: if args.is_null() {
                "{}".into()
            } else {
                args.to_string()
            },
        },
    };
    let decision = session_tool_decision(state, &slot, tool.as_ref(), &call).await;
    if !slot.approver.approve(&call, decision).await {
        return Some(Ok((
            "This command was denied by Mira's policy or the user. Do not retry it unchanged."
                .into(),
            true,
        )));
    }
    let ctx = slot
        .make_tool_ctx(state.sandbox.clone())
        .await
        .with_compute_slot(slot.environments.slot())
        .with_bg_processes(slot.bg_processes.clone())
        .with_bg_progress(std::sync::Arc::new(crate::slot::SessionProgress {
            tx: slot.events_tx.clone(),
        }));
    Some(match tool.invoke(&call, &ctx).await {
        Ok(r) => Ok((r.content, r.is_error)),
        Err(e) => Ok((e.to_string(), true)),
    })
}

#[cfg(test)]
mod browser_verb_tests {
    use super::*;

    #[test]
    fn every_verb_maps_to_a_browser_action_with_its_own_arguments() {
        // A sample call per tool, using every property its schema offers:
        // `deny_unknown_fields` on the action rejects anything that drifts.
        let samples: &[(&str, Value)] = &[
            (
                "browser_open",
                serde_json::json!({ "url": "http://localhost:5173" }),
            ),
            ("browser_snapshot", serde_json::json!({})),
            ("browser_screenshot", serde_json::json!({})),
            (
                "browser_click",
                serde_json::json!({ "ref": "e3", "selector": "#a", "coordinate": [1.0, 2.0], "double": true }),
            ),
            (
                "browser_type",
                serde_json::json!({ "text": "hi", "ref": "e1", "selector": "#q", "clear": true, "submit": true }),
            ),
            ("browser_press", serde_json::json!({ "key": "Enter" })),
            (
                "browser_scroll",
                serde_json::json!({ "direction": "down", "amount": 2, "ref": "e9" }),
            ),
            (
                "browser_evaluate",
                serde_json::json!({ "expression": "document.title" }),
            ),
            ("browser_wait", serde_json::json!({ "duration": 1.5 })),
            ("browser_back", serde_json::json!({})),
            ("browser_reload", serde_json::json!({})),
            (
                "browser_resize",
                serde_json::json!({ "width": 390, "height": 844, "mobile": true }),
            ),
            ("browser_tabs", serde_json::json!({})),
            ("browser_switch_tab", serde_json::json!({ "index": 1 })),
            ("browser_hover", serde_json::json!({ "ref": "e2" })),
            (
                "browser_select",
                serde_json::json!({ "values": ["Canada"], "selector": "#country" }),
            ),
            (
                "browser_upload",
                serde_json::json!({ "files": ["/tmp/a.png"], "ref": "e5" }),
            ),
            (
                "browser_drag",
                serde_json::json!({ "from_ref": "e1", "from_selector": "#a", "to_ref": "e2", "to_selector": "#b" }),
            ),
            (
                "browser_dialog",
                serde_json::json!({ "accept": true, "text": "yes" }),
            ),
            (
                "browser_wait_for",
                serde_json::json!({ "text": "Saved", "selector": "#ok", "gone": false, "timeout": 5 }),
            ),
            (
                "browser_color_scheme",
                serde_json::json!({ "scheme": "dark" }),
            ),
        ];
        assert_eq!(samples.len(), BROWSER_VERBS.len(), "a sample per tool");
        for (name, args) in samples {
            let verb = BROWSER_VERBS.iter().find(|v| v.name == *name).expect(name);
            // The sample uses exactly the schema's properties.
            let props = (verb.properties)();
            for k in args.as_object().unwrap().keys() {
                assert!(props.get(k).is_some(), "{name}: `{k}` isn't in its schema");
            }
            let action = browser_verb_action(name, args).unwrap();
            assert!(action.is_ok(), "{name}: {:?}", action.err());
            assert_eq!(action.unwrap().verb(), verb.action);
        }
        assert!(browser_verb_action("browser", &serde_json::json!({})).is_none());
    }
}

#[cfg(test)]
mod tool_profile_tests {
    use super::*;

    #[test]
    fn codex_is_shown_only_the_tools_it_lacks() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(ToolProfile::from_headers(&headers), ToolProfile::All);
        headers.insert(
            mira_acp::session::MIRA_TOOL_PROFILE_HEADER,
            mira_acp::session::CODEX_TOOL_PROFILE.parse().unwrap(),
        );
        let codex = ToolProfile::from_headers(&headers);
        assert_eq!(codex, ToolProfile::Codex);
        let listed: Vec<_> = SESSION_MCP_TOOLS
            .iter()
            .filter(|n| codex.lists(n))
            .collect();
        assert_eq!(listed, [&"ask_user"]);
        assert!(SESSION_MCP_TOOLS.iter().all(|n| ToolProfile::All.lists(n)));
        assert!(!codex.instructions().contains("plan displays"));
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;

    #[test]
    fn a_token_grants_one_chat_until_revoked() {
        let t = issue_mcp_grant("sess_grant_a", McpGate::Mira);
        assert_eq!(
            grant_for(&t),
            Some(McpGrant {
                session: "sess_grant_a".into(),
                gate: McpGate::Mira
            })
        );
        assert_eq!(grant_for("not-a-token"), None);
        // An agent restarted in the chat gets the same, still-valid token.
        assert_eq!(issue_mcp_grant("sess_grant_a", McpGate::Mira), t);
        assert!(grant_for(&t).is_some());
        // Another gate is another token; both stay valid.
        let t2 = issue_mcp_grant("sess_grant_a", McpGate::Agent);
        assert_ne!(t2, t);
        assert!(grant_for(&t).is_some());
        assert_eq!(grant_for(&t2).unwrap().gate, McpGate::Agent);
        // Deleting the chat ends them.
        revoke_mcp_grants("sess_grant_a");
        assert_eq!(grant_for(&t), None);
        assert_eq!(grant_for(&t2), None);
        // Another chat's token never resolves to this one.
        let other = issue_mcp_grant("sess_grant_c", McpGate::Mira);
        assert_ne!(other, t);
        assert_eq!(grant_for(&other).unwrap().session, "sess_grant_c");
        revoke_mcp_grants("sess_grant_c");
    }

    #[test]
    fn the_token_never_rides_in_the_url() {
        let cfg = agent_mcp_config(8787, "sess_grant_b");
        let url = cfg["mcpServers"]["mira"]["url"].as_str().unwrap();
        assert_eq!(url, "http://127.0.0.1:8787/mcp");
        let auth = cfg["mcpServers"]["mira"]["headers"]["Authorization"]
            .as_str()
            .unwrap();
        let token = auth.strip_prefix("Bearer ").unwrap();
        assert!(!url.contains(token));
        assert_eq!(grant_for(token).unwrap().session, "sess_grant_b");
        revoke_mcp_grants("sess_grant_b");
    }
}

#[cfg(test)]
mod extra_tool_tests {
    use super::*;

    #[test]
    fn extra_tools_are_well_formed() {
        let t = extra_agent_tools();
        assert_eq!(t[0]["name"], "browser_devices");
        assert_eq!(t[1]["name"], "delegate_task");
        assert_eq!(
            t[1]["inputSchema"]["required"],
            serde_json::json!(["prompt"])
        );
        let devices = t[0]["inputSchema"]["properties"]["devices"]["items"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(devices.len(), DEVICES.len());
    }
}
