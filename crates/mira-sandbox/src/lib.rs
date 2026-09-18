//! Command execution.
//!
//! Bash is spawned attached to a pseudo-terminal via `portable-pty` so
//! anything checking `isatty()` (npm progress bars, `gh`, Ink-based
//! installers, cargo, apt) renders as it would in a real terminal, and
//! the output can be streamed line-by-line to the UI while the command
//! is still running.
//!
//! Two shapes live here:
//!
//!   - [`Sandbox::run`] — one-shot: spawn a fresh PTY, run the command,
//!     collect output, tear down.
//!   - [`shell::PersistentShell`] — session-lived: one long-running bash
//!     under a PTY, individual commands multiplexed with a sentinel line
//!     so `cd`, `export`, and venv state persist across calls.
//!
//! Both take an optional [`ProgressSink`] the caller uses to receive
//! each line as it lands. The final [`Outcome`] still contains the full
//! transcript so consumers that ignore streaming see no change.
//!

pub mod shell;
pub use shell::PersistentShell;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("pty: {0}")]
    Pty(String),
    #[error("sandbox containment: {0}")]
    Containment(String),
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub exit_code: i32,
    pub timed_out: bool,
    /// Full stdout+stderr transcript, in the order they were written.
    /// Callers that also passed a [`ProgressSink`] have already received
    /// each line as it arrived; this is the same content joined by `\n`.
    pub output: String,
}

/// One-way stream of stdout+stderr lines emitted while a command is
/// still running. The caller creates the channel; every line the PTY
/// produces gets pushed on the sender before the command returns. Lines
/// arrive without their trailing newline.
pub type ProgressSink = mpsc::UnboundedSender<String>;

#[derive(Clone, Debug, Default)]
pub struct SandboxConfig {
    /// If true, strip variables that commonly hold secrets before spawn.
    pub scrub_env: bool,
}

/// Per-mode sandbox posture applied at PTY spawn time. Callers pick a
/// profile based on the current [`mira_policy::Mode`] and pass it in;
/// each variant renders to a specific `sandbox-exec` policy on macOS.
///
///   - `Restricted` — deny every filesystem write and outbound network
///     call. Used for read-only modes (`plan`, `manual`) so an
///     accidentally-approved bash command still can't write anywhere or
///     phone home.
///   - `Workspace`  — writes allowed under the workspace (canonical
///     cwd) plus the user's `~/.mira/` config dir; network open. The
///     production working posture (`auto`, `edit`).
///   - `Unrestricted` — no seatbelt at all. `yolo` mode only.
///
/// On non-macOS platforms every variant currently falls back to the
/// old "refuse to run without a real sandbox" error — Linux Landlock
/// support is a follow-up. `Unrestricted` deliberately skips
/// `sandbox-exec` entirely so `yolo` still runs there.
// Workspace-scoped writes only — matches the pre-mode-aware behaviour so
// callers that never opt in still get sensible containment.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SandboxProfile {
    Restricted,
    #[default]
    Workspace,
    Unrestricted,
}

pub struct Sandbox {
    cfg: SandboxConfig,
}

fn workspace_root(path: &Path) -> Result<PathBuf, SandboxError> {
    let root = path.canonicalize()?;
    if !root.is_dir() || root.parent().is_none() {
        return Err(SandboxError::Containment(
            "workspace must be an existing directory other than the filesystem root".into(),
        ));
    }
    Ok(root)
}

#[cfg(target_os = "macos")]
fn contained_command(root: &Path, profile: SandboxProfile) -> Result<CommandBuilder, SandboxError> {
    // `Unrestricted` bypasses sandbox-exec entirely — that's the whole
    // point of `yolo` mode. Everything else builds a per-profile
    // seatbelt policy and wraps bash.
    if profile == SandboxProfile::Unrestricted {
        let mut cmd = CommandBuilder::new("/bin/bash");
        cmd.env_remove("BASH_ENV");
        cmd.env_remove("ENV");
        cmd.env("HISTFILE", "/dev/null");
        return Ok(cmd);
    }

    let root = root
        .to_str()
        .ok_or_else(|| SandboxError::Containment("workspace path must be valid UTF-8".into()))?;
    let mira_home = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".mira"))
        .and_then(|p| p.to_str().map(|s| s.to_owned()))
        .unwrap_or_else(|| "/nonexistent-mira-home".to_owned());

    let policy = match profile {
        SandboxProfile::Restricted => {
            // Read-only + no network. Every write goes through the
            // /dev/null|/dev/tty pair so `printf`, `read -p`, etc. still
            // work without touching real files. `deny network*` covers
            // both outbound (network-outbound) and inbound
            // (network-inbound / network-bind) — safest possible bash.
            "(version 1) (allow default) \
             (deny file-write*) \
             (deny network*) \
             (allow file-write-data (literal \"/dev/null\") (literal \"/dev/tty\"))"
        }
        SandboxProfile::Workspace => {
            // Writes under cwd + ~/.mira, network open. Matches the
            // pre-profile default behaviour. `curl`, `git fetch`,
            // `npm install`, etc. all continue to work.
            "(version 1) (allow default) \
             (deny file-write*) \
             (allow file-write* (subpath (param \"WORKSPACE\"))) \
             (allow file-write* (subpath (param \"MIRA_HOME\"))) \
             (allow file-write-data (literal \"/dev/null\") (literal \"/dev/tty\"))"
        }
        SandboxProfile::Unrestricted => unreachable!("handled above"),
    };

    let mut cmd = CommandBuilder::new("/usr/bin/sandbox-exec");
    cmd.arg("-p");
    cmd.arg(policy);
    cmd.arg("-D");
    cmd.arg(format!("WORKSPACE={root}"));
    cmd.arg("-D");
    cmd.arg(format!("MIRA_HOME={mira_home}"));
    cmd.arg("/bin/bash");
    cmd.env_remove("BASH_ENV");
    cmd.env_remove("ENV");
    cmd.env("HISTFILE", "/dev/null");
    Ok(cmd)
}

