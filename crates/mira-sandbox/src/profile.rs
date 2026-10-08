use std::path::{Path, PathBuf};

/// Filesystem/network capabilities granted to a sandboxed command.
///
/// The profile is intentionally tool-agnostic. Cargo, xcodebuild, npm,
/// swift, python, go, etc. are all treated as ordinary processes that need
/// filesystem/network capabilities.
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Repository/workspace root.
    pub repo_root: PathBuf,

    /// Additional directories that commands may read.
    pub readable_dirs: Vec<PathBuf>,

    /// Additional directories that commands may modify.
    pub writable_dirs: Vec<PathBuf>,

    /// Whether network access is permitted.
    pub network: bool,

    /// Maximum command lifetime.
    pub timeout_secs: u64,

    /// Credential directories explicitly approved for this command.
    pub allowed_credentials: Vec<PathBuf>,

    /// Optional HOME visible to the command.
    ///
    /// If unset, the host HOME is inherited.
    pub home_dir: Option<PathBuf>,

    /// Optional command-local temporary directory.
    pub temp_dir: Option<PathBuf>,
}

impl SandboxProfile {
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            readable_dirs: Vec::new(),
            writable_dirs: Vec::new(),
            network: false,
            timeout_secs: 30,
            allowed_credentials: Vec::new(),
            home_dir: None,
            temp_dir: None,
        }
    }

    pub fn readable(mut self, dir: impl Into<PathBuf>) -> Self {
        self.readable_dirs.push(dir.into());
        self
    }

    pub fn writable(mut self, dir: impl Into<PathBuf>) -> Self {
        self.writable_dirs.push(dir.into());
        self
    }

    pub fn network(mut self, allow: bool) -> Self {
        self.network = allow;
        self
    }

    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn home(mut self, dir: impl Into<PathBuf>) -> Self {
        self.home_dir = Some(dir.into());
        self
    }

    pub fn temp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(dir.into());
        self
    }

    /// Allow only known credential directories.
    ///
    /// We intentionally do not accept arbitrary paths here. Credentials
    /// should be granted through a small, explicit allow-list.
    pub fn allow_credentials(mut self, dirs: Vec<PathBuf>) -> Self {
        let known = credential_paths();

        self.allowed_credentials = resolve_existing(dirs)
            .into_iter()
            .filter(|dir| known.contains(dir))
            .collect();

        self
    }

    /// Generate a macOS Seatbelt profile.
    ///
    /// The default policy is:
    /// - normal reads are allowed
    /// - writes are denied by default
    /// - repository + explicitly writable directories are writable
    /// - sensitive credential/keychain locations are denied
    /// - network is denied unless explicitly enabled
    #[cfg(target_os = "macos")]
    pub fn seatbelt_profile(&self) -> String {
        let mut profile = String::new();

        profile.push_str("(version 1)\n");
        profile.push_str("(allow default)\n");

        // ------------------------------------------------------------------
        // Filesystem write boundary
        // ------------------------------------------------------------------

        profile.push_str("(deny file-write*)\n");

        // Processes need normal device writes.
        profile.push_str(
            "(allow file-write-data \
             (require-all \
             (path \"/dev/null\") \
             (vnode-type CHARACTER-DEVICE)))\n",
        );

        profile.push_str("(allow file-write* (subpath \"/dev\"))\n");

        let writable = self.writable_paths();

        if !writable.is_empty() {
            profile.push_str("(allow file-write*");

            for path in &writable {
                profile.push_str(&format!(" (subpath \"{}\")", escape(path)));
            }

            profile.push_str(")\n");
        }

        // ------------------------------------------------------------------
        // Protected repository paths
        // ------------------------------------------------------------------
        //
        // Even though the repository itself is writable, these paths remain
        // protected so an agent cannot silently alter git hooks, workflows,
        // or repository configuration.
        let protected = self.protected_repo_paths();

        if !protected.is_empty() {
            profile.push_str("(deny file-write*");

            for path in &protected {
                profile.push_str(&format!(" (subpath \"{}\")", escape(path)));
            }

            profile.push_str(")\n");
        }

        // ------------------------------------------------------------------
        // Sensitive host data
        // ------------------------------------------------------------------

        let denied = hard_denied()
            .into_iter()
            .chain(credential_paths())
            .filter(|path| !self.allowed_credentials.contains(path))
            .collect::<Vec<_>>();

        if !denied.is_empty() {
            profile.push_str("(deny file-read* file-write*");

            for path in &denied {
                profile.push_str(&format!(" (subpath \"{}\")", escape(path)));
            }

            profile.push_str(")\n");
        }

        // Keychain files are denied above, but `(allow default)` still
        // lets a command ask the keychain daemons for secrets, e.g.
        // `git credential-osxkeychain get` returns the saved GitHub
        // token. Close that door too. (Code signing needs the keychain,
        // so signed Xcode builds have to run outside the sandbox.)
        profile.push_str(
            "(deny mach-lookup \
             (global-name \"com.apple.SecurityServer\") \
             (global-name \"com.apple.securityd\") \
             (global-name \"com.apple.securityd.xpc\") \
             (global-name \"com.apple.secd\"))\n",
        );

        // ------------------------------------------------------------------
        // Network
        // ------------------------------------------------------------------

        if !self.network {
            profile.push_str("(deny network*)\n");
        }

        profile
    }

    /// Generate Linux bubblewrap arguments.
    ///
    /// Bubblewrap constructs a new filesystem namespace. The repository is
    /// writable while system directories remain read-only.
    #[cfg(target_os = "linux")]
    pub fn bwrap_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // --------------------------------------------------------------
        // Basic system filesystem
        // --------------------------------------------------------------

        bind_read_only(&mut args, "/usr");
        bind_read_only(&mut args, "/lib");
        bind_read_only(&mut args, "/bin");

        if Path::new("/lib64").exists() {
            args.push("--symlink".into());
            args.push("usr/lib64".into());
            args.push("/lib64".into());
        }

        args.push("--proc".into());
        args.push("/proc".into());

        args.push("--dev".into());
        args.push("/dev".into());

        // --------------------------------------------------------------
        // Temporary directory
        // --------------------------------------------------------------

        //
        // We only expose an explicit temp directory here. This prevents the
        // sandbox from automatically gaining write access to the host's
        // entire /tmp tree.
        //
        // If no temp directory was supplied, create a tmpfs.
        //
        // This has to come before any bind below: bwrap applies mounts in
        // order, so a tmpfs mounted after the repository would hide a
        // repository that lives under /tmp (scratch copies do).
        if let Some(temp) = &self.temp_dir {
            if temp.exists() {
                bind_rw_path(&mut args, temp);
            }
        } else {
            args.push("--tmpfs".into());
            args.push("/tmp".into());
        }

        // --------------------------------------------------------------
        // Repository
        // --------------------------------------------------------------

        bind_rw(&mut args, &self.repo_root, &self.repo_root);

        // --------------------------------------------------------------
        // Explicit read-only directories
        // --------------------------------------------------------------

        for dir in &self.readable_dirs {
            if dir.exists() {
                bind_read_only_path(&mut args, dir);
            }
        }

        // --------------------------------------------------------------
        // Explicit writable directories
        // --------------------------------------------------------------

        for dir in &self.writable_dirs {
            if dir.exists() {
                bind_rw_path(&mut args, dir);
            }
        }

        // --------------------------------------------------------------
        // Credential directories
        // --------------------------------------------------------------

        for dir in &self.allowed_credentials {
            if dir.exists() {
                bind_read_only_path(&mut args, dir);
            }
        }

        // --------------------------------------------------------------
        // Protected repository paths
        // --------------------------------------------------------------
        //
        // Re-bind protected paths read-only after the repository was mounted
        // writable.
        for path in self.protected_repo_paths() {
            if path.exists() {
                bind_read_only_path(&mut args, &path);
            }
        }

        // --------------------------------------------------------------
        // HOME
        // --------------------------------------------------------------

        if let Some(home) = &self.home_dir {
            if home.exists() {
                bind_rw_path(&mut args, home);
            }
        }

        // --------------------------------------------------------------
        // Network
        // --------------------------------------------------------------

        if !self.network {
            args.push("--unshare-net".into());
        }

        // Child processes die with the sandbox.
        args.push("--die-with-parent".into());

        args
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn protected_repo_paths(&self) -> Vec<PathBuf> {
        resolve_existing(
            [
                self.repo_root.join(".git/hooks"),
                self.repo_root.join(".git/config"),
                self.repo_root.join(".github/workflows"),
            ]
            .to_vec(),
        )
    }

    /// Default writable paths on macOS.
    ///
    /// These are intentionally limited to developer-tool state rather than
    /// granting arbitrary access to the user's entire home directory.
    #[cfg(target_os = "macos")]
    fn writable_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.repo_root.clone()];

        paths.extend(self.writable_dirs.clone());

        // macOS tools commonly use these locations for temporary files.
        paths.extend([
            PathBuf::from("/tmp"),
            PathBuf::from("/var/folders"),
            PathBuf::from("/var/tmp"),
        ]);

        // Common developer package/build caches.
        if let Some(home) = dirs::home_dir() {
            paths.extend([
                home.join(".cargo"),
                home.join(".rustup"),
                home.join(".npm"),
                home.join(".bun"),
                home.join(".yarn"),
                home.join("Library/pnpm"),
                home.join(".cache"),
                home.join("Library/Caches"),
            ]);
        }

        if let Some(temp) = &self.temp_dir {
            paths.push(temp.clone());
        }

        if let Some(home) = &self.home_dir {
            paths.push(home.clone());
        }

        resolve_existing(paths)
    }
}

