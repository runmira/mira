//! Footer — a two-row status block:
//!
//! ```text
//!  enter send · / cmd · @ file · shift+tab mode · esc esc quit
//!                                     plan · shift+tab to cycle · ↑12k ↓4k · 8% · $0.02
//! ```
//!
//! Top row = context-aware hint / flash / esc-pending message. Bottom
//! row = right-aligned mode chip plus live usage accounting. Splitting
//! the two means the hint line never has to fight the token counters
//! for space on a narrow terminal, and the counters have the whole
//! width to spread across on the right when they need it.
//!
//! An active error flash takes over the top row in solid red so a
//! provider 401/429 is impossible to scroll past; the bottom row keeps
//! showing mode + usage so the layout doesn't jump.
//!
//! The hint row is context-aware: when a card holds the keys (approval,
//! plan, ask), when the palette is open, or while streaming, the footer
//! names the keys that actually work right now instead of the default
//! composer hints — the cards can scroll out of view, the footer can't.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::components::{mode_style, MUTED, SALMON};
use crate::tui::inline_term::Frame;
use crate::tui::state::{Palette, TuiState};

pub(crate) fn footer(f: &mut Frame, area: Rect, state: &TuiState) {
    // Split the footer area into two rows: hints on top, right-aligned
    // status (mode + usage) on the bottom. Falls back gracefully if
    // some caller only reserved one row — the top row wins and the
    // right-status row is silently dropped.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let top = rows[0];
    let bottom = rows.get(1).copied();

    // Error flash trumps the top row — a red-bg bar for the full
    // duration of `state.error_flash_until`. Renders solid so the
    // label reads as an alert bar rather than a floating chip that
    // could get lost against the transcript. The bottom row keeps
    // showing mode + usage so the layout doesn't jump.
    if state.error_flash_active() {
        let label = state
            .error_flash_label
            .as_deref()
            .unwrap_or("provider error");
        let bar = format!(" ⚠  {label}");
        let bar = pad_to_width(&bar, top.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ))),
            top,
        );
        if let Some(bottom) = bottom {
            render_status_row(f, bottom, state);
        }
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
    } else if let Some(hint) = context_hints(state) {
        Line::from(Span::styled(hint, Style::default().fg(MUTED())))
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
    f.render_widget(Paragraph::new(left), top);
    if let Some(bottom) = bottom {
        render_status_row(f, bottom, state);
    }
}

/// Right-aligned mode chip plus live usage accounting. Rendered into
/// the footer's bottom row so it always sits on its own line under the
/// hints — no truncation fight for space at narrow widths.
fn render_status_row(f: &mut Frame, area: Rect, state: &TuiState) {
    let mut spans = vec![
        Span::styled(state.mode.chip_label(), mode_style(state.mode)),
        Span::styled(" · shift+tab to cycle ", Style::default().fg(MUTED())),
    ];
    let usage = format_usage_spans(state);
    if !usage.is_empty() {
        spans.push(Span::styled("· ", Style::default().fg(MUTED())));
        spans.extend(usage);
        spans.push(Span::styled(" ", Style::default()));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        area,
    );
}

/// Keys that work right now, or `None` for the default composer hints.
/// Priority mirrors key routing in `input/keyboard.rs`: modal cards
/// first (they steal every key), then palette, then streaming. Flash
/// messages and the default hints are handled by the caller.
pub(crate) fn context_hints(state: &TuiState) -> Option<String> {
    if !state.pending_approvals.is_empty() {
        return Some(" ↑↓ move · 1-3 pick · y/a/n decide · esc deny".into());
    }
    if state.pending_plan.is_some() {
        return Some(
            " space toggle · ↑↓ move · enter yes, start editing · esc no, keep planning".into(),
        );
    }
    if let Some(ask) = state.pending_ask.as_ref() {
        if ask.text_mode {
            return Some(" enter commit text · esc stop editing".into());
        }
        if ask.proposal.questions.len() > 1 {
            return Some(" ↑↓ move · enter select/next · tab skip · esc cancel".into());
        }
        return Some(" ↑↓ move · enter select/send · esc cancel".into());
    }
    if state.palette.kind != Palette::None {
        return Some(" ↑↓ move · tab/enter accept · esc dismiss".into());
    }
    if state.streaming {
        return Some(" esc interrupt · enter queue message".into());
    }
    None
}

