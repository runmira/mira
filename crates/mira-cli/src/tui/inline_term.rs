//! Inline-viewport terminal with a dynamic-height pane pinned to the bottom
//! of the screen and a real scrollback above it. The cursor is queried once
//! at startup and never re-anchored — everything else is bookkept in Rust
//! state so a growing composer, a settling turn, or a terminal resize can
//! move the pane without ever asking the terminal where it is.
//!
//! Why this exists at all: ratatui's own `Viewport::Inline` calls
//! `get_cursor_position` on every `resize`, which races the `EventStream`
//! background reader for the internal-event mutex and times out with
//! "cursor position could not be read within a normal duration". Rolling our
//! own inline pane skips that whole class of races.
//!
//! Structure: the pane is a [`Rect`] we track locally. [`Transcript`] counts
//! every row that has been printed above it so a rewrap can be replayed row
//! by row after a window resize instead of guessed. [`set_height`] moves the
//! pane's top boundary — growing borrows rows from the transcript (they
//! scroll into real scrollback), shrinking hands rows back to the empty
//! screen below.

use std::collections::VecDeque;
use std::io::{self, Stdout, Write};

use anyhow::Result;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use ratatui::widgets::{StatefulWidget, Widget};
use unicode_width::UnicodeWidthStr;

/// Anything wider than this makes prose unreadable, so both the pane and the
/// scrollback blocks cap here even if the terminal is wider.
const MAX_WIDTH: u16 = 240;

/// The one width every layer measures against — pane, scrollback blocks,
/// desired height. If two layers disagreed we would get a blank row or a
/// clipped one, so this is the single source of truth.
fn content_width(screen: Size) -> u16 {
    screen.width.clamp(1, MAX_WIDTH)
}

/// The API surface a `render::draw` closure needs. Mirrors the pieces of
/// `ratatui::Frame` we actually use — `area`, `buffer_mut`,
/// `render_widget`, `render_stateful_widget`, `set_cursor_position` — so
/// existing render code compiles unchanged against this type. Constructed
/// only by [`InlineTerm::draw`] and (in tests) [`Frame::from_parts`].
pub(crate) struct Frame<'a> {
    area: Rect,
    buf: &'a mut Buffer,
    cursor: &'a mut Option<Position>,
}

impl<'a> Frame<'a> {
    /// Test-only constructor so tests can drive `render::draw` against a
    /// buffer they built themselves (typically via `TestBackend`).
    #[cfg(test)]
    pub(crate) fn from_parts(
        area: Rect,
        buf: &'a mut Buffer,
        cursor: &'a mut Option<Position>,
    ) -> Self {
        Self { area, buf, cursor }
    }

    pub(crate) fn area(&self) -> Rect {
        self.area
    }

    #[allow(dead_code)]
    pub(crate) fn buffer_mut(&mut self) -> &mut Buffer {
        self.buf
    }

    pub(crate) fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buf);
    }

    pub(crate) fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        area: Rect,
        state: &mut W::State,
    ) {
        widget.render(area, self.buf, state);
    }

    pub(crate) fn set_cursor_position<P: Into<Position>>(&mut self, pos: P) {
        *self.cursor = Some(pos.into());
    }
}

pub(crate) struct InlineTerm<W: Write = Stdout> {
    backend: CrosstermBackend<W>,
    buffers: [Buffer; 2],
    current: usize,
    viewport: Rect,
    screen: Size,
    transcript: Transcript,
    /// Last row we parked the caret on. On a shrinking window this is how
    /// far we scrolled to keep the caret visible, i.e. how much transcript
    /// slid out from under us.
    cursor_row: u16,
}

/// Bookkeeping for every row printed above the pane. Each entry stores the
/// column width at which the row was printed (spaces at the tail don't
/// count) so a later rewrap at a different width can be computed instead of
/// guessed. `banked` splits rows already scrolled into real scrollback from
/// those still on screen; `consumed` tracks the column-level offset into the
/// first still-on-screen row (a rewrap can start mid-row).
#[derive(Default)]
struct Transcript {
    rows: VecDeque<u16>,
    /// Number of leading `rows` that have scrolled into real scrollback.
    banked: usize,
    /// Columns of the first still-on-screen row already off the top.
    /// Terminals wrap whole rows, so a narrower window can land the top of
    /// the visible area part-way through one — we hold that boundary in
    /// columns to avoid a row of drift per rewrap.
    consumed: u16,
}

