//! Transcript rendering — turns `TuiState` into visible rows.
//!
//! Two consumers today:
//!
//! - [`pane_cards`] — the interactive cards (approval / plan / ask,
//!   streaming status, tasks) that live INSIDE the inline pane above the
//!   composer.
//! - [`settled_lines`] — a flattened block dump for entries in
//!   `[emitted, end)`, pushed straight into real terminal scrollback via
//!   `InlineTerm::insert_history` on each event-loop tick.
//!
//! Assistant text is not routed through here — the streaming
//! `MarkdownStream` in `event_loop.rs` emits it line-by-line into
//! scrollback as it arrives.

use ratatui::text::Line;

use crate::tui::components::{self, trim_empty, BuildCtx};
use crate::tui::render::layout::SCROLLBACK_ROW;
use crate::tui::state::{LogEntry, TuiState};

/// Build the transcript's line list for scrollback emission. Retained
/// solely for the test suite (which asserts block assembly against a
/// known-good line dump) and the truncation-notice tests — the pane no
/// longer holds a transcript region.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct Built {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) entry_row_starts: Vec<usize>,
}

#[cfg(test)]
pub(crate) fn build_lines(state: &TuiState, width: u16) -> Built {
    let query = state
        .search
        .as_ref()
        .map(|s| s.query.clone())
        .unwrap_or_default();
    let active_hit = state.active_hit();
    let hit_entry = active_hit.map(|(idx, _)| idx);

    let blocks = assemble_blocks(state);
    let (lines, entry_row_starts) = render_block_list(
        &blocks,
        &query,
        hit_entry,
        width,
        BlockFilter::Live {
            emitted: state.emitted_entries,
        },
    );

    Built {
        lines,
        entry_row_starts,
    }
}

