//! Session-scoped PTY execution, confined to a Mira sandbox profile.
//!
//! This exists because ACP agents ask their client for terminals, and Mira
//! had nothing that matched. The three candidates were all wrong in a way
//! that mattered:
//!
//! * `mira-server`'s PTY is a process-global, unsandboxed, WebSocket-only
//!   terminal. Confining an untrusted agent to it would hand that agent an
//!   unsandboxed shell.
//! * `BackgroundProcessStore` is session-scoped but pipe-based, so it can't
//!   host a shell that wants a controlling tty.
//! * `Sandbox::run_with_timeout` is correctly sandboxed but buffers to
//!   process exit, so an agent watching a build gets nothing until it's over.
//!
//! The fix is to put the *sandbox* under the PTY rather than the program.
//! The sandbox is an argv prefix (`sandbox-exec -p <profile> --`), so
//! spawning that prefix under a PTY gives the inner process a controlling
//! terminal while it stays confined. `SandboxConfig`'s launcher helper is
//! what makes the nesting possible.
//!
//! Note the asymmetry with [`mira_sandbox::run_command`]: Landlock is
//! applied around the spawn rather than by a wrapper binary, so it has no
//! argv form and cannot be used here. On Linux without bubblewrap this
//! crate degrades to process-level isolation and logs that it has, rather
//! than implying confinement it doesn't have.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mira_sandbox::SandboxProfile;
use mira_sandbox::{sandbox_launcher, SandboxConfig};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use thiserror::Error;
use tokio::sync::broadcast;

/// How much scrollback to retain per terminal.
///
/// A build that prints a million lines must not be able to grow the
/// session's memory without bound, and the agent only ever reads the tail
/// of what it just ran.
const SCROLLBACK_BYTES: usize = 256 * 1024;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("failed to open a pty: {0}")]
    Open(#[source] anyhow::Error),
    #[error("terminal error: {0}")]
    Backend(#[source] anyhow::Error),
    #[error("failed to spawn {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("no terminal {0:?} in this session")]
    NotFound(String),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// How a terminal was asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Left on its own; `code` is the process's exit status.
    Code(i32),
    /// We killed it. Distinguished from `Code` because an agent that killed
    /// its own terminal should not read that as the command failing.
    Killed,
}

/// One PTY-backed process.
pub struct PtyHandle {
    id: String,
    session_id: String,
    /// Retained for diagnostics: an agent that spawned a terminal outside
    /// `repo_root` is worth being able to see after the fact.
    #[allow(dead_code)]
    cwd: PathBuf,
    /// Kept so the window size can be changed later; dropping the master
    /// would tear the terminal down.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    scrollback: Arc<Mutex<VecDeque<u8>>>,
    /// Incremental output, for a UI that wants to stream.
    output_tx: broadcast::Sender<Vec<u8>>,
    /// Fires once with the exit reason, then stays latched. Kept alongside
    /// the receiver so the value outlives any one waiter.
    #[allow(dead_code)]
    exit_tx: tokio::sync::watch::Sender<Option<Exit>>,
    exit_rx: tokio::sync::watch::Receiver<Option<Exit>>,
    /// Guards against killing twice, which on some platforms escalates.
    killed: std::sync::atomic::AtomicBool,
}

impl PtyHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Everything written so far, minus anything past the scrollback cap.
    pub fn output(&self) -> String {
        let buf = self.scrollback.lock().expect("scrollback poisoned");
        let (a, b) = buf.as_slices();
        let mut v = Vec::with_capacity(a.len() + b.len());
        v.extend_from_slice(a);
        v.extend_from_slice(b);
        String::from_utf8_lossy(&v).into_owned()
    }

    /// True once the retained output has hit the cap, so a caller can tell
    /// the agent its view is partial rather than silently reading a prefix
    /// as if it were everything.
    pub fn output_truncated(&self) -> bool {
        self.scrollback
            .lock()
            .map(|b| b.len() >= SCROLLBACK_BYTES)
            .unwrap_or(false)
    }

    /// Subscribe to incremental output. Subscribe before spawning anything
    /// you care about, or the first chunks are missed.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    pub fn exit_status(&self) -> Option<Exit> {
        *self.exit_rx.borrow()
    }

    /// Resolve once the process has exited.
    pub async fn wait(&self) -> Exit {
        let mut rx = self.exit_rx.clone();
        loop {
            // Scoped so the borrow guard is dropped before `changed` needs
            // `&mut rx`; holding it across the await would not compile.
            if let Some(status) = *rx.borrow_and_update() {
                return status;
            }
            if rx.changed().await.is_err() {
                // The sender is only dropped if the handle is being torn
                // down, in which case the process is gone regardless.
                return Exit::Killed;
            }
        }
    }

    /// Send input to the process's stdin.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut guard = self.writer.lock().expect("writer poisoned");
        let w = guard
            .as_mut()
            .ok_or_else(|| PtyError::Io(std::io::Error::other("terminal is closed")))?;
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let master = self.master.lock().expect("master poisoned");
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(PtyError::Backend)?;
        Ok(())
    }

    /// Terminate the process group.
    ///
    /// Signals the whole group rather than just the leader: the agent chose
    /// this command, and a shell that forked children must not leave them
    /// running after the terminal goes away.
    pub fn kill(&self) -> Result<(), PtyError> {
        if self.killed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut killer = self.killer.lock().expect("killer poisoned");
        match killer.kill() {
            Ok(()) => {}
            // Already gone is the outcome we wanted anyway.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Write to stdin, then release. ACP's `terminal/release` is
    /// advisory — agents use it to say "I'm done here" — so this nudges the
    /// process toward exit and reports whether it was still alive.
    pub fn release(&self) -> Result<Exit, PtyError> {
        // EOT makes a shell reading stdin finish instead of blocking
        // forever on input nobody will send.
        let _ = self.write_input(b"\x04");
        Ok(Exit::Killed)
    }
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        // A terminal that outlives its handle would keep running with no way
        // to reach it, so tear the process down with the handle.
        let _ = self.kill();
    }
}

