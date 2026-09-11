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
        }
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
