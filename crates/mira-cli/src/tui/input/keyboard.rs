//! Keyboard, mouse, and palette input handling.
//!
//! One file owns every user-initiated state transition: composer
//! editing, scrolling, turn navigation, approval answers, search keys,
//! palette matching/accepting, mode cycling, and clipboard. Slash
//! command *dispatch* lives in `super` (tui::mod); everything that
//! happens on the way there lives here.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use futures::stream::BoxStream;
use mira_harness::{HarnessEvent, Session};
use std::io::Write;

use crate::tui::event_loop::{interrupt_stream, submit_composed};
use crate::tui::state::{Palette, PaletteItem, PaletteState, TuiState};

use crate::tui::{is_reserved_slash, TuiConfig, SLASH_COMMANDS};

// ---- scrolling ----

/// Scroll the transcript by `n` rows toward the top.
pub(crate) fn scroll_up(state: &mut TuiState, n: u16) {
    state.follow_tail = false;
    state.scroll = state.scroll.saturating_sub(n);
}

/// Scroll the transcript by `n` rows toward the bottom. Re-engages
/// follow-tail once the user catches up with the newest content, so
/// streamed tokens keep landing in view. The tail comes from the
/// derived layout cache (`cached_tail`) — possibly one event stale,
/// which the next frame corrects.
pub(crate) fn scroll_down(state: &mut TuiState, n: u16) {
    let new = state.scroll.saturating_add(n);
    match state.cached_tail() {
        Some(tail) if new >= tail => {
            state.scroll = tail;
            state.follow_tail = true;
        }
        _ => {
            state.scroll = new;
            state.follow_tail = false;
        }
    }
}

/// A page's worth of rows for PgUp/PgDn — most of the transcript
/// viewport, minus a small overlap so the reader can anchor on a
/// familiar line between jumps. Uses the derived layout's region
/// height when available (frame height minus header/composer/footer);
/// clamped to a sane minimum for tiny terminals.
pub(crate) fn page_step(state: &TuiState) -> u16 {
    let vh = state
        .layout_cache
        .as_ref()
        .map(|l| l.height)
        .unwrap_or_else(|| state.viewport_height.max(4));
    vh.saturating_sub(2).max(1)
}

// ---- turn navigation ----

/// Ctrl+↑ — step to the previous user prompt. Sets `follow_tail = false`
/// so the transcript stays put, then defers to the layout's
/// `entry_row_starts` (via `turn_scroll_target`) to scroll it in.
fn jump_to_prev_user_turn(state: &mut TuiState) {
    let anchor = state.turn_nav_anchor();
    let next = match anchor {
        Some(idx) => state.user_entry_before(idx),
        None => state.prev_user_entry_idx(state.scroll),
    };
    match next {
        Some(idx) => {
            state.set_turn_nav_anchor(Some(idx));
            state.turn_scroll_target = Some(idx);
            state.follow_tail = false;
            state.flash = Some(format!(
                "turn {}/{}",
                user_ordinal(state, idx),
                user_count(state)
            ));
        }
        None => {
            state.flash = Some("no earlier user turn".into());
        }
    }
}

/// Ctrl+↓ — step to the next user prompt (or the tail if we've reached
/// the last one).
fn jump_to_next_user_turn(state: &mut TuiState) {
    let anchor = state.turn_nav_anchor();
    let next = anchor.and_then(|idx| state.user_entry_after(idx));
    match next {
        Some(idx) => {
            state.set_turn_nav_anchor(Some(idx));
            state.turn_scroll_target = Some(idx);
            state.follow_tail = false;
            state.flash = Some(format!(
                "turn {}/{}",
                user_ordinal(state, idx),
                user_count(state)
            ));
        }
        None => {
            // Past the newest turn — snap back to live tail.
            state.set_turn_nav_anchor(None);
            state.follow_tail = true;
            state.turn_scroll_target = None;
            state.flash = Some("caught up".into());
        }
    }
}

fn user_ordinal(state: &TuiState, idx: usize) -> usize {
    state
        .entries()
        .iter()
        .take(idx + 1)
        .filter(|e| matches!(e, crate::tui::state::LogEntry::User(_)))
        .count()
}

