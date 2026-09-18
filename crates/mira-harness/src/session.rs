use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use futures::{stream::BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ResponseFormat};
use mira_core::{Message, Role, SessionId, ToolCall, ToolResult};
use mira_memory::{
    EpisodicEntry, EpisodicSource, EpisodicStore, MemoryQuery, MemorySnapshot, DEFAULT_TOKEN_BUDGET,
};
use mira_policy::{Decision, Mode, Policy, Request as PolicyRequest};
use mira_sandbox::{PersistentShell, SandboxProfile};
use mira_tools::context::{ChildCancel, ChildTracker, ToolProgressSink};
use mira_tools::{compute_preview, DiffPreview, FileGuard, Registry, TaskStore, ToolContext};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::AbortHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// How much of a tool result actually goes back into the model's context.
/// A single `ls -R` on a real repo can be 20k+ tokens which blows past
/// every provider's per-request cap — cap in the harness so the frontend
/// still sees the full result but the model only sees a preview.
const TOOL_RESULT_HISTORY_CAP: usize = 4000;

/// When we truncate, how much of the cap goes to the tail. The head is
/// usually most relevant (first lines of a diff, start of a file listing),
/// but for many bash tools the tail carries the actual outcome (test
/// summary, error message, exit banner). Keeping ~20% of the budget for
/// the tail preserves that signal without shrinking the head too much.
const TOOL_RESULT_TAIL_FRACTION: f64 = 0.2;

use crate::approver::Approver;
use crate::event::HarnessEvent;
use crate::goal::{self, Goal, GoalStatus, GoalVerdict};

/// Type alias for the slot the harness parks the current turn's event
/// sender into so long-running tools can stream progress out. Held
/// behind a `std::sync::Mutex` because [`ToolProgressSink::emit`] is
/// synchronous — locking a tokio mutex from a sync callback isn't
/// safe. Held very briefly (send-and-return) so std lock contention
/// doesn't matter.
type ProgressSlot = Arc<StdMutex<Option<mpsc::Sender<HarnessEvent>>>>;

/// Bridge from tools that emit live output (bash) → the current turn's
/// event stream. Attached once to the session's `ToolContext` and kept
/// there for the session's lifetime; each turn installs its own tx via
/// [`TurnProgress::install`] and drops it via [`TurnProgress::clear`]
/// so lines from a stale run don't fan into a fresh turn.
struct TurnProgress {
    slot: ProgressSlot,
}

impl TurnProgress {
    fn new(slot: ProgressSlot) -> Self {
        Self { slot }
    }
}

impl ToolProgressSink for TurnProgress {
    fn emit(&self, call_id: &str, line: &str) {
        // std::sync::Mutex — poisoning shouldn't take down the session,
        // so grab it defensively and fall back to a no-op.
        let Ok(guard) = self.slot.lock() else { return };
        if let Some(tx) = guard.as_ref() {
            // try_send: don't block the sync callback if the channel
            // buffer is full. A dropped progress line is fine — the
            // final `ToolEnd` still carries the whole transcript.
            let _ = tx.try_send(HarnessEvent::ToolProgress {
                call_id: call_id.to_owned(),
                line: line.to_owned(),
            });
        }
    }
}

/// RAII helper that clears the progress slot when it drops. Ensures
/// `run_loop` unwiring happens regardless of whether the turn ended
/// normally, cancellation, or panic.
struct ProgressSlotGuard {
    slot: ProgressSlot,
}

impl ProgressSlotGuard {
    fn new(slot: ProgressSlot) -> Self {
        Self { slot }
    }
}

impl Drop for ProgressSlotGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.lock() {
            *guard = None;
        }
    }
}

/// RAII helper that clears the per-turn cancel slot when it drops.
/// Ensures a leftover token from a cancelled or panicked turn can't
/// be re-fired against the next turn. Held behind a tokio `Mutex`
/// because `Session::current_cancel` is async-locked; drop uses a
/// non-blocking `try_lock` and, on the (unlikely) contended branch,
/// spawns a short cleanup task so the guard's drop never blocks.
struct CancelSlotGuard {
    slot: Arc<Mutex<Option<CancellationToken>>>,
}

impl Drop for CancelSlotGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.try_lock() {
            *guard = None;
            return;
        }
        let slot = self.slot.clone();
        tokio::spawn(async move {
            *slot.lock().await = None;
        });
    }
}
use crate::persist::{now_ms, now_secs, SessionRecord, SessionStore, TurnMeta, UsageTotals};

/// Runtime configuration for a session. Everything the loop needs besides
/// mutable state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub model: String,
    /// Number of tool-call rounds allowed within one user turn. Guards
    /// against runaway loops (model calling itself forever).
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Reasoning effort for models that expose it (OpenAI `reasoning_effort`,
    /// Anthropic thinking budget, …). `None` = don't send the field. Values
    /// mirror OpenAI's vocabulary: `"minimal" | "low" | "medium" | "high"`.
    /// Providers that don't recognise the field ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Constrain the model's text output. When set, the harness attaches
    /// this to every `ChatRequest` for this session. AgentTool wires
    /// this from an agent type's `response_schema` so subagents can
    /// return structured JSON the parent parses into `ToolResult.data`.
    /// `None` = freeform prose (default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// Optional cheaper model used only for rolling compaction. A long
    /// session on Opus can compact with Haiku for roughly a 15× cost
    /// drop; the compactor's job is single-shot summarization, so a
    /// smaller model handles it fine. `None` = reuse `model`.
    ///
    /// The compaction *trigger* still uses `model`'s context window —
    /// only the summarizer call swaps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compactor_model: Option<String>,
}

/// Ceiling on tool-call rounds within a single user turn. Guards against
/// runaway loops (a model calling tools forever) without cutting real
/// refactors short. Set high so a genuine feature build or repo-wide
/// refactor never trips it in practice — the runaway-loop guard still
/// bites, just at 200 rounds instead of a number a real task can hit.
fn default_max_rounds() -> usize {
    200
}

/// Post-round auto-extractor configuration.
///
/// Attached via [`Session::with_auto_extract`]. When set + enabled, after
/// every user turn that included at least one tool call the harness spawns
/// a background task that: (1) calls the extractor model with a
/// fact-mining prompt, (2) parses the response into bullets, (3) dedups
/// against the last N episodic entries, (4) appends survivors to the
/// session's `EpisodicStore` (from `tool_ctx.episodic`).
///
/// Kept separate from [`SessionConfig`] so tests / callers that don't
/// want the feature can leave it unset and pay nothing.
#[derive(Clone, Debug)]
pub struct AutoExtractConfig {
    /// Master switch. Even when this struct is attached the extractor is
    /// a no-op if `enabled == false` — useful for per-repo disable via
    /// `mira.yaml` without dropping the extractor plumbing entirely.
    pub enabled: bool,
    /// Model passed to the extractor call. `None` = reuse the session's
    /// active model (expensive; usually you want a cheap tier).
    pub model: Option<String>,
}

impl AutoExtractConfig {
    /// Convenience: on with the session's model as fallback.
    pub fn enabled(model: Option<String>) -> Self {
        Self {
            enabled: true,
            model,
        }
    }
}

/// Retrieval-mode settings for the memory snapshot.
///
/// Defaults to enabled with the standard token budget; wire from the
/// runtime config via [`Session::with_memory_retrieval`] to override.
#[derive(Clone, Debug)]
pub struct MemoryRetrievalConfig {
    /// When `false`, the harness renders memory in legacy dump-everything
    /// mode (byte-capped, whole files pasted, episodic tail appended).
    pub enabled: bool,
    /// Token budget for the rendered memory block. `chars / 4` is the
    /// approximate cost function; the snapshot renders a `<!-- memory:
    /// used / budget tokens -->` marker so operators can tune this.
    pub token_budget: usize,
}

impl Default for MemoryRetrievalConfig {
    fn default() -> Self {
        // Retrieval-on by default: it degrades gracefully to
        // "everything fits under the budget" when memory is small, and
        // matters immediately once it grows.
        Self {
            enabled: true,
            token_budget: DEFAULT_TOKEN_BUDGET,
        }
    }
}

/// How many recent episodic entries to consider when deciding whether a
/// candidate is a duplicate. Small enough that dedup is O(N) with a
/// substring check; big enough to catch the "I just remembered that last
/// turn" case.
const DEDUP_LOOKBACK: usize = 30;

/// Hard wall-clock ceiling on one extraction call. If the extractor
/// provider hangs, we log and move on rather than leaking a task per
/// finished turn.
const EXTRACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Cap on how much round transcript we feed the extractor. Big rounds
/// with fat tool results would otherwise balloon the extraction prompt.
const EXTRACTION_INPUT_MAX: usize = 8000;

/// Cap on the extractor's own reply length. Enough for a handful of
/// bullets; short enough that a runaway extractor can't cost real money.
const EXTRACTION_OUTPUT_TOKENS: u32 = 512;

impl SessionConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_rounds: default_max_rounds(),
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            response_format: None,
            compactor_model: None,
        }
    }
}

/// A single conversation.
///
/// All mutable state (history, policy) is behind `Arc<Mutex<_>>` so [`send`]
/// can spawn its loop on a background task and stream events out without
/// holding a mutable borrow on `Session`. Cheap to `Clone` — every field is
/// an `Arc` or a small `Clone` value.
///
/// [`send`]: Session::send
#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    cfg: Arc<Mutex<SessionConfig>>,
    history: Arc<Mutex<Vec<Message>>>,
    /// Human-readable nickname. Generated post-hoc by the server after the
    /// first assistant reply; the harness itself only reads + persists it.
    title: Arc<Mutex<Option<String>>>,
    /// Per-turn wall-clock timing. Appended when [`Session::send`] pushes a
    /// user message; the last entry's `ended_at` is stamped when the loop
    /// emits its final `Done` event.
    turns: Arc<Mutex<Vec<TurnMeta>>>,
    /// Aggregate token usage. Folded in whenever the provider emits a usage
    /// trailer; persisted alongside the session.
    usage: Arc<Mutex<UsageTotals>>,
    created_at: u64,

    provider: Arc<dyn ChatProvider>,
    registry: Arc<Registry>,
    policy: Arc<Mutex<Policy>>,
    approver: Arc<dyn Approver>,
    tool_ctx: ToolContext,
    store: Option<Arc<dyn SessionStore>>,
    /// Renders the "live" memory block (user + project `MIRA.md`) on every
    /// round so mid-session edits — from `/remember`, from a memory tool,
    /// or straight from a text editor — reach the model on the very next
    /// turn. Persisted history keeps only the fixed system prefix; the
    /// live block is inserted at request time and never checkpointed.
    memory_snapshot: Option<Arc<dyn MemorySnapshot>>,
    /// Retrieval settings applied when the harness renders the memory
    /// snapshot. When `retrieval_enabled` is on, the harness builds a
    /// [`MemoryQuery`] from recent conversation and hands it to the
    /// snapshot so entries get scored + budgeted; when off, the snapshot
    /// falls back to dumping every file wholesale (legacy behaviour).
    memory_retrieval: MemoryRetrievalConfig,
    /// Post-round background extraction settings. See [`AutoExtractConfig`].
    auto_extract: Option<AutoExtractConfig>,
    /// Handle to the currently-running turn task, if any. `cancel()` aborts
    /// it; the loop's `tx.send` calls then fail as the channel closes and
    /// the frontend stops seeing new events.
    current_turn: Arc<Mutex<Option<AbortHandle>>>,
    /// When set, this session was spawned as a subagent by another
    /// session (parent). Copied into every checkpoint so the sidebar can
    /// hide subagents from the primary chat list and delete flows can
    /// cascade from the parent. `None` for top-level chats.
    parent_id: Option<SessionId>,
    /// Currently in-flight children spawned by this session — kept so an
    /// interrupt on the parent cascades to every subagent whose turn is
    /// still running. Each entry is a boxed cancel callback keyed by a
    /// monotonic id (`ChildTracker` contract) so `agent` can deregister
    /// on completion without racing new spawns.
    children: Arc<Mutex<HashMap<u64, Box<dyn ChildCancel>>>>,
    /// Monotonic counter that hands out ids for the `children` map. Wraps
    /// around at u64::MAX (effectively never in a real session).
    next_child_id: Arc<AtomicU64>,
    /// Session-scoped task list. Shared with `tool_ctx.tasks` (same
    /// Arc) so the `task_*` tools and the checkpoint path see one
    /// view.
    tasks: Arc<TaskStore>,
    /// Standing `/goal` — `None` means no autonomous loop. When set +
    /// `status.is_active()`, `run_loop` re-enters the round loop after a
    /// clean stop until the evaluator returns a terminal verdict or the
    /// iteration cap is hit.
    goal: Arc<Mutex<Option<Goal>>>,
    /// The current turn's event sender, when a turn is active. Long-
    /// running tools (bash today, others later) route live output
    /// through the [`ToolProgressSink`] attached to `tool_ctx`; that
    /// sink reads from this slot so a fresh turn always sees the right
    /// tx and lines from a cancelled turn don't leak into the next.
    progress_slot: ProgressSlot,
    /// Diff previews for edit/write calls, keyed by tool call id.
    /// Computed inside `dispatch_call` before the tool runs (so the
    /// "before" file state is still accurate), broadcast live as a
    /// `HarnessEvent::ToolPreview`, and persisted alongside the
    /// history so a reloaded transcript renders the same diff the
    /// user saw. Missing entries fall through to the client's
    /// arg-only fallback preview.
    previews: Arc<Mutex<HashMap<String, DiffPreview>>>,
    /// The cancellation token for the currently-running turn, if any.
    /// Installed by `run_loop` at turn start and cleared at the end.
    /// [`Session::cancel`] fires this BEFORE aborting the turn's tokio
    /// task, giving long-running tools (bash, web_fetch) a chance to
    /// clean up native resources (kill child processes, close sockets)
    /// instead of being torn down mid-await.
    current_cancel: Arc<Mutex<Option<CancellationToken>>>,
}

