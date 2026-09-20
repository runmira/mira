//! Message components — user prompts, assistant replies, warnings, info.
//!
//! Owns *how one message looks*: the `> user` prompt treatment, the
//! markdown pipeline for assistant text, the plan-card conversion, the
//! plan-mode gutter, and the streaming cursor. Nothing here knows where
//! the lines land on screen.

use ratatui::style::{Stylize, Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{CREAM, MUTED, SALMON, highlight_line, line_is_empty};
use crate::tui::markdown;

/// Structured startup banner — the first transcript entry on a fresh
/// session. Three quiet lines + a tip, Mira-flavored:
///
///     ℳ mira v0.3.6
///     sonnet-4.5 · auto · ~/Desktop/coding_agent
///
///     ✳ type while mira works — enter queues the next turn
pub(crate) struct WelcomeView<'a> {
    pub model: &'a str,
    pub mode: mira_policy::Mode,
    pub cwd: &'a str,
    pub version: &'static str,
    pub tip: &'a str,
}

pub(crate) fn welcome_lines(w: &WelcomeView<'_>) -> Vec<Line<'static>> {
    let dim = Style::default().fg(MUTED());
    vec![
        Line::from(vec![
            Span::styled("ℳ ", Style::default().fg(SALMON()).bold()),
            Span::styled("mira ", Style::default().fg(CREAM()).bold()),
            Span::styled(format!("v{}", w.version), dim),
        ]),
        Line::from(vec![
            Span::styled(w.model.to_owned(), Style::default().fg(CREAM())),
            Span::styled(" · ", dim),
            Span::styled(w.mode.as_str(), super::mode_style(w.mode)),
            Span::styled(" · ", dim),
            Span::styled(w.cwd.to_owned(), Style::default().fg(MUTED())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("✳ ", Style::default().fg(SALMON())),
            Span::styled(w.tip.to_owned(), Style::default().fg(CREAM())),
        ]),
    ]
}

/// Claude-style user turn: `> ` prompt in salmon, message in cream on
/// its own row(s). Multi-line messages (Ctrl+J) get the `> ` marker on
/// the first row only and a hanging indent on the rest so paragraphs
/// read cleanly.
pub(crate) fn user_lines(s: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut first = true;
    for line in s.lines() {
        let mark = if first { "> " } else { "  " };
        out.push(Line::from(vec![
            Span::styled(mark, Style::default().fg(SALMON()).bold()),
            Span::styled(line.to_owned(), Style::default().fg(CREAM())),
        ]));
        first = false;
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(
            "> ",
            Style::default().fg(SALMON()).bold(),
        )));
    }
    out
}

/// Assistant reply rendering. Order of concerns:
///
/// 1. In-flight (streaming) replies render via the streaming path —
///    markdown plus the soft `▍` cursor, no plan-card conversion
///    (that flickers as list steps stream in mid-list).
/// 2. Search mode flattens to plain text so the yellow highlight
///    overlay is visible instead of fighting markdown styling.
/// 3. Otherwise: a "Plan:" reply renders as a bordered card, anything
///    else through the markdown renderer.
/// 4. Plan mode prepends the blue gutter so it's obvious the model is
///    brainstorming, not executing.
pub(crate) fn assistant_lines(
    text: &str,
    streaming: bool,
    plan: bool,
    query: &str,
    focused: bool,
) -> Vec<Line<'static>> {
    let mut out = if streaming {
        let mut ls = markdown::render(text);
        append_streaming_cursor(&mut ls);
        ls
    } else if query.is_empty() {
        match try_parse_plan(text) {
            Some(plan_doc) => render_plan_card(&plan_doc),
            None => markdown::render(text),
        }
    } else {
        // In search mode, render Assistant entries as plain text so the
        // highlight overlay is visible. Markdown styling and yellow-on-
        // black highlights would fight each other otherwise.
        text.lines()
            .map(|l| Line::from(Span::raw(l.to_owned())))
            .map(|line| highlight_line(line, query, focused))
            .collect()
    };
    if plan {
        out = plan_gutter(out);
    }
    out
}

pub(crate) fn warning_lines(s: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("⚠ ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(s.to_owned(), Style::default().fg(Color::Yellow)),
    ])]
}

pub(crate) fn info_lines(s: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        s.to_owned(),
        Style::default().fg(MUTED()).italic(),
    ))]
}

/// Append a subtle `▍` cursor to the last non-empty line of a
/// streaming assistant entry. Uses the salmon accent + bold so the
/// eye can see progress even during a silent provider pause. Empty
/// input (assistant just started) gets a single cursor row so the
/// user isn't looking at nothing.
fn append_streaming_cursor(lines: &mut Vec<Line<'static>>) {
    let cursor = Span::styled(
        "▍",
        Style::default().fg(SALMON()).add_modifier(Modifier::BOLD),
    );
    if let Some(last) = lines.iter_mut().rev().find(|l| !line_is_empty(l)) {
        last.spans.push(Span::raw(" "));
        last.spans.push(cursor);
        return;
    }
    lines.push(Line::from(cursor));
}

