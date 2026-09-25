//! Composer — the input area at the bottom of the screen.
//!
//! Visual contract: a single coral hairline on top (no full box — one
//! border row, not two), the `▸▸` prompt inline with the text the user
//! is typing, and collapsed pastes rendered as `[pasted N lines]`
//! chips.
//!
//! Wrapping is owned here, not delegated to `Paragraph::wrap`: the
//! composer pre-splits input into visual rows with a greedy word wrap
//! and places the terminal caret using the *same* layout, so the caret
//! can never drift from the text on wrapped lines. Long input scrolls
//! inside the capped area, bottom-anchored — the caret and the queued
//! lines below it always stay visible.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::components::{CREAM, DIM, HAIRLINE, MUTED, SALMON};
use crate::tui::inline_term::Frame;
use crate::tui::state::{PasteChunk, TuiState};

/// Prompt prefix rendered inline on the first input line. `▸` is
/// U+25B8 (narrow triangle) so it lays out as one column each in a
/// monospace font — the whole prefix is three columns wide.
const PROMPT: &str = "▸▸ ";
const PROMPT_COLS: u16 = 3;
/// Continuation-line indent (both wrapped rows and new logical lines)
/// so everything aligns under the first character after `▸▸ `.
const PROMPT_CONT: &str = "   ";

/// Body rows the input may occupy before it starts scrolling
/// internally (bottom-anchored, caret pinned).
pub(crate) const INPUT_CAP: u16 = 8;
/// Queued messages shown under the input before collapsing.
const QUEUED_CAP: usize = 3;

// ---- atoms: the display units a logical line wraps on ----

#[derive(Clone, Copy, PartialEq)]
enum AtomKind {
    /// Ordinary text — rendered cream.
    Text,
    /// A `[[paste:N]]` chip — rendered salmon, never split.
    Chip,
}

struct Atom {
    /// Display text (for chips: the resolved ` [pasted N lines] `
    /// label, spaces included — the chip is one unbreakable unit).
    text: String,
    kind: AtomKind,
    /// Display width in terminal columns.
    width: usize,
    /// Char index of this atom's first char within its logical line.
    start_char: usize,
}

/// One visual row of wrapped input: its atoms plus the char range of
/// the logical line it covers. `end_char` includes wrap-consumed
/// spaces, so a caret sitting in them maps to this row's end.
struct WrappedRow {
    atoms: Vec<Atom>,
    start_char: usize,
    end_char: usize,
}

fn atom(text: String, kind: AtomKind, start_char: usize) -> Atom {
    Atom {
        width: text.width(),
        text,
        kind,
        start_char,
    }
}

/// Chip display label for a paste placeholder found in the input.
/// `id` is `None` for malformed tokens; a known id renders the live
/// stash's line count, an unknown one degrades to `[pasted ?]`.
fn chip_display(id: Option<u32>, pastes: &[PasteChunk]) -> String {
    match id.and_then(|id| pastes.iter().find(|p| p.id == id)) {
        Some(p) => format!(
            " [pasted {} line{}] ",
            p.lines,
            if p.lines == 1 { "" } else { "s" }
        ),
        None => " [pasted ?] ".to_owned(),
    }
}

/// Split a logical line into wrap units: paste chips are atomic,
/// plain text splits into word / space runs (display-width measured).
fn tokenize(line: &str, pastes: &[PasteChunk]) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::new();
    let mut rest = line;
    let mut char_off = 0usize;
    while !rest.is_empty() {
        if let Some(token_end) = paste_token_end(rest) {
            let token = &rest[..token_end];
            let id = token["[[paste:".len()..token.len() - 2].parse::<u32>().ok();
            out.push(atom(chip_display(id, pastes), AtomKind::Chip, char_off));
            char_off += token.chars().count();
            rest = &rest[token_end..];
            continue;
        }
        // Plain text run up to the next chip (or end of line). An
        // unclosed `[[paste:` here is plain text: search past it, or this
        // loop would never advance.
        let from = usize::from(rest.starts_with("[[paste:"));
        let next = rest[from..]
            .find("[[paste:")
            .map_or(rest.len(), |i| i + from);
        let mut r = &rest[..next];
        while !r.is_empty() {
            let split = if r.starts_with(char::is_whitespace) {
                r.find(|c: char| !c.is_whitespace()).unwrap_or(r.len())
            } else {
                r.find(char::is_whitespace).unwrap_or(r.len())
            };
            let (piece, tail) = r.split_at(split);
            out.push(atom(piece.to_owned(), AtomKind::Text, char_off));
            char_off += piece.chars().count();
            r = tail;
        }
        if next == rest.len() {
            break;
        }
        rest = &rest[next..];
    }
    out
}