/// Cap on rows kept. Only the on-screen tail decides layout and only the
/// rows immediately above it can ever come back after an `unbank`, so older
/// rows are dropped. No real terminal is anywhere near this tall.
const MAX_TRACKED: usize = 4096;

/// How many screen rows one transcript row of `used` printed columns takes
/// when reflowed at `width`. Empty rows still occupy one.
fn height(used: u16, width: u16) -> u16 {
    used.div_ceil(width.max(1)).max(1)
}

impl Transcript {
    fn push(&mut self, used: u16) {
        self.rows.push_back(used);
        while self.rows.len() > MAX_TRACKED {
            self.rows.pop_front();
            match self.banked.checked_sub(1) {
                Some(left) => self.banked = left,
                None => self.consumed = 0,
            }
        }
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.rows.clear();
        self.banked = 0;
        self.consumed = 0;
    }

    /// Screen rows the on-screen part covers when printed at `width`.
    fn on_screen(&self, width: u16) -> u16 {
        self.rows
            .iter()
            .enumerate()
            .skip(self.banked)
            .map(|(i, used)| {
                let rows = height(*used, width);
                match i == self.banked {
                    true => rows - self.off_screen(rows, width),
                    false => rows,
                }
            })
            .fold(0u16, u16::saturating_add)
    }

    /// Rows of the first on-screen entry that are above the top of the screen.
    fn off_screen(&self, rows: u16, width: u16) -> u16 {
        (self.consumed / width.max(1)).min(rows)
    }

    /// Account for `rows` screen rows having scrolled off the top.
    fn bank(&mut self, rows: u16, width: u16) {
        let mut left = rows;
        while left > 0 && self.banked < self.rows.len() {
            let total = height(self.rows[self.banked], width);
            let showing = total - self.off_screen(total, width);
            if left < showing {
                self.consumed += left * width.max(1);
                return;
            }
            left -= showing;
            self.banked += 1;
            self.consumed = 0;
        }
    }

    /// Account for `rows` screen rows having come back out of scrollback.
    #[allow(dead_code)]
    fn unbank(&mut self, rows: u16, width: u16) {
        let mut left = rows;
        while left > 0 {
            let hidden = self.consumed / width.max(1);
            if hidden > 0 {
                let back = left.min(hidden);
                self.consumed -= back * width.max(1);
                left -= back;
                continue;
            }
            let Some(prev) = self.banked.checked_sub(1) else {
                return;
            };
            self.banked = prev;
            let total = height(self.rows[prev], width);
            if left >= total {
                left -= total;
                self.consumed = 0;
            } else {
                self.consumed = (total - left) * width.max(1);
                return;
            }
        }
    }
}

impl InlineTerm<Stdout> {
    /// Anchor the pane at the current cursor position. This is the ONE place
    /// we call `get_cursor_position`, and it must run before the event loop
    /// starts an `EventStream` — otherwise the `EventStream` reader thread
    /// consumes the CPR reply and we hang.
    pub(crate) fn new(height: u16) -> Result<Self> {
        let mut backend = CrosstermBackend::new(io::stdout());
        let screen = backend.size()?;
        let anchor = backend.get_cursor_position()?.y;
        Self::anchored(backend, screen, anchor, height)
    }

    /// Called on `Event::Resize` — recomputes the pane position from
    /// tracked transcript rows without asking the terminal anything.
    pub(crate) fn resized(&mut self) -> Result<()> {
        let screen = self.backend.size()?;
        self.resized_to(screen)
    }
}

impl<W: Write> InlineTerm<W> {
    /// Open a pane of `height` rows at the cursor. If the cursor is too far
    /// down for the pane to fit, we scroll the screen up first.
    fn anchored(
        mut backend: CrosstermBackend<W>,
        screen: Size,
        anchor: u16,
        height: u16,
    ) -> Result<Self> {
        let height = clamp_height(height, screen);
        // Open room below the cursor so the viewport fits on screen.
        let below = height.saturating_sub(1);
        backend.append_lines(below)?;
        let overflow = (anchor + height).saturating_sub(screen.height);
        let top = anchor - overflow.min(anchor);

        let viewport = Rect::new(0, top, content_width(screen), height);
        Ok(Self {
            backend,
            buffers: [Buffer::empty(viewport), Buffer::empty(viewport)],
            current: 0,
            viewport,
            screen,
            transcript: Transcript::default(),
            cursor_row: top,
        })
    }

