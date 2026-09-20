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
                    "staged": {
                        "type": "boolean",
                        "default": false
                    },
                    "path": {
                        "type": "string"
                    },
                    "commit": {
                        "type": "string",
                        "description": "Ref or range, e.g. `HEAD~3..HEAD`."
                    },
                    "stat": {
                        "type": "boolean",
                        "default": false
                    }
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

        let mut git_args = vec!["--no-pager".to_owned(), "diff".to_owned()];

        if args.stat {
            git_args.push("--stat".to_owned());
        }

        if let Some(commit) = &args.commit {
            git_args.push(commit.clone());
        } else if args.staged {
            git_args.push("--staged".to_owned());
        }

        if let Some(path) = &args.path {
            let resolved = ctx
                .resolve(path)
                .ok_or_else(|| ToolError::Failed(format!("path escapes repository: {path}")))?;

            git_args.push("--".to_owned());
            git_args.push(resolved.to_string_lossy().into_owned());
        }

        let out = super::run_git(ctx, &git_args, 30).await?;
        let output = out.combined_output();

        let body = if output.trim().is_empty() {
            "(no diff)".to_owned()
        } else {
            super::truncate(&output, 32_000)
        };

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
