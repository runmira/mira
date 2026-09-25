//! Language-agnostic single-pass code tokenizer.
//!
//! Applied to tool-result snippets, expanded tool output, and any other
//! place where code-shaped text lands without an explicit language tag
//! (so `markdown::highlight_code_line` — which needs a filename or
//! fenced language — can't help).
//!
//! Trade-off: the tokenizer intentionally knows nothing about grammar.
//! It shades strings, numbers, comments, and a small pan-language
//! keyword set — enough to make a `grep` hit or a `read_file` snippet
//! read as code instead of a wall of grey. Anything that fights the
//! grammar (heredocs, raw strings, template literals) simply falls back
//! to plain text — never garbled colours.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Span;

use super::components::{DIM, MUTED};

/// Tint palette. Fixed RGBs (not theme entries) so tool output reads
/// the same across themes — the theme owns chrome, this owns semantics.
const KW_FG: Color = Color::Rgb(214, 155, 214); // muted violet — keywords
const STR_FG: Color = Color::Rgb(196, 168, 128); // warm sand — strings
const NUM_FG: Color = Color::Rgb(214, 186, 140); // warm sand-lite — numbers
const IDENT_FG: Color = Color::Rgb(210, 210, 214); // near-white for identifiers

/// A short cross-language keyword set. Deliberately small — matching
/// too aggressively (e.g. `type` as a keyword) discolours identifiers
/// in every JSON blob or shell output. This set covers what a reader
/// scanning a snippet needs to spot control flow / declarations at a
/// glance, and nothing else.
const KEYWORDS: &[&str] = &[
    // Rust / C-like
    "fn",
    "let",
    "mut",
    "const",
    "static",
    "pub",
    "use",
    "mod",
    "struct",
    "enum",
    "trait",
    "impl",
    "match",
    "return",
    "self",
    "async",
    "await",
    "move",
    "ref",
    "as",
    "where",
    // Control flow (shared across many langs)
    "if",
    "else",
    "for",
    "while",
    "loop",
    "break",
    "continue",
    "in",
    "of",
    "do",
    "then",
    // Python / JS / TS-ish
    "def",
    "class",
    "import",
    "from",
    "yield",
    "lambda",
    "function",
    "var",
    "typeof",
    "instanceof",
    "new",
    "try",
    "catch",
    "finally",
    "throw",
    // Truth values
    "true",
    "false",
    "null",
    "None",
    "True",
    "False",
    "nil",
    // Common shell
    "echo",
    "exit",
];

