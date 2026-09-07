//! `git_diff` — unstaged / staged / historical changes.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Diff of unstaged / staged / historical changes.
pub struct GitDiff;

#[derive(Deserialize)]
struct DiffArgs {
    #[serde(default)]
    staged: bool,
    #[serde(default)]
    path: Option<String>,
    /// A commit or range like `HEAD~3..HEAD`. When set, overrides `staged`.
    #[serde(default)]
    commit: Option<String>,
    /// `--stat` summary instead of full diff.
    #[serde(default)]
    stat: bool,
}

#[async_trait]
impl Tool for GitDiff {
    fn spec(&self) -> ToolSpec {
        spec(
            "git_diff",
            "Show a git diff. By default: unstaged changes in the working \
             tree. Set `staged: true` for the index vs HEAD, or `commit` to \
             a ref/range like `HEAD~3..HEAD`. `path` scopes to one path. \
             `stat: true` returns a `--stat` summary instead of the full diff.",
            json!({
                "type": "object",
                "properties": {
                    "staged": { "type": "boolean", "default": false },
                    "path":   { "type": "string" },
                    "commit": { "type": "string", "description": "Ref or range, e.g. `HEAD~3..HEAD`." },
                    "stat":   { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: DiffArgs = call.parse_arguments()?;
        let mut cmd = String::from("git --no-pager diff");
        if args.stat {
            cmd.push_str(" --stat");
        }
        if let Some(c) = &args.commit {
            cmd.push(' ');
            cmd.push_str(&super::shell_quote(c));
        } else if args.staged {
            cmd.push_str(" --staged");
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
            "(no diff)".to_owned()
        } else {
            super::truncate(&out.output, 32_000)
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
