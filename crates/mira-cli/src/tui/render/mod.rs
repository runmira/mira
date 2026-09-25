//! Frame assembly — one `draw` per event-loop tick.
//!
//! There is deliberately no persistent header: session identity lives
//! in the one-time banner at the top of the transcript (which scrolls
//! into scrollback like everything else), so it never re-appears
//! mid-conversation after every turn. The live usage accounting the
//! header used to carry now rides on the footer's right side.
//!
//! The root split is:
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ GOAL ────────────────────────────            │ goal panel (when set)
//! │ Implement authentication                     │
//! │ ℳ running · iteration 2/10 · …               │
//! │                                              │
//! │  conversation / agent output                 │ transcript
//! │                                              │
//! │ ▸▸ Type a message…                           │ composer
//! │  enter send · / cmd · …   auto · ↑12k ↓4k    │ footer
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Region modules own their internals; this file only decides where
//! they go and keeps the viewport bookkeeping in sync with the real
//! terminal size (covers the first frame and any Resize event the loop
//! somehow missed).

pub(crate) mod composer;
pub(crate) mod footer;
pub(crate) mod layout;
pub(crate) mod overlays;
pub(crate) mod transcript;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::Wrap;

use crate::tui::components::status;
use crate::tui::inline_term::Frame;
use crate::tui::state::{Palette, TuiState};

