//! WebSocket wire protocol.
//!
//! Every frame is a single JSON object with a `"type"` discriminator. Server
//! frames carry harness events + approval requests; client frames carry user
//! input, approvals, and control (mode/model swap, interrupt, clear).

use mira_ai::TokenUsage;
use mira_core::{Message, ToolCall, ToolResult};
use mira_harness::{Goal, GoalStatus, HarnessEvent, TurnMeta, UsageTotals};
use mira_policy::Mode;
use mira_review::{Finding, Progress as ReviewProgress};
use mira_tools::DiffPreview;
use serde::{Deserialize, Serialize};

use crate::interactive::{AskUserProposal, PlanProposal, PromptResponse};
use crate::slot::BackgroundMode;

/// Scope on an `Approve` reply — how long the user's decision applies
/// for. `Once` (the default and legacy shape) affects only the current
/// call. `Session` promotes the target to `Allow` on the session's
/// in-memory policy so identical follow-up calls skip the modal.
/// `Always` also appends the rule to `~/.mira/mira.yaml` so it persists
/// across sessions.
///
/// Only meaningful when the reply's `allow` is `true` — a scoped deny
/// isn't a concept the current UI exposes.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    #[default]
    Once,
    Session,
    Always,
}

/// Client → server.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Start a new user turn.
    Send { text: String },
    /// Answer a pending approval prompt. `scope` says whether the
    /// decision only covers this specific call, or should also add a
    /// rule to the session policy (and optionally persist it) so
    /// identical future calls skip the modal.
    Approve {
        call_id: String,
        allow: bool,
        #[serde(default)]
        scope: ApprovalScope,
    },
    /// Answer an interactive tool prompt (plan review, question, etc.).
    /// `prompt_id` matches the `prompt_id` on the server frame that opened
    /// the modal.
    PromptResponse {
        prompt_id: String,
        #[serde(flatten)]
        response: PromptResponse,
    },
    /// Hot-swap the model for the next turn.
    SetModel { model: String },
    /// Change the permission mode.
    SetMode { mode: Mode },
    /// Set reasoning effort for the current session. `None` (or the string
    /// `"off"`) clears the field entirely so non-reasoning models aren't
    /// hit with an unexpected parameter.
    SetEffort {
        #[serde(default)]
        effort: Option<String>,
    },
    /// Best-effort cancel the current turn.
    Interrupt,
    /// Set (or replace) the session's standing `/goal`. `max_iterations`
    /// is optional — the server uses [`mira_harness::DEFAULT_MAX_ITERATIONS`]
    /// when absent so the client doesn't have to hardcode the cap. When
    /// `evaluator_model` is `None` the harness reuses the session's
    /// active model (usually you want a cheaper tier here).
    SetGoal {
        condition: String,
        #[serde(default)]
        max_iterations: Option<usize>,
        #[serde(default)]
        evaluator_model: Option<String>,
        /// Optional external verifier — a shell command whose exit
        /// code (and optional stdout regex) gates the "Met" verdict.
        /// See [`mira_harness::VerifyCommand`] for the contract.
        #[serde(default)]
        verify: Option<mira_harness::VerifyCommand>,
        /// Optional hard token budget (input + output, summed across
        /// all rounds). Breach → `Exhausted`.
        #[serde(default)]
        budget_tokens: Option<u64>,
        /// Optional hard USD budget. Requires the active model to be
        /// in `mira-ai`'s pricing table; unpriced models skip the USD
        /// check silently.
        #[serde(default)]
        budget_usd: Option<f64>,
    },
    /// Drop the session's standing goal. Idempotent — clearing a
    /// session without a goal is a no-op.
    ClearGoal,
    /// Ask the server to re-emit its current state (used on reconnect).
    Sync,
    /// Watch a specific session on this WS connection. Multi-session
    /// clients switch between sessions by sending Attach rather than
    /// hitting `POST /api/sessions/:id/load` — attaching doesn't kill
    /// any in-flight turn on the previous session, it just re-points
    /// this socket's event forwarder.
    ///
    /// The server responds by:
    /// 1. Decrementing the previously-attached slot's `attached` count.
    /// 2. Swapping the forwarder's subscription to the target slot's
    ///    events channel.
    /// 3. Incrementing the new slot's `attached` count.
    /// 4. Emitting a fresh `Ready` frame for the new session.
    /// 5. Updating the server's `active` pointer so subsequent
    ///    HTTP calls target this session.
    Attach { session_id: String },
    /// Detach from any session on this WS connection — used when a client
    /// wants to explicitly stop watching without closing the socket. Rare
    /// on the current UI (tab close does the same job); provided for
    /// completeness so a future "let this session finish in the background"
    /// affordance has a clean signal to send.
    Detach,
    /// Update the *attached* session's background mode. Applies to how
    /// the approver answers `Ask` decisions when no client is attached.
    SetBackgroundMode { mode: BackgroundMode },
}

