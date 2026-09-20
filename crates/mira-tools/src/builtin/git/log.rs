
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
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 20
                    },
                    "path": {
                        "type": "string"
                    },
                    "oneline": {
                        "type": "boolean",
                        "default": true
                    }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: LogArgs = call.parse_arguments()?;

        if args.limit == 0 {
            return Err(ToolError::Failed(
                "limit must be greater than zero".to_owned(),
            ));
        }

        let limit = args.limit.min(500);

        let mut git_args = vec![
            "--no-pager".to_owned(),
            "log".to_owned(),
            "--graph".to_owned(),
            "--decorate".to_owned(),
            "-n".to_owned(),
            limit.to_string(),
        ];

        if args.oneline {
            git_args.push("--oneline".to_owned());
        } else {
            git_args.push("--pretty=format:%h %an, %ar%n  %s%n".to_owned());
        }

        if let Some(path) = &args.path {
            let resolved = ctx
                .resolve(path)
                .ok_or_else(|| {
                    ToolError::Failed(format!("path escapes repository: {path}"))
                })?;

            git_args.push("--".to_owned());
            git_args.push(resolved.to_string_lossy().into_owned());
        }

        let out = super::run_git(ctx, &git_args, 30).await?;
        let output = out.combined_output();

        let body = if output.trim().is_empty() {
            "(no commits)".to_owned()
        } else {
            super::truncate(&output, 16_000)
        };

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}