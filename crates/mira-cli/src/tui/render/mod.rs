//! Frame assembly — one `draw` per event-loop tick.
//!
//! The root split is:
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ ℳ mira · model · mode · ⎇ branch     ↑12k ↓4k │ header
//! │                                              │ (breathing row)
//! │ GOAL ────────────────────────────            │ goal panel (when set)
//! │ Implement authentication                     │
//! │ ℳ running · iteration 2/10 · …               │
//! │                                              │
//! │  conversation / agent output                 │ transcript
//! │                                              │
//! │ ▸▸ Type a message…                           │ composer
//! │  enter send · / cmd · …        auto · shift+tab │ footer
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Region modules own their internals; this file only decides where
//! they go and keeps the viewport bookkeeping in sync with the real
//! terminal size (covers the first frame and any Resize event the loop
//! somehow missed).

pub(crate) mod composer;
pub(crate) mod footer;
pub(crate) mod header;
pub(crate) mod layout;
pub(crate) mod overlays;
pub(crate) mod transcript;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

use crate::tui::components::status;
use crate::tui::state::{Palette, TuiState};

pub(crate) fn draw(f: &mut Frame, state: &mut TuiState) {
    // Keep the viewport bookkeeping honest: the first draw and any
    // missed resize both land here. `handle_resize` invalidates the
    // transcript layout cache and reconciles scroll.
    let area = f.area();
    if state.viewport_width != area.width || state.viewport_height != area.height {
        state.handle_resize(area.width, area.height);
    }

    // Input area grows with content — wrapping included — up to 8 body
    // rows, plus one row per queued message (typed while a turn was
    // streaming), then shrinks back after submit. Beyond the caps the
    // body scrolls bottom-anchored so the caret stays visible.
    // Composer uses a *top-only* coral hairline instead of a full box
    // border — 1 border row + N body rows, not 2 borders + N.
    let input_rows = composer::body_rows(state, area.width).min(composer::INPUT_CAP);
    let queued_rows = composer::queued_rows(state);
    let input_height = input_rows + queued_rows + 1;
    let goal_height = status::goal_rows(state.goal.as_ref());

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),      // header
            Constraint::Length(1),      // deliberate breathing row under the header
            Constraint::Length(goal_height), // persistent goal panel (0 when unset)
            Constraint::Min(1),         // transcript
            Constraint::Length(input_height),
            Constraint::Length(1),      // single unified footer
        ])
        .split(area);

    header::header(f, root[0], state);
    // root[1] is a blank row — air between the header rule and the
    // first `> user` prompt (or the GOAL card when one is running).
    if goal_height > 0 {
        if let Some(goal) = state.goal.as_ref() {
            let pulse_secs = state.booted_at.elapsed().as_secs_f32();
            let lines = status::goal_panel_lines(goal, pulse_secs);
            f.render_widget(ratatui::widgets::Paragraph::new(lines), root[2]);
        }
    }
    transcript::transcript(f, root[3], state);
    composer::composer(f, root[4], state);
    footer::footer(f, root[5], state);

    // Overlays: palette above the input, search bar likewise. The
    // inline approval renders as a transcript block below the tool
    // call that triggered it — no centered modal.
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
        terminal
            .draw(|f| draw(f, state))
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
    fn full_frame_renders_every_region() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);

        // header
        assert!(screen.contains("mira"), "header missing");
        assert!(screen.contains("test-model"), "model chip missing");
        assert!(screen.contains("auto"), "mode chip missing");
        // transcript content
        assert!(screen.contains(">"), "user prompt marker missing");
        assert!(screen.contains("what changed in render?"), "user text missing");
        assert!(screen.contains("I refactored the renderer"), "assistant missing");
        // turn-end full stop
        assert!(screen.contains("for 4.2s"), "turn-end marker missing");
        // footer
        assert!(screen.contains("enter send"), "footer hint missing");
        // composer
        assert!(screen.contains("message"), "composer placeholder missing");
    }

    #[test]
    fn tool_group_renders_compactly() {
        use std::time::Instant;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
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
        });
        let _ = Instant::now();
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("Read src/main.rs"), "tool header missing: {screen}");
        assert!(screen.contains("fn main() {}"), "tool snippet missing");
    }

    #[test]
    fn goal_panel_appears_when_goal_set() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        st.goal = Some(mira_harness::Goal::new("Implement authentication"));
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("GOAL"), "goal panel header missing");
        assert!(screen.contains("Implement authentication"), "condition missing");
            // and disappears again when cleared
        st.goal = None;
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(!screen.contains("GOAL"), "goal panel should be gone");
    }

    #[test]
    fn resize_rebuilds_and_keeps_frame_valid() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = populated_state();
        draw_once(&mut terminal, &mut st);
        assert_eq!(st.viewport_width, 100);
        assert_eq!(st.viewport_height, 30);
        assert!(st.layout_cache.is_some());

        // Simulate the terminal shrinking — the backend resizes, the
        // crossterm Resize event fires, then the next draw happens.
        terminal.backend_mut().resize(50, 12);
        st.handle_resize(50, 12);
        assert!(st.layout_cache.is_none(), "width change must invalidate");
        draw_once(&mut terminal, &mut st);
        assert_eq!(st.viewport_width, 50);
        assert!(st.layout_cache.is_some());
        // Scroll can never point past the tail after a resize + redraw.
        let tail = st.layout_cache.as_ref().unwrap().tail();
        assert!(st.scroll <= tail);

        // Height-only resize: cache survives, frame dims update.
        let cached_before = st.layout_cache.as_ref().unwrap().total_rows;
        terminal.backend_mut().resize(50, 20);
        st.handle_resize(50, 20);
        assert!(st.layout_cache.is_some());
        assert_eq!(st.layout_cache.as_ref().unwrap().total_rows, cached_before);
        draw_once(&mut terminal, &mut st);
    }

    #[test]
    fn search_hit_scrolls_into_view() {
        // Hit sits early in a long transcript: the scroll resolver
        // anchors it a third of the way down the viewport (mid-screen,
        // clear of the search overlay above the composer).
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        for i in 0..5 {
            st.push_info(format!("filler line {i}"));
        }
        st.push_user("the special token".into());
        for i in 0..25 {
            st.push_info(format!("later filler {i}"));
        }
        st.search_open();
        for c in "special".chars() {
            st.search_push(c);
        }
        assert!(st.active_hit().is_some(), "search must find a hit");
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("the special token"), "hit not scrolled into view");
    }

    #[test]
    fn startup_banner_renders_clean() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = TuiState::new("sonnet-4.5".into(), mira_policy::Mode::Auto);
        st.push_welcome(
            "sonnet-4.5".into(),
            mira_policy::Mode::Auto,
            "~/Desktop/coding_agent".into(),
            "type while mira works — enter queues the next turn",
        );
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("mira"), "brand missing");
        assert!(screen.contains("v0.3.6"), "version missing");
        assert!(screen.contains("sonnet-4.5"), "model missing");
        assert!(screen.contains("~/Desktop/coding_agent"), "cwd missing");
        assert!(screen.contains("queues the next turn"), "tip missing");
        // The old box-drawing banner is gone.
        assert!(!screen.contains("╭─"), "old box banner still present");
    }

    #[test]
    fn turn_receipt_renders_under_reply() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.push_user("do the thing".into());
        st.push_tool_call_raw("edit_file".into(), r#"{"path":"a.rs"}"#.into());
        st.push_tool_result_replay("edited");
        st.push_assistant("done.".into());
        // The receipt computes itself from the turn's entries: one
        // write tool ran (no preview on the replay path → no diffstats).
        st.push_turn_end(34_100, 900, Some(0.021));
        draw_once(&mut terminal, &mut st);
        let screen = screen_text(&terminal);
        assert!(screen.contains("for 34s"), "elapsed missing");
        assert!(screen.contains("1 file"), "files missing");
        assert!(screen.contains("1 tool"), "tools missing");
        assert!(screen.contains("$0.02"), "cost missing");
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