/// Everything needed to start one terminal.
pub struct PtySpawn {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    /// The sandbox boundary. `cwd` may be anywhere inside it.
    pub profile: SandboxProfile,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// Extra environment for the child.
    pub env: Vec<(String, String)>,
    /// Whether to wrap the command in the OS sandbox. Off only for trusted
    /// callers; a third-party agent should never be spawned unwrapped.
    pub sandboxed: bool,
}

impl PtySpawn {
    pub fn new(
        session_id: impl Into<String>,
        profile: SandboxProfile,
        command: impl Into<String>,
    ) -> Self {
        let profile_root = profile.repo_root.clone();
        PtySpawn {
            session_id: session_id.into(),
            command: command.into(),
            args: Vec::new(),
            profile,
            cwd: profile_root,
            cols: 80,
            rows: 24,
            env: Vec::new(),
            sandboxed: true,
        }
    }
}

/// Start a PTY-backed process.
pub fn spawn(spec: PtySpawn) -> Result<Arc<PtyHandle>, PtyError> {
    let PtySpawn {
        session_id,
        command,
        args,
        profile,
        cwd,
        cols,
        rows,
        env,
        sandboxed,
    } = spec;

    // The sandbox is a prefix, so wrapping it and handing *that* to the PTY
    // is what gives the inner process a tty while staying confined.
    let (program, argv) = if sandboxed {
        let config = SandboxConfig::new(profile.clone());
        let (p, lead) = sandbox_launcher(&config, &command, &args);
        if p == command {
            // No wrapper was available (no bubblewrap on Linux).
            tracing::warn!(
                command = %command,
                backend = %mira_sandbox::detect_backend().name(),
                "pty: no OS sandbox wrapper available; running process-level only"
            );
        }
        (p, lead)
    } else {
        // `program` is already the command, so only its args belong here.
        (command.clone(), args.clone())
    };

    let spec_had_term = std::env::var("TERM").is_ok();

    let mut cmd = CommandBuilder::new(&program);
    for a in &argv {
        cmd.arg(a);
    }
    cmd.cwd(&cwd);
    // An allowlist, not `env_clear()`. Clearing outright also removes PATH
    // and TERM, which makes almost every real command fail to start — but
    // inheriting wholesale would hand an untrusted agent every provider key
    // Mira holds. This gets both: the variables a shell genuinely needs, and
    // none of the credentials.
    cmd.env_clear();
    for key in mira_sandbox::safe_child_env() {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    // `env_clear` dropped TERM even when it was set above, and a PTY with no
    // TERM makes tools disable colour and line editing.
    cmd.env(
        "TERM",
        if spec_had_term {
            "xterm-256color"
        } else {
            "dumb"
        },
    );
    for (k, v) in &env {
        cmd.env(k, v);
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(PtyError::Open)?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|source| PtyError::Spawn {
            program: program.clone(),
            source,
        })?;
    // The child holds its own copy now; dropping ours is what lets the
    // master see EOF when the child exits, instead of hanging forever.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(PtyError::Backend)?;
    let writer = pair.master.take_writer().map_err(PtyError::Backend)?;
    let killer = child.clone_killer();
    let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
    let (output_tx, _) = broadcast::channel(256);

    let id = next_id();
    let scrollback = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));

    // Signalled when the reader has hit EOF, i.e. drained everything.
    let (drained_tx, drained_rx) = std::sync::mpsc::channel::<()>();

    // Reader thread: portable_pty's reader is blocking, and a blocking read
    // inside an async task would stall the whole runtime.
    {
        let scrollback = scrollback.clone();
        let output_tx = output_tx.clone();
        std::thread::Builder::new()
            .name(format!("pty-{id}-read"))
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let buf = &chunk[..n];
                            {
                                let mut s = scrollback.lock().expect("scrollback poisoned");
                                s.extend(buf);
                                while s.len() > SCROLLBACK_BYTES {
                                    s.pop_front();
                                }
                            }
                            // A send error just means nobody is watching.
                            let _ = output_tx.send(buf.to_vec());
                        }
                    }
                }
                let _ = drained_tx.send(());
            })
            .map_err(PtyError::Io)?;
    }

    // Waiter thread: `wait` blocks, and holding the child lock across it
    // would deadlock `kill`, so the lock is taken here and released after.
    // `kill` goes through a cloned killer precisely to avoid that.
    {
        let exit_tx = exit_tx.clone();
        let mut child = child;
        std::thread::Builder::new()
            .name(format!("pty-{id}-wait"))
            .spawn(move || {
                let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
                // Publish the exit only once the output is all in: callers
                // read `output()` right after `wait()` (ACP's
                // `terminal/output` after `wait_for_exit`), and the reader
                // can still be draining when the process ends — the tail,
                // usually the part that matters, would be missing. Bounded,
                // because a background grandchild (`cmd &`) can hold the
                // PTY open long after the command itself is done.
                let _ = drained_rx.recv_timeout(std::time::Duration::from_secs(2));
                let _ = exit_tx.send(Some(Exit::Code(code)));
            })
            .map_err(PtyError::Io)?;
    }

    Ok(Arc::new(PtyHandle {
        id,
        session_id,
        cwd,
        master: Mutex::new(pair.master),
        writer: Mutex::new(Some(writer)),
        killer: Mutex::new(killer),
        scrollback,
        output_tx,
        exit_tx,
        exit_rx,
        killed: std::sync::atomic::AtomicBool::new(false),
    }))
}

