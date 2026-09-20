use std::time::Instant;

use mira_core::{ToolCall, ToolResult};
use mira_harness::{Goal, UsageTotals};
use mira_policy::Mode;
use mira_tools::DiffPreview;

use crate::tui::approver::ApprovalRequest;
use crate::tui::render::layout::TranscriptLayout;

/// An approval request enriched with an optional diff preview (present
/// only for edit_file / write_file calls). The preview is computed
/// asynchronously by the event loop after the request arrives.
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub preview: Option<DiffPreview>,
}

/// One item in the agent's task list, as the TUI sees it.
///
/// Hydrated from the structured `data` payloads that `task_create` /
/// `task_update` / `task_list` return (mirrors
/// `mira_tools::tasks::TaskItem`); the TUI never talks to the task
/// store directly — it just observes the tool stream.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct TaskItem {
    pub id: u32,
    pub subject: String,
    /// Present-continuous form shown while the task is in progress
    /// ("Running tests"). Falls back to `subject`.
    #[serde(default)]
    pub active_form: Option<String>,
    pub status: TaskStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    /// Soft-deleted on the wire — dropped from the panel on upsert.
    Deleted,
}

/// Full tool result content is capped at this many bytes so a runaway
/// tool doesn't balloon the session's memory footprint. Enough for a
/// typical ripgrep hit list or a few dozen lines of build output.
const TOOL_RESULT_MAX_BYTES: usize = 8 * 1024;

/// Visible-entry cap. Once exceeded, the oldest entries are dropped
/// and `TuiState::dropped_entries` grows so the render layer can show
/// "… N earlier entries truncated" at the top. Mirrors the harness's
/// own history-compaction model rather than growing memory forever.
const MAX_ENTRIES: usize = 500;

/// One row in the visible transcript.
///
/// Assistant tokens accumulate onto the trailing `Assistant` entry so a
/// streaming reply reads as one paragraph, not one entry per token.
#[derive(Clone, Debug)]
pub enum LogEntry {
    User(String),
    Assistant(String),
    /// Turn-end marker: rendered under the assistant reply as
    /// `✳ Baked for 12.4s`. Pushed once per `HarnessEvent::Done`, so
    /// the transcript grows a visual full-stop after every reply — a
    /// design borrowed from Claude Code. Verb is chosen at render
    /// time from [`crate::tui::components::status::turn_end_label`]
    /// using the millisecond bucket, so an identical duration always
    /// reads the same word ("baked for 4s" every time, not a random
    /// flavor each render).
    TurnEnd {
        elapsed_ms: u32,
        /// Receipt — what this turn actually did, computed at push
        /// time by scanning the turn's entries. Zero when the turn
        /// ran no tools, so the marker stays a quiet full-stop.
        files: u16,
        adds: u32,
        dels: u32,
        tools: u16,
        cost: Option<f64>,
    },
    ToolCall {
        name: String,
        args: String,
        /// Diff preview computed at approval time (or, in auto-approve
        /// modes, when the tool call arrives). Rendered inside the
        /// tool group after a successful Edit/Write so the user sees
        /// exactly what changed, not just "wrote N bytes".
        preview: Option<DiffPreview>,
        /// Tool-call id from the underlying `ToolCall.id`. Used by
        /// the event loop to look this entry back up when the paired
        /// preview arrives asynchronously.
        call_id: String,
        /// Wall-clock stamp when the ToolStart event arrived. Read by
        /// the renderer to draw a `· 1.2s` ticker next to in-flight
        /// `◐` tool headers so long file reads / long bash calls stop
        /// looking hung. Not stored in the session record because
        /// `Instant` isn't monotonic across process restarts anyway.
        started_at: Instant,
    },
    ToolResult {
        ok: bool,
        snippet: String,
        /// Full (possibly truncated at [`TOOL_RESULT_MAX_BYTES`]) tool
        /// output. `Ctrl+E` toggles [`expanded`] to render this in full
        /// instead of the one-line snippet.
        full: String,
        expanded: bool,
        /// User override for auto-collapse (`1–9` toggles recent
        /// groups): `None` = automatic (groups from turns before the
        /// last user prompt render header-only), `Some(false)` = pinned
        /// open, `Some(true)` = explicitly collapsed.
        collapsed_override: Option<bool>,
    },
    Warning(String),
    Info(String),
    /// First-run startup banner — structured so the component renders
    /// it with brand colors (info lines are muted-italic by design).
    Welcome {
        model: String,
        mode: Mode,
        cwd: String,
        tip: &'static str,
    },
}

/// Which overlay list is open above the composer, if any.
///
/// `Slash` fires when the composer starts with `/` — the filter is the
/// rest of the line. `AtFile` fires when there's an `@word` at cursor —
/// the filter is what comes after `@`, and completions come from
/// `rg --files` in the cwd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    None,
    Slash,
    AtFile,
    /// Fires when the composer starts with `/model ` — completions
    /// come from the live provider's model list (cached).
    Model,
    /// Fires when the composer starts with `/theme ` — completions
    /// are the bundled presets plus the `reload` / `save` verbs.
    Theme,
    /// Fires when the composer starts with `/save ` — completions
    /// come from the file index (same rg output the `@file` picker
    /// uses) plus a handful of sensible defaults.
    SavePath,
}

pub struct PaletteState {
    pub kind: Palette,
    /// Selection cursor within the filtered results (0-based).
    pub cursor: usize,
    /// The last-computed filtered results (indices into the source list).
    /// Rebuilt whenever the composer or `kind` changes.
    pub matches: Vec<PaletteItem>,
}

impl PaletteState {
    pub fn none() -> Self {
        Self {
            kind: Palette::None,
            cursor: 0,
            matches: Vec::new(),
        }
    }
}

