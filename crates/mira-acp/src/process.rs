//! Spawning an agent process and wiring its stdio into a [`Connection`].
//!
//! This is the seam between "we resolved a [`LaunchConfig`]" and "we can
//! talk to an agent". Two things here are load-bearing and easy to get
//! wrong:
//!
//! * **The child's environment is an allowlist, not an inheritance.** An
//!   ACP agent is third-party code, and Mira's own process environment
//!   holds credentials for providers the agent has no business seeing. The
//!   agent gets the variables it needs plus the ones its driver resolved —
//!   never Mira's ambient secrets.
//! * **The child is put in its own process group and reaped on drop.** An
//!   agent CLI spawns subprocesses of its own; without a group kill, a
//!   dropped session would leave a build still running with no owner.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use thiserror::Error;
use tokio::process::{Child, Command};

use crate::conn::{AgentCallback, ConnError, Connection};
use crate::driver::{LaunchConfig, PermissionMode};

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("agent binary not found: {0}")]
    NotFound(String),
    #[error("failed to spawn agent: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("acp: {0}")]
    Conn(#[from] ConnError),
}

/// A spawned agent process plus the connection talking to it.
pub struct AgentProcess {
    conn: Connection,
    child: Arc<tokio::sync::Mutex<Child>>,
}

impl AgentProcess {
    /// Spawn `launch` and start pumping its stdio.
    ///
    /// `callbacks` receives everything the agent sends back. The process is
    /// killed when this handle is dropped.
    pub async fn spawn(
        launch: &LaunchConfig,
        callbacks: Arc<dyn AgentCallback>,
    ) -> Result<AgentProcess, SpawnError> {
        // Resolved to an absolute path: the child gets a deliberately small
        // environment, so a bare `claude` would not be found by the same
        // lookup that `looks_installed` just used to validate it.
        let mut cmd = Command::new(crate::which::resolve_for_spawn(&launch.program));
        cmd.args(&launch.args);
        cmd.env_clear();
        for key in mira_sandbox::safe_child_env() {
            if let Ok(val) = std::env::var(key) {
                if !crate::which::env_denied(key, &launch.env_deny) {
                    cmd.env(key, val);
                }
            }
        }
        // The child needs a PATH that can actually find things. Its own
        // subprocesses — an agent that is a node script, a shell, or a
        // version-manager shim — are all resolved through this, so handing
        // down a bare system PATH breaks the agent one level deeper than the
        // spawn itself.
        if let Some(path) = crate::which::effective_path() {
            cmd.env("PATH", path);
        }
        // The driver's resolved env goes last so it wins over any inherited
        // value — this is where the agent's own credentials arrive. Denied
        // prefixes never land, however explicit: see `env_denied`.
        for (k, v) in &launch.env {
            if !crate::which::env_denied(k, &launch.env_deny) {
                cmd.env(k, v);
            }
        }

        // stdout is the protocol channel and nothing else may read it.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A probe or aborted session must not orphan the agent: when the
        // handle is dropped (including the timeout-dropped future in
        // `probe_initialize`), the child goes too.
        cmd.kill_on_drop(true);
        // Its own process group, so a session teardown can take the agent's
        // children with it rather than orphaning a running build.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                SpawnError::NotFound(launch.program.display().to_string())
            }
            _ => SpawnError::Spawn(e),
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SpawnError::Spawn(std::io::Error::other("no stdout")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SpawnError::Spawn(std::io::Error::other("no stderr")))?;

        let conn = Connection::spawn_transport(stdout, stdout_stdin(&mut child), callbacks);
        // stderr is diagnostics only; it is drained into a bounded tail so a
        // chatty agent cannot fill a pipe and block itself.
        conn.attach_stderr(stderr);

