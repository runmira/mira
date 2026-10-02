//! The context inspector (`/api/context`): what fills the active chat's
//! context window, and taking a tool result out of it.
//!
//! Two sources, one shape. For Mira's own models the harness sizes the
//! request it would send (`mira_harness::context`). For an external agent
//! that can report it — Claude Code, over its control protocol — the
//! agent's own figures are used, mapped onto the same parts. Only Mira's
//! own history can have results dropped; an agent owns its history.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// GET /api/context
pub async fn breakdown(State(state): State<AppState>) -> Response {
    let slot = state.active_slot().await;
    if slot.acp_launch.lock().await.is_some() {
        let Some(agent) = slot.acp_agent.read().await.clone() else {
            return err(
                StatusCode::CONFLICT,
                "The agent isn't running yet — send a message first.",
            );
        };
        let Some(handle) = agent.agent().await else {
            return err(StatusCode::CONFLICT, "The agent isn't running yet.");
        };
        return match handle.context_usage().await {
            Ok(usage) => Json(from_agent(&agent.display_name, &usage)).into_response(),
            Err(e) => err(StatusCode::CONFLICT, e),
        };
    }
    let session = slot.session.read().await.clone();
    let cfg = session.config().await;
    let window = mira_harness::history::context_window_with(&cfg.model, cfg.context_window) as u64;
    let b = session.context_breakdown().await;
    Json(json!({
        "source": "mira",
        "droppable": true,
        "window": window,
        "compact_at": mira_harness::history::auto_compact_at(window),
        "breakdown": b,
        "details": [],
    }))
    .into_response()
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

/// A stable id for a category name (`Memory files` → `memory_files`).
fn slug(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn home_relative(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// An MCP server's display name: claude.ai connectors arrive as
/// `claude_ai_Gmail`; the prefix says nothing the user needs.
fn server_label(server: &str) -> String {
    server
        .strip_prefix("claude_ai_")
        .unwrap_or(server)
        .replace('_', " ")
}

/// Claude Code's `get_context_usage` answer, in the inspector's shape.
///
/// Only categories that are actually in the context count as parts: free
/// space, the auto-compact buffer and deferred tools (loaded on demand) are
/// reported by the agent but aren't sent with each request.
fn from_agent(agent: &str, u: &Value) -> Value {
    let parts: Vec<Value> = u["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["kind"] == "used" && n(&c["tokens"]) > 0)
        .map(|c| {
            let name = c["name"].as_str().unwrap_or("Other");
            json!({ "id": slug(name), "label": name, "tokens": n(&c["tokens"]) })
        })
        .collect();

    // Tool use by type, biggest first — the agent's equivalent of the
    // largest-results list (not individually droppable).
    let mut by_type: Vec<Value> = u["messageBreakdown"]["toolCallsByType"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let name = ["name", "toolName", "type"]
                .iter()
                .find_map(|k| t[*k].as_str())
                .unwrap_or("tool");
            // Every `*Tokens` field it reports (calls, results).
            let tokens: u64 = t
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| k.to_ascii_lowercase().ends_with("tokens"))
                .map(|(_, v)| n(v))
                .sum();
            json!({ "call_id": "", "tool": name, "label": "all calls", "tokens": tokens })
        })
        .filter(|t| n(&t["tokens"]) > 0)
        .collect();
    by_type.sort_by_key(|t| std::cmp::Reverse(n(&t["tokens"])));
    by_type.truncate(15);

    let memory: Vec<Value> = u["memoryFiles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| json!({ "label": home_relative(f["path"].as_str().unwrap_or("")), "tokens": n(&f["tokens"]) }))
        .collect();
    let mut skills: Vec<Value> = u["skills"]["skillFrontmatter"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({ "label": s["name"].as_str().unwrap_or(""), "tokens": n(&s["tokens"]) }))
        .collect();
    skills.sort_by_key(|s| std::cmp::Reverse(n(&s["tokens"])));
    skills.truncate(8);
    // One row per MCP server, not per tool: a connected Gmail alone brings
    // dozens of tools, which listed one by one drowned everything else.
    let mut servers: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    for t in u["mcpTools"].as_array().into_iter().flatten() {
        let server = t["serverName"].as_str().unwrap_or("other");
        let e = servers.entry(server_label(server)).or_default();
        e.0 += 1;
        e.1 += n(&t["tokens"]);
    }
    let mut mcp: Vec<Value> = servers
        .into_iter()
        .map(|(server, (count, tokens))| {
            let tools = if count == 1 {
                "1 tool".to_string()
            } else {
                format!("{count} tools")
            };
            json!({ "label": format!("{server} · {tools}"), "tokens": tokens })
        })
        .collect();
    mcp.sort_by_key(|m| std::cmp::Reverse(n(&m["tokens"])));
    mcp.truncate(8);
    let details: Vec<Value> = [
        ("Memory files", memory),
        ("Largest skills", skills),
        ("MCP servers", mcp),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .map(|(title, items)| json!({ "title": title, "items": items }))
    .collect();

    let total = u["totalTokens"]
        .as_u64()
        .unwrap_or_else(|| parts.iter().map(|p| n(&p["tokens"])).sum());
    json!({
        "source": "agent",
        "agent": agent,
        "droppable": false,
        "window": n(&u["maxTokens"]),
        "compact_at": u["autoCompactThreshold"].as_u64(),
        "breakdown": {
            "parts": parts,
            "total": total,
            // The agent's own count, not an estimate.
            "calibrated": true,
            "last_reported": null,
            "largest_results": by_type,
        },
        "details": details,
    })
}

#[derive(Deserialize)]
pub struct DropRequest {
    pub call_id: String,
}

/// POST /api/context/drop { call_id }
pub async fn drop_result(State(state): State<AppState>, Json(req): Json<DropRequest>) -> Response {
    let slot = state.active_slot().await;
    if slot.acp_launch.lock().await.is_some() {
        return err(
            StatusCode::CONFLICT,
            "An external agent keeps its own history; ask it to compact instead.",
        );
    }
    let session = slot.session.read().await.clone();
    match session.drop_tool_result(&req.call_id).await {
        Ok(d) => Json(d).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real Claude Code 2.1 `get_context_usage` reply.
    fn claude_reply() -> Value {
        json!({
            "categories": [
                {"name": "System prompt", "tokens": 2196, "color": "promptBorder", "kind": "used"},
                {"name": "System tools", "tokens": 21636, "color": "inactive", "kind": "used"},
                {"name": "System tools (deferred)", "tokens": 31523, "isDeferred": true, "kind": "deferred"},
                {"name": "Memory files", "tokens": 99, "color": "claude", "kind": "used"},
                {"name": "Skills", "tokens": 5507, "color": "warning", "kind": "used"},
                {"name": "Messages", "tokens": 12000, "kind": "used"},
                {"name": "Autocompact buffer", "tokens": 33000, "kind": "buffer"},
                {"name": "Free space", "tokens": 937562, "kind": "free"}
            ],
            "totalTokens": 41438,
            "maxTokens": 1000000,
            "autoCompactThreshold": 967000,
            "memoryFiles": [{"path": "/nonexistent-home/.claude/CLAUDE.md", "type": "User", "tokens": 99}],
            "mcpTools": [],
            "skills": {"tokens": 5507, "skillFrontmatter": [
                {"name": "dataviz", "source": "built-in", "tokens": 482},
                {"name": "artifact-design", "source": "built-in", "tokens": 66}
            ]},
            "messageBreakdown": {"toolCallsByType": [
                {"name": "Read", "callTokens": 300, "resultTokens": 6000},
                {"name": "Bash", "callTokens": 200, "resultTokens": 1500}
            ]}
        })
    }

    #[test]
    fn only_what_is_in_the_context_counts() {
        let v = from_agent("Claude Code", &claude_reply());
        let labels: Vec<_> = v["breakdown"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["label"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            labels,
            [
                "System prompt",
                "System tools",
                "Memory files",
                "Skills",
                "Messages"
            ]
        );
        let sum: u64 = v["breakdown"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| n(&p["tokens"]))
            .sum();
        assert_eq!(
            sum,
            n(&v["breakdown"]["total"]),
            "parts add up to the agent's own total"
        );
        assert_eq!(v["window"], 1_000_000);
        assert_eq!(v["compact_at"], 967_000);
        assert_eq!(v["droppable"], false);
    }

    #[test]
    fn tool_use_and_details_are_listed_biggest_first() {
        let v = from_agent("Claude Code", &claude_reply());
        let tools = v["breakdown"]["largest_results"].as_array().unwrap();
        assert_eq!(
            (tools[0]["tool"].as_str(), n(&tools[0]["tokens"])),
            (Some("Read"), 6300)
        );
        let details = v["details"].as_array().unwrap();
        let titles: Vec<_> = details
            .iter()
            .map(|d| d["title"].as_str().unwrap())
            .collect();
        assert_eq!(
            titles,
            ["Memory files", "Largest skills"],
            "empty groups are left out"
        );
        assert_eq!(details[1]["items"][0]["label"], "dataviz");
    }

    #[test]
    fn mcp_tools_are_grouped_by_server() {
        let mut reply = claude_reply();
        reply["mcpTools"] = json!([
            {"name": "mcp__claude_ai_Gmail__search", "serverName": "claude_ai_Gmail", "tokens": 300},
            {"name": "mcp__claude_ai_Gmail__send", "serverName": "claude_ai_Gmail", "tokens": 200},
            {"name": "mcp__claude_ai_Luno__price", "serverName": "claude_ai_Luno", "tokens": 90}
        ]);
        let v = from_agent("Claude Code", &reply);
        let group = v["details"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["title"] == "MCP servers")
            .unwrap();
        let rows: Vec<_> = group["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| (i["label"].as_str().unwrap().to_string(), n(&i["tokens"])))
            .collect();
        assert_eq!(
            rows,
            [
                ("Gmail · 2 tools".to_string(), 500),
                ("Luno · 1 tool".to_string(), 90)
            ]
        );
    }

    #[test]
    fn categories_get_stable_ids() {
        assert_eq!(slug("System tools (deferred)"), "system_tools__deferred");
        assert_eq!(slug("Memory files"), "memory_files");
    }
}
