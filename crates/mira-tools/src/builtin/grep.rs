use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Search the repo with ripgrep.
///
/// We shell out to `rg` (bundled or on PATH) rather than reimplementing —
/// nothing in Rust matches ripgrep's performance for this workload, and
/// the model already knows its flags.
pub struct Grep;

#[derive(Deserialize)]
struct Args {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_max() -> usize {
    200
}

#[async_trait]
impl Tool for Grep {
    fn spec(&self) -> ToolSpec {
        spec(
            "grep",
            "Search for a regex pattern across the repo using ripgrep. \
             Returns matching lines with file:line prefixes.",
            json!({
                "type": "object",
                "properties": {
                    "pattern":          { "type": "string" },
                    "path":             { "type": "string", "description": "Subdirectory to restrict to." },
                    "glob":             { "type": "string", "description": "Ripgrep glob filter, e.g. `*.rs`." },
                    "case_insensitive": { "type": "boolean", "default": false },
                    "max_results":      { "type": "integer", "minimum": 1, "maximum": 2000, "default": 200 }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        let mut cmd = String::from("rg --line-number --no-heading --color never");
        if args.case_insensitive {
            cmd.push_str(" -i");
        }
        cmd.push_str(&format!(" --max-count {}", args.max_results));
        if let Some(g) = &args.glob {
            cmd.push_str(&format!(" --glob {}", shell_quote(g)));
        }
        cmd.push(' ');
        cmd.push_str(&shell_quote(&args.pattern));
        if let Some(p) = &args.path {
            cmd.push(' ');
            let resolved = ctx
                .resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))?;
            cmd.push_str(&shell_quote(&resolved.to_string_lossy()));
        }

        let outcome = ctx
            .sandbox
            .run(&cmd, &ctx.cwd, Duration::from_secs(30))
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        // ripgrep exits 1 when there are no matches — surface that as ok
        // rather than an error, so the model can see "nothing found".
        let body = if outcome.output.trim().is_empty() {
            format!("no matches for /{}/", args.pattern)
        } else {
            outcome.output
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/// Single-quote a string for `bash -lc`. Anything not printable-ASCII gets
/// escaped by ripgrep's own argument parser, so we only worry about `'`.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}
