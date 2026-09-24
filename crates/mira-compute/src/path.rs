//! Mapping tool paths onto the workspace.
//!
//! The model sees the user's local paths (the system prompt names the
//! local working directory), and sometimes the sandbox's own paths from
//! command output. Both map onto one workspace-relative form, and
//! anything that would escape the workspace is refused. This is purely
//! lexical: it never looks at a filesystem, since the real one may be
//! a VM away.

use std::path::{Component, Path, PathBuf};

use crate::{ComputeError, Result};

/// Resolve `path` to a workspace-relative path (`""` = the root, never a
/// leading `/`).
///
/// - `local_root`: the user's repo root (absolute local paths under it
///   are accepted).
/// - `remote_root`: the backend's workspace root (absolute paths under it
///   are accepted too).
/// - `cwd`: the current directory, relative to the workspace root.
pub fn resolve(path: &str, local_root: &Path, remote_root: &str, cwd: &str) -> Result<String> {
    let p = Path::new(path);
    let rel: PathBuf = if p.is_absolute() {
        if let Ok(r) = p.strip_prefix(local_root) {
            r.to_path_buf()
        } else if let Ok(r) = p.strip_prefix(remote_root) {
            r.to_path_buf()
        } else {
            return Err(ComputeError::InvalidPath(format!(
                "{path} is outside the workspace ({})",
                local_root.display()
            )));
        }
    } else {
        Path::new(cwd).join(p)
    };

    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(ComputeError::InvalidPath(format!(
                        "{path} escapes the workspace"
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ComputeError::InvalidPath(path.to_owned()))
            }
        }
    }
    Ok(parts.join("/"))
}

/// `root` joined with a workspace-relative path.
pub fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_owned()
    } else {
        format!("{}/{rel}", root.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REMOTE: &str = "/home/user/workspace";

    fn r(p: &str, cwd: &str) -> Result<String> {
        resolve(p, Path::new("/Users/me/proj"), REMOTE, cwd)
    }

    #[test]
    fn maps_relative_local_and_remote_paths() {
        assert_eq!(r("src/main.rs", "").unwrap(), "src/main.rs");
        assert_eq!(r("./a/../b.rs", "crates").unwrap(), "crates/b.rs");
        assert_eq!(r("/Users/me/proj/Cargo.toml", "x").unwrap(), "Cargo.toml");
        assert_eq!(r("/home/user/workspace/src/x.rs", "").unwrap(), "src/x.rs");
        assert_eq!(r(".", "").unwrap(), "");
    }

    #[test]
    fn refuses_escapes() {
        assert!(r("../secret", "").is_err());
        assert!(r("a/../../x", "").is_err());
        assert!(r("/etc/passwd", "").is_err());
        assert!(r("/Users/me/project-other/x", "").is_err());
    }

    #[test]
    fn join_handles_root() {
        assert_eq!(join(REMOTE, ""), REMOTE);
        assert_eq!(join("/w/", "a/b"), "/w/a/b");
    }
}
