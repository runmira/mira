use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Run a shell command through the sandbox.
///
/// We deliberately do not let the model exec arbitrary argv — everything
/// goes through `bash -lc "..."` so we get consistent quoting semantics and
/// a single string for the policy engine to gate on.
pub struct Bash;

#[derive(Deserialize)]
struct Args {
    command: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    120_000
}

#[async_trait]
impl Tool for Bash {
    fn spec(&self) -> ToolSpec {
        spec(
            "bash",
            "Run a shell command via `bash -lc`. Working directory is the \
             repo root. Combined stdout/stderr is returned. Default timeout \
             120s; override with `timeout_ms` (max 600000).",
            json!({
                "type": "object",
                "properties": {
                    "command":    { "type": "string" },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 600000,
                        "default": 120000
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let timeout = std::time::Duration::from_millis(args.timeout_ms.min(600_000));

        let outcome = ctx
            .sandbox
            .run(&args.command, &ctx.cwd, timeout)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        // Compose a compact summary the model can act on. Truncate long
        // output so a single misfire can't blow the context window.
        let mut body = String::new();
        body.push_str(&format!("exit={}\n", outcome.exit_code));
        if outcome.timed_out {
            body.push_str("(command timed out)\n");
        }
        body.push_str("--- output ---\n");
        body.push_str(&truncate(&outcome.output, 32_000));
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn truncate(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_owned();
    }
    let head_end = floor_boundary(s, limit / 2);
    let tail_start = ceil_boundary(s, s.len().saturating_sub(limit / 2));
    format!(
        "{}\n... [truncated {} bytes] ...\n{}",
        &s[..head_end],
        s.len() - limit,
        &s[tail_start..]
    )
}

/// Round `at` down to the nearest char boundary (never past `s.len()`).
fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `at` up to the nearest char boundary.
fn ceil_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
