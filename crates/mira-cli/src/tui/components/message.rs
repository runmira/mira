//! Message components — user prompts, assistant replies, warnings, info.
//!
//! Owns *how one message looks*: the `> user` prompt treatment, the
//! markdown pipeline for assistant text, the plan-card conversion, the
//! plan-mode gutter, and the streaming cursor. Nothing here knows where
//! the lines land on screen.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use super::{highlight_line, line_is_empty, CREAM, MUTED, SALMON, USER_WASH};
use crate::tui::markdown;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Session banner — the first transcript entry on a fresh session,
/// shown once before the message stream. The viewport carries no
/// persistent header, so this is the session's identity record:
///
///     ℳ mira v0.3.6
///     model     sonnet-4.5
///     provider  anthropic
///     cwd       ~/Desktop/coding_agent ⎇ main
///     mode      auto
///     skills    review · plan · +2 more
///
///     /help for commands and keys · shift+tab changes mode · esc esc quits
///     ✨ Tip: type while mira works — enter queues the next turn
pub(crate) struct WelcomeView<'a> {
    pub model: &'a str,
    pub provider: &'a str,
    pub mode: mira_policy::Mode,
    pub cwd: &'a str,
    pub branch: Option<&'a str>,
    pub skills: &'a [String],
    pub version: &'static str,
    pub tip: &'a str,
}

/// Skills shown before the `+N more` cap.
const BANNER_SKILLS_SHOWN: usize = 8;

/// 9×10 pixel-art ℳ rendered as 5 half-block rows using `▀`/`▄`.
/// Each pixel pair (top+bottom) is packed into one character cell;
/// a gradient runs from cream at the top to salmon at the bottom.
fn mark_lines() -> Vec<Line<'static>> {
    // Lit columns per pixel row (0-indexed, 9 columns wide).
    // Bold M: 2-pixel-wide outer strokes, V diagonals in rows 1-3.
    const ON: [&[usize]; 10] = [
        &[0, 1, 7, 8],           // top corners
        &[0, 1, 2, 6, 7, 8],     // diagonals step 1
        &[0, 1, 3, 5, 7, 8],     // diagonals step 2
        &[0, 1, 4, 7, 8],        // V base
        &[0, 1, 7, 8],           // outer legs
        &[0, 1, 7, 8],
        &[0, 1, 7, 8],
        &[0, 1, 7, 8],
        &[0, 1, 7, 8],
        &[],                     // row 9 pairs with row 8 above via ▀
    ];
    let (sr, sg, sb) = if let Color::Rgb(r, g, b) = SALMON() {
        (r as i32, g as i32, b as i32)
    } else {
        (232, 156, 104)
    };
    let (cr, cg, cb) = if let Color::Rgb(r, g, b) = CREAM() {
        (r as i32, g as i32, b as i32)
    } else {
        (240, 235, 226)
    };
    let row_color = |row: usize| -> Color {
        let t = row as i32;
        Color::Rgb(
            (cr + (sr - cr) * t / 9).clamp(0, 255) as u8,
            (cg + (sg - cg) * t / 9).clamp(0, 255) as u8,
            (cb + (sb - cb) * t / 9).clamp(0, 255) as u8,
        )
    };
    let lit = |row: usize, col: usize| row < 10 && ON[row].contains(&col);
    (0..10usize)
        .step_by(2)
        .map(|top| {
            let bot = top + 1;
            let tc = row_color(top);
            let bc = row_color(bot);
            let spans: Vec<Span<'static>> = (0..9)
                .map(|col| match (lit(top, col), lit(bot, col)) {
                    (true, true) => Span::styled("▀", Style::default().fg(tc).bg(bc)),
                    (true, false) => Span::styled("▀", Style::default().fg(tc)),
                    (false, true) => Span::styled("▄", Style::default().fg(bc)),
                    (false, false) => Span::raw(" "),
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

/// One `label    value…` banner row.
fn banner_row(label: &str, mut value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("{label:<8}"),
            Style::default().fg(MUTED()),
        ),
        Span::styled("  ", Style::default()),
    ];
    spans.append(&mut value);
    Line::from(spans)
}

pub(crate) fn welcome_lines(w: &WelcomeView<'_>) -> Vec<Line<'static>> {
    let dim = Style::default().fg(MUTED());
    let cream = Style::default().fg(CREAM());
    // Pixel-art ℳ logo (5 half-block rows) + text caption below.
    let mut out = vec![Line::from("")];
    for mut logo_line in mark_lines() {
        logo_line.spans.insert(0, Span::raw("  "));
        out.push(logo_line);
    }
    out.push(Line::from(vec![
        Span::styled("  ℳ mira", Style::default().fg(SALMON()).bold()),
        Span::styled(format!("  v{}", w.version), dim),
    ]));
    out.push(Line::from(""));
    // Two-column field list — labels padded so values align.
    out.push(banner_row("model", vec![Span::styled(
        w.model.to_owned(),
        cream,
    )]));
    out.push(banner_row("provider", vec![Span::styled(
        w.provider.to_owned(),
        cream,
    )]));
    if let Some(branch) = w.branch {
        out.push(banner_row(
            "cwd",
            vec![
                Span::styled(w.cwd.to_owned(), cream),
                Span::styled(format!("  ⎇ {branch}"), dim),
            ],
        ));
    } else {
        out.push(banner_row("cwd", vec![Span::styled(
            w.cwd.to_owned(),
            cream,
        )]));
    }
    out.push(banner_row("mode", vec![Span::styled(
        w.mode.as_str(),
        super::mode_style(w.mode),
    )]));
    if w.skills.is_empty() {
        out.push(banner_row("skills", vec![Span::styled("none", dim)]));
    } else {
        let shown: Vec<&str> = w.skills.iter().take(BANNER_SKILLS_SHOWN).map(String::as_str).collect();
        let mut text = shown.join(" · ");
        if w.skills.len() > BANNER_SKILLS_SHOWN {
            text.push_str(&format!("  +{} more", w.skills.len() - BANNER_SKILLS_SHOWN));
        }
        out.push(banner_row("skills", vec![Span::styled(text, cream)]));
    }
    out.push(Line::from(""));
    out.push(Line::from(Span::styled(
        "/help for commands and keys · shift+tab changes mode · esc esc quits",
        dim,
    )));
    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("✨ Tip: ", Style::default().fg(SALMON())),
        Span::styled(w.tip.to_owned(), cream),
    ]));
    out
}