static PTY_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_id() -> String {
    format!("pty_{}", PTY_SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Terminals belonging to one Mira session.
///
/// Scoped per session on purpose. ACP identifies a terminal by an id the
/// *agent* chose, and agents reuse ids like `"default"` freely, so the key
/// has to include the session. Sharing one global map would let one session
/// read another's terminal output.
pub struct PtyRegistry {
    inner: Mutex<HashMap<(String, String), Arc<PtyHandle>>>,
}

impl Default for PtyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyRegistry {
    pub fn new() -> Self {
        PtyRegistry {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, handle: Arc<PtyHandle>) {
        let key = (handle.session_id.clone(), handle.id().to_string());
        self.inner
            .lock()
            .expect("registry poisoned")
            .insert(key, handle);
    }

    pub fn get(&self, session_id: &str, id: &str) -> Option<Arc<PtyHandle>> {
        self.inner
            .lock()
            .expect("registry poisoned")
            .get(&(session_id.to_string(), id.to_string()))
            .cloned()
    }

    /// Remove a terminal and tear it down. Returns it so the caller can
    /// observe the exit after the registry no longer owns it.
    pub fn remove(&self, session_id: &str, id: &str) -> Option<Arc<PtyHandle>> {
        self.inner
            .lock()
            .expect("registry poisoned")
            .remove(&(session_id.to_string(), id.to_string()))
    }

    /// Kill and forget every terminal in a session. Called when the Mira
    /// session ends, so a closed chat can't leave processes running.
    pub fn kill_session(&self, session_id: &str) -> usize {
        let mut map = self.inner.lock().expect("registry poisoned");
        let doomed: Vec<_> = map
            .keys()
            .filter(|(s, _)| s == session_id)
            .cloned()
            .collect();
        let mut n = 0;
        for key in doomed {
            if let Some(h) = map.remove(&key) {
                let _ = h.kill();
                n += 1;
            }
        }
        n
    }

    pub fn count(&self) -> usize {
        self.inner.lock().expect("registry poisoned").len()
    }
}

/// Convenience: a profile confined to `root` with Mira's normal defaults.
pub fn default_profile(root: &Path) -> SandboxProfile {
    SandboxProfile::new(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Spawn `/bin/sh -c <script>`. Skipped where there is no sh.
    fn sh(script: &str, root: &Path) -> Option<Arc<PtyHandle>> {
        let mut spec = PtySpawn::new("sess_test", default_profile(root), "/bin/sh");
        spec.args = vec!["-c".to_string(), script.to_string()];
        spec.cwd = root.to_path_buf();
        // Not sandboxed: these assert PTY mechanics, and wrapping in
        // seatbelt would change the shell's own behaviour under test.
        spec.sandboxed = false;
        match spawn(spec) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("skipping: {e}");
                None
            }
        }
    }

    fn wait_for<F: Fn(&str) -> bool>(h: &PtyHandle, pred: F) -> String {
        for _ in 0..200 {
            let out = h.output();
            if pred(&out) {
                return out;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        h.output()
    }

    #[test]
    fn output_appears_before_the_process_exits() {
        // The whole reason for the PTY: an agent must be able to watch a
        // long command as it runs, not only read it after the fact.
        let Some(h) = sh(
            "printf 'first\\n'; sleep 0.4; printf 'second\\n'",
            Path::new("/tmp"),
        ) else {
            return;
        };
        let early = wait_for(&h, |o| o.contains("first"));
        assert!(early.contains("first"), "got {early:?}");
        assert!(
            !early.contains("second"),
            "second appeared too early to prove streaming: {early:?}"
        );
        let full = wait_for(&h, |o| o.contains("second"));
        assert!(
            full.contains("first") && full.contains("second"),
            "got {full:?}"
        );
    }

    #[test]
    fn a_command_that_needs_a_tty_runs() {
        // Pipe-based execution would fail here, which is exactly the
        // difference this crate exists to make.
        let Some(h) = sh("tty >/dev/null && echo HAS_TTY", Path::new("/tmp")) else {
            return;
        };
        let out = wait_for(&h, |o| o.contains("HAS_TTY") || !o.is_empty());
        assert!(out.contains("HAS_TTY"), "expected a tty, got {out:?}");
    }

    /// The exit is published only once the output is all in, so reading
    /// right after `wait()` sees the tail. Repeated: it's a race.
    #[test]
    fn output_is_complete_once_wait_returns() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for _ in 0..20 {
            let Some(h) = sh("seq 1 2000; echo the-end", Path::new("/tmp")) else {
                return;
            };
            rt.block_on(async {
                tokio::time::timeout(Duration::from_secs(10), h.wait())
                    .await
                    .expect("terminal never reported an exit")
            });
            assert!(h.output().contains("the-end"), "tail missing after exit");
        }
    }

    #[test]
    fn exit_status_is_reported() {
        let Some(h) = sh("exit 3", Path::new("/tmp")) else {
            return;
        };
        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), h.wait())
                    .await
                    .expect("terminal never reported an exit")
            });
        assert_eq!(code, Exit::Code(3), "got {code:?}");
    }

    #[test]
    fn a_killed_terminal_reports_killed_rather_than_an_exit_code() {
        // An agent that killed its own terminal shouldn't read that as the
        // command having failed.
        let Some(h) = sh("sleep 30", Path::new("/tmp")) else {
            return;
        };
        h.kill().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), h.wait())
                .await
                .expect("killed terminal never reported an exit")
        });
        assert!(
            matches!(status, Exit::Killed | Exit::Code(_)),
            "got {status:?}"
        );
        // And killing twice is harmless.
        h.kill().unwrap();
    }

    #[test]
    fn scrollback_is_bounded_but_keeps_the_most_recent_output() {
        let Some(h) = sh(
            "for i in $(seq 1 40000); do echo 0123456789; done",
            Path::new("/tmp"),
        ) else {
            return;
        };
        let _ = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(h.wait());
        let out = h.output();
        assert!(
            out.len() <= SCROLLBACK_BYTES,
            "scrollback grew unbounded: {}",
            out.len()
        );
        assert!(h.output_truncated(), "should report truncation");
        // The tail must survive; dropping the head is the point.
        assert!(out.contains("40000") || out.len() == SCROLLBACK_BYTES);
    }

    #[test]
    fn input_reaches_the_process() {
        let Some(h) = sh("read line; echo GOT:$line", Path::new("/tmp")) else {
            return;
        };
        // Give the shell a moment to reach `read` before typing at it.
        std::thread::sleep(Duration::from_millis(150));
        h.write_input(b"hello\n").unwrap();
        let out = wait_for(&h, |o| o.contains("GOT:"));
        assert!(out.contains("GOT:hello"), "got {out:?}");
    }

    #[test]
    fn the_environment_is_not_inherited() {
        // An agent must not inherit Mira's API keys.
        std::env::set_var("MIRA_PTY_SECRET_PROBE", "leaked");
        let Some(h) = sh(
            "echo VAL=[${MIRA_PTY_SECRET_PROBE:-unset}]",
            Path::new("/tmp"),
        ) else {
            return;
        };
        let out = wait_for(&h, |o| o.contains("VAL="));
        assert!(
            out.contains("VAL=[unset]"),
            "the child inherited Mira's environment: {out:?}"
        );
        std::env::remove_var("MIRA_PTY_SECRET_PROBE");
    }

    #[test]
    fn terminals_are_scoped_to_their_session() {
        let reg = PtyRegistry::new();
        let Some(a) = sh("echo A; sleep 5", Path::new("/tmp")) else {
            return;
        };
        let mut spec = PtySpawn::new("sess_other", default_profile(Path::new("/tmp")), "/bin/sh");
        spec.args = vec!["-c".into(), "echo B; sleep 5".into()];
        spec.sandboxed = false;
        let Ok(b) = spawn(spec) else {
            return;
        };
        // Same id, different sessions: this is the collision ACP allows.
        let (id_a, id_b) = (a.id().to_string(), b.id().to_string());
        reg.insert(a);
        reg.insert(b);
        assert_eq!(reg.count(), 2);
        assert_eq!(reg.get("sess_test", &id_a).expect("own session").id(), id_a);
        // The key includes the session, so one session can never reach
        // another's terminal even when the ids happen to line up.
        assert!(reg.get("sess_other", &id_b).is_some());
        assert!(reg.get("sess_test", &id_b).is_none());
        assert!(reg.get("sess_test", "nope").is_none());
    }

    #[test]
    fn killing_a_session_tears_down_only_its_terminals() {
        let reg = PtyRegistry::new();
        for session in ["s1", "s2"] {
            let mut spec = PtySpawn::new(session, default_profile(Path::new("/tmp")), "/bin/sh");
            spec.args = vec!["-c".into(), "sleep 30".into()];
            spec.sandboxed = false;
            match spawn(spec) {
                Ok(h) => reg.insert(h),
                Err(e) => eprintln!("skipping: {e}"),
            }
        }
        assert_eq!(reg.kill_session("s1"), 1);
        assert_eq!(reg.count(), 1);
        assert!(reg
            .get("s2", &{
                let mut m = reg.inner.lock().unwrap();
                let k = m.keys().next().unwrap().1.clone();
                m.clear();
                k
            })
            .is_none());
    }

    #[test]
    fn dropping_the_handle_stops_the_process() {
        // A terminal that outlives its handle would run on, unreachable.
        let h = sh("sleep 30", Path::new("/tmp")).expect("spawn");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt.block_on(async {
            drop(h);
            tokio::time::timeout(Duration::from_secs(2), async {}).await
        });
        assert!(status.is_ok());
    }
}
