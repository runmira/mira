//! Git for the worker: clone, branch, commit, push.
//!
//! The token never lands in `.git/config` or on a command line: it rides
//! in an `http.extraHeader` passed through `GIT_CONFIG_*` environment
//! variables on each network command only.

use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::spec::GitIdentity;
use crate::CloudError;

pub struct Git {
    dir: PathBuf,
    token: String,
    identity: GitIdentity,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>, token: &str, identity: GitIdentity) -> Self {
        Self {
            dir: dir.into(),
            token: token.to_owned(),
            identity,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn auth_env(&self) -> Vec<(String, String)> {
        if self.token.is_empty() {
            return Vec::new();
        }
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("x-access-token:{}", self.token));
        vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            ("GIT_CONFIG_KEY_0".into(), "http.extraHeader".into()),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: Basic {basic}"),
            ),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]
    }

    async fn run(&self, cwd: &Path, args: &[&str], auth: bool) -> Result<String, CloudError> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.current_dir(cwd)
            .args([
                "-c",
                &format!("user.name={}", self.identity.name),
                "-c",
                &format!("user.email={}", self.identity.email),
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdin(std::process::Stdio::null());
        if auth {
            cmd.envs(self.auth_env());
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| CloudError::Git(format!("git {}: {e}", args.join(" "))))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).replace(&self.token, "***");
            return Err(CloudError::Git(format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or(""),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Clone `base` from `url` and create `branch` on top of it.
    pub async fn clone_and_branch(
        &self,
        url: &str,
        base: &str,
        branch: &str,
    ) -> Result<(), CloudError> {
        let parent = self.dir.parent().unwrap_or(Path::new("/"));
        tokio::fs::create_dir_all(parent).await?;
        let dir = self.dir.to_string_lossy().into_owned();
        self.run(
            parent,
            &[
                "clone", "--quiet", "--depth", "50", "--branch", base, url, &dir,
            ],
            true,
        )
        .await?;
        self.run(&self.dir, &["checkout", "--quiet", "-b", branch], false)
            .await?;
        Ok(())
    }

    /// Commit everything (if anything changed). Returns whether a commit
    /// was made.
    pub async fn commit_all(&self, message: &str) -> Result<bool, CloudError> {
        self.run(&self.dir, &["add", "-A"], false).await?;
        let status = self
            .run(&self.dir, &["status", "--porcelain"], false)
            .await?;
        if status.trim().is_empty() {
            return Ok(false);
        }
        self.run(
            &self.dir,
            &["commit", "--quiet", "--no-verify", "-m", message],
            false,
        )
        .await?;
        Ok(true)
    }

    pub async fn commit_empty(&self, message: &str) -> Result<(), CloudError> {
        self.run(
            &self.dir,
            &[
                "commit",
                "--quiet",
                "--no-verify",
                "--allow-empty",
                "-m",
                message,
            ],
            false,
        )
        .await
        .map(drop)
    }

    pub async fn push(&self, branch: &str) -> Result<(), CloudError> {
        self.run(
            &self.dir,
            &[
                "push",
                "--quiet",
                "-u",
                "origin",
                &format!("HEAD:refs/heads/{branch}"),
            ],
            true,
        )
        .await
        .map(drop)
    }

    /// `git diff --stat` of the branch against where it started.
    pub async fn stat_since(&self, base_ref: &str) -> Result<String, CloudError> {
        self.run(&self.dir, &["diff", "--stat", base_ref, "HEAD"], false)
            .await
    }

    pub async fn head(&self) -> Result<String, CloudError> {
        Ok(self
            .run(&self.dir, &["rev-parse", "HEAD"], false)
            .await?
            .trim()
            .to_owned())
    }
}
