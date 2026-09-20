//! Transcript region — turns `TuiState` into visible rows.
//!
//! Pipeline per frame:
//!
//! ```text
//! entries ─▶ components::build_blocks ─▶ Vec<Block>
//!        ─▶ components::render_block  ─▶ Vec<Line>      (+ row-start table)
//!        ─▶ layout::resolve_scroll    ─▶ scroll offset
//!        ─▶ Paragraph                 ─▶ draw + cache layout
//! ```
//!
//! The renderer owns no scrolling logic of its own — spacing rules are
//! the only "where" it knows, and the scroll offset comes from
//! `render::layout`.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Stylize, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::components::{self, BuildCtx, SALMON, trim_empty};
use crate::tui::render::layout::{TranscriptLayout, resolve_scroll};
use crate::tui::state::{LogEntry, TuiState};

/// Lines built for one frame plus the entry→row mapping the layout
/// cache and scroll resolution need.
pub(crate) struct Built {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) entry_row_starts: Vec<usize>,
}

pub(crate) fn transcript(f: &mut Frame, area: Rect, state: &mut TuiState) {
    let built = build_lines(state, area.width);

    // Measure wrapped rows and cache the layout — this is the derived
    // state key handlers read via `cached_tail()`, and what resize
    // reconciliation clamps against.
    let text = Text::from(built.lines);
    let para = Paragraph::new(text).wrap(Wrap { trim: false });
    let total = para.line_count(area.width) as u16;
    let layout = TranscriptLayout {
        height: area.height,
        total_rows: total,
        entry_row_starts: built.entry_row_starts,
    };

    let scroll = resolve_scroll(state, &layout, area.height);
    // Turn-nav scroll is one-shot — consume it so a subsequent user
    // PgDn doesn't get snapped back to the anchor on next render.
    state.turn_scroll_target = None;
    // Persist the effective scroll so PgUp/PgDn work from where the user
    // is actually looking — not from a stale 0 they never chose.
    state.scroll = scroll;
    state.layout_cache = Some(layout);

    // Discovery hint: when the user has scrolled off the tail, show a
    // small chip in the top-right of the transcript so they know
    // earlier content is preserved and how to get back to live output.
    let tail = total.saturating_sub(area.height);
    if !state.follow_tail && tail > 0 {
        let msg = "↑ scrolled  ·  pgdn to catch up";
        let w = (msg.chars().count() as u16).min(area.width.saturating_sub(2));
        let chip = Rect {
            x: area.x + area.width.saturating_sub(w + 1),
            y: area.y,
            width: w,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.to_owned(),
                Style::default().fg(SALMON()).bold(),
            )))
            .alignment(Alignment::Right),
            chip,
        );
    }

    f.render_widget(para.scroll((scroll, 0)), area);
}

