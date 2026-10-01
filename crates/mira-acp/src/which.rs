//! Finding an executable that the user's shell can see but the process
//! running Mira may not.
//!
//! `claude` is installed by npm under `~/.nvm/versions/node/<v>/bin`, and that
//! directory exists on `PATH` only because a shell rc file put it there. An
//! app launched from Finder or a Dock icon inherits the bare system `PATH` —
//! `/usr/bin:/bin:/usr/sbin:/sbin` — and no amount of looking will find
//! `claude` there. The result was the settings panel reporting "Not
//! installed" for the coding agent its user ran all day, while the same probe
//! run from a terminal found it immediately.
//!
//! Three sources, cheapest first:
//!
//! 1. The inherited `PATH` — correct when Mira was started from a shell.
//! 2. Directories every version manager is known to use, which needs no
//!    subprocess and so cannot fail or hang.
//! 3. The login shell's own `PATH`. This is the authoritative answer and it
//!    covers anything a user has set up that we do not know about, at the cost
//!    of spawning a shell once.
//!
//! Resolution is cached: a login shell costs real time, and agent health is
//! checked repeatedly from a UI that expects an instant answer.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directories that hold user-installed binaries, relative to `$HOME`.
///
/// Deliberately a list of well-known locations rather than a scan: a recursive
/// walk of `$HOME` would be slow and would pick up things that are not on
/// anyone's `PATH`.
const HOME_BIN_DIRS: &[&str] = &[
    ".local/bin",
    ".npm-global/bin",
    ".npm-packages/bin",
    ".bun/bin",
    ".volta/bin",
    ".deno/bin",
    ".claude/local",
    ".config/yarn/global/node_modules/.bin",
    ".yarn/bin",
    ".cargo/bin",
    ".composer/vendor/bin",
    ".gem/ruby/bin",
    ".pyenv/shims",
    ".asdf/shims",
    ".local/share/mise/shims",
    "go/bin",
    "bin",
];

/// System directories worth checking even when `PATH` is empty.
const SYSTEM_BIN_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/snap/bin",
];

/// How long the login shell gets before we give up on it.
///
/// The shell probe is a last resort after the cheap sources, but it must not
/// be able to hang a settings panel.
const SHELL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

static CACHE: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();

/// The `PATH` Mira should use, discovered once.
pub fn effective_path() -> Option<std::ffi::OsString> {
    CACHE
        .get_or_init(|| {
            let mut dirs = candidate_dirs(
                home_dir().as_deref(),
                std::env::var_os("PATH").as_deref(),
                shell_path().as_deref(),
            );
            dirs.dedup();
            if dirs.is_empty() {
                return None;
            }
            std::env::join_paths(dirs).ok()
        })
        .clone()
}

/// Resolve a command name to a full path, or `None` if it is not installed.
///
/// The name may be a bare command or a path. A path is returned as-is, since
/// a user who set an explicit binary path means it.
pub fn resolve(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return is_executable_file(candidate).then(|| candidate.to_path_buf());
    }

    let path = effective_path()?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(name);
        if is_executable_file(&full) {
            return Some(full);
        }
    }
    None
}

/// Resolve a program for spawning, falling back to the given path.
///
/// A bare command name is resolved against the discovered `PATH`, and anything
/// with a path component is left alone. Used at every spawn site: resolving
/// only at *check* time means the check passes and the launch still fails,
/// which is exactly what happened with `claude` — validated as present, then
/// `No such file or directory` because the child had a bare system `PATH`.
pub fn resolve_for_spawn(program: &Path) -> PathBuf {
    resolve(&program.to_string_lossy()).unwrap_or_else(|| program.to_path_buf())
}

/// Every directory to search, in order.
///
/// Split out from [`effective_path`] so the ordering and contents can be
/// tested without touching the real environment.
fn candidate_dirs(
    home: Option<&Path>,
    inherited: Option<&std::ffi::OsStr>,
    from_shell: Option<&str>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };

    // 1. Inherited, split on ':'. Kept first so a correct PATH is never
    //    second-guessed by a guess.
    if let Some(p) = inherited {
        for d in std::env::split_paths(p) {
            if !d.as_os_str().is_empty() {
                push(d);
            }
        }
    }

    // 2. Known locations.
    for rel in HOME_BIN_DIRS {
        if let Some(h) = home {
            push(h.join(rel));
        }
    }
    for abs in SYSTEM_BIN_DIRS {
        push(PathBuf::from(abs));
    }

    // 2b. Every node version nvm has installed.
    //
    // Enumerated rather than trusted from the shell: `nvm` lives in `.zshrc`,
    // which a non-interactive `zsh -c` does not read, so asking a shell is
    // exactly the wrong mechanism for the most common case. Reading the
    // directory is both faster and the thing that is actually true.
    if let Some(h) = home {
        let versions = h.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(&versions) {
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path().join("bin"))
                .filter(|p| p.is_dir())
                .collect();
            // Newest version last in name order is usually what a user means
            // by "my node", so search the highest first.
            found.sort();
            found.reverse();
            for d in found {
                push(d);
            }
        }
    }

    // 3. The login shell, which knows about everything else. Appended rather
    //    than prepended: it is a subprocess and a last resort, and its answer
    //    agrees with the inherited PATH whenever there was one.
    if let Some(sp) = from_shell {
        for d in sp.split(':') {
            if !d.is_empty() {
                push(PathBuf::from(d));
            }
        }
    }

    out
}