/// Compact live accounting as styled spans: `↑12.3k ↓4.1k · 15% · $0.024`.
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
        let pct = ((u.prompt_tokens as f64 / ctx as f64) * 100.0).min(100.0);
        const BAR: usize = 6;
        let filled = ((pct / 100.0) * BAR as f64).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(BAR - filled));
        let bar_color = if pct < 60.0 {
            Color::Green
        } else if pct < 85.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        spans.push(Span::styled(" · ", muted));
        spans.push(Span::styled(bar, Style::default().fg(bar_color)));
        spans.push(Span::styled(format!(" {pct:.0}%"), muted));
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
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD)
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
    use crate::tui::state::{PaletteState, PendingAsk, PendingPlan};

    fn blank_state() -> TuiState {
        TuiState::new("m".into(), mira_policy::Mode::Manual)
    }

    fn dummy_reply() -> tokio::sync::oneshot::Sender<mira_tools::prompt::PromptResponse> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(rx);
        tx
    }

    fn single_ask() -> PendingAsk {
        PendingAsk::new(
            "c".into(),
            mira_tools::prompt::AskUserProposal {
                questions: vec![mira_tools::prompt::AskUserQuestion {
                    question: "Q?".into(),
                    header: None,
                    options: vec![mira_tools::prompt::AskUserOption {
                        label: "A".into(),
                        description: None,
                        recommended: false,
                    }],
                    multi_select: false,
                }],
            },
            dummy_reply(),
        )
    }

    fn multi_ask() -> PendingAsk {
        let mut a = PendingAsk::new(
            "c".into(),
            mira_tools::prompt::AskUserProposal {
                questions: vec![
                    mira_tools::prompt::AskUserQuestion {
                        question: "Q1?".into(),
                        header: None,
                        options: vec![mira_tools::prompt::AskUserOption {
                            label: "A".into(),
                            description: None,
                            recommended: false,
                        }],
                        multi_select: false,
                    },
                    mira_tools::prompt::AskUserQuestion {
                        question: "Q2?".into(),
                        header: None,
                        options: vec![mira_tools::prompt::AskUserOption {
                            label: "B".into(),
                            description: None,
                            recommended: false,
                        }],
                        multi_select: false,
                    },
                ],
            },
            dummy_reply(),
        );
        a.question = 1;
        a
    }

    #[test]
    fn pad_exact_and_overflow() {
        assert_eq!(pad_to_width("ab", 4), "ab  ");
        assert_eq!(pad_to_width("abcdef", 4), "abc…");
        assert_eq!(pad_to_width("abc", 3), "abc");
    }

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

    #[test]
    fn no_context_means_default_hints() {
        assert_eq!(context_hints(&blank_state()), None);
    }

    #[test]
    fn approval_hints_name_allow_keys() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut st = blank_state();
        st.push_approval(crate::tui::state::PendingApproval {
            request: crate::tui::approver::ApprovalRequest {
                call: mira_core::ToolCall {
                    id: "1".into(),
                    kind: mira_core::message::ToolCallKind::Function,
                    function: mira_core::message::ToolCallFunction {
                        name: "shell".into(),
                        arguments: "{}".into(),
                    },
                },
                reply: tx,
            },
            preview: None,
        });
        let h = context_hints(&st).expect("approval context");
        assert!(h.contains("↑↓"), "{h}");
        assert!(h.contains("1-3"), "{h}");
        assert!(h.contains("y/a/n"), "{h}");
        assert!(h.contains("esc deny"), "{h}");
    }

    #[test]
    fn plan_hints_name_space_toggle() {
        let mut st = blank_state();
        st.pending_plan = Some(PendingPlan::new(
            "c".into(),
            mira_tools::prompt::PlanProposal {
                title: "t".into(),
                steps: vec![mira_tools::prompt::PlanStep {
                    description: "s".into(),
                    why: None,
                }],
            },
            dummy_reply(),
        ));
        let h = context_hints(&st).expect("plan context");
        assert!(h.contains("space"), "{h}");
        assert!(h.contains("yes, start editing"), "{h}");
        assert!(h.contains("no, keep planning"), "{h}");
    }

    #[test]
    fn ask_hints_follow_card_shape() {
        let mut st = blank_state();
        st.pending_ask = Some(single_ask());
        let h = context_hints(&st).expect("ask context");
        assert!(h.contains("select/send"), "{h}");

        let mut st = blank_state();
        st.pending_ask = Some(multi_ask());
        let h = context_hints(&st).expect("multi ask context");
        assert!(h.contains("select/next"), "{h}");

        let mut st = blank_state();
        let mut a = single_ask();
        a.text_mode = true;
        st.pending_ask = Some(a);
        let h = context_hints(&st).expect("text-mode context");
        assert!(h.contains("commit text"), "{h}");
    }

    #[test]
    fn palette_and_streaming_hints() {
        let mut st = blank_state();
        st.palette = PaletteState {
            kind: Palette::Slash,
            cursor: 0,
            matches: Vec::new(),
            filter: String::new(),
        };
        let h = context_hints(&st).expect("palette context");
        assert!(h.contains("accept"), "{h}");

        let mut st = blank_state();
        st.streaming = true;
        let h = context_hints(&st).expect("streaming context");
        assert!(h.contains("interrupt"), "{h}");
        assert!(h.contains("queue"), "{h}");
    }

    #[test]
    fn modal_cards_outrank_palette_and_streaming() {
        let mut st = blank_state();
        st.streaming = true;
        st.palette = PaletteState {
            kind: Palette::Slash,
            cursor: 0,
            matches: Vec::new(),
            filter: String::new(),
        };
        st.pending_plan = Some(PendingPlan::new(
            "c".into(),
            mira_tools::prompt::PlanProposal {
                title: "t".into(),
                steps: vec![mira_tools::prompt::PlanStep {
                    description: "s".into(),
                    why: None,
                }],
            },
            dummy_reply(),
        ));
        let h = context_hints(&st).expect("plan wins");
        assert!(h.contains("yes, start editing"), "{h}");
    }
}
