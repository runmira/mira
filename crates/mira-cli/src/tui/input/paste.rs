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
    let by_lines = line_count >= crate::tui::state::PASTE_COLLAPSE_LINES;
    let by_chars = char_count >= crate::tui::state::PASTE_COLLAPSE_CHARS;
    if !by_lines && !by_chars {
        state.input_push_str(s);
        return;
    }
    let token = state.stash_paste(s.to_owned());
    state.input_push_str(&token);
    // Name the actual trigger — a wide one-line URL that tripped the
    // char threshold shouldn't be labelled "N lines" (it's always 1).
    let label = if by_lines {
        format!("{line_count} lines")
    } else {
        format!("{char_count} chars")
    };
    state.flash = Some(format!(
        "collapsed paste ({label}) — enter sends full text, ctrl+x to remove"
    ));
}