impl Session {
    pub fn new(
        cfg: SessionConfig,
        system_prompt: impl Into<String>,
        provider: Arc<dyn ChatProvider>,
        registry: Arc<Registry>,
        policy: Arc<Mutex<Policy>>,
        approver: Arc<dyn Approver>,
        mut tool_ctx: ToolContext,
    ) -> Self {
        let id = SessionId::new();
        // Stamp the session id on the tool context so tools that persist
        // provenance-tagged state (episodic memory, undo snapshots) see it.
        tool_ctx = tool_ctx.with_session_id(id.clone());
        // Attach a session-scoped FileGuard for conflict detection + undo.
        // Failure is logged and swallowed — a missing guard just means those
        // features are disabled for this session (files still get read /
        // written normally).
        if let Ok(g) = FileGuard::open(&id.to_string(), tool_ctx.cwd.clone()) {
            tool_ctx = tool_ctx.with_guard(Arc::new(g));
        } else {
            warn!(session = %id, "file guard init failed; undo + conflict detection disabled");
        }
        // Persistent bash: lazy — the shell struct doesn't fork bash until
        // the first bash command lands. Storing it here just means `cd`,
        // venvs, and env exports persist across calls for the whole session.
        //
        // The initial sandbox profile is derived from the policy's mode
        // so a session that starts in `plan` gets Restricted seatbelt
        // even before the user issues the first `/mode` command. Mode
        // changes later flow through `Session::set_sandbox_profile` and
        // respawn the shell on next use.
        // Grab the mode via try_lock — Session::new/resume_from are
        // sync, and at construction the caller owns the Arc so there's
        // no real contention. On the (impossible-in-practice) contended
        // branch we fall back to the default profile and log; the
        // profile will get corrected on the first `/mode` change.
        let initial_profile = policy
            .try_lock()
            .map(|p| profile_for_mode(p.mode()))
            .unwrap_or_else(|_| {
                warn!(
                    "policy lock contended during Session construction; defaulting sandbox profile"
                );
                SandboxProfile::default()
            });
        let cwd_for_shell = tool_ctx.cwd.clone();
        tool_ctx = tool_ctx.with_shell(Arc::new(Mutex::new(PersistentShell::with_profile(
            cwd_for_shell,
            true,
            initial_profile,
        ))));
        // Wire the child-tracker onto ToolContext so the `agent` tool can
        // register any subagent it spawns with this session's `children`
        // map. Sharing the Arcs (rather than a getter) means the tracker
        // and Session point at the exact same slot — no drift on rebuild.
        let children = Arc::new(Mutex::new(HashMap::new()));
        let next_child_id = Arc::new(AtomicU64::new(0));
        let tracker: Arc<dyn ChildTracker> = Arc::new(SessionChildTracker {
            children: children.clone(),
            next_id: next_child_id.clone(),
        });
        tool_ctx = tool_ctx.with_child_tracker(tracker);
        // Fresh session → empty task store.
        let tasks = TaskStore::new();
        tool_ctx = tool_ctx.with_tasks(tasks.clone());
        // Progress bridge: shared slot the harness parks the current
        // turn's tx into. The sink itself is a thin wrapper over the
        // slot; both live for the session's lifetime.
        let progress_slot: ProgressSlot = Arc::new(StdMutex::new(None));
        let progress_sink: Arc<dyn ToolProgressSink> =
            Arc::new(TurnProgress::new(progress_slot.clone()));
        tool_ctx = tool_ctx.with_progress(progress_sink);
        Self {
            id,
            cfg: Arc::new(Mutex::new(cfg)),
            history: Arc::new(Mutex::new(vec![Message::system(system_prompt)])),
            title: Arc::new(Mutex::new(None)),
            turns: Arc::new(Mutex::new(Vec::new())),
            usage: Arc::new(Mutex::new(UsageTotals::default())),
            created_at: now_secs(),
            provider,
            registry,
            policy,
            approver,
            tool_ctx,
            tasks,
            store: None,
            memory_snapshot: None,
            memory_retrieval: MemoryRetrievalConfig::default(),
            auto_extract: None,
            current_turn: Arc::new(Mutex::new(None)),
            parent_id: None,
            children,
            next_child_id,
            goal: Arc::new(Mutex::new(None)),
            progress_slot,
            previews: Arc::new(Mutex::new(HashMap::new())),
            current_cancel: Arc::new(Mutex::new(None)),
        }
    }

