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

use crate::interactive::{PlanProposal, PromptResponse};

/// Client → server.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Start a new user turn.
    Send { text: String },
    /// Answer a pending approval prompt.
    Approve { call_id: String, allow: bool },
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
    },
    /// Drop the session's standing goal. Idempotent — clearing a
    /// session without a goal is a no-op.
    ClearGoal,
    /// Ask the server to re-emit its current state (used on reconnect).
    Sync,
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
    SubagentToken { parent_call_id: String, text: String },
    /// The child dispatched a tool call.
    SubagentToolStart { parent_call_id: String, call: ToolCall },
    /// The child's tool call finished.
    SubagentToolEnd { parent_call_id: String, result: ToolResult },
    /// A child-level warning (verify failure, hit max_rounds, etc.).
    SubagentWarning { parent_call_id: String, text: String },
    /// Intermediate progress update from the child, emitted when the
    /// subagent explicitly calls the `progress` tool. Distinct from
    /// tokens (which are freeform assistant text) and from warnings
    /// (which imply something's off) — this is the child announcing
    /// "here's where I am" during a long investigation.
    SubagentProgress { parent_call_id: String, text: String },
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
        }
    }
}