/// One entry in a palette result list.
///
/// `insert` is the string to substitute into the composer when the user
/// picks this item (replacing the trigger + filter run). `title` is the
/// primary rendering, `detail` the greyed hint on the right.
#[derive(Clone, Debug)]
pub struct PaletteItem {
    pub insert: String,
    pub title: String,
    pub detail: String,
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
    /// Byte offset of the caret within `input`. Always at a char
    /// boundary. All input mutations move it consistently.
    cursor: usize,
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
    /// Cached `git rev-parse --abbrev-ref HEAD` — refreshed at
    /// startup and after every turn, shown as a `⎇ <branch>` chip in
    /// the header. `None` when the cwd isn't a git repo.
    pub git_branch: Option<String>,
    /// Uncommitted-change count from `git status --porcelain`,
    /// refreshed with the branch. `Some(0)` = clean worktree; `None`
    /// outside a repo or when git failed.
    pub git_dirty: Option<u32>,
    /// Last output line of the in-flight tool `(call_id, line)` — the
    /// live tail under the `◐` header so a 90-second command doesn't
    /// look hung. Cleared on ToolEnd / interrupt.
    pub tool_tail: Option<(String, String)>,
    pub streaming: bool,
    pub pending_approval: Option<PendingApproval>,
    pub esc_pending: bool,
    pub scroll: u16,
    /// Height of the transcript viewport, kept in sync from both the
    /// resize event and the draw pass. Read by the key handler so
    /// PgUp/PgDn move by a real page instead of a hard-coded step.
    pub viewport_height: u16,
    /// Width of the transcript viewport. Wrapping — and therefore the
    /// whole transcript layout — depends on it; a width change
    /// invalidates the derived layout cache.
    pub viewport_width: u16,
    /// When true, the UI auto-scrolls the transcript to the bottom on new
    /// entries. Flipped off when the user PgUps, on when they PgDn back.
    pub follow_tail: bool,
    pub should_quit: bool,
    /// Non-empty while showing a one-shot status blip (e.g. "model → X").
    pub flash: Option<String>,
    /// Session-wide token totals, updated whenever the provider emits usage.
    /// Zeroed for providers that don't report usage — the status bar renders
    /// nothing in that case.
    pub usage: UsageTotals,
    /// Session token totals captured at the start of the current turn.
    /// The in-turn streaming indicator renders `usage - turn_usage_baseline`
    /// so each turn's counter starts at zero instead of inheriting the
    /// previous turn's running total. Re-snapshotted on every `start_stream`.
    pub turn_usage_baseline: UsageTotals,
    /// Optional session-wide spend cap in USD. When set, the header
    /// dollar figure paints red past the cap, and
    /// [`super::start_stream`] blocks the next send until the user
    /// either raises the cap or clears it with `/budget off`. Wired
    /// through `/budget <$X>` in the TUI. Not persisted across sessions.
    pub budget_usd: Option<f64>,
    /// Messages typed while a turn was streaming. Sent FIFO as each
    /// turn completes; ↑ while streaming pops the newest back into the
    /// composer for editing. Survives an interrupt so nothing the user
    /// typed is lost.
    pub queued: Vec<String>,
    /// The agent's task list, hydrated from `task_*` tool payloads.
    /// Rendered under the streaming indicator with checkboxes so the
    /// user can follow the plan while the agent works. Empty until the
    /// agent first touches the task tools.
    pub tasks: Vec<TaskItem>,
    /// Standing `/goal`, if any. Renders as a chip in the header + a
    /// live-updating line in the status bar during autonomous runs.
    /// `None` means goal-directed mode is off.
    pub goal: Option<Goal>,
    /// Overlay above the composer. `Palette::None` = no overlay.
    pub palette: PaletteState,
    /// When the current provider stream started. Drives the "3.2s"
    /// elapsed counter next to the "thinking" indicator so long silent
    /// pauses look alive instead of hung. Cleared on `HarnessEvent::Done`.
    pub stream_started_at: Option<Instant>,
    /// Whether crossterm mouse capture is currently on. Off by default
    /// so the terminal's own text selection keeps working; Alt+M
    /// toggles it. The event loop reads this to enable/disable capture
    /// as it changes.
    pub mouse_capture: bool,
    /// How many entries have been dropped off the front of `entries`
    /// to stay under [`MAX_ENTRIES`]. Zero on a fresh session; render
    /// prepends "… N earlier entries truncated" whenever this is
    /// non-zero.
    pub dropped_entries: usize,
    /// Ctrl+R search mode. `None` when off; when set, the overlay is
    /// active and typing goes to the query instead of the composer.
    pub search: Option<SearchState>,
    /// Live pastes stashed out of the composer buffer. When a bracketed
    /// paste larger than [`PASTE_COLLAPSE_LINES`] arrives, the actual
    /// content lands here and the composer gets a `[[paste:<id>]]`
    /// placeholder — rendered visually as `[pasted N lines]`. On
    /// submit, [`Self::expand_pastes`] swaps placeholders back for
    /// their full text. Cleared with the composer.
    pub pastes: Vec<PasteChunk>,
    /// Next id for [`Self::stash_paste`]. Monotonic per TUI process —
    /// never re-used within a session so the placeholder-to-content
    /// map stays stable even after some pastes are removed.
    next_paste_id: u32,
    /// When non-`None`, the status bar paints a red `!` chip until this
    /// instant. Fired by [`Self::error_flash`] on provider 401/429
    /// warnings — a full-second visual beat that's harder to miss
    /// than a yellow line scrolling past. Cleared naturally once
    /// `Instant::now()` passes the deadline.
    pub error_flash_until: Option<Instant>,
    /// One-line label for the current error flash — shown red-bg in
    /// the status row while [`Self::error_flash_until`] is live.
    pub error_flash_label: Option<String>,
    /// Anchor entry index for turn-nav (`Ctrl+↑` / `Ctrl+↓`). Persisted
    /// across presses so a sequence steps backwards through the user
    /// prompts one at a time. Reset any time the composer accepts a
    /// keystroke or a new user message lands.
    turn_nav_idx: Option<usize>,
    /// One-shot scroll request from the key handler to the render
    /// layer: "scroll so entry `idx` sits near the top of the visible
    /// area." Consumed on the next draw so a follow-up scroll from
    /// the user doesn't get stomped.
    pub turn_scroll_target: Option<usize>,
    /// Derived transcript geometry, cached by the render pass and
    /// invalidated by resize. **Derived state, not authoritative** —
    /// scroll handlers read `cached_tail()` off it; they never write
    /// it. `None` until the first draw or after a width change.
    pub layout_cache: Option<TranscriptLayout>,
    /// Process start instant — drives the goal panel's breathing
    /// animation phase so the pulse doesn't need its own timer.
    pub booted_at: Instant,
}

/// Placeholder token that stands in for a stashed paste inside
/// `state.input`. Kept short so a normal-width composer can hold
/// several placeholders without wrapping; the visible rendering is
/// `[pasted N lines]`, done at draw time.
pub fn paste_placeholder(id: u32) -> String {
    format!("[[paste:{id}]]")
}

/// A collapsed paste, stashed out of the composer buffer. See
/// [`TuiState::pastes`].
#[derive(Clone, Debug)]
pub struct PasteChunk {
    pub id: u32,
    /// Full pasted content, verbatim.
    pub content: String,
    /// Line count at stash time — cached so we don't recount for the
    /// placeholder label on every render.
    pub lines: usize,
}

/// Any bracketed paste with at least this many lines (or a hard char
/// count above [`PASTE_COLLAPSE_CHARS`]) gets collapsed to a
/// placeholder. Chosen so a normal multi-line message you meant to
/// paste (5-10 lines of context) still lives in the composer, but a
/// 500-line dump doesn't scroll it off screen.
pub const PASTE_COLLAPSE_LINES: usize = 12;
/// Character cap — long single-line pastes (e.g. a full URL-encoded
/// blob or a wide one-line CSV) also count as "big" even if they
/// don't have many newlines.
pub const PASTE_COLLAPSE_CHARS: usize = 800;

