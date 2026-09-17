use std::path::{Path, PathBuf};
use std::sync::Arc;

use mira_core::SessionId;
use mira_memory::{EpisodicStore, MemoryStore};
use mira_sandbox::{PersistentShell, Sandbox};
use tokio::sync::Mutex;

use crate::guard::FileGuard;
use crate::tasks::TaskStore;

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
    /// Optional per-session file-safety layer: read-watermarks + undo
    /// snapshots. `None` means the caller didn't wire one up (tests, some
    /// headless runs); tools should degrade to plain file operations.
    pub guard: Option<Arc<FileGuard>>,
    /// Optional long-lived shell scoped to this session. When present,
    /// the `bash` tool routes commands through it so `cd`, activated
    /// venvs, and `export`s persist across calls. Absent for headless /
    /// test runs; Bash falls back to the fresh-per-call sandbox path.
    pub shell: Option<Arc<Mutex<PersistentShell>>>,
    /// Optional handle to the shared memory store. Tools that read or
    /// mutate `MIRA.md` (`memory_read`, `memory_append`, `memory_edit`,
    /// `memory_search`) go through this so per-scope locking is honored
    /// across every writer (agent tools, HTTP endpoints, future clients).
    /// Absent for tests that don't want a real filesystem behind memory.
    pub memory: Option<Arc<dyn MemoryStore>>,
    /// Optional handle to the cross-session episodic store (JSONL). The
    /// `memory_remember` tool appends here, and the post-round
    /// auto-extraction pass writes here too.
    pub episodic: Option<Arc<dyn EpisodicStore>>,
    /// Optional current session id. Attached by the harness inside
    /// `Session::new` / `Session::resume_from` so tools that persist
    /// state (episodic entries, undo snapshots) can stamp provenance.
    pub session_id: Option<SessionId>,
    /// How deeply nested the currently-executing session is under the
    /// user's top-level chat. Parent (user-facing) = 0; first-level
    /// subagent = 1; nested subagent = 2, etc. Read by the `agent` tool
    /// to enforce a hard cap on runaway spawn recursion — new subagents
    /// inherit `parent.agent_depth + 1`.
    pub agent_depth: usize,
    /// Opaque handle to the parent session's active-children list. The
    /// `agent` tool registers each child it spawns here so an interrupt
    /// on the parent cascades to every subagent currently in flight.
    /// Absent for headless / test contexts — the tool falls back to
    /// spawning without registration.
    ///
    /// Kept `Arc<dyn ChildTracker>` so mira-tools doesn't have to know
    /// about `Session` (which lives in mira-harness). See
    /// [`ChildTracker`] for the two-method contract.
    pub child_tracker: Option<Arc<dyn ChildTracker>>,
    /// Session-scoped task list the `task_*` tools mutate. Attached
    /// by the harness inside `Session::new` / `Session::resume_from`.
    /// Absent for tests / headless runs — the task tools return an
    /// error in that case.
    pub tasks: Option<Arc<TaskStore>>,
    /// Optional live-progress emitter — the `bash` tool forwards each
    /// stdout+stderr line from a running command through here so the
    /// UI can render output under the pending tool card while the
    /// command is still executing. Absent for headless runs; the tool
    /// falls back to the return-at-end path.
    pub progress: Option<Arc<dyn ToolProgressSink>>,
}

/// Contract the harness's `Session` fulfills to let the `agent` tool
/// register and de-register in-flight children by ID. Keeping this at
/// the tool-context layer avoids a `mira-tools → mira-harness` cycle.
///
/// `register` is called with a boxed cancel callback the parent invokes
/// on interrupt. `deregister` removes the entry on child completion.
#[async_trait::async_trait]
pub trait ChildTracker: Send + Sync {
    /// Add a new child. Returns an opaque id the caller passes back to
    /// [`Self::deregister`] once the child finishes.
    async fn register(&self, cancel: Box<dyn ChildCancel>) -> u64;
    async fn deregister(&self, id: u64);
}

/// Cancellation callback the parent uses to abort a specific in-flight
/// child. The trait is object-safe so we can hold `Box<dyn ChildCancel>`
/// on the parent side without knowing the concrete session type.
#[async_trait::async_trait]
pub trait ChildCancel: Send + Sync {
    async fn cancel(&self);
}

/// Contract the harness fulfills to receive live per-line output from a
/// still-running tool call. Used today by the `bash` tool to stream PTY
/// output into the transcript; wired into other long-running tools as
/// they need it. Non-blocking best-effort — the tool doesn't fail if
/// the sink drops.
pub trait ToolProgressSink: Send + Sync {
    /// Called with each stdout+stderr line as it lands. `call_id` maps
    /// back to the corresponding `ToolStart` event so the renderer can
    /// route the line under the right pending card.
    fn emit(&self, call_id: &str, line: &str);
}