/// Ask the login shell what its `PATH` is.
///
/// This is the standard way to recover the environment a user configured: it
/// is where `nvm`, `mise`, `asdf`, `pyenv` and friends actually get set up,
/// and no amount of hardcoded paths covers all of them.
fn shell_path() -> Option<String> {
    let shell = std::env::var_os("SHELL")?;
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "command -p echo $PATH"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    // A shell that hangs — waiting on a prompt, a slow rc file — must not be
    // able to wedge the settings list, so it is killed rather than waited on.
    let deadline = std::time::Instant::now() + SHELL_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Whether an env key is denied for a child process.
///
/// Prefixes, not exact names: credential families (`GOOGLE_*`) come in
/// groups. Deny wins over everything, including explicit config — a denied
/// prefix in the agent's own env is almost certainly a mistake, and silent
/// inheritance is the threat this exists to kill.
pub fn env_denied(key: &str, deny: &[String]) -> bool {
    deny.iter().any(|p| key.starts_with(p.as_str()))
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Option<PathBuf> {
        Some(PathBuf::from("/home/tester"))
    }

    #[test]
    fn the_inherited_path_comes_first() {
        let dirs = candidate_dirs(
            home().as_deref(),
            Some(std::ffi::OsStr::new("/first:/second")),
            None,
        );
        assert_eq!(dirs[0], PathBuf::from("/first"));
        assert_eq!(dirs[1], PathBuf::from("/second"));
    }

    #[test]
    fn version_manager_bins_are_searched_even_with_a_bare_path() {
        // The real failure: PATH is `/usr/bin:/bin` and the agent is in nvm.
        let bare = std::ffi::OsStr::new("/usr/bin:/bin");
        let dirs = candidate_dirs(home().as_deref(), Some(bare), None);
        assert!(
            dirs.contains(&PathBuf::from("/usr/bin")),
            "the inherited path must be kept"
        );
        // No nvm glob here, but the documented home locations are present.
        assert!(dirs.contains(&PathBuf::from("/home/tester/.local/bin")));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn the_login_shells_path_is_included_and_deduplicated() {
        let dirs = candidate_dirs(
            home().as_deref(),
            Some(std::ffi::OsStr::new("/usr/bin")),
            Some("/usr/bin:/nvm/v22/bin"),
        );
        // Appears once, not twice.
        assert_eq!(
            dirs.iter().filter(|d| *d == &PathBuf::from("/usr/bin")).count(),
            1
        );
        assert!(dirs.contains(&PathBuf::from("/nvm/v22/bin")));
    }

    #[test]
    fn an_explicit_path_is_never_searched_for() {
        let resolved = resolve("/definitely/not/here/claude");
        assert!(resolved.is_none());
    }

    #[test]
    fn nvm_versions_are_enumerated_from_disk() {
        let home = std::path::Path::new("/Users/damilola");
        if !home.join(".nvm/versions/node").is_dir() {
            return; // not an nvm machine; nothing to assert
        }
        let bare = std::ffi::OsStr::new("/usr/bin:/bin");
        let dirs = candidate_dirs(Some(home), Some(bare), None);
        let nvm_bins: Vec<_> = dirs
            .iter()
            .filter(|d| d.to_string_lossy().contains("/.nvm/versions/node/"))
            .collect();
        assert!(
            !nvm_bins.is_empty(),
            "an nvm install must be searchable without the shell's PATH"
        );
    }

    #[test]
    fn spawn_resolution_keeps_explicit_paths() {
        // A path with a component is the user's decision and is never second
        // guessed; a bare name is looked up.
        let explicit = PathBuf::from("/opt/agent/bin/thing");
        assert_eq!(resolve_for_spawn(&explicit), explicit);

        let bare = PathBuf::from("sh");
        assert!(resolve_for_spawn(&bare).is_absolute());
    }

    #[test]
    fn resolution_finds_a_real_command() {
        // Whichever `PATH` this machine has, `sh` must be findable — if the
        // discovery logic is broken this fails.
        assert!(resolve("sh").is_some(), "sh must resolve");
    }
}
