// Only the Landlock module needs `unsafe` (raw syscalls).
#![deny(unsafe_code)]

use std::sync::{Arc, RwLock};

#[cfg(target_os = "linux")]
mod landlock;
mod profile;
mod runner;

pub use profile::{command_name, credential_paths, credentials_for, SandboxProfile};

pub use runner::{run_command, run_unsandboxed, CommandOutput, SandboxConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    Seatbelt,
    Bubblewrap,
    /// Linux Landlock: used when bubblewrap is missing or can't run.
    Landlock,
    ProcessLevel,
}

impl SandboxBackend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Seatbelt => "sandbox-exec",
            Self::Bubblewrap => "bubblewrap",
            Self::Landlock => "landlock",
            Self::ProcessLevel => "none",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "seatbelt" | "sandbox-exec" => Self::Seatbelt,
            "bwrap" | "bubblewrap" => Self::Bubblewrap,
            "landlock" => Self::Landlock,
            "none" | "process" | "off" => Self::ProcessLevel,
            _ => return None,
        })
    }
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
        *self.profile.write().expect("sandbox profile lock poisoned") = profile;
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

/// The backend commands run under. `MIRA_SANDBOX_BACKEND` (`bwrap`,
/// `landlock`, `seatbelt` or `none`) overrides the choice; otherwise the
/// strongest one that works here wins. Probed once per process.
pub fn detect_backend() -> SandboxBackend {
    if let Some(forced) = std::env::var("MIRA_SANDBOX_BACKEND")
        .ok()
        .and_then(|v| SandboxBackend::parse(&v))
    {
        return forced;
    }
    static DETECTED: std::sync::OnceLock<SandboxBackend> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(probe_backend)
}

fn probe_backend() -> SandboxBackend {
    #[cfg(target_os = "macos")]
    {
        if binary_on_path("sandbox-exec") {
            return SandboxBackend::Seatbelt;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if binary_on_path("bwrap") {
            if bwrap_works() {
                return SandboxBackend::Bubblewrap;
            }
            tracing::warn!("bubblewrap can't create namespaces here; trying Landlock");
        }
        if landlock::available() {
            return SandboxBackend::Landlock;
        }
    }

    SandboxBackend::ProcessLevel
}

/// bwrap is often installed but unusable: containers without user
/// namespaces, or AppArmor restricting them (Ubuntu 24.04+).
#[cfg(target_os = "linux")]
fn bwrap_works() -> bool {
    std::process::Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--unshare-net",
            "--die-with-parent",
            "true",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