fn user_count(state: &TuiState) -> usize {
    state
        .entries()
        .iter()
        .filter(|e| matches!(e, crate::tui::state::LogEntry::User(_)))
        .count()
}

/// Route a terminal (non-key) event: pastes, mouse wheel. Resize is
/// handled by the event loop itself — it's a state transition, not an
/// input.
pub(crate) fn handle_mouse(m: MouseEvent, state: &mut TuiState) {
    match m.kind {
        MouseEventKind::ScrollUp => scroll_up(state, 3),
        MouseEventKind::ScrollDown => scroll_down(state, 3),
        _ => {}
    }
}

// ---- key dispatch ----

pub(crate) async fn handle_key(
    key: KeyEvent,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    // Approval steals the keys EXCEPT scrolling — deciding on a diff
    // usually means reading the code above it, so PgUp/PgDn, Alt+arrows
    // and j/k keep working while the prompt is up.
    if state.pending_approval.is_some() {
        match (key.code, key.modifiers) {
            (KeyCode::Up, KeyModifiers::ALT) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                scroll_up(state, 3);
                return;
            }
            (KeyCode::Down, KeyModifiers::ALT) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                scroll_down(state, 3);
                return;
            }
            (KeyCode::PageUp, _) => {
                scroll_up(state, page_step(state));
                return;
            }
            (KeyCode::PageDown, _) => {
                scroll_down(state, page_step(state));
                return;
            }
            _ => {}
        }
        handle_approval_key(key, state, cfg).await;
        return;
    }

    // Search overlay steals the keys (typing goes to the query, not the
    // composer). Esc / Enter close it.
    if state.search.is_some() {
        handle_search_key(key, state);
        return;
    }

    // Palette steals a few keys (arrows, Tab, Esc, Enter) — everything
    // else falls through to composer editing, then we refresh the
    // palette's matches based on the new input.
    if state.palette.kind != Palette::None && handle_palette_key(key, state) {
        return;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            interrupt_stream(state, agent_stream);
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            if state.is_input_empty() {
                state.should_quit = true;
            } else {
                state.input_delete_forward();
            }
        }
        // Ctrl+J → newline in the composer. Terminals send LF (0x0A) for
        // Ctrl+J and CR (0x0D) for Enter, which crossterm maps to
        // KeyCode::Enter with the CONTROL modifier for the LF case.
        (KeyCode::Char('j'), KeyModifiers::CONTROL)
        | (KeyCode::Enter, KeyModifiers::CONTROL)
        | (KeyCode::Enter, KeyModifiers::SHIFT) => {
            state.input_newline();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Esc, _) => {
            // Three-way route:
            //   1. Turn in flight → single Esc interrupts (matches the
            //      "esc to interrupt" hint in the streaming line).
            //   2. Text in composer → Esc x2 clears the buffer.
            //   3. Empty composer, idle → Esc x2 quits.
            // Second-press behavior is set by `esc_pending`; the flash
            // string tells the user what the *next* Esc will do so they
            // don't guess.
            if agent_stream.is_some() {
                interrupt_stream(state, agent_stream);
                state.esc_pending = true;
                return;
            }
            if state.esc_pending {
                if !state.is_input_empty() {
                    let _ = state.input_clear();
                    state.flash = Some("input cleared".into());
                    // Clearing the buffer is a terminal action — the
                    // double-press is consumed. Without this the next
                    // stray Esc would quit, which is the exact
                    // "three Esc to clear then quit" footgun we're
                    // here to remove.
                    state.esc_pending = false;
                } else {
                    state.should_quit = true;
                }
            } else {
                state.esc_pending = true;
            }
            return;
        }
        (KeyCode::Enter, m) if !m.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) => {
            if state.is_input_empty() {
                return;
            }
            if state.streaming {
                // Mid-turn: park the message instead of dropping it.
                // It auto-sends when the turn completes (and ↑ pulls it
                // back for editing before then) — the user keeps
                // composing instead of waiting on the agent.
                let raw = state.input_clear();
                let text = state.expand_pastes(&raw);
                state.pastes.clear();
                state.palette = PaletteState::none();
                let depth = state.queue_message(text);
                state.flash = Some(format!(
                    "queued — sends when this turn finishes ({depth} queued)"
                ));
                state.esc_pending = false;
                return;
            }
            let raw = state.input_clear();
            submit_composed(state, session, agent_stream, cfg, raw).await;
        }
        // ---- composer editing ----
        (KeyCode::Backspace, KeyModifiers::ALT) => {
            state.kill_word_left();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Char('d'), KeyModifiers::ALT) => {
            state.kill_word_right();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Backspace, _) => {
            state.input_backspace();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Delete, _) => {
            state.input_delete_forward();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Left, KeyModifiers::ALT) => state.move_word_left(),
        (KeyCode::Left, _) => state.move_left(),
        (KeyCode::Right, KeyModifiers::ALT) => state.move_word_right(),
        (KeyCode::Right, _) => state.move_right(),
        (KeyCode::Home, _) => state.move_line_start(),
        (KeyCode::End, _) => state.move_line_end(),
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => state.move_line_start(),
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            // Ctrl+E toggles the last tool result unless the composer
            // has content — then it acts as "line end" per emacs
            // convention. This makes the shortcut discoverable without
            // stealing an editing keystroke.
            if state.is_input_empty() {
                if !state.toggle_last_tool_result() {
                    state.flash = Some("no tool result to expand".into());
                }
            } else {
                state.move_line_end();
            }
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            state.kill_word_left();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            state.kill_to_line_start();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            state.kill_to_line_end();
            refresh_palette(state, cfg, file_index).await;
        }
        (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
            // Ctrl+X removes the collapsed paste placeholder under
            // (or immediately next to) the caret. Silent no-op when
            // the cursor isn't inside one — we don't want to hijack
            // the shortcut in the general case.
            if state.remove_paste_at_cursor() {
                state.flash = Some("paste removed".into());
                refresh_palette(state, cfg, file_index).await;
                state.esc_pending = false;
                return;
            }
        }
        (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
            // One-keystroke reversal of the last file write — the
            // `/undo to revert` chip under the newest edit, without
            // typing the command.
            crate::tui::run_undo_slash("", state, session).await;
            state.esc_pending = false;
            return;
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            // Edit-and-retry: rewind session history AND the visible
            // transcript to the start of the last user turn, then put
            // the prompt back in the composer. Enter re-sends it.
            if state.is_input_empty() && !state.streaming {
                if let Some(text) = state.rewind_last_turn() {
                    session.rewind_last_turn().await;
                    state.input_replace(&text);
                    state.follow_tail = true;
                    state.flash =
                        Some("rewound — edit the prompt, enter re-sends it".into());
                    state.esc_pending = false;
                    return;
                }
                state.flash = Some("nothing to rewind — no user turn yet".into());
                state.esc_pending = false;
                return;
            }
        }
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => match state.last_assistant_text() {
            Some(text) => {
                copy_to_clipboard(text);
                state.flash = Some(format!("copied {} bytes", text.len()));
            }
            None => state.flash = Some("nothing to copy — no assistant reply yet".into()),
        },
        // Ctrl+Shift+C dumps the full visible transcript as plain text
        // via OSC 52. Complements Ctrl+Y (last assistant only) so users
        // don't have to fight mouse-capture to grab an older reply.
        (KeyCode::Char('C'), m)
            if m.contains(KeyModifiers::CONTROL) && m.contains(KeyModifiers::SHIFT) =>
        {
            let text = state.transcript_plaintext();
            if text.is_empty() {
                state.flash = Some("nothing to copy — transcript is empty".into());
            } else {
                copy_to_clipboard(&text);
                state.flash = Some(format!("copied transcript ({} bytes)", text.len()));
            }
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            state.search_open();
        }
        (KeyCode::Char('m'), KeyModifiers::ALT) => {
            // Toggle mouse capture. When on, terminals stop letting
            // the user select text natively; when off, our ratatui
            // scroll shortcuts still work (they're key-based).
            state.mouse_capture = !state.mouse_capture;
            let mut out = std::io::stdout();
            if state.mouse_capture {
                let _ = crossterm::execute!(out, crossterm::event::EnableMouseCapture);
                state.flash = Some("mouse capture: on".into());
            } else {
                let _ = crossterm::execute!(out, crossterm::event::DisableMouseCapture);
                state.flash = Some("mouse capture: off (select with mouse)".into());
            }
        }
        // Shift+Tab cycles permission mode (Plan → Manual → Auto → Edit
        // → Yolo → Plan). Mirrors Claude Code's "shift+tab to cycle" hint
        // — a keyboard-only path so users don't need /mode.
        (KeyCode::BackTab, _) => {
            let next = state.mode.next();
            state.mode = next;
            cfg.policy.lock().await.set_mode(next);
            state.flash = Some(format!("mode → {}", next.as_str()));
            return;
        }
        // ---- transcript scroll ----
        // Alt+Up/Down scrolls the transcript line-by-line — Mac laptops
        // rarely have PageUp/Down keys, so this gives them a native path.
        // Must come before the bare `(Up, _)` history arm below.
        (KeyCode::Up, KeyModifiers::ALT) => scroll_up(state, 1),
        (KeyCode::Down, KeyModifiers::ALT) => scroll_down(state, 1),
        // Ctrl+Up / Ctrl+Down jumps between user turns. Anchored so a
        // sequence of presses walks steadily back through history
        // rather than snapping to the newest turn every time. Returns
        // early so the `turn 3/5` flash survives past the trailing
        // `state.flash = None` clear that closes the handler.
        (KeyCode::Up, KeyModifiers::CONTROL) => {
            jump_to_prev_user_turn(state);
            state.esc_pending = false;
            return;
        }
        (KeyCode::Down, KeyModifiers::CONTROL) => {
            jump_to_next_user_turn(state);
            state.esc_pending = false;
            return;
        }
        (KeyCode::PageUp, _) => scroll_up(state, page_step(state)),
        (KeyCode::PageDown, _) => scroll_down(state, page_step(state)),
        // ---- history recall ----
        // While streaming, ↑ edits the queue instead: the newest parked
        // message comes back into the composer; Enter re-queues it.
        (KeyCode::Up, _) => {
            if state.streaming {
                if state.pop_queued_to_input() {
                    state.flash = Some("editing queued message — enter re-queues it".into());
                    state.esc_pending = false;
                    return;
                }
            } else {
                state.history_prev();
            }
        }
        (KeyCode::Down, _) => {
            if !state.streaming {
                state.history_next();
            }
        }
        // ---- printable ----
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            // With an empty composer, 1–9 toggle the nine most recent
            // tool groups (1 = newest) between collapsed and expanded.
            if state.is_input_empty()
                && state.palette.kind == Palette::None
                && matches!(c, '1'..='9')
            {
                let n = c.to_digit(10).unwrap_or(0) as usize;
                if state.toggle_nth_tool_group_from_end(n) {
                    state.esc_pending = false;
                    return;
                }
                // Fewer than n groups — fall through and type the digit.
            }
            state.input_push(c);
            refresh_palette(state, cfg, file_index).await;
        }
        _ => {}
    }
    state.esc_pending = false;
    state.flash = None;
}