    /// Rehydrate from a persisted record — same wiring as [`Session::new`]
    /// but the id, history, config, and creation timestamp come from disk.
    /// Attach a store afterwards with [`Session::with_store`] to keep
    /// autosaving.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_from(
        record: SessionRecord,
        provider: Arc<dyn ChatProvider>,
        registry: Arc<Registry>,
        policy: Arc<Mutex<Policy>>,
        approver: Arc<dyn Approver>,
        mut tool_ctx: ToolContext,
    ) -> Self {
        // Same guard wiring as `new` — seq counter picks up where the
        // previous run left off (see FileGuard::open).
        tool_ctx = tool_ctx.with_session_id(record.id.clone());
        if let Ok(g) = FileGuard::open(&record.id.to_string(), tool_ctx.cwd.clone()) {
            tool_ctx = tool_ctx.with_guard(Arc::new(g));
        } else {
            warn!(session = %record.id, "file guard init failed on resume");
        }
        // Resumed sessions get a fresh shell (bash state doesn't survive a
        // restart), but `cd` + env persistence resumes from the next call.
        // Same mode-derived profile treatment as `Session::new`.
        // Grab the mode via try_lock — Session::new/resume_from are
        // sync, and at construction the caller owns the Arc so there's
        // no real contention. On the (impossible-in-practice) contended
        // branch we fall back to the default profile and log; the
        // profile will get corrected on the first `/mode` change.
        let initial_profile = policy
            .try_lock()
            .map(|p| profile_for_mode(p.mode()))
            .unwrap_or_else(|_| {
                warn!(
                    "policy lock contended during Session construction; defaulting sandbox profile"
                );
                SandboxProfile::default()
            });
        let cwd_for_shell = tool_ctx.cwd.clone();
        tool_ctx = tool_ctx.with_shell(Arc::new(Mutex::new(PersistentShell::with_profile(
            cwd_for_shell,
            true,
            initial_profile,
        ))));
        // Same tracker wiring as `new` — resumed sessions can still spawn
        // subagents and their turns should cascade-cancel with the parent.
        let children = Arc::new(Mutex::new(HashMap::new()));
        let next_child_id = Arc::new(AtomicU64::new(0));
        let tracker: Arc<dyn ChildTracker> = Arc::new(SessionChildTracker {
            children: children.clone(),
            next_id: next_child_id.clone(),
        });
        tool_ctx = tool_ctx.with_child_tracker(tracker);
        // Rehydrate the task store from the persisted snapshot — id
        // sequence continues past the largest we saw so nothing gets
        // reassigned.
        let tasks = TaskStore::restore(record.tasks);
        tool_ctx = tool_ctx.with_tasks(tasks.clone());
        // Same progress bridge as `new` — resumed sessions stream
        // bash output too.
        let progress_slot: ProgressSlot = Arc::new(StdMutex::new(None));
        let progress_sink: Arc<dyn ToolProgressSink> =
            Arc::new(TurnProgress::new(progress_slot.clone()));
        tool_ctx = tool_ctx.with_progress(progress_sink);
        Self {
            id: record.id,
            cfg: Arc::new(Mutex::new(record.cfg)),
            history: Arc::new(Mutex::new(record.messages)),
            title: Arc::new(Mutex::new(record.title)),
            turns: Arc::new(Mutex::new(record.turns)),
            usage: Arc::new(Mutex::new(record.usage)),
            created_at: record.created_at,
            provider,
            registry,
            policy,
            approver,
            tool_ctx,
            tasks,
            store: None,
            memory_snapshot: None,
            memory_retrieval: MemoryRetrievalConfig::default(),
            auto_extract: None,
            current_turn: Arc::new(Mutex::new(None)),
            parent_id: record.parent_id,
            children,
            next_child_id,
            goal: Arc::new(Mutex::new(record.goal)),
            progress_slot,
            previews: Arc::new(Mutex::new(record.previews)),
            current_cancel: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach a store so the session autosaves after each round.
    pub fn with_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Mark this session as a subagent spawned by `parent`. The id is
    /// serialized on every checkpoint so the sidebar can hide subagents
    /// from the primary chat list and future delete flows can cascade
    /// from the parent.
    pub fn with_parent_id(mut self, parent: SessionId) -> Self {
        self.parent_id = Some(parent);
        self
    }

    /// Attach a memory snapshot. When set, the harness reads it before
    /// every provider round and injects the rendered block as a second
    /// system message. The block is *not* checkpointed into history — so
    /// resumed sessions render fresh from disk, and mid-session edits are
    /// picked up on the next turn.
    pub fn with_memory_snapshot(mut self, snapshot: Arc<dyn MemorySnapshot>) -> Self {
        self.memory_snapshot = Some(snapshot);
        self
    }

    /// Configure how the memory snapshot is rendered — off = legacy
    /// dump; on = retrieval-scored under a token budget. Default is on
    /// with [`DEFAULT_TOKEN_BUDGET`]; wire from [`mira_config::MemoryRuntimeConfig`]
    /// to expose it in `mira.yaml`.
    pub fn with_memory_retrieval(mut self, cfg: MemoryRetrievalConfig) -> Self {
        self.memory_retrieval = cfg;
        self
    }

    /// Enable post-round auto-extraction. Only fires when the session's
    /// `tool_ctx.episodic` handle is also set (nothing to write to
    /// otherwise). See [`AutoExtractConfig`] for the settings.
    pub fn with_auto_extract(mut self, cfg: AutoExtractConfig) -> Self {
        self.auto_extract = Some(cfg);
        self
    }

    /// Snapshot the current history.
    pub async fn history(&self) -> Vec<Message> {
        self.history.lock().await.clone()
    }

    pub async fn config(&self) -> SessionConfig {
        self.cfg.lock().await.clone()
    }

    /// Read the current nickname, if one has been generated.
    pub async fn title(&self) -> Option<String> {
        self.title.lock().await.clone()
    }

    /// Snapshot of per-turn timing. Same ordering as user messages in
    /// `history()`.
    pub async fn turns(&self) -> Vec<TurnMeta> {
        self.turns.lock().await.clone()
    }

    /// Snapshot the aggregate token usage across every provider round in
    /// this session so far.
    pub async fn usage(&self) -> UsageTotals {
        *self.usage.lock().await
    }

    /// Non-deleted tasks in the session's todo list. UI reads this to
    /// hydrate the task panel on load / reconnect.
    pub async fn tasks(&self) -> Vec<mira_tools::TaskItem> {
        self.tasks.list().await
    }

    /// Snapshot the standing goal. Returns `None` when no goal has been
    /// set on this session (or the last one was cleared).
    pub async fn goal(&self) -> Option<Goal> {
        self.goal.lock().await.clone()
    }

    /// Snapshot the captured diff previews (keyed by tool call id).
    /// The server pulls this on Ready so a reloaded transcript can
    /// restore the same diffs that rendered live.
    pub async fn previews(&self) -> HashMap<String, DiffPreview> {
        self.previews.lock().await.clone()
    }

    /// Replace the standing goal. Any previous goal — active or
    /// terminal — is dropped in favour of the new one. Emits
    /// `HarnessEvent::GoalSet` on the caller's event stream is the
    /// caller's job (the harness itself emits only on the round loop's
    /// event channel). Persists immediately.
    pub async fn set_goal(&self, goal: Goal) {
        *self.goal.lock().await = Some(goal);
        checkpoint(self).await;
    }

    /// Drop the standing goal. Idempotent — clearing a session with no
    /// goal is a no-op. Persists immediately.
    pub async fn clear_goal(&self) {
        *self.goal.lock().await = None;
        checkpoint(self).await;
    }

    /// Expose the session's undo/conflict guard. `None` when the FileGuard
    /// failed to initialise (see the warn! in `new` / `resume_from`).
    pub fn file_guard(&self) -> Option<Arc<FileGuard>> {
        self.tool_ctx.guard.clone()
    }

    /// Stamp `ended_at` on the most recent open turn, if any. Idempotent —
    /// calling twice on the same turn keeps the first end time. Used by
    /// callers that emit `HarnessEvent::Done` outside the harness's own
    /// loop (e.g. `Interrupt` in the server WS handler).
    pub async fn end_current_turn(&self) {
        let mut guard = self.turns.lock().await;
        if let Some(last) = guard.last_mut() {
            if last.ended_at.is_none() {
                last.ended_at = Some(now_ms());
            }
        }
    }

    /// Set the nickname and persist immediately. Callers are expected to
    /// have generated a sensible short title; the harness doesn't validate
    /// content beyond trimming whitespace and enforcing a hard cap so a
    /// runaway model can't stuff the sidebar with a paragraph.
    pub async fn set_title(&self, title: impl Into<String>) {
        let mut t = title.into().trim().to_string();
        if t.is_empty() {
            return;
        }
        const MAX: usize = 80;
        if t.chars().count() > MAX {
            t = t.chars().take(MAX).collect();
        }
        *self.title.lock().await = Some(t);
        // Flush a checkpoint so a crash/reload after title generation still
        // shows the nickname. Failures are logged inside `checkpoint`.
        checkpoint(self).await;
    }

    /// Hot-swap the model. Applies to the next `send()` — an in-flight turn
    /// finishes on the model it started with, since `run_loop` snapshots the
    /// config at spawn time.
    pub async fn set_model(&self, model: impl Into<String>) {
        self.cfg.lock().await.model = model.into();
    }

    /// Set the reasoning-effort field for future turns. `None` clears it so
    /// non-reasoning models aren't hit with an ignored parameter. Same
    /// snapshot-at-spawn caveat as `set_model` — in-flight turn keeps the
    /// prior value.
    pub async fn set_reasoning_effort(&self, effort: Option<String>) {
        self.cfg.lock().await.reasoning_effort = effort;
    }

    /// Update the persistent shell's sandbox profile — usually called in
    /// response to a `/mode` change. When the profile actually changes,
    /// the shell marks itself dirty so the next bash call respawns with
    /// the new seatbelt policy; a `plan → auto` switch takes effect
    /// without a session restart (`cd` / `export` state is lost on
    /// respawn — the trade for a real containment change).
    pub async fn set_sandbox_profile(&self, profile: SandboxProfile) {
        if let Some(shell) = &self.tool_ctx.shell {
            shell.lock().await.set_profile(profile);
        }
    }

    /// Run one user turn to completion.
    ///
    /// The returned stream ends with [`HarnessEvent::Done`]. Callers may
    /// drop it early to cancel — the task keeps mutating history until it
    /// hits a checkpoint, then exits when the send channel closes.
    pub async fn send(&self, user_input: impl Into<String>) -> BoxStream<'static, HarnessEvent> {
        // Repair history before appending the new user turn. A prior
        // interrupt can abort the loop between `history.push(assistant_msg)`
        // (with tool_calls) and the matching `Message::tool(...)` push in
        // `dispatch_call`, leaving orphaned tool_calls in the transcript.
        // Providers (both Anthropic and OpenAI) reject that shape on the
        // next request, and even when they don't, the extra reasoning
        // burns latency. Synthesize a short error tool result for each
        // dangling call so the transcript stays well-formed.
        {
            let mut hist = self.history.lock().await;
            let repaired = repair_dangling_tool_calls(&mut hist);
            if repaired > 0 {
                warn!(
                    count = repaired,
                    "history: repaired dangling tool_calls left by a prior interrupt"
                );
            }
        }
        self.history.lock().await.push(Message::user(user_input));
        // Open a new turn timer; `run_loop` stamps `ended_at` on the way out.
        self.turns.lock().await.push(TurnMeta {
            started_at: now_ms(),
            ended_at: None,
        });

        let cfg = self.cfg.lock().await.clone();
        let this = self.clone();
        let (tx, rx) = mpsc::channel::<HarnessEvent>(64);

        // If a prior turn is still running (shouldn't happen with a
        // well-behaved UI but easy to hit while debugging), abort it —
        // otherwise two tasks race to mutate history.
        let handle = tokio::spawn(async move { run_loop(this, cfg, tx).await });
        let mut slot = self.current_turn.lock().await;
        if let Some(prev) = slot.take() {
            prev.abort();
        }
        *slot = Some(handle.abort_handle());

        ReceiverStream::new(rx).boxed()
    }

    /// Cancel the currently-running turn, if any. Any in-flight tool call
    /// finishes on its own thread (we don't kill child processes), but the
    /// model stream stops pumping events and the next round never starts.
    ///
    /// Cascades to any subagents this session spawned that are still
    /// running: each registered child gets its own `cancel()` fired via
    /// `tokio::spawn` (fire-and-forget) so pressing Stop on the parent
    /// halts every layer of delegated work at once.
    pub async fn cancel(&self) -> bool {
        // Fire the cooperative cancel first so tools observing the
        // token (bash, web_fetch) can clean up native resources
        // (kill child processes, close sockets) before the outer
        // tokio task is torn down by `abort()` below. A tool that
        // doesn't observe the token still gets aborted — this is
        // strictly additive.
        if let Some(token) = self.current_cancel.lock().await.take() {
            token.cancel();
        }

        let cancelled_self = {
            let mut slot = self.current_turn.lock().await;
            match slot.take() {
                Some(h) => {
                    h.abort();
                    true
                }
                None => false,
            }
        };

        // Drain the child map and fire each child's cancel concurrently.
        // We drain (rather than clone) so a lingering child from a
        // previous turn that never de-registered doesn't get cancelled
        // twice on a later interrupt.
        let children: Vec<Box<dyn ChildCancel>> = {
            let mut guard = self.children.lock().await;
            guard.drain().map(|(_, c)| c).collect()
        };
        for child in children {
            tokio::spawn(async move {
                child.cancel().await;
            });
        }

        cancelled_self
    }

    /// Expose the shared child-tracker so callers can wire it into a
    /// spawned subagent's `ToolContext`. Each subagent registers itself
    /// on entry and de-registers on completion; parent's `cancel()`
    /// walks the map.
    pub fn child_tracker(&self) -> Arc<dyn ChildTracker> {
        Arc::new(SessionChildTracker {
            children: self.children.clone(),
            next_id: self.next_child_id.clone(),
        })
    }
}

