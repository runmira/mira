
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;

use crate::{detect_backend, SandboxBackend, SandboxProfile};

/// Runtime environment made available to sandboxed developer commands.
///
/// The sandbox owns command discovery through PATH. Individual tools
/// (grep, git, cargo, etc.) should simply request their executable by name
/// rather than independently checking whether it exists.
fn sandbox_path() -> String {
    let mut paths = Vec::<PathBuf>::new();

    let mut add = |path: PathBuf| {
        if path.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    };

    #[cfg(target_os = "macos")]
    {
        // Homebrew on Apple Silicon.
        add(PathBuf::from("/opt/homebrew/bin"));
        add(PathBuf::from("/opt/homebrew/sbin"));

        // Homebrew / developer tools on Intel macOS.
        add(PathBuf::from("/usr/local/bin"));
        add(PathBuf::from("/usr/local/sbin"));
    }

    #[cfg(target_os = "linux")]
    {
        add(PathBuf::from("/usr/local/bin"));
        add(PathBuf::from("/usr/bin"));
        add(PathBuf::from("/bin"));
        add(PathBuf::from("/usr/local/sbin"));
        add(PathBuf::from("/usr/sbin"));
        add(PathBuf::from("/sbin"));
    }

    // Preserve useful paths from the environment that launched Mira.
    //
    // This is important for developer toolchains such as:
    //   ~/.cargo/bin
    //   ~/.local/bin
    //   custom language runtimes
    //   user-installed developer tools
    if let Some(existing) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&existing) {
            add(path);
        }
    }

    std::env::join_paths(paths)
        .expect("failed to construct sandbox PATH")
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub profile: SandboxProfile,
    pub env: Vec<(String, String)>,
    pub unset_env: Vec<String>,
}

impl SandboxConfig {
    pub fn new(profile: SandboxProfile) -> Self {
        Self {
            profile,
            env: Vec::new(),
            unset_env: Vec::new(),
        }
    }

