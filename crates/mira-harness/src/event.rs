use mira_ai::TokenUsage;
use mira_core::{ToolCall, ToolResult};
use mira_tools::DiffPreview;

use crate::goal::{Goal, GoalStatus};
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
    /// One line of live stdout+stderr from a running tool. Emitted while
    /// the tool is still executing so the UI can render progress under
    /// the pending card instead of staring at a spinner. `call_id` maps
    /// to the corresponding `ToolStart`; the final `ToolEnd.content`
    /// still carries the full transcript so nothing depends on the UI
    /// having caught every line.
    ToolProgress { call_id: String, line: String },
    /// Diff preview captured for an `edit_file` / `write_file` call just
    /// before it runs. Emitted so live clients see the same diff whether
    /// the write was auto-allowed or approval-gated (the approver only
    /// ships a preview on the `Ask` path). Also persisted alongside the
    /// session so reload can render the same diff.
    ToolPreview {
        call_id: String,
        preview: DiffPreview,
    },
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
    MemoryLearned { count: usize },
    /// The harness compacted `messages_removed` older non-system messages
    /// into a single synthetic summary before this round's model call.
    /// Fired at most once per round, at the point history was rewritten.
    Compacted { messages_removed: usize },
    /// A `/goal` was set on the session. Emitted immediately when the
    /// user sets or replaces the standing goal — even outside a
    /// running turn, so the UI can flip its state right away.
    GoalSet { goal: Goal },
    /// The user (or the UI) cleared the standing goal. Emitted whether
    /// or not a turn is currently running.
    GoalCleared,
    /// The evaluator just ran and produced a verdict. Sent right before
    /// the harness decides whether to loop or exit. `iteration` reflects
    /// the count *after* the bump (so the first evaluation shows `1`).
    GoalProgress {
        iteration: usize,
        max_iterations: usize,
        status: GoalStatus,
        /// Evaluator's free-text reason, if any.
        reason: Option<String>,
    },
    /// The goal has reached a terminal state (met, impossible,
    /// needs_user, cleared, exhausted). No more autonomous iterations
    /// will run on this record.
    GoalDone {
        status: GoalStatus,
        reason: Option<String>,
    },
}

/// Warning prefixes for failures that end the turn without an answer.
pub(crate) const PROVIDER_ERROR: &str = "provider error";
pub(crate) const STREAM_ERROR: &str = "stream error";
pub(crate) const STREAM_TIMEOUT: &str = "stream timed out";

/// True for a [`HarnessEvent::Warning`] that means the turn failed (the
/// provider refused the request, or its stream broke or hung), as
/// opposed to notes like a denied tool call or a hook message.
pub fn is_fatal_warning(warning: &str) -> bool {
    [PROVIDER_ERROR, STREAM_ERROR, STREAM_TIMEOUT]
        .iter()
        .any(|p| warning.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_turn_ending_failures_are_fatal() {
        assert!(is_fatal_warning(&format!("{PROVIDER_ERROR}: 401")));
        assert!(is_fatal_warning(&format!("{STREAM_ERROR}: reset")));
        assert!(is_fatal_warning(&format!("{STREAM_TIMEOUT} after 300s")));
        assert!(!is_fatal_warning("[hook] blocked"));
        assert!(!is_fatal_warning("[stuck-loop] same calls"));
    }
}
