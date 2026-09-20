//! The event loop — one `tokio::select!` across three event sources:
//!
//! 1. Terminal events from `crossterm::event::EventStream` (keys,
//!    mouse, paste, **resize**)
//! 2. Harness events from the stream returned by `Session::send`
//! 3. Approval requests from [`crate::tui::approver::TuiApprover`]
//!
//! Resize is treated as an explicit state transition:
//! `state.handle_resize(w, h)` updates the viewport, invalidates the
//! derived transcript layout when the width changed, and reconciles
//! scroll — *before* the next draw, so nothing ever renders from stale
//! measurements.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use futures::stream::BoxStream;
use futures::StreamExt;
use mira_core::Role;
use mira_harness::{GoalStatus, HarnessEvent, Session};
use ratatui::Terminal;

use crate::tui::components::tool_call::summarize_tool;
use crate::tui::components::status::format_turn_elapsed;
use crate::tui::input::{keyboard, paste};
use crate::tui::render;
use crate::tui::state::{PendingApproval, TuiState};

use super::{run_slash, TuiConfig};

/// How often to redraw while a stream is in flight — drives the
/// elapsed-time counter next to "thinking" so long silent gaps don't
/// look hung.
const STREAM_TICK: Duration = Duration::from_millis(100);

/// Duration a provider-error flash stays visible on the status row.
/// One second lands as "a beat you can't miss" without wasting screen
/// space on a persistent banner — the yellow warning line is still
/// there in the transcript for the details.
const ERROR_FLASH_DURATION: Duration = Duration::from_millis(1000);

pub(super) async fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    session: Session,
    mut cfg: TuiConfig,
    mouse_capture_out: &mut bool,
) -> Result<()> {
    let mut state = TuiState::new(cfg.model.clone(), cfg.mode);
    // Start with mouse capture off so text selection works by default.
    // Alt+M will toggle it on for scroll-wheel driving.
    state.mouse_capture = false;
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
                handle_harness_event(evt, &mut state, &mut agent_stream, &session, &mut cfg).await;
            }
            Some(req) = cfg.approval_rx.recv() => {
                let (friendly, _) = summarize_tool(
                    &req.call.function.name,
                    &req.call.function.arguments,
                );
                terminal_notify(&format!("mira · approval needed: {friendly}"));
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
            // Redraw tick during an error flash so the red bar clears
            // itself once the deadline passes without the user having
            // to press a key. Cheap — fires at most a handful of
            // times per flash.
            _ = tokio::time::sleep(STREAM_TICK), if state.error_flash_active() => {}
        }

        if state.should_quit {
            break;
        }
    }

    *mouse_capture_out = state.mouse_capture;
    Ok(())
}

/// Route one terminal event. Keys and mouse go to the input layer;
/// paste goes to its own router; resize is a state transition here.
async fn handle_terminal_event(
    evt: Event,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    match evt {
        Event::Resize(width, height) => {
            state.handle_resize(width, height);
        }
        Event::Paste(s) => {
            paste::handle_paste(&s, state);
            state.esc_pending = false;
            // Don't clobber the flash — `handle_paste` sets it on a
            // large-paste collapse so the user gets confirmation the
            // 500 lines they just dropped in are safely stashed and
            // will expand on submit.
            keyboard::refresh_palette(state, cfg, file_index).await;
        }
        Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press => {
            keyboard::handle_key(k, state, session, agent_stream, cfg, file_index).await;
        }
        Event::Mouse(m) => keyboard::handle_mouse(m, state),
        _ => {}
    }
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

/// Count uncommitted changes (`git status --porcelain` lines) — the
/// header's dirty chip. `None` outside a repo or on any failure.
async fn count_git_dirty(cwd: &std::path::Path) -> Option<u32> {
    let cwd = cwd.to_owned();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&cwd)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).lines().count() as u32)
    })
    .await
    .ok()
    .flatten()
}

/// Startup-banner tips — one shows per launch, deterministic
/// rotation so the same session picks the same one on resume.
const TIPS: &[&str] = &[
    "type while mira works — enter queues the next turn",
    "ctrl+p rewinds and re-edits the last prompt",
    "ctrl+z undoes the last file write",
    "1-9 expand or collapse recent tool groups",
    "ctrl+r searches the whole transcript",
    "/theme swaps the palette · /theme save keeps it",
];

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
        // One quiet tip per launch — everything else lives in /help.
        let tip = TIPS[std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize % TIPS.len())
            .unwrap_or(0)];
        state.push_welcome(state.model.clone(), state.mode, cwd_short, tip);
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

