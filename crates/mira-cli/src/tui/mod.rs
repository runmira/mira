//! Full-screen ratatui frontend for the Mira harness.
//!
//! The event loop selects across three sources:
//!
//! 1. Keyboard events from `crossterm::event::EventStream`
//! 2. Harness events from the stream returned by `Session::send`
//! 3. Approval requests from [`approver::TuiApprover`]
//!
//! Slash commands (`/help`, `/mode`, `/model`, `/clear`, `/quit`) are
//! handled inline. `Ctrl+C` drops the current agent stream — the harness's
//! spawned loop task exits when its send channel closes.

pub mod approver;
mod markdown;
mod render;
mod state;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::stdout;
use futures::stream::BoxStream;
use futures::StreamExt;
use mira_core::Role;
use mira_harness::{Goal, GoalStatus, HarnessEvent, Session, SessionStore};
use mira_policy::{Mode, Policy};
use mira_tools::builtin::skill::SkillHandle;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use approver::ApprovalRequest;
pub use approver::TuiApprover;
use state::{Palette, PaletteItem, PendingApproval, TuiState};

/// Everything the TUI needs beyond what `Session` already owns.
pub struct TuiConfig {
    pub model: String,
    pub mode: Mode,
    /// Handle to the shared policy so `/mode` can update it live.
    pub policy: Arc<Mutex<Policy>>,
    /// Receiver paired with the [`TuiApprover`] handed to `Session`.
    pub approval_rx: mpsc::UnboundedReceiver<ApprovalRequest>,
    /// Repo root — used to resolve relative paths in edit/write diff previews.
    pub cwd: std::path::PathBuf,
    /// Loaded skill registry — the same handle the `Skill` tool consults.
    /// Backs `/skills` (list) and `/skill <name>` (detail) so the user can
    /// inspect the roster without leaving the TUI.
    pub skills: SkillHandle,
    /// Session persistence store, when the run has one. Backs
    /// `/sessions` (list recent in cwd) so the user can find and
    /// re-launch prior conversations without leaving the TUI.
    /// `None` when persistence is disabled (`--no-persist`).
    pub store: Option<Arc<dyn SessionStore>>,
}

/// Built-in slash commands the palette suggests. Order is display order.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "list commands"),
    ("/mode", "switch permission mode (plan|manual|auto|edit|yolo)"),
    ("/model", "switch model for this session"),
    ("/goal", "set / inspect / clear the standing goal"),
    ("/skills", "list loaded skills"),
    ("/skill", "show one skill in detail"),
    ("/permissions", "list session policy · add \"Rule(...)\""),
    ("/undo", "revert the last file write in this session"),
    ("/save", "export the transcript as markdown"),
    ("/sessions", "list recent sessions in this folder"),
    ("/resume", "show shell command to resume a session"),
    ("/budget", "cap this session's spend (e.g. /budget $2 · /budget off)"),
    ("/cost", "print token & dollar breakdown for this session"),
    ("/clear", "clear the visible transcript"),
    ("/quit", "exit the TUI"),
];

/// How often to redraw while a stream is in flight — drives the
/// elapsed-time counter next to "thinking" so long silent gaps don't
/// look hung.
const STREAM_TICK: std::time::Duration = std::time::Duration::from_millis(100);

pub async fn run(session: Session, cfg: TuiConfig) -> Result<()> {
    let mut terminal = enter()?;
    // Track the terminating state's mouse-capture flag so `leave`
    // can pair a matching Disable with any Enable the event loop
    // toggled on. Starts `true` because `enter()` turns capture on.
    let mut mouse_capture_on = true;
    let outcome = event_loop(&mut terminal, session, cfg, &mut mouse_capture_on).await;
    leave(&mut terminal, mouse_capture_on)?;
    outcome
}

// ---- terminal lifecycle ----