/// Your submitted turn: `> ` prompt in salmon, message in cream, over
/// a full-width semi-subtle wash so your words read apart from the
/// agent's at a glance. Rows pad to `width` (display-width aware) so
/// the wash spans edge to edge. Long lines are pre-wrapped here so every
/// visual row gets the wash — without this the ratatui Wrap pass creates
/// continuation rows that have no background span at all.
/// Multi-line messages (Ctrl+J) get the `> ` marker on the first row
/// only and a hanging indent on the rest so paragraphs read cleanly.
pub(crate) fn user_lines(s: &str, width: u16) -> Vec<Line<'static>> {
    let width = width.max(1) as usize;
    // 2 display cols for "  " / "> " prefix; clamp to at least 1 so zero-
    // width terminals don't loop forever.
    let content_width = width.saturating_sub(2).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut first_logical = true;
    for logical_line in s.lines() {
        let segments = wrap_to_width(logical_line, content_width);
        let mut first_visual = true;
        for seg in &segments {
            let mark = if first_logical && first_visual { "> " } else { "  " };
            out.push(wash_line(
                vec![
                    Span::styled(
                        mark,
                        Style::default().fg(SALMON()).bold().bg(USER_WASH()),
                    ),
                    Span::styled(
                        seg.clone(),
                        Style::default().fg(CREAM()).bg(USER_WASH()),
                    ),
                ],
                width,
            ));
            first_visual = false;
        }
        if segments.is_empty() {
            let mark = if first_logical { "> " } else { "  " };
            out.push(wash_line(
                vec![Span::styled(
                    mark,
                    Style::default().fg(SALMON()).bold().bg(USER_WASH()),
                )],
                width,
            ));
        }
        first_logical = false;
    }
    if out.is_empty() {
        out.push(wash_line(
            vec![Span::styled(
                "> ",
                Style::default().fg(SALMON()).bold().bg(USER_WASH()),
            )],
            width,
        ));
    }
    out
}

/// Split `s` into segments each at most `max_width` display columns wide.
fn wrap_to_width(s: &str, max_width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if current_w + cw > max_width && !current.is_empty() {
            rows.push(current.clone());
            current.clear();
            current_w = 0;
        }
        current.push(ch);
        current_w += cw;
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

/// Pad a wash row with trailing spaces to `width` display columns so
/// the background spans edge to edge.
fn wash_line(mut spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(USER_WASH()),
        ));
    }
    Line::from(spans)
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
    let body = if streaming {
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
    // Prefix the `●` inline on the first non-empty line so the reply
    // reads as `● Hi! …` on one row, matching the tool-call groups.
    // A separate header row would waste a scrollback line.
    let mut out = prefix_assistant_dot(body);
    if plan {
        out = plan_gutter(out);
    }
    out
}

/// The `● ` marker prepended to the first content line of an assistant
/// reply. Kept as a public helper so the streaming path in the event
/// loop can build a one-line block with the marker inline the same way.
pub(crate) fn assistant_dot_span() -> Span<'static> {
    Span::styled(
        "● ",
        Style::default().fg(SALMON()).add_modifier(Modifier::BOLD),
    )
}

/// Inject the `●` marker as the leading span of `lines`'s first
/// non-empty line. Leading blank lines are dropped — a reply that opens
/// on a blank shouldn't waste a scrollback row on it.
pub(crate) fn prefix_assistant_dot(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines
        .first()
        .map(line_is_empty)
        .unwrap_or(false)
    {
        lines.remove(0);
    }
    if let Some(first) = lines.first_mut() {
        first.spans.insert(0, assistant_dot_span());
    } else {
        // Reply with no content yet — emit a lone marker row so the
        // user sees the dot immediately (streaming placeholder).
        lines.push(Line::from(assistant_dot_span()));
    }
    lines
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
            if let Some(rest) = after
                .strip_prefix(". ")
                .or_else(|| after.strip_prefix(") "))
            {
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
        let ls = user_lines("line one\nline two", 40);
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].spans[0].content, "> ");
        assert_eq!(ls[1].spans[0].content, "  ");
    }

    #[test]
    fn user_rows_wash_full_width() {
        let ls = user_lines("hi", 20);
        assert_eq!(ls.len(), 1);
        // Every span carries the lighter user-wash background…
        for sp in &ls[0].spans {
            assert_eq!(sp.style.bg, Some(USER_WASH()));
        }
        // …and the row pads to the full width (display columns).
        let cols: usize = ls[0].spans.iter().map(|sp| sp.content.width()).sum();
        assert_eq!(cols, 20, "{:?}", ls[0].spans);
    }
}