/// `Some(end)` when `s` starts with a closed `[[paste:…]]` token. The id
/// may be malformed; [`chip_display`] shows those as `[pasted ?]`.
fn paste_token_end(s: &str) -> Option<usize> {
    let after = s.strip_prefix("[[paste:")?;
    let end = after.find("]]")?;
    Some("[[paste:".len() + end + 2)
}

/// Greedy word wrap over atoms. Rows never exceed `max` display
/// columns; an unbreakable text atom wider than `max` is hard-split
/// (paste chips are never split). A
/// space run that straddles the boundary is consumed by the wrap but
/// stays inside the previous row's char range for caret mapping.
fn wrap_atoms(atoms: Vec<Atom>, max: usize) -> Vec<WrappedRow> {
    let mut rows: Vec<WrappedRow> = Vec::new();
    let mut cur = WrappedRow {
        atoms: Vec::new(),
        start_char: 0,
        end_char: 0,
    };
    let mut w = 0usize;

    macro_rules! flush {
        () => {
            if !cur.atoms.is_empty() || rows.is_empty() {
                rows.push(WrappedRow {
                    atoms: std::mem::take(&mut cur.atoms),
                    start_char: cur.start_char,
                    end_char: cur.end_char,
                });
            }
        };
    }

    for a in atoms {
        let aw = a.width;
        let chars = a.text.chars().count();
        if w + aw <= max {
            cur.end_char = a.start_char + chars;
            cur.atoms.push(a);
            w += aw;
        } else if a.kind == AtomKind::Text && a.text.chars().all(char::is_whitespace) {
            // Consumed by the wrap — rendered nowhere, but the caret
            // can sit inside it, so it belongs to the trailing row.
            cur.end_char = a.start_char + chars;
            flush!();
            w = 0;
            cur.start_char = a.start_char + chars;
            cur.end_char = cur.start_char;
        } else if aw <= max || a.kind == AtomKind::Chip {
            // Chips never split. One wider than the row keeps its own row
            // with its padding dropped (the terminal clips any excess).
            let a = if aw > max {
                atom(a.text.trim().to_owned(), a.kind, a.start_char)
            } else {
                a
            };
            let aw = a.width;
            flush!();
            cur.start_char = a.start_char;
            cur.end_char = a.start_char + chars;
            cur.atoms.push(a);
            w = aw;
        } else {
            // Hard-split an overlong atom (a long URL, say) across rows.
            flush!();
            let mut col = 0usize;
            let mut chunk_start = a.start_char;
            let mut chunk_start_char = a.start_char;
            let mut chunk = String::new();
            for c in a.text.chars() {
                let cw = c.width().unwrap_or(0);
                if col + cw > max && !chunk.is_empty() {
                    let n = chunk.chars().count();
                    rows.push(WrappedRow {
                        atoms: vec![atom(std::mem::take(&mut chunk), a.kind, chunk_start_char)],
                        start_char: chunk_start,
                        end_char: chunk_start + n,
                    });
                    chunk_start += n;
                    chunk_start_char += n;
                    col = 0;
                }
                chunk.push(c);
                col += cw;
            }
            if !chunk.is_empty() {
                let n = chunk.chars().count();
                cur.start_char = chunk_start;
                cur.end_char = chunk_start + n;
                cur.atoms = vec![atom(chunk, a.kind, chunk_start_char)];
                w = cur.atoms.last().map(|x| x.width).unwrap_or(0);
            }
        }
    }
    flush!();
    rows
}

// ---- geometry (height calc lives in render/mod.rs) ----

/// Wrapped visual rows the composer body needs for the current input,
/// at `region_width`. Uncapped — the caller applies [`INPUT_CAP`].
pub(crate) fn body_rows(state: &TuiState, region_width: u16) -> u16 {
    layout(state, region_width).rows.len() as u16
}

/// Queued-message rows rendered under the input (capped, with an
/// overflow line).
pub(crate) fn queued_rows(state: &TuiState) -> u16 {
    (state.queued.len().min(QUEUED_CAP) as u16) + u16::from(state.queued.len() > QUEUED_CAP)
}

/// Full composer layout: visual rows (span-ready, prefixes applied)
/// plus the caret's (row, col) in the same coordinate space.
pub(crate) struct ComposerLayout {
    pub rows: Vec<Vec<Span<'static>>>,
    pub caret: (u16, u16),
}

