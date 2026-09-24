//! Markdown-to-ratatui converter for assistant replies.
//!
//! Supports the shapes that actually show up in coding-agent output:
//!
//! - Headers (`#`..`######`)
//! - `**bold**`, `*italic*`, `` `inline code` ``
//! - Fenced code blocks with syntect syntax highlighting
//! - `-`, `*`, `+` bullets and `1.` numbered lists — indent preserved
//!   so nested lists render at the right depth
//! - Blockquotes (`>`), including nested `> >`
//! - Pipe tables (` | a | b | ` + separator row) — column widths
//!   auto-computed, rendered with box-drawing characters
//! - Inline links (`[text](url)`) — text shown blue+underline, URL dim
//!
//! The parser is streaming-tolerant: unclosed spans render literally
//! until their closer arrives. Cheap enough to re-run every frame on
//! ~10 KB messages.

use std::sync::LazyLock;

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

/// Prose text color. Reads the live theme on every call so `/theme
/// <name>` updates the next render tick. `Color` is `Copy` so the
/// read lock releases immediately; the hot path is a single atomic
/// on the uncontended read side of an `RwLock`.
#[inline]
fn prose() -> Color {
    crate::tui::theme::current().prose
}
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

// One-time load of syntect's default syntaxes + themes. First TUI
// render pays ~50-100ms; every subsequent one is a `Arc`-clone.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let ts = ThemeSet::load_defaults();
    // "base16-eighties.dark" is the closest built-in theme to what
    // most terminals already use for their own defaults — pairs
    // cleanly with a black bg.
    ts.themes
        .get("base16-eighties.dark")
        .cloned()
        .expect("built-in syntect theme")
});

/// Convert a markdown string into styled lines. Wrap-friendly: each
/// `Line` maps to one visual row, and ratatui's `Paragraph::wrap`
/// handles overflow.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut lines: Vec<&str> = text.split('\n').collect();
    // Push in reverse so we can pop from the end cheaply. Reverse
    // once, then treat it as a stack that yields in original order.
    lines.reverse();

    while let Some(raw) = lines.pop() {
        // ---- code fence: consume the whole block, syntax-highlight ----
        if let Some(lang) = fence_open(raw) {
            let mut code: Vec<String> = Vec::new();
            while let Some(next) = lines.pop() {
                if is_fence_close(next) {
                    break;
                }
                code.push(next.to_owned());
            }
            emit_code_block(&mut out, &lang, &code.join("\n"));
            continue;
        }

        // ---- table: consume header + separator + body rows ----
        if looks_like_table_header(raw, lines.last().copied()) {
            let header = parse_row(raw);
            // Consume the separator line so it doesn't render.
            let _ = lines.pop();
            let mut rows: Vec<Vec<String>> = Vec::new();
            while let Some(next) = lines.last().copied() {
                if !looks_like_row(next) {
                    break;
                }
                rows.push(parse_row(next));
                lines.pop();
            }
            emit_table(&mut out, header, rows);
            continue;
        }

        // ---- single-line blocks ----
        if let Some((depth, body)) = blockquote_split(raw) {
            out.push(blockquote_line(depth, body));
            continue;
        }
        if let Some((hashes, body)) = header_split(raw) {
            out.push(header_line(hashes, body));
            continue;
        }
        if let Some((indent, marker, body)) = list_split(raw) {
            let mut spans = vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(marker, Style::default().fg(Color::Cyan)),
            ];
            spans.extend(inline_spans(body));
            out.push(Line::from(spans));
            continue;
        }
        out.push(Line::from(inline_spans(raw)));
    }
    out
}

// -------------------------- headers --------------------------

fn header_split(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &trimmed[hashes..];
    let body = after.strip_prefix(' ')?;
    Some((hashes, body))
}

fn header_line(hashes: usize, body: &str) -> Line<'static> {
    let style = match hashes {
        1 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    };
    let marker = match hashes {
        1 => "▎ ",
        2 => "▏ ",
        _ => "  ",
    };
    let mut spans = vec![Span::styled(marker, style)];
    for mut span in inline_spans(body) {
        span.style = style.patch(span.style);
        spans.push(span);
    }
    Line::from(spans)
}

