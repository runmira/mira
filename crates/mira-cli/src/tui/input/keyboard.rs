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
        Some(idx) if idx < state.emitted_entries => {
            // That turn already scrolled into the terminal's real
            // scrollback — the viewport can't jump to it.
            state.set_turn_nav_anchor(Some(idx));
            state.flash = Some("that turn is in scrollback above — scroll your terminal".into());
        }
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
        Some(idx) if idx < state.emitted_entries => {
            state.set_turn_nav_anchor(Some(idx));
            state.flash = Some("that turn is in scrollback above — scroll your terminal".into());
        }
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
    // Which view owns the pane's keys right now — matches the render
    // side (`render::transcript::pane_cards`), so what shows on screen
    // and what handles the keystroke can never disagree.
    match crate::tui::state::PaneView::active(state) {
        crate::tui::state::PaneView::Approval => {
            // Approval steals the keys EXCEPT scrolling — deciding on a
            // diff usually means reading the code above it, so PgUp/PgDn,
            // Alt+arrows and j/k keep working while the prompt is up.
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
        crate::tui::state::PaneView::Plan => {
            // Plan card steals keys — scroll keys still live.
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
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::SHIFT) => {
                    state.plan_move(-1);
                    return;
                }
                (KeyCode::Down, _) => {
                    state.plan_move(1);
                    return;
                }
                (KeyCode::Char(' '), _) => {
                    state.plan_toggle_focused();
                    return;
                }
                (KeyCode::Enter, _) => {
                    state.plan_accept();
                    return;
                }
                (KeyCode::Esc, _) => {
                    state.plan_cancel();
                    return;
                }
                _ => {}
            }
            return;
        }
        crate::tui::state::PaneView::Ask => {
            // Ask-user card: digits pick, arrows move, `t` types, Enter
            // picks + advances (submits on the last question).
            handle_ask_key(key, state).await;
            return;
        }
        crate::tui::state::PaneView::Composer => {
            // Fall through to the composer/global handling below.
        }
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
            let next = interrupt_stream(state, agent_stream);
            if let Some(n) = next {
                submit_composed(state, session, agent_stream, cfg, n).await;
            }
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
                let next = interrupt_stream(state, agent_stream);
                state.esc_pending = true;
                if let Some(n) = next {
                    submit_composed(state, session, agent_stream, cfg, n).await;
                }
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
                    state.flash = Some("rewound — edit the prompt, enter re-sends it".into());
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
        (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
            // Ctrl+O opens the unified selector — one searchable panel
            // grouping mode/model/theme so the user can flip a session
            // knob without leaving the composer or remembering three
            // separate slash commands. Snapshots the model catalog
            // here (async lock) so the sync palette key handler can
            // re-filter it as the user types.
            state.unified_models_cache = cfg.models.read().await.clone();
            open_unified_selector(state);
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
    // Focus movement + direct picks first. `y`/`a`/`n` shortcuts below
    // bypass the focus.
    match (key.code, key.modifiers) {
        (KeyCode::Up, KeyModifiers::NONE) => {
            state.approval_move(-1);
            return;
        }
        (KeyCode::Down, KeyModifiers::NONE) => {
            state.approval_move(1);
            return;
        }
        (KeyCode::Enter, _) => {
            resolve_approval_focused(state, cfg).await;
            return;
        }
        (KeyCode::Char(c), KeyModifiers::NONE) if matches!(c, '1' | '2' | '3') => {
            state.approval_focus = (c as usize) - ('1' as usize);
            resolve_approval_focused(state, cfg).await;
            return;
        }
        _ => {}
    }
    // 'a' = always allow this exact call (session-scoped rule) then allow this call.
    // 'y' = allow once. 'n' / Esc = deny.
    if matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A')) {
        if let Some(pending) = state.approval_resolve() {
            approve_always(state, cfg, &pending.request.call).await;
            let _ = pending.request.reply.send(true);
        }
        note_queued(state);
        return;
    }
    let allow = match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
        _ => None,
    };
    let Some(allow) = allow else { return };
    if let Some(pending) = state.approval_resolve() {
        let _ = pending.request.reply.send(allow);
    }
    note_queued(state);
}