#[cfg(not(target_os = "macos"))]
fn contained_command(
    _root: &Path,
    profile: SandboxProfile,
) -> Result<CommandBuilder, SandboxError> {
    // Linux Landlock is a follow-up. Until then, `Unrestricted` still
    // runs (the user explicitly opted into `yolo`); other profiles bail
    // rather than silently execute without a sandbox.
    if profile == SandboxProfile::Unrestricted {
        let mut cmd = CommandBuilder::new("/bin/bash");
        cmd.env_remove("BASH_ENV");
        cmd.env_remove("ENV");
        cmd.env("HISTFILE", "/dev/null");
        return Ok(cmd);
    }
    Err(SandboxError::Containment(
        "workspace-write containment is currently supported only on macOS; \
         switch to `yolo` mode to run without a sandbox, or wait for the \
         Landlock backend."
            .into(),
    ))
}

impl Sandbox {
    pub fn new(cfg: SandboxConfig) -> Self {
        Self { cfg }
    }

    /// Convenience: default configuration (env scrubbing on).
    pub fn default_scrubbed() -> Self {
        Self::new(SandboxConfig { scrub_env: true })
    }

    /// Run `command` under `bash -lc` attached to a PTY. Combined
    /// stdout+stderr is streamed live (if `progress` is `Some`) and
    /// captured in [`Outcome::output`].
    ///
    /// On timeout the child is killed and `Outcome::timed_out` is true.
    /// Defaults to [`SandboxProfile::Workspace`] — callers that want
    /// stricter or looser containment use [`Self::run_with_profile`].
    pub async fn run(
        &self,
        command: &str,
        cwd: &Path,
        deadline: Duration,
    ) -> Result<Outcome, SandboxError> {
        self.run_streaming(command, cwd, deadline, None).await
    }

    /// Same as [`Self::run`] but forwards each stdout+stderr line to
    /// `progress` as it arrives.
    pub async fn run_streaming(
        &self,
        command: &str,
        cwd: &Path,
        deadline: Duration,
        progress: Option<ProgressSink>,
    ) -> Result<Outcome, SandboxError> {
        self.run_with_profile(command, cwd, deadline, SandboxProfile::default(), progress)
            .await
    }

    /// Run under a caller-picked [`SandboxProfile`]. Used by the harness
    /// to apply mode-aware containment (`plan`/`manual` → Restricted,
    /// `auto`/`edit` → Workspace, `yolo` → Unrestricted).
    pub async fn run_with_profile(
        &self,
        command: &str,
        cwd: &Path,
        deadline: Duration,
        profile: SandboxProfile,
        progress: Option<ProgressSink>,
    ) -> Result<Outcome, SandboxError> {
        debug!(%command, cwd = %cwd.display(), ?profile, pty = true, "sandbox exec");

        let command = command.to_owned();
        let cwd = cwd.to_path_buf();
        let scrub = self.cfg.scrub_env;

        let outcome = tokio::task::spawn_blocking(move || {
            run_pty_blocking(&command, &cwd, deadline, scrub, profile, progress)
        })
        .await
        .map_err(|e| SandboxError::Pty(format!("join: {e}")))??;

        Ok(outcome)
    }
}