fn enter() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();
    // Mouse capture on by default so scroll-wheel drives the transcript
    // (matches Claude Code / most modern TUIs). Users who want native
    // text selection can toggle with Alt+M — most terminals also honour
    // Option/Shift+drag as a "bypass capture" selection modifier.
    execute!(
        out,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn leave(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mouse_capture: bool,
) -> Result<()> {
    disable_raw_mode()?;
    if mouse_capture {
        let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    }
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ---- loop ----

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    session: Session,
    mut cfg: TuiConfig,
    mouse_capture_out: &mut bool,
) -> Result<()> {
    let mut state = TuiState::new(cfg.model.clone(), cfg.mode);
    // `enter()` already turned mouse capture on; mirror it in state so
    // Alt+M can toggle correctly on first press.
    state.mouse_capture = true;
    state.git_branch = detect_git_branch(&cfg.cwd).await;
    hydrate_from_history(&session, &mut state).await;

    // File index for `@` completion — a one-shot `rg --files` in the cwd,
    // shared between palette openings so we don't reshell for every '@'.
    // Computed lazily on first use.
    let mut file_index: Option<Vec<String>> = None;

    let mut input_events = EventStream::new();
    let mut agent_stream: Option<BoxStream<'static, HarnessEvent>> = None;

    loop {
        terminal.draw(|f| render::draw(f, &mut state))?;

        tokio::select! {
            evt = input_events.next() => {
                match evt {
                    Some(Ok(e)) => handle_terminal_event(e, &mut state, &session, &mut agent_stream, &mut cfg, &mut file_index).await,
                    Some(Err(_)) | None => break,
                }
            }
            Some(evt) = next_agent_event(&mut agent_stream) => {
                handle_harness_event(evt, &mut state, &mut agent_stream, &cfg.cwd).await;
            }
            Some(req) = cfg.approval_rx.recv() => {
                let preview = mira_tools::compute_preview(&cfg.cwd, &req.call).await;
                // Stash the diff on the matching ToolCall entry so
                // the completed tool group can render it once the
                // ToolResult lands (closes the "approve → what
                // actually changed?" feedback loop).
                if let Some(p) = preview.as_ref() {
                    state.attach_preview(&req.call.id.to_string(), p.clone());
                }
                state.pending_approval = Some(PendingApproval { request: req, preview });
                // Snap to tail so the inline prompt is visible even if
                // the user had scrolled up mid-turn to read history.
                state.follow_tail = true;
            }
            // Redraw tick while the stream is running so the "3.2s"
            // elapsed counter advances even during silent gaps. Guard
            // with `state.streaming` so we don't burn CPU when idle.
            _ = tokio::time::sleep(STREAM_TICK), if state.streaming => {}
        }

        if state.should_quit {
            break;
        }
    }

    *mouse_capture_out = state.mouse_capture;
    Ok(())
}

/// Detect the current git branch by shelling out. Runs at startup
/// only (branch changes mid-session are rare enough to not be worth
/// a filesystem watcher). Returns `None` outside a repo or on any
/// failure.
async fn detect_git_branch(cwd: &std::path::Path) -> Option<String> {
    let cwd = cwd.to_owned();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&cwd)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if s.is_empty() || s == "HEAD" {
            None
        } else {
            Some(s)
        }
    })
    .await
    .ok()
    .flatten()
}

/// Rebuild the visible transcript from a session's history. On a fresh
/// session (only a system message) we just show the welcome line; on
/// resume we replay user/assistant/tool entries so it's obvious the
/// conversation continued rather than starting empty.
async fn hydrate_from_history(session: &Session, state: &mut TuiState) {
    // Restore any standing goal so the header chip appears the moment
    // a resumed session opens. Kept before the "resumed" info line so
    // the goal is the first thing on screen when it matters.
    state.goal = session.goal().await;

    let history = session.history().await;
    // Only count non-system messages when deciding whether to announce
    // "resumed"; a fresh session has just the system prompt.
    let visible_count = history.iter().filter(|m| m.role != Role::System).count();

    if visible_count == 0 {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~".into());
        // Compact cwd — strip $HOME → `~` so the banner width doesn't
        // depend on how deep the user's home path is nested.
        let cwd_short = if let Some(home) = std::env::var_os("HOME") {
            let home = home.to_string_lossy().to_string();
            cwd.strip_prefix(&home)
                .map(|rest| format!("~{rest}"))
                .unwrap_or(cwd)
        } else {
            cwd
        };

        state.push_info("".to_string());
        state.push_info(format!(
            "  ╭─  ℳ  mira  ·  {}  ·  {}  ─╮",
            state.model,
            state.mode.as_str()
        ));
        state.push_info(format!("  │   cwd  {cwd_short}"));
        state.push_info("  │".to_string());
        state.push_info("  │   / commands   @ files   shift+tab mode   ctrl+r search".to_string());
        state.push_info("  │   ctrl+e expand tool   alt+↑↓ scroll   esc esc quit".to_string());
        state.push_info("  ╰─".to_string());
        state.push_info("".to_string());
        return;
    }

    state.push_info(format!(
        "resumed session {} · {} message{}",
        session.id,
        visible_count,
        if visible_count == 1 { "" } else { "s" }
    ));

    for msg in history {
        match msg.role {
            Role::System => {}
            Role::User => {
                if let Some(c) = msg.content {
                    state.push_user(c);
                }
            }
            Role::Assistant => {
                if let Some(c) = msg.content.as_deref() {
                    if !c.is_empty() {
                        state.push_assistant(c.to_owned());
                    }
                }
                for call in msg.tool_calls {
                    state.push_tool_call_raw(call.function.name, call.function.arguments);
                }
            }
            Role::Tool => {
                if let Some(c) = msg.content {
                    state.push_tool_result_replay(&c);
                }
            }
        }
    }
}

async fn next_agent_event(
    stream: &mut Option<BoxStream<'static, HarnessEvent>>,
) -> Option<HarnessEvent> {
    match stream {
        Some(s) => s.next().await,
        // If there's no live stream, park forever. `tokio::select!` polls
        // us again as soon as one appears.
        None => std::future::pending().await,
    }
}

async fn handle_terminal_event(
    evt: Event,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    match evt {
        Event::Paste(s) => {
            state.input_push_str(&s);
            state.esc_pending = false;
            state.flash = None;
            refresh_palette(state, cfg, file_index).await;
        }
        Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press => {
            handle_key(k, state, session, agent_stream, cfg, file_index).await;
        }
        Event::Mouse(m) => handle_mouse(m, state),
        _ => {}
    }
}

