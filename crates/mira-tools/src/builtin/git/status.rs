//! `git_status` — current branch, ahead/behind, and per-file status.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Current branch, ahead/behind, and per-file status.
pub struct GitStatus;

#[async_trait]
impl Tool for GitStatus {
    fn spec(&self) -> ToolSpec {
        spec(
            "git_status",
            "Show the working-tree status: current branch, ahead/behind vs \
             upstream, and per-file staged/unstaged/untracked state. Uses \
             `git status --porcelain=v1 --branch`.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let out = super::run_git(ctx, "git status --porcelain=v1 --branch", 15).await?;
        Ok(ToolResult::ok(call.id.clone(), format_status(&out.output)))
    }
}

/// Massage porcelain into something more scannable. Header stays; file lines
/// get a short human-readable code prefix.
fn format_status(raw: &str) -> String {
    let mut out = String::new();
    let mut file_count = 0usize;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            out.push_str("branch: ");
            out.push_str(rest);
            out.push('\n');
            continue;
        }
        if line.len() < 3 {
            continue;
        }
        let (code, path) = line.split_at(2);
        let path = path.trim_start();
        let label = match code {
            "??" => "untracked",
            " M" => "modified",
            "M " => "modified (staged)",
            "MM" => "modified (staged+unstaged)",
            "A " => "added (staged)",
            " D" => "deleted",
            "D " => "deleted (staged)",
            "R " => "renamed (staged)",
            "C " => "copied (staged)",
            "UU" => "conflict",
            other => other.trim(),
        };
        out.push_str(&format!("  {label}: {path}\n"));
        file_count += 1;
    }
    if file_count == 0 {
        out.push_str("(working tree clean)\n");
    }
    out
}