/// Blocking PTY exec. Runs on a `spawn_blocking` thread. Streams each
/// terminal line to `progress` (when set); the caller strips ANSI CSI
/// sequences before rendering — we keep them here because some tools
/// (progress bars) rely on carriage returns / cursor moves that a
/// consumer might legitimately want to render as-is.
fn run_pty_blocking(
    command: &str,
    cwd: &Path,
    deadline: Duration,
    scrub_env: bool,
    profile: SandboxProfile,
    progress: Option<ProgressSink>,
) -> Result<Outcome, SandboxError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| SandboxError::Pty(format!("openpty: {e}")))?;

    let root = workspace_root(cwd)?;
    let mut cmd = contained_command(&root, profile)?;
    cmd.arg("--noprofile");
    cmd.arg("--norc");
    cmd.arg("-c");
    cmd.arg(command);
    cmd.cwd(cwd);
    // Give TUI apps a sensible TERM. Without this, many programs
    // (`less`, `top`, node/Ink) either bail or render garbage.
    cmd.env("TERM", "xterm-256color");
    // Quiet BSD-style tools that print job-control chatter.
    cmd.env("PS1", "");
    cmd.env("PS2", "");
    cmd.env("PROMPT_COMMAND", "");
    if scrub_env {
        for k in SECRET_ENV_KEYS {
            cmd.env_remove(k);
        }
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| SandboxError::Pty(format!("spawn: {e}")))?;

    // Drop the slave FD after the child has inherited it. Some kernels
    // block reads on the master until every writer to the slave is
    // closed; the child holds the last remaining one, and closing our
    // copy is what lets EOF actually propagate when it exits.
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| SandboxError::Pty(format!("clone reader: {e}")))?;
    // Close our end of stdin — bash reads its command from `-c`, and any
    // interactive prompt (`read`, npx picker, git-editor) is expected
    // to fail fast rather than hang forever on an empty stream.
    if let Ok(mut writer) = pair.master.take_writer() {
        // Feed a single newline so simple "press Enter to continue"
        // prompts don't wedge the whole run; anything expecting more
        // than that will still hit the timeout, but with a printed
        // prompt in the streamed transcript so the user knows why.
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }

    // Read loop runs in its own thread so we can time-race the wait.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    let reader_thread = std::thread::spawn(move || {
        let mut br = BufReader::new(&mut reader as &mut dyn Read);
        loop {
            let mut line = String::new();
            // read_line preserves the trailing `\n`; strip it before
            // handing off so the progress sink sees clean text.
            match br.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let clean = strip_trailing_nl(&line);
                    if line_tx.send(clean).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Consume lines with a deadline check between reads. `try_recv` is
    // fine because the reader thread is producing continuously; on
    // idle we sleep briefly to avoid busy-looping.
    let start = Instant::now();
    let mut buf = String::new();
    let mut timed_out = false;
    loop {
        // Deadline check first — if we're already past it, kill.
        if start.elapsed() >= deadline {
            let _ = child.kill();
            timed_out = true;
            break;
        }
        let remaining = deadline.saturating_sub(start.elapsed());
        // Poll: prefer a short recv_timeout so pending output lands
        // promptly but the outer deadline stays responsive.
        let slice = std::cmp::min(remaining, Duration::from_millis(100));
        match line_rx.recv_timeout(slice) {
            Ok(line) => {
                if let Some(p) = progress.as_ref() {
                    // Best-effort — ignore the send error, the caller
                    // may have dropped the receiver already.
                    let _ = p.send(line.clone());
                }
                if buf.len() < OUTPUT_CAP {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // No line in this slice — check if the child exited.
                // `try_wait` returns Ok(Some(..)) once the child is
                // gone; give the reader thread a moment to drain, then
                // break.
                if let Ok(Some(_)) = child.try_wait() {
                    // Drain remaining buffered lines from the reader.
                    while let Ok(line) = line_rx.recv_timeout(Duration::from_millis(50)) {
                        if let Some(p) = progress.as_ref() {
                            let _ = p.send(line.clone());
                        }
                        if buf.len() < OUTPUT_CAP {
                            buf.push_str(&line);
                            buf.push('\n');
                        }
                    }
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Ensure the reader thread wakes and exits.
    let _ = reader_thread.join();

    let exit_code = if timed_out {
        -1
    } else {
        match child.wait() {
            Ok(status) => status.exit_code() as i32,
            Err(e) => {
                warn!(%e, "pty wait failed");
                -1
            }
        }
    };

    Ok(Outcome {
        exit_code,
        timed_out,
        output: buf,
    })
}

fn strip_trailing_nl(s: &str) -> String {
    let mut s = s.to_owned();
    if s.ends_with('\n') {
        s.pop();
    }
    if s.ends_with('\r') {
        s.pop();
    }
    s
}

/// Maximum bytes of output we accumulate per command. Long-running
/// commands still complete — we just stop growing the string once we
/// hit the cap so a `find /` doesn't OOM the process.
const OUTPUT_CAP: usize = 4 * 1024 * 1024;

/// Common env keys that hold credentials. Not exhaustive — this is
/// defence in depth, not a substitute for real secret management.
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_shot_allows_workspace_writes() {
        let dir = tempfile::tempdir().unwrap();
        let out = Sandbox::default_scrubbed()
            .run("printf ok > result", dir.path(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("result")).unwrap(),
            "ok"
        );
    }

    #[tokio::test]
    async fn one_shot_denies_writes_outside_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("out.txt");
        let outside_arg = outside_path.display().to_string();
        let out = Sandbox::default_scrubbed()
            .run(
                &format!("echo no > {outside_arg}"),
                ws.path(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_ne!(out.exit_code, 0, "{}", out.output);
        assert!(!outside_path.exists());
    }
}
