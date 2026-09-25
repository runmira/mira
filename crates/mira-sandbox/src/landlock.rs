//! Linux Landlock backend.
//!
//! Used when bubblewrap is missing or can't create namespaces (common
//! in containers and on distros that restrict unprivileged user
//! namespaces). Landlock needs no privileges and no helper binary: the
//! parent builds a ruleset, and the child applies it to itself right
//! before `exec`.
//!
//! What it enforces:
//! - reads and execs only from system directories, the repository, the
//!   profile's readable/writable directories, approved credentials, the
//!   directories on PATH, and developer caches in HOME
//! - writes only to the repository, the profile's writable directories,
//!   the temp directory, and developer caches
//! - no TCP bind/connect when network is off (kernel 6.7+, ABI v4)
//!
//! What it can't do, compared with bubblewrap:
//! - Rules only grant, so `.git/hooks`, `.git/config` and
//!   `.github/workflows` stay writable inside a writable repository.
//! - `/tmp` is shared with the host unless the profile sets a temp dir.
//! - UDP (and so DNS) isn't covered by Landlock's network rules.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::profile::credential_paths;
use crate::SandboxProfile;

const CREATE_RULESET_VERSION: u32 = 1 << 0;
const RULE_PATH_BENEATH: libc::c_int = 1;

const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
/// ABI v2: rename/link across directories.
const FS_REFER: u64 = 1 << 13;
/// ABI v3.
const FS_TRUNCATE: u64 = 1 << 14;

/// ABI v4.
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;

const READ: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;
/// Rights that make sense on a regular file (the rest need a directory).
const FILE_RIGHTS: u64 = FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE | FS_TRUNCATE;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// The kernel's Landlock ABI version, or 0 when Landlock is unavailable
/// (old kernel, or disabled at boot).
pub fn abi() -> i32 {
    static ABI: OnceLock<i32> = OnceLock::new();
    *ABI.get_or_init(|| {
        // SAFETY: the version query takes no attribute pointer.
        let v = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            )
        };
        if v < 0 {
            0
        } else {
            v as i32
        }
    })
}

pub fn available() -> bool {
    abi() >= 1
}

/// A ruleset built in the parent, ready to be applied in the child.
pub struct Ruleset {
    fd: OwnedFd,
}