/// Prepend a soft blue `│ ` gutter to every line — used to mark
/// assistant text as "planning" output when the session is in plan
/// mode. Stateless, so tests + future callers can reuse it.
fn plan_gutter(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let gutter_style = Style::default().fg(Color::LightBlue);
    lines
        .into_iter()
        .map(|l| {
            let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", gutter_style)];
            spans.extend(l.spans);
            Line::from(spans)
        })
        .collect()
}

/// A model reply that reads as a plan proposal. Detected by a
/// heading line starting with `# Plan`, `## Plan`, or a first line of
/// `Plan:` (case-insensitive), followed by a numbered or bulleted
/// list. Returns the parsed shape so [`render_plan_card`] can lay it
/// out as a bordered card.
struct Plan {
    title: String,
    steps: Vec<String>,
}

fn try_parse_plan(s: &str) -> Option<Plan> {
    let mut lines = s.lines().peekable();
    let first = lines.next()?.trim();
    // Accept `# Plan …`, `## Plan …`, or `Plan: …` / `Plan …` / plain `Plan`
    let title = if let Some(rest) = first.strip_prefix("# Plan") {
        rest.trim_start_matches(':').trim().to_owned()
    } else if let Some(rest) = first.strip_prefix("## Plan") {
        rest.trim_start_matches(':').trim().to_owned()
    } else if first.eq_ignore_ascii_case("plan") {
        String::new()
    } else {
        // Accept `Plan:`, `Plan — foo`. Anything after the colon is
        // the title; a bare non-"plan" first line isn't a plan.
        let lowered = first.to_ascii_lowercase();
        let rest = lowered.strip_prefix("plan")?;
        let rest = rest.trim_start_matches(':').trim();
        if rest.is_empty() {
            return None;
        }
        rest.to_owned()
    };

    // Collect numbered- or bulleted-list steps. Blank lines end the
    // step run; anything not-a-list-item after that ends parsing.
    let mut steps: Vec<String> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() && steps.is_empty() {
            continue; // skip blanks before the list starts
        }
        if line.is_empty() {
            break; // stop at the blank line after the list
        }
        // `1. do X`, `12. do Y`, `- do X`, `* do Y`
        let step = if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            rest.to_owned()
        } else {
            let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                break;
            }
            let after = &line[digits.len()..];
            if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
                rest.to_owned()
            } else {
                break;
            }
        };
        steps.push(step);
    }

    if steps.len() < 2 {
        return None; // one-step "plan" is just a sentence, don't box it
    }
    Some(Plan { title, steps })
}

fn render_plan_card(plan: &Plan) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let bar_style = Style::default().fg(SALMON());
    // Top rule with inline title chip
    let mut header: Vec<Span<'static>> = vec![
        Span::styled("╭─ ", bar_style),
        Span::styled("plan", Style::default().fg(SALMON()).bold()),
    ];
    if !plan.title.is_empty() {
        header.push(Span::styled("  ", Style::default()));
        header.push(Span::styled(
            plan.title.clone(),
            Style::default().fg(CREAM()).bold(),
        ));
    }
    out.push(Line::from(header));

    for step in &plan.steps {
        out.push(Line::from(vec![
            Span::styled("│  ", bar_style),
            Span::styled("[ ] ", Style::default().fg(MUTED())),
            Span::styled(step.clone(), Style::default().fg(CREAM())),
        ]));
    }
    // Footer hint — no interaction yet, but signal the card affordance.
    out.push(Line::from(vec![
        Span::styled("╰─ ", bar_style),
        Span::styled(
            "review the plan · say `go` / `edit` / `cancel`",
            Style::default().fg(MUTED()).italic(),
        ),
    ]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_doc_becomes_card() {
        let ls = assistant_lines(
            "## Plan Ship it\n1. write code\n2. run tests\n",
            false,
            false,
            "",
            false,
        );
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("plan"));
        assert!(joined.contains("write code"));
        assert!(joined.contains("run tests"));
    }

    #[test]
    fn streaming_suppresses_plan_card_and_adds_cursor() {
        let ls = assistant_lines("## Plan Ship it\n1. a\n2. b\n", true, false, "", false);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(!joined.contains("[ ]"));
        assert!(joined.contains("▍"));
    }

    #[test]
    fn plan_mode_adds_gutter() {
        let ls = assistant_lines("hello\nworld", false, true, "", false);
        assert!(ls.len() >= 2);
        assert_eq!(ls[0].spans[0].content, "│ ");
    }

    #[test]
    fn search_mode_highlights_plain_text() {
        let ls = assistant_lines("the quick brown fox", false, false, "QUICK", true);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("quick"));
    }

    #[test]
    fn parse_plan_rejects_single_step() {
        assert!(try_parse_plan("Plan\n1. just one thing").is_none());
        assert!(try_parse_plan("just a normal reply").is_none());
    }

    #[test]
    fn user_marker_hangs_indent_on_multiline() {
        let ls = user_lines("line one\nline two");
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].spans[0].content, "> ");
        assert_eq!(ls[1].spans[0].content, "  ");
    }
}