/// Reverse-search state — populated when Ctrl+R is pressed.
///
/// Hits are stored as (entry_index, byte_offset_into_entry_text). The
/// active hit is `hits[cursor]`; the render layer uses it to force
/// scroll to reveal the matched entry.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub query: String,
    pub hits: Vec<(usize, usize)>,
    pub cursor: usize,
}

impl TuiState {
    pub fn new(model: String, mode: Mode) -> Self {
        Self {
            entries: Vec::new(),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_cursor: None,
            history_stash: String::new(),
            mode,
            model,
            git_branch: None,
            git_dirty: None,
            tool_tail: None,
            streaming: false,
            pending_approval: None,
            esc_pending: false,
            scroll: 0,
            viewport_height: 0,
            viewport_width: 0,
            follow_tail: true,
            should_quit: false,
            flash: None,
            usage: UsageTotals::default(),
            turn_usage_baseline: UsageTotals::default(),
            budget_usd: None,
            queued: Vec::new(),
            tasks: Vec::new(),
            goal: None,
            palette: PaletteState::none(),
            stream_started_at: None,
            mouse_capture: false,
            dropped_entries: 0,
            search: None,
            pastes: Vec::new(),
            next_paste_id: 1,
            error_flash_until: None,
            error_flash_label: None,
            turn_nav_idx: None,
            turn_scroll_target: None,
            layout_cache: None,
            booted_at: Instant::now(),
        }
    }

    // ---- viewport / derived layout ----

    /// Explicit resize transition — the event loop calls this on
    /// `Event::Resize` (and the draw pass on first frame). Updates the
    /// viewport, invalidates derived layout when the width changed
    /// (wrapping is width-dependent), and reconciles scroll so the
    /// view lands somewhere valid.
    pub fn handle_resize(&mut self, width: u16, height: u16) {
        let width_changed = self.viewport_width != width;
        self.viewport_width = width;
        self.viewport_height = height;
        if width_changed {
            // Wrapping is width-dependent — the row table is meaningless
            // at a new width. The next draw rebuilds it and clamps
            // scroll; reconcile below defers via the missing cache.
            self.layout_cache = None;
        }
        self.reconcile_scroll();
    }

    /// Clamp scroll back into the valid range after a viewport change.
    /// Following the tail re-snaps to it; a user-held position is
    /// clamped so it never points past the end. No-op when the layout
    /// cache can't describe the current viewport (next draw fixes it).
    pub fn reconcile_scroll(&mut self) {
        let Some(tail) = self.cached_tail() else {
            return;
        };
        if self.follow_tail {
            self.scroll = tail;
        } else {
            self.scroll = self.scroll.min(tail);
        }
    }

    /// Tail offset (max valid scroll) from the derived layout cache —
    /// what scroll handlers clamp against. At most one event stale
    /// (content may have grown since the last draw); handlers tolerate
    /// that and the next draw corrects everything. `None` before the
    /// first draw or after a width-changing resize.
    pub fn cached_tail(&self) -> Option<u16> {
        self.layout_cache.as_ref().map(|l| l.tail())
    }

    // ---- pastes ----

    /// Stash a large paste and return the placeholder token to insert
    /// into the composer in its place. Called from the paste handler
    /// when the incoming text is big enough to warrant collapsing.
    pub fn stash_paste(&mut self, content: String) -> String {
        let lines = content.matches('\n').count() + 1;
        let id = self.next_paste_id;
        self.next_paste_id += 1;
        self.pastes.push(PasteChunk { id, content, lines });
        paste_placeholder(id)
    }

