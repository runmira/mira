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

use super::{CREAM, MUTED, SALMON, truncate};
use crate::tui::state::{PendingAsk, PendingPlan};

/* ---------- shared primitives ---------- */

const CARD_WIDTH: usize = 54;

fn top_border(label: &str, accent: Color, icon: &str) -> Line<'static> {
    let label = format!(" {icon} {label} ");
    let remaining = CARD_WIDTH.saturating_sub(label.chars().count() + 3);
    Line::from(vec![
        Span::styled("╭─", Style::default().fg(accent).bold()),
        Span::styled(label, Style::default().fg(accent).bold()),
        Span::styled("─".repeat(remaining), Style::default().fg(accent)),
        Span::styled("╮", Style::default().fg(accent).bold()),
    ])
}

fn bottom_border(accent: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(CARD_WIDTH.saturating_sub(2))),
        Style::default().fg(accent),
    ))
}

fn divider(accent: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", "─".repeat(CARD_WIDTH.saturating_sub(4))),
        Style::default().fg(MUTED()),
    ))
}

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

fn detail_line(text: String, accent: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("     └─ ", Style::default().fg(accent)),
        Span::styled(text, Style::default().fg(MUTED()).italic()),
    ])
}

fn progress_line(done: usize, total: usize, accent: Color) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", Style::default())];
    for i in 0..total {
        spans.push(Span::styled(
            if i < done { "●" } else { "○" },
            Style::default().fg(if i < done { accent } else { MUTED() }),
        ));
        if i + 1 < total {
            spans.push(Span::raw(" "));
        }
    }
    spans.push(Span::styled(
        format!("   {done} / {total} complete"),
        Style::default().fg(MUTED()),
    ));
    Line::from(spans)
}

/// One friendly option/checklist row.
///
/// Focus gets a subtle filled background instead of relying only on a cursor.
/// This makes keyboard navigation immediately visible in a dense terminal.
#[allow(clippy::too_many_arguments)]
fn choice_row(
    idx: usize,
    label: &str,
    focused: bool,
    checkbox: Option<bool>,
    picked: bool,
    recommended: bool,
    accent: Color,
) -> Line<'static> {
    let marker = match checkbox {
        Some(true) => "✓",
        Some(false) => "○",
        None if picked => "◆",
        None => "◇",
    };

    let marker_style = if focused {
        Style::default().fg(accent).bold()
    } else if checkbox == Some(true) || picked {
        Style::default().fg(Color::Green).bold()
    } else {
        Style::default().fg(MUTED())
    };

    let mut spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(marker, marker_style),
        Span::styled(
            format!("  {label}"),
            if focused {
                Style::default().fg(CREAM()).bold().bg(Color::Rgb(35, 35, 43))
            } else if checkbox == Some(true) || picked {
                Style::default().fg(Color::Green).bold()
            } else {
                Style::default().fg(CREAM())
            },
        ),
    ];

    if recommended {
        spans.push(Span::styled(
            "  ✦ recommended",
            Style::default().fg(Color::Green).italic(),
        ));
    }

    // Keep the index available as a subtle keyboard affordance without making
    // every row look like a numbered list.
    if focused {
        spans.insert(
            1,
            Span::styled(
                format!("{idx}. "),
                Style::default().fg(accent).bold(),
            ),
        );
    }

    Line::from(spans)
}

/* ---------- plan card ---------- */