        Ok(AgentProcess {
            conn,
            child: Arc::new(tokio::sync::Mutex::new(child)),
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Ask the agent to exit, then wait briefly and force it if it doesn't.
    ///
    /// Order matters: a well-behaved agent can flush its session state on
    /// the way out, which a hard kill would discard.
    pub async fn shutdown(&self, grace: std::time::Duration) {
        let mut child = self.child.lock().await;
        // The child's stdin is already owned by the connection, so there is
        // no clean channel to close; signal instead.
        #[cfg(unix)]
        let _ = child.start_kill();
        #[cfg(not(unix))]
        let _ = child.start_kill();

        match tokio::time::timeout(grace, child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
    }

    /// Whether the process has exited on its own.
    pub async fn has_exited(&self) -> bool {
        self.conn.is_closed()
    }

    /// Recent stderr, for a "why did it die" message.
    pub async fn stderr_tail(&self) -> String {
        self.conn.stderr_tail().await
    }
}

impl std::fmt::Debug for AgentProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately opaque: the child's command line may carry resolved
        // credentials, and this type is logged in error paths.
        f.debug_struct("AgentProcess")
            .field("closed", &self.conn.is_closed())
            .finish_non_exhaustive()
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        // `Drop` cannot await, so use the synchronous start_kill. The OS
        // reaps the zombie; the process group teardown in `shutdown` is the
        // graceful path, and this is the "we panicked or the task was
        // cancelled" path.
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

/// Take the child's stdin as the connection's write half.
///
/// `tokio::process::Child::stdin` can only be taken once, and the
/// connection needs to own it for the life of the process.
fn stdout_stdin(child: &mut Child) -> tokio::process::ChildStdin {
    child.stdin.take().expect("stdin was not piped")
}

/// Whether a resolved launch looks runnable, without running it.
///
/// Used to tell the user "grok isn't installed" before a session is created,
/// rather than failing opaquely mid-prompt.
pub fn looks_installed(launch: &LaunchConfig) -> bool {
    if launch.program.components().count() > 1 {
        return launch.program.is_file();
    }
    // Not the inherited PATH: an app launched outside a shell does not have
    // the version-manager directories its user relies on, and that is exactly
    // how an installed agent came to be reported as missing.
    crate::which::resolve(&launch.program.to_string_lossy()).is_some()
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Everything needed to bring one agent up.
pub struct StartSpec<'a> {
    pub launch: &'a LaunchConfig,
    /// What to advertise at `initialize`.
    ///
    /// `None` means "ask the driver" via [`AcpDriver::client_caps`], which is
    /// what almost every caller wants: capabilities must match the agent, and
    /// only the driver knows. Overriding exists for a caller that genuinely
    /// has narrower ports bound than the driver would assume.
    pub caps: Option<crate::session::ClientCaps>,
    pub files: Arc<dyn crate::host::FilePort>,
    pub terminals: Arc<dyn crate::host::TerminalPort>,
    pub permissions: Arc<dyn crate::host::PermissionPort>,
    pub events: Arc<dyn crate::host::EventPort>,
    /// Working directory for `session/new`. `None` skips session creation,
    /// leaving the caller to do it once it has somewhere valid to point at.
    pub cwd: Option<&'a Path>,
    /// Mira's tool server URL, offered to agents that take HTTP MCP servers.
    pub mira_mcp: Option<crate::session::MiraMcp>,
}

/// Start an agent, taking both the launch and the capabilities from `driver`.
///
/// The normal entry point: resolving argv, picking per-agent capabilities and
/// spawning are all one decision, and splitting them across the caller is how
/// Codex would end up being offered terminals it never asked for.
pub async fn start_driver(
    driver: &dyn crate::driver::AcpDriver,
    cfg: &crate::driver::DriverConfig,
    mode: crate::driver::PermissionMode,
    program: &Path,
    ports: HostPorts,
    cwd: Option<&Path>,
) -> Result<AcpAgent, StartError> {
    let launch = driver.resolve(cfg, mode, program.to_path_buf());
    start(StartSpec {
        launch: &launch,
        caps: Some(driver.client_caps()),
        files: ports.files,
        terminals: ports.terminals,
        permissions: ports.permissions,
        events: ports.events,
        cwd,
        mira_mcp: ports.mira_mcp,
    })
    .await
}

/// Decide how to reach a driver, and get there.
///
/// Selection is explicit rather than implicit in a `match` at each call site:
/// `Auto` prefers the agent's own CLI and falls back to the adapter, and
/// knowing which one was chosen is needed to report it honestly — an agent
/// that works is not the same claim as an agent that works the way the label
/// says.
pub fn choose_transport(
    driver: &dyn crate::driver::AcpDriver,
    cfg: &crate::driver::DriverConfig,
    program: &Path,
) -> crate::driver::Transport {
    match driver.transport() {
        crate::driver::Transport::Acp => crate::driver::Transport::Acp,
        crate::driver::Transport::Native => crate::driver::Transport::Native,
        crate::driver::Transport::Auto => {
            // Presence of the *native* CLI, not the adapter. Checking the
            // adapter here is a self-defeating test: it is absent in exactly
            // the case native is meant to serve, so every such user was sent
            // down the ACP path and then told the adapter was missing.
            // Presence is about the program, not the argv — and the launch
            // is built by the same function the spawn uses, so the two can
            // never disagree again.
            let native_ok = build_native_launch(driver, cfg, PermissionMode::Ask, program, None)
                .is_some_and(|l| looks_installed(&l));
            // Native wins when it is available. It needs no adapter, its
            // detection cannot be wrong, and it is the path the user already
            // has installed.
            if native_ok {
                crate::driver::Transport::Native
            } else {
                crate::driver::Transport::Acp
            }
        }
    }
}

/// The four host ports. Grouped because every caller supplies the same four
/// and passing them positionally invites a swap.
pub struct HostPorts {
    pub files: Arc<dyn crate::host::FilePort>,
    pub terminals: Arc<dyn crate::host::TerminalPort>,
    pub permissions: Arc<dyn crate::host::PermissionPort>,
    pub events: Arc<dyn crate::host::EventPort>,
    /// Mira's tool server for this session, if the host runs one.
    pub mira_mcp: Option<crate::session::MiraMcp>,
}

#[derive(Debug, Error)]
pub enum StartError {
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    #[error(transparent)]
    Session(#[from] crate::session::SessionError),
    /// The transport was unusable before anything was spawned — the driver has
    /// no native interface, or the agent CLI would not start. Carried as a
    /// message rather than a `Spawn` because nothing failed to spawn that way.
    #[error("{0}")]
    Transport(String),
}

/// A running agent: process, session, and the host that answers it.
pub struct AcpAgent {
    process: Arc<AgentProcess>,
    session: Arc<crate::session::AcpSession>,
    /// Retained because the host owns the per-session tool-call state and
    /// the pending-permission tracker the session also reads.
    host: Arc<crate::host::AcpHost>,
}

impl Clone for AcpAgent {
    fn clone(&self) -> Self {
        AcpAgent {
            process: self.process.clone(),
            session: self.session.clone(),
            host: self.host.clone(),
        }
    }
}

impl std::fmt::Debug for AcpAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAgent")
            .field("session_id", &self.session_id())
            .finish_non_exhaustive()
    }
}

impl AcpAgent {
    pub fn session(&self) -> &Arc<crate::session::AcpSession> {
        &self.session
    }

    pub fn process(&self) -> &AgentProcess {
        &self.process
    }

    pub async fn stderr_tail(&self) -> String {
        self.process.stderr_tail().await
    }

    pub fn host(&self) -> &Arc<crate::host::AcpHost> {
        &self.host
    }

    /// The agent's own session id, once `session/new` has run.
    pub fn session_id(&self) -> Option<String> {
        self.session.session_id()
    }

    /// Run a prompt turn to completion.
    pub async fn prompt(
        &self,
        blocks: Vec<agent_client_protocol::schema::v1::ContentBlock>,
    ) -> Result<crate::session::PromptOutcome, crate::session::SessionError> {
        let sid = self.session_id().ok_or_else(|| {
            crate::session::SessionError::Other("no agent session yet".to_string())
        })?;
        let res = self.session.prompt(&sid, blocks).await?;
        Ok(crate::session::PromptOutcome {
            stop_reason: res.stop_reason,
            ended_normally: crate::session::ended_normally(&res),
            was_cancelled: crate::session::was_cancelled(&res),
        })
    }

    pub async fn cancel(&self) -> Result<(), crate::session::SessionError> {
        let sid = self.session_id().unwrap_or_default();
        self.session.cancel(&sid).await
    }

    /// Stop the process, answering any permission it is still blocked on.
    pub async fn shutdown(&self) {
        if let Err(e) = self.cancel().await {
            tracing::warn!(error = %e, "acp: cancel during shutdown failed");
        }
        self.process
            .shutdown(std::time::Duration::from_millis(500))
            .await;
    }
}

/// Resolve, spawn, initialize, and optionally open a session.
///
/// This is the one call a host needs. The ordering is enforced here rather
/// than left to callers, because two of the steps are coupled: the host must
/// exist before the process starts (it is the callback), and the session must
/// share the host's pending-permission tracker — otherwise `cancel` would
/// silently leave the agent blocked on a prompt nobody will ever answer.
pub async fn start(spec: StartSpec<'_>) -> Result<AcpAgent, StartError> {
    // Derived from the agent itself, so a driver that does not use
    // client-side terminals never gets told we offer them.
    let caps = spec.caps.unwrap_or_default();
    let host = crate::host::AcpHost::new(spec.files, spec.terminals, spec.permissions, spec.events);

    let process = AgentProcess::spawn(spec.launch, host.clone() as Arc<dyn AgentCallback>)
        .await
        .map_err(StartError::Spawn)?;

    let session = crate::session::AcpSession::new(
        process.conn().clone(),
        caps,
        host.pending_permissions.clone(),
    );

    // An agent that cannot initialize is not usable, so tear it down rather
    // than hand back a half-live pair.
    if let Err(e) = session.initialize().await {
        process
            .shutdown(std::time::Duration::from_millis(500))
            .await;
        return Err(StartError::Session(e));
    }
    session.set_mira_mcp(spec.mira_mcp.clone()).await;
    if let Some(cwd) = spec.cwd {
        if let Err(e) = session.new_session(cwd, Vec::new()).await {
            process
                .shutdown(std::time::Duration::from_millis(500))
                .await;
            return Err(StartError::Session(e));
        }
    }

    Ok(AcpAgent {
        process: Arc::new(process),
        session,
        host,
    })
}

#[cfg(test)]
mod tests {
    use crate::driver::{AcpDriver, DriverConfig, Transport};
    use crate::drivers::ClaudeCodeDriver;

    /// The reported failure: the panel said Claude Code was ready, "Use in
    /// this session" was pressed, and the error was `looked for
    /// claude-agent-acp`. `Auto` was choosing ACP because it tested the
    /// adapter's presence, which is absent in the one case native exists to
    /// serve. Selection must depend on the native CLI being present.
    #[test]
    fn auto_prefers_native_when_the_adapter_is_absent() {
        if crate::which::resolve("claude").is_none() {
            return; // not a machine with Claude Code installed
        }
        let cfg = DriverConfig::default();
        let chosen = choose_transport(
            &ClaudeCodeDriver,
            &cfg,
            std::path::Path::new("claude-agent-acp"),
        );
        assert_eq!(
            chosen,
            Transport::Native,
            "must not route to a missing adapter"
        );
    }

    /// The inverse: without a native interface nothing can be preferred,
    /// so `Auto` settles on the adapter deterministically — no CLI to find,
    /// no choice to make.
    #[test]
    fn auto_without_a_native_interface_stays_on_acp() {
        use crate::drivers::OpenCodeDriver;
        let d = OpenCodeDriver;
        assert!(d.native_flavor().is_none());
        let chosen = choose_transport(
            &d,
            &DriverConfig::default(),
            std::path::Path::new("opencode"),
        );
        assert_eq!(chosen, Transport::Acp);
    }

    use super::*;
    use serde_json::{json, Value};
    use std::sync::Mutex as StdMutex;

    /// An ACP agent written as a shell script, so the spawn path is
    /// exercised against a real child process rather than a mock.
    fn fake_agent_script(body: &str) -> String {
        format!(
            "#!/bin/sh\n\
             # Answer any request by echoing a canned result keyed off the\n\
             # method name, and mirror stdin so the test can watch it.\n\
             while IFS= read -r line; do\n\
             {body}\n\
             done\n"
        )
    }

    /// Poll a condition rather than sleeping a guessed interval: process
    /// startup under a loaded test binary is far slower than a fixed sleep
    /// assumes, and a too-short sleep fails intermittently.
    async fn wait_until<F: Fn() -> bool>(pred: F, what: &str) {
        for _ in 0..200 {
            if pred() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn launch(script: &str) -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sh");
        std::fs::write(&path, fake_agent_script(script)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            LaunchConfig {
                program: path.clone(),
                args: vec![],
                env: Default::default(),
                secret_env: vec![],
                env_deny: Vec::new(),
            },
            dir,
        )
    }

    /// Replies `{"result": <fixed>}` to the first request, then exits.
    const REPLY_AND_EXIT: &str = r#"
        case "$line" in
          *initialize*) printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"x","name":"X"}]}}\n' ;;
          *) printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"live"}}}}\n' ;;
        esac
    "#;

    #[tokio::test]
    async fn a_spawned_agent_can_be_initialized_over_its_real_stdio() {
        // The point of this test: everything from fork/exec through the
        // NDJSON pump, against an actual child process.
        let (cfg, _dir) = launch(REPLY_AND_EXIT);
        let proc = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
            .await
            .expect("spawn");
        let sess = crate::session::AcpSession::new(
            proc.conn().clone(),
            crate::session::ClientCaps::default(),
            crate::host::PendingPermissions::new(),
        );
        let res = sess.initialize().await.expect("initialize");
        assert_eq!(res.protocol_version.as_u16(), 1);
    }

    /// Denied prefixes must not reach the child, even when set explicitly on
    /// the launch. The script reports its environment on stderr (stdout is
    /// the protocol channel and must stay clean), and the parent reads it
    /// back through the tail.
    #[tokio::test]
    async fn denied_env_prefixes_never_reach_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\necho \"KEEP=${KEEP_SPY-unset} DENIED=${GOOGLE_SPY-unset}\" >&2\nsleep 30\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cfg = LaunchConfig {
            program: path,
            args: vec![],
            env: [
                ("KEEP_SPY".to_string(), "yes".to_string()),
                ("GOOGLE_SPY".to_string(), "yes".to_string()),
            ]
            .into_iter()
            .collect(),
            secret_env: vec![],
            env_deny: vec!["GOOGLE_".to_string()],
        };
        let proc = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
            .await
            .expect("spawn");
        // Poll the tail: stderr arrives on its own task, not with spawn. A
        // deadline rather than a count — under a loaded parallel test run,
        // starting a sandboxed shell can take seconds.
        let mut seen = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            seen = proc.stderr_tail().await;
            if seen.contains("KEEP=") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        proc.shutdown(std::time::Duration::from_millis(500)).await;
        assert!(seen.contains("KEEP=yes"), "allowed env must arrive: {seen}");
        assert!(
            seen.contains("DENIED=unset"),
            "denied prefixes must not arrive: {seen}"
        );
    }

    #[tokio::test]
    async fn notifications_from_a_real_process_reach_the_callback() {
        let (cfg, _dir) = launch(REPLY_AND_EXIT);
        #[derive(Default)]
        struct Watch {
            notes: StdMutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl AgentCallback for Watch {
            async fn on_request(
                &self,
                _id: &Value,
                _m: &str,
                _p: Value,
            ) -> Result<Value, ConnError> {
                Ok(json!({}))
            }
            async fn on_notification(&self, method: &str, _p: Value) {
                self.notes.lock().unwrap().push(method.to_string());
            }
        }
        let watch = Arc::new(Watch::default());
        let proc = AgentProcess::spawn(&cfg, watch.clone() as Arc<dyn AgentCallback>)
            .await
            .expect("spawn");
        // Nudge until the agent answers. A single nudge raced the child
        // process starting up, and the test failed intermittently on a
        // loaded machine; retrying is what the assertion actually means.
        // A generous budget: this only ever finishes in milliseconds, so a
        // long wait means the child never answered, and waiting longer just
        // turns a failure into a slow failure.
        for _ in 0..200 {
            if watch
                .notes
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "session/update")
            {
                break;
            }
            let _ = proc.conn().notify("test/nudge", json!({})).await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            watch
                .notes
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "session/update"),
            "no notification arrived: {:?}",
            watch.notes
        );
    }

    #[tokio::test]
    async fn a_missing_binary_is_reported_as_not_found() {
        let cfg = LaunchConfig {
            program: "/nonexistent/definitely-not-here".into(),
            args: vec![],
            env: Default::default(),
            secret_env: vec![],
            env_deny: Vec::new(),
        };
        let err = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
            .await
            .expect_err("should fail");
        assert!(matches!(err, SpawnError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn dropping_the_handle_kills_the_agent() {
        // An orphaned agent would keep running, and its children with it,
        // long after the session that owned it is gone.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("alive");
        let script = format!(
            "#!/bin/sh\n\
             trap '' TERM\n\
             while true; do printf x >> {0}; sleep 0.05; done\n",
            marker.display()
        );
        let path = dir.path().join("agent.sh");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cfg = LaunchConfig {
            program: path,
            args: vec![],
            env: Default::default(),
            secret_env: vec![],
            env_deny: Vec::new(),
        };
        {
            let proc = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
                .await
                .expect("spawn");
            let size = || std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
            wait_until(|| size() > 0, "the agent to start writing").await;
            // Confirm it is genuinely still alive and looping, not a one-shot.
            let first = size();
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            assert!(size() > first, "the agent exited early");
            let _ = proc.stderr_tail().await;
        }
        // After the handle drops, the process must stop for good.
        let size = || std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let after_kill = size();
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert_eq!(
            after_kill,
            size(),
            "the agent kept running after its handle was dropped"
        );
    }

    #[tokio::test]
    async fn an_agent_cannot_see_miras_credentials() {
        // The single most important property of the env allowlist: Mira's
        // own provider keys must not reach a third-party agent.
        std::env::set_var("MIRA_ACP_SECRET_PROBE", "leaked");
        let (cfg, _dir) =
            launch("case \"$line\" in *\"$MIRA_ACP_SECRET_PROBE\"*) : ;; esac; exit 0");
        let _ = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks)).await;
        std::env::remove_var("MIRA_ACP_SECRET_PROBE");
    }

    #[tokio::test]
    async fn the_resolved_env_reaches_the_agent() {
        // The driver's own credentials must survive the allowlist.
        let dir = tempfile::tempdir().unwrap();
        // The agent only answers if it can see the key *in its own
        // environment*; the request itself never mentions it.
        let script = concat!(
            "#!/bin/sh\n",
            "while IFS= read -r line; do\n",
            "  if [ -n \"$XAI_API_KEY\" ]; then\n",
            "    printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"sawKey\":true}}\\n'\n",
            "  fi\n",
            "done\n",
        );
        let path = dir.path().join("agent.sh");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut env = std::collections::BTreeMap::new();
        env.insert("XAI_API_KEY".to_string(), "sk-test".to_string());
        let cfg = LaunchConfig {
            program: path,
            args: vec![],
            env,
            secret_env: vec!["XAI_API_KEY".into()],
            env_deny: Vec::new(),
        };
        let proc = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
            .await
            .expect("spawn");
        // The agent can only respond if it found the key it was launched with.
        let got = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            proc.conn().request::<Value>("authenticate", json!({})),
        )
        .await
        .expect("agent did not answer")
        .expect("request");
        assert_eq!(got, json!({ "sawKey": true }));
    }

    #[tokio::test]
    async fn shutdown_stops_the_process() {
        let (cfg, _dir) = launch("while true; do sleep 0.05; done\n");
        let proc = AgentProcess::spawn(&cfg, Arc::new(crate::conn::NullCallbacks))
            .await
            .expect("spawn");
        proc.shutdown(std::time::Duration::from_millis(300)).await;
        // The connection notices via stdout EOF; allow a moment to observe it.
        for _ in 0..100 {
            if proc.has_exited().await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(proc.has_exited().await, "the process outlived shutdown");
    }

    #[test]
    fn a_launch_is_reported_as_installed_only_when_it_is() {
        let mut cfg = LaunchConfig {
            program: "definitely-not-a-real-binary-xyz".into(),
            args: vec![],
            env: Default::default(),
            secret_env: vec![],
            env_deny: Vec::new(),
        };
        assert!(
            !looks_installed(&cfg),
            "a bogus name must not look installed"
        );
        cfg.program = std::env::current_exe().unwrap();
        assert!(
            looks_installed(&cfg),
            "the current executable should be found"
        );
    }

    #[test]
    fn a_secret_in_the_resolved_env_is_redacted_for_logging() {
        // Guards the redaction path that Process::spawn's diagnostics rely on.
        let mut env = std::collections::BTreeMap::new();
        env.insert("XAI_API_KEY".to_string(), "sk-super-secret".to_string());
        let cfg = LaunchConfig {
            program: "/bin/grok".into(),
            args: vec!["agent".into(), "stdio".into()],
            env,
            secret_env: vec!["XAI_API_KEY".into()],
            env_deny: Vec::new(),
        };
        let rendered = cfg.redacted();
        assert!(!rendered.contains("sk-super-secret"), "leaked: {rendered}");
        assert!(rendered.contains("redacted"), "got {rendered}");
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    use crate::events::PermissionRequest;
    use crate::host::{DenyAll, EventPort, PermissionPort};
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    /// A full agent script: initialize, open a session, ask for permission,
    /// then report a stop reason. Exercises the whole stack against a real
    /// child process — the wiring that unit tests with in-memory pipes
    /// cannot cover is exactly what breaks between the layers.
    const FULL_AGENT: &str = r#"
case "$line" in
  *'"initialize"'*)
    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"a","name":"A"}]}}\n' "$id" ;;
  *'"session/new"'*)
    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"sess_x"}}\n' "$id" ;;
  *'"session/prompt"'*)
    # Remember which request we owe an answer to, then ask the client for
    # permission. The reply to the permission comes back under a different
    # id, so answering with that one would leave the prompt unsettled.
    pending="$id"
    printf '{"jsonrpc":"2.0","id":9001,"method":"session/request_permission","params":{"sessionId":"sess_x","toolCall":{"toolCallId":"t1","title":"Write file","kind":"edit"},"options":[{"optionId":"ok","name":"Allow","kind":"allow_once"}]}}\n'
    ;;
  *'9001'*)
    printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$pending" ;;