/// Build the full transcript line list for the current state.
pub(crate) fn build_lines(state: &TuiState, width: u16) -> Built {
    let query = state
        .search
        .as_ref()
        .map(|s| s.query.clone())
        .unwrap_or_default();
    let active_hit = state.active_hit();
    let hit_entry = active_hit.map(|(idx, _)| idx);

    // Identity of the entry that is currently receiving live tokens —
    // used to render its trailing edge with a soft cursor. Only when
    // `state.streaming` is true; a completed reply that happens to sit
    // at the tail shouldn't be marked as live.
    let streaming_tail_idx: Option<usize> = if state.streaming {
        state
            .entries()
            .iter()
            .rposition(|e| matches!(e, LogEntry::Assistant(_)))
    } else {
        None
    };

    // Locate the most-recent successful edit_file/write_file/
    // apply_patch/create_file call. Only *that one* gets the
    // `/undo to revert` chip below its result, so the affordance
    // always points at what `/undo` will actually roll back.
    let undoable_idx = components::tool_call::last_undoable_call_idx(state.entries());

    // Tool groups from turns before the last user prompt auto-collapse
    // (unless the user pinned them via 1–9).
    let current_turn_start = state
        .entries()
        .iter()
        .rposition(|e| matches!(e, LogEntry::User(s) if !s.trim().is_empty()));
    let ctx = BuildCtx {
        streaming: state.streaming,
        plan_mode: state.mode == mira_policy::Mode::Plan,
        streaming_tail_idx,
        undoable_idx,
        current_turn_start,
        tool_tail: state.tool_tail.clone(),
    };
    let mut blocks = components::build_blocks(state.entries(), &ctx);

    // Head-of-transcript truncation notice when the entry cap has
    // dropped older content — the layout keeps it out of the
    // entry→row table (consumed: 0).
    if state.dropped_entries > 0 {
        blocks.insert(
            0,
            components::Block {
                kind: components::TranscriptBlock::Truncated(state.dropped_entries),
                first_entry: 0,
                consumed: 0,
            },
        );
    }

    // Ephemeral blocks — never stored in `entries`, so they appear and
    // disappear cleanly with the session state. Order matters: the
    // working indicator first, then the inline approval prompt below it.
    if state.streaming {
        let elapsed_secs = state
            .stream_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let tokens = state
            .usage
            .completion_tokens
            .saturating_sub(state.turn_usage_baseline.completion_tokens);
        // Name the actual work: a trailing ToolCall with no result yet
        // is the tool in flight. Task bookkeeping stays invisible —
        // it's already suppressed from the stream and instant anyway.
        let tool = state.entries().iter().rev().find_map(|e| match e {
            LogEntry::ToolResult { .. } => None,
            LogEntry::ToolCall { name, args, started_at, .. } => {
                if components::is_task_tool(name) {
                    return None;
                }
                let (label, summary) = components::tool_call::summarize_tool(name, args);
                Some(components::status::InFlightTool {
                    label,
                    summary,
                    elapsed_secs: started_at.elapsed().as_secs_f32(),
                })
            }
            // Info/warning entries can interleave without changing
            // what's in flight.
            _ => None,
        });
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Working(components::status::StatusView {
                elapsed_secs,
                tokens,
                tool,
            }),
            first_entry: 0,
            consumed: 0,
        });
    }
    if !state.tasks.is_empty() {
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Tasks(components::tasks::TaskListView {
                items: &state.tasks,
            }),
            first_entry: 0,
            consumed: 0,
        });
    }
    if let Some(pending) = state.pending_approval.as_ref() {
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Approval(components::approval::ApprovalView {
                name: pending.request.call.function.name.as_str(),
                args: pending.request.call.function.arguments.as_str(),
                preview: pending.preview.as_ref(),
            }),
            first_entry: 0,
            consumed: 0,
        });
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut entry_row_starts: Vec<usize> = Vec::with_capacity(state.entries().len());

    let last_idx = blocks.len().saturating_sub(1);
    for (bi, block) in blocks.iter().enumerate() {
        let start = lines.len();
        let focused = components::hit_in_block(block, hit_entry);
        let rendered = components::render_block(block, &query, focused, width);
        // Blank edges get trimmed so separators stay one row tall —
        // markdown ending in `\n\n` would otherwise leave two blank
        // lines *plus* the separator below. (The approval card's
        // deliberate internal blanks survive: only edges are trimmed.)
        lines.extend(trim_empty(rendered));

        // Every entry the block covers maps to the block's first row so
        // search jumps and turn-nav land in the right place. Ephemeral
        // blocks cover no entries.
        for _ in 0..block.consumed {
            entry_row_starts.push(start);
        }

        // One blank row between blocks — anything more and it looks
        // like the transcript has scroll gaps. When the *next* block is
        // a user prompt, add a second blank row so the `> user` line
        // gets breathing space above and below (the ephemeral
        // indicator/approval render after a blank like any neighbor).
        // Suppressed blocks (task bookkeeping) are spacing-invisible:
        // hiding a tool call must not leave a hole in the transcript.
        if bi != last_idx && !is_suppressed(block) && !next_is_suppressed(&blocks, bi) {
            lines.push(Line::from(""));
            if next_is_user(&blocks, bi) {
                lines.push(Line::from(""));
            }
        }
    }

    if lines.is_empty() {
        append_onboarding(&mut lines);
    }

    Built {
        lines,
        entry_row_starts,
    }
}

fn is_suppressed(block: &components::Block<'_>) -> bool {
    matches!(block.kind, components::TranscriptBlock::Suppressed)
}