async fn handle_approval_key(key: KeyEvent, state: &mut TuiState, cfg: &TuiConfig) {
    // 'a' = always allow this exact call (session-scoped rule) then allow this call.
    // 'y' = allow once. 'n' / Esc = deny.
    if matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A')) {
        if let Some(pending) = state.pending_approval.take() {
            // Only log when we actually mutated policy —
            // "session-allow: <rule>" is new state worth surfacing.
            // A quiet "allowing once only" flash sufficed for the y
            // path; a full info line every approval was noise.
            match TuiState::rule_for_call(&pending.request.call) {
                Some(rule) => match cfg.policy.lock().await.add_allow_rule(&rule) {
                    Ok(_) => state.push_info(format!("[policy] session-allow: {rule}")),
                    Err(e) => state.push_warning(format!("[policy] couldn't add rule: {e}")),
                },
                None => {
                    state.flash = Some("allowed once (no reusable rule)".into());
                }
            }
            let _ = pending.request.reply.send(true);
        }
        return;
    }
    let allow = match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
        _ => None,
    };
    let Some(allow) = allow else { return };
    if let Some(pending) = state.pending_approval.take() {
        let _ = pending.request.reply.send(allow);
    }
}

fn handle_search_key(key: KeyEvent, state: &mut TuiState) {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => state.search_close(),
        (KeyCode::Enter, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => state.search_next(),
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => state.search_prev(),
        (KeyCode::Backspace, _) => state.search_backspace(),
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => state.search_push(c),
        _ => {}
    }
}