esac
"#;

    #[derive(Default)]
    struct Seen {
        prompts: StdMutex<Vec<PermissionRequest>>,
        allow: bool,
    }

    #[async_trait::async_trait]
    impl EventPort for Seen {
        async fn emit(&self, _e: crate::events::NormalizedEvent) {}
    }

    #[async_trait::async_trait]
    impl PermissionPort for Seen {
        async fn request_permission(
            &self,
            req: &crate::events::PermissionRequest,
        ) -> Result<Option<String>, crate::host::HostError> {
            self.prompts.lock().unwrap().push(req.clone());
            Ok(if self.allow {
                Some("ok".to_string())
            } else {
                None
            })
        }
    }

    fn script_agent(body: &str) -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sh");
        // Pull the id out of each request so replies correlate.
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 while IFS= read -r line; do\n\
                 id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\\([0-9]*\\).*/\\1/p')\n\
                 {body}\n\
                 done\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            LaunchConfig {
                program: path,
                args: vec![],
                env: Default::default(),
                secret_env: vec![],
                env_deny: Vec::new(),
            },
            dir,
        )
    }

    /// Every start-up test is bounded: a wiring bug that never settles a
    /// request would otherwise hang the whole suite with no diagnostic.
    async fn bounded<T, F: std::future::Future<Output = T>>(f: F) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(20), f)
            .await
            .expect("timed out: the agent and client failed to make progress")
    }

    #[tokio::test]
    async fn start_brings_up_an_initialized_session_with_a_real_process() {
        let (launch, _dir) = script_agent(FULL_AGENT);
        let seen = Arc::new(Seen {
            allow: true,
            ..Default::default()
        });
        let cwd = std::env::temp_dir();
        let agent = bounded(start(StartSpec {
            launch: &launch,
            caps: None,
            files: Arc::new(DenyAll),
            terminals: Arc::new(DenyAll),
            permissions: seen.clone() as Arc<dyn PermissionPort>,
            events: seen.clone() as Arc<dyn EventPort>,
            cwd: Some(&cwd),
            mira_mcp: None,
        }))
        .await
        .expect("start");

        assert_eq!(agent.session_id().as_deref(), Some("sess_x"));
        agent.shutdown().await;
    }

    #[tokio::test]
    async fn a_permission_request_reaches_the_host_and_the_turn_completes() {
        // The full round trip: agent asks, host projects the decision onto
        // the agent's own option id, turn finishes.
        let (launch, _dir) = script_agent(FULL_AGENT);
        let seen = Arc::new(Seen {
            allow: true,
            ..Default::default()
        });
        let cwd = std::env::temp_dir();
        let agent = bounded(start(StartSpec {
            launch: &launch,
            caps: None,
            files: Arc::new(DenyAll),
            terminals: Arc::new(DenyAll),
            permissions: seen.clone() as Arc<dyn PermissionPort>,
            events: seen.clone() as Arc<dyn EventPort>,
            cwd: Some(&cwd),
            mira_mcp: None,
        }))
        .await
        .expect("start");

        let outcome = bounded(agent.prompt(vec![serde_json::from_value(json!({
            "type": "text", "text": "do it"
        }))
        .expect("text block")]))
        .await
        .expect("prompt");

        assert!(outcome.ended_normally, "got {outcome:?}");
        let prompts = seen.prompts.lock().unwrap().clone();
        assert_eq!(prompts.len(), 1, "the host was never asked");
        assert_eq!(prompts[0].title, "Write file");
        assert_eq!(prompts[0].tool_call_id, "t1");
        // And it was settled, not left dangling.
        assert_eq!(agent.session().pending_permission_count().await, 0);
        agent.shutdown().await;
    }

    #[tokio::test]
    async fn a_declined_permission_still_lets_the_turn_end() {
        // `Cancelled` is not an error; the agent unwinds the turn itself.
        let (launch, _dir) = script_agent(FULL_AGENT);
        let seen = Arc::new(Seen {
            allow: false,
            ..Default::default()
        });
        let cwd = std::env::temp_dir();
        let agent = bounded(start(StartSpec {
            launch: &launch,
            caps: None,
            files: Arc::new(DenyAll),
            terminals: Arc::new(DenyAll),
            permissions: seen.clone() as Arc<dyn PermissionPort>,
            events: seen.clone() as Arc<dyn EventPort>,
            cwd: Some(&cwd),
            mira_mcp: None,
        }))
        .await
        .expect("start");
        let outcome = bounded(agent.prompt(vec![serde_json::from_value(json!({
            "type": "text", "text": "go"
        }))
        .expect("text block")]))
        .await
        .expect("prompt");
        assert!(outcome.ended_normally, "got {outcome:?}");
        assert_eq!(seen.prompts.lock().unwrap().len(), 1);
        agent.shutdown().await;
    }

    #[tokio::test]
    async fn an_agent_that_fails_to_initialize_is_torn_down() {
        // No half-live agent is handed back: a process that cannot
        // initialize is not usable, and leaving it running would leak it.
        let (launch, _dir) = script_agent("case \"$line\" in *'\"initialize\"'*) exit 1 ;; esac\n");
        let seen = Arc::new(Seen::default());
        let res = bounded(start(StartSpec {
            launch: &launch,
            caps: None,
            files: Arc::new(DenyAll),
            terminals: Arc::new(DenyAll),
            permissions: seen.clone() as Arc<dyn PermissionPort>,
            events: seen.clone() as Arc<dyn EventPort>,
            cwd: None,
            mira_mcp: None,
        }))
        .await;
        assert!(res.is_err(), "expected a start failure");
        assert!(matches!(res, Err(StartError::Session(_))), "got {res:?}");
    }

    #[tokio::test]
    async fn a_missing_agent_binary_fails_before_anything_else_happens() {
        let seen = Arc::new(Seen::default());
        let launch = LaunchConfig {
            program: "/nonexistent/grok".into(),
            args: vec!["agent".into(), "stdio".into()],
            env: Default::default(),
            secret_env: vec![],
            env_deny: Vec::new(),
        };
        let res = bounded(start(StartSpec {
            launch: &launch,
            caps: None,
            files: Arc::new(DenyAll),
            terminals: Arc::new(DenyAll),
            permissions: seen.clone() as Arc<dyn PermissionPort>,
            events: seen.clone() as Arc<dyn EventPort>,
            cwd: None,
            mira_mcp: None,
        }))
        .await;
        assert!(
            matches!(res, Err(StartError::Spawn(SpawnError::NotFound(_)))),
            "got {res:?}"
        );
    }
}

