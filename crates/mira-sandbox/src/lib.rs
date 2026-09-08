//! Command execution.
//!
//! Today: a thin wrapper around `bash -lc` with timeout, env scrubbing, and
//! combined stdout/stderr capture. Tomorrow: platform-specific isolation —
//! `landlock` on Linux, `sandbox-exec` (seatbelt profiles) on macOS — behind
//! the same `Sandbox` handle. Downstream code should never spawn processes
//! directly; funnelling everything through here means we can tighten the
//! screws in one place.

pub mod shell;
pub use shell::PersistentShell;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub exit_code: i32,
    pub timed_out: bool,
    /// Combined stdout+stderr, in the order they were written.
    pub output: String,
}

#[derive(Clone, Debug, Default)]
pub struct SandboxConfig {
    /// If true, strip variables that commonly hold secrets before spawn.
    pub scrub_env: bool,
}

pub struct Sandbox {
    cfg: SandboxConfig,
}

impl Sandbox {
    pub fn new(cfg: SandboxConfig) -> Self {
        Self { cfg }
    }

    /// Convenience: default configuration (env scrubbing on).
    pub fn default_scrubbed() -> Self {
        Self::new(SandboxConfig { scrub_env: true })
    }

    /// Run `command` under `bash -lc`. Combined stdout+stderr is captured.
    ///
    /// On timeout the child is killed and `Outcome::timed_out` is true.
    pub async fn run(
        &self,
        command: &str,
        cwd: &Path,
        deadline: Duration,
    ) -> Result<Outcome, SandboxError> {
        debug!(%command, cwd = %cwd.display(), "sandbox exec");

        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg(command)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        if self.cfg.scrub_env {
            for k in SECRET_ENV_KEYS {
                cmd.env_remove(k);
            }
        }

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let read = async move {
            let mut buf = String::new();
            if let Some(mut o) = stdout {
                let mut s = String::new();
                o.read_to_string(&mut s).await.ok();
                buf.push_str(&s);
            }
            if let Some(mut e) = stderr {
                let mut s = String::new();
                e.read_to_string(&mut s).await.ok();
                buf.push_str(&s);
            }
            buf
        };

        // Race the child's exit against the deadline. Read pipes in parallel
        // via `tokio::join!` so a chatty command can't wedge us.
        let wait = child.wait();
        let output_fut = read;

        match timeout(deadline, async {
            let (status, output) = tokio::join!(wait, output_fut);
            (status, output)
        })
        .await
        {
            Ok((Ok(status), output)) => Ok(Outcome {
                exit_code: status.code().unwrap_or(-1),
                timed_out: false,
                output,
            }),
            Ok((Err(e), output)) => Ok(Outcome {
                exit_code: -1,
                timed_out: false,
                output: format!("{output}\n(error: {e})"),
            }),
            Err(_) => {
                // `kill_on_drop` above means dropping `child` after timeout
                // sends SIGKILL. We rebuild output from whatever came through.
                Ok(Outcome {
                    exit_code: -1,
                    timed_out: true,
                    output: String::new(),
                })
            }
        }
    }
}

/// Common env keys that hold credentials. Not exhaustive — this is defence
/// in depth, not a substitute for real secret management.
const SECRET_ENV_KEYS: &[&str] = &[
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "MIRA_API_KEY",
    "HF_TOKEN",
    "GOOGLE_APPLICATION_CREDENTIALS",
];
