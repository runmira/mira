use mira_ai::TokenUsage;
use mira_core::{ToolCall, ToolResult};

use crate::persist::UsageTotals;

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
    /// Token usage for the round that just finished, plus running session
    /// totals. Emitted immediately after each provider-reported usage
    /// trailer so the UI can update a live cost/tokens indicator.
    Usage {
        /// Usage for the round that just finished.
        round: TokenUsage,
        /// Aggregate session totals (including `round`).
        totals: UsageTotals,
    },
    /// Post-round auto-extractor added N durable facts to the episodic
    /// store. Emitted as its own frame (rather than a `Warning`) so a UI
    /// can render it distinctly — a subtle "mira remembered N things"
    /// chip beats a scary-looking warning banner. `count == 0` is not
    /// emitted; the pass just stays quiet when nothing was worth keeping.
    MemoryLearned {
        count: usize,
    },
}
