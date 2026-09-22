//! Footer — the single unified status row:
//!
//! ```text
//!  enter send · / cmd · @ file · shift+tab mode · esc esc quit      plan · shift+tab to cycle
//! ```
//!
//! Left = hint / flash / esc-pending / error bar. Right = mode chip in
//! its own color. An active error flash takes over the whole row in
//! solid red so a provider 401/429 is impossible to scroll past.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::components::{mode_style, MUTED, SALMON};
use crate::tui::state::TuiState;

pub(crate) fn footer(f: &mut Frame, area: Rect, state: &TuiState) {
    // Error flash trumps everything — a red-bg row for the full
    // duration of `state.error_flash_until`. Renders the row solid
    // so the label reads as an alert bar rather than a floating chip
    // that could get lost against the transcript.
    if state.error_flash_active() {
        let label = state
            .error_flash_label
            .as_deref()
            .unwrap_or("provider error");
        let bar = format!(" ⚠  {label}");
        let bar = pad_to_width(&bar, area.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }

    let left: Line<'static> = if state.esc_pending {
        let hint = if !state.is_input_empty() {
            "press esc again to clear input"
        } else {
            "press esc again to quit"
        };
        Line::from(Span::styled(
            format!(" {hint}"),
            Style::default().fg(SALMON()).bold(),
        ))
    } else if let Some(flash) = &state.flash {
        Line::from(Span::styled(
            format!(" {flash}"),
            Style::default().fg(Color::Green).italic(),
        ))
    } else {
        // Trimmed to the actions the landing mockup shows — the longer
        // inventory lives in `/help`.
        Line::from(Span::styled(
            " enter send · / cmd · @ file · shift+tab mode · esc esc quit",
            Style::default().fg(MUTED()),
        ))
    };

    let right = Line::from(vec![
        Span::styled(state.mode.chip_label(), mode_style(state.mode)),
        Span::styled(" · shift+tab to cycle ", Style::default().fg(MUTED())),
    ]);
    let right_len = right
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_len)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), cols[1]);
}

/// Pad `s` on the right with spaces so its char-count reaches `width`,
/// or truncate with `…` if it already exceeds. Used by the status bar
/// so a solid-color background (error flash) covers the whole row —
/// otherwise the terminal shows the transcript underneath the tail
/// end of the row.
fn pad_to_width(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count == width {
        return s.to_owned();
    }
    if count > width {
        let cut: String = s.chars().take(width.saturating_sub(1)).collect();
        return format!("{cut}…");
    }
    let mut out = s.to_owned();
    for _ in count..width {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_exact_and_overflow() {
        assert_eq!(pad_to_width("ab", 4), "ab  ");
        assert_eq!(pad_to_width("abcdef", 4), "abc…");
        assert_eq!(pad_to_width("abc", 3), "abc");
    }
}
