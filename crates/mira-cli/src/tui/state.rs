use mira_core::{ToolCall, ToolResult};
use mira_policy::Mode;

use crate::tui::approver::ApprovalRequest;

/// One row in the visible transcript.
///
/// Assistant tokens accumulate onto the trailing `Assistant` entry so a
/// streaming reply reads as one paragraph, not one entry per token.
#[derive(Clone, Debug)]
pub enum LogEntry {
    User(String),
    Assistant(String),
    ToolCall { name: String, args: String },
    ToolResult { ok: bool, snippet: String },
    Warning(String),
    Info(String),
}

/// All state the render loop reads.
///
/// Kept dumb on purpose — every field is public to the tui modules but
/// mutation happens through the small `push_*` helpers so invariants (like
/// "assistant tokens append onto a trailing Assistant entry") live in one
/// place.
pub struct TuiState {
    entries: Vec<LogEntry>,
    input: String,
    pub mode: Mode,
    pub model: String,
    pub streaming: bool,
    pub pending_approval: Option<ApprovalRequest>,
    pub esc_pending: bool,
    pub scroll: u16,
    /// When true, the UI auto-scrolls the transcript to the bottom on new
    /// entries. Flipped off when the user PgUps, on when they PgDn back.
    pub follow_tail: bool,
    pub should_quit: bool,
    /// Non-empty while the transcript is being cleared (`/clear`) or reset
    /// to show a one-shot status line ("switched model to X").
    pub flash: Option<String>,
}

impl TuiState {
    pub fn new(model: String, mode: Mode) -> Self {
        Self {
            entries: Vec::new(),
            input: String::new(),
            mode,
            model,
            streaming: false,
            pending_approval: None,
            esc_pending: false,
            scroll: 0,
            follow_tail: true,
            should_quit: false,
            flash: None,
        }
    }

    // ---- transcript ----

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn clear_entries(&mut self) {
        self.entries.clear();
        self.scroll = 0;
        self.follow_tail = true;
    }

    pub fn push_user(&mut self, s: String) {
        self.entries.push(LogEntry::User(s));
    }

    pub fn push_info(&mut self, s: impl Into<String>) {
        self.entries.push(LogEntry::Info(s.into()));
    }

    pub fn push_warning(&mut self, s: String) {
        self.entries.push(LogEntry::Warning(s));
    }

    /// Append a streamed assistant token onto the tail assistant entry —
    /// or start a new one if the previous entry was something else.
    pub fn append_token(&mut self, t: &str) {
        if let Some(LogEntry::Assistant(buf)) = self.entries.last_mut() {
            buf.push_str(t);
        } else {
            self.entries.push(LogEntry::Assistant(t.to_owned()));
        }
    }

    pub fn push_tool_call(&mut self, call: &ToolCall) {
        self.entries.push(LogEntry::ToolCall {
            name: call.function.name.clone(),
            args: call.function.arguments.clone(),
        });
    }

    pub fn push_tool_result(&mut self, r: &ToolResult) {
        self.entries.push(LogEntry::ToolResult {
            ok: !r.is_error,
            snippet: first_line(&r.content, 200),
        });
    }

    // ---- input ----

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn input_push(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn input_backspace(&mut self) {
        self.input.pop();
    }

    pub fn input_clear(&mut self) -> String {
        std::mem::take(&mut self.input)
    }

    pub fn is_input_empty(&self) -> bool {
        self.input.trim().is_empty()
    }
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_owned()
    } else {
        let truncated: String = line.chars().take(max).collect();
        format!("{truncated}…")
    }
}
