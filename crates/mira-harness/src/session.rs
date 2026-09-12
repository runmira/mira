use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::{stream::BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ResponseFormat};
use mira_core::{Message, Role, SessionId, ToolCall, ToolResult};
use mira_memory::{
    EpisodicEntry, EpisodicSource, EpisodicStore, MemoryQuery, MemorySnapshot,
    DEFAULT_TOKEN_BUDGET,
};
use mira_policy::{Decision, Policy, Request as PolicyRequest};
use mira_sandbox::PersistentShell;
use mira_tools::context::{ChildCancel, ChildTracker};
use mira_tools::{FileGuard, Registry, TaskStore, ToolContext};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::AbortHandle;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

/// How much of a tool result actually goes back into the model's context.
/// A single `ls -R` on a real repo can be 20k+ tokens which blows past
/// every provider's per-request cap — cap in the harness so the frontend
/// still sees the full result but the model only sees a preview.
const TOOL_RESULT_HISTORY_CAP: usize = 4000;

use crate::approver::Approver;
use crate::event::HarnessEvent;
use crate::goal::{self, Goal, GoalStatus, GoalVerdict};
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
}

/// Ceiling on tool-call rounds within a single user turn. Guards against
/// runaway loops (a model calling tools forever) without cutting real
/// refactors short. Raise via `max_rounds` in `mira.yaml` when 60 isn't
/// enough — long feature builds legitimately exceed it.
fn default_max_rounds() -> usize {
    60
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
        let cwd_for_shell = tool_ctx.cwd.clone();
        tool_ctx = tool_ctx.with_shell(Arc::new(Mutex::new(PersistentShell::new(
            cwd_for_shell,
            true,
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
        let cwd_for_shell = tool_ctx.cwd.clone();
        tool_ctx = tool_ctx.with_shell(Arc::new(Mutex::new(PersistentShell::new(
            cwd_for_shell,
            true,
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

    /// Run one user turn to completion.
    ///
    /// The returned stream ends with [`HarnessEvent::Done`]. Callers may
    /// drop it early to cancel — the task keeps mutating history until it
    /// hits a checkpoint, then exits when the send channel closes.
    pub async fn send(&self, user_input: impl Into<String>) -> BoxStream<'static, HarnessEvent> {
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

async fn run_loop(sess: Session, cfg: SessionConfig, tx: mpsc::Sender<HarnessEvent>) {
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
        }
        let mut round_outcome = RoundOutcome::MaxRounds;

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
            match crate::history::maybe_compact(
                &mut history,
                sess.provider.as_ref(),
                &cfg.model,
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

        while let Some(evt) = stream.next().await {
            match evt {
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
                    let _ = tx
                        .send(HarnessEvent::Warning(format!("stream error: {e}")))
                        .await;
                    break;
                }
            }
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
            let _ = tx
                .send(HarnessEvent::GoalDone {
                    status: current_goal.status,
                    reason: current_goal.last_reason.clone(),
                })
                .await;
            break 'goal_loop;
        }

        // Evaluate. Failures are downgraded to `not_met` with a warning
        // so a transient provider blip doesn't kill an in-progress
        // autonomous run — the next iteration gets another shot.
        let eval_model = current_goal
            .evaluator_model
            .clone()
            .unwrap_or_else(|| cfg.model.clone());
        let condition = current_goal.condition.clone();
        let transcript = sess.history.lock().await.clone();
        let eval = match goal::evaluate(
            sess.provider.as_ref(),
            &eval_model,
            &condition,
            &transcript,
        )
        .await
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
async fn dispatch_call(
    sess: &Session,
    call: ToolCall,
    tx: &mpsc::Sender<HarnessEvent>,
) -> bool {
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

    let target = tool.policy_target(&call);
    let decision = sess.policy.lock().await.evaluate(&PolicyRequest {
        action: tool.action(),
        target: &target,
    });

    let allowed = match decision {
        Decision::Allow => true,
        Decision::Deny => false,
        Decision::Ask => sess.approver.approve(&call, decision).await,
    };

    let _ = tx.send(HarnessEvent::ToolStart(call.clone())).await;

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
        match tool.invoke(&call, &sess.tool_ctx).await {
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
    };
    if let Err(e) = store.save(&record).await {
        warn!(session = %sess.id, %e, "session checkpoint failed");
    }
}

/// Truncate a tool result to a size the model can safely re-ingest. We keep
/// the head (usually the most relevant part — first lines of a diff, the
/// start of a file listing) and add a marker line telling the model how
/// much was elided.
fn truncate_for_history(content: &str) -> String {
    if content.len() <= TOOL_RESULT_HISTORY_CAP {
        return content.to_owned();
    }
    let cut = content
        .char_indices()
        .take_while(|(i, _)| *i < TOOL_RESULT_HISTORY_CAP)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0);
    let head = &content[..cut];
    let omitted = content.len() - cut;
    format!("{head}\n\n… [{omitted} bytes truncated; ask again with a narrower query to see more]")
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
    let new_writes: Vec<std::path::PathBuf> = now
        .difference(writes_at_turn_start)
        .cloned()
        .collect();
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
            Err(_) => warn!(timeout_s = EXTRACTION_TIMEOUT.as_secs(), "auto-extract: timed out"),
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
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
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
        Some(build_memory_query(&msgs, sess.memory_retrieval.token_budget))
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