/// Handle keys the palette wants to intercept. Returns `true` when the
/// key was consumed (caller skips its own dispatch).
fn handle_palette_key(key: KeyEvent, state: &mut TuiState) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            state.palette = PaletteState::none();
            true
        }
        (KeyCode::Up, _) => {
            if !state.palette.matches.is_empty() {
                state.palette.cursor = state.palette.cursor.saturating_sub(1);
            }
            true
        }
        (KeyCode::Down, _) => {
            let last = state.palette.matches.len().saturating_sub(1);
            state.palette.cursor = (state.palette.cursor + 1).min(last);
            true
        }
        (KeyCode::Tab, _) | (KeyCode::Enter, KeyModifiers::NONE) => {
            accept_palette(state);
            true
        }
        _ => false,
    }
}

/// Replace the current trigger (`/…` or `@…`) with the selected
/// palette item's `insert` string, close the palette, and place the
/// cursor at the end of the completion.
fn accept_palette(state: &mut TuiState) {
    let Some(item) = state.palette.matches.get(state.palette.cursor).cloned() else {
        state.palette = PaletteState::none();
        return;
    };
    match state.palette.kind {
        Palette::Slash => {
            let rest_after_slash = &state.input()[1..];
            let word_end = rest_after_slash
                .find(char::is_whitespace)
                .map(|i| i + 1)
                .unwrap_or_else(|| state.input().len());
            // Rewrite the leading "/word" run with the picked slash
            // command, preserving anything after the first whitespace.
            let tail = state.input()[word_end..].to_owned();
            let new_input = format!("{} {}", item.insert, tail.trim_start());
            let cursor = item.insert.len() + 1; // right after the space
            let cleaned = new_input.trim_end().to_owned();
            let cursor = cursor.min(cleaned.len());
            replace_input(state, cleaned, cursor);
        }
        Palette::AtFile => {
            let (start, end) = at_word_bounds(state.input(), state.cursor());
            let mut new_input = String::new();
            new_input.push_str(&state.input()[..start]);
            new_input.push_str(&item.insert);
            new_input.push_str(&state.input()[end..]);
            let cursor = start + item.insert.len();
            replace_input(state, new_input, cursor);
        }
        Palette::Model => {
            // Rewrite the whole line to `/model <picked-id>`. No tail
            // to preserve — `/model` doesn't take further args.
            let new_input = format!("/model {}", item.insert);
            let cursor = new_input.len();
            replace_input(state, new_input, cursor);
        }
        Palette::Theme => {
            // Same shape as `/model` — rewrite the whole line so
            // partial-arg typos get cleaned up when the user picks.
            let new_input = format!("/theme {}", item.insert);
            let cursor = new_input.len();
            replace_input(state, new_input, cursor);
        }
        Palette::SavePath => {
            let new_input = format!("/save {}", item.insert);
            let cursor = new_input.len();
            replace_input(state, new_input, cursor);
        }
        Palette::None => {}
    }
    state.palette = PaletteState::none();
}