/// Concrete `ChildTracker` implementation backed by the shared
/// `Session.children` map. Hidden behind the trait so mira-tools
/// (where `ChildTracker` lives) doesn't take a dependency on the
/// harness types.
struct SessionChildTracker {
    children: Arc<Mutex<HashMap<u64, Box<dyn ChildCancel>>>>,
    next_id: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl ChildTracker for SessionChildTracker {
    async fn register(&self, cancel: Box<dyn ChildCancel>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.children.lock().await.insert(id, cancel);
        id
    }

    async fn deregister(&self, id: u64) {
        self.children.lock().await.remove(&id);
    }
}

// ---- the loop ----

/// Cap on how many times a single turn will inject verify failures back
/// into the model before giving up. Prevents an unfixable error from
/// looping the turn forever; the user still sees a warning frame and can
/// intervene manually.
const MAX_VERIFY_ATTEMPTS: usize = 3;

/// How many consecutive rounds with an identical tool-call signature
/// (same set of `(name, canonicalized-args)`) constitutes a stuck loop.
/// On the Nth such round the harness skips dispatch, emits a warning,
/// and injects a "try a different approach" nudge as a user message
/// so the model breaks out on its own instead of grinding through the
/// full 200-round `max_rounds` ceiling.
///
/// 3 = allow two identical rounds (retry with the same args once) but
/// intervene on the third. Loose enough that a legitimate retry after
/// a transient tool failure passes; tight enough to catch the classic
/// "grep for the same pattern forever" and "read the same file 4 times"
/// patterns cheaply.
const STUCK_LOOP_THRESHOLD: usize = 3;

async fn run_loop(sess: Session, cfg: SessionConfig, tx: mpsc::Sender<HarnessEvent>) {
    // Park a clone of the current turn's tx into the shared progress
    // slot so tools that stream live output (bash's PTY reader) can
    // fan lines into this turn's event stream. Cleared in the guard
    // below when the turn winds down so cancelled / dead tools can't
    // spray into a subsequent turn.
    if let Ok(mut slot) = sess.progress_slot.lock() {
        *slot = Some(tx.clone());
    }
    let _progress_guard = ProgressSlotGuard::new(sess.progress_slot.clone());

    // Fresh cancellation token for this turn. Stored on the session
    // so `Session::cancel` can fire it; also threaded through
    // `dispatch_call` into each tool invocation's `ToolContext` so
    // long-running tools observe it. Cleared on turn wind-down.
    let turn_cancel = CancellationToken::new();
    *sess.current_cancel.lock().await = Some(turn_cancel.clone());
    let _cancel_guard = CancelSlotGuard {
        slot: sess.current_cancel.clone(),
    };

    // Outer `'goal_loop` wraps the per-user-turn round loop. When a
    // standing `/goal` is active it drives the evaluator after each
    // clean stop and, on `not_met`, injects a synthetic "keep going"
    // user message + a fresh TurnMeta and re-enters the round loop for
    // another autonomous turn. Sessions without a goal make exactly
    // one pass through this outer loop — the shape matches the
    // pre-goal behavior exactly.
    'goal_loop: loop {
        // Persist the user's turn-opening message right away so the sidebar
        // shows the new thread as soon as they hit send — before the model's
        // first response comes back. Without this, `list_sessions` (which
        // walks disk) can't see the session because nothing has flushed yet.
        checkpoint(&sess).await;

        // Snapshot the set of paths already written to at turn start so a
        // later diff tells us what *this* turn touched. When there's no
        // FileGuard, apply-verify is disabled entirely (empty set → no
        // detected writes → verify skipped). Recomputed each goal
        // iteration so a second autonomous turn only verifies its own
        // new writes.
        let writes_at_turn_start: std::collections::HashSet<std::path::PathBuf> =
            match sess.tool_ctx.guard.as_ref() {
                Some(g) => g.written_snapshot().await,
                None => std::collections::HashSet::new(),
            };
        let mut verify_attempts = 0usize;

        // Where the auto-extractor's "just-finished round" slice starts. The
        // user message for this turn was pushed either in `Session::send`
        // (first goal iteration) or by us as a synthetic continuation at
        // the bottom of the previous iteration — either way it's at
        // len()-1.
        let turn_start_idx = sess.history.lock().await.len().saturating_sub(1);
        // Auto-extractor gate: only fire when the turn actually did work AND
        // at least one tool call succeeded. Skipping error-only turns matters
        // because a session where (say) every `memory_read` returned "not
        // wired" would otherwise get summarized into episodic memory and
        // pollute every future session — self-poisoning loop.
        let mut any_successful_tool_call = false;

        // Outcome of the round loop below. Drives whether the outer
        // 'goal_loop continues (goal-check tick), exits early (provider
        // error), or emits a hit-max-rounds warning before checking.
        #[derive(PartialEq, Eq)]
        enum RoundOutcome {
            /// Clean stop from the model — verify (if any) already ran.
            CleanStop,
            /// Ran out of `cfg.max_rounds` inside a single autonomous
            /// turn. Common when the model dispatches lots of tools
            /// without stopping; we still tick the goal loop.
            MaxRounds,
            /// Provider failed on the initial stream call. Bail on the
            /// whole send (goal doesn't get a chance to retry — the
            /// provider might be genuinely down).
            ProviderError,
            /// Mid-stream failure (SSE dropped, wall-clock timeout, or
            /// upstream `Err(_)` frame). Distinct from `CleanStop` so
            /// the goal evaluator doesn't judge a truncated transcript
            /// as if the model finished on its own.
            StreamError,
        }
        let mut round_outcome = RoundOutcome::MaxRounds;

        // Stuck-loop detector. Fingerprint of the previous round's
        // tool-call set (name + canonicalized args, order-independent).
        // Rounds with an identical fingerprint accumulate; on the Nth
        // in a row we skip dispatch and inject a "try something
        // different" nudge instead of grinding through max_rounds.
        let mut prev_call_fingerprint: Option<u64> = None;
        let mut consecutive_dupe_rounds: usize = 0;

        for round in 0..cfg.max_rounds {
            info!(round, "harness: model turn");

            // Rolling compaction: if history has grown past the trigger,
            // summarize the older tail via the provider and splice a
            // single synthetic user message in its place. Kept inside the
            // round loop so a long turn with many tool calls can also
            // trigger it (not just the between-turn edge). Failure is
            // logged and swallowed — running with un-compacted history is
            // strictly better than aborting the turn.
            {
                let mut history = sess.history.lock().await;
                let summarizer_model = cfg.compactor_model.as_deref().unwrap_or(&cfg.model);
                match crate::history::maybe_compact(
                    &mut history,
                    sess.provider.as_ref(),
                    &cfg.model,
                    summarizer_model,
                )
                .await
                {
                    Ok(Some(n)) => {
                        let _ = tx
                            .send(HarnessEvent::Compacted {
                                messages_removed: n,
                            })
                            .await;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, "compaction failed; continuing with full history");
                    }
                }
            }

            let req = ChatRequest {
                model: cfg.model.clone(),
                messages: build_request_messages(&sess).await,
                tools: sess.registry.specs(),
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
                reasoning_effort: cfg.reasoning_effort.clone(),
                response_format: cfg.response_format.clone(),
            };

            let mut stream = match sess.provider.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx
                        .send(HarnessEvent::Warning(format!("provider error: {e}")))
                        .await;
                    round_outcome = RoundOutcome::ProviderError;
                    break;
                }
            };

            let mut assistant_text = String::new();
            let mut pending_calls: Vec<ToolCall> = Vec::new();
            let mut finish: FinishReason = FinishReason::Other;
            // Tracks whether the stream ended cleanly (Done frame) or
            // via error/timeout. Only a clean Done keeps CleanStop
            // eligibility; anything else routes into `StreamError` so
            // the goal evaluator can skip a truncated transcript
            // instead of scoring the model on partial output.
            let mut stream_errored = false;
            // Per-round wall-clock guard on the provider stream — a
            // hung SSE socket used to park the turn indefinitely. The
            // ceiling matches other bounded LLM calls (evaluator /
            // extractor) so a legitimate long stream still completes;
            // pathological hangs surface as `StreamError`.
            const STREAM_TIMEOUT_SECS: u64 = 300;
            let stream_deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

            loop {
                let next = tokio::time::timeout_at(stream_deadline, stream.next()).await;
                match next {
                    Err(_) => {
                        stream_errored = true;
                        let _ = tx
                            .send(HarnessEvent::Warning(format!(
                                "stream timed out after {STREAM_TIMEOUT_SECS}s — turn aborted"
                            )))
                            .await;
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(evt)) => match evt {
                        Ok(ChatEvent::TextDelta(t)) => {
                            assistant_text.push_str(&t);
                            if tx.send(HarnessEvent::Token(t)).await.is_err() {
                                return;
                            }
                        }
                        Ok(ChatEvent::ToolCalls(calls)) => {
                            pending_calls = calls;
                        }
                        Ok(ChatEvent::Usage(round)) => {
                            let totals = {
                                let mut u = sess.usage.lock().await;
                                u.add_round(round);
                                *u
                            };
                            // Ignore send errors — a dropped receiver just means the
                            // UI stopped listening; the totals are still recorded.
                            let _ = tx.send(HarnessEvent::Usage { round, totals }).await;
                        }
                        Ok(ChatEvent::Done(reason)) => {
                            finish = reason;
                            break;
                        }
                        Err(e) => {
                            stream_errored = true;
                            let _ = tx
                                .send(HarnessEvent::Warning(format!("stream error: {e}")))
                                .await;
                            break;
                        }
                    },
                }
            }
            // Fold stream error / timeout into the round outcome so the
            // goal-loop tick below sees StreamError instead of falling
            // through to CleanStop with a truncated assistant message.
            if stream_errored {
                round_outcome = RoundOutcome::StreamError;
            }

            // Record the assistant turn — may carry text, tool calls, or both.
            let assistant_msg = if pending_calls.is_empty() {
                Message::assistant(assistant_text.clone())
            } else if assistant_text.is_empty() {
                Message::assistant_calls(pending_calls.clone())
            } else {
                let mut m = Message::assistant(assistant_text.clone());
                m.tool_calls = pending_calls.clone();
                m
            };
            sess.history.lock().await.push(assistant_msg);
            checkpoint(&sess).await;
            let _ = tx.send(HarnessEvent::TurnComplete).await;

            if pending_calls.is_empty() || finish == FinishReason::Stop {
                // Apply-verify: if the model wrote source files this turn, run
                // the project's natural safety check (cargo check / tsc / …).
                // On failure, feed the errors back and let the model take one
                // more crack at it — up to MAX_VERIFY_ATTEMPTS total.
                if verify_attempts < MAX_VERIFY_ATTEMPTS {
                    if let Some((check, output)) =
                        run_verify(&sess, &writes_at_turn_start, &tx).await
                    {
                        verify_attempts += 1;
                        // Inject the failure as a user message so the next
                        // model round sees it as fresh feedback (rather than
                        // as a tool_result which requires a matching call).
                        let synthetic = format!(
                            "The `{}` check just failed after your last edits:\n\n\
                         ```\n{}\n```\n\n\
                         Fix the errors and continue. You have {} more automatic \
                         verify retries before I stop.",
                            check.name,
                            truncate_for_history(&output),
                            MAX_VERIFY_ATTEMPTS - verify_attempts,
                        );
                        sess.history.lock().await.push(Message::user(synthetic));
                        continue;
                    }
                } else {
                    // We hit the retry cap. Emit a warning so the user knows
                    // and doesn't wonder why the errors are still there.
                    let _ = tx
                        .send(HarnessEvent::Warning(format!(
                        "[verify] still failing after {MAX_VERIFY_ATTEMPTS} attempts — stopping"
                    )))
                        .await;
                }

                round_outcome = RoundOutcome::CleanStop;
                break;
            }

            // Stuck-loop guard. If the model is issuing the exact same
            // set of tool calls (name + canonicalized args, order-
            // independent) round after round, it's not making progress
            // — a nudge is cheaper than letting it grind to `max_rounds`.
            let fp = fingerprint_calls(&pending_calls);
            if Some(fp) == prev_call_fingerprint {
                consecutive_dupe_rounds += 1;
            } else {
                consecutive_dupe_rounds = 1;
                prev_call_fingerprint = Some(fp);
            }
            if consecutive_dupe_rounds >= STUCK_LOOP_THRESHOLD {
                warn!(
                    consecutive = consecutive_dupe_rounds,
                    "stuck-loop detected — skipping dispatch, injecting nudge"
                );
                let _ = tx
                    .send(HarnessEvent::Warning(format!(
                        "[stuck-loop] the model has called the same tools with the same arguments \
                         {consecutive_dupe_rounds} rounds in a row — asking it to try a different \
                         approach"
                    )))
                    .await;
                sess.history.lock().await.push(Message::user(
                    "You've made the same set of tool calls with the same arguments several rounds \
                     in a row. That's a strong sign the current approach isn't making progress. \
                     Try something different: a narrower or wider query, a different tool, a \
                     different file, or step back and reconsider the plan. If you genuinely can't \
                     make progress, stop and tell me what you tried and what's blocking you."
                        .to_owned(),
                ));
                // Reset so we don't fire again on the very next round if
                // the model happens to repeat once more; it gets a fresh
                // window to change course.
                consecutive_dupe_rounds = 0;
                prev_call_fingerprint = None;
                continue;
            }

            // Dispatch calls in batches. Consecutive parallel-safe calls
            // (`Tool::parallel_safe(...)` = true) run concurrently via
            // `join_all`; anything else stays sequential. Preserves relative
            // order across batches so `edit → agent → read` semantics stay
            // intact — the model expects the batch of writes to land before
            // the reads that follow.
            let batches = plan_dispatch_batches(&sess, pending_calls).await;
            for batch in batches {
                if batch.calls.len() == 1 || !batch.parallel {
                    for call in batch.calls {
                        if dispatch_call(&sess, call, &tx).await {
                            any_successful_tool_call = true;
                        }
                    }
                } else {
                    let futures: Vec<_> = batch
                        .calls
                        .into_iter()
                        .map(|call| dispatch_call(&sess, call, &tx))
                        .collect();
                    let outcomes = futures::future::join_all(futures).await;
                    if outcomes.into_iter().any(|ok| ok) {
                        any_successful_tool_call = true;
                    }
                }
            }
            checkpoint(&sess).await;
        }
        // ---- end of round loop ----

        // Common cleanup: close out the turn's timer, flush a checkpoint,
        // fire the auto-extractor. Runs for every outcome so the UI + on
        // -disk state are consistent whether we hit max_rounds, cleaned up
        // gracefully, or plan to loop again for a goal iteration.
        sess.end_current_turn().await;
        checkpoint(&sess).await;
        maybe_spawn_extractor(
            &sess,
            &cfg,
            turn_start_idx,
            any_successful_tool_call,
            tx.clone(),
        )
        .await;

        if round_outcome == RoundOutcome::MaxRounds {
            let _ = tx
                .send(HarnessEvent::Warning(format!(
                    "hit max_rounds ({}) — send `continue` to resume, or raise `max_rounds` in mira.yaml",
                    cfg.max_rounds
                )))
                .await;
        }
        if round_outcome == RoundOutcome::ProviderError {
            // Genuine provider failure — the evaluator would just fail
            // the same way. Exit the whole send.
            break 'goal_loop;
        }
        if round_outcome == RoundOutcome::StreamError {
            // Mid-stream drop / wall-clock timeout — the transcript is
            // truncated, so grading the goal on this iteration would be
            // meaningless (and could flip a passing goal to `impossible`
            // on an infra glitch). Exit the send; the next user turn
            // can retry cleanly.
            break 'goal_loop;
        }

        // ---- goal-loop tick ----
        //
        // If a `/goal` is set and still active, run the evaluator and
        // decide whether to keep going, terminate, or fall through.
        let goal_snapshot = sess.goal.lock().await.clone();
        let Some(mut current_goal) = goal_snapshot else {
            break 'goal_loop; // no goal → single-pass behaviour
        };
        if !current_goal.status.is_active() {
            break 'goal_loop; // already terminal
        }
        if current_goal.iterations >= current_goal.max_iterations {
            current_goal.status = GoalStatus::Exhausted;
            current_goal.last_reason = Some(format!(
                "hit iteration cap of {}",
                current_goal.max_iterations
            ));
            *sess.goal.lock().await = Some(current_goal.clone());
            checkpoint(&sess).await;
            record_goal_outcome_to_episodic(&sess, &current_goal).await;
            let _ = tx
                .send(HarnessEvent::GoalDone {
                    status: current_goal.status,
                    reason: current_goal.last_reason.clone(),
                })
                .await;
            break 'goal_loop;
        }

        // External verifier gate. When the goal declares a
        // `verify` command, we run it BEFORE the LLM evaluator on
        // every iteration:
        //   - fail → short-circuit to `NotMet` with the command's
        //     tail-output as the reason; SKIP the LLM call entirely
        //     (this is the anti-gaming fix — a model that claims
        //     "tests pass" can't lie its way past `cargo test`).
        //   - pass → still call the LLM evaluator; the script is a
        //     *necessary* condition, not a sufficient one (tests can
        //     pass while the API is nonsensical).
        //   - run error → treated as `NotMet` with a warning so a
        //     broken verify command doesn't wedge the goal loop.
        let condition = current_goal.condition.clone();
        let mut short_circuit_eval: Option<goal::Evaluation> = None;
        if let Some(verify) = current_goal.verify.clone() {
            match goal::run_verify(&sess.tool_ctx.sandbox, &sess.tool_ctx.cwd, &verify).await {
                Ok(outcome) if outcome.passed => {
                    // Fall through to the LLM evaluator.
                }
                Ok(outcome) => {
                    // Verify failed — short-circuit to NotMet with the
                    // command's output as the reason. The LLM never sees
                    // the transcript this iteration, so we save both cost
                    // and a wrong verdict.
                    short_circuit_eval = Some(goal::Evaluation {
                        verdict: GoalVerdict::NotMet,
                        reason: outcome.short_reason(),
                    });
                }
                Err(err) => {
                    warn!(%err, "goal verify command errored; treating as not_met");
                    let _ = tx
                        .send(HarnessEvent::Warning(format!(
                            "[goal] verify command errored: {err} — treating as not_met"
                        )))
                        .await;
                    short_circuit_eval = Some(goal::Evaluation {
                        verdict: GoalVerdict::NotMet,
                        reason: format!("verify error: {err}"),
                    });
                }
            }
        }

        // Evaluate. Failures are downgraded to `not_met` with a warning
        // so a transient provider blip doesn't kill an in-progress
        // autonomous run — the next iteration gets another shot.
        let eval_model = current_goal
            .evaluator_model
            .clone()
            .unwrap_or_else(|| cfg.model.clone());
        let transcript = sess.history.lock().await.clone();
        let eval = if let Some(pre) = short_circuit_eval {
            pre
        } else {
            match goal::evaluate(sess.provider.as_ref(), &eval_model, &condition, &transcript).await
            {
                Ok(e) => e,
                Err(err) => {
                    warn!(%err, "goal evaluator failed; treating as not_met");
                    let _ = tx
                        .send(HarnessEvent::Warning(format!(
                            "[goal] evaluator failed: {err} — treating as not_met"
                        )))
                        .await;
                    goal::Evaluation {
                        verdict: GoalVerdict::NotMet,
                        reason: format!("evaluator error: {err}"),
                    }
                }
            }
        };

        // Bump iteration + record reason. The `Active` case leaves the
        // status untouched so a `not_met` verdict keeps looping.
        current_goal.iterations += 1;
        current_goal.last_reason = Some(eval.reason.clone());
        let iteration_now = current_goal.iterations;
        let max_iterations = current_goal.max_iterations;
        match eval.verdict {
            GoalVerdict::Met => current_goal.status = GoalStatus::Met,
            GoalVerdict::Impossible => current_goal.status = GoalStatus::Impossible,
            GoalVerdict::NeedsUser => current_goal.status = GoalStatus::NeedsUser,
            GoalVerdict::NotMet => {}
        }

        // Budget check. Runs even when the LLM said Met — a cap
        // breach is a terminal state either way, but we want a
        // truthful reason. Only overrides an *Active* verdict; a
        // clean `Met` from the LLM wins over a budget breach that
        // happens on the same iteration (the work finished, cost
        // just crept over — no reason to hide the success).
        if current_goal.status.is_active() {
            let usage_now = *sess.usage.lock().await;
            match goal::check_budget(
                current_goal.budget_tokens,
                current_goal.budget_usd,
                &cfg.model,
                usage_now,
            ) {
                goal::BudgetCheck::Exceeded(reason) => {
                    current_goal.status = GoalStatus::Exhausted;
                    current_goal.last_reason = Some(reason);
                }
                goal::BudgetCheck::Skipped | goal::BudgetCheck::Under => {}
            }
        }

        let status_now = current_goal.status;
        *sess.goal.lock().await = Some(current_goal.clone());
        checkpoint(&sess).await;

        let _ = tx
            .send(HarnessEvent::GoalProgress {
                iteration: iteration_now,
                max_iterations,
                status: status_now,
                reason: Some(eval.reason.clone()),
            })
            .await;

        if !status_now.is_active() {
            record_goal_outcome_to_episodic(&sess, &current_goal).await;
            let _ = tx
                .send(HarnessEvent::GoalDone {
                    status: status_now,
                    reason: Some(eval.reason.clone()),
                })
                .await;
            break 'goal_loop;
        }

        // `not_met` — inject a synthetic continuation user message +
        // open a new TurnMeta so the UI knows another autonomous turn
        // just started, then loop back for another round-loop pass.
        let synthetic = format!(
            "Goal not yet met. Keep working toward the standing goal:\n\n{condition}\n\n\
             Evaluator's note on iteration {iteration_now}/{max_iterations}: {reason}\n\n\
             Continue.",
            reason = eval.reason,
        );
        sess.history.lock().await.push(Message::user(synthetic));
        sess.turns.lock().await.push(TurnMeta {
            started_at: now_ms(),
            ended_at: None,
        });
        // fall through — 'goal_loop iterates and the next round starts
    }

    let _ = tx.send(HarnessEvent::Done).await;
}

