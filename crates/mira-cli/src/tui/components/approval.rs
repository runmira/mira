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
//! - **Boxed key badges**: `[ y ] approve` reads faster than
//!   `y allow · a always · n deny` and looks like buttons.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use super::tool_call::summarize_tool;
use super::tool_result;
use super::{CREAM, DIM, MUTED, SALMON};

/// Borrows of the pending approval request, taken at block-build time.
pub(crate) struct ApprovalView<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub preview: Option<&'a mira_tools::DiffPreview>,
}

pub(crate) fn render(v: &ApprovalView<'_>, width: u16) -> Vec<Line<'static>> {
    let (friendly, primary) = summarize_tool(v.name, v.args);
    let mut out: Vec<Line<'static>> = Vec::new();

    // Top rule so the block stands out from the surrounding transcript.
    out.push(rule_line());

    // Header: warning glyph + friendly tool name + (optional) target.
    let mut header: Vec<Span<'static>> = vec![
        Span::styled("  ⚠  ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(friendly, Style::default().fg(SALMON()).bold()),
        Span::styled("  ·  approval required", Style::default().fg(CREAM())),
    ];
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
    out.push(Line::from(header));
    out.push(Line::from(""));

    // Body: the full unified diff when available — the user must see
    // exactly what they're approving, uncollapsed — otherwise the
    // primary arg (command / path / pattern) as a clean code-style
    // snippet, falling back to a compact JSON view only when empty.
    if let Some(preview) = v.preview {
        out.extend(tool_result::diff_preview_lines(preview, width));
    } else if !primary.is_empty() {
        // A shell command / path / URL, whatever the summarizer picked.
        // Keep it on one line — the outer Paragraph::wrap handles reflow
        // gracefully at any terminal width.
        out.push(Line::from(vec![
            Span::styled("     $  ", Style::default().fg(DIM())),
            Span::styled(primary, Style::default().fg(CREAM())),
        ]));
    } else {
        for l in pretty_args(v.args).lines().take(6) {
            out.push(Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(l.to_owned(), Style::default().fg(CREAM())),
            ]));
        }
    }
    out.push(Line::from(""));

    // Key badges — filled backgrounds so they look like buttons.
    out.push(Line::from(vec![
        Span::styled("     ", Style::default()),
        Span::styled(
            " y ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  approve       ", Style::default().fg(CREAM())),
        Span::styled(
            " a ",
            Style::default()
                .fg(Color::Black)
                .bg(SALMON())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  always this session       ", Style::default().fg(CREAM())),
        Span::styled(
            " n ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  deny", Style::default().fg(CREAM())),
    ]));

    // Bottom rule closes the block.
    out.push(rule_line());

    out
}

/// Horizontal rule used to bracket the approval block. Long enough
/// for most terminal widths; ratatui's wrap won't split it because
/// it's a single Span and the transcript uses `Wrap { trim: false }`.
fn rule_line() -> Line<'static> {
    Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────────",
        Style::default().fg(DIM()),
    ))
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
        assert!(joined.contains("approval required"));
        assert!(!joined.contains("{\"cmd\""));
    }

    #[test]
    fn key_badges_present() {
        let ls = render(&view("shell", r#"{"cmd":"ls"}"#), 100);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains(" y "));
        assert!(joined.contains(" a "));
        assert!(joined.contains(" n "));
        assert!(joined.contains("always this session"));
    }

    #[test]
    fn bracketed_by_rules() {
        let ls = render(&view("shell", r#"{"cmd":"ls"}"#), 100);
        assert!(ls.first().is_some_and(|l| l.spans[0].content.contains('─')));
        assert!(ls.last().is_some_and(|l| l.spans[0].content.contains('─')));
    }
}