/// Drop the current harness stream and record a warning entry.
/// Invoked from both Ctrl+C and (single-press) Esc during a turn, so
/// the two shortcuts stay in sync — no risk of one leaving the state
/// half-torn-down.
pub(crate) fn interrupt_stream(
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
) {
    if agent_stream.is_none() {
        return;
    }
    let msg = match state.in_flight_tool() {
        Some((name, args)) => {
            let short = truncate_for_warning(&args, 80);
            format!("interrupted while `{name}({short})` was running — tool result discarded")
        }
        None => "interrupted".to_owned(),
    };
    *agent_stream = None;
    state.streaming = false;
    state.stream_started_at = None;
    state.clear_tool_tail();
    state.push_warning(msg);
}

async fn handle_harness_event(
    evt: HarnessEvent,
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    session: &Session,
    cfg: &mut TuiConfig,
) {
    match evt {
        HarnessEvent::Token(t) => state.append_token(&t),
        HarnessEvent::ToolStart(call) => {
            state.push_tool_call(&call);
            // Compute the diff preview eagerly, not just on
            // approval — otherwise session-allow'd `edit_file` /
            // `write_file` calls render as a bare "edited /path (1
            // replacement)" line with no coloured diff, which is the
            // whole point of showing the change inline. `compute_preview`
            // returns `None` for tools that don't produce diffs, so
            // this is a no-op for `read_file`, `bash`, etc.
            if let Some(p) = mira_tools::compute_preview(&cfg.cwd, &call).await {
                state.attach_preview(&call.id.to_string(), p);
            }
        }
        HarnessEvent::ToolEnd(result) => {
            state.clear_tool_tail();
            // Task tools return the authoritative item(s) in `data` —
            // mirror them into the TUI's task panel before the result
            // lands in the transcript.
            state.apply_task_payload(&result);
            state.push_tool_result(&result);
        }
        HarnessEvent::Warning(w) => {
            // Auth / rate-limit warnings are silent-killers if they
            // scroll past as a yellow line — mira looks "hung" while
            // the user misses the 401. Detect them here and paint the
            // status row red for a beat so it's impossible to ignore.
            if let Some(label) = classify_provider_error(&w) {
                state.error_flash(label, ERROR_FLASH_DURATION);
            }
            state.push_warning(w);
        }
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
        HarnessEvent::ToolProgress { call_id, line } => {
            // Keep the newest line as the live tail under the `◐`
            // header — one dim row that tells "hung" from "working"
            // without burying the transcript in shell noise. Full
            // content still lands via `ToolEnd` and is one Ctrl+E away.
            state.set_tool_tail(&call_id, &line);
        }
        HarnessEvent::ToolPreview { .. } => {
            // The TUI already renders diffs via its own approval flow;
            // skip the harness-side preview to avoid double rendering.
        }
        HarnessEvent::Done => {
            // Drop a `✳ Baked for 12.4s` marker into the transcript so
            // the reply is visibly bounded — matches Claude Code's
            // full-stop treatment. Cap at u32 max in case a stream
            // ran for days (broken provider); no reason to widen.
            let mut elapsed_ms = 0;
            if let Some(t0) = state.stream_started_at {
                elapsed_ms = t0.elapsed().as_millis().min(u32::MAX as u128) as u32;
                // Turn receipt inputs: this turn's usage delta priced
                // at the current model, if it's priced at all.
                let usage = &state.usage;
                let base = &state.turn_usage_baseline;
                let cost = mira_ai::cost_usd(
                    &state.model,
                    mira_ai::TokenUsage {
                        prompt_tokens: usage.prompt_tokens.saturating_sub(base.prompt_tokens).min(u32::MAX as u64) as u32,
                        completion_tokens: usage.completion_tokens.saturating_sub(base.completion_tokens).min(u32::MAX as u64) as u32,
                        cached_input_tokens: usage.cached_input_tokens.saturating_sub(base.cached_input_tokens).min(u32::MAX as u64) as u32,
                    },
                );
                state.push_turn_end(
                    elapsed_ms,
                    usage.completion_tokens.saturating_sub(base.completion_tokens),
                    cost,
                );
            }
            state.streaming = false;
            state.stream_started_at = None;
            *agent_stream = None;
            // Turn finished is finished — notify even when a queued
            // message starts the next turn immediately, or a user who
            // always types ahead would never hear a completion.
            terminal_notify(&format!(
                "mira · turn finished ({})",
                format_turn_elapsed(elapsed_ms)
            ));
            // The agent just (maybe) touched the worktree — refresh the
            // header's branch + dirty chip so it never goes stale
            // exactly when it matters.
            state.git_branch = detect_git_branch(&cfg.cwd).await;
            state.git_dirty = count_git_dirty(&cfg.cwd).await;
            // The user may have typed while the agent was working —
            // tap the queue FIFO so the next turn starts immediately.
            if let Some(next) = state.take_next_queued() {
                submit_composed(state, session, agent_stream, cfg, next).await;
            }
        }
    }
}