// -------------------------- lists --------------------------

/// Detect a bullet or numbered-list line, returning `(visual_indent,
/// marker, body)`. `visual_indent` is doubled leading-space width in
/// chars — we render two spaces per depth level so nesting is obvious
/// without eating too much horizontal room.
fn list_split(line: &str) -> Option<(usize, String, &str)> {
    let leading: usize = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let indent_level = leading / 2; // 2 spaces per nesting level
    let trimmed = &line[leading..];
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let indent_chars = indent_level * 2;
            return Some((indent_chars, "• ".to_owned(), rest));
        }
    }
    // Numbered: 1. foo   /   12. bar
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after = &trimmed[digits.len()..];
        if let Some(rest) = after.strip_prefix(". ") {
            let indent_chars = indent_level * 2;
            return Some((indent_chars, format!("{digits}. "), rest));
        }
    }
    None
}

// -------------------------- blockquotes --------------------------

/// `> body` → `(1, body)`. `> > body` → `(2, body)`. Requires the
/// space after each `>` so we don't hijack shell prompt lines like
/// `>` on its own.
fn blockquote_split(line: &str) -> Option<(usize, &str)> {
    let mut depth = 0;
    let mut rest = line.trim_start();
    while let Some(stripped) = rest.strip_prefix("> ").or_else(|| rest.strip_prefix('>')) {
        depth += 1;
        rest = stripped.strip_prefix(' ').unwrap_or(stripped);
        // Only accept an unspaced `>` if it's the last char on the line.
        if depth > 0 && rest.is_empty() && line.trim_end().ends_with('>') {
            break;
        }
    }
    if depth == 0 {
        None
    } else {
        Some((depth, rest))
    }
}

fn blockquote_line(depth: usize, body: &str) -> Line<'static> {
    let mut spans = Vec::with_capacity(depth + 1);
    for _ in 0..depth {
        spans.push(Span::styled("┃ ", Style::default().fg(Color::DarkGray)));
    }
    for mut span in inline_spans(body) {
        span.style = span
            .style
            .add_modifier(Modifier::ITALIC)
            .fg(span.style.fg.unwrap_or(Color::Gray));
        spans.push(span);
    }
    Line::from(spans)
}

// -------------------------- fenced code blocks --------------------------

fn fence_open(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("```")?;
    Some(rest.trim().to_owned())
}

fn is_fence_close(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "```" || trimmed.starts_with("```")
}

fn emit_code_block(out: &mut Vec<Line<'static>>, lang: &str, code: &str) {
    // Small language tag on its own line, no decorative rules. The
    // previous `─── ts ───` / `─── end ───` frame turned into stray
    // "ts" / "end" bleed-through in terminals whose fonts render the
    // U+2500 box chars as narrow glyphs, so the block stopped looking
    // like a block. A single dim label + a solid gutter reads as code
    // in any monospace font.
    if !lang.is_empty() {
        out.push(Line::from(Span::styled(
            format!(" {lang}"),
            Style::default().fg(Color::DarkGray).italic(),
        )));
    }

    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(lang))
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, &THEME);

    // `▎` (U+258E LEFT ONE QUARTER BLOCK) reads as a solid spine even
    // when `│` (U+2502) collapses to a thin ambiguous line. Coloured
    // dim so it frames without competing with the highlighted code.
    let gutter_style = Style::default().fg(Color::Rgb(120, 120, 128));

    for raw_line in LinesWithEndings::from(code) {
        // syntect may fail on pathological input; fall back to a
        // plain line rather than dropping content.
        let ranges: Vec<(SynStyle, &str)> = h
            .highlight_line(raw_line, &SYNTAX_SET)
            .unwrap_or_else(|_| vec![(SynStyle::default(), raw_line)]);
        let mut spans: Vec<Span<'static>> = vec![Span::styled("▎ ", gutter_style)];
        for (style, chunk) in ranges {
            // Trim only the trailing newline so per-line layout stays
            // one Line per source line.
            let content = chunk.trim_end_matches('\n').to_owned();
            if content.is_empty() {
                continue;
            }
            spans.push(Span::styled(content, syntect_to_ratatui(style)));
        }
        out.push(Line::from(spans));
    }
}