#[cfg(target_os = "macos")]
fn hard_denied() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    resolve_existing(vec![home.join("Library/Keychains")])
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
/// Host environment variables a child process may inherit.
///
/// # Why this exists
///
/// Mira's own process environment holds credentials for providers, and an
/// agent — or a shell an agent starts — is third-party code that has no
/// business seeing them. Inheriting wholesale would hand it every key Mira
/// holds, so both the ACP process spawner and the PTY used for an agent's
/// terminals build their child environment from this list instead.
///
/// Defined once, here, because the two call sites previously carried
/// separate copies and drifted: the ACP spawner had picked up the Node and
/// Python variables that agent CLIs need to start, while the PTY had not.
///
/// # The rule
///
/// Include a variable only if a command genuinely cannot run without it
/// (`PATH`, `HOME`) or if its absence makes a tool misbehave rather than
/// merely different (`TERM` on a pty, `NODE_PATH` for a Node CLI).
///
/// **Deliberately excluded: anything that carries a capability.** See
/// [`credential_paths`] — `~/.ssh` and friends are readable only when
/// `allowed_credentials` says so, and `SSH_AUTH_SOCK` is the same grant by a
/// different route: a live handle to the user's ssh-agent, which would let a
/// third-party shell authenticate as the user while the key on disk stays
/// correctly denied. Keep it out of this list; add it through the profile
/// when a specific command genuinely needs it.
///
/// # Not the same as `env_clear`
///
/// Clearing the environment outright also removes `PATH` and `TERM`, which
/// does not make a child more secure so much as broken: shells fail to find
/// their own utilities and interactive tools disable line editing. The
/// allowlist is the part that matters.
pub fn safe_child_env() -> &'static [&'static str] {
    &[
        // Needed for a command to run at all.
        "PATH",
        "HOME",
        "TMPDIR",
        // Locale and identity; tools format output differently without them.
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SHELL",
        "USER",
        "LOGNAME",
        "TZ",
        // Agent CLIs are frequently Node or Python, and without these they
        // fail to start rather than merely behaving differently.
        "NODE_PATH",
        "NODE_OPTIONS",
        "NVM_DIR",
        "PYTHONPATH",
        "VIRTUAL_ENV",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        // Editor-backed agents launch a helper through $EDITOR.
        "EDITOR",
        "VISUAL",
    ]
}