    /// Expand every `[[paste:N]]` placeholder in `text` back to its
    /// stashed content. Unknown or malformed ids stay literal — the
    /// user may have typed them by hand.
    pub fn expand_pastes(&self, text: &str) -> String {
        if self.pastes.is_empty() || !text.contains("[[paste:") {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(hit) = rest.find("[[paste:") {
            out.push_str(&rest[..hit]);
            let after = &rest[hit + "[[paste:".len()..];
            let Some(end) = after.find("]]") else {
                out.push_str(&rest[hit..]);
                rest = "";
                break;
            };
            let id_part = &after[..end];
            match id_part.parse::<u32>() {
                Ok(id) => match self.pastes.iter().find(|p| p.id == id) {
                    Some(p) => out.push_str(&p.content),
                    None => {
                        // Placeholder without a stash — leave it literal.
                        out.push_str("[[paste:");
                        out.push_str(id_part);
                        out.push_str("]]");
                    }
                },
                Err(_) => {
                    out.push_str("[[paste:");
                    out.push_str(id_part);
                    out.push_str("]]");
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    /// Erase the placeholder around/near `self.cursor` and drop the
    /// paired stash. Returns `true` if a placeholder was found and
    /// removed. Used by `Ctrl+X` in the composer.
    pub fn remove_paste_at_cursor(&mut self) -> bool {
        // Find any placeholder whose byte range covers or borders the
        // cursor. Prefer the placeholder whose range contains the
        // cursor; fall back to the nearest one on the same line.
        let input = self.input.clone();
        for p in self.pastes.clone() {
            let tok = paste_placeholder(p.id);
            let Some(pos) = input.find(&tok) else {
                continue;
            };
            let end = pos + tok.len();
            if self.cursor >= pos && self.cursor <= end {
                self.input.drain(pos..end);
                if self.cursor > end {
                    self.cursor -= tok.len();
                } else {
                    self.cursor = pos;
                }
                self.pastes.retain(|q| q.id != p.id);
                return true;
            }
        }
        false
    }

    /// Trigger a red-flash beat in the status bar for `duration`, with
    /// `label` as the one-line message. Overrides any previous flash
    /// still in flight.
    pub fn error_flash(&mut self, label: impl Into<String>, duration: std::time::Duration) {
        self.error_flash_until = Some(Instant::now() + duration);
        self.error_flash_label = Some(label.into());
    }

    /// True while an error flash is still visible. Cheap enough to call
    /// on every render tick.
    pub fn error_flash_active(&self) -> bool {
        self.error_flash_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    /// Byte offset of the previous `User` entry's start in a
    /// paragraph-height iteration — used by Ctrl+↑ to jump between
    /// turns. Returns entry indices (not row offsets); the render
    /// layer maps indices → rows via `entry_row_starts`.
    pub fn prev_user_entry_idx(&self, before_row: u16) -> Option<usize> {
        // Fall back is "the current view's top" — we don't know that
        // here, so callers pass in an approximate scroll position and
        // we return the nearest user entry above it.
        let _ = before_row; // reserved for future row-aware refinement
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| matches!(e, LogEntry::User(_)))
            .map(|(i, _)| i)
    }

    /// Index of the user entry immediately preceding `idx`. Used by
    /// `Ctrl+↑` to walk backward from the currently-visible one.
    pub fn user_entry_before(&self, idx: usize) -> Option<usize> {
        if idx == 0 {
            return None;
        }
        self.entries[..idx]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| matches!(e, LogEntry::User(_)))
            .map(|(i, _)| i)
    }

    /// Index of the user entry immediately following `idx`.
    pub fn user_entry_after(&self, idx: usize) -> Option<usize> {
        let start = idx.saturating_add(1);
        if start >= self.entries.len() {
            return None;
        }
        self.entries[start..]
            .iter()
            .enumerate()
            .find(|(_, e)| matches!(e, LogEntry::User(_)))
            .map(|(off, _)| start + off)
    }

    /// The entry index the caret is currently anchored to when the user
    /// jumps between turns — persisted across Ctrl+↑ presses so a
    /// sequence walks steadily backward through history rather than
    /// hunting from the tail each time.
    pub fn turn_nav_anchor(&self) -> Option<usize> {
        self.turn_nav_idx
    }

    pub fn set_turn_nav_anchor(&mut self, idx: Option<usize>) {
        self.turn_nav_idx = idx;
    }

    // ---- transcript ----

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn clear_entries(&mut self) {
        self.entries.clear();
        self.dropped_entries = 0;
        self.scroll = 0;
        self.follow_tail = true;
    }

    /// Drop entries from the front until we're back under [`MAX_ENTRIES`],
    /// bumping `dropped_entries` so the render layer can show the
    /// truncation notice. Called from every push helper.
    fn enforce_cap(&mut self) {
        while self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
            self.dropped_entries = self.dropped_entries.saturating_add(1);
        }
    }

    pub fn push_user(&mut self, s: String) {
        self.entries.push(LogEntry::User(s));
        self.enforce_cap();
        // A fresh user turn resets the turn-nav anchor — Ctrl+↑ from
        // here should walk *this* turn's history, not still be pointed
        // at whatever the previous session was scrolled to.
        self.turn_nav_idx = None;
    }

    /// Push a turn-end marker with the wall-clock reply time plus a
    /// receipt of what the turn did (files written, diff sizes, tools
    /// run, this turn's spend). Called from the event loop on
    /// `HarnessEvent::Done` so the transcript grows a `✳ Baked for
    /// 12.4s · 3 files (+18 −4) · 7 tools` line under the last
    /// assistant reply — the visual full stop between turns.
    pub fn push_turn_end(&mut self, elapsed_ms: u32, tokens: u64, cost: Option<f64>) {
        let _ = tokens;
        let turn_start = self
            .entries
            .iter()
            .rposition(|e| matches!(e, LogEntry::User(_)));
        let (mut files, mut adds, mut dels, mut tools) = (0u16, 0u32, 0u32, 0u16);
        for e in self.entries.iter().skip(turn_start.unwrap_or(0)) {
            if let LogEntry::ToolCall { name, preview, .. } = e {
                // Task bookkeeping is suppressed from the stream — it
                // doesn't count as work the user saw happening.
                if crate::tui::components::is_task_tool(name) {
                    continue;
                }
                tools += 1;
                if crate::tui::components::tool_call::is_undoable_tool(name) {
                    files += 1;
                }
                if let Some(p) = preview {
                    let (a, d) = crate::tui::components::tool_result::diff_stats(p);
                    adds += a as u32;
                    dels += d as u32;
                }
            }
        }
        self.entries.push(LogEntry::TurnEnd {
            elapsed_ms,
            files,
            adds,
            dels,
            tools,
            cost,
        });
        self.enforce_cap();
    }

    /// Push the startup banner. One structured entry (not a pile of
    /// info lines) so the component can render it with brand colors.
    pub fn push_welcome(&mut self, model: String, mode: Mode, cwd: String, tip: &'static str) {
        self.entries.push(LogEntry::Welcome {
            model,
            mode,
            cwd,
            tip,
        });
        self.enforce_cap();
    }

    pub fn push_info(&mut self, s: impl Into<String>) {
        self.entries.push(LogEntry::Info(s.into()));
        self.enforce_cap();
    }

    pub fn push_warning(&mut self, s: String) {
        self.entries.push(LogEntry::Warning(s));
        self.enforce_cap();
    }

    /// Append a streamed assistant token onto the tail assistant entry —
    /// or start a new one if the previous entry was something else.
    pub fn append_token(&mut self, t: &str) {
        if let Some(LogEntry::Assistant(buf)) = self.entries.last_mut() {
            buf.push_str(t);
        } else {
            self.entries.push(LogEntry::Assistant(t.to_owned()));
            self.enforce_cap();
        }
    }

    pub fn push_tool_call(&mut self, call: &ToolCall) {
        self.entries.push(LogEntry::ToolCall {
            name: call.function.name.clone(),
            args: call.function.arguments.clone(),
            preview: None,
            call_id: call.id.to_string(),
            started_at: Instant::now(),
        });
        self.enforce_cap();
    }

    /// Attach a diff preview to the most-recent matching `ToolCall`
    /// entry — invoked when the approval receiver computes one, so
    /// the tool group can render the diff once the call completes.
    /// Silent no-op if no matching entry is found (rare — the entry
    /// may have scrolled off under `MAX_ENTRIES`).
    pub fn attach_preview(&mut self, id: &str, p: DiffPreview) {
        for e in self.entries.iter_mut().rev() {
            if let LogEntry::ToolCall {
                call_id, preview, ..
            } = e
            {
                if call_id == id {
                    *preview = Some(p);
                    return;
                }
            }
        }
    }

    pub fn push_tool_result(&mut self, r: &ToolResult) {
        self.entries.push(LogEntry::ToolResult {
            ok: !r.is_error,
            snippet: first_line(&r.content, 200),
            full: truncate_bytes(&r.content, TOOL_RESULT_MAX_BYTES),
            expanded: false,
            collapsed_override: None,
        });
        self.enforce_cap();
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
            full: truncate_bytes(content, TOOL_RESULT_MAX_BYTES),
            expanded: false,
            collapsed_override: None,
        });
        self.enforce_cap();
    }

    /// Push a raw tool call from history (name + args string).
    /// Replay path — the call already completed on a previous run so
    /// `started_at` is only cosmetic (the ticker never renders for
    /// replayed calls because they always have a matching result
    /// pushed right after).
    pub fn push_tool_call_raw(&mut self, name: String, args: String) {
        self.entries.push(LogEntry::ToolCall {
            name,
            args,
            preview: None,
            call_id: String::new(),
            started_at: Instant::now(),
        });
        self.enforce_cap();
    }

    /// Push an already-complete assistant message from history.
    pub fn push_assistant(&mut self, s: String) {
        self.entries.push(LogEntry::Assistant(s));
        self.enforce_cap();
    }

    /// Toggle the `expanded` flag on the most recent tool-result entry.
    /// Returns `true` when it found one to toggle so the key handler can
    /// flash a hint on no-op ("no tool result yet").
    pub fn toggle_last_tool_result(&mut self) -> bool {
        for e in self.entries.iter_mut().rev() {
            if let LogEntry::ToolResult {
                expanded,
                collapsed_override,
                ..
            } = e
            {
                *expanded = !*expanded;
                // Full output is meaningless under auto-collapse — pin
                // the group open whenever we expand it.
                if *expanded {
                    *collapsed_override = Some(false);
                }
                return true;
            }
        }
        false
    }

    /// Toggle auto-collapse for the `n`-th most recent tool group
    /// (1-based; `1` = the newest). Task-bookkeeping calls don't count
    /// — they render nowhere. Returns `false` when there are fewer
    /// than `n` toggleable groups or the newest such call is still
    /// in-flight (no result to collapse).
    pub fn toggle_nth_tool_group_from_end(&mut self, n: usize) -> bool {
        if n == 0 {
            return false;
        }
        let current_turn_start = self
            .entries
            .iter()
            .rposition(|e| matches!(e, LogEntry::User(s) if !s.trim().is_empty()));
        let mut seen = 0usize;
        for i in (0..self.entries.len()).rev() {
            let name = match &self.entries[i] {
                LogEntry::ToolCall { name, .. } => name.clone(),
                _ => continue,
            };
            if crate::tui::components::is_task_tool(&name) {
                continue;
            }
            seen += 1;
            if seen < n {
                continue;
            }
            let result_idx = match self.entries.get(i + 1) {
                Some(LogEntry::ToolResult { .. }) => i + 1,
                _ => return false, // in-flight call — nothing to toggle
            };
            let auto_collapsed = current_turn_start.is_some_and(|start| i < start);
            if let LogEntry::ToolResult {
                collapsed_override,
                ..
            } = &mut self.entries[result_idx]
            {
                *collapsed_override = Some(!collapsed_override.unwrap_or(auto_collapsed));
            }
            return true;
        }
        false
    }

    /// Drop the visible transcript back to the start of the last user
    /// turn and return that turn's prompt for editing. Pairs with
    /// `Session::rewind_last_turn` — call both (state first) for a
    /// consistent edit-and-retry.
    pub fn rewind_last_turn(&mut self) -> Option<String> {
        let idx = self
            .entries
            .iter()
            .rposition(|e| matches!(e, LogEntry::User(s) if !s.trim().is_empty()))?;
        let text = match &self.entries[idx] {
            LogEntry::User(s) => s.clone(),
            _ => unreachable!("rposition matched User"),
        };
        self.entries.truncate(idx);
        self.turn_nav_idx = None;
        self.turn_scroll_target = None;
        self.follow_tail = true;
        Some(text)
    }

    /// Name + args of the tool call currently in flight — a trailing
    /// `ToolCall` entry without a matching `ToolResult` after it.
    /// Returned by [`Self::in_flight_tool`] so the Ctrl+C handler can
    /// name the tool it just interrupted.
    pub fn in_flight_tool(&self) -> Option<(String, String)> {
        for e in self.entries.iter().rev() {
            match e {
                LogEntry::ToolResult { .. } => return None,
                LogEntry::ToolCall { name, args, .. } => {
                    return Some((name.clone(), args.clone()));
                }
                // Skip info/warning/token/banner entries — they can
                // appear between a ToolCall and its ToolResult
                // (streaming progress lines) without changing what's
                // in flight.
                _ => continue,
            }
        }
        None
    }

    /// Text of the last assistant reply, if any — the target of `Ctrl+Y`.
    pub fn last_assistant_text(&self) -> Option<&str> {
        for e in self.entries.iter().rev() {
            if let LogEntry::Assistant(s) = e {
                return Some(s);
            }
        }
        None
    }

    /// Flatten the whole visible transcript into a plain-text
    /// conversation dump — target of `Ctrl+Shift+C`. Tool call/results
    /// are annotated in-line so a pasted transcript still reads as a
    /// coherent session log outside the TUI.
    pub fn transcript_plaintext(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            match e {
                LogEntry::User(s) => {
                    out.push_str("> ");
                    out.push_str(s);
                    out.push_str("\n\n");
                }
                LogEntry::TurnEnd { elapsed_ms, .. } => {
                    out.push_str(&format!(
                        "[{}]\n\n",
                        crate::tui::components::status::turn_end_label(*elapsed_ms)
                    ));
                }
                LogEntry::Assistant(s) => {
                    out.push_str(s);
                    if !s.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                LogEntry::ToolCall { name, args, .. } => {
                    out.push_str(&format!("[tool] {name}({args})\n"));
                }
                LogEntry::ToolResult {
                    ok, snippet, full, ..
                } => {
                    let mark = if *ok { "ok" } else { "err" };
                    let body = if full.is_empty() { snippet } else { full };
                    out.push_str(&format!("[{mark}] {body}\n\n"));
                }
                LogEntry::Warning(s) => {
                    out.push_str(&format!("[warn] {s}\n\n"));
                }
                LogEntry::Info(s) => {
                    out.push_str(&format!("[info] {s}\n"));
                }
                LogEntry::Welcome { model, cwd, tip, .. } => {
                    out.push_str(&format!("mira · {model} · {cwd}\n> {tip}\n\n"));
                }
            }
        }
        out
    }

    // ---- live tool tail ----

    /// Record the newest output line of the in-flight tool. Blank
    /// lines are skipped so a silent stretch keeps the last real one.
    pub fn set_tool_tail(&mut self, call_id: &str, line: &str) {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            return;
        }
        let mut short: String = trimmed.chars().take(160).collect();
        if short.len() != trimmed.len() {
            short.push('…');
        }
        self.tool_tail = Some((call_id.to_owned(), short));
    }