/// Whole-input replace via the state helpers so history-browse and
/// cursor invariants stay consistent.
fn replace_input(state: &mut TuiState, new_input: String, new_cursor: usize) {
    // Cheapest way to reset both fields without exposing them mutably.
    state.input_clear();
    state.input_push_str(&new_input);
    // input_push_str moves the cursor to end; walk it back to the
    // requested position by moving left char-by-char.
    while state.cursor() > new_cursor {
        state.move_left();
    }
}

// ---- palette matching ----

/// Recompute the palette matches based on the composer's current
/// contents. Called after every input mutation. Opens or closes the
/// palette as needed.
pub(crate) async fn refresh_palette(
    state: &mut TuiState,
    cfg: &TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    // Slash palette wins when the very first char is '/', regardless of
    // where the cursor sits — matches how users think about slash
    // commands ("it's a slash line").
    if state.input().starts_with('/') && !state.input().contains(' ') {
        let filter = state.input().trim_start_matches('/').to_ascii_lowercase();
        let matches = slash_matches(state, &filter, cfg).await;
        open_palette(state, Palette::Slash, matches);
        return;
    }

    // `/model <partial>` — completions from the live provider's
    // model catalog. Cached in TuiConfig so the fetch runs in the
    // background at boot and every subsequent open is a Vec lookup.
    if let Some(filter) = state.input().strip_prefix("/model ") {
        let filter = filter.trim_start().to_ascii_lowercase();
        let matches = model_matches(&filter, cfg).await;
        open_palette(state, Palette::Model, matches);
        return;
    }

    // `/theme <partial>` — bundled presets + the two housekeeping
    // verbs (`reload`, `save`). Same UX contract as `/model` so the
    // pattern reads the same across every arg-taking slash.
    if let Some(filter) = state.input().strip_prefix("/theme ") {
        let filter = filter.trim_start().to_ascii_lowercase();
        let matches = theme_matches(&filter);
        open_palette(state, Palette::Theme, matches);
        return;
    }

    // `/save <partial-path>` — reuse the rg-driven file index for
    // completion so the pattern matches `@file`. Also offers
    // sensible defaults (`transcript.md`, dated timestamp) when the
    // arg is empty. Matches Aider/Claude Code's tab-completion feel
    // on filenames.
    if let Some(filter) = state.input().strip_prefix("/save ") {
        let filter = filter.trim_start();
        let files = ensure_file_index(file_index, &cfg.cwd).await;
        let matches = save_path_matches(files, filter, &cfg.cwd);
        open_palette(state, Palette::SavePath, matches);
        return;
    }

    // @file picker when the cursor sits inside an `@word` run.
    if let Some((word_start, word_end)) = find_at_word(state.input(), state.cursor()) {
        let filter = state.input()[word_start + 1..word_end].to_ascii_lowercase();
        let files = ensure_file_index(file_index, &cfg.cwd).await;
        let matches = at_file_matches(files, &filter);
        open_palette(state, Palette::AtFile, matches);
        return;
    }

    // No trigger — close the palette.
    state.palette = PaletteState::none();
}