pub fn credential_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    resolve_existing(vec![
        home.join(".ssh"),
        home.join(".aws"),
        home.join(".gnupg"),
        home.join(".config/gh"),
        home.join(".kube"),
    ])
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn credential_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// Determine which credentials a command may need.
///
/// This function only *identifies* credentials. The caller still needs to
/// explicitly add them to SandboxProfile::allow_credentials().
pub fn credentials_for(binary: &str, args: &[String]) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    let wanted: &[&str] = match command_name(binary).as_str() {
        "gh" => &[".config/gh"],

        "aws" | "sam" => &[".aws"],

        "kubectl" | "helm" | "k9s" => &[".kube"],

        "ssh" | "scp" | "sftp" | "ssh-add" => &[".ssh"],

        "gpg" | "gpg2" => &[".gnupg"],

        "git" if reaches_remote(args) => &[".ssh", ".gnupg"],

        _ => &[],
    };

    resolve_existing(wanted.iter().map(|dir| home.join(dir)).collect())
}

/// Extract the executable name from a potentially absolute path.
pub fn command_name(binary: &str) -> String {
    let name = Path::new(binary)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| binary.to_string());

    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// Determine whether a git invocation can reach a remote.
///
/// This is intentionally conservative.
fn reaches_remote(args: &[String]) -> bool {
    const REMOTE_COMMANDS: &[&str] = &["push", "fetch", "pull", "clone", "ls-remote", "remote"];

    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .is_some_and(|subcommand| REMOTE_COMMANDS.contains(&subcommand.as_str()))
}