pub(crate) fn layout(state: &TuiState, region_width: u16) -> ComposerLayout {
    // Empty composer: one placeholder row ("message"), caret at origin.
    if state.input().is_empty() {
        return ComposerLayout {
            rows: vec![vec![
                Span::styled(PROMPT.to_owned(), Style::default().fg(SALMON()).bold()),
                Span::styled("message", Style::default().fg(DIM()).italic()),
            ]],
            caret: (0, 0),
        };
    }
    let wrap_width = region_width.saturating_sub(PROMPT_COLS).max(1) as usize;
    let input = state.input();
    let cursor_byte = state.cursor();

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut caret: Option<(u16, u16)> = None;
    let mut byte_off = 0usize;

    for (li, logical) in input.split('\n').enumerate() {
        let line_rows = wrap_atoms(tokenize(logical, &state.pastes), wrap_width);
        let base = rows.len();

        // Does the caret sit in this logical line?
        if caret.is_none() && cursor_byte >= byte_off && cursor_byte <= byte_off + logical.len() {
            let cursor_char = input[byte_off..cursor_byte].chars().count();
            // The last row starting at or before the cursor — rows
            // partition the line's char space (end-inclusive), so this
            // is the row typing would continue on.
            let picked = line_rows
                .iter()
                .rposition(|row| row.start_char <= cursor_char)
                .unwrap_or(0);
            let mut col = 0usize;
            for a in &line_rows[picked].atoms {
                let atom_end = a.start_char + a.text.chars().count();
                if atom_end <= cursor_char {
                    col += a.width;
                } else {
                    // Only the part of the atom before the cursor.
                    let before = cursor_char.saturating_sub(a.start_char);
                    col += a
                        .text
                        .chars()
                        .take(before)
                        .map(|c| c.width().unwrap_or(0))
                        .sum::<usize>();
                    break;
                }
            }
            caret = Some(((base + picked) as u16, col as u16));
        }

        for (ri, row) in line_rows.iter().enumerate() {
            let (prefix, style) = if li == 0 && ri == 0 {
                (PROMPT, Style::default().fg(SALMON()).bold())
            } else {
                (PROMPT_CONT, Style::default())
            };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(prefix.to_owned(), style)];
            for a in &row.atoms {
                spans.push(render_atom(a));
            }
            rows.push(spans);
        }

        byte_off += logical.len() + 1; // +1 for the '\n'
    }

    let caret = caret.unwrap_or(((rows.len() as u16).saturating_sub(1), 0));
    ComposerLayout { rows, caret }
}

fn render_atom(a: &Atom) -> Span<'static> {
    match a.kind {
        AtomKind::Text => Span::styled(a.text.clone(), Style::default().fg(CREAM())),
        AtomKind::Chip => Span::styled(
            a.text.clone(),
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(SALMON())
                .add_modifier(Modifier::BOLD),
        ),
    }
}