/// Highlight one line of code with the syntax matched from a file
/// path (extension-driven: `.rs`, `.ts`, `.tsx`, …). Returns `None`
/// for blank lines or when the highlighter chokes.
///
/// Stateless per line on purpose: diff rows interleave two versions
/// of a file (removed lines from the old, added from the new), so a
/// shared multi-line highlighter state would corrupt both. Per-line
/// highlighting still colors keywords/strings/types correctly.
pub(crate) fn highlight_code_line(line: &str, path: &str) -> Option<Vec<Span<'static>>> {
    if line.trim().is_empty() {
        return None;
    }
    let syntax = SYNTAX_SET
        .find_syntax_for_file(path)
        .ok()
        .flatten()
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, &THEME);
    // syntect wants the trailing newline for its line-ending handling.
    let with_nl = format!("{line}\n");
    let ranges = h.highlight_line(&with_nl, &SYNTAX_SET).ok()?;
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (style, chunk) in ranges {
        let content = chunk.trim_end_matches('\n').to_owned();
        if content.is_empty() {
            continue;
        }
        spans.push(Span::styled(content, syntect_to_ratatui(style)));
    }
    if spans.is_empty() {
        None
    } else {
        Some(spans)
    }
}

fn syntect_to_ratatui(s: SynStyle) -> Style {
    let fg = Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b);
    let mut style = Style::default().fg(fg);
    // Deliberately drop syntect's background — most terminals already
    // have a chosen bg and forcing one clashes.
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

// -------------------------- tables --------------------------

fn looks_like_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.contains('|') && !is_separator_row(line)
}

fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return false;
    }
    trimmed.trim_matches('|').split('|').all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
    })
}

/// A header row triggers a table only when the very next line is a
/// separator (`|---|---|`). Otherwise it's just a paragraph that
/// happens to contain pipes.
fn looks_like_table_header(row: &str, peek: Option<&str>) -> bool {
    looks_like_row(row) && peek.is_some_and(is_separator_row)
}

fn parse_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_owned())
        .collect()
}

fn emit_table(out: &mut Vec<Line<'static>>, header: Vec<String>, rows: Vec<Vec<String>>) {
    let n_cols = header
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if n_cols == 0 {
        return;
    }
    let mut widths = vec![0usize; n_cols];
    for row in std::iter::once(&header).chain(rows.iter()) {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let border_style = Style::default().fg(Color::DarkGray);
    let header_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    let sep = |left: &str, mid: &str, right: &str| -> Line<'static> {
        let mut buf = String::new();
        buf.push_str(left);
        for (i, w) in widths.iter().enumerate() {
            for _ in 0..w + 2 {
                buf.push('─');
            }
            buf.push_str(if i + 1 == widths.len() { right } else { mid });
        }
        Line::from(Span::styled(buf, border_style))
    };

    out.push(sep("┌", "┬", "┐"));
    out.push(render_row(&header, &widths, header_style, border_style));
    out.push(sep("├", "┼", "┤"));
    for row in &rows {
        out.push(render_row(row, &widths, Style::default(), border_style));
    }
    out.push(sep("└", "┴", "┘"));
}

fn render_row(
    cells: &[String],
    widths: &[usize],
    cell_style: Style,
    border_style: Style,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(widths.len() * 2 + 1);
    spans.push(Span::styled("│", border_style));
    for (i, w) in widths.iter().enumerate() {
        let text = cells.get(i).cloned().unwrap_or_default();
        let pad = w.saturating_sub(text.chars().count());
        spans.push(Span::styled(format!(" {text}"), cell_style));
        spans.push(Span::styled(" ".repeat(pad + 1), cell_style));
        spans.push(Span::styled("│", border_style));
    }
    Line::from(spans)
}

