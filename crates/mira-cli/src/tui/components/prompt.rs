//! Interactive prompt cards — the `plan` and `ask_user` tool cards.
//!
//! These are the tools' UI: ephemeral transcript blocks that steal keys until
//! answered. The visual language is intentionally friendly and compact: Mira
//! feels like a little agent talking to you, not a form rendered inside a
//! terminal window.
//!
//! `plan` uses Mira's salmon accent and checklist/progress language.
//! `ask_user` uses cyan and conversational question language.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use super::{CREAM, DIM, MUTED, SALMON, truncate};
use crate::tui::state::{PendingAsk, PendingPlan};
use unicode_width::UnicodeWidthStr;

/* ---------- shared primitives ---------- */

fn blank() -> Line<'static> {
    Line::default()
}

fn keycap(key: &str) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::default()
            .fg(CREAM())
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_hints(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(items.len() * 3);
    for (i, (key, action)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(MUTED())));
        }
        spans.push(keycap(key));
        spans.push(Span::styled(
            format!("  {action}"),
            Style::default().fg(MUTED()),
        ));
    }
    Line::from(spans)
}

fn detail_line(text: String, _accent: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("     ", Style::default()),
        Span::styled(text, Style::default().fg(MUTED()).italic()),
    ])
}

/// Count line over the rows, e.g. `3 steps · 2 included`.
fn count_line(head: String, tail: String) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(head, Style::default().fg(CREAM()).bold()),
        Span::styled(tail, Style::default().fg(MUTED())),
    ])
}

/// One picker row in the shared question/picker language: every row is
/// numbered (`N.` doubles as the digit hotkey), the focused row gets
/// the `▸` cursor plus a full-width wash (padded to `width` so the
/// selection spans edge to edge), and a picked row reads green with a
/// `✓` suffix. Mirrors the generic list-selection look: number,
/// label, state — nothing else competes for the eye.
fn picker_row(
    idx: usize,
    label: &str,
    focused: bool,
    picked: bool,
    recommended: bool,
    accent: Color,
    width: u16,
) -> Line<'static> {
    let wash = Style::default().bg(DIM());
    let mut spans = vec![
        Span::styled(
            if focused { "▸ " } else { "  " },
            if focused {
                Style::default().fg(accent).bold().bg(DIM())
            } else {
                Style::default()
            },
        ),
        Span::styled(
            format!("{idx}. "),
            if focused {
                Style::default().fg(MUTED()).bg(DIM())
            } else {
                Style::default().fg(MUTED())
            },
        ),
        Span::styled(
            label.to_owned(),
            if focused {
                Style::default().fg(SALMON()).bold().bg(DIM())
            } else if picked {
                Style::default().fg(Color::Green).bold()
            } else {
                Style::default().fg(CREAM())
            },
        ),
    ];

    if picked {
        spans.push(Span::styled(
            " ✓",
            if focused {
                Style::default().fg(Color::Green).bold().bg(DIM())
            } else {
                Style::default().fg(Color::Green).bold()
            },
        ));
    }

    if recommended {
        spans.push(Span::styled(
            "  ✦ recommended",
            if focused {
                Style::default().fg(Color::Green).italic().bg(DIM())
            } else {
                Style::default().fg(Color::Green).italic()
            },
        ));
    }

    if focused {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let width = width.max(1) as usize;
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), wash));
        }
    }

    Line::from(spans)
}

/* ---------- plan card ---------- */

pub(crate) fn plan_card(p: &PendingPlan, width: u16) -> Vec<Line<'static>> {
    let accent = SALMON();
    let mut out = vec![];

    if !p.proposal.title.is_empty() {
        out.push(Line::from(Span::styled(
            format!("  {}", p.proposal.title),
            Style::default().fg(CREAM()).bold(),
        )));
        out.push(blank());
    }

    let done = p.checked.iter().filter(|checked| **checked).count();
    let total = p.proposal.steps.len();
    out.push(count_line(
        format!("{total} step{}", if total == 1 { "" } else { "s" }),
        format!(
            " · {done} included{}",
            if total - done > 0 {
                format!(", {} excluded", total - done)
            } else {
                String::new()
            }
        ),
    ));
    out.push(blank());

    for (i, step) in p.proposal.steps.iter().enumerate() {
        let focused = i == p.focus;
        out.push(picker_row(
            i + 1,
            &step.description,
            focused,
            p.checked[i],
            false,
            accent,
            width,
        ));

        if focused {
            if let Some(why) = &step.why {
                if !why.trim().is_empty() {
                    out.push(detail_line(truncate(why, 100), accent));
                }
            }
        }
    }

    out.push(blank());
    out.push(footer_hints(&[
        ("Space", "toggle"),
        ("↑↓", "move"),
        ("Enter", "yes, start editing"),
        ("Esc", "no, keep planning"),
    ]));
    out
}