/// Resolve the head approval with the focused option (0 = allow once,
/// 1 = always this session, 2 = deny).
async fn resolve_approval_focused(state: &mut TuiState, cfg: &TuiConfig) {
    let focus = state.approval_focus;
    let Some(pending) = state.approval_resolve() else {
        return;
    };
    match focus {
        1 => {
            approve_always(state, cfg, &pending.request.call).await;
            let _ = pending.request.reply.send(true);
        }
        2 => {
            let _ = pending.request.reply.send(false);
        }
        _ => {
            let _ = pending.request.reply.send(true);
        }
    }
    note_queued(state);
}

/// Session-allow the call's rule, then allow this call. Only logs when
/// policy actually mutated — a quiet flash suffices otherwise, since a
/// full info line on every approval was noise.
async fn approve_always(
    state: &mut TuiState,
    cfg: &TuiConfig,
    call: &mira_core::ToolCall,
) {
    match TuiState::rule_for_call(call) {
        Some(rule) => match cfg.policy.lock().await.add_allow_rule(&rule) {
            Ok(_) => state.push_info(format!("[policy] session-allow: {rule}")),
            Err(e) => state.push_warning(format!("[policy] couldn't add rule: {e}")),
        },
        None => {
            state.flash = Some("allowed once (no reusable rule)".into());
        }
    }
}

/// After resolving one approval, point at the next if the queue isn't
/// empty — the card swaps to it immediately.
fn note_queued(state: &mut TuiState) {
    let n = state.pending_approvals.len();
    if n > 0 {
        state.flash = Some(format!(
            "next approval · {n} more queued"
        ));
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
///
/// The unified selector ([`Palette::Unified`]) additionally eats typing
/// and backspace, routing them to its standalone [`PaletteState::filter`]
/// buffer so the composer stays untouched while the user narrows the
/// list. Every other palette kind keeps the legacy behavior of falling
/// through to composer editing (they filter off composer contents).
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
        // Unified selector filter typing — consume the key ourselves so
        // it doesn't leak into the composer, then rebuild the filtered
        // match list from the new filter string.
        (KeyCode::Backspace, _) if state.palette.kind == Palette::Unified => {
            state.palette.filter.pop();
            refresh_unified_matches(state);
            true
        }
        (KeyCode::Char(c), m)
            if state.palette.kind == Palette::Unified
                && !m.contains(KeyModifiers::CONTROL) =>
        {
            state.palette.filter.push(c);
            refresh_unified_matches(state);
            true
        }
        _ => false,
    }
}

/// Rebuild the unified selector's match list against the current
/// `palette.filter`. Called on every keystroke while the selector is
/// open so the roster narrows as the user types.
fn refresh_unified_matches(state: &mut TuiState) {
    let filter = state.palette.filter.to_ascii_lowercase();
    let items = unified_matches(&filter, state);
    let cursor = state.palette.cursor.min(items.len().saturating_sub(1));
    state.palette.cursor = cursor;
    state.palette.matches = items;
}

