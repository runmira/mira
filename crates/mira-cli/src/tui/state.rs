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
/// mutation happens through the small `push_*` / `input_*` helpers so
/// invariants (like "assistant tokens append onto a trailing Assistant
/// entry") live in one place.
pub struct TuiState {
    entries: Vec<LogEntry>,

    /// Composer buffer. May contain '\n' for multi-line input (Ctrl+J).
    input: String,
    /// Ring of submitted messages for up/down arrow recall.
    history: Vec<String>,
    /// Cursor into `history` when browsing; `None` means we're on a fresh
    /// composer, not viewing a past message.
    history_cursor: Option<usize>,
    /// The composer's live contents at the moment the user first pressed
    /// Up — restored when they browse back past the newest history entry.
    history_stash: String,

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
    /// Non-empty while showing a one-shot status blip (e.g. "model → X").
    pub flash: Option<String>,
}

impl TuiState {
    pub fn new(model: String, mode: Mode) -> Self {
        Self {
            entries: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            history_cursor: None,
            history_stash: String::new(),
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

    /// Replay a tool result from history — we don't know its is_error
    /// flag anymore (the wire message doesn't carry it), so callers pass
    /// their best guess. `denied by policy` and `io error` snippets get
    /// marked as errors so the visual state matches how they originally
    /// rendered.
    pub fn push_tool_result_replay(&mut self, content: &str) {
        let is_error = content.starts_with("denied by policy")
            || content.starts_with("io error")
            || content.starts_with("no such tool")
            || content.starts_with("tool failed")
            || content.starts_with("invalid arguments");
        self.entries.push(LogEntry::ToolResult {
            ok: !is_error,
            snippet: first_line(content, 200),
        });
    }

    /// Push a raw tool call from history (name + args string).
    pub fn push_tool_call_raw(&mut self, name: String, args: String) {
        self.entries.push(LogEntry::ToolCall { name, args });
    }

    /// Push an already-complete assistant message from history.
    pub fn push_assistant(&mut self, s: String) {
        self.entries.push(LogEntry::Assistant(s));
    }

    // ---- input ----

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn input_push(&mut self, c: char) {
        self.leave_history_browse();
        self.input.push(c);
    }

    pub fn input_push_str(&mut self, s: &str) {
        self.leave_history_browse();
        self.input.push_str(s);
    }

    pub fn input_newline(&mut self) {
        self.leave_history_browse();
        self.input.push('\n');
    }

    pub fn input_backspace(&mut self) {
        self.leave_history_browse();
        self.input.pop();
    }

    pub fn input_clear(&mut self) -> String {
        self.history_cursor = None;
        self.history_stash.clear();
        std::mem::take(&mut self.input)
    }

    pub fn is_input_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    /// Record a submitted message so up-arrow can recall it later.
    pub fn remember_submission(&mut self, s: &str) {
        // Skip duplicate consecutive entries — mirrors bash/zsh behaviour.
        if self.history.last().map(String::as_str) == Some(s) {
            return;
        }
        self.history.push(s.to_owned());
    }

    /// Up-arrow: step backward through submitted messages. First press
    /// stashes the live composer so we can restore it if the user cycles
    /// back to the present.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_cursor {
            None => {
                self.history_stash = std::mem::take(&mut self.input);
                self.history.len() - 1
            }
            Some(0) => 0, // clamp at oldest
            Some(i) => i - 1,
        };
        self.history_cursor = Some(next);
        self.input = self.history[next].clone();
    }

    /// Down-arrow: step forward. Past the newest, restore the stash.
    pub fn history_next(&mut self) {
        let Some(i) = self.history_cursor else { return };
        if i + 1 < self.history.len() {
            self.history_cursor = Some(i + 1);
            self.input = self.history[i + 1].clone();
        } else {
            self.history_cursor = None;
            self.input = std::mem::take(&mut self.history_stash);
        }
    }

    /// Any input mutation (typing, backspace, paste) drops us out of
    /// history browse mode — otherwise the next up-arrow would clobber
    /// their edit.
    fn leave_history_browse(&mut self) {
        if self.history_cursor.is_some() {
            self.history_cursor = None;
            self.history_stash.clear();
        }
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
