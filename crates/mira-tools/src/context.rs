use std::path::{Path, PathBuf};
use std::sync::Arc;

use mira_core::SessionId;
use mira_memory::{EpisodicStore, MemoryStore};
use mira_sandbox::Sandbox;
use tokio_util::sync::CancellationToken;

use crate::guard::FileGuard;
use crate::tasks::TaskStore;

/// Ambient state made available to every tool invocation.
///
/// This is the seam for adding new capabilities without changing the
/// `Tool` trait: session metadata, cancellation tokens, memory handles,
/// an LSP client, etc. Keep additions optional (`Option<T>`) so tools
/// can degrade gracefully when a facility isn't wired up.
#[derive(Clone)]
pub struct ToolContext {
    /// Absolute repository root.
    ///
    /// This is the filesystem security boundary for the session.
    /// It never changes when the logical working directory changes.
    pub repo_root: PathBuf,

    /// Current logical working directory for the session.
    ///
    /// This is used to resolve relative paths and to provide the
    /// working directory for command execution. It is independent
    /// of the sandbox security boundary.
    pub cwd: PathBuf,

    /// Sandbox that command-running tools must defer to.
    ///
    /// The sandbox is responsible for OS-level process isolation
    /// such as macOS Seatbelt or Linux Bubblewrap. Tools should not
    /// contain command-specific sandbox logic.
    pub sandbox: Arc<Sandbox>,

    /// Optional per-session file-safety layer: read-watermarks +
    /// undo snapshots.
    ///
    /// `None` means the caller didn't wire one up (tests, some
    /// headless runs); tools degrade to plain file operations.
    pub guard: Option<Arc<FileGuard>>,

    /// Optional handle to the shared memory store.
    ///
    /// Tools that read or mutate `MIRA.md` go through this so
    /// per-scope locking is honored across every writer.
    pub memory: Option<Arc<dyn MemoryStore>>,

    /// Optional handle to the cross-session episodic store.
    ///
    /// `memory_remember` and post-round extraction use this store.
    pub episodic: Option<Arc<dyn EpisodicStore>>,

    /// Optional current session id.
    ///
    /// Attached by the harness so persisted state can retain
    /// provenance.
    pub session_id: Option<SessionId>,

    /// How deeply nested the current session is under the user's
    /// top-level chat.
    ///
    /// Parent = 0, first-level subagent = 1, etc.
    pub agent_depth: usize,

    /// Opaque handle to the parent session's active-children list.
    ///
    /// The `agent` tool registers each spawned child here so an
    /// interrupt on the parent can cascade to in-flight subagents.
    pub child_tracker: Option<Arc<dyn ChildTracker>>,

    /// Session-scoped task list.
    pub tasks: Option<Arc<TaskStore>>,

    /// Optional live-progress emitter.
    ///
    /// Command-running tools forward stdout/stderr lines through this
    /// sink so the UI can render them while the command is executing.
    pub progress: Option<Arc<dyn ToolProgressSink>>,

    /// Cooperative cancellation signal for the current turn.
    ///
    /// The harness installs a fresh token at the beginning of every
    /// turn and fires it before aborting the turn's Tokio task.
    pub cancel: Option<CancellationToken>,
}

/// Contract the harness's `Session` fulfills to let the `agent` tool
/// register and de-register in-flight children by ID.
#[async_trait::async_trait]
pub trait ChildTracker: Send + Sync {
    /// Add a new child and return its opaque registration id.
    async fn register(&self, cancel: Box<dyn ChildCancel>) -> u64;

    /// Remove a previously registered child.
    async fn deregister(&self, id: u64);
}

/// Cancellation callback used by the parent session to stop a
/// specific in-flight child.
#[async_trait::async_trait]
pub trait ChildCancel: Send + Sync {
    async fn cancel(&self);
}

/// Contract used by the harness to receive live per-line output from
/// long-running tools.
pub trait ToolProgressSink: Send + Sync {
    /// Called whenever a stdout/stderr line arrives.
    ///
    /// `call_id` maps the line back to the corresponding ToolStart
    /// event so the renderer can place it under the pending tool card.
    fn emit(&self, call_id: &str, line: &str);
}

impl ToolContext {
    /// Create a context whose initial working directory is the repo root.
    pub fn new(repo_root: impl Into<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        let repo_root = repo_root.into();

        Self {
            cwd: repo_root.clone(),
            repo_root,
            sandbox,
            guard: None,
            memory: None,
            episodic: None,
            session_id: None,
            agent_depth: 0,
            child_tracker: None,
            tasks: None,
            progress: None,
            cancel: None,
        }
    }

    /// Attach a cooperative cancellation token.
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Attach the live-progress emitter.
    pub fn with_progress(mut self, sink: Arc<dyn ToolProgressSink>) -> Self {
        self.progress = Some(sink);
        self
    }

    /// Attach the session-scoped task store.
    pub fn with_tasks(mut self, tasks: Arc<TaskStore>) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// Attach the per-session file guard.
    pub fn with_guard(mut self, guard: Arc<FileGuard>) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Attach the shared memory store.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Attach the cross-session episodic store.
    pub fn with_episodic(mut self, episodic: Arc<dyn EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    /// Stamp the current session id.
    pub fn with_session_id(mut self, id: SessionId) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set the subagent nesting depth.
    pub fn with_agent_depth(mut self, depth: usize) -> Self {
        self.agent_depth = depth;
        self
    }

    /// Attach the parent session's child tracker.
    pub fn with_child_tracker(mut self, tracker: Arc<dyn ChildTracker>) -> Self {
        self.child_tracker = Some(tracker);
        self
    }

    /// Change the logical working directory.
    ///
    /// This does NOT change the sandbox boundary. The sandbox remains
    /// rooted at `repo_root`.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Option<Self> {
        let cwd = cwd.into();
        let resolved = self.resolve_path(&cwd)?;

        if !resolved.is_dir() {
            return None;
        }

        self.cwd = resolved;
        Some(self)
    }

