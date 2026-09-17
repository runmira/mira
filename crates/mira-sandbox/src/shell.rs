//! Long-lived shell for per-session state (cd / venv / env exports).
//!
//! The default [`Sandbox::run`] spawns a fresh `bash -lc` per command
//! under a PTY — clean but forgets everything between calls. This
//! module keeps ONE bash process alive per Mira session, also under a
//! PTY, and runs each user command inside it. `cd`, activated venvs,
//! exported vars, and TTY-dependent tools (npx installers, Ink UIs,
//! progress bars) all keep working across calls.
//!
//! Execution model:
//!
//! 1. On first use, spawn `bash -l` attached to a PTY.
//! 2. For each command: write the command followed by a unique
//!    sentinel `printf` that includes the exit status. A dedicated
//!    reader thread accumulates output until it sees the sentinel
//!    line, then parses the exit code and hands the result back.
//! 3. Each line is optionally forwarded to a caller-supplied progress
//!    sink so the UI can render live output under the pending tool
//!    card.
//! 4. On timeout, kill the child and mark the shell dead. The next
//!    call respawns a fresh one — state loss is the trade for a
//!    robust recovery story. Users see a clear "timed out" outcome.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Mutex as StdMutex;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child as PtyChild, MasterPty, PtySize};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{Outcome, ProgressSink, SandboxError, SandboxProfile};

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

/// Maximum bytes of output we accumulate per command.
const OUTPUT_CAP: usize = 4 * 1024 * 1024;

pub struct PersistentShell {
    /// Live bash process wrapped so it can be sent between threads.
    /// `None` after a fatal error or timeout; the next `run()` respawns.
    child: Option<Box<dyn PtyChild + Send + Sync>>,
    /// PTY master handle — owns the writer + reader for the shell.
    master: Option<Box<dyn MasterPty + Send>>,
    /// Buffered line stream fed by a dedicated reader thread. Held
    /// behind a Mutex so the reader thread + `run()` share it.
    lines: Option<Arc<StdMutex<std_mpsc::Receiver<String>>>>,
    /// Handle to the reader thread. Detached on `reset()` rather than
    /// joined — the thread exits on EOF once the child dies, and
    /// blocking async code on a join can stall the tokio runtime.
    reader: Option<std::thread::JoinHandle<()>>,
    /// Directory bash was spawned in — used when we need to respawn.
    cwd: PathBuf,
    workspace: Result<PathBuf, String>,
    writer: Option<Box<dyn Write + Send>>,
    scrub_env: bool,
    /// Sandbox posture the currently-running bash was spawned with. A
    /// call to [`Self::set_profile`] with a different value marks the
    /// shell dirty so the next `run_streaming` respawns with the new
    /// seatbelt policy — mode changes (`/mode auto`, `/mode plan`)
    /// take effect on the next bash call without a session restart.
    profile: SandboxProfile,
    /// Set to `true` when a `run_streaming` future is dropped before it
    /// finishes normally (e.g. user interrupt, task cancellation). The
    /// underlying `spawn_blocking` task can still be holding the mpsc
    /// receiver mutex — reusing this shell would wedge on that lock for
    /// up to `deadline` seconds. On the next `run_streaming` we check
    /// this flag first and reset the shell so state is guaranteed clean.
    ///
    /// Also flipped by [`Self::set_profile`] when the requested profile
    /// differs from the one the current shell was spawned under —
    /// same reset-on-next-call machinery keeps the switch atomic.
    ///
    /// Shared via `Arc` with a per-call `RunGuard` so drop can flip it
    /// even after the outer async function is gone. The bool is atomic
    /// so the check-and-clear in `run_streaming` doesn't need a mutex.
    dirty: Arc<AtomicBool>,
}

/// RAII guard that marks the shell dirty if its `run_streaming` scope
/// exits abnormally (future dropped, panic, `?`-early-return). On a
/// clean completion the caller flips `completed = true` first so drop
/// becomes a no-op. See [`PersistentShell.dirty`] for why.
struct RunGuard {
    dirty: Arc<AtomicBool>,
    completed: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }
}

impl PersistentShell {
    /// Construct without spawning. The actual bash starts on the first
    /// `run()` call so a session that never uses bash pays nothing.
    ///
    /// The initial [`SandboxProfile`] defaults to `Workspace`; callers
    /// that want stricter or looser containment use [`Self::with_profile`]
    /// or update it later via [`Self::set_profile`].
    pub fn new(cwd: PathBuf, scrub_env: bool) -> Self {
        Self::with_profile(cwd, scrub_env, SandboxProfile::default())
    }