/// Filter the bundled theme presets + housekeeping verbs. Same
/// substring match as `slash_matches` uses so the palette feels
/// identical across `/model` and `/theme`.
fn theme_matches(filter: &str) -> Vec<PaletteItem> {
    // Presets first, then the two verbs, so a bare `/theme <TAB>` lists
    // the roster before the housekeeping actions.
    let mut items: Vec<PaletteItem> = crate::tui::theme::PRESETS
        .iter()
        .filter(|(name, _, _)| filter.is_empty() || name.to_ascii_lowercase().contains(filter))
        .map(|(name, _, desc)| PaletteItem {
            insert: (*name).to_owned(),
            title: (*name).to_owned(),
            detail: (*desc).to_owned(),
        })
        .collect();
    for (verb, desc) in [
        ("reload", "re-read ~/.mira/theme.yaml"),
        ("save", "write current palette to yaml (persists)"),
    ] {
        if filter.is_empty() || verb.contains(filter) {
            items.push(PaletteItem {
                insert: verb.to_owned(),
                title: verb.to_owned(),
                detail: desc.to_owned(),
            });
        }
    }
    items
}

/// Filter the cached model catalog with a substring match on the id
/// and stamp each result as ready-to-insert (`insert` = model id,
/// `title` = same, `detail` = provider hint pulled from the id prefix
/// when there is one — `openai/…`, `anthropic/…`, etc.).
async fn model_matches(filter: &str, cfg: &TuiConfig) -> Vec<PaletteItem> {
    let models = cfg.models.read().await;
    if models.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<PaletteItem> = models
        .iter()
        .filter(|id| filter.is_empty() || id.to_ascii_lowercase().contains(filter))
        .map(|id| {
            let detail = id
                .split_once('/')
                .map(|(p, _)| p.to_owned())
                .unwrap_or_default();
            PaletteItem {
                insert: id.clone(),
                title: id.clone(),
                detail,
            }
        })
        .collect();
    // Cap the palette — a provider catalog can be hundreds of models
    // (OpenRouter), and the overlay itself caps at 8 rows anyway.
    out.truncate(32);
    out
}

