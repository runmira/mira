use mira_core::{ToolCall, ToolResult};

/// UI-facing event stream produced by [`crate::Session::send`].
///
/// The harness re-shapes provider events plus its own tool-dispatch progress
/// into one stream so a renderer only has to consume one type.
#[derive(Clone, Debug)]
pub enum HarnessEvent {
    /// A fragment of assistant text.
    Token(String),
    /// The model requested a tool call. Emitted before it runs.
    ToolStart(ToolCall),
    /// A tool call finished.
    ToolEnd(ToolResult),
    /// The current model turn is complete (may kick off another turn if
    /// tool calls were dispatched).
    TurnComplete,
    /// The whole user turn is done — no further tool calls, we're waiting
    /// for the next user input.
    Done,
    /// Non-fatal warning surfaced to the UI (e.g. denied tool call).
    Warning(String),
}