fn next_is_suppressed(blocks: &[components::Block<'_>], idx: usize) -> bool {
    blocks
        .get(idx + 1)
        .map(is_suppressed)
        .unwrap_or(false)
}

fn next_is_user(blocks: &[components::Block<'_>], idx: usize) -> bool {
    matches!(
        blocks.get(idx + 1).map(|b| &b.kind),
        Some(components::TranscriptBlock::User(_))
    )
}

/// First-run onboarding card. Three lines that answer the three "how do
/// I…" questions first-timers actually have. Fades out the moment the
/// transcript has any content — including our own boot info line — so
/// it never competes with real content. Styled small and quiet so it
/// reads as scaffolding.
fn append_onboarding(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("▸▸", Style::default().fg(SALMON()).bold()),
        Span::styled("  ask anything", Style::default().fg(components::CREAM())),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("/ ", Style::default().fg(SALMON()).bold()),
        Span::styled(" commands  ·  ", Style::default().fg(components::MUTED())),
        Span::styled("@", Style::default().fg(SALMON()).bold()),
        Span::styled("  include a file", Style::default().fg(components::MUTED())),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("shift+tab ", Style::default().fg(SALMON()).bold()),
        Span::styled("cycle modes  ·  ", Style::default().fg(components::MUTED())),
        Span::styled("/help ", Style::default().fg(SALMON()).bold()),
        Span::styled("for everything else", Style::default().fg(components::MUTED())),
    ]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn state_with(entries: Vec<LogEntry>) -> TuiState {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        for e in entries {
            match e {
                LogEntry::User(s) => st.push_user(s),
                LogEntry::Assistant(s) => st.push_assistant(s),
                LogEntry::Info(s) => st.push_info(s),
                LogEntry::Warning(s) => st.push_warning(s),
                LogEntry::TurnEnd { elapsed_ms, .. } => st.push_turn_end(elapsed_ms, 0, None),
                other => panic!("test helper doesn't support {other:?}"),
            }
        }
        st
    }

    #[test]
    fn blank_row_between_blocks_and_double_before_user() {
        let st = state_with(vec![
            LogEntry::Info("a".into()),
            LogEntry::Assistant("reply".into()),
            LogEntry::User("next".into()),
        ]);
        let built = build_lines(&st, 100);
        // info(1) blank assistant(1) blank blank user(1)
        assert_eq!(built.lines.len(), 6);
        assert_eq!(built.entry_row_starts, vec![0, 2, 5]);
    }

    #[test]
    fn truncated_notice_is_one_row_with_separator() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.dropped_entries = 3;
        st.push_info("hello");
        let built = build_lines(&st, 100);
        // notice(1) blank info(1)
        assert_eq!(built.lines.len(), 3);
        assert_eq!(built.entry_row_starts, vec![2]);
    }

    #[test]
    fn empty_state_shows_onboarding() {
        let st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let built = build_lines(&st, 100);
        assert!(built
            .lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("ask anything"))));
    }

    #[test]
    fn working_indicator_appended_when_streaming() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.streaming = true;
        st.push_info("x");
        let built = build_lines(&st, 100);
        let joined: String = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("esc to interrupt"));
        // Indicator follows a blank separator like any other block.
        assert!(built.lines[built.lines.len() - 2].spans.is_empty());
    }

    #[test]
    fn approval_prompt_appended_below_transcript() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let call = mira_core::ToolCall {
            id: "1".into(),
            kind: mira_core::message::ToolCallKind::Function,
            function: mira_core::message::ToolCallFunction {
                name: "shell".into(),
                arguments: r#"{"cmd":"cargo test"}"#.into(),
            },
        };
        st.pending_approval = Some(crate::tui::state::PendingApproval {
            request: crate::tui::approver::ApprovalRequest { call, reply: tx },
            preview: None,
        });
        let built = build_lines(&st, 100);
        let joined: String = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("approval required"));
    }

    #[test]
    fn task_tool_groups_are_suppressed_without_gaps() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.push_user("go".into());
        st.push_tool_call_raw("task_create".into(), r#"{"subject":"Choose a file"}"#.into());
        st.push_tool_result_replay("created task #1: Choose a file");
        st.push_tool_call_raw("read_file".into(), r#"{"path":"README.md"}"#.into());
        st.push_tool_result_replay("hi");
        let built = build_lines(&st, 100);
        let joined: String = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        // Task bookkeeping is invisible…
        assert!(!joined.contains("Choose a file"), "{joined}");
        assert!(!joined.contains("created task"), "{joined}");
        // …the real tool still renders…
        assert!(joined.contains("Read README.md"), "{joined}");
        // …and no double blank rows where the call was hidden.
        let rows = &built.lines;
        let gaps = rows
            .windows(2)
            .filter(|w| {
                w.iter().all(|l| {
                    l.spans.iter().all(|s| s.content.trim().is_empty())
                })
            })
            .count();
        assert_eq!(gaps, 0, "no consecutive blank rows: {rows:?}");
        // Entry→row table stays aligned for search/turn-nav.
        assert_eq!(built.entry_row_starts.len(), st.entries().len());
    }

    #[test]
    fn task_panel_reflects_tool_payloads() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.apply_task_payload(&mira_core::ToolResult {
            call_id: "1".into(),
            content: String::new(),
            is_error: false,
            data: Some(serde_json::json!({ "task": {
                "id": 1, "subject": "Choose a file to edit",
                "status": "pending", "created_at": 0, "updated_at": 0
            }})),
        });
        st.apply_task_payload(&mira_core::ToolResult {
            call_id: "2".into(),
            content: String::new(),
            is_error: false,
            data: Some(serde_json::json!({ "task": {
                "id": 1, "subject": "Choose a file to edit",
                "status": "in_progress", "created_at": 0, "updated_at": 1
            }})),
        });
        assert_eq!(st.tasks.len(), 1);
        assert_eq!(st.tasks[0].status, crate::tui::state::TaskStatus::InProgress);
        // Deletion drops it from the panel.
        st.apply_task_payload(&mira_core::ToolResult {
            call_id: "3".into(),
            content: String::new(),
            is_error: false,
            data: Some(serde_json::json!({ "task": {
                "id": 1, "subject": "x", "status": "deleted",
                "created_at": 0, "updated_at": 2
            }})),
        });
        assert!(st.tasks.is_empty());
    }

    #[test]
    fn search_query_flows_into_highlight() {
        let mut st = state_with(vec![LogEntry::User("find the needle here".into())]);
        st.search_open();
        for c in "needle".chars() {
            st.search_push(c);
        }
        let built = build_lines(&st, 100);
        let highlighted = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.add_modifier.intersects(Modifier::BOLD));
        assert!(highlighted);
    }
}