/* ---------- ask card ---------- */

fn question_answered(a: &PendingAsk, qi: usize) -> bool {
    !a.picked[qi].is_empty() || a.custom[qi].is_some()
}

fn tabs_row(a: &PendingAsk, accent: Color) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", Style::default())];

    for (qi, q) in a.proposal.questions.iter().enumerate() {
        let label = q
            .header
            .clone()
            .unwrap_or_else(|| format!("Q{}", qi + 1));
        let active = qi == a.question;
        let answered = question_answered(a, qi);

        if qi > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }

        let icon = if answered { "✓" } else { "○" };
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .bold()
        } else if answered {
            Style::default().fg(Color::Green).bold()
        } else {
            Style::default().fg(MUTED())
        };

        spans.push(Span::styled(format!(" {icon} {label} "), style));
    }

    Line::from(spans)
}

pub(crate) fn ask_card(a: &PendingAsk, width: u16) -> Vec<Line<'static>> {
    let accent = Color::Cyan;
    let mut out = vec![];

    if a.proposal.questions.len() > 1 {
        out.push(tabs_row(a, accent));
        out.push(blank());
    } else if let Some(header) = &a.proposal.questions[a.question].header {
        if !header.trim().is_empty() {
            out.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    header.to_uppercase(),
                    Style::default().fg(accent).bold(),
                ),
            ]));
            out.push(blank());
        }
    }

    let qi = a.question;
    let q = &a.proposal.questions[qi];

    out.push(Line::from(Span::styled(
        format!("  {}", q.question),
        Style::default().fg(CREAM()).bold(),
    )));
    out.push(blank());

    for (oi, opt) in q.options.iter().enumerate() {
        let focused = oi == a.option;
        let picked = a.picked[qi].iter().any(|l| *l == opt.label);
        out.push(picker_row(
            oi + 1,
            &opt.label,
            focused,
            picked,
            opt.recommended,
            accent,
            width,
        ));

        // Descriptions always shown (not just on focus) so users can
        // compare options at a glance without navigating to each one.
        if let Some(desc) = &opt.description {
            if !desc.trim().is_empty() {
                out.push(detail_line(truncate(desc, 100), accent));
            }
        }
    }

    out.push(blank());
    if a.text_mode {
        out.push(Line::from(vec![
            Span::styled("  ❯ t  ", Style::default().fg(accent).bold()),
            Span::styled(a.text.clone(), Style::default().fg(CREAM())),
            Span::styled(
                "▍",
                Style::default().fg(SALMON()).add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::styled("  t  ", Style::default().fg(accent).bold()),
            Span::styled(
                "type something",
                Style::default().fg(MUTED()).italic(),
            ),
        ]));
    }

    if let Some(custom) = &a.custom[qi] {
        out.push(blank());
        out.push(Line::from(vec![
            Span::styled("  ✓  ", Style::default().fg(Color::Green).bold()),
            Span::styled(
                format!("you said: {custom}"),
                Style::default().fg(Color::Green).italic(),
            ),
        ]));
    }

    out.push(blank());
    out.push(Line::from(Span::styled(
        "─".repeat((width as usize).saturating_sub(2)),
        Style::default().fg(DIM()),
    )));
    let footer_text = if a.proposal.questions.len() > 1 {
        "Enter to select  ·  ↑↓ / Tab to navigate  ·  Esc to cancel"
    } else {
        "Enter to select  ·  ↑↓ to navigate  ·  Esc to cancel"
    };
    out.push(Line::from(Span::styled(
        footer_text,
        Style::default().fg(MUTED()),
    )));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_tools::prompt::{
        AskUserOption, AskUserQuestion, AskUserProposal, PlanStep, PromptResponse,
    };

    /// Tests only exercise rendering, never the round-trip — a sender
    /// whose receiver was dropped, so `send` just no-ops.
    fn dummy_reply() -> tokio::sync::oneshot::Sender<PromptResponse> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(rx);
        tx
    }

    fn plan() -> PendingPlan {
        PendingPlan::new(
            "call-1".into(),
            mira_tools::prompt::PlanProposal {
                title: "Refactor renderer".into(),
                steps: vec![
                    PlanStep {
                        description: "Extract layout module".into(),
                        why: Some("single source for scroll math".into()),
                    },
                    PlanStep {
                        description: "Move scroll into state".into(),
                        why: None,
                    },
                ],
            },
            dummy_reply(),
        )
    }

    fn ask() -> PendingAsk {
        PendingAsk::new(
            "call-2".into(),
            AskUserProposal {
                questions: vec![AskUserQuestion {
                    question: "Which storage backend?".into(),
                    header: Some("Backend".into()),
                    options: vec![
                        AskUserOption {
                            label: "SQLite".into(),
                            description: Some("zero-config, single file".into()),
                            recommended: true,
                        },
                        AskUserOption {
                            label: "Postgres".into(),
                            description: None,
                            recommended: false,
                        },
                    ],
                    multi_select: false,
                }],
            },
            dummy_reply(),
        )
    }

    fn text_of(ls: &[Line<'static>]) -> String {
        ls.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn plan_card_shows_chip_title_and_focus() {
        let p = plan();
        let t = text_of(&plan_card(&p, 100));
        assert!(t.contains("Refactor renderer"), "title");
        assert!(t.contains("2 steps"), "count line: {t}");
        assert!(t.contains("2 included"), "count line: {t}");
        assert!(t.contains("▸ 1. Extract layout module ✓"), "focused checked row: {t}");
        assert!(t.contains("2. Move scroll into state ✓"), "checked row: {t}");
        assert!(t.contains("single source for scroll math"), "focused why");
        assert!(t.contains("Space"), "footer hints: {t}");
        assert!(t.contains("toggle"), "footer hints: {t}");
        assert!(t.contains("yes, start editing"), "footer hints: {t}");
        assert!(t.contains("no, keep planning"), "footer hints: {t}");
    }

    #[test]
    fn plan_card_hides_why_for_unfocused_step() {
        // Focused step (0) has a `why`; step 1 doesn't, and shouldn't
        // be expected to show anything even when it becomes focused.
        let mut p = plan();
        p.focus = 1;
        let t = text_of(&plan_card(&p, 100));
        assert!(!t.contains("single source for scroll math"));
    }

    #[test]
    fn plan_card_reflects_toggles_and_focus() {
        let mut p = plan();
        p.checked[0] = !p.checked[0];
        p.focus = 1;
        let t = text_of(&plan_card(&p, 100));
        assert!(t.contains("1 included"), "count line: {t}");
        assert!(t.contains("1 excluded"), "count line: {t}");
        assert!(!t.contains("1. Extract layout module ✓"), "unchecked: {t}");
        assert!(t.contains("1. Extract layout module"), "row still listed: {t}");
        assert!(t.contains("▸ 2. Move scroll into state ✓"), "focused checked: {t}");
    }

    #[test]
    fn ask_card_shows_chip_question_and_hotkeys() {
        let t = text_of(&ask_card(&ask(), 100));
        assert!(t.contains("BACKEND"), "header");
        assert!(t.contains("Which storage backend?"));
        assert!(t.contains("SQLite"), "option 1");
        assert!(t.contains("Postgres"), "option 2");
        assert!(t.contains("✦ recommended"), "badge");
        assert!(t.contains("t  type something"), "free-text affordance");
        assert!(t.contains("Enter to select"), "footer: {t}");
        assert!(t.contains("Esc"), "hints: {t}");
        assert!(t.contains("cancel"), "hints: {t}");
    }

    #[test]
    fn ask_card_shows_option_descriptions() {
        let t = text_of(&ask_card(&ask(), 100)); // option 0 focused by default
        assert!(t.contains("zero-config, single file"), "option 0 desc visible");
    }

    #[test]
    fn ask_card_shows_unfocused_option_description() {
        let mut a = ask();
        a.option = 1; // focus on Postgres
        let t = text_of(&ask_card(&a, 100));
        // SQLite's description is always visible even when its row is not focused.
        assert!(t.contains("zero-config, single file"), "unfocused desc always shown");
    }

    #[test]
    fn ask_card_reflects_pick_and_custom() {
        let mut a = ask();
        a.picked[0] = vec!["Postgres".into()];
        a.custom[0] = Some("but keep the file format".into());
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains("2. Postgres ✓"), "single-select pick: {t}");
        assert!(t.contains("▸ 1. SQLite"), "focus stays put: {t}");
        assert!(t.contains("you said: but keep the file format"));
    }

    #[test]
    fn plan_focused_row_washes_full_width() {
        use unicode_width::UnicodeWidthStr;
        let ls = plan_card(&plan(), 100);
        let focused: Vec<_> = ls
            .iter()
            .filter(|l| {
                l.spans
                    .iter()
                    .any(|s| s.content.contains('▸'))
            })
            .collect();
        assert_eq!(focused.len(), 1);
        let cols: usize = focused[0]
            .spans
            .iter()
            .map(|sp| sp.content.width())
            .sum();
        assert_eq!(cols, 100, "{:?}", focused[0].spans);
    }

    #[test]
    fn ask_card_picker_rows_number_every_option() {
        // Every row carries its digit hotkey, not just the focused one.
        let t = text_of(&ask_card(&ask(), 100));
        assert!(t.contains("▸ 1. SQLite"), "focused row: {t}");
        assert!(t.contains("2. Postgres"), "unfocused row still numbered: {t}");
    }

    #[test]
    fn multi_select_uses_checkboxes() {
        let mut a = PendingAsk::new(
            "c".into(),
            AskUserProposal {
                questions: vec![AskUserQuestion {
                    question: "Which features?".into(),
                    header: None,
                    options: vec![
                        AskUserOption {
                            label: "A".into(),
                            description: None,
                            recommended: false,
                        },
                        AskUserOption {
                            label: "B".into(),
                            description: None,
                            recommended: false,
                        },
                    ],
                    multi_select: true,
                }],
            },
            dummy_reply(),
        );
        a.picked[0] = vec!["A".into()];
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains("▸ 1. A ✓"), "{t}");
        assert!(t.contains("2. B"), "{t}");
        assert!(!t.contains("2. B ✓"), "{t}");
    }

    fn multi_question_ask() -> PendingAsk {
        PendingAsk::new(
            "c2".into(),
            AskUserProposal {
                questions: vec![
                    AskUserQuestion {
                        question: "Q1?".into(),
                        header: Some("Display mode".into()),
                        options: vec![AskUserOption {
                            label: "A".into(),
                            description: None,
                            recommended: false,
                        }],
                        multi_select: false,
                    },
                    AskUserQuestion {
                        question: "Q2?".into(),
                        header: Some("Language".into()),
                        options: vec![AskUserOption {
                            label: "B".into(),
                            description: None,
                            recommended: false,
                        }],
                        multi_select: false,
                    },
                ],
            },
            dummy_reply(),
        )
    }

    #[test]
    fn ask_card_shows_only_the_active_question() {
        // Question 0 is active by default: its body renders, question
        // 1's body does not — only its tab chip does.
        let a = multi_question_ask();
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains("Q1?"), "active question body: {t}");
        assert!(!t.contains("Q2?"), "inactive question body must be hidden");
        assert!(t.contains("Display mode"), "active tab chip");
        assert!(t.contains("Language"), "inactive tab chip still shown");
    }

    #[test]
    fn ask_card_switching_question_swaps_the_visible_body() {
        let mut a = multi_question_ask();
        a.question = 1;
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains("Q2?"), "now-active question body: {t}");
        assert!(!t.contains("Q1?"), "no-longer-active body must be hidden");
    }

    #[test]
    fn ask_card_tabs_mark_answered_questions() {
        let mut a = multi_question_ask();
        a.picked[0] = vec!["A".into()];
        a.question = 1; // move off question 0 so its ✓ isn't just "active"
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains('✓'), "answered question gets a check: {t}");
        assert!(t.contains('○'), "unanswered question keeps the circle glyph");
    }

    #[test]
    fn ask_card_single_question_has_no_tab_strip() {
        // A single-question ask shouldn't grow tab-strip chrome it
        // doesn't need — it falls back to a plain header chip.
        let t = text_of(&ask_card(&ask(), 100));
        assert!(!t.contains("Display mode"), "no tab strip for a lone question");
    }

    #[test]
    fn ask_card_footer_omits_tab_hint_for_single_question() {
        let t = text_of(&ask_card(&ask(), 100));
        // Single-question footer: "Enter to select  ·  ↑↓ to navigate  ·  Esc to cancel"
        assert!(!t.contains("Tab"), "no Tab hint in single-question footer: {t}");
    }

    #[test]
    fn ask_card_free_text_mode_is_inline() {
        let mut a = ask();
        a.text_mode = true;
        a.text = "sqlite but embedded".into();
        let t = text_of(&ask_card(&a, 100));
        assert!(t.contains("❯ t  sqlite but embedded"));
    }
}