/// A group of tool calls dispatched together. `parallel = true` means
/// every call in `calls` reported `Tool::parallel_safe(&call) == true`;
/// the loop hands the whole slice to `join_all`. Sequential batches
/// (either a single call or a run containing at least one non-safe
/// tool) get their calls awaited one at a time in `calls` order.
struct DispatchBatch {
    calls: Vec<ToolCall>,
    parallel: bool,
}

/// Split `pending_calls` into batches. Consecutive parallel-safe calls
/// collapse into one parallel batch; each non-safe call becomes its own
/// sequential batch. Unknown tools are treated as non-parallel so an
/// error-path lookup can never accidentally parallelize.
async fn plan_dispatch_batches(sess: &Session, calls: Vec<ToolCall>) -> Vec<DispatchBatch> {
    let mut batches: Vec<DispatchBatch> = Vec::new();
    let mut current: Vec<ToolCall> = Vec::new();

    for call in calls {
        let is_safe = match sess.registry.get(&call.function.name) {
            Some(tool) => tool.parallel_safe(&call),
            None => false,
        };
        if is_safe {
            current.push(call);
        } else {
            if !current.is_empty() {
                batches.push(DispatchBatch {
                    calls: std::mem::take(&mut current),
                    parallel: true,
                });
            }
            batches.push(DispatchBatch {
                calls: vec![call],
                parallel: false,
            });
        }
    }
    if !current.is_empty() {
        batches.push(DispatchBatch {
            calls: current,
            parallel: true,
        });
    }
    batches
}