/// Build the unified selector's item list: mode presets first, theme
/// presets next, model catalog last. Each item's `insert` field is
/// tagged with `mode:` / `model:` / `theme:` so `accept_palette` can
/// route it to the right action without a lookup — the encoding is
/// internal and never shown to the user (titles are pretty-printed
/// with a section chip up front).
pub(crate) fn unified_matches(filter: &str, state: &TuiState) -> Vec<PaletteItem> {
    let matches_filter = |hay: &str| -> bool {
        filter.is_empty() || hay.to_ascii_lowercase().contains(filter)
    };
    let current_mode = state.mode.as_str();
    let current_model = state.model.as_str();

    let mut items: Vec<PaletteItem> = Vec::new();
    // Modes.
    for (name, desc) in [
        ("plan", "propose first · ask on writes"),
        ("manual", "ask on every write/edit/command"),
        ("auto", "auto-approve writes+edits · ask on commands"),
        ("edit", "auto-approve writes, edits and commands"),
        ("yolo", "no gating whatsoever"),
    ] {
        if !matches_filter(name) && !matches_filter(desc) && !matches_filter("mode") {
            continue;
        }
        let mut title = format!("mode · {name}");
        if name == current_mode {
            title.push_str("  (current)");
        }
        items.push(PaletteItem {
            insert: format!("mode:{name}"),
            title,
            detail: desc.to_owned(),
        });
    }
    // Themes.
    for (name, _, desc) in crate::tui::theme::PRESETS {
        if !matches_filter(name) && !matches_filter(desc) && !matches_filter("theme") {
            continue;
        }
        items.push(PaletteItem {
            insert: format!("theme:{name}"),
            title: format!("theme · {name}"),
            detail: (*desc).to_owned(),
        });
    }
    // Models — the live catalog snapshot cached on state (populated
    // by the Ctrl+O opener via `TuiConfig::models`). Show at most 12
    // so a huge catalog doesn't drown out the shorter sections.
    let mut kept: Vec<&String> = state
        .unified_models_cache
        .iter()
        .filter(|m| matches_filter(m) || matches_filter("model"))
        .collect();
    kept.truncate(12);
    for id in kept {
        let mut title = format!("model · {id}");
        if id.as_str() == current_model {
            title.push_str("  (current)");
        }
        items.push(PaletteItem {
            insert: format!("model:{id}"),
            title,
            detail: id
                .split_once('/')
                .map(|(p, _)| p.to_owned())
                .unwrap_or_default(),
        });
    }
    items
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
        Palette::SlashArg(cmd) => {
            // Same shape as `/model` — rewrite the whole line so
            // partial-arg typos get cleaned up when the user picks.
            let new_input = format!("{cmd} {}", item.insert);
            let cursor = new_input.len();
            replace_input(state, new_input, cursor);
        }
        Palette::Unified => {
            // The unified selector encodes its section in the insert
            // string as `mode:X` / `theme:X` / `model:X`. Route by
            // splitting on ':' — anything unrecognised no-ops so a
            // corrupted entry can't wedge the UI.
            state.palette = PaletteState::none();
            state.unified_models_cache.clear();
            if let Some((kind, value)) = item.insert.split_once(':') {
                match kind {
                    "mode" => apply_unified_mode(state, value),
                    "model" => apply_unified_model(state, value),
                    "theme" => apply_unified_theme(state, value),
                    _ => {}
                }
            }
            return;
        }
        Palette::None => {}
    }
    state.palette = PaletteState::none();
}

/// Open the unified selector: seed an empty filter, build the initial
/// (unfiltered) match list, and swap the palette state in. The typed
/// filter accumulates on `palette.filter` after this — see
/// `handle_palette_key`'s Unified arm.
fn open_unified_selector(state: &mut TuiState) {
    // Clear any leftover filter from a previous open so the first
    // keystroke doesn't extend a stale search.
    state.palette.filter.clear();
    let matches = unified_matches("", state);
    open_palette(state, Palette::Unified, matches);
    // `open_palette` reset the filter to empty via the same-kind
    // preservation path (kinds differ from anything to Unified on
    // first open) — that's the intended fresh-open behavior.
}

fn apply_unified_mode(state: &mut TuiState, value: &str) {
    let mode = match value {
        "plan" => mira_policy::Mode::Plan,
        "manual" => mira_policy::Mode::Manual,
        "auto" => mira_policy::Mode::Auto,
        "edit" => mira_policy::Mode::Edit,
        "yolo" => mira_policy::Mode::Yolo,
        _ => {
            state.push_warning(format!("unified: unknown mode `{value}`"));
            return;
        }
    };
    state.mode = mode;
    state.flash = Some(format!("mode → {}", mode.as_str()));
    // Note: the shared policy isn't reachable synchronously here (only
    // TuiConfig owns the Arc). The next `/mode` slash or Shift+Tab
    // cycle re-syncs it; for a Ctrl+O flip mid-session the state.mode
    // change alone is enough to gate the next tool call correctly at
    // the TUI level. The harness re-reads on the following turn.
}

