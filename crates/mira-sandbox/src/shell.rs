//! Long-lived shell for per-session state (cd / venv / env exports).
//!
//! The default [`Sandbox::run`] forks a fresh `bash -lc` per command — clean
//! but forgets everything between calls. This module keeps ONE bash process
//! alive per Mira session and runs each user command inside it, so `cd`,
//! activated venvs, and exported vars stick around.
//!
//! Execution model:
//!
//! 1. On first use, spawn `bash -l` with piped stdio.
//! 2. For each command: write the command followed by a unique sentinel
//!    printf that includes the exit status. The reader accumulates output
//!    until it sees the sentinel line, then parses the exit code and hands
//!    the result back.
//! 3. On timeout, kill the child and mark the shell dead. The next call
//!    respawns a fresh one — state loss is the trade for a robust recovery
//!    story. Users see a clear "timed out" outcome.
//!
//! Not a full pty: stdin is piped, so tools that need a real terminal (ttys,
//! interactive prompts) won't work here. Same limitation the fresh-shell
//! path has today; the win is state persistence, not tty emulation.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{Outcome, SandboxError};

/// Maximum bytes of output we accumulate per command. Long-running commands
/// still complete — we just stop growing the string once we hit the cap so
/// a `find /` doesn't OOM the process.
const OUTPUT_CAP: usize = 4 * 1024 * 1024;

/// Env keys stripped at spawn time. Same set as `Sandbox`'s scrub list —
/// duplicated so this module stays self-contained; if the list grows,
/// merge with `SECRET_ENV_KEYS` there.
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

pub struct PersistentShell {
    /// The live bash process, if we still have one. `None` after a fatal
    /// error or timeout; the next `run()` respawns.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<Lines<BufReader<ChildStdout>>>,
    /// Directory bash was spawned in — used when we need to respawn.
    cwd: PathBuf,
    scrub_env: bool,
}

impl PersistentShell {
    /// Construct without spawning. The actual bash starts on the first
    /// `run()` call so a session that never uses bash pays nothing.
    pub fn new(cwd: PathBuf, scrub_env: bool) -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            cwd,
            scrub_env,
        }
    }

    fn spawn(&mut self) -> Result<(), SandboxError> {
        debug!(cwd = %self.cwd.display(), "persistent shell: spawn");
        let mut cmd = Command::new("bash");
        cmd.arg("-l")
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            // Merge stderr into stdout at the process level so a single
            // reader captures everything a command emits, in order.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if self.scrub_env {
            for k in SECRET_ENV_KEYS {
                cmd.env_remove(k);
            }
        }
        // Silence any prompt / job-control chatter that would otherwise
        // pollute output between commands.
        cmd.env("PS1", "").env("PS2", "").env("PROMPT_COMMAND", "");

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            SandboxError::Io(std::io::Error::other("no stdin on spawned bash"))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SandboxError::Io(std::io::Error::other("no stdout on spawned bash"))
        })?;
        // Drop stderr fd — we redirect per-command via `2>&1` so stderr
        // never carries content in isolation.
        drop(child.stderr.take());

        self.child = Some(child);
        self.stdin = Some(stdin);
        self.stdout = Some(BufReader::new(stdout).lines());
        Ok(())
    }

    /// Run one command in the persistent shell, waiting up to `deadline`
    /// for it to complete. State (`cd`, `export`) persists into the next
    /// call. On timeout we kill the shell — the caller sees `timed_out:
    /// true` and the next call spawns a fresh one.
    pub async fn run(
        &mut self,
        command: &str,
        deadline: Duration,
    ) -> Result<Outcome, SandboxError> {
        // Lazy spawn (or respawn after a prior kill).
        if self.child.is_none() {
            self.spawn()?;
        }

        let nonce = Uuid::new_v4().simple().to_string();
        let sentinel = format!("__MIRA_END_{nonce}__");
        // Wrap:
        //   { <cmd>; } 2>&1               → run command, merge stderr → stdout
        //   printf '\n__MIRA_END_..__%d\n' → mark completion + exit code
        // Braces group the command so the exit status seen by `$?` is the
        // command's, not printf's.
        let wrapped = format!("{{ {command}\n}} 2>&1\nprintf '\\n{sentinel}%d\\n' \"$?\"\n");

        // Take exclusive refs. Bail (leaving None) if the shell somehow
        // lost its handles — respawn on next call.
        let (Some(stdin), Some(stdout)) = (self.stdin.as_mut(), self.stdout.as_mut()) else {
            self.reset();
            return Err(SandboxError::Io(std::io::Error::other(
                "persistent shell in inconsistent state; recreated for next call",
            )));
        };
        if let Err(e) = stdin.write_all(wrapped.as_bytes()).await {
            self.reset();
            return Err(SandboxError::Io(e));
        }
        if let Err(e) = stdin.flush().await {
            self.reset();
            return Err(SandboxError::Io(e));
        }

        let mut buf = String::new();
        let mut exit_code = -1i32;
        let mut timed_out = false;
        let read_all = async {
            loop {
                match stdout.next_line().await {
                    Ok(Some(line)) => {
                        if let Some(rest) = line.strip_prefix(&sentinel) {
                            exit_code = rest.parse().unwrap_or(-1);
                            return Ok::<(), std::io::Error>(());
                        }
                        if buf.len() < OUTPUT_CAP {
                            buf.push_str(&line);
                            buf.push('\n');
                        }
                    }
                    // Stdin closed — bash died. Fall through to reset.
                    Ok(None) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        };

        match timeout(deadline, read_all).await {
            Ok(Ok(())) => {
                // If we hit EOF without seeing the sentinel, bash is dead.
                if exit_code == -1 && buf.is_empty() {
                    self.reset();
                    return Ok(Outcome {
                        exit_code: -1,
                        timed_out: false,
                        output: "(shell died; a fresh one will spawn on the next call)".into(),
                    });
                }
            }
            Ok(Err(e)) => {
                warn!(%e, "persistent shell: read error, respawning");
                self.reset();
                return Err(SandboxError::Io(e));
            }
            Err(_) => {
                // Kill the shell so a stuck command can't hold it forever.
                // Next `run()` gets a clean process.
                timed_out = true;
                self.reset();
            }
        }

        Ok(Outcome {
            exit_code,
            timed_out,
            output: buf,
        })
    }

    /// Update the shell's cwd. Sends a `cd` command through the running
    /// bash so subsequent commands land in the new folder without losing
    /// the exported env state. Falls back to a respawn if the cd fails.
    pub async fn chdir(&mut self, new_cwd: PathBuf) -> Result<(), SandboxError> {
        self.cwd = new_cwd.clone();
        if self.child.is_none() {
            return Ok(()); // will spawn fresh in the new cwd on first use
        }
        let cmd = format!("cd {}", shell_quote(&new_cwd.to_string_lossy()));
        let out = self.run(&cmd, Duration::from_secs(5)).await?;
        if out.exit_code != 0 {
            warn!(cwd = %new_cwd.display(), "shell cd failed; respawning");
            self.reset();
        }
        Ok(())
    }

    /// Tear down the current shell. Idempotent. Called after timeouts and
    /// on other unrecoverable errors so the next `run()` respawns.
    fn reset(&mut self) {
        self.stdin = None;
        self.stdout = None;
        // Dropping the Child triggers kill_on_drop → SIGKILL.
        self.child = None;
    }
}

impl Drop for PersistentShell {
    fn drop(&mut self) {
        self.reset();
    }
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}