/// Tokenize + colour one line. Returns Spans ready to hand to a
/// `Line`. Multi-line strings / comments are NOT tracked across calls —
/// each line stands alone, which is the correct policy for a snippet
/// that may already be truncated at either edge.
pub(crate) fn highlight_line(text: &str) -> Vec<Span<'static>> {
    if text.is_empty() {
        return Vec::new();
    }
    let default = Style::default().fg(MUTED());
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut chars = text.char_indices().peekable();

    // Emit the plain-run buffer as one span with the default style.
    // Called before switching into a coloured span.
    let flush = |out: &mut Vec<Span<'static>>, buf: &mut String| {
        if !buf.is_empty() {
            out.push(Span::styled(std::mem::take(buf), default));
        }
    };

    while let Some((i, c)) = chars.next() {
        // Line comments: `//` (Rust/C/JS), `#` (shell/Python) at start
        // of a token — treat everything remaining as comment. Skipping
        // when the `#` looks like a colour literal (`#ff8080`) or a
        // markdown heading isn't worth the complexity here; tool-result
        // context makes false positives cheap.
        if c == '/' && matches!(chars.peek(), Some((_, '/'))) {
            flush(&mut out, &mut buf);
            chars.next(); // the second `/`, already in `rest`
            let mut rest = String::from("//");
            for (_, ch) in chars.by_ref() {
                rest.push(ch);
            }
            out.push(Span::styled(rest, Style::default().fg(DIM()).italic()));
            return out;
        }
        // `#` starts a comment only at the start of a token (line start or
        // after whitespace): `abc#tag` and URL fragments stay plain.
        if c == '#' && (i == 0 || text[..i].ends_with(char::is_whitespace)) {
            flush(&mut out, &mut buf);
            let mut rest = String::from("#");
            for (_, ch) in chars.by_ref() {
                rest.push(ch);
            }
            out.push(Span::styled(rest, Style::default().fg(DIM()).italic()));
            return out;
        }

        // Quoted strings — greedy up to a matching unescaped quote on
        // the same line. Backslash escapes the next char so `"\""` is
        // ONE string, not two adjacent empty ones.
        if c == '"' || c == '\'' || c == '`' {
            flush(&mut out, &mut buf);
            let quote = c;
            let mut s = String::from(c);
            let mut escaped = false;
            let mut closed = false;
            for (_, ch) in chars.by_ref() {
                s.push(ch);
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == quote {
                    closed = true;
                    break;
                }
            }
            // Unclosed string on this line → still colour it, so the
            // reader sees "yes, this ran off the edge" instead of
            // untinted plaintext.
            let style = Style::default().fg(STR_FG);
            let _ = closed;
            out.push(Span::styled(s, style));
            continue;
        }

        // Numbers — a run of digits (optionally with a decimal or
        // trailing units letter like `k`, `ms`). Only recognise when
        // preceded by non-alphanumeric so we don't tint `abc123` as
        // partial-number.
        if c.is_ascii_digit() && !last_char_is_ident(&buf) {
            flush(&mut out, &mut buf);
            let mut n = String::from(c);
            while let Some((_, ch)) = chars.peek().copied() {
                if ch.is_ascii_digit() || ch == '.' || ch == '_' {
                    n.push(ch);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(Span::styled(n, Style::default().fg(NUM_FG)));
            continue;
        }

        // Identifiers / keywords — a run of alphanumerics + `_`. Look
        // it up in KEYWORDS on close; otherwise fall through to
        // default-styled identifier tinting.
        if c.is_alphabetic() || c == '_' {
            flush(&mut out, &mut buf);
            let mut w = String::from(c);
            while let Some((_, ch)) = chars.peek().copied() {
                if ch.is_alphanumeric() || ch == '_' {
                    w.push(ch);
                    chars.next();
                } else {
                    break;
                }
            }
            let is_kw = KEYWORDS.iter().any(|k| *k == w);
            let style = if is_kw {
                Style::default().fg(KW_FG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(IDENT_FG)
            };
            out.push(Span::styled(w, style));
            continue;
        }

        // Everything else (whitespace, punctuation, symbols) — buffer
        // to a plain-styled span so we don't emit one span per char.
        buf.push(c);
    }
    flush(&mut out, &mut buf);
    out
}

fn last_char_is_ident(buf: &str) -> bool {
    buf.chars()
        .last()
        .map(|c| c.is_alphanumeric() || c == '_')
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn styles(spans: &[Span<'_>]) -> Vec<(String, Option<Color>)> {
        spans
            .iter()
            .map(|s| (s.content.to_string(), s.style.fg))
            .collect()
    }

    #[test]
    fn empty_stays_empty() {
        assert!(highlight_line("").is_empty());
    }

    #[test]
    fn preserves_text_round_trip() {
        let src = "let x = \"hi\"; // note";
        assert_eq!(joined(&highlight_line(src)), src);
    }

    #[test]
    fn tints_keyword_and_string_distinctly() {
        let spans = highlight_line("let name = \"aster\";");
        let s = styles(&spans);
        // `let` gets KW_FG; `"aster"` gets STR_FG; the identifier
        // `name` gets IDENT_FG. Three distinct colours in one line.
        let fgs: Vec<Color> = s.iter().filter_map(|(_, f)| *f).collect();
        assert!(fgs.contains(&KW_FG), "keyword fg missing: {fgs:?}");
        assert!(fgs.contains(&STR_FG), "string fg missing: {fgs:?}");
        assert!(fgs.contains(&IDENT_FG), "ident fg missing: {fgs:?}");
    }

    #[test]
    fn hash_at_line_start_is_comment() {
        let spans = highlight_line("# a shell comment");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(DIM()));
    }

    #[test]
    fn hash_after_text_is_not_comment() {
        // `abc#tag` is plaintext (matches URL fragments, colours).
        let spans = highlight_line("abc#tag");
        assert!(spans.iter().all(|s| s.style.fg != Some(DIM())));
    }

    #[test]
    fn slash_slash_comment_tinted_dim() {
        let spans = highlight_line("let x = 1; // trailing");
        assert!(spans
            .iter()
            .any(|s| s.content.starts_with("//") && s.style.fg == Some(DIM())));
    }

    #[test]
    fn number_needs_word_boundary() {
        // `abc123` is one identifier — no number tint.
        let spans = highlight_line("abc123");
        assert!(spans.iter().all(|s| s.style.fg != Some(NUM_FG)));

        // `x 123` gives a number.
        let spans = highlight_line("x 123");
        assert!(spans.iter().any(|s| s.style.fg == Some(NUM_FG)));
    }

    #[test]
    fn unclosed_string_still_tints() {
        let spans = highlight_line("msg = \"never closed");
        assert!(spans.iter().any(|s| s.style.fg == Some(STR_FG)));
    }

    #[test]
    fn escaped_quote_keeps_one_string() {
        let spans = highlight_line("s = \"a \\\" b\";");
        let strs: Vec<&Span<'_>> = spans
            .iter()
            .filter(|s| s.style.fg == Some(STR_FG))
            .collect();
        assert_eq!(strs.len(), 1);
        assert!(strs[0].content.contains("\\\""));
    }
}
