//! Approval card — the inline prompt rendered below the tool call that
//! triggered it. The single highest-stakes surface in the UI, so it
//! gets its own component.
//!
//! Design goals:
//!
//! - **One-glance clarity**: `⚠  Bash  ·  approval required` — no
//!   raw JSON dumps, no "approve X?" as a header line.
//! - **Show the meaningful thing, not the wrapper**: extract the
//!   command / path from the tool call and format it like a code
//!   snippet, not a `{"command":"..."}` blob.
//! - **Picker rows, not badges**: the three decisions render in the
//!   shared `▸ N. label` language (digits + arrows + Enter, with the
//!   `y`/`a`/`n` shortcuts dimmed at each row's end).
//! - **Stacked, not dropped**: further approvals queue behind the head
//!   with a `+N more` count instead of replacing it.

use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::tool_call::summarize_tool;
use super::tool_result;
use super::{CREAM, DIM, MUTED, SALMON};

/// Borrows of the pending approval request, taken at block-build time.
/// `focus` selects among the three decision rows (0 = allow once,
/// 1 = always this session, 2 = deny); `queued` counts further
/// approvals waiting behind this one.
pub(crate) struct ApprovalView<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub preview: Option<&'a mira_tools::DiffPreview>,
    pub focus: usize,
    pub queued: usize,
}

pub(crate) fn render(v: &ApprovalView<'_>, width: u16) -> Vec<Line<'static>> {
    let (friendly, primary) = summarize_tool(v.name, v.args);
    let mut out: Vec<Line<'static>> = Vec::new();

    // Header: clean tool-family label, no warning glyph.
    // Queued count appended when stacked so the user knows what's next.
    let mut header: Vec<Span<'static>> = vec![Span::styled(
        format!("{friendly} command"),
        Style::default().fg(SALMON()).bold(),
    )];
    if let Some(preview) = v.preview {
        let kind_label = match preview.kind {
            mira_tools::DiffKind::Edit => "edit",
            mira_tools::DiffKind::Overwrite => "overwrite",
            mira_tools::DiffKind::Create => "create",
        };
        header.push(Span::styled("  ·  ", Style::default().fg(DIM())));
        header.push(Span::styled(
            preview.path.clone(),
            Style::default().fg(CREAM()).bold(),
        ));
        header.push(Span::styled(
            format!("  ({kind_label})"),
            Style::default().fg(MUTED()),
        ));
    }
    if v.queued > 0 {
        header.push(Span::styled(
            format!("  ·  +{} more", v.queued),
            Style::default().fg(MUTED()).italic(),
        ));
    }
    out.push(Line::from(header));
    out.push(Line::from(""));

    // Body: full diff when available, otherwise the primary arg
    // (command / path) as a plain snippet, falling back to compact JSON.
    if let Some(preview) = v.preview {
        out.extend(tool_result::diff_preview_lines(preview, width));
    } else if !primary.is_empty() {
        out.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(primary.clone(), Style::default().fg(CREAM())),
        ]));
    } else {
        for l in pretty_args(v.args).lines().take(6) {
            out.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(l.to_owned(), Style::default().fg(CREAM())),
            ]));
        }
    }
    out.push(Line::from(""));

    // Question prompt — sets the frame for the picker below.
    out.push(Line::from(Span::styled(
        "Do you want to proceed?",
        Style::default().fg(CREAM()).bold(),
    )));
    out.push(Line::from(""));

    // Decision rows. "Always" option names the command when available
    // so the scope of the session-wide allow is unambiguous.
    let always_label = if !primary.is_empty() {
        let cmd = truncate_primary(&primary, 48);
        format!("Yes, and don't ask again for: {cmd}")
    } else {
        "Yes, and don't ask again this session".to_owned()
    };
    let option_labels = ["Yes".to_owned(), always_label, "No".to_owned()];

    for (i, label) in option_labels.iter().enumerate() {
        let focused = i == v.focus;
        let wash = Style::default().bg(DIM());
        let mut spans = vec![
            Span::styled(
                if focused { "▸ " } else { "  " },
                Style::default()
                    .fg(SALMON())
                    .bold()
                    .bg(if focused { DIM() } else { Color::Reset }),
            ),
            Span::styled(
                format!("{}. ", i + 1),
                if focused {
                    Style::default().fg(MUTED()).bg(DIM())
                } else {
                    Style::default().fg(MUTED())
                },
            ),
            Span::styled(
                label.clone(),
                if focused {
                    Style::default().fg(SALMON()).bold().bg(DIM())
                } else {
                    Style::default().fg(CREAM())
                },
            ),
        ];
        if focused {
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let w = width.max(1) as usize;
            if used < w {
                spans.push(Span::styled(" ".repeat(w - used), wash));
            }
        }
        out.push(Line::from(spans));
    }

    out
}

fn truncate_primary(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

fn pretty_args(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(name: &'a str, args: &'a str) -> ApprovalView<'a> {
        ApprovalView {
            name,
            args,
            preview: None,
            focus: 0,
            queued: 0,
        }
    }

    #[test]
    fn shell_command_renders_as_snippet_not_json() {
        let ls = render(&view("shell", r#"{"cmd":"cargo test"}"#), 100);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("cargo test"));
        assert!(!joined.contains("approval required"));
        assert!(!joined.contains("{\"cmd\""));
        assert!(joined.contains("Do you want to proceed?"));
    }

    #[test]
    fn decision_rows_present() {
        let ls = render(&view("shell", r#"{"cmd":"ls"}"#), 100);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("1. "), "{joined}");
        assert!(joined.contains("Yes"), "{joined}");
        assert!(joined.contains("2. "), "{joined}");
        assert!(joined.contains("don't ask again"), "{joined}");
        assert!(joined.contains("3. "), "{joined}");
        assert!(joined.contains("No"), "{joined}");
        assert!(joined.contains("▸"), "focused row cursor: {joined}");
    }

    #[test]
    fn focus_moves_cursor_and_queue_count_shows() {
        let mut v = view("shell", r#"{"cmd":"ls"}"#);
        v.focus = 2;
        v.queued = 2;
        let ls = render(&v, 100);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        // Cursor sits on the No row, not the first.
        let cursor_rows: Vec<_> = ls
            .iter()
            .filter(|l| {
                let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                t.contains('▸') && t.contains("No")
            })
            .collect();
        assert_eq!(cursor_rows.len(), 1, "{joined}");
        assert!(joined.contains("+2 more"), "{joined}");
    }

    #[test]
    fn no_box_rules() {
        // Borderless like every other card — no rule lines.
        let ls = render(&view("shell", r#"{"cmd":"ls"}"#), 100);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(!joined.contains('─'), "{joined}");
        assert!(!joined.contains('╭'), "{joined}");
        assert!(!joined.contains('╰'), "{joined}");
    }

    #[test]
    fn focused_row_washes_full_width() {
        use unicode_width::UnicodeWidthStr;
        let ls = render(&view("shell", r#"{"cmd":"ls"}"#), 100);
        // The focused option row pads to the full width so the wash
        // spans edge to edge instead of hiding behind the text.
        let focused: Vec<_> = ls
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.contains('▸')))
            .collect();
        assert_eq!(focused.len(), 1);
        let cols: usize = focused[0].spans.iter().map(|sp| sp.content.width()).sum();
        assert_eq!(cols, 100, "{:?}", focused[0].spans);
        for sp in &focused[0].spans {
            assert_eq!(sp.style.bg, Some(DIM()));
        }
    }
}