    /// Resolve a possibly-relative path against the current working
    /// directory and ensure it remains inside the repository root or
    /// the explicitly permitted `~/.mira` directory.
    ///
    /// Existing symlinks are resolved before containment is checked.
    /// For paths that don't exist yet, the deepest existing ancestor
    /// is canonicalized and the missing tail is appended.
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let expanded = shellexpand::tilde(path);
        let candidate = Path::new(expanded.as_ref());

        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        };

        self.resolve_path(&joined)
    }

    /// Resolve a path that has already been converted to an absolute
    /// candidate.
    fn resolve_path(&self, path: &Path) -> Option<PathBuf> {
        let normalized = normalize(path);
        let resolved = resolve_symlinks(&normalized)?;

        let repo_root = std::fs::canonicalize(&self.repo_root).ok()?;

        if resolved.starts_with(&repo_root) {
            return Some(resolved);
        }

        // Second explicitly permitted root:
        //
        // ~/.mira
        //
        // This is intentionally much narrower than permitting the
        // entire home directory.
        if let Some(mira_home) = mira_home_dir() {
            match std::fs::canonicalize(&mira_home) {
                Ok(canon) => {
                    if resolved.starts_with(&canon) {
                        return Some(resolved);
                    }
                }
                Err(_) => {
                    let normalized_home = normalize(&mira_home);

                    if resolved.starts_with(&normalized_home) {
                        return Some(resolved);
                    }
                }
            }
        }

        None
    }
}

/// Absolute path to ~/.mira if HOME is available.
fn mira_home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;

    Some(PathBuf::from(home).join(".mira"))
}

/// Canonicalize the longest existing prefix of a path and append
/// missing components.
///
/// This makes symlink escapes detectable even when the final
/// destination does not exist yet.
fn resolve_symlinks(path: &Path) -> Option<PathBuf> {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return Some(canon);
    }

    let mut existing = path.to_path_buf();
    let mut trailing = Vec::<std::ffi::OsString>::new();

    loop {
        if let Ok(canon) = std::fs::canonicalize(&existing) {
            let mut result = canon;

            for component in trailing.iter().rev() {
                result.push(component);
            }

            return Some(result);
        }

        let name = existing.file_name()?.to_owned();
        trailing.push(name);

        if !existing.pop() {
            return None;
        }
    }
}

/// Pure lexical normalization.
///
/// Resolves `.` and `..` without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut output = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                output.pop();
            }
            Component::CurDir => {}
            other => {
                output.push(other.as_os_str());
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(cwd: PathBuf) -> ToolContext {
        let sandbox = Arc::new(Sandbox::default_scrubbed());

        ToolContext::new(cwd, sandbox)
    }

    #[test]
    fn resolve_refuses_symlink_that_escapes_cwd() {
        let cwd = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let secret = outside.path().join("secret.txt");

        std::fs::write(&secret, "shh").unwrap();

        let link = cwd.path().join("config");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&secret, &link).unwrap();

        let ctx = ctx_for(cwd.path().to_path_buf());

        assert!(
            ctx.resolve("config").is_none(),
            "symlink escaping repo must not resolve"
        );
    }

    #[test]
    fn resolve_accepts_regular_file_inside_cwd() {
        let cwd = tempfile::tempdir().unwrap();
        let inside = cwd.path().join("hello.txt");

        std::fs::write(&inside, "hi").unwrap();

        let ctx = ctx_for(cwd.path().to_path_buf());

        let resolved = ctx.resolve("hello.txt").expect("should resolve");

        assert!(resolved.ends_with("hello.txt"));
    }

    #[test]
    fn resolve_accepts_new_file_in_nonexistent_subdir() {
        let cwd = tempfile::tempdir().unwrap();

        let ctx = ctx_for(cwd.path().to_path_buf());

        let resolved = ctx
            .resolve("new_subdir/inner/file.txt")
            .expect("should resolve non-existent path");

        let canonical_cwd = cwd.path().canonicalize().unwrap();

        assert!(resolved.starts_with(canonical_cwd));
        assert!(resolved.ends_with("file.txt"));
    }

    #[test]
    fn cwd_can_move_inside_repo() {
        let cwd = tempfile::tempdir().unwrap();

        let subdir = cwd.path().join("src");
        std::fs::create_dir(&subdir).unwrap();

        let ctx = ctx_for(cwd.path().to_path_buf());

        let ctx = ctx.with_cwd(subdir.clone()).expect("cwd should be valid");

        assert_eq!(ctx.cwd, subdir.canonicalize().unwrap());

        assert_eq!(ctx.repo_root, cwd.path().canonicalize().unwrap());
    }

    #[test]
    fn cwd_cannot_escape_repo() {
        let cwd = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let ctx = ctx_for(cwd.path().to_path_buf());

        assert!(ctx.with_cwd(outside.path()).is_none());
    }
}
