//! Tool-result body rendering — the unified diff stream, the `└
//! snippet` line, and the expanded `│` gutter view.
//!
//! The diff renderer's contract (editor-native, not a GitHub-style
//! two-column view):
//!
//! - ONE vertical stream: file context → removed line → added lines →
//!   file context. The removed line and the lines that replace it
//!   share the same patch flow, distinguished only by their band.
//! - Old AND new line numbers in a left gutter, like a real unified
//!   diff — the reader sees where the change lands and how the file
//!   shifts below it.
//! - Removed lines get a red tint across the full code width, added
//!   lines a green tint; the `-`/`+` markers sit inside the band.
//! - Code is syntax-highlighted by the target file's extension.
//! - Whitespace is preserved exactly; long lines are clipped (never
//!   wrapped) so column alignment survives at any terminal width.

use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use mira_tools::{DiffLine, DiffPreview, DiffRow};

use super::{DIM, MUTED};
use crate::tui::markdown;

// Diff tint palette. Fixed RGBs rather than theme entries — the theme
// drives prose/chrome; diff bands keep editor semantics (red = removed,
// green = added) in every palette.
const ADD_BG: Color = Color::Rgb(24, 56, 30);
const DEL_BG: Color = Color::Rgb(72, 28, 26);
const ADD_FG: Color = Color::Rgb(184, 224, 184);
const DEL_FG: Color = Color::Rgb(228, 162, 154);
const NUM_FG: Color = Color::Rgb(120, 120, 128);
const NUM_DEL_FG: Color = Color::Rgb(150, 96, 92);
const NUM_ADD_FG: Color = Color::Rgb(96, 140, 100);

/// A rendered ToolResult passed alongside its ToolCall so the group can
/// show as one visual block. Borrowed strings, since the paired entries
/// outlive the render pass.
#[derive(Clone, Copy)]
pub(crate) struct ResultView<'a> {
    pub ok: bool,
    pub snippet: &'a str,
    /// Full (capped) tool output — shown when `expanded`.
    pub full: &'a str,
    pub expanded: bool,
    /// Auto-collapse state resolved at build time (older turns collapse
    /// unless the user pinned the group). `tool_call::render` reads it
    /// off the sibling ToolView; carried here so the view stays one
    /// borrow-shaped value.
    pub collapsed: bool,
}

/// Body rows for one tool result, below the call header. Chooses
/// between three shapes:
///
/// 1. diff preview (when the call succeeded and carries one) — the
///    "what actually changed" view;
/// 2. expanded full output behind a `│` gutter (Ctrl+E);
/// 3. the one-line `└` snippet — suppressed entirely for a green-dot
///    bash success whose only content is `exit=0`, which reads as pure
///    noise under an already-green header.
pub(crate) fn body_lines(result: &ResultView<'_>) -> Vec<Line<'static>> {
    match (result.expanded, result.full.is_empty()) {
        (true, false) => {
            let mut out = Vec::new();
            for line in result.full.lines() {
                out.push(Line::from(vec![
                    Span::styled("  │  ", Style::default().fg(DIM())),
                    Span::styled(line.to_owned(), Style::default().fg(MUTED())),
                ]));
            }
            out
        }
        _ => {
            // A green-dot success + a bare "exit=0" snippet is pure
            // noise — the dot already says "worked." Suppress the body
            // row in that specific case; if Bash produced real output
            // the user still sees the chevron and can Ctrl+E.
            let bash_success_no_output = result.ok
                && result.snippet.trim() == "exit=0"
                && !result
                    .full
                    .split("--- output ---\n")
                    .nth(1)
                    .map(|o| !o.trim().is_empty())
                    .unwrap_or(false);
            if bash_success_no_output {
                return Vec::new();
            }
            let body = if result.snippet.is_empty() {
                "(no output)".to_owned()
            } else {
                super::truncate(result.snippet, 200)
            };
            vec![Line::from(vec![
                Span::styled("  └  ", Style::default().fg(DIM())),
                Span::styled(body, Style::default().fg(MUTED())),
            ])]
        }
    }
}

/// Count `+`/`-` rows for the header stat chip. Prefers the numbered
/// hunk view; falls back to the flat compat lines.
pub(crate) fn diff_stats(p: &DiffPreview) -> (usize, usize) {
    if !p.hunks.is_empty() {
        let mut adds = 0;
        let mut dels = 0;
        for h in &p.hunks {
            for r in &h.rows {
                match r {
                    DiffRow::Add { .. } => adds += 1,
                    DiffRow::Del { .. } => dels += 1,
                    DiffRow::Ctx { .. } => {}
                }
            }
        }
        return (adds, dels);
    }
    let mut adds = 0;
    let mut dels = 0;
    for l in &p.lines {
        match l {
            DiffLine::Add(_) => adds += 1,
            DiffLine::Del(_) => dels += 1,
            _ => {}
        }
    }
    (adds, dels)
}

/// The unified diff stream, sized to `width` (the transcript region
/// width) so add/del bands span the full code area. Renders every row
/// the preview carries — changed lines are never hidden behind an
/// interaction; generation caps the pathological cases instead.
pub(crate) fn diff_preview_lines(p: &DiffPreview, width: u16) -> Vec<Line<'static>> {
    if p.hunks.is_empty() {
        // Preview persisted before hunks existed (old session record)
        // — render the flat compat view without a gutter.
        return p.lines.iter().map(indented_diff_line).collect();
    }

    let num_w = gutter_width(p);
    let mut out: Vec<Line<'static>> = Vec::new();
    for (i, hunk) in p.hunks.iter().enumerate() {
        if i > 0 {
            out.push(hunk_gap_line());
        }
        for row in &hunk.rows {
            out.push(row_line(row, &p.path, num_w, width));
        }
    }
    if p.truncated {
        out.push(Line::from(Span::styled(
            "  ⋯ diff truncated — remaining lines elided",
            Style::default().fg(DIM()).italic(),
        )));
    }
    out
}

