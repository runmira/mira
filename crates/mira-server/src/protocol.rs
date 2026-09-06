//! WebSocket wire protocol.
//!
//! Every frame is a single JSON object with a `"type"` discriminator. Server
//! frames carry harness events + approval requests; client frames carry user
//! input, approvals, and control (mode/model swap, interrupt, clear).

use mira_core::{Message, ToolCall, ToolResult};
use mira_harness::HarnessEvent;
use mira_policy::Mode;
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
        }
    }
}