fn open_palette(state: &mut TuiState, kind: Palette, matches: Vec<PaletteItem>) {
    let cursor = if matches.is_empty() {
        0
    } else {
        state.palette.cursor.min(matches.len() - 1)
    };
    // Reset cursor when switching kinds so a fresh open doesn't inherit
    // a stale selection from a different list.
    let cursor = if state.palette.kind != kind {
        0
    } else {
        cursor
    };
    state.palette = PaletteState {
        kind,
        cursor,
        matches,
    };
}

/// Slash command palette source: built-in commands first, then any
/// skill that mounts a slash alias (frontmatter `slash:` or default
/// to the skill's name). Reserved built-ins shadow skill aliases so
/// a rogue skill can't hijack `/quit`.
async fn slash_matches(_state: &TuiState, filter: &str, cfg: &TuiConfig) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = SLASH_COMMANDS
        .iter()
        .filter(|(name, _)| {
            name.trim_start_matches('/')
                .to_ascii_lowercase()
                .contains(filter)
        })
        .map(|(name, desc)| PaletteItem {
            insert: (*name).to_owned(),
            title: (*name).to_owned(),
            detail: (*desc).to_owned(),
        })
        .collect();

    let reg = cfg.skills.read().await.clone();
    for s in reg.skills.values() {
        let Some(alias) = s.slash.as_deref() else {
            continue;
        };
        let slash = format!("/{alias}");
        if is_reserved_slash(&slash) {
            continue;
        }
        if !alias.to_ascii_lowercase().contains(filter) {
            continue;
        }
        items.push(PaletteItem {
            insert: slash.clone(),
            title: slash,
            detail: format!("skill · {}", s.description),
        });
    }
    items
}

async fn ensure_file_index<'a>(
    file_index: &'a mut Option<Vec<String>>,
    cwd: &std::path::Path,
) -> &'a [String] {
    if file_index.is_none() {
        *file_index = Some(list_files(cwd).await);
    }
    file_index.as_deref().unwrap_or(&[])
}

/// Shell out to `rg --files` in the cwd — respects .gitignore and is
/// present anywhere Mira runs (built-in rg dep). Falls back to an empty
/// list on any error, which just means the palette shows "no matches".
async fn list_files(cwd: &std::path::Path) -> Vec<String> {
    let cwd = cwd.to_owned();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("rg")
            .arg("--files")
            .current_dir(&cwd)
            .output();
        match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    })
    .await
    .unwrap_or_default()
}