    pub fn clear_tool_tail(&mut self) {
        self.tool_tail = None;
    }

    // ---- message queue ----

    /// Park a composed message for sending when the current turn
    /// finishes. Returns the queue depth for the flash message.
    pub fn queue_message(&mut self, text: String) -> usize {
        self.queued.push(text);
        self.queued.len()
    }

    /// Pop the newest queued message back into the composer (↑ while
    /// streaming). Returns `false` when the queue is empty.
    pub fn pop_queued_to_input(&mut self) -> bool {
        match self.queued.pop() {
            Some(text) => {
                self.input_replace(&text);
                true
            }
            None => false,
        }
    }

    /// Take the oldest queued message — the event loop calls this on
    /// `HarnessEvent::Done` to auto-send.
    pub fn take_next_queued(&mut self) -> Option<String> {
        if self.queued.is_empty() {
            None
        } else {
            Some(self.queued.remove(0))
        }
    }

    // ---- task list ----

    /// Hydrate the task list from a `task_*` tool result's structured
    /// `data` payload. Shapes seen on the wire:
    /// - `{ "task": <TaskItem> }`  (create / get / update)
    /// - `{ "tasks": [<TaskItem>…] }` (list)
    ///
    /// Unknown shapes are ignored silently — the panel just stays as-is.
    /// `deleted` tasks are dropped from the view.
    pub fn apply_task_payload(&mut self, r: &ToolResult) {
        let Some(data) = r.data.as_ref() else {
            return;
        };
        if let Some(task) = data.get("task") {
            let Ok(item) = serde_json::from_value::<TaskItem>(task.clone()) else {
                return;
            };
            self.upsert_task(item);
        } else if let Some(tasks) = data.get("tasks") {
            let Ok(items) = serde_json::from_value::<Vec<TaskItem>>(tasks.clone()) else {
                return;
            };
            for item in items {
                self.upsert_task(item);
            }
        }
    }

