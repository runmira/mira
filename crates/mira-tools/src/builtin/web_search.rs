//! Web search via the Brave Search API.
//!
//! Chose Brave because it has a legitimate free tier (2000 queries/month
//! as of early 2026) and doesn't require ad-hoc HTML scraping. Users
//! provide `BRAVE_SEARCH_API_KEY` in their environment; if it isn't set,
//! the tool returns a clear error explaining how to configure it —
//! better than silently swapping to an unofficial backend.
//!
//! Swap path: the tool exposes a `SearchBackend` trait shape via its
//! module structure — replacing Brave with Tavily / Exa / etc. means
//! changing `endpoint` + response parsing. Config to pick a backend at
//! runtime can slot in when we grow more than one.

use std::env;
use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// How long we wait for Brave before giving up. Their p95 is under a
/// second; we cap at 15s so a stalled request can't wedge a turn.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Cap on `count`. Brave itself allows up to 20; larger just wastes
/// tokens without materially improving grounding.
const MAX_COUNT: usize = 20;
const DEFAULT_COUNT: usize = 5;

pub struct WebSearch;

#[derive(Deserialize)]
struct Args {
    /// Search query — free-form; passed through to Brave verbatim.
    query: String,
    /// Number of results to return. Defaults to 5, capped at 20.
    #[serde(default)]
    count: Option<usize>,
}

#[async_trait]
impl Tool for WebSearch {
    fn spec(&self) -> ToolSpec {
        spec(
            "web_search",
            "Search the web via Brave Search. Returns a list of results \
             (title, url, snippet) for the given query. Use for finding \
             docs, error message references, blog posts, or verifying \
             claims. Requires the `BRAVE_SEARCH_API_KEY` env var — the \
             tool returns a clear error if it's missing.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to search for." },
                    "count": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "default": 5,
                        "description": "How many results to return."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Network fetch; not a filesystem or bash action. `Pure` means the
        // policy engine doesn't gate — safe because we only hit a
        // configured search endpoint with the user's own API key.
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let query = args.query.trim();
        if query.is_empty() {
            return Err(ToolError::InvalidArgs("query is empty".into()));
        }
        let count = args.count.unwrap_or(DEFAULT_COUNT).clamp(1, MAX_COUNT);

        let Ok(api_key) = env::var("BRAVE_SEARCH_API_KEY") else {
            return Err(ToolError::Failed(
                "BRAVE_SEARCH_API_KEY not set. Get a free key at \
                 https://api.search.brave.com/ (2000 queries/month) \
                 and export it in your shell, or add it to your \
                 shell profile."
                    .into(),
            ));
        };

        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ToolError::Failed(format!("http client build: {e}")))?;

        let resp = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query), ("count", &count.to_string())])
            .header("X-Subscription-Token", &api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("http request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::Failed(format!(
                "Brave search returned {status}: {}",
                truncate(&body, 300)
            )));
        }

        let payload: BraveResponse = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(format!("parse: {e}")))?;

        let results = payload.web.map(|w| w.results).unwrap_or_default();
        let body = if results.is_empty() {
            format!("no results for `{query}`")
        } else {
            format_results(&results)
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/* ---------- wire types ---------- */

#[derive(Deserialize)]
struct BraveResponse {
    #[serde(default)]
    web: Option<BraveWeb>,
}

#[derive(Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    /// Brave calls the snippet `description`. Some rows also carry a
    /// dedicated `snippet` field on rich cards; we don't distinguish.
    #[serde(default)]
    description: String,
}

fn format_results(rows: &[BraveResult]) -> String {
    let mut out = String::new();
    for (i, r) in rows.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, r.title));
        out.push_str(&format!("   {}\n", r.url));
        // Strip Brave's `<strong>` highlight markup so the snippet reads
        // clean; harmless if any leak through.
        let snippet = r.description.replace("<strong>", "").replace("</strong>", "");
        if !snippet.trim().is_empty() {
            out.push_str(&format!("   {}\n", snippet.trim()));
        }
        out.push('\n');
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_owned()
    } else {
        let mut i = n;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        format!("{}…", &s[..i])
    }
}