/// Route a fully-composed message: expand pastes, reset the palette,
/// dispatch slash commands locally, and open the harness stream for
/// anything that reaches the model. Shared by composer Enter, queued
/// auto-send on `Done`, and slash-triggered skill invocations so all
/// three paths hit identical wiring.
pub(crate) async fn submit_composed(
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    raw: String,
) {
    let text = state.expand_pastes(&raw);
    state.pastes.clear();
    state.palette = crate::tui::state::PaletteState::none();
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

/// Push a user message and open the harness stream — factored so the
/// composer-enter path and slash-triggered skill invocations both hit
/// the same wiring (`remember_submission`, follow_tail, stream_started_at).
///
/// If the session's cost has already exceeded `state.budget_usd`, this
/// refuses to send and pushes a warning instead. The user's cap → the
/// user's call: they either raise it (`/budget $X`) or clear it
/// (`/budget off`) before the next turn goes out.
pub(crate) async fn start_stream(
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
                state.input_replace(&text);
                return;
            }
        }
    }
    state.remember_submission(&text);
    state.push_user(text.clone());
    state.streaming = true;
    // Snapshot the running session totals so the in-turn indicator can
    // show this turn's delta, not the whole session's total.
    state.turn_usage_baseline = state.usage;
    state.stream_started_at = Some(std::time::Instant::now());
    state.follow_tail = true;
    *agent_stream = Some(session.send(text).await);
}

/// Current session cost in USD, if the model is priced and the provider
/// has reported at least one usage round. Mirrors the formula used in
/// the header usage chip so both surfaces agree.
pub(crate) fn current_cost_usd(state: &TuiState) -> Option<f64> {
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
/// large ones compact (`$12.4`).
pub(crate) fn format_dollars_short(d: f64) -> String {
    if d >= 1.0 {
        format!("${d:.2}")
    } else {
        format!("${d:.3}")
    }
}

/// Ping the user through the terminal: an OSC 9 desktop notification
/// (iTerm2, kitty, WezTerm, foot) followed by a bare BEL so plain
/// terminals and tmux still ring/flash. Fired when a turn finishes or
/// an approval is pending — the two moments worth leaving the window
/// for. Opt out with `MIRA_NO_NOTIFY=1`.
fn terminal_notify(summary: &str) {
    if std::env::var_os("MIRA_NO_NOTIFY").is_some() {
        return;
    }
    // OSC payloads must stay on one line and can't contain control
    // bytes or `;` (the field separator for both OSC 9 text and 777).
    let safe: String = summary
        .chars()
        .filter(|c| !c.is_control() && *c != ';')
        .collect();

    // Emit every mainstream notification protocol; terminals ignore
    // the ones they don't know:
    //   OSC 9   — iTerm2 / Ghostty / WezTerm / foot desktop notification
    //   OSC 777 — notify;title;body (rxvt-unicode, a few others)
    //   BEL     — the audible fallback everywhere else
    let osc9 = format!("\x1b]9;{safe}\x07");
    let osc777 = format!("\x1b]777;notify;mira;{safe}\x07");
    let mut seq = String::new();
    if std::env::var_os("TMUX").is_some() {
        // tmux swallows unknown OSC from panes unless wrapped in its
        // DCS passthrough — with every inner ESC doubled.
        for osc in [&osc9, &osc777] {
            seq.push_str(&osc.replace('\x1b', "\x1b\x1b"));
        }
        seq.push_str("\x1b\\");
    } else {
        seq.push_str(&osc9);
        seq.push_str(&osc777);
    }
    // The bare bell sits OUTSIDE the passthrough so tmux's own bell
    // handling forwards it (audible-bell settings still apply).
    seq.push('\x07');

    let mut out = std::io::stdout();
    let _ = std::io::Write::write_all(&mut out, seq.as_bytes());
    let _ = std::io::Write::flush(&mut out);
}

/// Pull a short label out of a provider-error warning when the
/// warning names an auth or rate-limit failure. Returns `None` for
/// anything else — we don't want to red-flash on every warning, only
/// the ones that mean the model won't respond until the user acts.
fn classify_provider_error(msg: &str) -> Option<String> {
    let m = msg.to_ascii_lowercase();
    if m.contains(" 401") || m.contains("unauthorized") || m.contains("invalid api key") {
        return Some("provider 401 — check API key".to_owned());
    }
    if m.contains(" 429")
        || m.contains("rate limit")
        || m.contains("rate_limited")
        || m.contains("too many requests")
    {
        return Some("provider 429 — rate limited".to_owned());
    }
    if m.contains(" 403") || m.contains("forbidden") {
        return Some("provider 403 — access denied".to_owned());
    }
    None
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