/// Drop the current harness stream and record a warning entry.
/// Invoked from both Ctrl+C and (single-press) Esc during a turn, so
/// the two shortcuts stay in sync — no risk of one leaving the state
/// half-torn-down.
fn interrupt_stream(
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
) {
    if agent_stream.is_none() {
        return;
    }
    let msg = match state.in_flight_tool() {
        Some((name, args)) => {
            let short = truncate_for_warning(&args, 80);
            format!(
                "interrupted while `{name}({short})` was running — tool result discarded"
            )
        }
        None => "interrupted".to_owned(),
    };
    *agent_stream = None;
    state.streaming = false;
    state.stream_started_at = None;
    state.push_warning(msg);
}

/// Scroll the transcript by `n` rows toward the top.
fn scroll_up(state: &mut TuiState, n: u16) {
    state.follow_tail = false;
    state.scroll = state.scroll.saturating_sub(n);
}

/// Scroll the transcript by `n` rows toward the bottom. Re-engages
/// follow-tail once the user catches up with the newest content, so
/// streamed tokens keep landing in view.
fn scroll_down(state: &mut TuiState, n: u16) {
    let new = state.scroll.saturating_add(n);
    if new >= state.transcript_tail {
        state.scroll = state.transcript_tail;
        state.follow_tail = true;
    } else {
        state.scroll = new;
        state.follow_tail = false;
    }
}

/// A page's worth of rows for PgUp/PgDn — most of the viewport, minus a
/// small overlap so the reader can anchor on a familiar line between
/// jumps. Clamped to a sane minimum for tiny terminals.
fn page_step(state: &TuiState) -> u16 {
    let vh = state.viewport_height.max(4);
    vh.saturating_sub(2).max(1)
}

fn handle_mouse(m: MouseEvent, state: &mut TuiState) {
    match m.kind {
        MouseEventKind::ScrollUp => scroll_up(state, 3),
        MouseEventKind::ScrollDown => scroll_down(state, 3),
        _ => {}
    }
}