impl ToolContext {
    pub fn new(cwd: impl Into<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            cwd: cwd.into(),
            sandbox,
            guard: None,
            shell: None,
            memory: None,
            episodic: None,
            session_id: None,
            agent_depth: 0,
            child_tracker: None,
            tasks: None,
            progress: None,
        }
    }

    /// Attach the live-progress emitter. Wired by the harness so bash
    /// (and any future long-running tool) can stream output to the UI
    /// while it runs.
    pub fn with_progress(mut self, sink: Arc<dyn ToolProgressSink>) -> Self {
        self.progress = Some(sink);
        self
    }

    /// Attach the session-scoped task store. Wired by the harness in
    /// `Session::new` / `Session::resume_from` so the `task_*` tools
    /// share one view with the checkpoint path.
    pub fn with_tasks(mut self, tasks: Arc<TaskStore>) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// Chainable setter used by the harness after Session::new picks a
    /// session id — the FileGuard is scoped to that id.
    pub fn with_guard(mut self, guard: Arc<FileGuard>) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Attach a per-session persistent shell so bash commands share state
    /// across calls. Same lifecycle as `guard`.
    pub fn with_shell(mut self, shell: Arc<Mutex<PersistentShell>>) -> Self {
        self.shell = Some(shell);
        self
    }

    /// Attach the shared memory store so the memory tools can read/append/
    /// edit `MIRA.md`. Wired by the server (or CLI) at startup — one store
    /// instance is shared with the `/api/memory/append` HTTP endpoint so
    /// concurrent writers all take the same per-scope lock.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Attach the cross-session episodic store. Needed by
    /// `memory_remember`; the harness's post-round auto-extractor also
    /// pulls this same handle to keep provenance / dedup consistent.
    pub fn with_episodic(mut self, episodic: Arc<dyn EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    /// Stamp the current session id. Used by episodic writes to record
    /// which session an entry came from — useful for later consolidation
    /// and for the review UI.
    pub fn with_session_id(mut self, id: SessionId) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set the subagent nesting depth. The `agent` tool uses this to cap
    /// runaway spawn recursion — see the constant in `AgentTool`.
    pub fn with_agent_depth(mut self, depth: usize) -> Self {
        self.agent_depth = depth;
        self
    }

    /// Attach the parent session's child tracker so the `agent` tool
    /// can register spawned children for interrupt cascade.
    pub fn with_child_tracker(mut self, tracker: Arc<dyn ChildTracker>) -> Self {
        self.child_tracker = Some(tracker);
        self
    }

    /// Resolve a possibly-relative path against `cwd` and ensure the result
    /// stays inside the cwd — or under the user's `~/.mira/` config dir,
    /// which is a permitted second destination for skill installers,
    /// memory writers, and prompt generators. Returns `None` if the path
    /// escapes both roots.
    ///
    /// Symlink-safe: any existing prefix of the target is canonicalized
    /// through the OS, so a symlink committed inside the workspace
    /// (`.env → ~/.ssh/id_rsa`) that would pass a lexical `starts_with`
    /// check is caught here and refused. New-file paths (destination
    /// doesn't exist yet, e.g. `write_file`) canonicalize the deepest
    /// existing ancestor and append the trailing components — so a
    /// legitimate `write_file` to a not-yet-created subdirectory still
    /// works while a symlink further up the chain still gets resolved.
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let expanded = shellexpand::tilde(path);
        let candidate = Path::new(expanded.as_ref());
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        };
        let normalized = normalize(&joined);
        // Real path: existing-prefix canonicalize + trailing tail so
        // symlinks anywhere in the chain surface as their true target
        // before we check containment.
        let resolved = resolve_symlinks(&normalized)?;

        if let Ok(cwd_canon) = std::fs::canonicalize(&self.cwd) {
            if resolved.starts_with(&cwd_canon) {
                return Some(resolved);
            }
        }
        // Second allowed root: the user's ~/.mira/ config dir. Kept
        // narrow to the mira subtree — not the full home dir. Falls
        // back to a lexical check when the dir doesn't exist yet, since
        // `canonicalize` of a missing dir fails; the fallback is safe
        // because `mira_home_dir()` is derived from `$HOME` (not user
        // input) so it can't be aliased to look like it's inside cwd.
        if let Some(mira_home) = mira_home_dir() {
            match std::fs::canonicalize(&mira_home) {
                Ok(canon) => {
                    if resolved.starts_with(&canon) {
                        return Some(resolved);
                    }
                }
                Err(_) => {
                    let normalized_home = normalize(&mira_home);
                    if normalized.starts_with(&normalized_home) {
                        return Some(normalized);
                    }
                }
            }
        }
        None
    }
}

/// Absolute path to `~/.mira/` if `$HOME` is set. Cached-free: called
/// once per path resolution, and `std::env::var_os` is cheap.
fn mira_home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".mira"))
}

/// Canonicalize the longest existing prefix of `path` and re-append
/// the trailing components. Returns `None` only when NO prefix of the
/// path exists — including the filesystem root, so in practice this
/// is a very unusual failure. Symlinks in the existing prefix are
/// resolved to their real targets; missing tail components are kept
/// as-is (matching `write_file`'s "create a new path" semantics).
fn resolve_symlinks(path: &Path) -> Option<PathBuf> {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return Some(canon);
    }
    let mut existing = path.to_path_buf();
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(cwd: PathBuf) -> ToolContext {
        ToolContext::new(
            cwd,
            std::sync::Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        )
    }

    #[test]
    fn resolve_refuses_symlink_that_escapes_cwd() {
        // Repo layout:
        //   <cwd>/config -> <outside>/secret.txt
        // The lexical check would accept `config` (it's inside cwd),
        // but the symlink points outside → resolve() must refuse.
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
            "symlink escaping cwd must not resolve"
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
    fn resolve_accepts_new_file_in_nonexistent_subdir_inside_cwd() {
        // write_file semantics: destination doesn't exist yet — the
        // symlink-safe path must fall back to canonicalising the
        // deepest existing ancestor + appending the missing tail.
        let cwd = tempfile::tempdir().unwrap();
        let ctx = ctx_for(cwd.path().to_path_buf());
        let resolved = ctx
            .resolve("new_subdir/inner/file.txt")
            .expect("should resolve non-existent path under cwd");
        assert!(resolved.starts_with(cwd.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("file.txt"));
    }
}