/// Row count `lines` would occupy after word-wrapping to `width`
/// columns — matches what `Paragraph::wrap(Wrap { trim: false })`
/// actually produces. Ratatui's own `Paragraph::line_count(width)`
/// (behind the `unstable-rendered-line-info` feature) under-reports
/// on `Vec<Line>` input, so long lines get sized to a single row and
/// clip at the right edge. Direct display-width math sidesteps that.
pub(crate) fn wrapped_row_count(lines: &[Line<'_>], width: u16) -> u16 {
    use unicode_width::UnicodeWidthStr;
    let w = width.max(1) as usize;
    let mut total: u32 = 0;
    for line in lines {
        let display: usize = line
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        let rows = if display == 0 { 1 } else { display.div_ceil(w) };
        total = total.saturating_add(rows as u32);
    }
    total.min(u16::MAX as u32) as u16
}

/// The composer chrome contributes 2 border rows on top of its body.
const COMPOSER_PAD: u16 = 1;

/// Footer rows: top row for hints, bottom row for the right-aligned
/// mode chip + live usage accounting. Keeping the right chip on its
/// own row means a narrow terminal never truncates the hints to make
/// room for the token counters — and the counters have more room to
/// breathe on the right.
const FOOTER_ROWS: u16 = 2;

/// Pane layout — six regions in order:
///
///   [0] goal panel (0 rows unless a goal is set)
///   [1] interactive cards (approval / plan / ask / status / tasks)
///   [2] overlay reservation (palette or search — 0 otherwise)
///   [3] one-row padding above the composer
///   [4] composer chrome + body
///   [5] footer (always 1 row)
///
/// Computed the same way whether the caller wants the total height (to
/// resize the pane) or wants to actually render — the two must agree
/// exactly, otherwise the overlay writes past the buffer's bottom edge
/// and ratatui panics.
struct PaneLayout {
    goal_height: u16,
    cards_height: u16,
    overlay_height: u16,
    input_height: u16,
    card_lines: Vec<ratatui::text::Line<'static>>,
}

impl PaneLayout {
    fn total(&self) -> u16 {
        self.goal_height
            + self.cards_height
            + self.overlay_height
            + COMPOSER_PAD
            + self.input_height
            + FOOTER_ROWS
    }
}

fn layout_for(state: &TuiState, width: u16) -> PaneLayout {
    let input_rows = composer::body_rows(state, width.saturating_sub(2)).min(composer::INPUT_CAP);
    let queued_rows = composer::queued_rows(state);
    let input_height = input_rows + queued_rows + 2;
    let goal_height = status::goal_rows(state.goal.as_ref());
    let card_lines = transcript::pane_cards(state, width);
    // Cards can carry lines wider than the terminal (question text,
    // long option descriptions, approval preview snippets) — size the
    // region to the WRAPPED row count so those lines wrap into a new
    // row instead of clipping at the right edge.
    let cards_height = wrapped_row_count(&card_lines, width);
    let overlay_height = if state.palette.kind != Palette::None && !state.palette.matches.is_empty()
    {
        (state.palette.matches.len() as u16).min(8) + 2
    } else if state.search.is_some() {
        3
    } else {
        0
    };
    PaneLayout {
        goal_height,
        cards_height,
        overlay_height,
        input_height,
        card_lines,
    }
}

/// Total pane rows the current state wants. The event loop's
/// `draw_frame` calls this to size the inline pane BEFORE the render
/// closure runs — otherwise overlays would write into an old,
/// too-small buffer and panic.
pub(crate) fn desired_pane_height(state: &TuiState, width: u16) -> u16 {
    layout_for(state, width).total()
}

pub(crate) fn draw(f: &mut Frame, state: &mut TuiState) {
    let area = f.area();
    if state.viewport_width != area.width || state.viewport_height != area.height {
        state.handle_resize(area.width, area.height);
    }

    // Recompute the layout against the ACTUAL frame width — a resize
    // between `desired_pane_height` and here would produce a mismatch
    // otherwise. Panning is cheap and this keeps the layout coherent.
    let layout = layout_for(state, area.width);
    state.desired_viewport_height = layout.total();

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(layout.goal_height),
            Constraint::Length(layout.cards_height),
            Constraint::Length(layout.overlay_height),
            Constraint::Length(COMPOSER_PAD),
            Constraint::Length(layout.input_height),
            Constraint::Length(FOOTER_ROWS),
        ])
        .split(area);

    if layout.goal_height > 0 {
        if let Some(goal) = state.goal.as_ref() {
            let pulse_secs = state.booted_at.elapsed().as_secs_f32();
            let lines = status::goal_panel_lines(goal, pulse_secs);
            f.render_widget(ratatui::widgets::Paragraph::new(lines), root[0]);
        }
    }
    if layout.cards_height > 0 {
        // `wrap(Wrap { trim: false })` pairs with `wrapped_row_count`
        // above — same wrapper, same width, so what we measured is
        // what actually paints. Without this, plan/ask/approval card
        // rows longer than the terminal width clip at the right edge.
        f.render_widget(
            ratatui::widgets::Paragraph::new(layout.card_lines).wrap(Wrap { trim: false }),
            root[1],
        );
    }
    // Queued-message notice sits in the 1-row pad above the composer.
    // Grey, flush-left — visible at a glance without crowding the box.
    if !state.queued.is_empty() {
        use crate::tui::components::{DIM, MUTED};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::Paragraph;
        let n = state.queued.len();
        let notice = Line::from(vec![
            Span::styled("  ↑ ", Style::default().fg(DIM())),
            Span::styled(
                format!(
                    "press ↑ to edit {} queued message{}",
                    n,
                    if n == 1 { "" } else { "s" }
                ),
                Style::default().fg(MUTED()).italic(),
            ),
        ]);
        f.render_widget(Paragraph::new(notice), root[3]);
    }
    composer::composer(f, root[4], state);
    footer::footer(f, root[5], state);

    if state.palette.kind != Palette::None && !state.palette.matches.is_empty() {
        overlays::palette(f, root[4], state);
    }
    if state.search.is_some() {
        overlays::search(f, root[4], state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn populated_state() -> TuiState {
        let mut st = TuiState::new("test-model".into(), mira_policy::Mode::Auto);
        st.push_info("boot line");
        st.push_user("what changed in render?".into());
        st.push_assistant("I refactored the renderer into blocks.".into());
        st.push_turn_end(4200, 0, None);
        st
    }

    fn draw_once(terminal: &mut Terminal<TestBackend>, state: &mut TuiState) {
        // Bridge ratatui's `Frame` (from `TestBackend`) to our
        // `inline_term::Frame` so `render::draw`'s signature — which
        // only knows about our Frame — accepts the test backend without
        // divergence between test and production render paths.
        terminal
            .draw(|f| {
                let area = f.area();
                let buf = f.buffer_mut();
                let mut cursor: Option<ratatui::layout::Position> = None;
                let mut frame = Frame::from_parts(area, buf, &mut cursor);
                draw(&mut frame, state);
            })
            .expect("draw must not fail");
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        buf.content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn pane_renders_only_composer_and_footer() {
        // Every transcript entry (user, assistant, turn-end) now flows
        // into real terminal scrollback via `InlineTerm::insert_history`
        // rather than the inline pane. `TestBackend` doesn't simulate
        // scrollback, so all we can verify at the pane level is that
        // the composer and footer render — and that no transcript
        // content leaks back into the pane.
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);

        // No persistent header — the banner scrolled into history.
        assert!(!screen.contains("mira · test-model"), "header must be gone");
        // Transcript belongs to real scrollback now.
        assert!(
            !screen.contains("what changed in render?"),
            "user text leaked into pane"
        );
        assert!(
            !screen.contains("I refactored the renderer"),
            "assistant text leaked into pane"
        );
        assert!(
            !screen.contains("for 4.2s"),
            "turn-end marker leaked into pane"
        );
        // Composer + footer are the whole live pane.
        assert!(screen.contains("enter send"), "footer hint missing");
        assert!(screen.contains("message"), "composer placeholder missing");
    }

    #[test]
    fn tool_group_settled_lines_render_offline() {
        // Tool blocks also flow through scrollback, but we can still
        // exercise the block pipeline directly via `settled_lines` to
        // confirm the tool call/result rendering is intact.
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.push_tool_call(&mira_core::ToolCall {
            id: "1".into(),
            kind: mira_core::message::ToolCallKind::Function,
            function: mira_core::message::ToolCallFunction {
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            },
        });
        st.push_tool_result(&mira_core::ToolResult {
            call_id: "1".into(),
            content: "fn main() {}".into(),
            is_error: false,
            data: None,
            images: Vec::new(),
        });
        let lines = super::transcript::settled_lines(&st, 100, st.entries().len());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(
            joined.contains("Read src/main.rs"),
            "tool header missing: {joined}"
        );
        assert!(joined.contains("fn main() {}"), "tool snippet missing");
    }

    #[test]
    fn goal_panel_appears_when_goal_set() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        st.goal = Some(mira_harness::Goal::new("Implement authentication"));
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("GOAL"), "goal panel header missing");
        assert!(
            screen.contains("Implement authentication"),
            "condition missing"
        );
        // and disappears again when cleared
        st.goal = None;
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(!screen.contains("GOAL"), "goal panel should be gone");
    }

    #[test]
    fn resize_updates_viewport_dimensions() {
        // The pane doesn't hold a transcript any more, so a resize is
        // just "did the viewport width/height propagate to state" —
        // there's no layout cache to invalidate.
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        draw_once(&mut terminal, &mut st);
        assert_eq!(st.viewport_width, 100);
        assert_eq!(st.viewport_height, 30);

        terminal.backend_mut().resize(50, 12);
        st.handle_resize(50, 12);
        draw_once(&mut terminal, &mut st);
        assert_eq!(st.viewport_width, 50);
        assert_eq!(st.viewport_height, 12);
    }

    #[test]
    fn startup_banner_flushes_to_scrollback() {
        // The welcome banner used to render into the pane; now it flows
        // to real scrollback via `insert_history`. `TestBackend` doesn't
        // simulate scrollback, so we exercise `settled_lines` (the
        // exact same code path `emit_settled` uses in production) to
        // confirm the banner's marker rows show up.
        let mut st = TuiState::new("sonnet-4.5".into(), mira_policy::Mode::Auto);
        st.push_welcome(
            "sonnet-4.5".into(),
            mira_policy::Mode::Auto,
            "~/Desktop/coding_agent".into(),
            crate::tui::state::SessionMeta {
                provider: "anthropic".into(),
                branch: Some("main".into()),
                skills: vec!["review".into(), "plan".into()],
            },
            "type while mira works — enter queues the next turn",
        );
        let lines = super::transcript::settled_lines(&st, 100, st.entries().len());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("ℳ"), "brand missing: {joined}");
        assert!(joined.contains("sonnet-4.5"), "model missing");
        assert!(joined.contains("anthropic"), "provider missing");
        assert!(joined.contains("~/Desktop/coding_agent"), "cwd missing");
        assert!(joined.contains("Tip:"), "tip missing");
    }

    #[test]
    fn turn_receipt_renders_via_settled_lines() {
        // The receipt flows to scrollback, so exercise the block
        // pipeline directly rather than trying to catch it in the pane.
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.push_user("do the thing".into());
        st.push_tool_call_raw("edit_file".into(), r#"{"path":"a.rs"}"#.into());
        st.push_tool_result_replay("edited");
        st.push_assistant("done.".into());
        st.push_turn_end(34_100, 900, Some(0.021));
        let lines = super::transcript::settled_lines(&st, 100, st.entries().len());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("for 34s"), "elapsed missing: {joined}");
        assert!(joined.contains("1 file"), "files missing");
        assert!(joined.contains("1 tool"), "tools missing");
        assert!(joined.contains("$0.02"), "cost missing");
    }

    #[test]
    fn dropped_entries_show_truncation_notice() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.dropped_entries = 2;
        st.push_info("kept");
        let built = super::transcript::build_lines(&st, 100);
        let joined: String = built
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("2 earlier entries truncated"));
        assert!(joined.contains("kept"));
    }
}