/// Completions for `/save <path>` — first the two ready-to-pick
/// defaults (`transcript.md` under cwd, a dated one under `.mira/`),
/// then the file index filtered by substring so the user can
/// overwrite an existing file with tab-completion.
fn save_path_matches(files: &[String], filter: &str, cwd: &std::path::Path) -> Vec<PaletteItem> {
    let filter = filter.trim();
    let filter_lc = filter.to_ascii_lowercase();
    let mut out: Vec<PaletteItem> = Vec::new();

    // Suggest defaults regardless of filter — they lead the list when
    // the arg is empty, and fall behind exact substrings otherwise.
    if filter.is_empty() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(PaletteItem {
            insert: "transcript.md".to_owned(),
            title: "transcript.md".to_owned(),
            detail: format!("into {}", cwd.display()),
        });
        out.push(PaletteItem {
            insert: format!(".mira/transcript-{ts}.md"),
            title: format!(".mira/transcript-{ts}.md"),
            detail: "dated · under .mira/".to_owned(),
        });
    }

    let mut scored: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|f| {
            if filter.is_empty() {
                return Some((3, f));
            }
            let hay = f.to_ascii_lowercase();
            if hay.starts_with(&filter_lc) {
                Some((0, f))
            } else if hay.contains(&filter_lc) {
                Some((1, f))
            } else if fuzzy_subseq(&hay, &filter_lc) {
                Some((2, f))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.len().cmp(&b.1.len())));
    for (_, f) in scored.into_iter().take(20) {
        out.push(PaletteItem {
            insert: f.clone(),
            title: f.clone(),
            detail: "overwrite".to_owned(),
        });
    }
    out
}

fn at_file_matches(files: &[String], filter: &str) -> Vec<PaletteItem> {
    let filter = filter.trim();
    let mut scored: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|f| {
            if filter.is_empty() {
                return Some((0, f));
            }
            let hay = f.to_ascii_lowercase();
            if hay.starts_with(filter) {
                Some((0, f))
            } else if hay.contains(filter) {
                Some((1, f))
            } else if fuzzy_subseq(&hay, filter) {
                Some((2, f))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.len().cmp(&b.1.len())));
    scored
        .into_iter()
        .take(20)
        .map(|(_, f)| PaletteItem {
            insert: format!("@{f}"),
            title: f.clone(),
            detail: String::new(),
        })
        .collect()
}

/// `needle` is a subsequence of `hay` (char-wise, case-preserved by the
/// caller). Fast enough for a few thousand files typed against on every
/// keystroke.
fn fuzzy_subseq(hay: &str, needle: &str) -> bool {
    let mut ni = needle.chars();
    let mut nc = ni.next();
    for hc in hay.chars() {
        if Some(hc) == nc {
            nc = ni.next();
            if nc.is_none() {
                return true;
            }
        }
    }
    nc.is_none()
}

/// Locate the `@word` run under the cursor, if any. Returns
/// `(start_of_@, one_past_end_of_word)`. Requires `@` to be preceded by
/// whitespace or the start of a line — matches the informal convention
/// so an email address like foo@bar doesn't trip the picker.
fn find_at_word(s: &str, cursor: usize) -> Option<(usize, usize)> {
    let before = &s[..cursor];
    // Walk back from cursor to find '@' before any whitespace.
    let at_pos = before.rfind(|c: char| c == '@' || c.is_whitespace())?;
    if !s[at_pos..].starts_with('@') {
        return None;
    }
    // Must be at start-of-line or preceded by whitespace to count.
    if at_pos > 0 {
        let prev = s[..at_pos].chars().next_back();
        if !matches!(prev, Some(c) if c.is_whitespace()) {
            return None;
        }
    }
    // Find end of the word (whitespace or end).
    let after_at = at_pos + 1;
    let end = s[after_at..]
        .find(char::is_whitespace)
        .map(|off| after_at + off)
        .unwrap_or_else(|| s.len());
    if end < cursor {
        return None; // cursor is past the word — don't hijack
    }
    Some((at_pos, end))
}

/// Same bounds as `find_at_word` but assumes the cursor is inside one
/// (used by `accept_palette`).
fn at_word_bounds(s: &str, cursor: usize) -> (usize, usize) {
    find_at_word(s, cursor).unwrap_or((cursor, cursor))
}

/// Copy `text` to the terminal clipboard via OSC 52. Works on iTerm2,
/// modern Terminal.app, Kitty, WezTerm, and through SSH sessions that
/// forward the escape sequence. Silent no-op if stdout fails.
fn copy_to_clipboard(text: &str) {
    let payload = B64.encode(text.as_bytes());
    // The BEL (\x07) terminator has the widest compatibility.
    let seq = format!("\x1b]52;c;{payload}\x07");
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}