/// Start an agent over whichever transport the driver resolves to.
///
/// The native path takes a permission gate instead of host ports: an agent
/// driven through its own CLI has no ACP filesystem or terminal ports to bind,
/// and the only thing it asks the host for is a decision.
#[allow(clippy::too_many_arguments)] // one per transport input; each is distinct
pub async fn start_agent(
    driver: &dyn crate::driver::AcpDriver,
    cfg: &crate::driver::DriverConfig,
    mode: crate::driver::PermissionMode,
    program: &Path,
    ports: HostPorts,
    cwd: Option<&Path>,
    gate: crate::native::PermissionGate,
    native: Option<crate::native::NativeOverrides>,
) -> Result<crate::native::AgentHandle, StartError> {
    match choose_transport(driver, cfg, program) {
        crate::driver::Transport::Native => {
            let Some(flavor) = driver.native_flavor() else {
                return Err(StartError::Transport(
                    "this agent has no native interface".to_string(),
                ));
            };
            match flavor {
                crate::native::NativeFlavor::Claude => {
                    let mut launch =
                        build_native_launch(driver, cfg, mode, program, native.as_ref())
                            .expect("flavor implies a launch");
                    // Stage prompt attachments where the CLI is allowed to
                    // read. Without `--add-dir` the agent could be told
                    // filenames it is forbidden from opening — a prompt that
                    // names files it cannot read.
                    let attachments_dir = cwd.map(|c| c.join(".mira/agent-files"));
                    if let Some(dir) = attachments_dir.as_ref() {
                        let _ = std::fs::create_dir_all(dir);
                        if !launch.args.iter().any(|a| a == "--add-dir") {
                            launch.args.push("--add-dir".to_string());
                            launch.args.push(dir.display().to_string());
                        }
                    }
                    let agent = crate::native::NativeAgent::start(&launch, gate, attachments_dir)
                        .await
                        .map_err(|e| StartError::Transport(e.to_string()))?;
                    Ok(crate::native::AgentHandle::Native(Arc::new(agent)))
                }
                crate::native::NativeFlavor::CodexAppServer => {
                    let prog = native_program(driver, cfg, program);
                    let launch = crate::appserver::launch(prog);
                    // The runtime mode id travels verbatim: the policy pair
                    // is derived inside, and the Modes event reports the id
                    // back so the picker and the process agree.
                    let runtime_mode = native
                        .as_ref()
                        .and_then(|o| o.permission_mode.clone())
                        .unwrap_or_else(|| crate::appserver::mode_for_permission(mode).to_string());
                    let model = native.as_ref().and_then(|o| o.model.clone());
                    let resume = native.as_ref().and_then(|o| o.resume.clone());
                    let cwd = cwd.map(|p| p.display().to_string());
                    let agent = crate::appserver::AppServerAgent::start(
                        &launch,
                        &runtime_mode,
                        cwd,
                        model,
                        resume,
                        gate,
                        // Mira's tools (browser, processes) for this chat.
                        ports.mira_mcp.clone(),
                    )
                    .await
                    .map_err(|e| StartError::Transport(e.to_string()))?;
                    Ok(crate::native::AgentHandle::AppServer(Arc::new(agent)))
                }
            }
        }
        crate::driver::Transport::Acp | crate::driver::Transport::Auto => {
            let agent = start_driver(driver, cfg, mode, program, ports, cwd).await?;
            Ok(crate::native::AgentHandle::Acp(agent))
        }
    }
}

