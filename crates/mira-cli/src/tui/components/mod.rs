//! Presentation model for the transcript.
//!
//! The layering contract:
//!
//! - **State owns what is happening** — `TuiState` accumulates `LogEntry`s
//!   from harness events plus a few ephemeral flags.
//! - **Components own how one thing looks** — each `TranscriptBlock` variant
//!   has a renderer in this module tree that knows nothing about scrolling,
//!   the viewport, or where the block lands on screen.
//! - **Layout owns where things go** — `render::layout` measures the rendered
//!   rows, keeps the derived row-start table, and resolves scroll.
//!
//! The pipeline each frame is:
//!
//! ```text
//! LogEntry slice ──▶ build_blocks ──▶ Vec<TranscriptBlock> ──▶ render_block ──▶ Vec<Line>
//! ```
//!
//! The renderer never needs to understand the agent lifecycle: adding a new
//! UI state (sub-agents, model routing, …) means adding one variant here and
//! one renderer in a sibling module.

pub mod approval;
pub mod message;
pub mod prompt;
pub mod status;
pub mod tasks;
pub mod tool_call;
pub mod tool_result;

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use crate::tui::state::LogEntry;
use crate::tui::theme;

use status::StatusView;

// ---- theme shims ----
//
// Backed by `crate::tui::theme` — an `RwLock`-guarded global that
// `/theme <name>`, `/theme reload`, and `/theme save` mutate at
// runtime. Every callsite goes through one of the shim fns below;
// each is a single read-lock acquisition, cheap enough for the 100ms
// render tick.
//
// Truecolor-only — 16-color terminals fall back to their closest
// match automatically via ratatui/crossterm.

/// Names stay uppercase to signal "palette constant" at every callsite
/// even though Rust convention normally wants snake_case for functions.
#[allow(non_snake_case)]
#[inline]
pub(crate) fn SALMON() -> Color {
    theme::current().salmon
}
#[allow(non_snake_case)]
#[inline]
pub(crate) fn CREAM() -> Color {
    theme::current().cream
}
#[allow(non_snake_case)]
#[inline]
pub(crate) fn MUTED() -> Color {
    theme::current().muted
}
#[allow(non_snake_case)]
#[inline]
pub(crate) fn DIM() -> Color {
    theme::current().dim
}
#[allow(non_snake_case)]
#[inline]
pub(crate) fn HAIRLINE() -> Color {
    theme::current().hairline
}

/// Mira's brand mark — the script-M is the closest Unicode analogue
/// of the flowing wave/M in the vector logo. Rendered wherever the
/// UI needs an app-identity glyph (header, welcome, streaming line).
pub(crate) const LOGO: &str = "ℳ";