pub(crate) fn plan_card(p: &PendingPlan) -> Vec<Line<'static>> {
    let accent = SALMON();
    let mut out = vec![top_border("PLAN", accent, "✦"), blank()];

    if !p.proposal.title.is_empty() {
        out.push(Line::from(Span::styled(
            format!("  {}", p.proposal.title),
            Style::default().fg(CREAM()).bold(),
        )));
        out.push(blank());
    }

    let done = p.checked.iter().filter(|checked| **checked).count();
    out.push(progress_line(done, p.proposal.steps.len(), accent));
    out.push(blank());

    for (i, step) in p.proposal.steps.iter().enumerate() {
        let focused = i == p.focus;
        out.push(choice_row(
            i + 1,
            &step.description,
            focused,
            Some(p.checked[i]),
            false,
            false,
            accent,
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
    out.push(divider(accent));
    out.push(footer_hints(&[
        ("Space", "toggle"),
        ("↑↓", "move"),
        ("Enter", "approve"),
        ("Esc", "cancel"),
    ]));
    out.push(bottom_border(accent));
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

pub(crate) fn ask_card(a: &PendingAsk) -> Vec<Line<'static>> {
    let accent = Color::Cyan;
    let mut out = vec![top_border("ASK MIRA", accent, "?"), blank()];

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
        let checkbox = q.multi_select.then_some(picked);
        out.push(choice_row(
            oi + 1,
            &opt.label,
            focused,
            checkbox,
            picked && !q.multi_select,
            opt.recommended,
            accent,
        ));

        if focused {
            if let Some(desc) = &opt.description {
                if !desc.trim().is_empty() {
                    out.push(detail_line(truncate(desc, 100), accent));
                }
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
                "tell Mira what to do differently",
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
    out.push(divider(accent));
    let hint = if a.proposal.questions.len() > 1 {
        vec![
            ("1–4", "choose"),
            ("↑↓", "move"),
            ("Tab", "next"),
            ("Enter", "send"),
        ]
    } else {
        vec![("1–4", "choose"), ("↑↓", "move"), ("Enter", "send"), ("Esc", "cancel")]
    };
    out.push(footer_hints(&hint));
    out.push(bottom_border(accent));
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
        let t = text_of(&plan_card(&p));
        assert!(t.contains("✦ PLAN"), "{t}");
        assert!(t.contains("Refactor renderer"), "title");
        assert!(t.contains("✓  Extract layout module"));
        assert!(t.contains("○  Move scroll into state"));
        assert!(t.contains("1. ✓"), "focused step marker");
        assert!(t.contains("single source for scroll math"), "focused why");
        assert!(t.contains("Space  toggle"), "footer hints");
        assert!(t.contains("Enter  approve"), "footer hints");
    }

    #[test]
    fn plan_card_hides_why_for_unfocused_step() {
        // Focused step (0) has a `why`; step 1 doesn't, and shouldn't
        // be expected to show anything even when it becomes focused.
        let mut p = plan();
        p.focus = 1;
        let t = text_of(&plan_card(&p));
        assert!(!t.contains("single source for scroll math"));
    }

    #[test]
    fn plan_card_reflects_toggles_and_focus() {
        let mut p = plan();
        p.checked[0] = !p.checked[0];
        p.focus = 1;
        let t = text_of(&plan_card(&p));
        assert!(t.contains("○  Extract layout module"));
        assert!(t.contains("○  Move scroll into state"));
    }

    #[test]
    fn ask_card_shows_chip_question_and_hotkeys() {
        let t = text_of(&ask_card(&ask()));
        assert!(t.contains("? ASK MIRA"), "{t}");
        assert!(t.contains("BACKEND"), "header");
        assert!(t.contains("Which storage backend?"));
        assert!(t.contains("SQLite"), "option 1");
        assert!(t.contains("Postgres"), "option 2");
        assert!(t.contains("✦ recommended"), "badge");
        assert!(t.contains("t  tell Mira what to do differently"), "free-text affordance");
        assert!(t.contains("Enter  send"), "hints");
        assert!(t.contains("Esc  cancel"), "hints");
    }

    #[test]
    fn ask_card_shows_focused_option_description_only() {
        let t = text_of(&ask_card(&ask())); // option 0 focused by default
        assert!(t.contains("zero-config, single file"), "focused desc");
    }

    #[test]
    fn ask_card_hides_description_for_unfocused_option() {
        let mut a = ask();
        a.option = 1; // Postgres, which has no description
        let t = text_of(&ask_card(&a));
        assert!(!t.contains("zero-config, single file"));
    }

    #[test]
    fn ask_card_reflects_pick_and_custom() {
        let mut a = ask();
        a.picked[0] = vec!["Postgres".into()];
        a.custom[0] = Some("but keep the file format".into());
        let t = text_of(&ask_card(&a));
        assert!(t.contains("◆  Postgres"), "single-select pick: {t}");
        assert!(t.contains("you said: but keep the file format"));
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
        let t = text_of(&ask_card(&a));
        assert!(t.contains("1. ✓  A"), "{t}");
        assert!(t.contains("○  B"));
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
        let t = text_of(&ask_card(&a));
        assert!(t.contains("Q1?"), "active question body: {t}");
        assert!(!t.contains("Q2?"), "inactive question body must be hidden");
        assert!(t.contains("Display mode"), "active tab chip");
        assert!(t.contains("Language"), "inactive tab chip still shown");
    }

    #[test]
    fn ask_card_switching_question_swaps_the_visible_body() {
        let mut a = multi_question_ask();
        a.question = 1;
        let t = text_of(&ask_card(&a));
        assert!(t.contains("Q2?"), "now-active question body: {t}");
        assert!(!t.contains("Q1?"), "no-longer-active body must be hidden");
    }

    #[test]
    fn ask_card_tabs_mark_answered_questions() {
        let mut a = multi_question_ask();
        a.picked[0] = vec!["A".into()];
        a.question = 1; // move off question 0 so its ✓ isn't just "active"
        let t = text_of(&ask_card(&a));
        assert!(t.contains('✓'), "answered question gets a check: {t}");
        assert!(t.contains('○'), "unanswered question keeps the circle glyph");
    }

    #[test]
    fn ask_card_single_question_has_no_tab_strip() {
        // A single-question ask shouldn't grow tab-strip chrome it
        // doesn't need — it falls back to a plain header chip.
        let t = text_of(&ask_card(&ask()));
        assert!(!t.contains("Display mode"), "no tab strip for a lone question");
    }

    #[test]
    fn ask_card_footer_omits_tab_hint_for_single_question() {
        let t = text_of(&ask_card(&ask()));
        assert!(!t.contains("Tab"));
    }

    #[test]
    fn ask_card_free_text_mode_is_inline() {
        let mut a = ask();
        a.text_mode = true;
        a.text = "sqlite but embedded".into();
        let t = text_of(&ask_card(&a));
        assert!(t.contains("❯ t  sqlite but embedded"));
    }
}