/// Dispatch a single tool call end-to-end: policy check, optional
/// approval, `Tool::invoke`, history push, `ToolEnd` event. Returns
/// `true` when the tool completed without error — the caller uses that
/// to update the "any successful tool call" flag that gates the
/// post-round auto-extractor.
///
/// Safe to `join_all` a slice of these when every call in the slice is
/// `Tool::parallel_safe`: `history`, `policy`, and the mpsc `tx` are
/// all `Send + Sync` (Arc-behind-Mutex / cloneable), and the tool_ctx
/// is shared by design. Concurrent history pushes serialize on the
/// history mutex, which keeps the recorded transcript coherent.
async fn dispatch_call(sess: &Session, call: ToolCall, tx: &mpsc::Sender<HarnessEvent>) -> bool {
    let Some(tool) = sess.registry.get(&call.function.name) else {
        let msg = format!("no such tool: {}", call.function.name);
        warn!(tool = %call.function.name, "unknown tool call");
        let result = ToolResult::err(call.id.clone(), msg);
        sess.history.lock().await.push(Message::tool(
            result.call_id.clone(),
            truncate_for_history(&result.content),
        ));
        let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
        return false;
    };

    // Every path/target this call would touch. For single-target tools
    // this is a one-element vec matching the old `policy_target()`
    // return; for multi-target tools (apply_patch) it's every affected
    // source AND destination. Deny wins over Ask wins over Allow: the
    // first Deny fails the whole call; any Ask (with no Deny) opens
    // one approval modal for the batch.
    let targets = tool.policy_targets(&call);
    let action = tool.action();
    let mut has_ask = false;
    let mut deny_target: Option<String> = None;
    {
        let policy = sess.policy.lock().await;
        for t in &targets {
            let d = policy.evaluate(&PolicyRequest { action, target: t });
            match d {
                Decision::Deny => {
                    deny_target = Some(t.clone());
                    break;
                }
                Decision::Ask => has_ask = true,
                Decision::Allow => {}
            }
        }
    }

    let allowed = if deny_target.is_some() {
        false
    } else if has_ask {
        sess.approver.approve(&call, Decision::Ask).await
    } else {
        true
    };
    // For the denial message we prefer the specific target that failed;
    // callers can then see exactly which path violated policy.
    let target = deny_target
        .clone()
        .unwrap_or_else(|| targets.first().cloned().unwrap_or_default());

    let _ = tx.send(HarnessEvent::ToolStart(call.clone())).await;

    // Capture the diff preview *before* running the tool so the "before"
    // file state matches what the user saw live. `compute_preview` is a
    // no-op for tools without a natural preview (bash, read_file, etc.),
    // so the map only ever grows with edit/write entries. Persisted via
    // `checkpoint` and broadcast so both live auto-allow flows and
    // reloaded transcripts render the real diff instead of the arg-only
    // reconstruction fallback.
    if allowed {
        if let Some(preview) = compute_preview(&sess.tool_ctx.cwd, &call).await {
            sess.previews
                .lock()
                .await
                .insert(call.id.to_string(), preview.clone());
            let _ = tx
                .send(HarnessEvent::ToolPreview {
                    call_id: call.id.to_string(),
                    preview,
                })
                .await;
        }
    }

    let result = if !allowed {
        ToolResult::err(
            call.id.clone(),
            format!(
                "denied by policy: {} on `{}`",
                format_action(tool.action()),
                target
            ),
        )
    } else {
        // Snap a per-call clone of tool_ctx with the current turn's
        // cancellation token attached. Cheap (all fields are Arcs);
        // done per-call so a token cancelled after this dispatch
        // doesn't affect the next call's clone.
        let mut per_call_ctx = sess.tool_ctx.clone();
        per_call_ctx.cancel = sess.current_cancel.lock().await.clone();
        match tool.invoke(&call, &per_call_ctx).await {
            Ok(r) => r,
            Err(e) => {
                error!(tool = %call.function.name, %e, "tool invocation failed");
                ToolResult::err(call.id.clone(), e.to_string())
            }
        }
    };

    let ok = !result.is_error;
    {
        let mut history = sess.history.lock().await;
        history.push(Message::tool(
            result.call_id.clone(),
            truncate_for_history(&result.content),
        ));
        // Dedup: if this was a read / write / edit for a specific path,
        // collapse any older `read_file` result targeting the same path
        // to a short stub. Same-lock scope so the walk sees exactly the
        // history we just pushed into.
        if ok {
            if let Some(path) = crate::history::path_from_args(&call.function.arguments) {
                crate::history::dedup_reads_for_path(
                    &mut history,
                    &call.id,
                    &call.function.name,
                    &path,
                );
            }
        }
    }
    let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
    ok
}

/// Write a one-line summary of a just-terminated `/goal` into the
/// session's episodic store (if one is wired). Cross-session
/// carryover: a future session inspecting `.mira/episodic.jsonl`
/// picks up "we tried X, verdict was Y" and can reason about it.
///
/// Only runs on non-`Cleared` terminal states — a user-cancelled
/// goal isn't a learning signal, just a manual abort. Failures are
/// warned and swallowed; a broken episodic write should never kill
/// the harness loop.
async fn record_goal_outcome_to_episodic(sess: &Session, goal: &Goal) {
    let Some(episodic) = sess.tool_ctx.episodic.clone() else {
        return;
    };
    let Some(text) = goal_outcome_summary(goal) else {
        return;
    };
    let mut entry = EpisodicEntry::now(text, EpisodicSource::Auto);
    entry = entry.with_session_id(sess.id.to_string());
    if let Err(e) = episodic.append(entry).await {
        warn!(%e, "episodic write for goal outcome failed");
    }
}

/// Format a terminal-goal outcome as a one-liner for `.mira/episodic.jsonl`.
/// Returns `None` for goals that aren't in a learning-worthy terminal
/// state (Active mid-loop, or Cleared by the user).
fn goal_outcome_summary(goal: &Goal) -> Option<String> {
    let verdict = match goal.status {
        GoalStatus::Met => "met",
        GoalStatus::Impossible => "impossible",
        GoalStatus::NeedsUser => "needs_user",
        GoalStatus::Exhausted => "exhausted",
        GoalStatus::Cleared | GoalStatus::Active => return None,
    };
    let contract = one_line(&goal.condition, 200);
    let reason = goal
        .last_reason
        .as_deref()
        .map(|r| one_line(r, 240))
        .unwrap_or_else(|| "(no evaluator reason recorded)".to_owned());
    Some(format!(
        "Goal outcome: {verdict} after {n}/{cap} iteration{s}. \
         Contract: \"{contract}\". Reason: {reason}",
        n = goal.iterations,
        cap = goal.max_iterations,
        s = if goal.iterations == 1 { "" } else { "s" },
    ))
}

/// Collapse newlines and trim `s` to at most `max` chars — episodic
/// entries are one JSON line each, so multi-paragraph content should
/// flatten before writing.
fn one_line(s: &str, max: usize) -> String {
    let flat = s
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max).collect();
    out.push('…');
    out
}

/// Snapshot the session and save through the attached store, if any.
/// Failures are logged and swallowed — losing a checkpoint shouldn't kill
/// the running conversation.
async fn checkpoint(sess: &Session) {
    let Some(store) = &sess.store else { return };
    let record = SessionRecord {
        id: sess.id.clone(),
        cwd: sess.tool_ctx.cwd.clone(),
        cfg: sess.cfg.lock().await.clone(),
        messages: sess.history.lock().await.clone(),
        created_at: sess.created_at,
        updated_at: now_secs(),
        title: sess.title.lock().await.clone(),
        turns: sess.turns.lock().await.clone(),
        usage: *sess.usage.lock().await,
        parent_id: sess.parent_id.clone(),
        tasks: sess.tasks.snapshot_all().await,
        goal: sess.goal.lock().await.clone(),
        previews: sess.previews.lock().await.clone(),
    };
    if let Err(e) = store.save(&record).await {
        warn!(session = %sess.id, %e, "session checkpoint failed");
    }
}

/// Truncate a tool result to a size the model can safely re-ingest.
/// Keeps a head slice (the start of the output, usually most relevant)
/// AND a tail slice (last lines — where test summaries, error messages,
/// and exit banners tend to live). A marker between the two tells the
/// model how many bytes were elided.
fn truncate_for_history(content: &str) -> String {
    if content.len() <= TOOL_RESULT_HISTORY_CAP {
        return content.to_owned();
    }
    let tail_budget = ((TOOL_RESULT_HISTORY_CAP as f64) * TOOL_RESULT_TAIL_FRACTION) as usize;
    let head_budget = TOOL_RESULT_HISTORY_CAP.saturating_sub(tail_budget);

    let head_end = char_boundary_at_most(content, head_budget);
    let tail_start = char_boundary_at_least(content, content.len().saturating_sub(tail_budget));

    // If head + tail would overlap (short content that squeaked past
    // the length check due to fractional accounting), fall back to a
    // pure-head slice.
    if tail_start <= head_end {
        let head = &content[..head_end];
        let omitted = content.len() - head_end;
        return format!(
            "{head}\n\n… [{omitted} bytes truncated; ask again with a narrower query to see more]"
        );
    }

    let head = &content[..head_end];
    let tail = &content[tail_start..];
    let omitted = tail_start - head_end;
    format!(
        "{head}\n\n… [{omitted} bytes truncated in the middle; ask with a narrower query for more] …\n\n{tail}"
    )
}

/// Order-independent fingerprint of a set of tool calls. Two rounds
/// with the same `(tool_name, canonicalized_args)` multiset produce
/// the same u64 — used by the round loop's stuck-loop detector to
/// notice "same batch, again."
///
/// - Args are canonicalized (recursively sorted-key JSON) so
///   `{"path":"a","limit":10}` and `{"limit":10,"path":"a"}` collide.
/// - Calls are put in a `BTreeSet` before hashing so call ORDER
///   within a round doesn't matter.
/// - Bad JSON in a call's arguments hashes the raw string — worst
///   case we don't collide on trivially-reordered malformed args,
///   which is fine (the detector just doesn't fire on the first
///   dupe round).
fn fingerprint_calls(calls: &[ToolCall]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::collections::BTreeSet;
    use std::hash::{Hash, Hasher};

    let mut entries: BTreeSet<(String, String)> = BTreeSet::new();
    for c in calls {
        let canon = canonicalize_args(&c.function.arguments);
        entries.insert((c.function.name.clone(), canon));
    }
    let mut hasher = DefaultHasher::new();
    for (n, a) in &entries {
        n.hash(&mut hasher);
        a.hash(&mut hasher);
    }
    hasher.finish()
}

/// Serialize a JSON args blob with recursively sorted object keys.
/// Falls back to the raw string on parse failure — the detector can
/// still catch textually-identical duplicates.
fn canonicalize_args(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => {
            let mut out = String::new();
            write_canonical_json(&mut out, &v);
            out
        }
        Err(_) => raw.to_owned(),
    }
}

fn write_canonical_json(out: &mut String, v: &serde_json::Value) {
    match v {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            out.push_str(&serde_json::to_string(s).unwrap_or_default());
        }
        serde_json::Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical_json(out, item);
            }
            out.push(']');
        }
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical_json(out, &m[k.as_str()]);
            }
            out.push('}');
        }
    }
}

