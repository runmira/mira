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
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::stream::BoxStream;
use futures::StreamExt;
use mira_core::Role;
use mira_harness::{HarnessEvent, Session};
use mira_policy::{Mode, Policy};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use approver::ApprovalRequest;
pub use approver::TuiApprover;
use state::{PendingApproval, TuiState};

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
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn leave(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
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
) -> Result<()> {
    let mut state = TuiState::new(cfg.model.clone(), cfg.mode);
    hydrate_from_history(&session, &mut state).await;

    let mut input_events = EventStream::new();
    let mut agent_stream: Option<BoxStream<'static, HarnessEvent>> = None;

    loop {
        terminal.draw(|f| render::draw(f, &mut state))?;

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
                let preview = mira_tools::compute_preview(&cfg.cwd, &req.call).await;
                state.pending_approval = Some(PendingApproval { request: req, preview });
            }
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
}

/// Rebuild the visible transcript from a session's history. On a fresh
/// session (only a system message) we just show the welcome line; on
/// resume we replay user/assistant/tool entries so it's obvious the
/// conversation continued rather than starting empty.
async fn hydrate_from_history(session: &Session, state: &mut TuiState) {
    let history = session.history().await;
    // Only count non-system messages when deciding whether to announce
    // "resumed"; a fresh session has just the system prompt.
    let visible_count = history.iter().filter(|m| m.role != Role::System).count();

    if visible_count == 0 {
        state.push_info("mira ready. type a message · ctrl-c interrupt · esc esc quit");
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
) {
    match evt {
        Event::Paste(s) => {
            state.input_push_str(&s);
            state.esc_pending = false;
            state.flash = None;
        }
        Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press => {
            handle_key(k, state, session, agent_stream, cfg).await;
        }
        _ => {}
    }
}

async fn handle_key(
    key: KeyEvent,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
) {
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
        // Ctrl+J → newline in the composer. Terminals send LF (0x0A) for
        // Ctrl+J and CR (0x0D) for Enter, which crossterm maps to
        // KeyCode::Enter with the CONTROL modifier for the LF case.
        (KeyCode::Char('j'), KeyModifiers::CONTROL)
        | (KeyCode::Enter, KeyModifiers::CONTROL)
        | (KeyCode::Enter, KeyModifiers::SHIFT) => state.input_newline(),
        (KeyCode::Esc, _) => {
            if state.esc_pending {
                state.should_quit = true;
            } else {
                state.esc_pending = true;
            }
            return; // don't reset esc_pending below
        }
        (KeyCode::Enter, m) if !m.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) => {
            if state.is_input_empty() || state.streaming {
                return;
            }
            let text = state.input_clear();
            if text.starts_with('/') {
                run_slash(&text, state, session, cfg).await;
            } else {
                state.remember_submission(&text);
                state.push_user(text.clone());
                state.streaming = true;
                state.follow_tail = true;
                *agent_stream = Some(session.send(text).await);
            }
        }
        (KeyCode::Backspace, _) => state.input_backspace(),
        (KeyCode::Up, _) => state.history_prev(),
        (KeyCode::Down, _) => state.history_next(),
        (KeyCode::PageUp, _) => {
            state.follow_tail = false;
            state.scroll = state.scroll.saturating_sub(5);
        }
        (KeyCode::PageDown, _) => {
            let new = state.scroll.saturating_add(5);
            if new >= state.transcript_tail {
                state.scroll = state.transcript_tail;
                state.follow_tail = true;
            } else {
                state.scroll = new;
                state.follow_tail = false;
            }
        }
        (KeyCode::Home, _) => {
            state.follow_tail = false;
            state.scroll = 0;
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
    if let Some(pending) = state.pending_approval.take() {
        let _ = pending.request.reply.send(allow);
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
        HarnessEvent::Done => {
            state.streaming = false;
            *agent_stream = None;
        }
    }
}

async fn run_slash(cmd: &str, state: &mut TuiState, session: &Session, cfg: &mut TuiConfig) {
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
                session.set_model(rest.to_owned()).await;
                state.flash = Some(format!("model → {rest}"));
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