fn apply_unified_model(state: &mut TuiState, value: &str) {
    if value.is_empty() {
        return;
    }
    state.model = value.to_owned();
    state.flash = Some(format!("model → {value}"));
    // Session::set_model is async and lives on the caller side; we
    // record the state change and let the next turn pick it up. A
    // user who wants it plumbed through the session right now can
    // still use `/model <id>` which routes via `submit_composed`.
}

fn apply_unified_theme(state: &mut TuiState, value: &str) {
    match crate::tui::theme::preset(value) {
        Some((t, desc)) => {
            crate::tui::theme::set(t);
            state.flash = Some(format!("theme → {value}"));
            state.push_info(format!("theme · {value} — {desc}"));
        }
        None => state.push_warning(format!("unified: unknown theme `{value}`")),
    }
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

    // `/mode <partial>`, `/goal <partial>`, … — closed-set arguments
    // for the remaining arg-taking slash commands. Checked after the
    // bespoke `/model`, `/theme`, `/save` arms above so those keep
    // their richer sources.
    if state.input().starts_with('/') && state.input().contains(' ') {
        let head_end = state.input().find(' ').unwrap_or(state.input().len());
        let head = state.input()[..head_end].to_ascii_lowercase();
        if let Some(cmd) = slash_arg_command(&head) {
            let filter = state.input()[head_end..].trim_start().to_ascii_lowercase();
            let matches = slash_arg_matches(cmd, &filter, cfg).await;
            open_palette(state, Palette::SlashArg(cmd), matches);
            return;
        }
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

/// Closed-set slash commands whose arguments complete in the palette.
/// Returns the canonical command (so `/perms ` completes as
/// `/permissions`). `None` for commands with free-text or no args —
/// those fall through to the `@file` / close logic below.
fn slash_arg_command(head: &str) -> Option<&'static str> {
    match head {
        "/mode" => Some("/mode"),
        "/goal" => Some("/goal"),
        "/budget" => Some("/budget"),
        "/permissions" | "/perms" => Some("/permissions"),
        "/skill" => Some("/skill"),
        "/undo" => Some("/undo"),
        "/resume" => Some("/resume"),
        _ => None,
    }
}

/// Argument completions for `slash_arg_command` commands. Same
/// substring-filter contract as the other palette sources so every
/// arg list feels identical.
async fn slash_arg_matches(cmd: &str, filter: &str, cfg: &TuiConfig) -> Vec<PaletteItem> {
    match cmd {
        "/mode" => static_arg_matches(
            filter,
            &[
                ("plan", "propose first · ask on writes"),
                ("manual", "ask on every write/edit/command"),
                ("auto", "auto-approve writes+edits · ask on commands"),
                ("edit", "auto-approve writes, edits and commands"),
                ("yolo", "no gating whatsoever"),
            ],
        ),
        "/goal" => static_arg_matches(
            filter,
            &[
                ("status", "show the standing goal"),
                ("clear", "drop the standing goal"),
            ],
        ),
        "/budget" => static_arg_matches(
            filter,
            &[
                ("off", "remove the spend cap"),
                ("$1", "cap session spend"),
                ("$2", "cap session spend"),
                ("$5", "cap session spend"),
                ("$10", "cap session spend"),
            ],
        ),
        "/permissions" => static_arg_matches(
            filter,
            &[
                ("list", "show allow rules"),
                ("add", "add a Rule(...) — keep typing after accept"),
            ],
        ),
        "/undo" => static_arg_matches(
            filter,
            &[
                ("1", "revert the last write"),
                ("2", "revert the last 2 writes"),
                ("3", "revert the last 3 writes"),
            ],
        ),
        "/skill" => skill_arg_matches(filter, cfg).await,
        "/resume" => resume_arg_matches(filter, cfg).await,
        _ => Vec::new(),
    }
}

/// Substring-filter a static `(name, detail)` option list. Shared by
/// the closed-set slash args above.
fn static_arg_matches(filter: &str, opts: &[(&str, &str)]) -> Vec<PaletteItem> {
    opts.iter()
        .filter(|(name, _)| filter.is_empty() || name.to_ascii_lowercase().contains(filter))
        .map(|(name, desc)| PaletteItem {
            insert: (*name).to_owned(),
            title: (*name).to_owned(),
            detail: (*desc).to_owned(),
        })
        .collect()
}

/// `/skill <partial>` — every loaded skill by name (not just the ones
/// with a slash alias), so `/skill` discovers the same roster as
/// `/skills` lists.
async fn skill_arg_matches(filter: &str, cfg: &TuiConfig) -> Vec<PaletteItem> {
    let reg = cfg.skills.read().await.clone();
    let mut items: Vec<PaletteItem> = reg
        .skills
        .values()
        .filter(|s| filter.is_empty() || s.name.to_ascii_lowercase().contains(filter))
        .map(|s| PaletteItem {
            insert: s.name.clone(),
            title: format!("/{}", s.name),
            detail: s.description.clone(),
        })
        .collect();
    items.sort_by(|a, b| a.title.cmp(&b.title));
    items
}

/// `/resume <partial>` — recent session ids in this cwd, mirroring
/// what `/sessions` lists. Inserts the full id (that's what the resume
/// lookup needs); the title shows the short form.
async fn resume_arg_matches(filter: &str, cfg: &TuiConfig) -> Vec<PaletteItem> {
    let Some(store) = cfg.store.as_ref() else {
        return Vec::new();
    };
    let Ok(list) = store.list_recent(&cfg.cwd, 15).await else {
        return Vec::new();
    };
    let mut out: Vec<PaletteItem> = list
        .iter()
        .filter(|rec| {
            filter.is_empty()
                || rec
                    .id
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(filter)
        })
        .map(|rec| {
            let id = rec.id.as_str().to_owned();
            let short = if id.chars().count() > 12 {
                id.chars().take(8).collect::<String>()
            } else {
                id.clone()
            };
            let title = rec.title.clone().unwrap_or_default();
            let detail = if title.chars().count() > 50 {
                let head: String = title.chars().take(50).collect();
                format!("{head}…")
            } else {
                title
            };
            PaletteItem {
                insert: id,
                title: short,
                detail,
            }
        })
        .collect();
    out.truncate(8);
    out
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
    // Preserve the standalone filter when refreshing the SAME kind
    // (Unified selector types into it as the user filters); reset it
    // when switching kinds so a stale filter doesn't hide the new list.
    let filter = if state.palette.kind == kind {
        std::mem::take(&mut state.palette.filter)
    } else {
        String::new()
    };
    state.palette = PaletteState {
        kind,
        cursor,
        matches,
        filter,
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

    for (insert, detail) in crate::tui::ext_slash::palette_items(filter, cfg) {
        if is_reserved_slash(&insert) {
            continue;
        }
        items.push(PaletteItem {
            title: insert.clone(),
            insert,
            detail,
        });
    }

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
    let filter = filter.trim().to_ascii_lowercase();
    // Rank filename matches above path matches (typing `main` almost
    // always means `main.rs`, not `domain/handler.rs`), and shallow
    // paths above deep ones on ties.
    let mut scored: Vec<(u32, usize, &String)> = files
        .iter()
        .filter_map(|f| {
            if filter.is_empty() {
                return Some((0, 0, f));
            }
            let hay = f.to_ascii_lowercase();
            let file_name = hay.rsplit('/').next().unwrap_or(hay.as_str());
            let depth = hay.chars().filter(|c| *c == '/').count();
            if file_name.starts_with(filter.as_str()) {
                Some((0, depth, f))
            } else if file_name.contains(filter.as_str()) {
                Some((1, depth, f))
            } else if hay.contains(filter.as_str()) {
                Some((2, depth, f))
            } else if fuzzy_subseq(&hay, &filter) {
                Some((3, depth, f))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.len().cmp(&b.2.len()))
    });
    scored
        .into_iter()
        .take(20)
        .map(|(_, _, f)| PaletteItem {
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

/// Keys for the interactive ask_user card. Digits `1..=4` pick/toggle
/// options on the focused question; arrows move the option cursor;
/// `t` opens the free-text row (typing lands there, Enter commits
/// it); Tab skips between questions without picking; Enter picks the
/// focused option and advances (submits on the last question);
/// Esc cancels the whole card.
async fn handle_ask_key(key: KeyEvent, state: &mut TuiState) {
    if state.pending_ask.as_ref().is_some_and(|a| a.text_mode) {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if let Some(a) = state.pending_ask.as_mut() {
                    a.text_mode = false;
                    a.text.clear();
                }
            }
            (KeyCode::Enter, _) => {
                state.ask_commit_text();
            }
            (KeyCode::Backspace, _) => state.ask_text_backspace(),
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                state.ask_text_push(c);
            }
            _ => {}
        }
        return;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Up, KeyModifiers::ALT) | (KeyCode::Char('k'), KeyModifiers::SHIFT) => {
            state.ask_move_option(-1);
        }
        (KeyCode::Down, KeyModifiers::ALT) => {
            state.ask_move_option(1);
        }
        (KeyCode::PageUp, _) => scroll_up(state, page_step(state)),
        (KeyCode::PageDown, _) => scroll_down(state, page_step(state)),
        (KeyCode::Tab, KeyModifiers::SHIFT) => {
            state.ask_move_question(-1);
        }
        (KeyCode::Tab, _) => {
            state.ask_move_question(1);
        }
        (KeyCode::Up, _) => {
            state.ask_move_option(-1);
        }
        (KeyCode::Down, _) => {
            state.ask_move_option(1);
        }
        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() => {
            state.ask_pick(c.to_digit(10).unwrap_or(0) as usize - 1);
        }
        (KeyCode::Char('t'), KeyModifiers::NONE) | (KeyCode::Char('T'), KeyModifiers::NONE) => {
            state.ask_start_text();
        }
        (KeyCode::Enter, _) => {
            state.ask_enter();
        }
        (KeyCode::Esc, _) => {
            state.ask_cancel();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_arg_command_routes_known_commands() {
        assert_eq!(slash_arg_command("/mode"), Some("/mode"));
        assert_eq!(slash_arg_command("/perms"), Some("/permissions"));
        assert_eq!(slash_arg_command("/resume"), Some("/resume"));
        assert_eq!(slash_arg_command("/model"), None);
        assert_eq!(slash_arg_command("/save"), None);
        assert_eq!(slash_arg_command("/cost"), None);
        assert_eq!(slash_arg_command("/quit"), None);
        assert_eq!(slash_arg_command("/nope"), None);
    }

    #[test]
    fn static_arg_matches_filters_by_substring() {
        let all = static_arg_matches("", &[("plan", "p"), ("manual", "m")]);
        assert_eq!(all.len(), 2);
        let one = static_arg_matches("man", &[("plan", "p"), ("manual", "m")]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].insert, "manual");
        assert_eq!(one[0].title, "manual");
        assert_eq!(one[0].detail, "m");
        assert!(static_arg_matches("zzz", &[("plan", "p")]).is_empty());
    }

    #[test]
    fn at_file_ranks_filename_above_path_and_shallow_above_deep() {
        let files = vec![
            "src/domain/handler.rs".to_owned(),
            "main.rs".to_owned(),
            "src/main.rs".to_owned(),
            "docs/readme.md".to_owned(),
        ];
        let titles: Vec<String> = at_file_matches(&files, "main")
            .into_iter()
            .map(|m| m.title)
            .collect();
        assert_eq!(titles[0], "main.rs");
        assert_eq!(titles[1], "src/main.rs");
        assert!(
            titles.contains(&"src/domain/handler.rs".to_owned()),
            "{titles:?}"
        );
        assert!(!titles.contains(&"docs/readme.md".to_owned()));
    }

    #[test]
    fn mode_arg_table_covers_all_modes() {
        let all = static_arg_matches(
            "",
            &[
                ("plan", ""),
                ("manual", ""),
                ("auto", ""),
                ("edit", ""),
                ("yolo", ""),
            ],
        );
        assert_eq!(all.len(), 5);
    }
}