/// Largest char boundary `<= target`. Safe to slice `content[..idx]`.
fn char_boundary_at_most(content: &str, target: usize) -> usize {
    if target >= content.len() {
        return content.len();
    }
    let mut idx = target;
    while idx > 0 && !content.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Smallest char boundary `>= target`. Safe to slice `content[idx..]`.
fn char_boundary_at_least(content: &str, target: usize) -> usize {
    if target >= content.len() {
        return content.len();
    }
    let mut idx = target;
    while idx < content.len() && !content.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn format_action(a: mira_tools::Action) -> &'static str {
    match a {
        mira_tools::Action::Read => "Read",
        mira_tools::Action::Edit => "Edit",
        mira_tools::Action::Write => "Write",
        mira_tools::Action::Bash => "Bash",
        mira_tools::Action::Pure => "Pure",
    }
}

/// Compute this turn's new writes vs the pre-turn snapshot, pick an
/// appropriate check, and run it. Returns `Some((check, output))` only
/// on failure — success + "no writes" + "no matching check" all return
/// `None` so the caller emits `Done` unchanged.
async fn run_verify(
    sess: &Session,
    writes_at_turn_start: &std::collections::HashSet<std::path::PathBuf>,
    tx: &mpsc::Sender<HarnessEvent>,
) -> Option<(crate::verify::VerifyCheck, String)> {
    let guard = sess.tool_ctx.guard.as_ref()?;
    let now = guard.written_snapshot().await;
    let new_writes: Vec<std::path::PathBuf> =
        now.difference(writes_at_turn_start).cloned().collect();
    if new_writes.is_empty() {
        return None;
    }
    let check = crate::verify::detect(&sess.tool_ctx.cwd, &new_writes)?;
    let _ = tx
        .send(HarnessEvent::Warning(format!(
            "[verify] running `{}`…",
            check.name
        )))
        .await;
    let outcome = crate::verify::run(&sess.tool_ctx.sandbox, &check, &sess.tool_ctx.cwd).await;
    if outcome.ok {
        let _ = tx
            .send(HarnessEvent::Warning(format!(
                "[verify] `{}` passed",
                check.name
            )))
            .await;
        None
    } else {
        let _ = tx
            .send(HarnessEvent::Warning(format!(
                "[verify] `{}` failed — asking model to fix",
                check.name
            )))
            .await;
        Some((check, outcome.output))
    }
}

/// If auto-extract is enabled and the gate passed (`any_tool_calls`), spawn
/// a background task that mines this round for durable facts and appends
/// them to the episodic store. Detached-fire-and-forget: the caller does
/// NOT await this. The task holds a clone of `tx` so it can emit a
/// `MemoryLearned` frame if the receiver is still around; the info log is
/// always emitted so CLI users see the outcome even after the stream is
/// closed.
async fn maybe_spawn_extractor(
    sess: &Session,
    cfg: &SessionConfig,
    turn_start_idx: usize,
    any_tool_calls: bool,
    tx: mpsc::Sender<HarnessEvent>,
) {
    if !any_tool_calls {
        return;
    }
    let Some(auto) = sess.auto_extract.as_ref() else {
        return;
    };
    if !auto.enabled {
        return;
    }
    let Some(episodic) = sess.tool_ctx.episodic.clone() else {
        return;
    };

    // Copy just the round's messages so we don't hang onto the session
    // history mutex or drag the whole transcript into the spawned task.
    let round: Vec<Message> = {
        let hist = sess.history.lock().await;
        if turn_start_idx >= hist.len() {
            return;
        }
        hist[turn_start_idx..].to_vec()
    };
    let round_content = format_round_for_extraction(&round);
    if round_content.trim().is_empty() {
        return;
    }

    let provider = sess.provider.clone();
    let model = auto.model.clone().unwrap_or_else(|| cfg.model.clone());
    let session_id = sess.id.clone();

    tokio::spawn(async move {
        let result = tokio::time::timeout(
            EXTRACTION_TIMEOUT,
            run_extraction(provider, model, round_content, episodic, session_id),
        )
        .await;
        match result {
            Ok(Ok(count)) if count > 0 => {
                info!(count, "auto-extract: remembered facts");
                let _ = tx.send(HarnessEvent::MemoryLearned { count }).await;
            }
            Ok(Ok(_)) => {
                // Extractor ran but nothing worth remembering — stay quiet.
            }
            Ok(Err(e)) => warn!(error = %e, "auto-extract: failed"),
            Err(_) => warn!(
                timeout_s = EXTRACTION_TIMEOUT.as_secs(),
                "auto-extract: timed out"
            ),
        }
    });
}

/// Extraction call → parse → dedup → append. Returns the number of
/// entries actually written (survivors of the dedup filter).
async fn run_extraction(
    provider: Arc<dyn ChatProvider>,
    model: String,
    round_content: String,
    episodic: Arc<dyn EpisodicStore>,
    session_id: SessionId,
) -> Result<usize, String> {
    let candidates = extract_facts(provider, model, round_content)
        .await
        .map_err(|e| e.to_string())?;
    if candidates.is_empty() {
        return Ok(0);
    }
    let existing = episodic
        .recent(DEDUP_LOOKBACK)
        .await
        .map_err(|e| e.to_string())?;
    let mut appended = 0usize;
    for text in candidates {
        if is_duplicate(&text, &existing) {
            continue;
        }
        let entry =
            EpisodicEntry::now(text, EpisodicSource::Auto).with_session_id(session_id.to_string());
        if let Err(e) = episodic.append(entry).await {
            warn!(error = %e, "auto-extract: append failed");
            continue;
        }
        appended += 1;
    }
    Ok(appended)
}

/// One provider call with the extraction prompt. Returns a list of
/// candidate bullets (deduped later). Uses the streaming API for
/// consistency with the rest of the codebase but the response is small
/// and we just accumulate it.
async fn extract_facts(
    provider: Arc<dyn ChatProvider>,
    model: String,
    round_content: String,
) -> Result<Vec<String>, mira_ai::ProviderError> {
    let system = EXTRACTION_SYSTEM.to_string();
    let user = format!("Round content:\n\n{round_content}\n\nFacts:");
    let req = ChatRequest {
        model,
        messages: vec![Message::system(system), Message::user(user)],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(EXTRACTION_OUTPUT_TOKENS),
        reasoning_effort: None,
        response_format: None,
    };
    let mut stream = provider.stream(req).await?;
    let mut text = String::new();
    while let Some(evt) = stream.next().await {
        match evt {
            Ok(ChatEvent::TextDelta(t)) => text.push_str(&t),
            Ok(ChatEvent::Done(_)) => break,
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(parse_extraction_bullets(&text))
}

/// System prompt for the extractor. Kept intentionally tight — the model
/// only needs to know the *criteria*, not why they exist.
const EXTRACTION_SYSTEM: &str = "\
You extract durable, cross-session facts from a single conversation turn between a user \
and an AI coding agent. Output ONE fact per line, each starting with '- '. \
Include a fact ONLY if a future session with no memory of this turn would benefit from \
knowing it while working on the same project.\n\
\n\
Good: repo conventions ('this repo uses pnpm not npm'), decisions with rationale \
('we chose migration B because of concurrent writes'), gotchas ('tests need \
SKIP_LINT=1 on macOS'), user preferences ('user prefers terse commit messages').\n\
Bad: transient state (current file, current TODO), things the code itself already \
documents, opinions about the assistant's own performance, one-off task details.\n\
\n\
If nothing is worth remembering, output the single line: NONE\n\
Output ONLY the bullets or 'NONE'. No preamble, no explanation, no headers.";

/// Turn the raw extractor response into a list of candidate facts.
/// Handles `-` / `*` prefixes and drops empties and `NONE` sentinels.
fn parse_extraction_bullets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("NONE") {
            continue;
        }
        let stripped = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .unwrap_or(t)
            .trim();
        if stripped.is_empty() {
            continue;
        }
        out.push(stripped.to_string());
    }
    out
}

/// Render a round's messages as plain text for the extractor prompt.
/// Only user + assistant text; tool results are skipped (they're usually
/// large and noise for extraction — the assistant's followup captures
/// what mattered). Bounded by `EXTRACTION_INPUT_MAX` bytes.
fn format_round_for_extraction(msgs: &[Message]) -> String {
    let mut out = String::new();
    for m in msgs {
        let content = match m.content.as_deref() {
            Some(c) if !c.trim().is_empty() => c,
            _ => continue,
        };
        let label = match m.role {
            Role::User => "[user]",
            Role::Assistant => "[assistant]",
            _ => continue,
        };
        out.push_str(&format!("{label}\n{content}\n\n"));
    }
    if out.len() > EXTRACTION_INPUT_MAX {
        let mut cut = EXTRACTION_INPUT_MAX;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n\n[transcript truncated for extractor]\n");
    }
    out.trim().to_string()
}

/// Case- and whitespace-insensitive duplicate check.
///
/// A candidate is a duplicate if its normalized text either contains, or
/// is contained by, any of the recent entries. Catches the "same fact
/// worded slightly longer" case in both directions — cheaper than
/// embeddings and good enough at v1 scale.
fn is_duplicate(candidate: &str, existing: &[EpisodicEntry]) -> bool {
    let cand = normalize_for_dedup(candidate);
    if cand.is_empty() {
        return true;
    }
    for e in existing {
        let e_norm = normalize_for_dedup(&e.text);
        if e_norm.is_empty() {
            continue;
        }
        if e_norm.contains(&cand) || cand.contains(&e_norm) {
            return true;
        }
    }
    false
}

fn normalize_for_dedup(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the message list for one provider round.
///
/// Starts from the persisted history (system prefix + conversation) and,
/// when a memory snapshot is attached, splices a fresh memory block in as
/// a *second* system message right after the prefix. That second message
/// intentionally is never persisted — resumed sessions render memory from
/// disk on their next round, so a mid-session edit is always live.
///
/// The prefix stays as message[0] so the provider's prompt cache still
/// hits — see `mira_ai::openai::WireMessage::from_message`, which marks
/// only the first system message with `cache_control: ephemeral`.
async fn build_request_messages(sess: &Session) -> Vec<Message> {
    let mut msgs = sess.history.lock().await.clone();
    let Some(snap) = sess.memory_snapshot.as_ref() else {
        return msgs;
    };
    // Retrieval: build a query from the recent conversation so scored
    // selection can weight relevant entries above stale ones. When
    // retrieval is off, pass `None` and the snapshot falls back to the
    // legacy dump-everything shape.
    let query = if sess.memory_retrieval.enabled {
        Some(build_memory_query(
            &msgs,
            sess.memory_retrieval.token_budget,
        ))
    } else {
        None
    };
    let Some(block) = snap.render(query.as_ref()).await else {
        return msgs;
    };
    // Find the first system message and insert the memory block right
    // after it. If there is no system message (shouldn't happen in
    // practice — `Session::new` always seeds one — but the code is
    // defensive) fall back to prepending.
    let insert_at = msgs
        .iter()
        .position(|m| matches!(m.role, Role::System))
        .map(|i| i + 1)
        .unwrap_or(0);
    msgs.insert(insert_at, Message::system(block));
    msgs
}

/// Build a retrieval query from the tail of the conversation. Weighted
/// toward the most-recent user message (that's what the model is about
/// to act on) plus a small slice of the preceding assistant/tool turns
/// for topical context. Deliberately cheap — no tokenisation here; the
/// scorer does that itself.
///
/// Cap on total query length keeps IDF calculation snappy even when a
/// tool result was gigantic in the last round.
fn build_memory_query(msgs: &[Message], token_budget: usize) -> MemoryQuery {
    const QUERY_CHAR_CAP: usize = 4000;
    const QUERY_TAIL_MESSAGES: usize = 6;

    let mut pieces: Vec<String> = Vec::new();
    let mut used = 0usize;
    // Walk from the newest message backward. The last user message is
    // the primary signal; earlier context supports it. Skip system
    // messages entirely (that's where memory itself lives — self-
    // referential scoring is not useful).
    for m in msgs.iter().rev().take(QUERY_TAIL_MESSAGES * 2) {
        if matches!(m.role, Role::System) {
            continue;
        }
        let Some(body) = m.content.as_deref() else {
            continue;
        };
        let trimmed = body.trim();
        if trimmed.is_empty() {
            continue;
        }
        let take = trimmed.len().min(QUERY_CHAR_CAP.saturating_sub(used));
        if take == 0 {
            break;
        }
        pieces.push(trimmed[..take].to_string());
        used += take;
        if pieces.len() >= QUERY_TAIL_MESSAGES || used >= QUERY_CHAR_CAP {
            break;
        }
    }
    // Reverse so oldest-first reads naturally.
    pieces.reverse();
    MemoryQuery {
        context: pieces.join("\n"),
        token_budget: Some(token_budget),
        now_secs: now_secs_wall(),
    }
}

fn now_secs_wall() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Map a policy [`Mode`] to the [`SandboxProfile`] the persistent shell
/// should spawn under. The mapping is deliberately conservative:
///
///   - `plan` and `manual` → `Restricted` (deny writes and network) —
///     these modes exist for read-only exploration and per-call user
///     approval respectively; a bash command that slipped past the
///     approver still can't damage the filesystem or exfiltrate data.
///   - `auto` and `edit` → `Workspace` — the working posture. Writes
///     under cwd + `~/.mira`, network open for `curl` / `git fetch` /
///     package managers.
///   - `yolo` → `Unrestricted` — no seatbelt. Only what the user
///     explicitly asked for.
pub fn profile_for_mode(mode: Mode) -> SandboxProfile {
    match mode {
        Mode::Plan | Mode::Manual => SandboxProfile::Restricted,
        Mode::Auto | Mode::Edit => SandboxProfile::Workspace,
        Mode::Yolo => SandboxProfile::Unrestricted,
    }
}

/// Walk history and synthesize a `Message::tool` error reply for every
/// tool_call on the most recent assistant message that lacks a matching
/// `tool_call_id` response. Called at the top of `send` so an
/// interrupted turn (abort between `history.push(assistant_msg)` and
/// the `Message::tool(...)` push in `dispatch_call`) can't leave a
/// broken transcript that the provider rejects on the next request.
///
/// Returns the number of orphaned calls that were filled in. Zero means
/// history was already coherent — the common case.
fn repair_dangling_tool_calls(history: &mut Vec<Message>) -> usize {
    let assistant_idx = match history
        .iter()
        .rposition(|m| m.role == Role::Assistant && !m.tool_calls.is_empty())
    {
        Some(idx) => idx,
        None => return 0,
    };

    let expected_ids: Vec<String> = history[assistant_idx]
        .tool_calls
        .iter()
        .map(|c| c.id.to_string())
        .collect();

    // Any messages already sitting after this assistant turn that answer
    // one of its calls. Provider order isn't guaranteed strict, so we
    // check by id rather than positional pairing.
    let answered: std::collections::HashSet<String> = history[assistant_idx + 1..]
        .iter()
        .filter_map(|m| m.tool_call_id.as_ref().map(|id| id.to_string()))
        .collect();

    let missing: Vec<String> = expected_ids
        .into_iter()
        .filter(|id| !answered.contains(id))
        .collect();

    if missing.is_empty() {
        return 0;
    }

    let n = missing.len();
    for call_id in missing {
        // The exact wording is stable copy — the model reads this back
        // as the tool's own output. Kept short so it doesn't bloat the
        // context; kept explicit so the model can see WHY the call
        // returned nothing and choose whether to retry.
        history.push(Message::tool(
            call_id.into(),
            "(previous turn was interrupted — this tool call did not complete)".to_owned(),
        ));
    }
    n
}

#[cfg(test)]
mod history_repair_tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{ToolCall, ToolCallId};

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from(id),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.to_owned(),
                arguments: "{}".to_owned(),
            },
        }
    }

    #[test]
    fn coherent_history_is_untouched() {
        let mut hist = vec![
            Message::user("hi"),
            {
                let mut m = Message::assistant("");
                m.tool_calls = vec![call("c1", "read_file")];
                m
            },
            Message::tool(ToolCallId::from("c1"), "ok"),
        ];
        assert_eq!(repair_dangling_tool_calls(&mut hist), 0);
        assert_eq!(hist.len(), 3);
    }

    #[test]
    fn dangling_call_gets_synthetic_reply() {
        let mut hist = vec![
            Message::user("hi"),
            {
                let mut m = Message::assistant("");
                m.tool_calls = vec![call("c1", "read_file"), call("c2", "bash")];
                m
            },
            // Only c1 was answered before interrupt.
            Message::tool(ToolCallId::from("c1"), "ok"),
        ];
        assert_eq!(repair_dangling_tool_calls(&mut hist), 1);
        // c2 now has a placeholder result appended.
        let last = hist.last().unwrap();
        assert_eq!(last.role, Role::Tool);
        assert_eq!(
            last.tool_call_id.as_ref().map(|id| id.to_string()),
            Some("c2".to_owned())
        );
        assert!(last
            .content
            .as_deref()
            .unwrap_or("")
            .contains("interrupted"));
    }

    #[test]
    fn history_with_no_assistant_calls_is_noop() {
        let mut hist = vec![Message::user("hi"), Message::assistant("hey")];
        assert_eq!(repair_dangling_tool_calls(&mut hist), 0);
    }
}