// -------------------------- inline --------------------------

/// Break a line into styled spans. Handles inline code, bold, italic,
/// and `[text](url)` links. Unclosed spans render as literal
/// characters — safe for streaming input.
fn inline_spans(line: &str) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;

    while i < bytes.len() {
        // `code` first — anything inside is literal.
        if bytes[i] == b'`' {
            if let Some(close) = find_close_byte(line, i + 1, b'`') {
                flush_plain(&mut out, line, plain_start, i);
                out.push(Span::styled(
                    line[i + 1..close].to_owned(),
                    inline_code_style(),
                ));
                i = close + 1;
                plain_start = i;
                continue;
            }
        }
        // Link: [text](url)
        if bytes[i] == b'[' {
            if let Some((end_bracket, end_paren)) = link_span(line, i) {
                flush_plain(&mut out, line, plain_start, i);
                let text = &line[i + 1..end_bracket];
                let url = &line[end_bracket + 2..end_paren];
                out.push(Span::styled(text.to_owned(), link_style()));
                out.push(Span::styled(
                    format!(" ({url})"),
                    Style::default().fg(Color::DarkGray),
                ));
                i = end_paren + 1;
                plain_start = i;
                continue;
            }
        }
        // Bold: **x**
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(close) = find_close_str(line, i + 2, "**") {
                flush_plain(&mut out, line, plain_start, i);
                out.push(Span::styled(
                    line[i + 2..close].to_owned(),
                    Style::default().fg(prose()).add_modifier(Modifier::BOLD),
                ));
                i = close + 2;
                plain_start = i;
                continue;
            }
        }
        // Italic: *x* (single star). Skip when preceded by alnum to
        // avoid hijacking `foo*bar*baz` inside identifiers.
        if bytes[i] == b'*' && !prev_is_alnum(bytes, i) {
            if let Some(close) = find_single_star_close(line, i + 1) {
                flush_plain(&mut out, line, plain_start, i);
                out.push(Span::styled(
                    line[i + 1..close].to_owned(),
                    Style::default().fg(prose()).add_modifier(Modifier::ITALIC),
                ));
                i = close + 1;
                plain_start = i;
                continue;
            }
        }
        i += 1;
    }

    flush_plain(&mut out, line, plain_start, line.len());
    if out.is_empty() {
        out.push(Span::raw(String::new()));
    }
    out
}

fn flush_plain(out: &mut Vec<Span<'static>>, s: &str, start: usize, end: usize) {
    if end > start {
        out.push(Span::styled(
            s[start..end].to_owned(),
            Style::default().fg(prose()),
        ));
    }
}