    pub(crate) fn width(&self) -> u16 {
        content_width(self.screen)
    }

    pub(crate) fn viewport_top(&self) -> u16 {
        self.viewport.y
    }

    #[allow(dead_code)]
    pub(crate) fn area(&self) -> Rect {
        self.viewport
    }

    /// Render one frame and flush only the cells that changed since the
    /// last one — same double-buffer diff strategy as ratatui's terminal.
    pub(crate) fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        let mut cursor = None;
        self.buffers[self.current].reset();
        render(&mut Frame {
            area: self.viewport,
            buf: &mut self.buffers[self.current],
            cursor: &mut cursor,
        });

        let previous = &self.buffers[1 - self.current];
        let updates = previous.diff(&self.buffers[self.current]);
        self.backend.draw(updates.into_iter())?;
        match cursor {
            Some(pos) => {
                self.cursor_row = pos.y;
                self.backend.set_cursor_position(pos)?;
                self.backend.show_cursor()?;
            }
            None => {
                self.backend.hide_cursor()?;
                self.park(self.viewport.y)?;
            }
        }
        Backend::flush(&mut self.backend)?;
        self.current = 1 - self.current;
        Ok(())
    }

    /// Move the pane boundary without ever touching the cursor. Growth
    /// takes rows from the transcript above (they scroll into real
    /// scrollback via the DEC scrolling-region escape); shrinking hands
    /// rows back to the empty screen below the pane. The pane never
    /// grows tall (it holds only the composer + footer + one-row
    /// padding), so shrink events are rare and the freed rows are
    /// simply cleared for the next draw.
    pub(crate) fn set_height(&mut self, height: u16) -> Result<()> {
        let height = clamp_height(height, self.screen);
        let old = self.viewport;
        if height == old.height {
            return Ok(());
        }

        let top = old.y.saturating_sub(borrowed(old, height, self.screen));
        self.scroll_into_scrollback(old.y, old.y - top)?;
        if top + height < old.bottom() {
            self.clear_rows(top + height..old.bottom())?;
        }

        self.viewport = Rect::new(0, top, content_width(self.screen), height);
        self.buffers = [Buffer::empty(self.viewport), Buffer::empty(self.viewport)];
        self.clear_rows(self.viewport.y..self.viewport.bottom())?;
        self.park(self.viewport.y)
    }

    /// Push finished lines above the pane, into real terminal scrollback.
    /// The DEC scrolling-region escape puts them there, and `Transcript`
    /// stays in step so a future `set_height` or resize knows they exist.
    pub(crate) fn insert_history(&mut self, cells: &Buffer) -> Result<()> {
        self.track(cells);
        let mut remaining: &[Cell] = &cells.content;
        let stride = cells.area.width;
        let mut height = cells.area.height;

        // If the viewport floats above the bottom, push it down first.
        if self.viewport.bottom() < self.screen.height {
            let to_draw = height.min(self.screen.height - self.viewport.bottom());
            self.backend.scroll_region_down(
                self.viewport.top()..self.viewport.bottom() + to_draw,
                to_draw,
            )?;
            remaining = self.draw_cleared(self.viewport.top(), to_draw, stride, remaining)?;
            self.viewport.y += to_draw;
            for buf in &mut self.buffers {
                buf.area.y = self.viewport.y;
            }
            height -= to_draw;
        }

        let top = self.viewport.top();
        while height > 0 && top > 0 {
            let to_draw = height.min(top);
            self.scroll_into_scrollback(top, to_draw)?;
            remaining = self.draw_cleared(top - to_draw, to_draw, stride, remaining)?;
            height -= to_draw;
        }
        Backend::flush(&mut self.backend)?;
        Ok(())
    }

    fn scroll_into_scrollback(&mut self, region_bottom: u16, rows: u16) -> Result<()> {
        if region_bottom == 0 || rows == 0 {
            return Ok(());
        }
        // DECSTBM + cursor addressing are 1-based; `region_bottom` names the
        // last row of the scrolling region, which is the row just above
        // the pane.
        write!(self.backend, "\x1b[1;{region_bottom}r")?;
        write!(self.backend, "\x1b[{region_bottom};1H")?;
        for _ in 0..rows {
            write!(self.backend, "\r\n")?;
        }
        write!(self.backend, "\x1b[r")?;
        self.transcript.bank(rows, self.width());
        Ok(())
    }

    /// `/clear`: wipe the screen AND the scrollback buffer, re-anchor the
    /// pane at row 0, and drop the stale diff buffers so the next draw
    /// repaints from a known-clean state.
    #[allow(dead_code)]
    pub(crate) fn clear_all(&mut self) -> Result<()> {
        use ratatui::crossterm::cursor::MoveTo;
        use ratatui::crossterm::execute;
        use ratatui::crossterm::terminal::{Clear, ClearType};
        execute!(
            self.backend,
            Clear(ClearType::All),
            Clear(ClearType::Purge),
            MoveTo(0, 0),
        )?;
        self.viewport = Rect::new(0, 0, content_width(self.screen), self.viewport.height);
        self.buffers = [Buffer::empty(self.viewport), Buffer::empty(self.viewport)];
        self.transcript.clear();
        self.cursor_row = 0;
        Ok(())
    }

    /// Terminal window changed size; the terminal has already rewrapped
    /// everything on screen. Wipe the OLD pane's cells (wherever the
    /// rewrap moved them) and re-anchor the pane flush against where
    /// the transcript actually ends. On grow we do NOT unbank rows we
    /// pushed into real scrollback earlier — terminals restore their
    /// own scrollback into the new visible area, and touching our
    /// transcript ledger to match that would double-count.
    pub(crate) fn resized_to(&mut self, screen: Size) -> Result<()> {
        let height = clamp_height(self.viewport.height, screen);
        let width = content_width(screen);
        // Rows of the pane above the caret, once rewrapped: how far to
        // step back up to reach the top of the old frame.
        let above = self.frame_rows_above_caret(width);
        self.erase_last_frame(above)?;
        self.screen = screen;

        // Rewrap accounting: only bank rows that fell off the top when
        // the caret would otherwise be below the last row. Do NOT
        // unbank — see the docstring.
        let caret = self.transcript.on_screen(width).saturating_add(above);
        self.transcript
            .bank(caret.saturating_sub(screen.height - 1), width);
        let end = self.transcript.on_screen(width).min(screen.height);
        // Plant the pane flush against the transcript: right after it
        // when transcript is shorter than the screen, and bottom-
        // anchored (scrolling transcript up) when transcript overflows.
        // With unbank gone, `end` matches what's actually on screen —
        // so the "min" no longer over-shoots and leaves a growing empty
        // band between transcript and pane.
        let top = end.min(screen.height.saturating_sub(height));

        if end > top {
            self.viewport.y = end;
            self.scroll_into_scrollback(end, end - top)?;
        }

        self.viewport = Rect::new(0, top, width, height);
        self.buffers = [Buffer::empty(self.viewport), Buffer::empty(self.viewport)];
        self.clear_rows(top..screen.height)?;
        self.park(top)
    }

    /// Clear the OLD pane's cells after a resize. The caret is inside
    /// the pane and the terminal has carried it to wherever its rewrap
    /// landed; stepping up by `above` rows lands on the frame's top.
    /// Only the pane's own rows get wiped — clearing to end of display
    /// would also erase transcript rows the terminal restored into the
    /// newly-visible area when the window grew.
    fn erase_last_frame(&mut self, above: u16) -> Result<()> {
        write!(self.backend, "\r")?;
        if above > 0 {
            write!(self.backend, "\x1b[{above}A")?;
        }
        let h = self.viewport.height;
        for i in 0..h {
            write!(self.backend, "\r\x1b[2K")?;
            if i + 1 < h {
                write!(self.backend, "\x1b[B")?;
            }
        }
        // Return to the top of the erased region so the caller's
        // repaint starts from a known row.
        if h > 1 {
            write!(self.backend, "\x1b[{}A", h - 1)?;
        }
        write!(self.backend, "\r")?;
        Ok(())
    }

    /// Height in screen rows of the pane portion above the caret when
    /// reflowed at `width`. Each of its rows rewraps like a transcript row
    /// would, so we measure by printed columns rather than counting rows.
    fn frame_rows_above_caret(&self, width: u16) -> u16 {
        let frame = &self.buffers[1 - self.current];
        frame
            .content
            .chunks(frame.area.width.max(1) as usize)
            .take(self.cursor_row.saturating_sub(self.viewport.y) as usize)
            .map(|row| (used(row) as u16).div_ceil(width).max(1))
            .fold(0u16, u16::saturating_add)
    }

    /// Move the caret to a row we know the value of. `erase_last_frame`
    /// steps upward from wherever the caret happens to be on the next
    /// resize, so any code path that moves the caret has to update
    /// `cursor_row` too — otherwise the erase starts from the wrong row.
    fn park(&mut self, row: u16) -> Result<()> {
        self.backend.set_cursor_position(Position::new(0, row))?;
        self.cursor_row = row;
        Ok(())
    }

    /// Record the printed column width of every row in `cells`. A later
    /// rewrap can then compute exactly how many screen rows each one will
    /// occupy at a new terminal width.
    fn track(&mut self, cells: &Buffer) {
        let width = cells.area.width as usize;
        for row in cells.content.chunks(width.max(1)) {
            self.transcript.push(used(row) as u16);
        }
    }

    fn draw_cleared<'a>(
        &mut self,
        y: u16,
        rows: u16,
        stride: u16,
        cells: &'a [Cell],
    ) -> Result<&'a [Cell]> {
        let width = stride as usize;
        let take = (width * rows as usize).min(cells.len());
        let (to_draw, rest) = cells.split_at(take);
        // Only draw up to the last cell that carries anything. Padding a
        // row out to full width would count as content for the terminal
        // and reflow into its own screen rows once the window narrowed,
        // filling scrollback with blanks.
        let iter = to_draw
            .iter()
            .enumerate()
            .filter(move |(i, _)| i % width < used(&to_draw[i - i % width..][..width]))
            .filter(keeps_cell(width))
            .map(|(i, c)| ((i % width) as u16, y + (i / width) as u16, c));
        self.backend.draw(iter)?;
        Ok(rest)
    }

    /// Clear via `EL` (erase line) rather than by overwriting with spaces
    /// — spaces are content, and a terminal that reflows will wrap them
    /// into extra rows once the window narrows.
    fn clear_rows(&mut self, rows: std::ops::Range<u16>) -> Result<()> {
        for y in rows {
            write!(self.backend, "\x1b[{};1H\x1b[2K", y + 1)?;
        }
        Ok(())
    }
}