/// Lines for the interactive cards that live INSIDE the pane above the
/// composer: the streaming/working indicator, the tasks panel, and any
/// pending approval / plan / ask cards. These are ephemeral by design —
/// they answer "what does the pane need to show me RIGHT NOW" — so they
/// never touch scrollback. Returns an empty vec when the pane should be
/// just composer + footer.
pub(crate) fn pane_cards(state: &TuiState, width: u16) -> Vec<Line<'static>> {
    let blocks = assemble_blocks(state);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // In-flight `agent` tool calls are held at the emission frontier
    // (they haven't been pushed to scrollback yet because the ToolResult
    // hasn't arrived). They have `consumed > 0` so they're invisible to
    // the ephemeral-only filter below. Render them live at the top of
    // the pane so the nested subagent activity is visible as it happens.
    // Once the ToolResult arrives the block emits to scrollback and this
    // path stops rendering it — no double-paint.
    for block in &blocks {
        if block.consumed == 0 {
            continue;
        }
        if let components::TranscriptBlock::Tool(v) = &block.kind {
            if v.result.is_none() && v.name == "agent"
                && block.first_entry >= state.emitted_entries
            {
                let rendered = components::render_block(block, "", false, width);
                let trimmed = trim_empty(rendered);
                if !trimmed.is_empty() {
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    lines.extend(trimmed);
                }
            }
        }
    }

    // Ephemeral cards: working indicator, tasks, approval, plan/ask.
    for block in &blocks {
        if block.consumed != 0 {
            continue;
        }
        match block.kind {
            components::TranscriptBlock::Working(_)
            | components::TranscriptBlock::Tasks(_)
            | components::TranscriptBlock::Approval(_)
            | components::TranscriptBlock::PlanCard(_)
            | components::TranscriptBlock::AskCard(_)
            | components::TranscriptBlock::Waiting(_) => {
                let rendered = components::render_block(block, "", false, width);
                if !lines.is_empty() && !rendered.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.extend(trim_empty(rendered));
            }
            _ => {}
        }
    }

    // Pre-compaction warning — show once context crosses 80 % and clear
    // automatically when the harness fires Compacted.
    if state.context_warn_shown {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let pct = state.context_fill_pct().unwrap_or(80.0);
        let warn = Line::from(vec![
            Span::styled(
                "  ⚠ ",
                Style::default()
                    .fg(Color::Rgb(200, 140, 40))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Context {pct:.0}% full — older messages may be summarized soon"),
                Style::default()
                    .fg(Color::Rgb(180, 130, 60))
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(warn);
    }

    lines
}

/// Render entries `[emitted, end)` for mirroring into real scrollback
/// via `insert_before`: entry-backed blocks fully inside the range —
/// no search highlight (it must not freeze into history), no
/// ephemeral blocks. Callers pass `state.emission_frontier()` during
/// the run (keeps the current turn live) and `entries().len()` at
/// exit (scrollback ends with the complete transcript).
///
/// A user prompt tends to emit in its own single-block batch (it's
/// pushed just before the assistant starts streaming line-by-line), so
/// the inter-block separator inside `render_block_list` never runs for
/// it. This function stitches the framing blanks on at the batch edges
/// so user messages read as clear paragraph breaks in scrollback
/// regardless of the batch boundary they fall on.
pub(crate) fn settled_lines(state: &TuiState, width: u16, end: usize) -> Vec<Line<'static>> {
    if state.emitted_entries >= end {
        return Vec::new();
    }
    let blocks = assemble_blocks(state);
    let (mut lines, _) = render_block_list(
        &blocks,
        "",
        None,
        width,
        BlockFilter::Settled {
            emitted: state.emitted_entries,
            settled: end,
        },
    );
    if lines.is_empty() {
        return lines;
    }

    let range = &state.entries()[state.emitted_entries..end];
    let is_nonempty_user = |e: &LogEntry| matches!(e, LogEntry::User(s) if !s.trim().is_empty());
    let starts_with_user = range.first().is_some_and(is_nonempty_user);
    let ends_with_user = range.last().is_some_and(is_nonempty_user);

    if starts_with_user && state.emitted_entries > 0 {
        let mut out = Vec::with_capacity(lines.len() + 2);
        out.push(Line::from(""));
        out.push(Line::from(""));
        out.extend(lines);
        lines = out;
    }
    if ends_with_user {
        lines.push(Line::from(""));
        lines.push(Line::from(""));
    }
    lines
}

/// Which blocks a render pass keeps. `Live` skips fully-emitted entry
/// blocks (they're scrollback now); `Settled` keeps only blocks fully
/// inside the emission range. `Live` is only reachable via the
/// test-only `build_lines` helper today — production emit goes through
/// `Settled` — but is kept so tests keep exercising the same filter code.
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum BlockFilter {
    Live { emitted: usize },
    Settled { emitted: usize, settled: usize },
}

impl BlockFilter {
    fn keep(&self, first: usize, consumed: usize) -> bool {
        if consumed == 0 {
            // Ephemeral chrome (working line, cards, notices): live
            // only, never mirrored.
            return matches!(self, BlockFilter::Live { .. });
        }
        match *self {
            BlockFilter::Live { emitted } => first + consumed > emitted,
            BlockFilter::Settled { emitted, settled } => {
                first >= emitted && first + consumed <= settled
            }
        }
    }
}

/// Assemble the full block list for one frame: entry blocks from
/// `build_blocks`, the head-truncation notice, then ephemeral blocks
/// (working line, tasks, approval, plan/ask cards). Pure — borrows
/// state, allocates only small per-block vectors.
fn assemble_blocks(state: &TuiState) -> Vec<components::Block<'_>> {
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
        skip_assistant_idx: &state.streamed_assistant_idx,
        agent_cells: &state.agent_cells,
        bg_output_lines: &state.bg_output_lines,
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
    //
    // While a plan/ask card holds the keys, the generic streaming line
    // (verbs, token counts, esc-to-interrupt) is suppressed — the card
    // below owns the keys, so a short pointer line takes its place.
    let awaiting_input = state.pending_plan.is_some() || state.pending_ask.is_some();
    if state.streaming && !awaiting_input {
        let elapsed_secs = state
            .stream_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        // Parent completion tokens this turn, plus a char-count proxy
        // for subagent output (4 chars ≈ 1 token). The parent session
        // produces no completion tokens while waiting for an agent tool,
        // so without the proxy the counter stays flat the whole time a
        // subagent is running.
        let base_tokens = state
            .usage
            .completion_tokens
            .saturating_sub(state.turn_usage_baseline.completion_tokens);
        let tokens = base_tokens.saturating_add(state.turn_subagent_chars / 4);
        // Use the generic streaming indicator (wrangling, e.t.c.) instead
        // of naming the last in-flight tool call — the tool's own
        // transcript entry already shows what's happening.
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Working(components::status::StatusView {
                elapsed_secs,
                tokens,
                tool: None,
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
    if let Some(pending) = state.pending_approvals.front() {
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Approval(components::approval::ApprovalView {
                name: pending.request.call.function.name.as_str(),
                args: pending.request.call.function.arguments.as_str(),
                preview: pending.preview.as_ref(),
                focus: state.approval_focus,
                queued: state.pending_approvals.len().saturating_sub(1),
            }),
            first_entry: 0,
            consumed: 0,
        });
    }
    // Interactive tool cards — plan (coral) then ask (cyan), below the
    // approval prompt so the highest-stakes gate reads first. Each card
    // is preceded by its pointer line so the eye lands on what's needed.
    let elapsed_secs = state
        .stream_started_at
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    if let Some(plan) = state.pending_plan.as_ref() {
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Waiting(components::status::WaitingView {
                kind: components::status::WaitingKind::Plan,
                elapsed_secs,
            }),
            first_entry: 0,
            consumed: 0,
        });
        blocks.push(components::Block {
            kind: components::TranscriptBlock::PlanCard(plan),
            first_entry: 0,
            consumed: 0,
        });
    }
    if let Some(ask) = state.pending_ask.as_ref() {
        blocks.push(components::Block {
            kind: components::TranscriptBlock::Waiting(components::status::WaitingView {
                kind: components::status::WaitingKind::Ask,
                elapsed_secs,
            }),
            first_entry: 0,
            consumed: 0,
        });
        blocks.push(components::Block {
            kind: components::TranscriptBlock::AskCard(ask),
            first_entry: 0,
            consumed: 0,
        });
    }

    blocks
}

