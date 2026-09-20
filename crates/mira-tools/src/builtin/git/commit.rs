//! `git_commit` — create a commit. Never passes `--no-verify` — hooks are the
//! user's safety net and this tool refuses to bypass them.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Create a commit. Never passes `--no-verify` — hooks are the user's safety
/// net and this tool refuses to bypass them.
pub struct GitCommit;

#[derive(Deserialize)]
struct CommitArgs {
    message: String,

    /// Include tracked-and-modified files (`git commit -a`). Untracked files
    /// are never auto-staged; the model must `git add` them explicitly via bash.
    #[serde(default)]
    all: bool,

    #[serde(default)]
    amend: bool,
}

#[async_trait]
impl Tool for GitCommit {
    fn spec(&self) -> ToolSpec {
        spec(
            "git_commit",
            "Create a git commit with the given message. Only staged changes \
             are committed unless `all: true` (which also stages tracked \
             modifications, but never untracked files). `amend: true` amends \
             HEAD. Hooks are always run — this tool refuses `--no-verify`.",
            json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string"
                    },
                    "all": {
                        "type": "boolean",
                        "default": false
                    },
                    "amend": {
                        "type": "boolean",
                        "default": false
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    fn policy_target(&self, call: &ToolCall) -> String {
        let args: CommitArgs = match call.parse_arguments() {
            Ok(a) => a,
            Err(_) => return "git commit".to_owned(),
        };

        build_commit_display(&args)
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: CommitArgs = call.parse_arguments()?;

        if args.message.trim().is_empty() {
            return Err(ToolError::Failed(
                "commit message cannot be empty".to_owned(),
            ));
        }

        let git_args = build_commit_args(&args);

        let out = super::run_git(ctx, &git_args, 60).await?;

        let mut body = String::new();

        body.push_str(&format!("$ {}\n", build_commit_display(&args)));
        body.push_str(&format!("exit={:?}\n", out.exit_code));

        if out.timed_out {
            body.push_str("(command timed out)\n");
        }

        let output = out.combined_output();

        body.push_str("--- output ---\n");
        body.push_str(&super::truncate(&output, 16_000));

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn build_commit_args(args: &CommitArgs) -> Vec<String> {
    let mut git_args = vec!["commit".to_owned()];

    if args.all {
        git_args.push("-a".to_owned());
    }

    if args.amend {
        git_args.push("--amend".to_owned());
    }

    git_args.push("-m".to_owned());
    git_args.push(args.message.clone());

    git_args
}

fn build_commit_display(args: &CommitArgs) -> String {
    let mut command = String::from("git commit");

    if args.all {
        command.push_str(" -a");
    }

    if args.amend {
        command.push_str(" --amend");
    }

    command.push_str(" -m ");
    command.push_str(&display_quote(&args.message));

    command
}

fn display_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}