#[cfg(test)]
mod goal_outcome_tests {
    use super::*;

    fn goal_with(status: GoalStatus, iterations: usize, reason: Option<&str>) -> Goal {
        let mut g = Goal::new("Make the tests pass");
        g.status = status;
        g.iterations = iterations;
        g.last_reason = reason.map(String::from);
        g
    }

    #[test]
    fn active_goal_yields_no_summary() {
        assert!(goal_outcome_summary(&goal_with(GoalStatus::Active, 0, None)).is_none());
    }

    #[test]
    fn cleared_goal_yields_no_summary() {
        // A user-cancelled goal isn't a learning signal.
        assert!(
            goal_outcome_summary(&goal_with(GoalStatus::Cleared, 5, Some("user cleared")))
                .is_none()
        );
    }

    #[test]
    fn met_goal_records_verdict_and_reason() {
        let g = goal_with(
            GoalStatus::Met,
            3,
            Some("All 42 tests pass; grep found no v1 sites"),
        );
        let s = goal_outcome_summary(&g).unwrap();
        assert!(s.starts_with("Goal outcome: met after 3/20 iterations."));
        assert!(s.contains("Make the tests pass"));
        assert!(s.contains("All 42 tests pass"));
    }

    #[test]
    fn exhausted_goal_records_iteration_cap() {
        let g = goal_with(GoalStatus::Exhausted, 20, Some("hit iteration cap of 20"));
        let s = goal_outcome_summary(&g).unwrap();
        assert!(s.contains("exhausted"));
        assert!(s.contains("20/20"));
    }

    #[test]
    fn needs_user_goal_recorded() {
        let g = goal_with(GoalStatus::NeedsUser, 4, Some("missing GITHUB_TOKEN"));
        let s = goal_outcome_summary(&g).unwrap();
        assert!(s.contains("needs_user"));
        assert!(s.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn missing_reason_gets_placeholder() {
        let g = goal_with(GoalStatus::Impossible, 2, None);
        let s = goal_outcome_summary(&g).unwrap();
        assert!(s.contains("impossible"));
        assert!(s.contains("no evaluator reason recorded"));
    }

    #[test]
    fn one_iteration_uses_singular() {
        let g = goal_with(GoalStatus::Met, 1, Some("done"));
        let s = goal_outcome_summary(&g).unwrap();
        assert!(s.contains("1/20 iteration."), "singular form: {s}");
    }

    #[test]
    fn one_line_flattens_newlines_and_truncates() {
        let long = format!("line one\nline two\r\nline three {}", "x".repeat(500));
        let out = one_line(&long, 100);
        assert!(!out.contains('\n'));
        assert!(!out.contains('\r'));
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 101, "len {}", out.chars().count());
    }

    #[test]
    fn one_line_short_input_untouched_except_flattening() {
        let s = one_line("a\nb  c", 50);
        assert_eq!(s, "a b c");
    }
}

#[cfg(test)]
mod stuck_loop_tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::ToolCallId;

    fn call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from(id),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.to_owned(),
                arguments: args.to_owned(),
            },
        }
    }

    #[test]
    fn identical_rounds_produce_matching_fingerprints() {
        let a = vec![call("1", "grep", r#"{"pattern":"foo","path":"src"}"#)];
        let b = vec![call("2", "grep", r#"{"pattern":"foo","path":"src"}"#)];
        assert_eq!(fingerprint_calls(&a), fingerprint_calls(&b));
    }

    #[test]
    fn reordered_args_hash_the_same() {
        // The classic "did the model swap key order" test.
        let a = vec![call("1", "read_file", r#"{"path":"a.rs","limit":10}"#)];
        let b = vec![call("2", "read_file", r#"{"limit":10,"path":"a.rs"}"#)];
        assert_eq!(fingerprint_calls(&a), fingerprint_calls(&b));
    }

    #[test]
    fn reordered_calls_within_a_round_hash_the_same() {
        // The stuck-loop condition is a *set* match — the model
        // permuting its tool_calls list shouldn't defeat the detector.
        let ab = vec![
            call("1", "grep", r#"{"pattern":"x"}"#),
            call("2", "read_file", r#"{"path":"a"}"#),
        ];
        let ba = vec![
            call("3", "read_file", r#"{"path":"a"}"#),
            call("4", "grep", r#"{"pattern":"x"}"#),
        ];
        assert_eq!(fingerprint_calls(&ab), fingerprint_calls(&ba));
    }

    #[test]
    fn different_args_diverge() {
        let a = vec![call("1", "grep", r#"{"pattern":"foo"}"#)];
        let b = vec![call("2", "grep", r#"{"pattern":"bar"}"#)];
        assert_ne!(fingerprint_calls(&a), fingerprint_calls(&b));
    }

    #[test]
    fn different_tool_names_diverge() {
        let a = vec![call("1", "grep", r#"{"pattern":"foo"}"#)];
        let b = vec![call("2", "rg", r#"{"pattern":"foo"}"#)];
        assert_ne!(fingerprint_calls(&a), fingerprint_calls(&b));
    }

    #[test]
    fn malformed_args_still_hash_stably() {
        // Not valid JSON — falls back to raw-string hash, which is
        // stable and still catches textually-identical dupes.
        let a = vec![call("1", "weird", "not-json{{")];
        let b = vec![call("2", "weird", "not-json{{")];
        assert_eq!(fingerprint_calls(&a), fingerprint_calls(&b));
    }

    #[test]
    fn nested_object_key_order_is_canonicalized() {
        let a = vec![call("1", "cfg", r#"{"opts":{"b":1,"a":2},"flag":true}"#)];
        let b = vec![call("2", "cfg", r#"{"flag":true,"opts":{"a":2,"b":1}}"#)];
        assert_eq!(fingerprint_calls(&a), fingerprint_calls(&b));
    }
}

#[cfg(test)]
mod truncate_tests {
    use super::*;

    #[test]
    fn short_content_passes_through() {
        let s = "short output";
        assert_eq!(truncate_for_history(s), s);
    }

    #[test]
    fn long_content_keeps_head_and_tail() {
        // A long payload with a distinctive head and a distinctive tail
        // (e.g. a test-summary line at the end of a bash log).
        let head_marker = "==== HEAD MARKER ====";
        let tail_marker = "==== TAIL MARKER: 42 passed, 0 failed ====";
        let filler = "x".repeat(TOOL_RESULT_HISTORY_CAP * 3);
        let payload = format!("{head_marker}\n{filler}\n{tail_marker}");
        let out = truncate_for_history(&payload);

        assert!(
            out.contains(head_marker),
            "head should survive: {}",
            &out[..out.len().min(200)]
        );
        assert!(
            out.contains(tail_marker),
            "tail should survive so test summaries reach the model"
        );
        assert!(
            out.contains("truncated"),
            "should include an elision marker"
        );
        assert!(
            out.len() <= TOOL_RESULT_HISTORY_CAP + 200,
            "truncated output should stay near the cap (got {} bytes)",
            out.len()
        );
    }

    #[test]
    fn multibyte_content_stays_on_char_boundaries() {
        // Emoji + CJK to ensure we never slice mid-UTF-8.
        let head = "🚀 launching ";
        let tail = " 完了しました 🎉";
        let payload = format!("{head}{}{tail}", "à".repeat(TOOL_RESULT_HISTORY_CAP));
        let out = truncate_for_history(&payload);
        // The mere fact that this returns (no panic) proves boundary
        // safety; the assertion just confirms both ends survive.
        assert!(out.starts_with("🚀 launching"), "head start: {out}");
        assert!(out.ends_with("🎉"), "tail end: {out}");
    }
}
