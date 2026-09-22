//! Bracketed-paste routing.
//!
//! Split from the keyboard handler so the composer's paste path and any
//! future `/paste` command hit the same collapsing logic.

use crate::tui::state::TuiState;

/// Route bracketed-paste content: small pastes go straight into the
/// composer as normal text; large ones get stashed as a
/// `[[paste:N]]` placeholder rendered as `[pasted N lines]`.
pub(crate) fn handle_paste(s: &str, state: &mut TuiState) {
    let line_count = s.matches('\n').count() + 1;
    let char_count = s.chars().count();
    let big = line_count >= crate::tui::state::PASTE_COLLAPSE_LINES
        || char_count >= crate::tui::state::PASTE_COLLAPSE_CHARS;
    if !big {
        state.input_push_str(s);
        return;
    }
    let token = state.stash_paste(s.to_owned());
    state.input_push_str(&token);
    state.flash = Some(format!(
        "collapsed paste ({line_count} lines) — enter sends full text, ctrl+x to remove"
    ));
}