/// Gutter column width: enough digits for the largest line number
/// *displayed* (del→old, add/ctx→new), min 3 so tiny diffs don't look
/// cramped.
fn gutter_width(p: &DiffPreview) -> usize {
    let max = p
        .hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .map(|r| match r {
            DiffRow::Ctx { old, new, .. } => (*old).max(*new),
            DiffRow::Add { new, .. } => *new,
            DiffRow::Del { old, .. } => *old,
        })
        .max()
        .unwrap_or(0);
    (max.to_string().len()).clamp(3, 5)
}

fn num_span(n: u32, w: usize, fg: Color) -> Span<'static> {
    Span::styled(format!("{n:>w$} "), Style::default().fg(fg))
}

/// One diff row — `num ± code` with a full-width tint band on changed
/// rows, matching the editor-native reference: ONE number column, the
/// *relevant* one per row (removed lines show their old number; added
/// and context lines show their new number). The file shift stays
/// readable because the context numbers jump by the insertion size.
/// Whitespace is preserved; long code is clipped (never wrapped) so
/// the gutter stays column-aligned.
fn row_line(row: &DiffRow, path: &str, num_w: usize, width: u16) -> Line<'static> {
    let prefix_cols = num_w + 3; // number + separator + marker + space
    let available = (width as usize).saturating_sub(prefix_cols);
    match row {
        DiffRow::Ctx { new, text, .. } => {
            let mut spans = vec![num_span(*new, num_w, NUM_FG), Span::raw("  ")];
            spans.extend(code_spans(
                text,
                path,
                available,
                Style::default().fg(MUTED()),
                None,
            ));
            Line::from(spans)
        }
        DiffRow::Del { old, text } => {
            let mut spans = vec![
                num_span(*old, num_w, NUM_DEL_FG),
                Span::styled("- ", Style::default().fg(Color::Red).bg(DEL_BG).bold()),
            ];
            spans.extend(code_spans(
                text,
                path,
                available,
                Style::default().fg(DEL_FG).bg(DEL_BG),
                Some(DEL_BG),
            ));
            pad_band(spans, DEL_BG, width)
        }
        DiffRow::Add { new, text } => {
            let mut spans = vec![
                num_span(*new, num_w, NUM_ADD_FG),
                Span::styled("+ ", Style::default().fg(Color::Green).bg(ADD_BG).bold()),
            ];
            spans.extend(code_spans(
                text,
                path,
                available,
                Style::default().fg(ADD_FG).bg(ADD_BG),
                Some(ADD_BG),
            ));
            pad_band(spans, ADD_BG, width)
        }
    }
}

