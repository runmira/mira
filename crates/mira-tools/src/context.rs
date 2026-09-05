use std::path::{Path, PathBuf};
use std::sync::Arc;

use mira_sandbox::Sandbox;

/// Ambient state made available to every tool invocation.
///
/// This is the seam for adding new capabilities without changing the `Tool`
/// trait: session metadata, cancellation tokens, memory handles, an LSP
/// client, etc. Keep additions optional (`Option<T>`) so tools can degrade
/// gracefully when a facility isn't wired up.
#[derive(Clone)]
pub struct ToolContext {
    /// The repo root. Tools that touch the filesystem MUST canonicalize
    /// against this and refuse to escape it.
    pub cwd: PathBuf,
    /// Sandbox that command-running tools should defer to.
    pub sandbox: Arc<Sandbox>,
}

impl ToolContext {
    pub fn new(cwd: impl Into<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            cwd: cwd.into(),
            sandbox,
        }
    }

    /// Resolve a possibly-relative path against `cwd` and ensure the result
    /// stays inside the cwd. Returns `None` if the path escapes.
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let expanded = shellexpand::tilde(path);
        let candidate = Path::new(expanded.as_ref());
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        };
        // We use lexical normalization rather than canonicalize() because the
        // target may not exist yet (e.g. `write_file` creating a new path).
        let normalized = normalize(&joined);
        let cwd = normalize(&self.cwd);
        normalized.starts_with(&cwd).then_some(normalized)
    }
}

/// Pure lexical normalization: resolve `.` and `..` without touching the FS.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
