//! Structured Git tools.
//!
//! Wraps the `git` CLI (via the sandbox) so the model sees VCS state as a
//! first-class concept instead of having to remember porcelain flags in bash.
//! Read-only operations (`git_status`, `git_diff`, `git_log`) report as
//! [`Action::Read`]; `git_commit` mutates state and reports as [`Action::Bash`]
//! so existing bash policy rules gate it.
//!
//! We deliberately don't wrap `push`, `checkout`, `reset`, `rebase` yet —
//! those have sharp edges and are better left to explicit bash where the user
//! can approve the exact invocation.

use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

// -- git_status ---------------------------------------------------------------

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
        let out = run_git(ctx, "git status --porcelain=v1 --branch", 15).await?;
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

// -- git_diff -----------------------------------------------------------------

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
            cmd.push_str(&shell_quote(c));
        } else if args.staged {
            cmd.push_str(" --staged");
        }
        if let Some(p) = &args.path {
            let resolved = ctx
                .resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))?;
            cmd.push_str(" -- ");
            cmd.push_str(&shell_quote(&resolved.to_string_lossy()));
        }
        let out = run_git(ctx, &cmd, 30).await?;
        let body = if out.output.trim().is_empty() {
            "(no diff)".to_owned()
        } else {
            truncate(&out.output, 32_000)
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

// -- git_log ------------------------------------------------------------------

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
            cmd.push_str(&shell_quote(&resolved.to_string_lossy()));
        }
        let out = run_git(ctx, &cmd, 30).await?;
        let body = if out.output.trim().is_empty() {
            "(no commits)".to_owned()
        } else {
            truncate(&out.output, 16_000)
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

// -- git_commit ---------------------------------------------------------------

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
                    "message": { "type": "string" },
                    "all":     { "type": "boolean", "default": false },
                    "amend":   { "type": "boolean", "default": false }
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
        build_commit_command(&args)
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: CommitArgs = call.parse_arguments()?;
        let cmd = build_commit_command(&args);
        let out = run_git(ctx, &cmd, 60).await?;
        let mut body = String::new();
        body.push_str(&format!("$ {cmd}\n"));
        body.push_str(&format!("exit={}\n", out.exit_code));
        if out.timed_out {
            body.push_str("(command timed out)\n");
        }
        body.push_str("--- output ---\n");
        body.push_str(&truncate(&out.output, 16_000));
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn build_commit_command(args: &CommitArgs) -> String {
    let mut cmd = String::from("git commit");
    if args.all {
        cmd.push_str(" -a");
    }
    if args.amend {
        cmd.push_str(" --amend");
    }
    cmd.push_str(" -m ");
    cmd.push_str(&shell_quote(&args.message));
    cmd
}

// -- shared -------------------------------------------------------------------

async fn run_git(
    ctx: &ToolContext,
    command: &str,
    timeout_secs: u64,
) -> Result<mira_sandbox::Outcome, ToolError> {
    ctx.sandbox
        .run(command, &ctx.cwd, Duration::from_secs(timeout_secs))
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
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

fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
