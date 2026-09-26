//! The model's reasoning ("thinking") as a transcript section.
//!
//! Two shapes, matching how other agent UIs surface thinking:
//!
//! ```text
//!   ✻ Thinking… 4s                      ← live, in the pane while it streams
//!     ┊ …the last few lines of what
//!     ┊ the model is reasoning about
//!
//!   ✻ Thought for 4s · ctrl+t to expand ← settled, collapsed (default)
//!
//!   ✻ Thought for 4s                    ← settled, expanded (ctrl+t)
//!     ┊ the full reasoning, wrapped
//! ```
//!
//! Settled blocks go into real terminal scrollback, which can't be
//! re-rendered, so `ctrl+t` decides how the *next* blocks settle (and
//! how much of the live one the pane shows) rather than folding
//! history in place.

use std::time::Duration;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{DIM, HAIRLINE, MUTED, SALMON};

/// Rows of the live tail the pane shows while collapsed — enough to
/// see the model's train of thought without pushing the composer away.
const LIVE_TAIL_ROWS: usize = 4;

#[derive(Clone, Debug)]
pub struct ThinkingView<'a> {
    pub text: &'a str,
    /// Time spent thinking so far (live) or in total (settled). `None`
    /// for thinking replayed from a resumed session — the duration
    /// wasn't recorded.
    pub elapsed: Option<Duration>,
    /// Still streaming — the pane renders it, scrollback doesn't yet.
    pub live: bool,
    /// Show the full text instead of the header alone (settled) or the
    /// last few rows (live).
    pub expanded: bool,
}

pub fn render(v: &ThinkingView<'_>, width: u16) -> Vec<Line<'static>> {
    let dur = v.elapsed.map(|d| {
        let secs = d.as_secs_f64();
        if secs < 1.0 {
            "<1s".to_owned()
        } else if secs < 60.0 {
            format!("{secs:.0}s")
        } else {
            format!("{}m {}s", secs as u64 / 60, secs as u64 % 60)
        }
    });
    let star = Span::styled("✻ ", Style::default().fg(SALMON()));
    let mut header = vec![Span::raw("  "), star];
    if v.live {
        header.push(Span::styled(
            "Thinking…",
            Style::default().fg(MUTED()).add_modifier(Modifier::ITALIC),
        ));
        if let Some(dur) = &dur {
            header.push(Span::styled(format!(" {dur}"), Style::default().fg(DIM())));
        }
    } else {
        header.push(Span::styled(
            match &dur {
                Some(dur) => format!("Thought for {dur}"),
                None => "Thought".to_owned(),
            },
            Style::default().fg(MUTED()),
        ));
        if !v.expanded && !v.text.trim().is_empty() {
            header.push(Span::styled(
                " · ctrl+t to expand",
                Style::default().fg(DIM()),
            ));
        }
    }
    let mut out = vec![Line::from(header)];

    let body = wrap(v.text.trim(), (width as usize).saturating_sub(8).max(20));
    let shown: &[String] = match (v.live, v.expanded) {
        (_, true) => &body,
        (true, false) => &body[body.len().saturating_sub(LIVE_TAIL_ROWS)..],
        (false, false) => &[],
    };
    let gutter = Style::default().fg(HAIRLINE());
    let text = Style::default().fg(MUTED()).add_modifier(Modifier::ITALIC);
    for row in shown {
        out.push(Line::from(vec![
            Span::styled("    ┊ ", gutter),
            Span::styled(row.clone(), text),
        ]));
    }
    out
}

/// Word-wrap to `width` columns, keeping the model's own line breaks.
/// Blank lines collapse — thinking is dense prose, and gaps in a dim
/// gutter read as rendering glitches.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
    {
        let mut line = String::new();
        let mut cols = 0;
        for word in para.split_whitespace() {
            let w = word.chars().count();
            if cols > 0 && cols + 1 + w > width {
                out.push(std::mem::take(&mut line));
                cols = 0;
            }
            if cols > 0 {
                line.push(' ');
                cols += 1;
            }
            line.push_str(word);
            cols += w;
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn settled_collapsed_is_one_header_row() {
        let v = ThinkingView {
            text: "first\nsecond",
            elapsed: Some(Duration::from_secs(4)),
            live: false,
            expanded: false,
        };
        let rows = text_of(&render(&v, 80));
        assert_eq!(rows, ["  ✻ Thought for 4s · ctrl+t to expand"]);
    }

    #[test]
    fn settled_expanded_shows_every_line() {
        let v = ThinkingView {
            text: "first\n\nsecond",
            elapsed: Some(Duration::from_millis(300)),
            live: false,
            expanded: true,
        };
        let rows = text_of(&render(&v, 80));
        assert_eq!(rows, ["  ✻ Thought for <1s", "    ┊ first", "    ┊ second"]);
    }

    #[test]
    fn live_collapsed_shows_only_the_tail() {
        let text = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let v = ThinkingView {
            text: &text,
            elapsed: Some(Duration::from_secs(2)),
            live: true,
            expanded: false,
        };
        let rows = text_of(&render(&v, 80));
        assert_eq!(rows[0], "  ✻ Thinking… 2s");
        assert_eq!(rows.len(), 1 + LIVE_TAIL_ROWS);
        assert_eq!(rows.last().unwrap(), "    ┊ line 10");
    }

    #[test]
    fn wrap_breaks_long_lines_on_words() {
        assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
    }
}