impl Ruleset {
    /// Build the ruleset for `profile`.
    pub fn for_profile(profile: &SandboxProfile) -> io::Result<Self> {
        let abi = abi();
        if abi < 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Landlock isn't available on this kernel",
            ));
        }
        let mut write = READ
            | FS_WRITE_FILE
            | FS_REMOVE_DIR
            | FS_REMOVE_FILE
            | FS_MAKE_CHAR
            | FS_MAKE_DIR
            | FS_MAKE_REG
            | FS_MAKE_SOCK
            | FS_MAKE_FIFO
            | FS_MAKE_BLOCK
            | FS_MAKE_SYM;
        if abi >= 2 {
            write |= FS_REFER;
        }
        if abi >= 3 {
            write |= FS_TRUNCATE;
        }
        let handled_net = if abi >= 4 && !profile.network {
            NET_BIND_TCP | NET_CONNECT_TCP
        } else {
            0
        };
        if !profile.network && handled_net == 0 {
            tracing::warn!("Landlock ABI {abi} can't block the network (needs kernel 6.7+)");
        }

        let attr = RulesetAttr {
            handled_access_fs: write,
            handled_access_net: handled_net,
        };
        // SAFETY: `attr` is a valid, initialised struct of the size passed.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the kernel just handed us this descriptor.
        let ruleset = Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
        };

        let (read_paths, write_paths) = paths(profile);
        for path in &read_paths {
            ruleset.allow(path, READ)?;
        }
        for path in &write_paths {
            ruleset.allow(path, write)?;
        }
        // /dev/null, /dev/tty and friends need writing, not creating.
        ruleset.allow(Path::new("/dev"), READ | FS_WRITE_FILE)?;
        Ok(ruleset)
    }

    /// Grant `access` beneath `path`. Missing paths are skipped.
    fn allow(&self, path: &Path, access: u64) -> io::Result<()> {
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            return Ok(());
        };
        // SAFETY: `c_path` is NUL-terminated and outlives the call.
        let raw = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if raw < 0 {
            return Ok(());
        }
        // SAFETY: `open` just returned this descriptor.
        let parent = unsafe { OwnedFd::from_raw_fd(raw) };
        let access = if path.is_dir() {
            access
        } else {
            access & FILE_RIGHTS
        };
        let attr = PathBeneathAttr {
            allowed_access: access,
            parent_fd: parent.as_raw_fd(),
        };
        // SAFETY: both descriptors are open and `attr` is fully initialised.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                self.fd.as_raw_fd(),
                RULE_PATH_BENEATH,
                &attr as *const PathBeneathAttr,
                0u32,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Make the command's child process restrict itself before `exec`.
    /// The ruleset must outlive `spawn`.
    pub fn apply_to(&self, command: &mut tokio::process::Command) {
        let fd = self.fd.as_raw_fd();
        // SAFETY: the hook runs between fork and exec, so it only makes
        // two raw syscalls: no allocation, no locks.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

/// Directories to grant, as (read-only, read-write).
fn paths(profile: &SandboxProfile) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let home = dirs::home_dir();
    let credentials = credential_paths();
    let secret = |p: &Path| {
        credentials
            .iter()
            .filter(|c| !profile.allowed_credentials.contains(c))
            .any(|c| p.starts_with(c) || c.starts_with(p))
    };

    let mut read: Vec<PathBuf> = [
        "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/nix",
        "/proc", "/sys", "/run", "/var/lib", "/snap",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    read.extend(profile.readable_dirs.iter().cloned());
    read.extend(profile.allowed_credentials.iter().cloned());
    // Tools on PATH (~/.cargo/bin, ~/.local/bin, …), but never a
    // directory that would expose HOME or the whole disk.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let broad = dir == Path::new("/") || home.as_deref() == Some(dir.as_path());
            if dir.is_absolute() && !broad {
                read.push(dir);
            }
        }
    }

    let mut write = vec![profile.repo_root.clone()];
    write.extend(profile.writable_dirs.iter().cloned());
    match &profile.temp_dir {
        Some(temp) => write.push(temp.clone()),
        None => write.extend([PathBuf::from("/tmp"), PathBuf::from("/var/tmp")]),
    }
    if let Some(dir) = &profile.home_dir {
        write.push(dir.clone());
    }
    write.push(PathBuf::from("/dev/shm"));
    if let Some(home) = &home {
        // Developer caches, as the macOS profile allows.
        for dir in [
            ".cargo",
            ".rustup",
            ".npm",
            ".bun",
            ".yarn",
            ".cache",
            ".pnpm-store",
        ] {
            write.push(home.join(dir));
        }
        for p in [
            ".gitconfig",
            ".config/git",
            ".local",
            ".nvm",
            ".pyenv",
            ".volta",
            ".deno",
            ".gradle",
            ".m2",
            "go",
        ] {
            read.push(home.join(p));
        }
    }

    read.retain(|p| p.exists() && !secret(p));
    write.retain(|p| p.exists() && !secret(p));
    (read, write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{run_with_backend, SandboxConfig};
    use crate::SandboxBackend;

    async fn sh(profile: SandboxProfile, script: &str) -> crate::CommandOutput {
        let cwd = profile.repo_root.clone();
        run_with_backend(
            SandboxBackend::Landlock,
            &SandboxConfig::new(profile),
            "sh",
            &["-c".to_owned(), script.to_owned()],
            &cwd,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn confines_reads_and_writes() {
        if !available() {
            eprintln!("skipping: no Landlock");
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s3cret").unwrap();
        let profile = SandboxProfile::new(repo.path()).temp_dir(temp.path());

        let out = sh(profile.clone(), "echo hi > made && cat made").await;
        assert_eq!(out.stdout.trim(), "hi", "{out:?}");

        let out = sh(
            profile.clone(),
            "echo x > \"$TMPDIR/t\" && cat \"$TMPDIR/t\"",
        )
        .await;
        assert_eq!(out.stdout.trim(), "x", "{out:?}");

        let target = outside.path().join("nope");
        let out = sh(profile.clone(), &format!("echo x > '{}'", target.display())).await;
        assert!(!out.success());
        assert!(!target.exists());

        let secret = outside.path().join("secret");
        let out = sh(profile, &format!("cat '{}'", secret.display())).await;
        assert!(!out.success());
        assert!(!out.stdout.contains("s3cret"));
    }

    #[tokio::test]
    async fn blocks_tcp_when_offline() {
        if abi() < 4 {
            eprintln!("skipping: Landlock ABI < 4");
            return;
        }
        // bash's /dev/tcp gives a dependency-free TCP connect.
        if !Path::new("/bin/bash").exists() && !Path::new("/usr/bin/bash").exists() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let repo = tempfile::tempdir().unwrap();
        let script = format!("bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}' && echo connected");

        let offline = SandboxProfile::new(repo.path());
        let out = sh(offline, &script).await;
        assert!(!out.stdout.contains("connected"), "{out:?}");

        let online = SandboxProfile::new(repo.path()).network(true);
        let out = sh(online, &script).await;
        assert!(out.stdout.contains("connected"), "{out:?}");
    }
}