fn resolve_existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut output = Vec::new();

    for path in paths {
        let Ok(resolved) = path.canonicalize() else {
            continue;
        };

        if !output.contains(&resolved) {
            output.push(resolved);
        }
    }

    output
}

#[cfg(target_os = "macos")]
fn escape(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', r"\\")
        .replace('"', "\\\"")
}

#[cfg(target_os = "linux")]
fn bind_read_only(args: &mut Vec<String>, path: &str) {
    if Path::new(path).exists() {
        args.push("--ro-bind".into());
        args.push(path.into());
        args.push(path.into());
    }
}

#[cfg(target_os = "linux")]
fn bind_read_only_path(args: &mut Vec<String>, path: &Path) {
    args.push("--ro-bind".into());
    args.push(path.display().to_string());
    args.push(path.display().to_string());
}

#[cfg(target_os = "linux")]
fn bind_rw(args: &mut Vec<String>, source: &Path, target: &Path) {
    args.push("--bind".into());
    args.push(source.display().to_string());
    args.push(target.display().to_string());
}

#[cfg(target_os = "linux")]
fn bind_rw_path(args: &mut Vec<String>, path: &Path) {
    bind_rw(args, path, path);
}

#[cfg(test)]
mod safe_child_env_tests {
    use super::*;

    #[test]
    fn the_allowlist_carries_nothing_that_authenticates() {
        // The whole point is that a child cannot use what Mira holds. A
        // credential-shaped variable here would be a silent bypass of
        // `allowed_credentials`, which gates the same credentials on disk.
        for key in safe_child_env() {
            let l = key.to_ascii_lowercase();
            assert!(
                !(l.contains("token")
                    || l.contains("secret")
                    || l.contains("password")
                    || l.contains("api_key")
                    || l.contains("apikey")),
                "{key} looks like a credential"
            );
        }
        // `SSH_AUTH_SOCK` is the specific one worth pinning: it grants the
        // same access as `~/.ssh` while the key itself stays denied.
        assert!(
            !safe_child_env().contains(&"SSH_AUTH_SOCK"),
            "a live ssh-agent handle must not be inherited by default"
        );
    }

    #[test]
    fn the_allowlist_still_contains_what_a_shell_needs() {
        // Clearing the environment outright is the tempting simplification
        // and it breaks every command: no PATH, no TERM.
        for required in ["PATH", "HOME", "TMPDIR", "TERM", "LANG"] {
            if required == "TERM" {
                // TERM is not read from the host (a non-tty parent has none);
                // the PTY spawner sets it explicitly. Everything else is
                // inherited verbatim.
                continue;
            }
            assert!(
                safe_child_env().contains(&required),
                "{required} must be inheritable"
            );
        }
    }

    #[test]
    fn the_allowlist_has_no_duplicates() {
        let mut seen = std::collections::BTreeSet::new();
        for k in safe_child_env() {
            assert!(seen.insert(*k), "{k} listed twice");
        }
    }

    #[test]
    fn the_allowlist_carries_no_credentials() {
        // The allowlist IS the ambient-deny mechanism for agent spawns:
        // anything inherited here reaches every agent, so no credential
        // variable may ever be added to it. An agent that bills to an
        // ambient key cannot be scoped, audited, or run as a second
        // identity — keys arrive only through the instance's own config.
        for k in safe_child_env() {
            let upper = k.to_ascii_uppercase();
            for banned in [
                "KEY",
                "TOKEN",
                "SECRET",
                "GOOGLE_",
                "AWS_",
                "AZURE_",
                "ANTHROPIC_",
                "OPENAI_",
                "CODEX_",
                "XAI_",
                "CURSOR_",
                "GEMINI_",
                "OAUTH",
            ] {
                assert!(
                    !upper.contains(banned),
                    "{k} looks like a credential and must not be inheritable"
                );
            }
        }
    }
}
