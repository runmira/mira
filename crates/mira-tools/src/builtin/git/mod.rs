
//! Git tools.
//!
//! Structured Git operations execute through Mira's sandbox using argv
//! directly. No shell interpolation is used.

mod status;
mod log;
mod diff;
mod commit;

pub use status::GitStatus;
pub use log::GitLog;
pub use diff::GitDiff;
pub use commit::GitCommit;

use crate::context::ToolContext;
use crate::tool::ToolError;

/// Run a Git command inside the sandbox.
///
/// `args` contains the arguments after the `git` executable.
///
/// For example:
///
/// ```text
/// run_git(ctx, &["status", "--short"], 15)
/// ```
///
/// becomes:
///
/// ```text
/// git status --short
/// ```
pub(crate) async fn run_git(
    ctx: &ToolContext,
    args: &[String],
    timeout_secs: u64,
) -> Result<mira_sandbox::CommandOutput, ToolError> {
    ctx.sandbox
        .run_with_timeout("git", args, &ctx.cwd, timeout_secs)
        .await
        .map_err(|e| ToolError::Failed(format!("git command failed: {e}")))
}

/// Truncate tool output to a maximum number of bytes while preserving UTF-8.
pub(crate) fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }

    let mut end = max_bytes.min(s.len());

    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    let mut out = s[..end].to_owned();
    out.push_str("\n… [output truncated]");
    out
}