fn find_close_byte(s: &str, from: usize, target: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == target {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_close_str(s: &str, from: usize, needle: &str) -> Option<usize> {
    s[from..].find(needle).map(|off| from + off)
}

fn find_single_star_close(s: &str, from: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            let is_double_start =
                i + 1 < bytes.len() && bytes[i + 1] == b'*' && (i == from || bytes[i - 1] != b'*');
            let is_double_end = i > 0 && bytes[i - 1] == b'*';
            if !is_double_start && !is_double_end {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Match `[text](url)` starting at `open` (the `[`). Returns
/// `(bracket_close_index, paren_close_index)` or `None` if the shape
/// isn't complete.
fn link_span(s: &str, open: usize) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let close_bracket = find_close_byte(s, open + 1, b']')?;
    // Must be immediately followed by `(`.
    if close_bracket + 1 >= bytes.len() || bytes[close_bracket + 1] != b'(' {
        return None;
    }
    let close_paren = find_close_byte(s, close_bracket + 2, b')')?;
    Some((close_bracket, close_paren))
}

fn prev_is_alnum(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    bytes[i - 1].is_ascii_alphanumeric()
}

fn inline_code_style() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .bg(Color::Rgb(30, 30, 30))
}

fn link_style() -> Style {
    Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::UNDERLINED)
}

// -------------------------- streaming --------------------------

/// Streaming markdown renderer for token-by-token assistant replies.
///
/// Feed it deltas as they arrive with [`push`](Self::push); it returns
/// the display lines that are now final (everything up to the last
/// newline). Partial trailing text stays buffered until either the next
/// delta closes the line or [`flush`](Self::flush) is called at
/// turn-end. Callers push each returned line into real terminal
/// scrollback via `InlineTerm::insert_history`, so the inline pane
/// never has to hold assistant text at all — which is what keeps long
/// replies from clipping against a fixed pane height.
///
/// The renderer holds two pieces of cross-line state that ordinary
/// per-frame markdown rendering doesn't need:
///
/// - **Fenced code blocks** must not emit until the closing `` ``` ``
///   arrives; otherwise the syntax highlighter runs on partial code and
///   picks the wrong tokens. Lines inside the fence buffer.
/// - **Tables** buffer until a non-`|` line closes them, so column
///   widths can be computed from the full body rather than reflowed
///   mid-stream.
#[derive(Default)]
pub(crate) struct MarkdownStream {
    /// Bytes that haven't ended in `\n` yet.
    buf: String,
    /// Inside a fenced code block: lines accumulate in `fence` until
    /// the closer arrives, then render as one syntax-highlighted block.
    in_fence: bool,
    fence_lang: String,
    fence_body: Vec<String>,
    /// Inside a pipe table: raw lines buffer here until a non-`|` line
    /// closes it, then render as one styled table.
    table: Vec<String>,
    /// Have we emitted any non-blank line yet? Leading blanks are dropped
    /// so a reply that opens with a `\n` doesn't waste the first
    /// scrollback row on nothing.
    seen_content: bool,
    /// Trailing blank line held back from the last `push`. A blank at
    /// the tail is ambiguous during streaming — it could be the start of
    /// a paragraph break (keep it) or the last line before the reply
    /// ends (drop it). Emitting it eagerly and then chasing it with more
    /// blanks stacks flicker; we hold it and re-emit lazily only if the
    /// next batch has real content.
    held_blank: bool,
}

impl MarkdownStream {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Append `delta`, emitting the lines that are now final. Adjacent
    /// blank lines are collapsed into one and trailing blanks are held
    /// until the next batch confirms they belong to a paragraph break —
    /// keeps the scrollback tight while a reply streams in.
    pub(crate) fn push(&mut self, delta: &str) -> Vec<Line<'static>> {
        self.buf.push_str(delta);
        let mut raw = Vec::new();
        while let Some(end) = self.buf.find('\n') {
            let s: String = self.buf.drain(..=end).collect();
            let line = s.trim_end_matches('\n').to_owned();
            self.consume_line(&line, &mut raw);
        }
        self.hold_blank_edges(raw)
    }

    /// Emit any buffered content — the tail of the buffer (may be a
    /// partial line), an open table's rows, and an unclosed fence's
    /// lines. Call this exactly once at turn-end. The held trailing
    /// blank from the last `push` is dropped intentionally — a reply
    /// that ends on a blank line would waste a scrollback row.
    pub(crate) fn flush(&mut self) -> Vec<Line<'static>> {
        let mut raw = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        if !rest.is_empty() {
            self.consume_line(rest.trim_end_matches('\n'), &mut raw);
        }
        if self.in_fence {
            // Unclosed fence: render what we have as a code block so
            // the user sees their partial code instead of nothing.
            let lang = std::mem::take(&mut self.fence_lang);
            let body = std::mem::take(&mut self.fence_body);
            emit_code_block(&mut raw, &lang, &body.join("\n"));
            self.in_fence = false;
        }
        self.flush_table(&mut raw);
        // Deliberately do NOT re-emit the held blank at flush time —
        // a reply's last line should never be an empty row.
        self.held_blank = false;
        // Reset for the next reply so an idle-then-reused stream
        // doesn't inherit "seen_content" from the previous turn.
        self.seen_content = false;
        drop_leading_and_trailing_blanks(raw)
    }

    fn consume_line(&mut self, line: &str, out: &mut Vec<Line<'static>>) {
        let trimmed = line.trim_start();

        // Fence toggle. We rely on the same helpers `render` uses so
        // opener/closer syntax stays consistent between batch and
        // streaming render paths.
        if !self.in_fence {
            if let Some(lang) = fence_open(line) {
                // Close any open table first — a table+fence sandwich
                // without a blank between them is rare but should
                // still render both.
                self.flush_table(out);
                self.in_fence = true;
                self.fence_lang = lang;
                self.fence_body.clear();
                return;
            }
        } else if is_fence_close(line) {
            let lang = std::mem::take(&mut self.fence_lang);
            let body = std::mem::take(&mut self.fence_body);
            emit_code_block(out, &lang, &body.join("\n"));
            self.in_fence = false;
            return;
        } else {
            self.fence_body.push(line.to_owned());
            return;
        }

        // Table rows buffer until a non-`|` line closes the table.
        if trimmed.starts_with('|') {
            self.table.push(line.to_owned());
            return;
        }
        self.flush_table(out);

        // Everything else: render the single line with the batch
        // renderer. Prose lines don't span, so a per-line call is
        // correct and matches non-streaming output byte-for-byte.
        let rendered = render(line);
        if rendered.is_empty() {
            out.push(Line::from(""));
        } else {
            out.extend(rendered);
        }
    }

    fn flush_table(&mut self, out: &mut Vec<Line<'static>>) {
        if self.table.is_empty() {
            return;
        }
        let joined = self.table.join("\n");
        self.table.clear();
        out.extend(render(&joined));
    }

    /// Filter one push()'s output so scrollback stays tight:
    ///
    /// - Drop leading blank lines until we've seen real content in the
    ///   reply (a stray `\n` before the first prose shouldn't waste the
    ///   first row).
    /// - Collapse runs of blank lines to one.
    /// - Hold the batch's trailing blank line back. Emit the previously
    ///   held blank at the head of the next batch when that batch has
    ///   real content — otherwise the reply just gained a hanging empty
    ///   row that never resolves.
    fn hold_blank_edges(&mut self, raw: Vec<Line<'static>>) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::with_capacity(raw.len() + 1);
        let mut trailing_blanks: usize = 0;
        for line in raw {
            let blank = super::components::line_is_empty(&line);
            if blank {
                trailing_blanks += 1;
                continue;
            }
            // Non-blank: the batch now has real content. Redeem any
            // previously held blank as a paragraph break, then apply any
            // in-batch blanks (capped at 1 — collapsing paragraph runs).
            if !self.seen_content {
                // Very first content line of the reply — no leading
                // paragraph break needed.
                trailing_blanks = 0;
                self.held_blank = false;
            }
            if self.held_blank {
                out.push(Line::from(""));
                self.held_blank = false;
            } else if trailing_blanks > 0 {
                out.push(Line::from(""));
            }
            trailing_blanks = 0;
            out.push(line);
            self.seen_content = true;
        }
        // Hold at most one trailing blank until the next batch confirms.
        if trailing_blanks > 0 && self.seen_content {
            self.held_blank = true;
        }
        out
    }
}