/// Number of columns in `row` that are worth emitting — everything up to
/// the last cell that would leave a visible mark. A cell counts as blank
/// only if it also carries no style (a shaded space still paints).
fn used(row: &[Cell]) -> usize {
    row.iter()
        .rposition(|c| c != &Cell::EMPTY)
        .map_or(0, |i| i + 1)
}

fn keeps_cell(width: usize) -> impl FnMut(&(usize, &Cell)) -> bool {
    let mut skip = 0usize;
    move |(i, cell)| {
        if *i % width == 0 {
            skip = 0;
        }
        if skip > 0 {
            skip -= 1;
            false
        } else {
            skip = cell.symbol().width().saturating_sub(1);
            true
        }
    }
}

/// How many rows a pane of `height` has to borrow from the transcript
/// above it after first using up whatever blank screen sits below.
fn borrowed(old: Rect, height: u16, screen: Size) -> u16 {
    let room_below = screen.height.saturating_sub(old.bottom());
    height
        .saturating_sub(old.height)
        .saturating_sub(room_below)
        .min(old.y)
}

const MAX_VIEWPORT_NUM: u16 = 3;
const MAX_VIEWPORT_DEN: u16 = 5;

fn clamp_height(height: u16, screen: Size) -> u16 {
    let share = (screen.height * MAX_VIEWPORT_NUM / MAX_VIEWPORT_DEN).max(1);
    let ceiling = share.min(screen.height.saturating_sub(1).max(1));
    height.clamp(1, ceiling)
}
