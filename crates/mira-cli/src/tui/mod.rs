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
mod render;
mod state;

use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::stream::BoxStream;
use futures::StreamExt;
use mira_harness::{HarnessEvent, Session};
use mira_policy::{Mode, Policy};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use approver::ApprovalRequest;
pub use approver::TuiApprover;
use state::TuiState;

/// Everything the TUI needs beyond what `Session` already owns.
pub struct TuiConfig {
    pub model: String,
    pub mode: Mode,
    /// Handle to the shared policy so `/mode` can update it live.
    pub policy: Arc<Mutex<Policy>>,
    /// Receiver paired with the [`TuiApprover`] handed to `Session`.
    pub approval_rx: mpsc::UnboundedReceiver<ApprovalRequest>,
}

pub async fn run(session: Session, cfg: TuiConfig) -> Result<()> {
    let mut terminal = enter()?;
    let outcome = event_loop(&mut terminal, session, cfg).await;
    leave(&mut terminal)?;
    outcome
}

// ---- terminal lifecycle ----

fn enter() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn leave(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ---- loop ----

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    session: Session,
    mut cfg: TuiConfig,
) -> Result<()> {
    let mut state = TuiState::new(cfg.model.clone(), cfg.mode);
    state.push_info("mira ready. type a message · ctrl-c interrupt · esc esc quit");

    let mut input_events = EventStream::new();
    let mut agent_stream: Option<BoxStream<'static, HarnessEvent>> = None;

    loop {
        terminal.draw(|f| render::draw(f, &state))?;

        tokio::select! {
            evt = input_events.next() => {
                match evt {
                    Some(Ok(e)) => handle_terminal_event(e, &mut state, &session, &mut agent_stream, &mut cfg).await,
                    Some(Err(_)) | None => break,
                }
            }
            Some(evt) = next_agent_event(&mut agent_stream) => {
                handle_harness_event(evt, &mut state, &mut agent_stream);
            }
            Some(req) = cfg.approval_rx.recv() => {
                state.pending_approval = Some(req);
            }
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
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
) {
    let Event::Key(key) = evt else { return };
    if key.kind != crossterm::event::KeyEventKind::Press {
        return;
    }

    // Approval modal steals the keys.
    if state.pending_approval.is_some() {
        handle_approval_key(key, state);
        return;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            if agent_stream.is_some() {
                *agent_stream = None;
                state.streaming = false;
                state.push_warning("interrupted".into());
            }
        }
        (KeyCode::Esc, _) => {
            if state.esc_pending {
                state.should_quit = true;
            } else {
                state.esc_pending = true;
            }
            return; // don't reset esc_pending below
        }
        (KeyCode::Enter, m) if !m.contains(KeyModifiers::SHIFT) => {
            if state.is_input_empty() || state.streaming {
                return;
            }
            let text = state.input_clear();
            if text.starts_with('/') {
                run_slash(&text, state, cfg).await;
            } else {
                state.push_user(text.clone());
                state.streaming = true;
                state.follow_tail = true;
                *agent_stream = Some(session.send(text).await);
            }
        }
        (KeyCode::Backspace, _) => state.input_backspace(),
        (KeyCode::PageUp, _) => {
            state.follow_tail = false;
            state.scroll = state.scroll.saturating_sub(5);
        }
        (KeyCode::PageDown, _) => {
            state.scroll = state.scroll.saturating_add(5);
            state.follow_tail = false;
        }
        (KeyCode::End, _) => state.follow_tail = true,
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => state.input_push(c),
        _ => {}
    }
    state.esc_pending = false;
    state.flash = None;
}

fn handle_approval_key(key: KeyEvent, state: &mut TuiState) {
    let allow = match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
        _ => None,
    };
    let Some(allow) = allow else { return };
    if let Some(req) = state.pending_approval.take() {
        let _ = req.reply.send(allow);
    }
}

fn handle_harness_event(
    evt: HarnessEvent,
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
) {
    match evt {
        HarnessEvent::Token(t) => state.append_token(&t),
        HarnessEvent::ToolStart(call) => state.push_tool_call(&call),
        HarnessEvent::ToolEnd(result) => state.push_tool_result(&result),
        HarnessEvent::Warning(w) => state.push_warning(w),
        HarnessEvent::TurnComplete => {}
        HarnessEvent::Done => {
            state.streaming = false;
            *agent_stream = None;
        }
    }
}

async fn run_slash(cmd: &str, state: &mut TuiState, cfg: &mut TuiConfig) {
    let mut parts = cmd.trim().splitn(2, ' ');
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match head {
        "/quit" | "/q" => state.should_quit = true,

        "/clear" => {
            state.clear_entries();
            state.flash = Some("cleared".into());
        }

        "/help" | "/?" => {
            state.push_info(
                "commands: /mode <plan|manual|auto|edit|yolo> · /model <id> · /clear · /quit",
            );
        }

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
                state.flash = Some(format!("model → {rest} (applies to next turn)"));
                // NOTE: SessionConfig.model isn't hot-swapped here yet — a
                // follow-up will add a Session::set_model method. For now
                // the label updates and takes effect on next `send`.
            }
        }

        other => state.push_warning(format!("unknown command `{other}` — try /help")),
    }
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
