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
//!
//! Each tool lives in its own file:
//! - [`status`] — `git_status`
//! - [`diff`]   — `git_diff`
//! - [`log`]    — `git_log`
//! - [`commit`] — `git_commit`

pub mod commit;
pub mod diff;
pub mod log;
pub mod status;

use std::time::Duration;

use crate::context::ToolContext;
use crate::tool::ToolError;

pub use commit::GitCommit;
pub use diff::GitDiff;
pub use log::GitLog;
pub use status::GitStatus;

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
