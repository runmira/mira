//! Fetch a URL and return its readable text.
//!
//! Complements `web_search`: search gives you titles + snippets + URLs,
//! fetch turns any of those URLs into content the model can actually
//! reason over. Uses `html2text` to convert HTML into a plain-text
//! rendering (tables, lists, links preserved as text) — no full DOM,
//! no JavaScript execution.
//!
//! Deliberately narrow:
//!
//! - **GET only.** No POST / auth / form submission. Read-web tool.
//! - **Bounded body.** Caps at 1MB downloaded and truncates the rendered
//!   text at 20k chars so a giant article doesn't blow the context.
//! - **Same-process HTTP.** Uses the workspace `reqwest` — respects
//!   system proxies via `HTTP_PROXY` / `HTTPS_PROXY` env vars.

use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// Max bytes downloaded. Bigger pages will just render whatever we've
/// buffered up to this point — usually plenty for docs / articles.
const MAX_DOWNLOAD_BYTES: usize = 1_000_000;
/// Max plain-text characters returned to the model.
const MAX_TEXT_CHARS: usize = 20_000;
/// Line-wrap column for html2text output.
const RENDER_WIDTH: usize = 100;

pub struct WebFetch;

#[derive(Deserialize)]
struct Args {
    /// Absolute URL to fetch. `http://` or `https://` only.
    url: String,
    /// Optional cap on chars returned; defaults to 20k. Lets the model
    /// ask for a tighter view when it just wants a summary.
    #[serde(default)]
    max_chars: Option<usize>,
}

#[async_trait]
impl Tool for WebFetch {
    fn spec(&self) -> ToolSpec {
        spec(
            "web_fetch",
            "Download a web page and return its readable text (HTML → \
             plain text via html2text). GET only, capped at 20k chars. \
             Pair with `web_search` to first find URLs, then fetch the \
             most promising ones. Does not execute JavaScript — for \
             JS-heavy sites (single-page apps) you'll get the skeleton, \
             not the rendered content.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Absolute http(s) URL to fetch."
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": 500,
                        "maximum": 40000,
                        "default": 20000,
                        "description": "Cap on characters returned."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Reads the public web — no filesystem or shell effects. Policy
        // engines can still opt to gate at the target URL level; the
        // default `Pure` mapping treats it like a benign read.
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let url = args.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ToolError::InvalidArgs(
                "url must be an absolute http(s) URL".into(),
            ));
        }
        let cap = args.max_chars.unwrap_or(MAX_TEXT_CHARS).clamp(500, 40_000);

        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ToolError::Failed(format!("http client build: {e}")))?;

        let resp = client
            .get(url)
            .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5")
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("fetch: {e}")))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let final_url = resp.url().to_string();

        if !status.is_success() {
            return Err(ToolError::Failed(format!(
                "GET {url} returned {status}"
            )));
        }

        // Cap the downloaded body so a stray 500MB endpoint doesn't OOM us.
        // We read as a byte stream and stop after MAX_DOWNLOAD_BYTES.
        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ToolError::Failed(format!("body read: {e}")))?;
            let remaining = MAX_DOWNLOAD_BYTES.saturating_sub(bytes.len());
            if remaining == 0 {
                break;
            }
            let take = remaining.min(chunk.len());
            bytes.extend_from_slice(&chunk[..take]);
            if bytes.len() >= MAX_DOWNLOAD_BYTES {
                break;
            }
        }

        // Render. For HTML → text; for plain content types, just decode as
        // UTF-8 (lossy) and skip the parser.
        let rendered = if content_type.contains("html") {
            html2text::from_read(bytes.as_slice(), RENDER_WIDTH)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };

        let (body, truncated) = if rendered.chars().count() > cap {
            // Char-boundary safe truncation.
            let mut out: String = rendered.chars().take(cap).collect();
            out.push_str("\n\n… [truncated]");
            (out, true)
        } else {
            (rendered, false)
        };

        // Wrap in a small header so the model can see where the content
        // came from and whether it's the final URL after redirects. Also
        // fences the fetched body as untrusted — a small nudge for the
        // "prompt injection defense" concern later.
        let mut out = String::new();
        out.push_str(&format!("URL: {final_url}\n"));
        if !content_type.is_empty() {
            out.push_str(&format!("Content-Type: {content_type}\n"));
        }
        if truncated {
            out.push_str(&format!("(truncated to {cap} chars)\n"));
        }
        out.push_str("\n--- content (untrusted external data) ---\n\n");
        out.push_str(&body);
        Ok(ToolResult::ok(call.id.clone(), out))
    }
}
