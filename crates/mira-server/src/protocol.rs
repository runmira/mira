//! WebSocket wire protocol.
//!
//! Every frame is a single JSON object with a `"type"` discriminator. Server
//! frames carry harness events + approval requests; client frames carry user
//! input, approvals, and control (mode/model swap, interrupt, clear).

use mira_ai::TokenUsage;
use mira_core::{Message, ToolCall, ToolResult};
use mira_harness::{HarnessEvent, TurnMeta, UsageTotals};
use mira_policy::Mode;
use mira_review::{Finding, Progress as ReviewProgress};
use mira_tools::DiffPreview;
use serde::{Deserialize, Serialize};

/// Client → server.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Start a new user turn.
    Send { text: String },
    /// Answer a pending approval prompt.
    Approve { call_id: String, allow: bool },
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
    ReviewProgress { run_id: String, event: ReviewProgress },
    /// Review finished — final confirmed findings list.
    ReviewResult { run_id: String, findings: Vec<Finding> },
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
        }
    }
}