/// Build the launch for a native transport: program, driver argv, extras,
/// user args and overrides, in that order.
///
/// ONE function serves the pre-flight check and the spawn. They previously
/// built argv separately and drifted — the check validated one command while
/// the spawn ran another.
pub fn build_native_launch(
    driver: &dyn crate::driver::AcpDriver,
    cfg: &crate::driver::DriverConfig,
    mode: crate::driver::PermissionMode,
    program: &Path,
    native: Option<&crate::native::NativeOverrides>,
) -> Option<LaunchConfig> {
    let flavor = driver.native_flavor()?;
    let prog = native_program(driver, cfg, program);
    // The driver's resolved env (home/api_key/user env) belongs on every
    // native launch. It was previously merged only by the pre-flight check,
    // so the spawn silently dropped user env, API keys and home isolation —
    // the check and the spawn disagreed again, same bug class as before.
    let resolved = driver.resolve(cfg, mode, program.to_path_buf());
    let mut launch = match flavor {
        crate::native::NativeFlavor::Claude => {
            let mut launch = crate::native::launch(prog, mode, None, None);
            launch.args.extend(driver.cli_extras(cfg));
            if let Some(o) = native {
                crate::native::apply_overrides(&mut launch, o);
            }
            launch
        }
        crate::native::NativeFlavor::CodexAppServer => crate::appserver::launch(prog),
    };
    launch.env = resolved.env;
    launch.secret_env = resolved.secret_env;
    launch.args.extend(cfg.launch_args.clone());
    Some(launch)
}

/// The binary to run natively.
///
/// The ACP `program` is the adapter; the native path wants the agent's own
/// CLI, which is a different name entirely. A user-set binary path is
/// respected here too, since that is the whole point of setting it.
pub fn native_program(
    driver: &dyn crate::driver::AcpDriver,
    cfg: &crate::driver::DriverConfig,
    program: &Path,
) -> PathBuf {
    if let Some(p) = &cfg.binary_path {
        // Only honour it when the user pointed at the agent rather than at an
        // adapter. An explicit adapter path is not a native binary.
        if !driver
            .binary_names()
            .iter()
            .any(|n| p.to_string_lossy() == *n)
        {
            return p.clone();
        }
    }
    driver
        .underlying_cli_names()
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| program.to_path_buf())
}
