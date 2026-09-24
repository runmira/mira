//! Where Mira's tools execute.
//!
//! By default every tool runs on the user's machine, straight through
//! `mira-sandbox` and the local filesystem; that path doesn't touch this
//! crate at all. With `--sandbox <backend>` the session instead gets a
//! [`ComputeBackend`], and the file and command tools route through it:
//!
//! - [`LocalBackend`] runs against a directory on this machine. The
//!   `scratch` environment points it at a copy of the repo, so the agent
//!   can't touch the real checkout.
//! - [`E2bBackend`] runs in an E2B Firecracker microVM.
//!
//! Either way the lifecycle is the same (see [`workspace`]): pack the
//! project, upload it, work in the sandbox, then pull the result back
//! as a patch the user applies. Nothing reaches the local checkout until
//! then.
//!
//! See `docs/remote-compute.md` for the background and provider choice.

use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;

pub mod e2b;
pub mod env;
pub mod local;
pub mod path;
pub mod workspace;

pub use e2b::{E2bBackend, E2bOptions};
pub use env::{ComputeSlot, EnvironmentManager, EnvironmentSpec, EnvironmentStatus, SwitchReport};
pub use local::LocalBackend;

#[derive(Debug, Error)]
pub enum ComputeError {
    #[error("{0}")]
    NotFound(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("{0} is not supported by this backend")]
    Unsupported(&'static str),
    #[error("configuration: {0}")]
    Config(String),
    #[error("{0}")]
    Remote(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ComputeError>;

/// A shell command to run in the workspace.
#[derive(Clone, Debug)]
pub struct ExecRequest {
    /// Passed to `bash -lc`, as the local bash tool does.
    pub command: String,
    /// Working directory relative to the workspace root (`""` = root).
    pub cwd: String,
    pub timeout: Duration,
}

impl ExecRequest {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: String::new(),
            timeout: Duration::from_secs(300),
        }
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Live output while a command runs. Feeds the TUI's tool tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputeEvent {
    Stdout(String),
    Stderr(String),
}

#[derive(Clone, Debug, Default)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    /// `None` when the process was killed (timeout) or never reported one.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// The execution substrate for one session's tools.
///
/// Paths are relative to the workspace root and already validated by
/// [`path::resolve`]; backends map them onto wherever the workspace
/// really lives. There's no `patch_file`: edits are read-modify-write
/// through [`Self::read_file`] and [`Self::write_file`], which keeps
/// every backend small and the edit semantics identical to local.
#[async_trait]
pub trait ComputeBackend: Send + Sync {
    /// Short name for logs and the system prompt (`local`, `e2b`).
    fn name(&self) -> &'static str;

    /// Absolute path of the workspace root inside the backend, shown to
    /// the model so absolute paths in command output make sense.
    fn workspace_root(&self) -> String;

    /// Run a command, forwarding output to `sink` as it arrives when the
    /// backend can stream (the full output is returned either way).
    async fn exec(
        &self,
        req: ExecRequest,
        sink: Option<mpsc::Sender<ComputeEvent>>,
    ) -> Result<ExecOutput>;

    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;

    /// Write a file, creating parent directories.
    async fn write_file(&self, path: &str, data: &[u8]) -> Result<()>;

    /// Persist state for a later resume; returns an opaque id.
    async fn checkpoint(&self) -> Result<String> {
        Err(ComputeError::Unsupported("checkpoint"))
    }

    /// Suspend (free compute, keep storage).
    async fn pause(&self) -> Result<()> {
        Err(ComputeError::Unsupported("pause"))
    }

    /// Release the backend's resources. Idempotent.
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