/// Render `blocks` to lines plus the entry→row table. `filter`
/// decides what each pass keeps; skipped entry-backed blocks still
/// push one [`SCROLLBACK_ROW`] slot per covered entry so the table
/// stays globally indexed. Separators only join consecutively
/// rendered blocks, so the scrollback/live seam stays clean on both
/// sides.
fn render_block_list(
    blocks: &[components::Block<'_>],
    query: &str,
    hit_entry: Option<usize>,
    width: u16,
    filter: BlockFilter,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut entry_row_starts: Vec<usize> = Vec::new();

    // Rendered-ness per block. Skipped (already-emitted) blocks act
    // exactly like suppressed ones for spacing: they render nothing
    // and swallow their separators, so the legacy layout is
    // byte-identical when nothing is skipped.
    let kept: Vec<bool> = blocks
        .iter()
        .map(|b| filter.keep(b.first_entry, b.consumed))
        .collect();
    let gone = |bi: usize| !kept[bi] || is_suppressed(&blocks[bi]);

    let last_idx = blocks.len().saturating_sub(1);
    for (bi, block) in blocks.iter().enumerate() {
        if !kept[bi] {
            for _ in 0..block.consumed {
                entry_row_starts.push(SCROLLBACK_ROW);
            }
            continue;
        }
        let start = lines.len();
        let focused = components::hit_in_block(block, hit_entry);
        let rendered = components::render_block(block, query, focused, width);
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
        // like the transcript has scroll gaps. User prompts get extra
        // breathing space on both sides: three blanks ABOVE the `> user`
        // line so the wash background reads as a paragraph break, and
        // two blanks BELOW the current user block before the next
        // reply lands, so the eye lands cleanly on the assistant dot.
        // Suppressed blocks (task bookkeeping) are spacing-invisible:
        // hiding a tool call must not leave a hole in the transcript.
        let next_gone = blocks.get(bi + 1).is_none_or(|_| gone(bi + 1));
        if bi != last_idx && !is_suppressed(block) && !next_gone {
            lines.push(Line::from(""));
            if next_is_user(blocks, bi) {
                lines.push(Line::from(""));
                lines.push(Line::from(""));
            }
            if is_user(block) {
                lines.push(Line::from(""));
                lines.push(Line::from(""));
            }
        }
    }

    (lines, entry_row_starts)
}

fn is_suppressed(block: &components::Block<'_>) -> bool {
    matches!(block.kind, components::TranscriptBlock::Suppressed)
}

fn next_is_user(blocks: &[components::Block<'_>], idx: usize) -> bool {
    matches!(
        blocks.get(idx + 1).map(|b| &b.kind),
        Some(components::TranscriptBlock::User(_))
    )
}

fn is_user(block: &components::Block<'_>) -> bool {
    matches!(block.kind, components::TranscriptBlock::User(_))
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
        // The assistant reply now carries the `●` header row, and
        // `next_is_user` inserts three blanks (1 default + 2 extra)
        // above the user line so the wash reads as a paragraph break:
        //   info(1) blank assistant-dot(1) assistant(1) blank×3 user(1)
        assert_eq!(built.lines.len(), 8);
        assert_eq!(built.entry_row_starts, vec![0, 2, 7]);
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

    fn joined(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect()
    }

    #[test]
    fn emitted_entries_leave_live_window_with_sentinels() {
        let mut st = state_with(vec![
            LogEntry::Info("old news".into()),
            LogEntry::Assistant("settled reply".into()),
            LogEntry::User("fresh question".into()),
        ]);
        st.emitted_entries = 2;
        let built = build_lines(&st, 100);
        let text = joined(&built.lines);
        assert!(!text.contains("old news"), "{text}");
        assert!(!text.contains("settled reply"), "{text}");
        assert!(text.contains("fresh question"), "{text}");
        // Table stays globally indexed: emitted slots are sentinels.
        assert_eq!(
            built.entry_row_starts,
            vec![
                crate::tui::render::layout::SCROLLBACK_ROW,
                crate::tui::render::layout::SCROLLBACK_ROW,
                0,
            ]
        );
    }

    #[test]
    fn settled_lines_mirror_only_the_frontier_range() {
        let mut st = state_with(vec![
            LogEntry::Info("one".into()),
            LogEntry::Info("two".into()),
            LogEntry::User("three".into()),
        ]);
        // Nothing streaming: everything is settled.
        let lines = settled_lines(&st, 100, st.entries().len());
        let text = joined(&lines);
        assert!(text.contains("one"), "{text}");
        assert!(text.contains("two"), "{text}");
        assert!(text.contains("three"), "{text}");

        st.emitted_entries = 3;
        assert!(settled_lines(&st, 100, st.entries().len()).is_empty());
    }

    #[test]
    fn settled_lines_hold_back_the_streaming_tail() {
        let mut st = state_with(vec![
            LogEntry::Info("done".into()),
            LogEntry::Assistant("still coming".into()),
        ]);
        st.streaming = true;
        let end = st.settled_count();
        let lines = settled_lines(&st, 100, end);
        let text = joined(&lines);
        assert!(text.contains("done"), "{text}");
        assert!(!text.contains("still coming"), "{text}");
        // …and no streaming cursor freezes into scrollback.
        assert!(!text.contains("▍"), "{text}");
    }

    #[test]
    fn emission_frontier_flushes_everything_settled() {
        // No streaming in flight: every entry has settled and is safe to
        // push into real scrollback, so the frontier is the entry count.
        let st = state_with(vec![
            LogEntry::User("first".into()),
            LogEntry::Assistant("reply one".into()),
            LogEntry::User("second".into()),
            LogEntry::Assistant("reply two".into()),
        ]);
        assert_eq!(st.emission_frontier(), 4);

        // Fresh session with no user message yet: pre-turn entries flush
        // the same way — nothing lives in the tiny inline viewport but
        // the composer and footer.
        let st = state_with(vec![LogEntry::Info("welcome-ish".into())]);
        assert_eq!(st.emission_frontier(), 1);
    }

    #[test]
    fn live_window_empties_once_the_turn_settles() {
        let mut st = state_with(vec![
            LogEntry::User("first".into()),
            LogEntry::Assistant("reply one".into()),
            LogEntry::User("second".into()),
            LogEntry::Assistant("reply two".into()),
        ]);
        // Simulate emission catching up to the frontier.
        st.emitted_entries = st.emission_frontier();
        let built = build_lines(&st, 100);
        let text = joined(&built.lines);
        // Nothing from either turn remains live — it all belongs to real
        // scrollback once settled.
        assert!(!text.contains("reply one"), "old turn scrolls: {text}");
        assert!(!text.contains("reply two"), "settled turn scrolls: {text}");
        assert!(!text.contains("second"), "settled turn scrolls: {text}");
        assert!(!text.contains("ask anything"), "no onboarding mid-session: {text}");
    }

    #[test]
    fn onboarding_only_on_fresh_session() {
        // Brand-new state still onboards…
        let st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let built = build_lines(&st, 100);
        assert!(joined(&built.lines).contains("ask anything"));

        // …but an all-emitted log does not re-onboard.
        let mut st = state_with(vec![LogEntry::Info("old".into())]);
        st.emitted_entries = 1;
        let built = build_lines(&st, 100);
        assert!(!joined(&built.lines).contains("ask anything"));
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
        st.push_approval(crate::tui::state::PendingApproval {
            request: crate::tui::approver::ApprovalRequest { call, reply: tx },
            preview: None,
        });
        let built = build_lines(&st, 100);
        let joined: String = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("Do you want to proceed?"));
    }

    #[test]
    fn task_tool_groups_are_suppressed_without_gaps() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.push_user("go".into());
        st.push_tool_call_raw(
            "task_create".into(),
            r#"{"subject":"Choose a file"}"#.into(),
        );
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
                w.iter()
                    .all(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
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
            images: Vec::new(),
        });
        st.apply_task_payload(&mira_core::ToolResult {
            call_id: "2".into(),
            content: String::new(),
            is_error: false,
            data: Some(serde_json::json!({ "task": {
                "id": 1, "subject": "Choose a file to edit",
                "status": "in_progress", "created_at": 0, "updated_at": 1
            }})),
            images: Vec::new(),
        });
        assert_eq!(st.tasks.len(), 1);
        assert_eq!(
            st.tasks[0].status,
            crate::tui::state::TaskStatus::InProgress
        );
        // Deletion drops it from the panel.
        st.apply_task_payload(&mira_core::ToolResult {
            call_id: "3".into(),
            content: String::new(),
            is_error: false,
            data: Some(serde_json::json!({ "task": {
                "id": 1, "subject": "x", "status": "deleted",
                "created_at": 0, "updated_at": 2
            }})),
            images: Vec::new(),
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
