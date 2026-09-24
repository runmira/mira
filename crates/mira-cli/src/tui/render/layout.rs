//! Transcript layout — the derived "where things go" layer.
//!
//! Scroll and viewport math used to live as write-backs from the
//! render function (`transcript_tail`, `viewport_height` were rendered
//! state that key handlers then read). That made resize, wrapping, and
//! scroll handling impossible to reason about: the metric a keyboard
//! event consumed was whatever the *previous* frame happened to write.
//!
//! Now the layout is explicitly derived state:
//!
//! ```text
//! TuiState ──▶ build_lines ──▶ TranscriptLayout ──▶ resolve_scroll ──▶ render
//! ```
//!
//! `TuiState` caches the last [`TranscriptLayout`]; the event loop
//! invalidates + reconciles it on `Event::Resize` before the next
//! draw, and handlers read `cached_tail()` off the cache instead of a
//! render write-back. Cache staleness is bounded to one event and
//! self-corrects at the next frame.

use crate::tui::state::TuiState;

/// Sentinel row for entries that live in the terminal's real
/// scrollback (mirrored via `insert_before`), not in the viewport.
/// Layout keeps one slot per entry so indices stay global — anything
/// reading the table must skip this value instead of scrolling to it.
pub(crate) const SCROLLBACK_ROW: usize = usize::MAX;

/// Derived geometry of the transcript for one (width, content) pair.
#[derive(Clone, Debug, Default)]
pub struct TranscriptLayout {
    /// Transcript-region height (visible rows). Distinct from the
    /// terminal's frame height — the header, goal panel, composer, and
    /// footer all subtract from it.
    pub height: u16,
    /// Total wrapped rows the transcript occupies.
    pub total_rows: u16,
    /// `entry_row_starts[i]` = first rendered row of log entry `i`.
    /// Batched tool groups map every covered entry to the same start
    /// so search hits and turn-nav jumps still land correctly.
    /// Entries already mirrored into real scrollback map to
    /// [`SCROLLBACK_ROW`] — callers must not scroll to it.
    pub entry_row_starts: Vec<usize>,
}

impl TranscriptLayout {
    /// Row offset where the tail sits — the maximum valid scroll
    /// value for the region this layout was measured against.
    pub fn tail(&self) -> u16 {
        self.total_rows.saturating_sub(self.height)
    }
}

/// Pick the effective scroll offset for this frame.
///
/// Precedence: turn-nav jump > active search hit > follow-tail >
/// wherever the user last scrolled to.
#[allow(dead_code)]
pub(crate) fn resolve_scroll(
    state: &TuiState,
    layout: &TranscriptLayout,
    viewport_height: u16,
) -> u16 {
    let tail = layout.tail();
    if let Some(idx) = state.turn_scroll_target {
        let start = layout.entry_row_starts.get(idx).copied().unwrap_or(0);
        if start == SCROLLBACK_ROW {
            // Target already scrolled into real scrollback — hold
            // position; the key handler flashes where to look.
            return state.scroll.min(tail);
        }
        // Anchor the user prompt line near the top so what comes after
        // (assistant reply, tool group) fills the viewport.
        (start as u16).saturating_sub(1).min(tail)
    } else if let Some((entry_idx, _)) = state.active_hit() {
        let start = layout
            .entry_row_starts
            .get(entry_idx)
            .copied()
            .unwrap_or(0);
        if start == SCROLLBACK_ROW {
            return state.scroll.min(tail);
        }
        (start as u16).saturating_sub(viewport_height / 3).min(tail)
    } else if state.follow_tail {
        tail
    } else {
        state.scroll.min(tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(total_rows: u16, height: u16, starts: Vec<usize>) -> TranscriptLayout {
        TranscriptLayout {
            height,
            total_rows,
            entry_row_starts: starts,
        }
    }

    #[test]
    fn tail_never_negative() {
        assert_eq!(layout(10, 3, vec![]).tail(), 7);
        assert_eq!(layout(10, 10, vec![]).tail(), 0);
        assert_eq!(layout(10, 40, vec![]).tail(), 0);
    }

    #[test]
    fn turn_target_anchors_near_top() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let l = layout(100, 10, vec![0, 5, 20, 40]);
        st.turn_scroll_target = Some(2);
        assert_eq!(resolve_scroll(&st, &l, 10), 19);
    }

    #[test]
    fn follow_tail_uses_tail() {
        let st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        let l = layout(100, 10, vec![]);
        assert_eq!(resolve_scroll(&st, &l, 10), 90);
    }

    #[test]
    fn user_scroll_clamps_to_tail() {
        let mut st = TuiState::new("m".into(), mira_policy::Mode::Manual);
        st.follow_tail = false;
        st.scroll = 500;
        let l = layout(100, 10, vec![]);
        assert_eq!(resolve_scroll(&st, &l, 10), 90);
    }
}