/// One block of the transcript as the presentation layer sees it.
///
/// Blocks borrow from `TuiState::entries` so building the list each
/// frame is allocation-free. `Approval` and `Working` are ephemeral —
/// they never touch `entries` and carry `consumed == 0`.
pub enum TranscriptBlock<'a> {
    /// Head-of-transcript truncation notice ("… N earlier entries").
    Truncated(usize),
    User(&'a str),
    Assistant {
        text: &'a str,
        /// Entry is currently receiving streamed tokens — renders the
        /// soft `▍` cursor and suppresses plan-card conversion.
        streaming: bool,
        /// Session is in plan mode — assistant text gets the blue gutter.
        plan: bool,
    },
    TurnEnd(status::TurnEndView),
    /// Startup banner (first entry on a fresh session).
    Welcome(message::WelcomeView<'a>),
    Tool(tool_call::ToolView<'a>),
    ToolBatch(tool_call::BatchView),
    Warning(&'a str),
    Info(&'a str),
    /// In-flight approval prompt, rendered inline below the tool call
    /// that triggered it.
    Approval(approval::ApprovalView<'a>),
    /// The agent's live task list (checkbox panel).
    Tasks(tasks::TaskListView<'a>),
    /// Interactive `plan` tool card (ephemeral, keys stolen while up).
    PlanCard(&'a crate::tui::state::PendingPlan),
    /// Interactive `ask_user` tool card.
    AskCard(&'a crate::tui::state::PendingAsk),
    /// A tool group deliberately kept out of the transcript — task
    /// bookkeeping (`task_create` / `task_update` / …) whose feedback
    /// lives in the [`TranscriptBlock::Tasks`] panel instead. Renders
    /// zero lines but still consumes its entries so the entry→row
    /// table stays aligned for search and turn navigation.
    Suppressed,
    /// "✻ Wrangling… (12s · ↓ 1.2k tokens)" working indicator.
    Working(StatusView),
}

/// A block plus the slice of `LogEntry`s it was built from. `consumed`
/// drives the `entry_row_starts` table (search-hit and turn-nav
/// scrolling both index entries); ephemeral blocks consume nothing.
pub struct Block<'a> {
    pub kind: TranscriptBlock<'a>,
    pub first_entry: usize,
    pub consumed: usize,
}

/// Render-time context that isn't per-entry: whether the session is
/// mid-stream, in plan mode, which entry is the streaming tail or the
/// newest undoable write, and where the current turn begins (the last
/// user prompt) — tool groups from earlier turns auto-collapse to
/// their header so long sessions stay readable.
pub struct BuildCtx {
    pub streaming: bool,
    pub plan_mode: bool,
    pub streaming_tail_idx: Option<usize>,
    pub undoable_idx: Option<usize>,
    pub current_turn_start: Option<usize>,
    /// Live tail `(call_id, line)` of the in-flight tool.
    pub tool_tail: Option<(String, String)>,
}

/// Group the entry log into presentation blocks. Pure — borrows the
/// entries, allocates only small per-block vectors. Preserves the
/// transcript's visual contract:
///
/// - a `ToolCall` pairs with the `ToolResult` immediately after it;
/// - consecutive same-family call/result pairs collapse into one
///   batched block ("Reading 3 files") unless a call carries a diff
///   preview or its result is expanded — both would lose information
///   in the collapsed form;
/// - an orphan `ToolResult` (its call scrolled off under the entry
///   cap) still renders as a stand-alone block.
pub fn build_blocks<'a>(entries: &'a [LogEntry], ctx: &BuildCtx) -> Vec<Block<'a>> {
    let mut out: Vec<Block<'_>> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        match &entries[i] {
            LogEntry::ToolCall {
                name,
                args,
                preview,
                started_at,
                ..
            } if !is_task_tool(name) => {
                // Try a same-family batch first.
                let (family, _) = tool_call::summarize_tool(name, args);
                let mut batch_end = i;
                let mut summaries = Vec::new();
                while let Some(LogEntry::ToolCall {
                    name,
                    args,
                    preview,
                    ..
                }) = entries.get(batch_end)
                {
                    let (f, s) = tool_call::summarize_tool(name, args);
                    if *f != family || preview.is_some() {
                        break;
                    }
                    // Must be followed by an untouched collapsed result
                    // to batch — expanded or user-toggled ones render
                    // individually so their state isn't silently
                    // swallowed by the collapsed form.
                    match entries.get(batch_end + 1) {
                        Some(LogEntry::ToolResult {
                            expanded: false,
                            collapsed_override: None,
                            ..
                        }) => {
                            summaries.push(s);
                            batch_end += 2;
                        }
                        _ => break,
                    }
                }
                if summaries.len() >= 2 {
                    out.push(Block {
                        kind: TranscriptBlock::ToolBatch(tool_call::BatchView {
                            family,
                            summaries,
                            collapsed: ctx.current_turn_start.is_some_and(|start| i < start),
                        }),
                        first_entry: i,
                        consumed: batch_end - i,
                    });
                    i = batch_end;
                    continue;
                }

                // Single call, paired with its immediate result when one
                // follows. In-flight calls (no result yet) render the
                // header with the elapsed ticker.
                let result = match entries.get(i + 1) {
                    Some(LogEntry::ToolResult {
                        ok,
                        snippet,
                        full,
                        expanded,
                        collapsed_override,
                    }) => Some(tool_result::ResultView {
                        ok: *ok,
                        snippet,
                        full,
                        expanded: *expanded,
                        collapsed: collapsed_override
                            .unwrap_or(ctx.current_turn_start.is_some_and(|start| i < start)),
                    }),
                    _ => None,
                };
                // Only feed the elapsed timer to the renderer for
                // in-flight calls — a completed call's ticker is noise.
                let elapsed = if result.is_none() {
                    Some(started_at.elapsed())
                } else {
                    None
                };
                let consumed = if result.is_some() { 2 } else { 1 };
                let call_id = match &entries[i] {
                    LogEntry::ToolCall { call_id, .. } => call_id.clone(),
                    _ => String::new(),
                };
                let tail = ctx
                    .tool_tail
                    .as_ref()
                    .filter(|(id, _)| *id == call_id)
                    .map(|(_, line)| line.clone());
                out.push(Block {
                    kind: TranscriptBlock::Tool(tool_call::ToolView {
                        name,
                        args,
                        preview: preview.as_ref(),
                        result,
                        elapsed,
                        undoable: ctx.undoable_idx == Some(i),
                        collapsed: result.as_ref().is_some_and(|r| r.collapsed),
                        tail,
                    }),
                    first_entry: i,
                    consumed,
                });
                i += consumed;
            }
            LogEntry::ToolResult {
                ok,
                snippet,
                full,
                expanded,
                collapsed_override,
            } => {
                // Orphan result (no preceding call) — should be rare, but
                // render defensively as a stand-alone block.
                out.push(Block {
                    kind: TranscriptBlock::Tool(tool_call::ToolView {
                        name: "(result)",
                        args: "",
                        preview: None,
                        result: Some(tool_result::ResultView {
                            ok: *ok,
                            snippet,
                            full,
                            expanded: *expanded,
                            collapsed: collapsed_override.unwrap_or(false),
                        }),
                        elapsed: None,
                        undoable: false,
                        collapsed: collapsed_override.unwrap_or(false),
                        tail: None,
                    }),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::User(s) => {
                out.push(Block {
                    kind: TranscriptBlock::User(s),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::Assistant(s) => {
                // Only the entry receiving live tokens is "streaming"; a
                // completed reply that happens to sit at the tail is not.
                let streaming = ctx.streaming && ctx.streaming_tail_idx == Some(i);
                out.push(Block {
                    kind: TranscriptBlock::Assistant {
                        text: s,
                        streaming,
                        plan: ctx.plan_mode,
                    },
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::TurnEnd {
                elapsed_ms,
                files,
                adds,
                dels,
                tools,
                cost,
            } => {
                out.push(Block {
                    kind: TranscriptBlock::TurnEnd(status::TurnEndView {
                        elapsed_ms: *elapsed_ms,
                        files: *files,
                        adds: *adds,
                        dels: *dels,
                        tools: *tools,
                        cost: *cost,
                    }),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::Welcome {
                model,
                mode,
                cwd,
                tip,
            } => {
                out.push(Block {
                    kind: TranscriptBlock::Welcome(message::WelcomeView {
                        model,
                        mode: *mode,
                        cwd,
                        version: env!("CARGO_PKG_VERSION"),
                        tip,
                    }),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::ToolCall { .. } => {
                // Task-bookkeeping call: hide the group (panel carries
                // the feedback) but consume call + paired result so the
                // entry→row table stays aligned.
                let consumed = if matches!(entries.get(i + 1), Some(LogEntry::ToolResult { .. })) {
                    2
                } else {
                    1
                };
                out.push(Block {
                    kind: TranscriptBlock::Suppressed,
                    first_entry: i,
                    consumed,
                });
                i += consumed;
            }
            LogEntry::Warning(s) => {
                out.push(Block {
                    kind: TranscriptBlock::Warning(s),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
            LogEntry::Info(s) => {
                out.push(Block {
                    kind: TranscriptBlock::Info(s),
                    first_entry: i,
                    consumed: 1,
                });
                i += 1;
            }
        }
    }
    out
}

/// Render one block to lines. `query`/`focused` drive the Ctrl+R search
/// highlight; ephemeral blocks ignore them (the highlight overlay is
/// for transcript content, not live prompts).
pub fn render_block(
    block: &Block<'_>,
    query: &str,
    focused: bool,
    width: u16,
) -> Vec<Line<'static>> {
    match &block.kind {
        TranscriptBlock::Truncated(n) => vec![Line::from(Span::styled(
            format!(
                "  … {} earlier entr{} truncated",
                n,
                if *n == 1 { "y" } else { "ies" },
            ),
            Style::default().fg(MUTED()).italic(),
        ))],
        TranscriptBlock::User(s) => message::user_lines(s),
        TranscriptBlock::Assistant {
            text,
            streaming,
            plan,
        } => message::assistant_lines(text, *streaming, *plan, query, focused),
        TranscriptBlock::TurnEnd(v) => status::turn_end_lines(v),
        TranscriptBlock::Welcome(w) => message::welcome_lines(w),
        TranscriptBlock::Tool(v) => tool_call::render(v, query, focused, width),
        TranscriptBlock::ToolBatch(v) => tool_call::render_batch(v),
        TranscriptBlock::Warning(s) => message::warning_lines(s),
        TranscriptBlock::Info(s) => message::info_lines(s),
        TranscriptBlock::Approval(v) => approval::render(v, width),
        TranscriptBlock::Tasks(v) => tasks::render(v),
        TranscriptBlock::PlanCard(p) => prompt::plan_card(p),
        TranscriptBlock::AskCard(a) => prompt::ask_card(a),
        TranscriptBlock::Suppressed => Vec::new(),
        TranscriptBlock::Working(v) => vec![status::working_line(v)],
    }
}

/// Task-bookkeeping tools: their tool groups are suppressed from the
/// transcript (the Tasks panel is the feedback surface).
pub(crate) fn is_task_tool(name: &str) -> bool {
    matches!(
        name,
        "task_create" | "task_update" | "task_list" | "task_get"
    )
}

/// True when `idx` is the entry a search hit currently points at —
/// used by callers to decide whether a block renders focused.
#[inline]
pub fn hit_in_block(block: &Block<'_>, hit_entry: Option<usize>) -> bool {
    match hit_entry {
        Some(idx) => idx >= block.first_entry && idx < block.first_entry + block.consumed,
        None => false,
    }
}

// ---- shared visual helpers ----

/// A `Line` is "empty" when every span it holds is blank. Used to
/// trim leading/trailing blank rows from a block's rendering so the
/// transcript separator stays a single row.
pub(crate) fn line_is_empty(l: &Line<'_>) -> bool {
    l.spans.iter().all(|s| s.content.trim().is_empty())
}

pub(crate) fn trim_empty(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines.first().map(line_is_empty).unwrap_or(false) {
        lines.remove(0);
    }
    while lines.last().map(line_is_empty).unwrap_or(false) {
        lines.pop();
    }
    lines
}

/// Re-scan one rendered line for the search needle and restyle match
/// runs. Loses per-span coloring but that's acceptable in search mode —
/// the highlight is the point.
pub(crate) fn highlight_line<'a>(line: Line<'a>, query: &str, focused: bool) -> Line<'a> {
    let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let needle = query.to_ascii_lowercase();
    let hay = raw.to_ascii_lowercase();
    let mut out: Vec<Span> = Vec::new();
    let mut cursor = 0;
    while let Some(off) = hay[cursor..].find(&needle) {
        let start = cursor + off;
        if start > cursor {
            out.push(Span::raw(raw[cursor..start].to_owned()));
        }
        let end = start + needle.len();
        let mut style = Style::default().bg(Color::Yellow).fg(Color::Black);
        if focused {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        out.push(Span::styled(raw[start..end].to_owned(), style));
        cursor = end;
    }
    if cursor < raw.len() {
        out.push(Span::raw(raw[cursor..].to_owned()));
    }
    if out.is_empty() {
        line
    } else {
        Line::from(out)
    }
}

/// Chip style per permission mode — shared by the header and footer.
pub(crate) fn mode_style(mode: mira_policy::Mode) -> Style {
    use mira_policy::Mode::*;
    let color = match mode {
        Plan => Color::Blue,
        Manual => Color::Green,
        Auto => Color::Yellow,
        Edit => Color::LightYellow,
        Yolo => Color::Red,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::LogEntry;

    fn ctx() -> BuildCtx {
        BuildCtx {
            streaming: false,
            plan_mode: false,
            streaming_tail_idx: None,
            undoable_idx: None,
            current_turn_start: None,
            tool_tail: None,
        }
    }

    fn call(name: &str, args: &str) -> LogEntry {
        LogEntry::ToolCall {
            name: name.into(),
            args: args.into(),
            preview: None,
            call_id: String::new(),
            started_at: std::time::Instant::now(),
        }
    }

    fn result(ok: bool, snippet: &str) -> LogEntry {
        LogEntry::ToolResult {
            ok,
            snippet: snippet.into(),
            full: String::new(),
            expanded: false,
            collapsed_override: None,
        }
    }

    #[test]
    fn tool_call_pairs_with_following_result() {
        let entries = vec![call("read_file", r#"{"path":"a.rs"}"#), result(true, "ok")];
        let blocks = build_blocks(&entries, &ctx());
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_entry, 0);
        assert_eq!(blocks[0].consumed, 2);
        assert!(matches!(
            blocks[0].kind,
            TranscriptBlock::Tool(tool_call::ToolView { .. })
        ));
    }

    #[test]
    fn consecutive_same_family_calls_batch() {
        let entries = vec![
            call("read_file", r#"{"path":"a.rs"}"#),
            result(true, "ok"),
            call("read_file", r#"{"path":"b.rs"}"#),
            result(true, "ok"),
            call("read_file", r#"{"path":"c.rs"}"#),
            result(true, "ok"),
        ];
        let blocks = build_blocks(&entries, &ctx());
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].consumed, 6);
        let TranscriptBlock::ToolBatch(b) = &blocks[0].kind else {
            panic!("expected batch");
        };
        assert_eq!(b.family, "Read");
        assert_eq!(b.summaries.len(), 3);
    }

    #[test]
    fn expanded_result_breaks_batching() {
        let entries = vec![
            call("read_file", r#"{"path":"a.rs"}"#),
            result(true, "ok"),
            LogEntry::ToolResult {
                ok: true,
                snippet: "big".into(),
                full: "big".into(),
                expanded: true,
                collapsed_override: None,
            },
            call("read_file", r#"{"path":"b.rs"}"#),
            result(true, "ok"),
        ];
        let blocks = build_blocks(&entries, &ctx());
        // First pair batches alone (< 2 → falls through to a plain pair),
        // the expanded result renders standalone, then the second pair.
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[1].consumed, 1);
    }

    #[test]
    fn different_families_dont_batch() {
        let entries = vec![
            call("read_file", r#"{"path":"a.rs"}"#),
            result(true, "ok"),
            call("shell", r#"{"cmd":"ls"}"#),
            result(true, "exit=0"),
        ];
        let blocks = build_blocks(&entries, &ctx());
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn orphan_result_renders_standalone() {
        let entries = vec![result(true, "leftover")];
        let blocks = build_blocks(&entries, &ctx());
        assert_eq!(blocks.len(), 1);
        let TranscriptBlock::Tool(v) = &blocks[0].kind else {
            panic!("expected tool view");
        };
        assert_eq!(v.name, "(result)");
        assert!(v.result.is_some());
    }

    #[test]
    fn tool_groups_before_current_turn_collapse() {
        let entries = vec![
            LogEntry::User("earlier".into()),
            call("read_file", r#"{"path":"a.rs"}"#),
            result(true, "ok"),
            LogEntry::User("now".into()),
            call("read_file", r#"{"path":"b.rs"}"#),
            result(true, "ok"),
        ];
        let c = BuildCtx {
            streaming: false,
            plan_mode: false,
            streaming_tail_idx: None,
            undoable_idx: None,
            current_turn_start: Some(3),
            tool_tail: None,
        };
        let blocks = build_blocks(&entries, &c);
        assert_eq!(blocks.len(), 3); // user, old pair, new pair
        let TranscriptBlock::Tool(old) = &blocks[1].kind else {
            panic!();
        };
        assert!(old.collapsed, "pre-turn group must auto-collapse");
        let TranscriptBlock::Tool(new) = &blocks[2].kind else {
            panic!();
        };
        assert!(!new.collapsed, "current-turn group stays expanded");
    }

    #[test]
    fn pinned_override_beats_auto_collapse() {
        let entries = vec![
            LogEntry::User("earlier".into()),
            call("read_file", r#"{"path":"a.rs"}"#),
            LogEntry::ToolResult {
                ok: true,
                snippet: "ok".into(),
                full: String::new(),
                expanded: false,
                collapsed_override: Some(false), // user pinned it open
            },
            LogEntry::User("now".into()),
        ];
        let c = BuildCtx {
            streaming: false,
            plan_mode: false,
            streaming_tail_idx: None,
            undoable_idx: None,
            current_turn_start: Some(3),
            tool_tail: None,
        };
        let blocks = build_blocks(&entries, &c);
        let TranscriptBlock::Tool(v) = &blocks[1].kind else {
            panic!();
        };
        assert!(!v.collapsed, "pinned-open beats auto-collapse");
    }

    #[test]
    fn user_toggled_calls_break_batching() {
        let entries = vec![
            call("read_file", r#"{"path":"a.rs"}"#),
            result(true, "ok"),
            call("read_file", r#"{"path":"b.rs"}"#),
            LogEntry::ToolResult {
                ok: true,
                snippet: "ok".into(),
                full: String::new(),
                expanded: false,
                collapsed_override: Some(true), // user collapsed this one
            },
        ];
        let blocks = build_blocks(&entries, &ctx());
        // The toggled result no longer batches — each pair renders alone.
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn only_streaming_tail_entry_is_marked_live() {
        let entries = vec![
            LogEntry::Assistant("first".into()),
            LogEntry::Assistant("second".into()),
        ];
        let c = BuildCtx {
            streaming: true,
            plan_mode: false,
            streaming_tail_idx: Some(1),
            undoable_idx: None,
            current_turn_start: None,
            tool_tail: None,
        };
        let blocks = build_blocks(&entries, &c);
        let TranscriptBlock::Assistant { streaming, .. } = &blocks[0].kind else {
            panic!();
        };
        assert!(!*streaming);
        let TranscriptBlock::Assistant { streaming, .. } = &blocks[1].kind else {
            panic!();
        };
        assert!(*streaming);
    }
}