/// Syntax-highlighted spans for one code line, clipped to `available`
/// display columns. `base` carries the row's fg (and bg tint, if any);
/// highlighting replaces the fg but keeps the tint so the band reads
/// as one continuous color.
fn code_spans(
    text: &str,
    path: &str,
    available: usize,
    base: Style,
    bg: Option<Color>,
) -> Vec<Span<'static>> {
    let (shown, clipped) = clip_to_width(text, available);
    let mut spans: Vec<Span<'static>> = match markdown::highlight_code_line(shown, path) {
        Some(highlighted) => highlighted
            .into_iter()
            .map(|s| {
                let fg = s.style.fg.unwrap_or_else(|| base.fg.unwrap_or_default());
                Span::styled(s.content, base.fg(fg))
            })
            .collect(),
        None => vec![Span::styled(shown.to_owned(), base)],
    };
    if clipped {
        let marker_style = match bg {
            Some(bg) => Style::default().fg(DIM()).bg(bg).bold(),
            None => Style::default().fg(DIM()).bold(),
        };
        spans.push(Span::styled("›", marker_style));
    }
    if spans.is_empty() {
        // Blank row — the band still spans the width via the pad.
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

/// Clip `text` to `available` display columns on char boundaries.
/// Returns the visible text and whether anything was elided.
fn clip_to_width(text: &str, available: usize) -> (&str, bool) {
    if UnicodeWidthStr::width(text) <= available {
        return (text, false);
    }
    let mut col = 0usize;
    for (i, c) in text.char_indices() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if col + cw > available.saturating_sub(1) {
            return (&text[..i], true);
        }
        col += cw;
    }
    (text, true)
}

/// Extend a changed row's spans with a bg-styled pad so the tint band
/// reaches the right edge of the region — the editor-native look.
fn pad_band(mut spans: Vec<Span<'static>>, bg: Color, width: u16) -> Line<'static> {
    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let fill = (width as usize).saturating_sub(used);
    if fill > 0 {
        spans.push(Span::styled(" ".repeat(fill), Style::default().bg(bg)));
    }
    Line::from(spans)
}

/// Dim `⋯` separator between hunks.
fn hunk_gap_line() -> Line<'static> {
    Line::from(Span::styled("  ⋯", Style::default().fg(DIM()).italic()))
}

