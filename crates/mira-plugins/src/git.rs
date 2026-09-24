//! The few git operations plugins need, via the `git` CLI.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

const GIT_TIMEOUT: Duration = Duration::from_secs(300);

async fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = tokio::process::Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    cmd.args(args)
        // Never prompt for credentials: private repos fail fast instead
        // of hanging a UI request.
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    let out = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| anyhow!("git {} timed out", args.join(" ")))?
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Shallow clone of `url` into `dest`, at `git_ref` if given, then pinned
/// to `sha` if given.
pub async fn clone(url: &str, dest: &Path, git_ref: Option<&str>, sha: Option<&str>) -> Result<()> {
    let dest_s = dest.to_string_lossy().into_owned();
    let mut args = vec!["clone", "--depth", "1", "--quiet"];
    if let Some(r) = git_ref {
        args.extend(["--branch", r]);
    }
    args.extend([url, dest_s.as_str()]);
    git(None, &args).await?;
    if let Some(sha) = sha {
        if head(dest).await.ok().as_deref() != Some(sha) {
            git(Some(dest), &["fetch", "--depth", "1", "--quiet", "origin", sha]).await?;
            git(Some(dest), &["checkout", "--quiet", sha]).await?;
        }
    }
    Ok(())
}

/// Bring a clone up to date with its remote's default branch (or the
/// branch it's on).
pub async fn pull(dir: &Path) -> Result<()> {
    git(Some(dir), &["fetch", "--depth", "1", "--quiet", "origin"]).await?;
    git(Some(dir), &["reset", "--hard", "--quiet", "FETCH_HEAD"]).await?;
    Ok(())
}

pub async fn head(dir: &Path) -> Result<String> {
    git(Some(dir), &["rev-parse", "HEAD"]).await
}