    fn upsert_task(&mut self, item: TaskItem) {
        if item.status == TaskStatus::Deleted {
            self.tasks.retain(|t| t.id != item.id);
            return;
        }
        match self.tasks.iter_mut().find(|t| t.id == item.id) {
            Some(slot) => *slot = item,
            None => self.tasks.push(item),
        }
    }

    // ---- search ----

    pub fn search_open(&mut self) {
        if self.search.is_none() {
            self.search = Some(SearchState::default());
        }
    }

    pub fn search_close(&mut self) {
        self.search = None;
    }

    pub fn search_push(&mut self, c: char) {
        if let Some(s) = self.search.as_mut() {
            s.query.push(c);
        }
        self.recompute_hits();
    }

    pub fn search_backspace(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.query.pop();
        }
        self.recompute_hits();
    }

    pub fn search_next(&mut self) {
        if let Some(s) = self.search.as_mut() {
            if !s.hits.is_empty() {
                s.cursor = (s.cursor + 1) % s.hits.len();
            }
        }
    }

    pub fn search_prev(&mut self) {
        if let Some(s) = self.search.as_mut() {
            if !s.hits.is_empty() {
                s.cursor = if s.cursor == 0 {
                    s.hits.len() - 1
                } else {
                    s.cursor - 1
                };
            }
        }
    }

    /// The (entry_index, byte_offset) of the currently focused search
    /// hit — used by the transcript renderer to scroll to it.
    pub fn active_hit(&self) -> Option<(usize, usize)> {
        let s = self.search.as_ref()?;
        s.hits.get(s.cursor).copied()
    }

    fn recompute_hits(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.hits.clear();
        s.cursor = 0;
        if s.query.is_empty() {
            return;
        }
        let needle = s.query.to_ascii_lowercase();
        for (idx, e) in self.entries.iter().enumerate() {
            let hay = entry_text(e).to_ascii_lowercase();
            let mut start = 0usize;
            while let Some(off) = hay[start..].find(&needle) {
                s.hits.push((idx, start + off));
                start += off + needle.len();
            }
        }
    }

    /// Session-scoped allow-rule string for a specific tool call, or
    /// `None` when we can't derive one (unknown tool, missing arg).
    ///
    /// Kept in state.rs (rather than approver.rs) because it's a pure
    /// function of the call shape and is used by both the approval
    /// modal and the `/permissions add` slash flow.
    pub fn rule_for_call(call: &ToolCall) -> Option<String> {
        let name = call.function.name.as_str();
        let args: serde_json::Value =
            serde_json::from_str(&call.function.arguments).unwrap_or_default();
        match name {
            "shell" | "bash" => {
                let cmd = args.get("cmd").and_then(|v| v.as_str())?;
                Some(format!("Bash({cmd})"))
            }
            "edit_file" | "write_file" | "apply_patch" | "create_file" => {
                let path = args
                    .get("path")
                    .or_else(|| args.get("target"))
                    .and_then(|v| v.as_str())?;
                Some(format!("Edit({path})"))
            }
            "read_file" | "view_file" => {
                let path = args.get("path").and_then(|v| v.as_str())?;
                Some(format!("Read({path})"))
            }
            _ => None,
        }
    }

    // ---- input ----

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn input_push(&mut self, c: char) {
        self.leave_history_browse();
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn input_push_str(&mut self, s: &str) {
        self.leave_history_browse();
        self.input.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    pub fn input_newline(&mut self) {
        self.input_push('\n');
    }

    pub fn input_backspace(&mut self) {
        self.leave_history_browse();
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.input, self.cursor);
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub fn input_delete_forward(&mut self) {
        self.leave_history_browse();
        if self.cursor >= self.input.len() {
            return;
        }
        let next = next_char_boundary(&self.input, self.cursor);
        self.input.drain(self.cursor..next);
    }

    pub fn input_clear(&mut self) -> String {
        self.history_cursor = None;
        self.history_stash.clear();
        self.cursor = 0;
        std::mem::take(&mut self.input)
    }

    /// Overwrite the composer buffer wholesale and place the cursor at
    /// the end. Used by `start_stream` to return a rejected message to
    /// the composer when the budget guardrail blocks a send — the user
    /// shouldn't have to retype what they wrote.
    pub fn input_replace(&mut self, s: &str) {
        self.input.clear();
        self.input.push_str(s);
        self.cursor = self.input.len();
        self.history_cursor = None;
        self.history_stash.clear();
    }

    pub fn is_input_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    // ---- cursor movement ----

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = prev_char_boundary(&self.input, self.cursor);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = next_char_boundary(&self.input, self.cursor);
        }
    }

    pub fn move_word_left(&mut self) {
        self.cursor = word_boundary_left(&self.input, self.cursor);
    }

    pub fn move_word_right(&mut self) {
        self.cursor = word_boundary_right(&self.input, self.cursor);
    }

    /// Move to the start of the current visual line (segment between
    /// '\n' boundaries containing the cursor).
    pub fn move_line_start(&mut self) {
        self.cursor = line_start(&self.input, self.cursor);
    }

    pub fn move_line_end(&mut self) {
        self.cursor = line_end(&self.input, self.cursor);
    }

    pub fn kill_word_left(&mut self) {
        self.leave_history_browse();
        let mut start = word_boundary_left(&self.input, self.cursor);
        // Also swallow one run of horizontal whitespace before the word so
        // consecutive Ctrl+W doesn't leave double-space litter. Newlines
        // stay — killing across a line break usually isn't wanted.
        let bytes = self.input.as_bytes();
        while start > 0 && (bytes[start - 1] == b' ' || bytes[start - 1] == b'\t') {
            start -= 1;
        }
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn kill_word_right(&mut self) {
        self.leave_history_browse();
        let end = word_boundary_right(&self.input, self.cursor);
        self.input.drain(self.cursor..end);
    }

    pub fn kill_to_line_start(&mut self) {
        self.leave_history_browse();
        let start = line_start(&self.input, self.cursor);
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn kill_to_line_end(&mut self) {
        self.leave_history_browse();
        let end = line_end(&self.input, self.cursor);
        self.input.drain(self.cursor..end);
    }

    // ---- history recall ----

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
        self.cursor = self.input.len();
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
        self.cursor = self.input.len();
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

// ---- text helpers (public within crate for testing) ----

fn prev_char_boundary(s: &str, byte: usize) -> usize {
    let mut i = byte.saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, byte: usize) -> usize {
    let mut i = byte + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

/// Walk left over whitespace, then over the word body (identifier-ish
/// runs — anything that isn't whitespace or an ASCII punctuation break).
/// Stops at start of buffer.
fn word_boundary_left(s: &str, byte: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = byte;
    // Skip trailing whitespace between cursor and word.
    while i > 0 && is_wordbreak(bytes[i - 1]) {
        i -= 1;
    }
    // Skip the word body.
    while i > 0 && !is_wordbreak(bytes[i - 1]) {
        i -= 1;
    }
    // Realign to a char boundary — safe because we only walked ASCII
    // wordbreak bytes (each is a full char), but wordy content may span
    // multi-byte chars we walked into byte-by-byte.
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn word_boundary_right(s: &str, byte: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = byte;
    let len = bytes.len();
    while i < len && is_wordbreak(bytes[i]) {
        i += 1;
    }
    while i < len && !is_wordbreak(bytes[i]) {
        i += 1;
    }
    while i < len && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(len)
}

fn is_wordbreak(b: u8) -> bool {
    // Match the emacs/bash convention: whitespace and ASCII punctuation
    // are breaks; letters/digits/underscore are word.
    b.is_ascii_whitespace() || (b.is_ascii_punctuation() && b != b'_')
}

fn line_start(s: &str, byte: usize) -> usize {
    s[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end(s: &str, byte: usize) -> usize {
    match s[byte..].find('\n') {
        Some(off) => byte + off,
        None => s.len(),
    }
}

/// Flatten one entry to plain text for search matching. Preserves
/// content — assistant paragraphs, user messages, tool args, tool
/// results.
fn entry_text(e: &LogEntry) -> String {
    match e {
        LogEntry::User(s) | LogEntry::Assistant(s) | LogEntry::Warning(s) | LogEntry::Info(s) => {
            s.clone()
        }
        LogEntry::TurnEnd { elapsed_ms, .. } => {
            format!("baked for {}ms", elapsed_ms)
        }
        LogEntry::Welcome { model, cwd, tip, .. } => {
            format!("mira {model} {cwd} {tip}")
        }
        LogEntry::ToolCall { name, args, .. } => format!("{name} {args}"),
        LogEntry::ToolResult { snippet, full, .. } => {
            if full.is_empty() {
                snippet.clone()
            } else {
                full.clone()
            }
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

/// Truncate at the last char boundary ≤ `max`, preserving valid UTF-8.
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_owned();
    out.push_str("\n… (truncated)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_left_right_ascii() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        for c in "hello".chars() {
            st.input_push(c);
        }
        assert_eq!(st.cursor(), 5);
        st.move_left();
        st.move_left();
        assert_eq!(st.cursor(), 3);
        st.input_push('X');
        assert_eq!(st.input(), "helXlo");
        assert_eq!(st.cursor(), 4);
    }

    #[test]
    fn cursor_walks_multibyte() {
        // "héllo" — 'é' is 2 bytes (U+00E9).
        let mut st = TuiState::new("m".into(), Mode::Manual);
        for c in "héllo".chars() {
            st.input_push(c);
        }
        // 5 chars → 6 bytes total.
        assert_eq!(st.input().len(), 6);
        assert_eq!(st.cursor(), 6);
        st.move_line_start();
        assert_eq!(st.cursor(), 0);
        st.move_right(); // past 'h'
        assert_eq!(st.cursor(), 1);
        st.move_right(); // past 'é' (2 bytes)
        assert_eq!(st.cursor(), 3);
    }

    #[test]
    fn word_jump_and_kill() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.input_push_str("hello world foo");
        st.move_line_start();
        st.move_word_right();
        assert_eq!(st.cursor(), 5);
        st.move_word_right();
        assert_eq!(st.cursor(), 11);
        st.kill_word_left();
        assert_eq!(st.input(), "hello foo");
    }

    #[test]
    fn line_home_end_multiline() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.input_push_str("first\nsecond line");
        st.move_line_start();
        assert_eq!(st.cursor(), 6);
        st.move_line_end();
        assert_eq!(st.cursor(), st.input().len());
    }

    #[test]
    fn rule_for_call_covers_common_tools() {
        use mira_core::message::{ToolCallFunction, ToolCallKind};
        use mira_core::ToolCall;
        let mk = |name: &str, args: &str| ToolCall {
            id: "1".into(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.into(),
            },
        };
        assert_eq!(
            TuiState::rule_for_call(&mk("shell", r#"{"cmd":"cargo test"}"#)).as_deref(),
            Some("Bash(cargo test)")
        );
        assert_eq!(
            TuiState::rule_for_call(&mk("edit_file", r#"{"path":"src/main.rs"}"#)).as_deref(),
            Some("Edit(src/main.rs)")
        );
        assert_eq!(
            TuiState::rule_for_call(&mk("read_file", r#"{"path":"README.md"}"#)).as_deref(),
            Some("Read(README.md)")
        );
        assert_eq!(TuiState::rule_for_call(&mk("unknown_tool", r#"{}"#)), None);
    }

    #[test]
    fn search_finds_and_cycles() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.entries.push(LogEntry::User("hello world".into()));
        st.entries.push(LogEntry::Assistant("goodbye world".into()));
        st.entries.push(LogEntry::Info("world peace".into()));
        st.search_open();
        for c in "world".chars() {
            st.search_push(c);
        }
        let s = st.search.as_ref().unwrap();
        assert_eq!(s.hits.len(), 3);
        assert_eq!(s.cursor, 0);
        st.search_next();
        assert_eq!(st.search.as_ref().unwrap().cursor, 1);
    }

    #[test]
    fn paste_stash_and_expand_round_trip() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.input_push_str("before ");
        let token = st.stash_paste("hello\nworld\nlots\nof\nlines".into());
        st.input_push_str(&token);
        st.input_push_str(" after");
        assert!(st.input().contains("[[paste:1]]"));
        let expanded = st.expand_pastes(st.input());
        assert_eq!(expanded, "before hello\nworld\nlots\nof\nlines after");
    }

    #[test]
    fn remove_paste_at_cursor_drops_stash() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        let token = st.stash_paste("x\ny\nz".into());
        st.input_push_str("hi ");
        st.input_push_str(&token);
        // Cursor sits at end of token; removal drops both placeholder
        // and stashed content, and the composer keeps the surrounding
        // text intact.
        assert!(st.remove_paste_at_cursor());
        assert_eq!(st.input(), "hi ");
        assert!(st.pastes.is_empty());
    }

    #[test]
    fn user_ordinal_walks_backward_from_anchor() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.push_user("first".into());
        st.push_assistant("reply".into());
        st.push_user("second".into());
        st.push_assistant("reply2".into());
        st.push_user("third".into());
        // Anchor at last user; step should land on "second".
        let last = st.prev_user_entry_idx(0).unwrap();
        let before = st.user_entry_before(last).unwrap();
        let earliest = st.user_entry_before(before).unwrap();
        assert!(matches!(&st.entries()[earliest], LogEntry::User(s) if s == "first"));
        assert!(st.user_entry_before(earliest).is_none());
    }

    #[test]
    fn push_turn_end_computes_receipt() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.push_user("go".into());
        st.push_tool_call_raw("edit_file".into(), r#"{"path":"a.rs"}"#.into());
        st.push_tool_result_replay("edited");
        st.push_tool_call_raw("shell".into(), r#"{"cmd":"ls"}"#.into());
        st.push_tool_result_replay("exit=0");
        st.push_assistant("done".into());
        st.push_turn_end(4200, 900, Some(0.021));
        match st.entries().last().unwrap() {
            LogEntry::TurnEnd {
                elapsed_ms,
                files,
                adds,
                dels,
                tools,
                cost,
            } => {
                assert_eq!(*elapsed_ms, 4200);
                assert_eq!(*files, 1);
                assert_eq!(*tools, 2);
                assert_eq!(*cost, Some(0.021));
                assert_eq!((*adds, *dels), (0, 0)); // replay path has no previews
            }
            _ => panic!("expected turn-end marker"),
        }
    }

    #[test]
    fn tool_tail_keeps_last_real_line() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.set_tool_tail("c1", "compiling foo
");
        st.set_tool_tail("c1", "   
");
        st.set_tool_tail("c1", "3 tests passed");
        assert_eq!(
            st.tool_tail
                .as_ref()
                .map(|(a, b)| (a.as_str(), b.as_str())),
            Some(("c1", "3 tests passed"))
        );
        st.clear_tool_tail();
        assert!(st.tool_tail.is_none());
    }

    #[test]
    fn tool_result_expand_toggles_last() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.entries.push(LogEntry::ToolResult {
            ok: true,
            snippet: "one".into(),
            full: "one\ntwo\nthree".into(),
            expanded: false,
            collapsed_override: None,
        });
        assert!(st.toggle_last_tool_result());
        let LogEntry::ToolResult { expanded, .. } = st.entries.last().unwrap() else {
            unreachable!()
        };
        assert!(*expanded);
    }

    #[test]
    fn toggle_nth_tool_group_flips_collapse_override() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.push_user("turn one".into());
        st.push_tool_call_raw("read_file".into(), r#"{"path":"a.rs"}"#.into());
        st.push_tool_result_replay("ok");
        st.push_user("turn two".into());
        st.push_tool_call_raw("read_file".into(), r#"{"path":"b.rs"}"#.into());
        st.push_tool_result_replay("ok");

        // Group 2 (older turn) is auto-collapsed; toggling pins it open.
        assert!(st.toggle_nth_tool_group_from_end(2));
        let idx = st
            .entries()
            .iter()
            .position(|e| matches!(e, LogEntry::ToolCall { name, .. } if name == "read_file") )
            .and_then(|i| match st.entries().get(i + 1) {
                Some(LogEntry::ToolResult { collapsed_override, .. }) => Some(*collapsed_override),
                _ => None,
            });
        assert_eq!(idx, Some(Some(false)));

        // Toggling again re-collapses.
        assert!(st.toggle_nth_tool_group_from_end(2));
        let idx = st
            .entries()
            .iter()
            .position(|e| matches!(e, LogEntry::ToolCall { name, .. } if name == "read_file"))
            .and_then(|i| match st.entries().get(i + 1) {
                Some(LogEntry::ToolResult { collapsed_override, .. }) => Some(*collapsed_override),
                _ => None,
            });
        assert_eq!(idx, Some(Some(true)));

        // More groups than exist → falls back to typing the digit.
        assert!(!st.toggle_nth_tool_group_from_end(5));
    }

    #[test]
    fn rewind_last_turn_truncates_transcript() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.push_user("first prompt".into());
        st.push_assistant("reply".into());
        st.push_user("second prompt".into());
        st.push_assistant("reply2".into());
        st.push_turn_end(1000, 0, None);

        let text = st.rewind_last_turn().unwrap();
        assert_eq!(text, "second prompt");
        // Everything from the last user prompt onward is gone.
        assert_eq!(st.entries().len(), 2); // first prompt + reply
        assert!(matches!(st.entries().last(), Some(LogEntry::Assistant(_))));

        // Composer gets the prompt back for editing.
        st.input_replace(&text);
        assert_eq!(st.input(), "second prompt");

        // And repeated rewinds walk further back.
        assert_eq!(st.rewind_last_turn().unwrap(), "first prompt");
        assert!(st.entries().is_empty());
        assert!(st.rewind_last_turn().is_none());
    }

    #[test]
    fn resize_invalidates_layout_and_reconciles_scroll() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.handle_resize(80, 24);
        assert_eq!(st.viewport_width, 80);
        assert_eq!(st.viewport_height, 24);
        assert!(st.layout_cache.is_none());

        // Simulate the draw pass caching a layout measured at width 80.
        st.layout_cache = Some(crate::tui::render::layout::TranscriptLayout {
            height: 24,
            total_rows: 200,
            entry_row_starts: vec![],
        });

        // Height-only resize: cache survives (wrapping is width-bound);
        // reconcile clamps against the cached region height (one frame
        // stale — the next draw recomputes with the new region).
        st.scroll = 500;
        st.follow_tail = false;
        st.handle_resize(80, 10);
        assert!(st.layout_cache.is_some());
        assert_eq!(st.scroll, 176); // 200 - 24 (cached region height)

        // Width change: cache invalidated; reconcile defers to the next
        // draw instead of using stale metrics.
        st.handle_resize(120, 10);
        assert!(st.layout_cache.is_none());
        assert_eq!(st.scroll, 176); // untouched — clamped on next frame
    }

    #[test]
    fn follow_tail_snaps_on_resize_reconcile() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.handle_resize(80, 24);
        st.layout_cache = Some(crate::tui::render::layout::TranscriptLayout {
            height: 24,
            total_rows: 100,
            entry_row_starts: vec![],
        });
        st.follow_tail = true;
        st.scroll = 0;
        st.handle_resize(80, 30);
        assert_eq!(st.scroll, 76); // tail from cached region height (100 - 24)
    }

    #[test]
    fn cached_tail_survives_until_width_change() {
        let mut st = TuiState::new("m".into(), Mode::Manual);
        st.handle_resize(80, 24);
        st.layout_cache = Some(crate::tui::render::layout::TranscriptLayout {
            height: 20,
            total_rows: 100,
            entry_row_starts: vec![],
        });
        assert_eq!(st.cached_tail(), Some(80));
        // A width-changing resize drops the cache — handlers then
        // degrade gracefully until the next draw rebuilds it.
        st.handle_resize(120, 24);
        assert_eq!(st.cached_tail(), None);
    }
}