/// Trim blank rows from both ends of `lines` — used at flush time so
/// a reply doesn't open or close on an empty row in scrollback.
fn drop_leading_and_trailing_blanks(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines
        .first()
        .map(super::components::line_is_empty)
        .unwrap_or(false)
    {
        lines.remove(0);
    }
    while lines
        .last()
        .map(super::components::line_is_empty)
        .unwrap_or(false)
    {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_get_marker_and_bold() {
        let lines = render("# Title\n## Sub\n### Small");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans.len() >= 2);
    }

    #[test]
    fn bold_italic_and_code_inline() {
        let lines = render("this is **bold** and *italic* and `code`");
        assert_eq!(lines.len(), 1);
        let styled: Vec<_> = lines[0]
            .spans
            .iter()
            .filter(|s| {
                s.style.add_modifier.contains(Modifier::BOLD)
                    || s.style.add_modifier.contains(Modifier::ITALIC)
                    || s.style.bg.is_some()
            })
            .collect();
        assert_eq!(styled.len(), 3);
    }

    #[test]
    fn unclosed_bold_renders_literally() {
        let lines = render("hello **world");
        assert_eq!(lines.len(), 1);
        assert!(!lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn fenced_code_block_syntax_highlights() {
        let lines = render("before\n```rust\nfn main() {}\n```\nafter");
        // before, ` rust`, `▎ fn main() {}` styled, after
        assert!(lines.len() >= 4);
        assert!(lines[1].spans.iter().any(|s| s.content.contains("rust")));
        // No closing "end" marker should ever land in the transcript —
        // it used to render as bleed-through text when the box chars
        // rendered narrow.
        assert!(lines
            .iter()
            .all(|l| l.spans.iter().all(|s| !s.content.contains("end"))));
        // The code line should contain some highlighted spans (>1
        // span means syntect broke it into tokens) and lead with the
        // solid gutter mark.
        let code_line = lines.iter().find(|l| {
            l.spans
                .iter()
                .any(|s| s.content.contains("main") || s.content.contains("fn"))
        });
        assert!(code_line.is_some());
        assert!(code_line.unwrap().spans[0].content.starts_with('▎'));
    }

    #[test]
    fn bullets_get_bullet_marker() {
        let lines = render("- one\n- two");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans.iter().any(|s| s.content.contains('•')));
    }

    #[test]
    fn numbered_list_keeps_number() {
        let lines = render("1. first\n2. second");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans.iter().any(|s| s.content.contains("1.")));
    }

    #[test]
    fn nested_bullets_indent() {
        // Two-space indent under a top-level bullet.
        let lines = render("- outer\n  - inner");
        assert_eq!(lines.len(), 2);
        // Inner line's first span is the indent padding.
        let first = &lines[1].spans[0];
        assert!(!first.content.is_empty());
        assert!(first.content.chars().all(|c| c == ' '));
    }

    #[test]
    fn hash_without_space_is_not_a_header() {
        let lines = render("#tag not a header");
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].spans[0].content.starts_with('▎'));
    }

    #[test]
    fn blockquote_renders_with_bar() {
        let lines = render("> quoted text");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('┃'));
    }

    #[test]
    fn nested_blockquote_stacks_bars() {
        let lines = render("> > double");
        assert_eq!(lines.len(), 1);
        let bar_count = lines[0]
            .spans
            .iter()
            .filter(|s| s.content.contains('┃'))
            .count();
        assert_eq!(bar_count, 2);
    }

    #[test]
    fn link_shows_text_and_url() {
        let lines = render("see [docs](https://example.com) for more");
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.content == "docs" && s.style.add_modifier.contains(Modifier::UNDERLINED)));
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.content.contains("example.com")));
    }

    #[test]
    fn unclosed_link_renders_literally() {
        let lines = render("see [docs](");
        assert!(!lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)));
    }

    #[test]
    fn pipe_table_renders_with_borders() {
        let md = "| name | ctx |\n|------|-----|\n| gpt | 128k |\n| claude | 200k |";
        let lines = render(md);
        // top border + header + separator + 2 body + bottom border = 6
        assert_eq!(lines.len(), 6);
        // First line should be a box-drawing top border.
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.content.contains('┌') || s.content.contains('─')));
        // Body cell content should appear.
        let has_gpt = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("gpt")));
        assert!(has_gpt);
    }

    #[test]
    fn pipe_line_without_separator_is_not_a_table() {
        let md = "| this is a bar chart | 42 |";
        let lines = render(md);
        assert_eq!(lines.len(), 1);
        // Should render as plain text, not table rows.
        assert!(!lines[0].spans.iter().any(|s| s.content.contains('┌')));
    }
}
