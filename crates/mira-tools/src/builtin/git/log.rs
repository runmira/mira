//! `git_log` — recent commits.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Recent commits.
pub struct GitLog;

#[derive(Deserialize)]
struct LogArgs {
    #[serde(default = "default_log_limit")]
    limit: usize,
    #[serde(default)]
    path: Option<String>,
    /// When true, one-line-per-commit output (the default). Set to false to
    /// get the full commit message + author for each commit.
    #[serde(default = "default_true")]
    oneline: bool,
}

fn default_log_limit() -> usize {
    20
}
fn default_true() -> bool {
    true
}

#[async_trait]
impl Tool for GitLog {
    fn spec(&self) -> ToolSpec {
        spec(
            "git_log",
            "Show recent commits with `git log --graph --decorate`. Defaults \
             to 20 commits in one-line form. Scope with `path`, expand with \
             `oneline: false`.",
            json!({
                "type": "object",
                "properties": {
                    "limit":   { "type": "integer", "minimum": 1, "maximum": 500, "default": 20 },
                    "path":    { "type": "string" },
                    "oneline": { "type": "boolean", "default": true }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: LogArgs = call.parse_arguments()?;
        let mut cmd = format!("git --no-pager log --graph --decorate -n {}", args.limit);
        if args.oneline {
            cmd.push_str(" --oneline");
        } else {
            cmd.push_str(" --pretty=format:'%h %an, %ar%n  %s%n'");
        }
        if let Some(p) = &args.path {
            let resolved = ctx
                .resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))?;
            cmd.push_str(" -- ");
            cmd.push_str(&super::shell_quote(&resolved.to_string_lossy()));
        }
        let out = super::run_git(ctx, &cmd, 30).await?;
        let body = if out.output.trim().is_empty() {
            "(no commits)".to_owned()
        } else {
            super::truncate(&out.output, 16_000)
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