pub(crate) fn composer(f: &mut Frame, area: Rect, state: &TuiState) {
    let border_color = if state.streaming { MUTED() } else { HAIRLINE() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let lay = layout(state, area.width.saturating_sub(2));

    // Queued messages render under the input as dim `+ queued:` lines.
    let mut body: Vec<Line<'static>> = lay.rows.into_iter().map(Line::from).collect();
    for msg in state.queued.iter().take(QUEUED_CAP) {
        let first = msg.lines().next().unwrap_or("");
        body.push(Line::from(vec![
            Span::styled("  + ", Style::default().fg(SALMON()).bold()),
            Span::styled(
                crate::tui::components::truncate(&format!("queued: {first}"), 200),
                Style::default().fg(MUTED()),
            ),
        ]));
    }
    if state.queued.len() > QUEUED_CAP {
        body.push(Line::from(Span::styled(
            format!("  … {} more queued", state.queued.len() - QUEUED_CAP),
            Style::default().fg(DIM()).italic(),
        )));
    }

    // Bottom-anchor: when the body exceeds the capped area, scroll so
    // the caret (and the queued lines below it) stay visible.
    let visible = area.height.saturating_sub(2); // minus top + bottom border
    let scroll = (body.len() as u16).saturating_sub(visible);

    let text = Text::from(body);
    f.render_widget(Paragraph::new(text).block(block).scroll((scroll, 0)), area);

    // Position the terminal caret. Body starts at +1 row (top border)
    // and +1 col (left border); every row carries the 3-col prefix.
    let (row, col) = lay.caret;
    f.set_cursor_position((
        area.x + 1 + PROMPT_COLS + col,
        area.y + 1 + row.saturating_sub(scroll),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(input: &str, cursor_chars: usize) -> TuiState {
        let mut s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        s.input_replace(input);
        s.move_line_start(); // input_replace leaves the cursor at the end
        let target = cursor_chars.min(input.chars().count());
        while s.input()[..s.cursor()].chars().count() < target {
            s.move_right();
        }
        s
    }

    fn row_text(lay: &ComposerLayout) -> Vec<String> {
        lay.rows
            .iter()
            .map(|r| r.iter().map(|sp| sp.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn empty_input_shows_placeholder() {
        let s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let lay = layout(&s, 80);
        assert_eq!(lay.rows.len(), 1);
        let row: String = lay.rows[0].iter().map(|sp| sp.content.clone()).collect();
        assert!(row.contains("message"), "{row}");
        assert_eq!(lay.caret, (0, 0));
    }

    #[test]
    fn single_short_line_is_one_row() {
        let s = st("hello", 5);
        let lay = layout(&s, 80);
        assert_eq!(lay.rows.len(), 1);
        assert_eq!(lay.caret, (0, 5));
    }

    #[test]
    fn long_line_wraps_and_caret_follows() {
        let s = st("the quick brown fox jumps over the lazy dog", 43);
        let lay = layout(&s, 30);
        assert!(lay.rows.len() >= 2, "must wrap at width 30");
        assert_eq!(lay.caret.0, lay.rows.len() as u16 - 1);
        assert!(lay.caret.1 > 0);
    }

    #[test]
    fn caret_mid_text_maps_into_wrapped_row() {
        let s = st("the quick brown fox", 6); // "the qu|ick"
        let lay = layout(&s, 12);
        assert!(lay.rows.len() >= 2);
        // Mid-word: only the part of "quick" before the caret counts.
        assert_eq!(lay.caret, (0, 6));
    }

    #[test]
    fn caret_on_wrapped_continuation_row() {
        let s = st("the quick brown fox", 19);
        let lay = layout(&s, 12);
        assert_eq!(lay.caret.0, (lay.rows.len() - 1) as u16);
    }

    #[test]
    fn multibyte_counts_display_columns() {
        let mut s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        s.input_replace("héllo 好");
        s.move_line_start();
        while s.input()[..s.cursor()].chars().count() < 6 {
            s.move_right();
        }
        let lay = layout(&s, 80);
        assert_eq!(lay.caret, (0, 6)); // 6 display cols, 7 bytes
    }

    #[test]
    fn trailing_newline_gets_caret_row() {
        let mut s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        s.input_replace("one\n");
        let lay = layout(&s, 80);
        assert_eq!(lay.rows.len(), 2);
        assert_eq!(lay.caret.0, 1);
    }

    #[test]
    fn paste_chip_is_atomic() {
        let mut s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let token = s.stash_paste("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n".into());
        s.input_replace(&format!("before{token}"));
        let lay = layout(&s, 20);
        let rows = row_text(&lay);
        assert!(
            rows.iter().any(|r| r.contains("[pasted 12 lines]")),
            "resolved chip must render: {rows:?}"
        );
        // Chip never split across rows.
        assert!(rows.iter().filter(|r| r.contains("pasted")).count() == 1);
    }

    #[test]
    fn overlong_word_hard_splits() {
        let word = "y".repeat(50);
        let s = st(&format!("x{word}"), 51);
        let lay = layout(&s, 20);
        assert!(lay.rows.len() >= 3, "51-col run must split at width 20");
    }

    #[test]
    fn body_rows_counts_wrapping() {
        let s = st("aaa bbb ccc ddd eee fff ggg hhh", 31);
        assert!(body_rows(&s, 20) > 1);
        assert_eq!(body_rows(&st("hi", 2), 20), 1);
    }

    #[test]
    fn queued_rows_cap_and_overflow() {
        let mut s = TuiState::new("m".into(), mira_policy::Mode::Manual);
        assert_eq!(queued_rows(&s), 0);
        for i in 0..3 {
            s.queue_message(format!("m{i}"));
        }
        assert_eq!(queued_rows(&s), 3);
        s.queue_message("m3".into());
        s.queue_message("m4".into());
        assert_eq!(queued_rows(&s), 4); // 3 shown + overflow line
    }

    #[test]
    fn malformed_paste_token_degrades_to_question_mark() {
        let s = st("a [[paste:xx]] b", 16);
        let lay = layout(&s, 80);
        let rows = row_text(&lay);
        assert!(rows.iter().any(|r| r.contains("[pasted ?]")),);
    }
}