    /// Same as [`Self::new`] but pins the initial sandbox profile.
    pub fn with_profile(cwd: PathBuf, scrub_env: bool, profile: SandboxProfile) -> Self {
        Self {
            child: None,
            master: None,
            lines: None,
            reader: None,
            workspace: crate::workspace_root(&cwd).map_err(|e| e.to_string()),
            writer: None,
            cwd,
            scrub_env,
            profile,
            dirty: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Swap the sandbox profile for future bash calls. Idempotent when
    /// the requested profile matches the current one; otherwise flips
    /// the dirty flag so the next `run_streaming` resets the live bash
    /// and respawns under the new seatbelt policy. Session state
    /// (`cd`, exported vars, activated venvs) is lost on that respawn
    /// — the trade for a real containment switch.
    pub fn set_profile(&mut self, profile: SandboxProfile) {
        if self.profile == profile {
            return;
        }
        debug!(
            from = ?self.profile,
            to = ?profile,
            "persistent shell: profile changed — will respawn on next call"
        );
        self.profile = profile;
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Read the currently-active profile. Mainly for tests / diagnostics.
    pub fn profile(&self) -> SandboxProfile {
        self.profile
    }

    fn spawn(&mut self) -> Result<(), SandboxError> {
        debug!(cwd = %self.cwd.display(), "persistent shell: spawn (pty)");
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SandboxError::Pty(format!("openpty: {e}")))?;

        let root = match &self.workspace {
            Ok(root) => root.clone(),
            Err(e) => return Err(SandboxError::Containment(e.clone())),
        };
        let mut cmd = crate::contained_command(&root, self.profile)?;
        cmd.arg("--noprofile");
        cmd.arg("--norc");
        cmd.arg("-l");
        cmd.cwd(&self.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("PS1", "");
        cmd.env("PS2", "");
        cmd.env("PROMPT_COMMAND", "");
        if self.scrub_env {
            for k in SECRET_ENV_KEYS {
                cmd.env_remove(k);
            }
        }

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| SandboxError::Pty(format!("take_writer: {e}")))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| SandboxError::Pty(format!("spawn: {e}")))?;

        // Drop our copy of the slave so EOF on the master actually
        // propagates when bash exits.
        drop(pair.slave);

        let reader_h = pair
            .master
            .try_clone_reader()
            .map_err(|e| SandboxError::Pty(format!("clone reader: {e}")))?;

        let (tx, rx) = std_mpsc::channel::<String>();
        let reader_thread = std::thread::spawn(move || {
            let mut r = reader_h;
            let mut br = BufReader::new(&mut r as &mut dyn Read);
            loop {
                let mut line = String::new();
                match br.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let clean = strip_trailing_nl(&line);
                        if tx.send(clean).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.child = Some(child);
        self.writer = Some(writer);
        self.master = Some(pair.master);
        self.lines = Some(Arc::new(StdMutex::new(rx)));
        self.reader = Some(reader_thread);
        Ok(())
    }

    /// Run one command in the persistent shell, waiting up to `deadline`
    /// for it to complete. State (`cd`, `export`) persists into the
    /// next call. On timeout we kill the shell — the caller sees
    /// `timed_out: true` and the next call spawns a fresh one.
    pub async fn run(
        &mut self,
        command: &str,
        deadline: Duration,
    ) -> Result<Outcome, SandboxError> {
        self.run_streaming(command, deadline, None).await
    }

    /// Same as [`Self::run`], but every line of stdout+stderr is also
    /// forwarded to `progress` (when provided) as it arrives.
    pub async fn run_streaming(
        &mut self,
        command: &str,
        deadline: Duration,
        progress: Option<ProgressSink>,
    ) -> Result<Outcome, SandboxError> {
        // If the previous call was cancelled mid-flight (future dropped
        // by an interrupt / turn abort), the `spawn_blocking` task can
        // still be holding `lines.lock()` and the underlying bash is in
        // an unknown state — mid-command, mid-heredoc, whatever. Reusing
        // it would wedge us on that lock until the old deadline expires.
        // Force a fresh shell so state is guaranteed clean; state loss
        // (cd/exports) is an acceptable trade for guaranteed progress.
        if self.dirty.swap(false, Ordering::SeqCst) {
            debug!("persistent shell: previous run was cancelled — resetting before reuse");
            self.reset();
        }

        // Lazy spawn (or respawn after a prior kill).
        if self.child.is_none() {
            self.spawn()?;
        }

        // Install the dirty-flag guard *before* any await point. If the
        // outer future is dropped from here on, `Drop` marks the shell
        // dirty so the next call knows to reset. Cleared to `completed`
        // just before we return successfully.
        let mut guard = RunGuard {
            dirty: self.dirty.clone(),
            completed: false,
        };

        let nonce = Uuid::new_v4().simple().to_string();
        let sentinel = format!("__MIRA_END_{nonce}__");
        // Wrap:
        //   { <cmd>; } 2>&1               → run command, merge stderr → stdout
        //   printf '__MIRA_END_..__%d\n'  → mark completion + exit code
        // Braces group the command so the exit status seen by `$?` is
        // the command's, not printf's.
        let wrapped = format!("{{ {command}\n}} 2>&1\nprintf '\\n{sentinel}%d\\n' \"$?\"\n");

        // Write the wrapped command onto the PTY.
        let write_res = if let Some(writer) = self.writer.as_mut() {
            writer
                .write_all(wrapped.as_bytes())
                .and_then(|_| writer.flush())
        } else {
            Err(std::io::Error::other("pty master missing"))
        };
        if let Err(e) = write_res {
            self.reset();
            return Err(SandboxError::Io(e));
        }

        let lines = match self.lines.as_ref() {
            Some(l) => l.clone(),
            None => {
                self.reset();
                return Err(SandboxError::Pty("line channel missing".into()));
            }
        };
        let sentinel_c = sentinel.clone();
        let progress_c = progress;

        // Drain lines until we see the sentinel — or the deadline. The
        // whole loop runs on a blocking thread so mpsc + PTY reads
        // don't block the tokio runtime.
        let outcome = tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let mut buf = String::new();
            let mut exit_code = -1i32;
            let mut timed_out = false;

            let rx = lines.lock().expect("line channel poisoned");
            loop {
                if start.elapsed() >= deadline {
                    timed_out = true;
                    break;
                }
                let remaining = deadline.saturating_sub(start.elapsed());
                let slice = std::cmp::min(remaining, Duration::from_millis(200));
                match rx.recv_timeout(slice) {
                    Ok(line) => {
                        if let Some(rest) = line.strip_prefix(&sentinel_c) {
                            exit_code = rest.trim().parse().unwrap_or(-1);
                            break;
                        }
                        if let Some(p) = progress_c.as_ref() {
                            let _ = p.send(line.clone());
                        }
                        if buf.len() < OUTPUT_CAP {
                            buf.push_str(&line);
                            buf.push('\n');
                        }
                    }
                    Err(std_mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                        // Shell died. Fall through with whatever we
                        // captured so the caller sees the failure text.
                        break;
                    }
                }
            }

            Outcome {
                exit_code,
                timed_out,
                output: buf,
            }
        })
        .await
        .map_err(|e| SandboxError::Pty(format!("join: {e}")))?;

        if outcome.timed_out {
            self.reset();
        } else if outcome.exit_code == -1 && outcome.output.is_empty() {
            self.reset();
            // Clean completion path — the shell died on us, but this
            // call itself finished normally with a synthetic outcome.
            guard.completed = true;
            return Ok(Outcome {
                exit_code: -1,
                timed_out: false,
                output: "(shell died; a fresh one will spawn on the next call)".into(),
            });
        }

        // Clean completion — no need to force a reset on the next call.
        guard.completed = true;
        Ok(outcome)
    }

    /// Update the shell's cwd. Sends a `cd` command through the running
    /// bash so subsequent commands land in the new folder without
    /// losing exported env state. Falls back to a respawn if the cd
    /// fails.
    pub async fn chdir(&mut self, new_cwd: PathBuf) -> Result<(), SandboxError> {
        self.cwd = new_cwd.clone();
        if self.child.is_none() {
            return Ok(());
        }
        let cmd = format!("cd {}", shell_quote(&new_cwd.to_string_lossy()));
        let out = self.run(&cmd, Duration::from_secs(5)).await?;
        if out.exit_code != 0 {
            warn!(cwd = %new_cwd.display(), "shell cd failed; respawning");
            self.reset();
        }
        Ok(())
    }

    /// Cooperative-cancel signal from the harness: kill the running
    /// child immediately so a Stop from the user doesn't wait for the
    /// next bash call to trigger a reset. Idempotent — a shell that
    /// wasn't running (or already reset) is untouched. Marks dirty so
    /// the reader thread's pending output can't leak into the next
    /// command's transcript.
    ///
    /// This is a thin wrapper over [`Self::reset`] with a friendlier
    /// name at the call site (bash tool observing `ctx.cancel`).
    pub fn interrupt(&mut self) {
        self.reset();
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Tear down the current shell. Idempotent. Called after timeouts,
    /// on other unrecoverable errors, and at the top of `run_streaming`
    /// when a prior cancellation left the shell dirty.
    ///
    /// The reader thread is *detached*, not joined — joining could stall
    /// the tokio runtime for however long the reader's blocking read
    /// takes to hit EOF after we kill the child. The thread exits on its
    /// own once EOF lands; the extra thread lingering for a few ms is
    /// harmless (its Arc-cloned `tx` is orphaned, so send errors and it
    /// returns).
    fn reset(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
        }
        self.writer = None;
        self.master = None;
        self.lines = None;
        // Detach: drop the JoinHandle without joining.
        let _ = self.reader.take();
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

// Kept for future signalling — `OnceLock` is a stand-in for a global
// "PTY subsystem available" gate we might wire up on platforms where
// the native PTY isn't guaranteed. Not currently used; left here as a
// hook so a fallback path (piped-stdio bash) can be plumbed without
// changing the public API.
#[allow(dead_code)]
static PTY_UNAVAILABLE: OnceLock<bool> = OnceLock::new();

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persistent_allows_workspace_and_denies_outside_writes() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("out.txt");
        let outside_arg = outside_path.display().to_string();
        let mut shell = PersistentShell::new(ws.path().to_path_buf(), true);

        let inside = shell
            .run("printf ok > result", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(inside.exit_code, 0, "{}", inside.output);
        assert_eq!(
            std::fs::read_to_string(ws.path().join("result")).unwrap(),
            "ok"
        );

        let denied = shell
            .run(&format!("echo no > {outside_arg}"), Duration::from_secs(10))
            .await
            .unwrap();
        assert_ne!(denied.exit_code, 0, "{}", denied.output);
        assert!(!outside_path.exists());
    }

    /// Regression for the "commands don't come back" wedge: if a
    /// `run_streaming` future is dropped mid-flight (user interrupt),
    /// the next call must not block on the previous call's stale
    /// `spawn_blocking` holding the mpsc receiver mutex.
    #[tokio::test]
    async fn persistent_shell_recovers_after_cancellation() {
        let ws = tempfile::tempdir().unwrap();
        let mut shell = PersistentShell::new(ws.path().to_path_buf(), true);

        // Warm the shell so we exercise the reused path.
        let _ = shell
            .run("printf warm", Duration::from_secs(5))
            .await
            .unwrap();

        // Kick off a long-running command with a big deadline, then
        // drop the future well before it finishes. Matches "user
        // hits Interrupt" mid-command.
        let cancel_result = tokio::time::timeout(
            Duration::from_millis(80),
            shell.run("sleep 5", Duration::from_secs(120)),
        )
        .await;
        assert!(cancel_result.is_err(), "expected timeout/cancellation");

        // The next call must return promptly — before the old deadline
        // — proving the dirty-flag drop guard cleaned up the wedge.
        let started = std::time::Instant::now();
        let ok = shell
            .run("printf resumed", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "next call should not wait on the previous deadline"
        );
        assert_eq!(ok.exit_code, 0, "{}", ok.output);
        assert!(ok.output.contains("resumed"), "got: {}", ok.output);
    }

    #[tokio::test]
    async fn persistent_shell_recovers_after_timeout() {
        let ws = tempfile::tempdir().unwrap();
        let mut shell = PersistentShell::new(ws.path().to_path_buf(), true);

        let slow = shell
            .run("sleep 5", Duration::from_millis(300))
            .await
            .unwrap();
        assert!(slow.timed_out);

        let recovered = shell
            .run("printf done > marker", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(recovered.exit_code, 0, "{}", recovered.output);
        assert_eq!(
            std::fs::read_to_string(ws.path().join("marker")).unwrap(),
            "done"
        );
    }
}