/// Flat-view fallback for previews persisted before hunks existed.
fn indented_diff_line(d: &DiffLine) -> Line<'static> {
    match d {
        DiffLine::Ctx(s) => Line::from(vec![
            Span::raw("   "),
            Span::styled(s.clone(), Style::default().fg(MUTED())),
        ]),
        DiffLine::Add(s) => Line::from(vec![
            Span::styled("  +", Style::default().fg(Color::Green).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Green)),
        ]),
        DiffLine::Del(s) => Line::from(vec![
            Span::styled("  -", Style::default().fg(Color::Red).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Red)),
        ]),
        DiffLine::HunkGap => Line::from(Span::styled("    ⋯", Style::default().fg(DIM()).italic())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_tools::DiffHunk;

    fn preview(path: &str, hunks: Vec<DiffHunk>) -> DiffPreview {
        DiffPreview {
            path: path.into(),
            kind: mira_tools::DiffKind::Edit,
            lines: Vec::new(),
            hunks,
            truncated: false,
        }
    }

    fn ctx(old: u32, new: u32, text: &str) -> DiffRow {
        DiffRow::Ctx {
            old,
            new,
            text: text.into(),
        }
    }

    fn text_of(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.clone()).collect()
    }

    fn used_width(l: &Line<'_>) -> usize {
        l.spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    #[test]
    fn rows_render_gutter_and_markers() {
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![
                    ctx(1, 1, "fn before() {}"),
                    DiffRow::Del {
                        old: 2,
                        text: "fn old() {}".into(),
                    },
                    DiffRow::Add {
                        new: 2,
                        text: "fn new() {}".into(),
                    },
                    ctx(3, 3, "fn after() {}"),
                ],
            }],
        );
        let ls = diff_preview_lines(&p, 80);
        assert_eq!(ls.len(), 4);
        // Single gutter column: context shows its NEW number, the del
        // shows its OLD number, the add shows its NEW number.
        let ctx_line = text_of(&ls[0]);
        assert!(ctx_line.starts_with("  1  "), "ctx num: {ctx_line}");
        assert!(!ctx_line.contains(" 1  1 "), "no second number: {ctx_line}");
        assert!(ctx_line.contains("fn before()"));
        let del_line = text_of(&ls[1]);
        assert!(del_line.starts_with("  2 - "), "del num: {del_line}");
        assert!(del_line.contains("fn old()"), "{del_line}");
        let add_line = text_of(&ls[2]);
        assert!(add_line.starts_with("  2 + "), "add num: {add_line}");
        assert!(add_line.contains("fn new()"), "{add_line}");
        // Shift visibility: context after the change continues from
        // the add's number.
        let after = text_of(&ls[3]);
        assert!(after.starts_with("  3  "), "{after}");
    }

    #[test]
    fn changed_rows_carry_bg_tint_and_pad_full_width() {
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![DiffRow::Add {
                    new: 1,
                    text: "let x = 1;".into(),
                }],
            }],
        );
        let ls = diff_preview_lines(&p, 80);
        let line = &ls[0];
        assert!(line.spans.iter().all(|s| s.style.bg == Some(ADD_BG)));
        assert_eq!(used_width(line), 80);
    }

    #[test]
    fn context_rows_have_no_band() {
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![ctx(1, 1, "plain")],
            }],
        );
        let ls = diff_preview_lines(&p, 80);
        assert!(ls[0].spans.iter().all(|s| s.style.bg.is_none()));
    }

    #[test]
    fn whitespace_is_preserved() {
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![DiffRow::Add {
                    new: 1,
                    text: "        deeply indented();".into(),
                }],
            }],
        );
        let ls = diff_preview_lines(&p, 80);
        assert!(text_of(&ls[0]).contains("        deeply indented();"));
    }

    #[test]
    fn long_lines_clip_never_wrap() {
        let long = "x".repeat(200);
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![DiffRow::Add { new: 1, text: long }],
            }],
        );
        let ls = diff_preview_lines(&p, 60);
        assert_eq!(used_width(&ls[0]), 60, "row must fit the region exactly");
        assert!(text_of(&ls[0]).contains('›'), "clip marker present");
    }

    #[test]
    fn multiple_hunks_get_gap_rows() {
        let p = preview(
            "src/lib.rs",
            vec![
                DiffHunk {
                    old_start: 1,
                    new_start: 1,
                    rows: vec![ctx(1, 1, "a")],
                },
                DiffHunk {
                    old_start: 10,
                    new_start: 12,
                    rows: vec![ctx(10, 12, "b")],
                },
            ],
        );
        let ls = diff_preview_lines(&p, 80);
        assert_eq!(ls.len(), 3);
        assert!(text_of(&ls[1]).contains('⋯'));
    }

    #[test]
    fn legacy_flat_lines_render_without_gutter() {
        let mut p = preview("src/lib.rs", vec![]);
        p.lines = vec![
            DiffLine::Ctx("before".into()),
            DiffLine::Del("old".into()),
            DiffLine::Add("new".into()),
        ];
        let ls = diff_preview_lines(&p, 80);
        assert_eq!(ls.len(), 3);
        assert!(!text_of(&ls[0]).contains(" 1  "));
    }

    #[test]
    fn truncated_flag_adds_notice() {
        let mut p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![ctx(1, 1, "a")],
            }],
        );
        p.truncated = true;
        let ls = diff_preview_lines(&p, 80);
        assert!(text_of(&ls[1]).contains("truncated"));
    }

    #[test]
    fn syntax_highlight_colors_rust_code() {
        let p = preview(
            "src/main.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![DiffRow::Add {
                    new: 1,
                    text: "let pattern = \"string\";".into(),
                }],
            }],
        );
        let ls = diff_preview_lines(&p, 120);
        // Keyword and string tokens get distinct fg colors from syntect.
        let fgs: Vec<_> = ls[0]
            .spans
            .iter()
            .filter(|s| !s.content.trim().is_empty() && s.content != "+")
            .map(|s| s.style.fg)
            .collect();
        assert!(fgs.len() > 1, "expected multi-token highlighting: {fgs:?}");
        assert!(fgs.iter().any(|f| f.is_some()));
    }

    #[test]
    fn ts_and_tsx_paths_render_without_panicking() {
        for path in ["components/Chip.tsx", "components/Chip.ts", "notes.md"] {
            let p = preview(
                path,
                vec![DiffHunk {
                    old_start: 1,
                    new_start: 1,
                    rows: vec![ctx(1, 1, "const x: string = 'v';")],
                }],
            );
            let ls = diff_preview_lines(&p, 80);
            assert!(text_of(&ls[0]).contains("const x: string = 'v';"));
        }
    }

    #[test]
    fn diff_stats_counts_from_hunks() {
        let p = preview(
            "src/lib.rs",
            vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                rows: vec![
                    ctx(1, 1, "a"),
                    DiffRow::Del {
                        old: 2,
                        text: "b".into(),
                    },
                    DiffRow::Add {
                        new: 2,
                        text: "c".into(),
                    },
                    DiffRow::Add {
                        new: 3,
                        text: "d".into(),
                    },
                ],
            }],
        );
        assert_eq!(diff_stats(&p), (2, 1));
    }
}