async fn handle_key(
    key: KeyEvent,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    // Approval modal steals the keys.
    if state.pending_approval.is_some() {
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
    if state.palette.kind != Palette::None {
        if handle_palette_key(key, state) {
            return;
        }
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
                state.esc_pending = false;
                return;
            }
            if state.esc_pending {
                if !state.is_input_empty() {
                    let _ = state.input_clear();
                    state.flash = Some("input cleared".into());
                } else {
                    state.should_quit = true;
                }
            } else {
                state.esc_pending = true;
            }
            return;
        }
        (KeyCode::Enter, m) if !m.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) => {
            if state.is_input_empty() || state.streaming {
                return;
            }
            let text = state.input_clear();
            state.palette = state::PaletteState::none();
            if text.starts_with('/') {
                // A slash command may either mutate local state (e.g. `/model`)
                // or synthesize a user message to send (e.g. `/review`, which
                // asks the model to invoke the code-review skill). Only the
                // latter opens a stream.
                if let Some(followup) = run_slash(&text, state, session, cfg).await {
                    start_stream(state, session, agent_stream, followup).await;
                }
            } else {
                start_stream(state, session, agent_stream, text).await;
            }
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
            let mut out = stdout();
            if state.mouse_capture {
                let _ = execute!(out, EnableMouseCapture);
                state.flash = Some("mouse capture: on".into());
            } else {
                let _ = execute!(out, DisableMouseCapture);
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
        (KeyCode::PageUp, _) => scroll_up(state, page_step(state)),
        (KeyCode::PageDown, _) => scroll_down(state, page_step(state)),
        // ---- history recall ----
        (KeyCode::Up, _) => state.history_prev(),
        (KeyCode::Down, _) => state.history_next(),
        // ---- printable ----
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
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
            // (#2, #8) Only log when we actually mutated policy —
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
            state.palette = state::PaletteState::none();
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
        state.palette = state::PaletteState::none();
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
        Palette::None => {}
    }
    state.palette = state::PaletteState::none();
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

/// Recompute the palette matches based on the composer's current
/// contents. Called after every input mutation. Opens or closes the
/// palette as needed.
async fn refresh_palette(
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

    // @file picker when the cursor sits inside an `@word` run.
    if let Some((word_start, word_end)) = find_at_word(state.input(), state.cursor()) {
        let filter = state.input()[word_start + 1..word_end].to_ascii_lowercase();
        let files = ensure_file_index(file_index, &cfg.cwd).await;
        let matches = at_file_matches(files, &filter);
        open_palette(state, Palette::AtFile, matches);
        return;
    }

    // No trigger — close the palette.
    state.palette = state::PaletteState::none();
}

fn open_palette(state: &mut TuiState, kind: Palette, matches: Vec<PaletteItem>) {
    let cursor = if matches.is_empty() {
        0
    } else {
        state.palette.cursor.min(matches.len() - 1)
    };
    // Reset cursor when switching kinds so a fresh open doesn't inherit
    // a stale selection from a different list.
    let cursor = if state.palette.kind != kind { 0 } else { cursor };
    state.palette = state::PaletteState {
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
        .filter(|(name, _)| name.trim_start_matches('/').to_ascii_lowercase().contains(filter))
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

async fn handle_harness_event(
    evt: HarnessEvent,
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cwd: &std::path::Path,
) {
    match evt {
        HarnessEvent::Token(t) => state.append_token(&t),
        HarnessEvent::ToolStart(call) => {
            state.push_tool_call(&call);
            // (ask 4) Compute the diff preview eagerly, not just on
            // approval — otherwise session-allow'd `edit_file` /
            // `write_file` calls render as a bare "edited /path (1
            // replacement)" line with no coloured diff, which is the
            // whole point of showing the change inline. `compute_preview`
            // returns `None` for tools that don't produce diffs, so
            // this is a no-op for `read_file`, `bash`, etc.
            if let Some(p) = mira_tools::compute_preview(cwd, &call).await {
                state.attach_preview(&call.id.to_string(), p);
            }
        }
        HarnessEvent::ToolEnd(result) => state.push_tool_result(&result),
        HarnessEvent::Warning(w) => state.push_warning(w),
        HarnessEvent::TurnComplete => {}
        HarnessEvent::Usage { totals, .. } => state.usage = totals,
        HarnessEvent::MemoryLearned { count } => {
            state.push_warning(format!(
                "[memory] remembered {count} thing{}",
                if count == 1 { "" } else { "s" }
            ));
        }
        HarnessEvent::Compacted { messages_removed } => {
            state.push_warning(format!(
                "[context] compacted {messages_removed} earlier message{} into a summary",
                if messages_removed == 1 { "" } else { "s" }
            ));
        }
        HarnessEvent::GoalSet { goal } => {
            state.push_info(format!("[goal] set: {}", goal.condition));
            state.goal = Some(goal);
        }
        HarnessEvent::GoalCleared => {
            state.push_info("[goal] cleared".to_string());
            state.goal = None;
        }
        HarnessEvent::GoalProgress {
            iteration,
            max_iterations,
            status,
            reason,
        } => {
            let status_word = match status {
                GoalStatus::Active => "still working",
                GoalStatus::Met => "met",
                GoalStatus::Impossible => "impossible",
                GoalStatus::NeedsUser => "needs you",
                GoalStatus::Cleared => "cleared",
                GoalStatus::Exhausted => "exhausted",
            };
            state.push_info(format!(
                "[goal] {iteration}/{max_iterations} · {status_word}{}",
                reason
                    .as_ref()
                    .map(|r| format!(" · {r}"))
                    .unwrap_or_default()
            ));
            if let Some(g) = state.goal.as_mut() {
                g.iterations = iteration;
                g.max_iterations = max_iterations;
                g.status = status;
                g.last_reason = reason;
            }
        }
        HarnessEvent::GoalDone { status, reason } => {
            let word = match status {
                GoalStatus::Met => "met",
                GoalStatus::Impossible => "impossible",
                GoalStatus::NeedsUser => "needs you",
                GoalStatus::Exhausted => "exhausted",
                GoalStatus::Cleared => "cleared",
                GoalStatus::Active => "active",
            };
            let msg = match reason {
                Some(r) => format!("[goal] {word} · {r}"),
                None => format!("[goal] {word}"),
            };
            state.push_info(msg);
            if let Some(g) = state.goal.as_mut() {
                g.status = status;
            }
        }
        HarnessEvent::ToolProgress { .. } => {
            // Deliberately silent. The pulsing `ℳ` indicator + the
            // in-flight `◐` on the tool call already show "something
            // is happening"; dumping every stdout line as an info
            // entry buries the actual transcript in shell noise.
            // Final content still lands via `ToolEnd` and is one
            // Ctrl+E away.
        }
        HarnessEvent::ToolPreview { .. } => {
            // The TUI already renders diffs via its own approval flow;
            // skip the harness-side preview to avoid double rendering.
        }
        HarnessEvent::Done => {
            state.streaming = false;
            state.stream_started_at = None;
            *agent_stream = None;
        }
    }
}

/// Push a user message and open the harness stream — factored so the
/// composer-enter path and slash-triggered skill invocations both hit
/// the same wiring (`remember_submission`, follow_tail, stream_started_at).
///
/// If the session's cost has already exceeded `state.budget_usd`, this
/// refuses to send and pushes a warning instead. The user's cap → the
/// user's call: they either raise it (`/budget $X`) or clear it
/// (`/budget off`) before the next turn goes out.
async fn start_stream(
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    text: String,
) {
    if let Some(cap) = state.budget_usd {
        if let Some(spent) = current_cost_usd(state) {
            if spent >= cap {
                state.push_warning(format!(
                    "over budget — {} spent ≥ ${:.2} cap. \
                     /budget off to keep going · /budget $X to raise",
                    format_dollars_short(spent),
                    cap,
                ));
                // Return the text to the composer so the user's message
                // isn't silently lost by the guardrail.
                let _ = state.input_replace(&text);
                return;
            }
        }
    }
    state.remember_submission(&text);
    state.push_user(text.clone());
    state.streaming = true;
    state.stream_started_at = Some(std::time::Instant::now());
    state.follow_tail = true;
    *agent_stream = Some(session.send(text).await);
}

/// Current session cost in USD, if the model is priced and the provider
/// has reported at least one usage round. Mirrors the formula used in
/// the header status strip so both surfaces agree.
pub(super) fn current_cost_usd(state: &TuiState) -> Option<f64> {
    let u = &state.usage;
    if u.is_zero() {
        return None;
    }
    mira_ai::cost_usd(
        &state.model,
        mira_ai::TokenUsage {
            prompt_tokens: u.prompt_tokens.min(u32::MAX as u64) as u32,
            completion_tokens: u.completion_tokens.min(u32::MAX as u64) as u32,
            cached_input_tokens: u.cached_input_tokens.min(u32::MAX as u64) as u32,
        },
    )
}

/// Short USD format that keeps small values readable (`$0.024`) and
/// large ones compact (`$12.4`). Mirrors the render.rs helper — kept
/// as a private duplicate to avoid re-exporting a whole rendering
/// module just for one formatter.
fn format_dollars_short(d: f64) -> String {
    if d >= 1.0 {
        format!("${d:.2}")
    } else {
        format!("${d:.3}")
    }
}

/// Parse the argument to `/budget`. Accepts `off`, `clear`, `none` for
/// disable; otherwise strips a leading `$` and reads an f64.
fn parse_budget(rest: &str) -> Result<Option<f64>, String> {
    let t = rest.trim();
    if t.is_empty() {
        return Err("usage: /budget $X · /budget off".into());
    }
    if matches!(t.to_ascii_lowercase().as_str(), "off" | "clear" | "none") {
        return Ok(None);
    }
    let raw = t.trim_start_matches('$').trim();
    let n: f64 = raw
        .parse()
        .map_err(|_| format!("can't parse `{t}` as a dollar amount"))?;
    if !n.is_finite() || n <= 0.0 {
        return Err(format!("budget must be positive · got {t}"));
    }
    Ok(Some(n))
}

/// True when `head` (with the leading `/`) is one of the built-in slash
/// commands. Built-ins always win over a skill alias — a skill named
/// `mode.md` can't shadow `/mode`.
fn is_reserved_slash(head: &str) -> bool {
    SLASH_COMMANDS.iter().any(|(name, _)| *name == head)
        || matches!(head, "/q" | "/?" | "/perms")
}

/// Look up a skill by its slash alias. Returns the underlying skill
/// name so callers can synthesize an invocation regardless of whether
/// the alias matches the skill's own name (`/verify` → `verify`) or
/// renames it (`/review` → `code-review`).
async fn find_skill_by_slash(
    slash: &str,
    skills: &mira_tools::builtin::skill::SkillHandle,
) -> Option<String> {
    let reg = skills.read().await.clone();
    reg.skills
        .values()
        .find(|s| s.slash.as_deref() == Some(slash))
        .map(|s| s.name.clone())
}

async fn run_slash(
    cmd: &str,
    state: &mut TuiState,
    session: &Session,
    cfg: &mut TuiConfig,
) -> Option<String> {
    let mut parts = cmd.trim().splitn(2, ' ');
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match head {
        "/quit" | "/q" => state.should_quit = true,

        "/clear" => {
            // clear_entries only touches the transcript + scroll — mode,
            // model, goal, usage totals, and policy stay put so /clear
            // reads as a "focus reset", not a session teardown. The info
            // line below reminds the user what's preserved.
            state.clear_entries();
            let mut kept = format!(
                "cleared · model {} · mode {}",
                state.model,
                state.mode.as_str()
            );
            if let Some(g) = state.goal.as_ref() {
                kept.push_str(&format!(" · goal {}/{}", g.iterations, g.max_iterations));
            }
            state.push_info(kept);
            state.flash = Some("cleared".into());
        }

        "/help" | "/?" => {
            state.push_info(
                "commands: /mode <plan|manual|auto|edit|yolo> · /model <id> · \
                 /goal <cond> · /goal status · /goal clear · /skills · \
                 /skill <name> · /budget <$X|off> · \
                 /permissions [add \"Rule(...)\"] · \
                 /undo [N] · /save [path] · /clear · /quit  ·  \
                 keys: @ file · / cmd · ctrl+r search · \
                 ctrl+y copy last reply · ctrl+e expand last tool · \
                 ctrl+w kill word",
            );
            // Also enumerate the mounted skill slashes — they change
            // per project, so hard-coding them in the line above would
            // rot. `/skills` still shows the full detail view.
            let reg = cfg.skills.read().await.clone();
            let mut aliases: Vec<String> = reg
                .skills
                .values()
                .filter_map(|s| s.slash.as_deref().map(|a| format!("/{a}")))
                .filter(|s| !is_reserved_slash(s))
                .collect();
            aliases.sort();
            aliases.dedup();
            if !aliases.is_empty() {
                state.push_info(format!("skill slashes: {}", aliases.join(" · ")));
            }
        }

        "/goal" => run_goal_slash(rest, state, session).await,

        "/skills" => run_skills_slash(state, &cfg.skills).await,

        "/skill" => run_skill_slash(rest, state, &cfg.skills).await,

        "/save" => run_save_slash(rest, state, &cfg.cwd),

        "/sessions" => run_sessions_slash(state, cfg).await,

        "/resume" => run_resume_slash(rest, state, cfg).await,

        "/undo" => run_undo_slash(rest, state, session).await,

        "/permissions" | "/perms" => run_permissions_slash(rest, state, cfg).await,

        "/mode" => match parse_mode(rest) {
            Some(m) => {
                state.mode = m;
                cfg.policy.lock().await.set_mode(m);
                state.flash = Some(format!("mode → {}", m.as_str()));
            }
            None => state.push_warning(format!("unknown mode `{rest}`")),
        },

        "/model" => {
            if rest.is_empty() {
                state.push_warning("usage: /model <id>".into());
            } else {
                state.model = rest.to_owned();
                session.set_model(rest.to_owned()).await;
                state.flash = Some(format!("model → {rest}"));
            }
        }

        "/budget" => match parse_budget(rest) {
            Ok(None) => {
                state.budget_usd = None;
                state.flash = Some("budget cleared".into());
            }
            Ok(Some(cap)) => {
                state.budget_usd = Some(cap);
                state.flash = Some(format!("budget → ${cap:.2}"));
                if let Some(spent) = current_cost_usd(state) {
                    if spent >= cap {
                        state.push_warning(format!(
                            "already at {} — next send blocked until you raise or clear the cap",
                            format_dollars_short(spent)
                        ));
                    }
                }
            }
            Err(msg) => state.push_warning(msg),
        },

        "/cost" => {
            let u = state.usage;
            if u.is_zero() {
                state.push_info("no usage reported yet".to_string());
            } else {
                let mut line = format!(
                    "cost · ↑{} ↓{} (cached {}) · {} rounds",
                    u.prompt_tokens, u.completion_tokens, u.cached_input_tokens, u.rounds
                );
                if let Some(spent) = current_cost_usd(state) {
                    line.push_str(&format!(" · {}", format_dollars_short(spent)));
                    if let Some(cap) = state.budget_usd {
                        let left = (cap - spent).max(0.0);
                        line.push_str(&format!(
                            " / ${cap:.2} cap · ${left:.3} left"
                        ));
                    }
                } else if state.budget_usd.is_some() {
                    line.push_str(" · (model unpriced — budget won't trip)");
                }
                state.push_info(line);
            }
        }

        // Fall through to the skill registry: any skill whose
        // frontmatter declares `slash: X` (default `X = skill.name`)
        // mounts as `/X`. Reserved commands above always win.
        other => {
            let alias = other.trim_start_matches('/');
            if let Some(skill_name) = find_skill_by_slash(alias, &cfg.skills).await {
                state.flash = Some(format!("skill → {skill_name}"));
                let arg_line = if rest.is_empty() {
                    String::new()
                } else {
                    format!("\n\nInvocation arg: {rest}")
                };
                return Some(format!(
                    "Please invoke the `{skill_name}` skill.{arg_line}"
                ));
            }
            state.push_warning(format!("unknown command `{other}` — try /help"));
        }
    }
    None
}

/// Dispatch for `/goal ...` — the sub-verb decides.
///
/// Shape:
/// - `/goal <condition>`     — set (or replace) the standing goal
/// - `/goal status`          — print the current goal
/// - `/goal clear`           — drop the goal
async fn run_goal_slash(rest: &str, state: &mut TuiState, session: &Session) {
    let rest = rest.trim();
    if rest.is_empty() || rest == "status" {
        // Snapshot to a local so the immutable borrow on `state.goal`
        // ends before we call the `&mut` push_info helpers.
        let snapshot = state.goal.clone();
        match snapshot {
            None => state.push_info(
                "no goal set. `/goal <condition>` to start an autonomous run.".to_string(),
            ),
            Some(g) => {
                let status_word = match g.status {
                    GoalStatus::Active => "active",
                    GoalStatus::Met => "met",
                    GoalStatus::Impossible => "impossible",
                    GoalStatus::NeedsUser => "needs you",
                    GoalStatus::Cleared => "cleared",
                    GoalStatus::Exhausted => "exhausted",
                };
                state.push_info(format!(
                    "goal · {status_word} · {}/{} · {}",
                    g.iterations, g.max_iterations, g.condition
                ));
                if let Some(r) = g.last_reason.as_ref() {
                    state.push_info(format!("last note: {r}"));
                }
            }
        }
        return;
    }
    if rest == "clear" {
        session.clear_goal().await;
        state.goal = None;
        state.push_info("[goal] cleared".to_string());
        state.flash = Some("goal cleared".into());
        return;
    }
    // Otherwise treat `rest` as the goal condition — set it.
    let goal = Goal::new(rest);
    session.set_goal(goal.clone()).await;
    state.goal = Some(goal.clone());
    state.push_info(format!("[goal] set: {}", goal.condition));
    state.flash = Some("goal set".into());
}

/// `/skills` — one line per loaded skill (bundled + user + project
/// merged, same view the composer palette in `mira serve` sees). Fires
/// as info entries so scroll-back keeps them.
async fn run_skills_slash(state: &mut TuiState, skills: &SkillHandle) {
    let reg = skills.read().await.clone();
    if reg.skills.is_empty() {
        state.push_info(
            "no skills loaded. drop a SKILL.md into ~/.mira/skills/<name>/ to add one.".to_string(),
        );
        return;
    }
    state.push_info(format!(
        "{} skill{} loaded:",
        reg.skills.len(),
        if reg.skills.len() == 1 { "" } else { "s" }
    ));
    for s in reg.skills.values() {
        state.push_info(format!("  /{:<24} {}", s.name, s.description));
    }
}

/// `/skill <name>` — description + source + attachment list + body.
/// Emitted line-by-line so the terminal transcript stays scroll-back
/// searchable rather than being one giant blob.
async fn run_skill_slash(rest: &str, state: &mut TuiState, skills: &SkillHandle) {
    let name = rest.split_whitespace().next().unwrap_or("").trim();
    if name.is_empty() {
        state.push_warning("usage: /skill <name>".into());
        return;
    }
    let reg = skills.read().await.clone();
    let Some(s) = reg.get(name).cloned() else {
        state.push_warning(format!("no skill named `{name}` — `/skills` to list."));
        return;
    };
    state.push_info(format!("/{} — {}", s.name, s.description));
    match s.source.as_ref() {
        Some(p) => state.push_info(format!("source: {}", p.display())),
        None => state.push_info("source: (bundled)".to_string()),
    }
    let attachments = s.attached_files();
    if !attachments.is_empty() {
        state.push_info(format!(
            "attached ({}): {}",
            attachments.len(),
            attachments
                .iter()
                .filter_map(|p| p.to_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    state.push_info("---".to_string());
    for line in s.body.lines() {
        state.push_info(line.to_string());
    }
}

/// `/undo [N]` — revert the last N file writes made in this session.
/// Defaults to 1. No-op with a clean flash message when there's
/// nothing on the guard's stack (or no guard at all).
async fn run_undo_slash(rest: &str, state: &mut TuiState, session: &Session) {
    let n: usize = rest.trim().parse().unwrap_or(1);
    let Some(guard) = session.file_guard() else {
        state.push_warning("undo isn't wired for this session (no FileGuard)".into());
        return;
    };
    match guard.undo(n).await {
        Ok(applied) => {
            if applied.is_empty() {
                state.push_info("nothing to undo");
                return;
            }
            let paths: Vec<&str> = applied.iter().map(|a| a.path.as_str()).collect();
            state.push_info(format!(
                "[undo] reverted {} write{}: {}",
                applied.len(),
                if applied.len() == 1 { "" } else { "s" },
                paths.join(", ")
            ));
            state.flash = Some("undone".into());
        }
        Err(e) => state.push_warning(format!("undo failed: {e}")),
    }
}

/// `/permissions` — inspect or extend the session policy.
///
/// Shape:
/// - `/permissions`                    → list current allow rules
/// - `/permissions add "Rule(...)"`    → session-scoped add
/// - `/permissions add Bash(cargo test:*)` → also accepted (quotes optional)
async fn run_permissions_slash(rest: &str, state: &mut TuiState, cfg: &TuiConfig) {
    let rest = rest.trim();
    if rest.is_empty() || rest == "ls" || rest == "list" {
        let policy = cfg.policy.lock().await;
        let n = policy.allow_count();
        state.push_info(format!(
            "policy · mode {} · {} allow rule{}",
            policy.mode().as_str(),
            n,
            if n == 1 { "" } else { "s" }
        ));
        state.push_info(
            "session-scoped rules aren't persisted; run `mira config edit` to save.".to_string(),
        );
        return;
    }
    if let Some(rest) = rest.strip_prefix("add ").map(str::trim) {
        // Strip a single pair of surrounding quotes if present.
        let rule = rest
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(rest);
        match cfg.policy.lock().await.add_allow_rule(rule) {
            Ok(_) => {
                state.push_info(format!("[policy] session-allow: {rule}"));
                state.flash = Some("rule added".into());
            }
            Err(e) => state.push_warning(format!("bad rule `{rule}`: {e}")),
        }
        return;
    }
    state.push_warning(
        "usage: /permissions  ·  /permissions add \"Rule(pattern)\"".into(),
    );
}

/// `/save [path]` — dump the visible transcript to a markdown file.
/// Default path is `<cwd>/.mira/transcript-<epoch>.md`; a caller-supplied
/// path is used verbatim.
fn run_save_slash(rest: &str, state: &mut TuiState, cwd: &std::path::Path) {
    let path: PathBuf = if rest.is_empty() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        cwd.join(".mira").join(format!("transcript-{ts}.md"))
    } else {
        PathBuf::from(rest)
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for e in state.entries() {
        match e {
            state::LogEntry::User(s) => {
                out.push_str("**you:** ");
                out.push_str(s);
                out.push_str("\n\n");
            }
            state::LogEntry::Assistant(s) => {
                out.push_str(s);
                out.push_str("\n\n");
            }
            state::LogEntry::ToolCall { name, args, .. } => {
                out.push_str(&format!("_tool call:_ `{name}({args})`\n\n"));
            }
            state::LogEntry::ToolResult {
                ok, snippet, full, ..
            } => {
                let mark = if *ok { "✓" } else { "✗" };
                let body = if full.is_empty() { snippet } else { full };
                out.push_str(&format!("_tool result {mark}:_\n```\n{body}\n```\n\n"));
            }
            state::LogEntry::Warning(s) => {
                out.push_str(&format!("> ⚠ {s}\n\n"));
            }
            state::LogEntry::Info(s) => {
                out.push_str(&format!("_{s}_\n\n"));
            }
        }
    }
    match std::fs::write(&path, out) {
        Ok(_) => {
            state.push_info(format!("saved transcript → {}", path.display()));
            state.flash = Some("saved".into());
        }
        Err(e) => state.push_warning(format!("save failed: {e}")),
    }
}

/// `/sessions` — list up to 15 recent sessions in the current cwd
/// with a title + relative-time chip. Read-only: switching to a
/// different session requires re-launching mira (see `/resume`).
async fn run_sessions_slash(state: &mut TuiState, cfg: &TuiConfig) {
    let Some(store) = cfg.store.as_ref() else {
        state.push_warning(
            "session persistence is off (`--no-persist`?) — nothing to list".into(),
        );
        return;
    };
    match store.list_recent(&cfg.cwd, 15).await {
        Err(e) => state.push_warning(format!("couldn't list sessions: {e}")),
        Ok(list) if list.is_empty() => {
            state.push_info(format!(
                "no saved sessions in {} — try coming back after a chat lands.",
                cfg.cwd.display()
            ));
        }
        Ok(list) => {
            state.push_info(format!("recent sessions in {}:", cfg.cwd.display()));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for rec in list {
                let id_short = short_session_id(rec.id.as_str());
                let title = rec
                    .title
                    .clone()
                    .or_else(|| first_user_message_from_record(&rec))
                    .unwrap_or_else(|| "(no messages yet)".into());
                let title = title.trim().replace('\n', " ");
                let title = if title.chars().count() > 60 {
                    let head: String = title.chars().take(60).collect();
                    format!("{head}…")
                } else {
                    title
                };
                let age = fmt_relative(now.saturating_sub(rec.updated_at));
                state.push_info(format!("  {id_short}  ·  {age:>8}  ·  {title}"));
            }
            state.push_info("resume: run `mira --resume <id>` (or `mira --pick`) from a new shell.".to_string());
        }
    }
}

/// `/resume` — the TUI can't hot-swap sessions safely (Session owns a
/// bunch of channels wired into the harness loop task). Instead, we
/// print the exact shell command that resumes the target session, so
/// the user can `Ctrl+C` out and paste it. `<id>` optional — omitted
/// resumes the most recent.
async fn run_resume_slash(rest: &str, state: &mut TuiState, cfg: &TuiConfig) {
    let target = rest.trim();
    let cmd = if target.is_empty() {
        "mira --resume ''".to_string()
    } else {
        format!("mira --resume {target}")
    };
    state.push_info(
        "resume can't hot-swap sessions mid-run — the harness owns the current one. \
         quit and run this in your shell:"
            .to_string(),
    );
    state.push_info(format!("  $ {cmd}"));
    if !target.is_empty() {
        state.push_info(
            "(or `mira --pick` for an interactive picker over recent sessions)".to_string(),
        );
    }
    // Also: if we know the session store, verify the id exists so users
    // don't quit only to hit "not found".
    if !target.is_empty() {
        if let Some(store) = cfg.store.as_ref() {
            if let Err(e) = store.load(&mira_core::SessionId::from(target)).await {
                state.push_warning(format!("(note: session lookup failed: {e})"));
            }
        }
    }
}

fn short_session_id(id: &str) -> String {
    // Session ids are usually uuid-like; the first 8 chars are enough
    // to disambiguate within a cwd. Fall back to the whole thing for
    // short custom ids.
    if id.chars().count() > 12 {
        id.chars().take(8).collect::<String>()
    } else {
        id.to_owned()
    }
}

fn first_user_message_from_record(rec: &mira_harness::SessionRecord) -> Option<String> {
    rec.messages
        .iter()
        .find(|m| m.role == mira_core::Role::User)
        .and_then(|m| m.content.clone())
}

/// Coarse "5m ago", "2d ago". Only used in `/sessions` so precision
/// doesn't matter past the right unit.
fn fmt_relative(secs: u64) -> String {
    if secs < 60 {
        "just now".to_owned()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Truncate arbitrary content for a one-line warning display — safe
/// on multi-byte chars and appends `…` when it clips.
fn truncate_for_warning(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars).collect();
    format!("{head}…")
}

fn parse_mode(s: &str) -> Option<Mode> {
    match s {
        "plan" => Some(Mode::Plan),
        "manual" => Some(Mode::Manual),
        "auto" => Some(Mode::Auto),
        "edit" => Some(Mode::Edit),
        "yolo" => Some(Mode::Yolo),
        _ => None,
    }
}