/// Server → client.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Emitted once when the socket opens, with the session's current state.
    Ready {
        session_id: String,
        model: String,
        mode: Mode,
        cwd: String,
        history: Vec<Message>,
        /// Per-turn timing, aligned with user messages in `history`. Empty
        /// for legacy sessions written before turn tracking.
        #[serde(default)]
        turns: Vec<TurnMeta>,
        /// Aggregate token usage carried over from prior turns on this
        /// session. Zeroed for legacy sessions or providers that don't
        /// report usage.
        #[serde(default, skip_serializing_if = "UsageTotals::is_zero")]
        usage: UsageTotals,
        /// Session-scoped task list. Empty for sessions where the
        /// model hasn't called `task_create`. Sent so the UI can
        /// rehydrate the task panel on reload without asking the
        /// harness to replay tool history.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tasks: Vec<mira_tools::TaskItem>,
        /// Standing `/goal` — `None` when the user hasn't set one on
        /// this session (or cleared it). Terminal statuses are also
        /// sent so the UI can render a "last goal" chip until cleared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<Goal>,
        /// Persisted diff previews for edit/write tool calls, keyed by
        /// tool call id. The frontend attaches them to the matching
        /// tool entry during `historyToEntries` so a reloaded transcript
        /// renders the real diff instead of falling back to an
        /// arg-only reconstruction. Empty for legacy sessions written
        /// before this landed.
        #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
        previews: std::collections::HashMap<String, DiffPreview>,
    },
    /// Fragment of assistant text.
    Token { text: String },
    /// Tool call dispatched (already policy-approved).
    ToolStart { call: ToolCall },
    /// Tool call finished with a result.
    ToolEnd { result: ToolResult },
    /// Model turn complete — may be followed by another turn if tools ran.
    TurnComplete,
    /// User turn fully complete — waiting for next user input.
    Done,
    /// Approval needed for a tool call the policy flagged `Ask`. Client should
    /// answer with `ClientMsg::Approve { call_id, allow }`.
    ApprovalRequest {
        call: ToolCall,
        /// Structured diff for edit_file / write_file. `None` for tools that
        /// don't have a natural preview (e.g. `bash`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<DiffPreview>,
    },
    /// Non-fatal warning surfaced to the UI.
    Warning { text: String },
    /// One line of live stdout+stderr from a still-running tool call.
    /// The renderer routes lines by `call_id` under the matching
    /// pending tool card so the user sees progress before the final
    /// `ToolEnd` frame lands.
    ToolProgress { call_id: String, line: String },
    /// Diff preview computed for an `edit_file` / `write_file` call.
    /// Fires whether or not the call was approval-gated so live UIs
    /// always have the real diff, not just the reconstruction. Also
    /// persisted; see `Ready.previews` for the reload path.
    ToolPreview {
        call_id: String,
        preview: DiffPreview,
    },
    /// The skill registry was reloaded (a file change was detected in
    /// one of the four skill directories, or a manual reload was
    /// triggered). The frontend refetches `/api/skills` when it sees
    /// this so the composer palette and Settings panel pick up new /
    /// edited skills automatically.
    SkillsReloaded,
    /// Model changed (echoes SetModel).
    ModelChanged { model: String },
    /// Mode changed (echoes SetMode).
    ModeChanged { mode: Mode },
    /// A protocol-level error (bad input, unknown call_id, etc.).
    Error { text: String },

    /// A `mira review` run started. Every subsequent `ReviewProgress` / `ReviewResult`
    /// / `ReviewError` frame with this `run_id` belongs to it — the frontend
    /// filters on it so overlapping runs don't scramble each other's UI.
    ReviewStarted { run_id: String },
    /// Streaming progress event from an in-flight review.
    ReviewProgress {
        run_id: String,
        event: ReviewProgress,
    },
    /// Review finished — final confirmed findings list.
    ReviewResult {
        run_id: String,
        findings: Vec<Finding>,
    },
    /// Review aborted with an error (bad diff, provider failure, etc.).
    ReviewError { run_id: String, text: String },

    /// A session's AI-generated nickname landed on disk. Frontend uses this
    /// to refresh the sidebar so the newly-titled row replaces the
    /// first-user-message fallback without waiting for the next `done`.
    SessionTitleUpdated { session_id: String, title: String },

    /// A session's background-mode was changed. Confirms `SetBackgroundMode`.
    BackgroundModeChanged {
        session_id: String,
        mode: BackgroundMode,
    },

    /// A background-only session transitioned running → idle. Emitted when
    /// a turn finishes on a slot with zero attached clients, so the sidebar
    /// can drop the "running" indicator even though nobody's tab is
    /// receiving the per-turn `Done` frame.
    SessionBackgroundIdle { session_id: String },

    /// Same idea for the running-transition — emitted on the SLOT's own
    /// channel when a turn kicks off, so if a client attaches mid-turn
    /// it can pick up the "still running" indicator without polling.
    SessionBackgroundRunning { session_id: String },

    /// Per-round + running-total token usage from the provider. Emitted at
    /// the end of every provider round the moment a usage trailer arrives;
    /// the UI uses `totals` for its status-bar counter and `round` for
    /// per-turn indicators.
    Usage {
        round: TokenUsage,
        totals: UsageTotals,
    },

    /// Post-round auto-extractor recorded N durable facts to
    /// `.mira/episodic.jsonl`. Emitted only when `count > 0`; the UI can
    /// render a small "mira remembered N things" chip to make cross-session
    /// memory writes visible.
    MemoryLearned { count: usize },

    /// The harness rolled up N older non-system messages into a single
    /// summary before the current round's model call. UI can render a
    /// "compacted N turns" chip for transparency.
    Compacted { messages_removed: usize },

    /// A `/goal` was set on this session. Emitted immediately after
    /// `SetGoal` so the UI can flip its state without waiting for the
    /// next turn.
    GoalSet { goal: Goal },
    /// The session's standing goal was cleared (by the user or the
    /// UI). Emitted whether or not a turn is running.
    GoalCleared,
    /// The evaluator just ran mid-goal. `iteration` is the count
    /// *after* the bump (1-indexed) and `status` reflects the goal
    /// state post-evaluation. `reason` is the evaluator's free-text
    /// note — shown in the goal panel so the user can see why the
    /// loop is still going.
    GoalProgress {
        iteration: usize,
        max_iterations: usize,
        status: GoalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// The goal reached a terminal state and no further autonomous
    /// iterations will run. The UI switches from the "working…" pill
    /// to the terminal one keyed on `status`.
    GoalDone {
        status: GoalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// The model called the `plan` tool. Open a review modal so the user can
    /// approve / edit / cancel. Client answers with
    /// `ClientMsg::PromptResponse` carrying a `Plan { … }` variant.
    PlanRequest {
        prompt_id: String,
        plan: PlanProposal,
    },
    /// The model called the `ask_user` tool with a batch of clarifying
    /// questions. Client renders the question card and answers with
    /// `ClientMsg::PromptResponse` carrying an `AskUser { … }` variant.
    AskUserRequest {
        prompt_id: String,
        proposal: AskUserProposal,
    },

    // -------- subagent (child session) event forwarding --------
    //
    // The `agent` tool now streams each child HarnessEvent to the parent's
    // WS with a `parent_call_id` tag so the frontend can build a live
    // per-child transcript in the right-side SubagentPanel. Kept as
    // distinct variants (rather than a wrapped envelope) so the JS side
    // pattern-matches by `type` the same way it already does.
    /// A subagent has been spawned. Sent immediately before the child
    /// begins consuming its prompt so the panel can open a tab.
    SubagentStarted {
        /// The parent's tool-call id — the AgentTool call that spawned
        /// this child. Every subsequent `Subagent*` frame with the same
        /// value belongs to this child.
        parent_call_id: String,
        /// Child session id — useful for future features (persistence,
        /// deep links) but not required by the current UI.
        agent_id: String,
        /// Model the child is running under. Lets the panel surface a
        /// small caption without extra bookkeeping.
        model: String,
        /// Full prompt the child received. Small ceiling upstream via
        /// `truncate_for_history`, so this can safely include the raw text.
        prompt: String,
    },
    /// Fragment of the child's assistant text.
    SubagentToken {
        parent_call_id: String,
        text: String,
    },
    /// The child dispatched a tool call.
    SubagentToolStart {
        parent_call_id: String,
        call: ToolCall,
    },
    /// The child's tool call finished.
    SubagentToolEnd {
        parent_call_id: String,
        result: ToolResult,
    },
    /// A child-level warning (verify failure, hit max_rounds, etc.).
    SubagentWarning {
        parent_call_id: String,
        text: String,
    },
    /// Intermediate progress update from the child, emitted when the
    /// subagent explicitly calls the `progress` tool. Distinct from
    /// tokens (which are freeform assistant text) and from warnings
    /// (which imply something's off) — this is the child announcing
    /// "here's where I am" during a long investigation.
    SubagentProgress {
        parent_call_id: String,
        text: String,
    },
    /// The child produced a final summary and its type has
    /// `review_required: true`. The parent's turn is paused; the UI
    /// must show the proposed summary and the user picks approve or
    /// deny (with optional note) before the tool result flows back to
    /// the parent.
    SubagentReviewRequest {
        parent_call_id: String,
        /// Prompt id the client echoes back in `PromptResponse`.
        /// Convention: `<parent_call_id>-review`.
        prompt_id: String,
        /// Full proposed summary (post-warnings, pre-worktree note).
        summary: String,
    },
    /// The child finished — final assistant text is already on the way as
    /// the AgentTool's `ToolEnd` frame. This exists purely so the panel
    /// can flip its status pill from "working" to "done" without waiting
    /// for the parent to update the same call id.
    SubagentDone { parent_call_id: String },
    /// A subagent posted an entry to the parent session's shared
    /// scratchpad via the `scratchpad_note` tool. Peers spawned from the
    /// same session share one pad so parallel researchers can see each
    /// other's mid-flight findings without waiting for a final summary
    /// round-trip.
    SubagentScratchpadNote {
        /// The `agent`-tool call id that spawned the author. Lets the
        /// panel attribute the note to the right child tab in addition
        /// to the shared pad view.
        parent_call_id: String,
        /// The parent session's id — the pad key. Multiple `parent_call_id`
        /// values can share this when a single user turn spawned several
        /// children in parallel.
        parent_session_id: String,
        /// The posted note. `author` is the subagent type (e.g. `explore`).
        entry: crate::interactive::ScratchpadEntry,
    },
}

impl ServerMsg {
    /// Map a raw harness event into a wire frame. Approval frames are emitted
    /// from the approver, not the event stream, so they don't appear here.
    pub fn from_harness(evt: HarnessEvent) -> Self {
        match evt {
            HarnessEvent::Token(text) => Self::Token { text },
            HarnessEvent::ToolStart(call) => Self::ToolStart { call },
            HarnessEvent::ToolEnd(result) => Self::ToolEnd { result },
            HarnessEvent::TurnComplete => Self::TurnComplete,
            HarnessEvent::Done => Self::Done,
            HarnessEvent::Warning(text) => Self::Warning { text },
            HarnessEvent::Usage { round, totals } => Self::Usage { round, totals },
            HarnessEvent::MemoryLearned { count } => Self::MemoryLearned { count },
            HarnessEvent::Compacted { messages_removed } => Self::Compacted { messages_removed },
            HarnessEvent::GoalSet { goal } => Self::GoalSet { goal },
            HarnessEvent::GoalCleared => Self::GoalCleared,
            HarnessEvent::GoalProgress {
                iteration,
                max_iterations,
                status,
                reason,
            } => Self::GoalProgress {
                iteration,
                max_iterations,
                status,
                reason,
            },
            HarnessEvent::GoalDone { status, reason } => Self::GoalDone { status, reason },
            HarnessEvent::ToolProgress { call_id, line } => Self::ToolProgress { call_id, line },
            HarnessEvent::ToolPreview { call_id, preview } => {
                Self::ToolPreview { call_id, preview }
            }
        }
    }
}