    pub fn env(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn unset(mut self, key: impl Into<String>) -> Self {
        self.unset_env.push(key.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

impl CommandOutput {
    /// Returns stdout and stderr as one string suitable for tool output.
    ///
    /// When both streams contain output, stderr is appended after stdout
    /// with a newline separator.
    pub fn combined_output(&self) -> String {
        match (self.stdout.is_empty(), self.stderr.is_empty()) {
            (true, true) => String::new(),

            (false, true) => self.stdout.clone(),

            (true, false) => self.stderr.clone(),

            (false, false) => {
                let mut output =
                    String::with_capacity(
                        self.stdout.len()
                            + self.stderr.len()
                            + 1,
                    );

                output.push_str(&self.stdout);
                output.push('\n');
                output.push_str(&self.stderr);

                output
            }
        }
    }

    /// Returns true when the command exited successfully and did not time out.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
const READER_GRACE: Duration = Duration::from_secs(5);

const DROPPED_ENV: &[&str] = &[
    "API_KEY",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITLAB_TOKEN",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPEN_ROUTER_API_KEY",
    "ASTER_API_KEY",
    "ASTER_SESSION",
];

const INHERITED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "PWD",

    // macOS.
    "TMP",
    "TEMP",

    // Windows compatibility for callers that may use this crate elsewhere.
    "SystemRoot",
    "SystemDrive",
    "ComSpec",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PATHEXT",
];

pub async fn run_command(
    config: &SandboxConfig,
    binary: &str,
    args: &[String],
    cwd: &Path,
) -> Result<CommandOutput> {
    let backend = detect_backend();

    tracing::debug!(
        backend = ?backend,
        binary = binary,
        cwd = %cwd.display(),
        "executing sandbox command"
    );

    run_with_backend(
        backend,
        config,
        binary,
        args,
        cwd,
    )
    .await
}

async fn run_with_backend(
    backend: SandboxBackend,
    config: &SandboxConfig,
    binary: &str,
    args: &[String],
    cwd: &Path,
) -> Result<CommandOutput> {
    let timeout = Duration::from_secs(
        config.profile.timeout_secs,
    );

    let mut command = match backend {
        #[cfg(target_os = "macos")]
        SandboxBackend::Seatbelt => {
            build_seatbelt_command(
                config,
                binary,
                args,
            )?
        }

        #[cfg(target_os = "linux")]
        SandboxBackend::Bubblewrap => {
            build_bwrap_command(
                config,
                binary,
                args,
            )?
        }

        SandboxBackend::ProcessLevel => {
            tracing::warn!(
                "no OS sandbox available; running with process-level isolation only"
            );

            build_process_command(
                binary,
                args,
            )
        }

        #[cfg(target_os = "macos")]
        SandboxBackend::Bubblewrap => {
            tracing::warn!(
                "bubblewrap requested on macOS; falling back to process-level execution"
            );

            build_process_command(
                binary,
                args,
            )
        }

        #[cfg(target_os = "linux")]
        SandboxBackend::Seatbelt => {
            tracing::warn!(
                "Seatbelt requested on Linux; falling back to process-level execution"
            );

            build_process_command(
                binary,
                args,
            )
        }
    };

    // `repo_root` remains the security boundary.
    //
    // `cwd` controls where the process starts. Callers are responsible for
    // validating that cwd is inside the repository before reaching here.
    command.current_dir(cwd);

    configure_environment(
        &mut command,
        config,
        backend,
    );

    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .context("spawning sandboxed command")?;

    let mut process_group =
        ProcessGroupKill(child.id());

    let stdout_buffer = captured();
    let stderr_buffer = captured();

    let stdout_task = tokio::spawn(
        read_capped(
            child.stdout.take(),
            stdout_buffer.clone(),
        ),
    );

    let stderr_task = tokio::spawn(
        read_capped(
            child.stderr.take(),
            stderr_buffer.clone(),
        ),
    );

    let wait_result = tokio::time::timeout(
        timeout,
        child.wait(),
    )
    .await;

    let timed_out = wait_result.is_err();

    if timed_out {
        process_group.kill();
    } else {
        process_group.disarm();
    }

    let readers = async {
        let _ = tokio::join!(
            stdout_task,
            stderr_task,
        );
    };

    let _ = tokio::time::timeout(
        READER_GRACE,
        readers,
    )
    .await;

    let stdout = snapshot(
        &stdout_buffer,
    );

    let stderr = snapshot(
        &stderr_buffer,
    );

    if timed_out {
        return Ok(CommandOutput {
            stdout,
            stderr,
            exit_code: None,
            timed_out: true,
        });
    }

    let status = wait_result
        .expect("timeout result already checked")
        .context("waiting for sandboxed command")?;

    Ok(CommandOutput {
        stdout,
        stderr,
        exit_code: status.code(),
        timed_out: false,
    })
}

/// Explicit escape hatch for callers that intentionally need an unsandboxed
/// process.
///
/// This should not be used for model-generated commands.
pub async fn run_unsandboxed(
    repo_root: &Path,
    binary: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<CommandOutput> {
    let profile = SandboxProfile::new(repo_root)
        .timeout(timeout_secs);

    let config = SandboxConfig::new(profile);

    run_with_backend(
        SandboxBackend::ProcessLevel,
        &config,
        binary,
        args,
        repo_root,
    )
    .await
}

fn configure_environment(
    command: &mut TokioCommand,
    config: &SandboxConfig,
    backend: SandboxBackend,
) {
    if backend == SandboxBackend::ProcessLevel {
        // Explicit process-level mode intentionally preserves the caller's
        // environment, except for credentials that Mira should never leak.
        for key in DROPPED_ENV {
            command.env_remove(key);
        }

        for key in &config.unset_env {
            command.env_remove(key);
        }

        for (key, value) in &config.env {
            command.env(key, value);
        }

        return;
    }

    // OS sandboxed commands receive a clean environment.
    command.env_clear();

    // ------------------------------------------------------------------
    // Deterministic developer PATH
    // ------------------------------------------------------------------
    //
    // Do this explicitly rather than trusting the PATH inherited by the
    // process that launched Mira.
    //
    // This fixes the common macOS situation where:
    //
    //   Terminal PATH != Xcode/Finder/.app PATH
    //
    // and therefore tools such as rg, brew-installed binaries, etc. appear
    // to be "missing" from Mira.
    command.env(
        "PATH",
        sandbox_path(),
    );

    // ------------------------------------------------------------------
    // Safe inherited environment
    // ------------------------------------------------------------------

    for key in INHERITED_ENV {
        if key == &"PATH" {
            // PATH was already set explicitly above.
            continue;
        }

        if config.unset_env.iter().any(|value| value == key) {
            continue;
        }

        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }

    // If a sandbox-specific HOME exists, use it.
    if let Some(home) = &config.profile.home_dir {
        command.env(
            "HOME",
            home.display().to_string(),
        );
    }

    // If a sandbox-specific temporary directory exists, use it.
    if let Some(temp) = &config.profile.temp_dir {
        command.env(
            "TMPDIR",
            temp.display().to_string(),
        );

        command.env(
            "TMP",
            temp.display().to_string(),
        );

        command.env(
            "TEMP",
            temp.display().to_string(),
        );
    }

    // Explicit caller-provided environment overrides defaults.
    for (key, value) in &config.env {
        command.env(key, value);
    }

    // Credentials are never inherited, even if they happen to be included
    // in the caller's environment.
    for key in DROPPED_ENV {
        command.env_remove(key);
    }

    for key in &config.unset_env {
        command.env_remove(key);
    }
}

#[cfg(target_os = "macos")]
fn build_seatbelt_command(
    config: &SandboxConfig,
    binary: &str,
    args: &[String],
) -> Result<TokioCommand> {
    let profile = config.profile.seatbelt_profile();

    let mut command = TokioCommand::new(
        "sandbox-exec",
    );

    command
        .arg("-p")
        .arg(profile)
        .arg("--")
        .arg(binary);

    for arg in args {
        command.arg(arg);
    }

    Ok(command)
}

#[cfg(target_os = "linux")]
fn build_bwrap_command(
    config: &SandboxConfig,
    binary: &str,
    args: &[String],
) -> Result<TokioCommand> {
    let bwrap_args = config.profile.bwrap_args();

    let mut command = TokioCommand::new(
        "bwrap",
    );

    for arg in &bwrap_args {
        command.arg(arg);
    }

    command
        .arg("--")
        .arg(binary);

    for arg in args {
        command.arg(arg);
    }

    Ok(command)
}

fn build_process_command(
    binary: &str,
    args: &[String],
) -> TokioCommand {
    let mut command = TokioCommand::new(
        binary,
    );

    for arg in args {
        command.arg(arg);
    }

    command
}

type Captured = Arc<Mutex<(Vec<u8>, bool)>>;

fn captured() -> Captured {
    Arc::new(
        Mutex::new((
            Vec::with_capacity(8192),
            false,
        )),
    )
}

async fn read_capped<R>(
    stream: Option<R>,
    buffer: Captured,
)
where
    R: AsyncRead + Unpin,
{
    let Some(mut stream) = stream else {
        return;
    };

    let mut chunk = [0u8; 8192];

    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        let Ok(mut state) = buffer.lock() else {
            break;
        };

        if state.0.len() >= MAX_CAPTURE_BYTES {
            state.1 = true;
            continue;
        }

        let remaining =
            MAX_CAPTURE_BYTES - state.0.len();

        let amount = read.min(remaining);

        state.0.extend_from_slice(
            &chunk[..amount],
        );

        if amount < read {
            state.1 = true;
        }
    }
}

fn snapshot(buffer: &Captured) -> String {
    let Ok(state) = buffer.lock() else {
        return String::new();
    };

    String::from_utf8_lossy(&state.0)
        .into_owned()
}

struct ProcessGroupKill(Option<u32>);

impl ProcessGroupKill {
    fn kill(&mut self) {
        #[cfg(unix)]
        {
            if let Some(pid) = self.0.take() {
                let process_group =
                    nix::unistd::Pid::from_raw(
                        -(pid as i32),
                    );

                let _ = nix::sys::signal::kill(
                    process_group,
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }

        #[cfg(not(unix))]
        {
            self.0.take();
        }
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessGroupKill {
    fn drop(&mut self) {
        if self.0.is_some() {
            self.kill();
        }
    }
}

