//! Header — one compact identity row: `ℳ mira · model · mode · ⎇ branch`
//! with the usage/cost chip right-aligned.
//!
//! The standing goal deliberately does *not* live here anymore — it has
//! its own persistent panel (`components::status::goal_panel_lines`)
//! pinned under the header, where there's room for condition, loop
//! progress, and the evaluator's reason.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::tui::components::{mode_style, CREAM, DIM, LOGO, MUTED, SALMON};
use crate::tui::state::TuiState;

pub(crate) fn header(f: &mut Frame, area: Rect, state: &TuiState) {
    let mut spans = vec![
        Span::styled(
            format!("{LOGO} mira "),
            Style::default().fg(SALMON()).bold(),
        ),
        Span::styled("· ", Style::default().fg(DIM())),
        Span::styled(state.model.as_str(), Style::default().fg(CREAM())),
        Span::styled(" · ", Style::default().fg(DIM())),
        Span::styled(state.mode.as_str(), mode_style(state.mode)),
    ];
    if let Some(branch) = state.git_branch.as_deref() {
        spans.push(Span::styled(" · ", Style::default().fg(DIM())));
        spans.push(Span::styled(
            format!("⎇ {branch}"),
            Style::default().fg(MUTED()),
        ));
        // Dirty count — refreshed after every turn, so it goes up the
        // moment the agent starts writing files.
        if let Some(n) = state.git_dirty {
            if n > 0 {
                spans.push(Span::styled(
                    format!(" {n}±"),
                    Style::default().fg(SALMON()).bold(),
                ));
            }
        }
    }
    let left = Line::from(spans);

    // Usage chip on the right side of the header, so the bottom status
    // row can stay purely a hint/flash area. Rendered as styled spans
    // (not a plain string) so the dollar figure can flip red past
    // `state.budget_usd` without repainting the whole chip.
    let usage_spans = format_usage_spans(state);
    if usage_spans.is_empty() {
        f.render_widget(ratatui::widgets::Paragraph::new(left), area);
        return;
    }
    let right_len: usize = usage_spans.iter().map(|s| s.content.chars().count()).sum();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_len as u16)])
        .split(area);
    f.render_widget(ratatui::widgets::Paragraph::new(left), cols[0]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(usage_spans)).alignment(Alignment::Right),
        cols[1],
    );
}

/// Compact right-side accounting as styled spans: `↑12.3k ↓4.1k · 15% · $0.024`.
/// Dollar figure paints red when `state.budget_usd` is set and the
/// running cost has met or exceeded it. Empty when the provider hasn't
/// reported any usage yet.
fn format_usage_spans(state: &TuiState) -> Vec<Span<'static>> {
    use crate::tui::components::status::short_num;
    let u = &state.usage;
    if u.is_zero() {
        return Vec::new();
    }
    let muted = Style::default().fg(MUTED());
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        format!(
            "↑{} ↓{}",
            short_num(u.prompt_tokens),
            short_num(u.completion_tokens)
        ),
        muted,
    ));
    if let Some(ctx) = model_context_len(&state.model) {
        let pct = ((u.prompt_tokens as f64 / ctx as f64) * 100.0).min(999.0);
        spans.push(Span::styled(format!(" · {pct:.0}%"), muted));
    }
    if let Some(dollars) = mira_ai::cost_usd(
        &state.model,
        mira_ai::TokenUsage {
            prompt_tokens: u.prompt_tokens.min(u32::MAX as u64) as u32,
            completion_tokens: u.completion_tokens.min(u32::MAX as u64) as u32,
            cached_input_tokens: u.cached_input_tokens.min(u32::MAX as u64) as u32,
        },
    ) {
        spans.push(Span::styled(" · ".to_owned(), muted));
        let over = state.budget_usd.map(|cap| dollars >= cap).unwrap_or(false);
        let dollar_style = if over {
            Style::default()
                .fg(ratatui::style::Color::Red)
                .add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            muted
        };
        let mut label = format_dollars(dollars);
        if let Some(cap) = state.budget_usd {
            // Show the cap alongside so the user sees the runway shrink
            // without having to `/cost`.
            label.push_str(&format!("/${cap:.2}"));
        }
        spans.push(Span::styled(label, dollar_style));
    }
    spans
}

/// Best-effort context window size for common model IDs. Substring
/// match on the model string covers OpenRouter's `provider/name` form
/// as well as bare provider names. Returns `None` for unknowns → the
/// status bar just omits the `%` column.
fn model_context_len(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    // Order matters: check more specific matches first.
    if m.contains("gpt-4.1") || m.contains("gpt-5") {
        Some(1_000_000)
    } else if m.contains("claude-3-5-sonnet")
        || m.contains("claude-sonnet-4")
        || m.contains("claude-3-5-haiku")
        || m.contains("claude-haiku-4")
        || m.contains("claude-3-opus")
        || m.contains("claude-opus-4")
    {
        Some(200_000)
    } else if m.contains("gemini-2.5") || m.contains("gemini-1.5") {
        Some(1_000_000)
    } else if m.contains("gpt-4o")
        || m.contains("llama-3.3")
        || m.contains("llama-3.1")
        || m.contains("grok")
    {
        Some(128_000)
    } else if m.contains("deepseek") {
        Some(64_000)
    } else {
        None
    }
}

fn format_dollars(d: f64) -> String {
    if d < 0.01 {
        format!("${d:.4}")
    } else if d < 1.0 {
        format!("${d:.3}")
    } else {
        format!("${d:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_len_matches_common_ids() {
        assert_eq!(model_context_len("gpt-5"), Some(1_000_000));
        assert_eq!(
            model_context_len("openrouter/anthropic/claude-sonnet-4"),
            Some(200_000)
        );
        assert_eq!(
            model_context_len("google/gemini-2.5-flash"),
            Some(1_000_000)
        );
        assert_eq!(model_context_len("mystery-model"), None);
    }

    #[test]
    fn dollar_formatting() {
        assert_eq!(format_dollars(0.001), "$0.0010");
        assert_eq!(format_dollars(0.5), "$0.500");
        assert_eq!(format_dollars(12.345), "$12.35");
    }
}
