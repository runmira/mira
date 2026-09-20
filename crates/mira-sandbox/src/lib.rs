#![forbid(unsafe_code)]

use std::sync::{Arc, RwLock};

mod profile;
mod runner;

pub use profile::{command_name, credential_paths, credentials_for, SandboxProfile};

pub use runner::{run_command, run_unsandboxed, CommandOutput, SandboxConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    Seatbelt,
    Bubblewrap,
    ProcessLevel,
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    profile: Arc<RwLock<SandboxProfile>>,
}

impl Sandbox {
    /// Create a sandbox rooted at `repo_root`.
    ///
    /// The repository root is the security boundary for commands
    /// executed through this sandbox.
    pub fn new(repo_root: impl Into<std::path::PathBuf>) -> Self {
        let repo_root = repo_root.into();

        Self {
            profile: Arc::new(RwLock::new(SandboxProfile::new(&repo_root))),
        }
    }

    /// Create a sandbox from a fully configured profile.
    ///
    /// Use this when the caller needs more than the defaults from
    /// [`Sandbox::new`] — network access, extra readable/writable
    /// directories (toolchains, caches), a custom HOME or temp dir, or
    /// approved credentials.
    pub fn with_profile(profile: SandboxProfile) -> Self {
        Self {
            profile: Arc::new(RwLock::new(profile)),
        }
    }

    /// Create a sandbox intended for a normal Mira workspace.
    pub fn for_workspace(repo_root: impl Into<std::path::PathBuf>) -> Self {
        Self::new(repo_root)
    }

    /// Compatibility constructor used by tests and headless contexts.
    pub fn default_scrubbed() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        Self::new(cwd)
    }

    pub fn profile(&self) -> SandboxProfile {
        self.profile
            .read()
            .expect("sandbox profile lock poisoned")
            .clone()
    }

    pub fn set_profile(&self, profile: SandboxProfile) {
        *self
            .profile
            .write()
            .expect("sandbox profile lock poisoned") = profile;
    }

    pub fn backend(&self) -> SandboxBackend {
        detect_backend()
    }

    pub fn has_os_sandbox(&self) -> bool {
        has_os_sandbox()
    }

    /// Run a command inside the sandbox.
    ///
    /// `cwd` controls the process working directory.
    /// `self.profile.repo_root` remains the security boundary.
    pub async fn run(
        &self,
        binary: &str,
        args: &[String],
        cwd: &std::path::Path,
    ) -> anyhow::Result<CommandOutput> {
        self.run_with_timeout(binary, args, cwd, self.profile().timeout_secs)
            .await
    }

    /// Run a command with a custom timeout.
    pub async fn run_with_timeout(
        &self,
        binary: &str,
        args: &[String],
        cwd: &std::path::Path,
        timeout_secs: u64,
    ) -> anyhow::Result<CommandOutput> {
        let profile = self.profile().timeout(timeout_secs);

        let config = SandboxConfig::new(profile);

        run_command(&config, binary, args, cwd).await
    }
}

pub fn detect_backend() -> SandboxBackend {
    #[cfg(target_os = "macos")]
    {
        if binary_on_path("sandbox-exec") {
            return SandboxBackend::Seatbelt;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if binary_on_path("bwrap") {
            return SandboxBackend::Bubblewrap;
        }
    }

    SandboxBackend::ProcessLevel
}

pub fn has_os_sandbox() -> bool {
    !matches!(detect_backend(), SandboxBackend::ProcessLevel)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn binary_on_path(binary: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_profile_keeps_the_supplied_profile() {
        let profile = SandboxProfile::new(std::path::Path::new("/tmp/repo"))
            .network(true)
            .timeout(7);
        let sandbox = Sandbox::with_profile(profile);

        assert!(sandbox.profile().network);
        assert_eq!(sandbox.profile().timeout_secs, 7);
        assert_eq!(
            sandbox.profile().repo_root,
            std::path::PathBuf::from("/tmp/repo")
        );
    }

    #[test]
    fn new_defaults_to_offline() {
        let sandbox = Sandbox::new("/tmp/repo");
        assert!(!sandbox.profile().network);
    }
}