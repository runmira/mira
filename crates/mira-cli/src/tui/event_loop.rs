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
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::tui::components::status::format_turn_elapsed;
use crate::tui::components::tool_call::summarize_tool;
use crate::tui::inline_term::InlineTerm;
use crate::tui::input::{keyboard, paste};
use crate::tui::render;
use crate::tui::state::{LogEntry, PendingApproval, TuiState};

use super::{run_slash, TuiConfig};

/// How often to redraw while a stream is in flight — drives the
/// elapsed-time counter next to "thinking" so long silent gaps don't
/// look hung.
const STREAM_TICK: Duration = Duration::from_millis(100);

/// Minimum wall-clock gap between two consecutive draws. Frame requests
/// are debounced to this — a burst of harness Token events at 50/sec
/// won't drive 50 redraws/sec, they'll coalesce into ~60fps at most.
/// Matches the deadline scheduler pattern the aster reference uses.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Duration a provider-error flash stays visible on the status row.
/// One second lands as "a beat you can't miss" without wasting screen
/// space on a persistent banner — the yellow warning line is still
/// there in the transcript for the details.
const ERROR_FLASH_DURATION: Duration = Duration::from_millis(1000);

pub(super) async fn event_loop(
    term: &mut InlineTerm,
    session: Session,
    mut cfg: TuiConfig,
    mouse_capture_out: &mut bool,
    viewport_height_out: &mut u16,
    summary_out: &mut Option<String>,
) -> Result<()> {
    let mut state = TuiState::new(cfg.model.clone(), cfg.mode);
    let session_started_at = std::time::Instant::now();
    // Start with mouse capture off so text selection works by default.
    // Alt+M will toggle it on for scroll-wheel driving.
    state.mouse_capture = false;
    // Paint immediately — history hydration below can take seconds
    // (session store I/O) and startup must never look hung.
    draw_frame(term, &mut state)?;
    state.git_branch = detect_git_branch(&cfg.cwd).await;
    hydrate_from_history(&session, &mut state, &cfg).await;
    for notice in cfg.extensions.notices() {
        state.push_warning(notice);
    }

    // File index for `@` completion — a one-shot `rg --files` in the cwd,
    // shared between palette openings so we don't reshell for every '@'.
    // Computed lazily on first use.
    let mut file_index: Option<Vec<String>> = None;

    let mut input_events = EventStream::new();
    let mut agent_stream: Option<BoxStream<'static, HarnessEvent>> = None;
    // Streaming markdown parser — assistant tokens feed this and every
    // completed line is pushed into real terminal scrollback so long
    // replies never clip against the small inline pane.
    let mut md_stream = crate::tui::markdown::MarkdownStream::new();
    // Whether the `●` assistant dot has been emitted for the reply
    // currently streaming. Reset on `Done`; guarded so a burst of
    // deltas doesn't emit the dot more than once per reply.
    let mut md_dot_emitted = false;

    // Coalesced frame scheduling: rather than drawing on every event,
    // we hold a `next_frame` deadline and let the tokio select's sleep
    // arm trigger the actual draw when it fires. Every event handler
    // just calls `schedule_frame` — bursts of harness Token events
    // collapse into a single ~16ms-later draw instead of 50/sec.
    let start = std::time::Instant::now();
    let mut last_draw = start - MIN_FRAME_INTERVAL;
    let mut next_frame: Option<std::time::Instant> = Some(start);

    loop {
        // Settle newly-arrived entries into real terminal scrollback
        // eagerly, on every iteration. `insert_history` isn't gated by
        // the frame scheduler — visibility of what the user just typed
        // shouldn't wait for the animation loop. Emit is a no-op when
        // nothing new has arrived, so this is cheap.
        let _ = emit_settled(term, &mut state);

        // Draws are debounced. Fire only when the deadline has arrived.
        if next_frame
            .map(|dl| std::time::Instant::now() >= dl)
            .unwrap_or(false)
        {
            draw_frame(term, &mut state)?;
            last_draw = std::time::Instant::now();
            next_frame = None;
        }

        let deadline = next_frame;
        tokio::select! {
            evt = input_events.next() => {
                match evt {
                    Some(Ok(e)) => {
                        handle_terminal_event(e, term, &mut state, &session, &mut agent_stream, &mut cfg, &mut file_index).await;
                        schedule_frame(&mut next_frame, last_draw);
                    }
                    Some(Err(_)) | None => break,
                }
            }
            Some(evt) = next_agent_event(&mut agent_stream) => {
                handle_harness_event(evt, term, &mut state, &mut agent_stream, &session, &mut cfg, &mut md_stream, &mut md_dot_emitted).await;
                schedule_frame(&mut next_frame, last_draw);
            }
            Some(req) = cfg.approval_rx.recv() => {
                let (friendly, _) = summarize_tool(
                    &req.call.function.name,
                    &req.call.function.arguments,
                );
                terminal_notify(&format!("mira · approval needed: {friendly}"));
                let preview = mira_tools::compute_preview(&cfg.cwd, &req.call).await;
                if let Some(p) = preview.as_ref() {
                    state.attach_preview(&req.call.id.to_string(), p.clone());
                }
                state.push_approval(PendingApproval { request: req, preview });
                state.follow_tail = true;
                schedule_frame(&mut next_frame, last_draw);
            }
            Some(prompt) = cfg.prompt_rx.recv() => {
                match prompt.request {
                    mira_tools::prompt::PromptRequest::Plan(proposal) => {
                        terminal_notify("mira · plan review needed");
                        state.pending_plan =
                            Some(crate::tui::state::PendingPlan::new(prompt.prompt_id, proposal, prompt.reply));
                    }
                    mira_tools::prompt::PromptRequest::AskUser(proposal) => {
                        terminal_notify("mira · mira has a question");
                        state.pending_ask =
                            Some(crate::tui::state::PendingAsk::new(prompt.prompt_id, proposal, prompt.reply));
                    }
                }
                state.follow_tail = true;
                schedule_frame(&mut next_frame, last_draw);
            }
            Some(update) = cfg.env_rx.recv() => {
                match update {
                    super::EnvUpdate::Info(m) => state.push_info(m),
                    super::EnvUpdate::Warning(m) => state.push_warning(m),
                }
                state.follow_tail = true;
                schedule_frame(&mut next_frame, last_draw);
            }
            Ok(msg) = cfg.subagent_events_rx.recv() => {
                handle_subagent_event(msg, &mut state);
                schedule_frame(&mut next_frame, last_draw);
            }
            _ = sleep_until(deadline), if deadline.is_some() => {
                // Deadline fired — next loop iteration will draw. No
                // additional bookkeeping needed here; the top-of-loop
                // check handles it.
            }
            // Heartbeat: while a turn is streaming, request a frame every
            // STREAM_TICK so the "3.2s" elapsed counter advances even
            // during silent provider pauses. Same for error-flash decay.
            _ = tokio::time::sleep(STREAM_TICK), if state.streaming || state.error_flash_active() => {
                schedule_frame(&mut next_frame, last_draw);
            }
        }

        if state.should_quit {
            break;
        }
    }

    // Final emission: everything (drop the streaming cursor) so
    // scrollback ends with the complete transcript, then report the
    // viewport height for the exit cleanup in `leave`.
    state.streaming = false;
    let end = state.entries().len();
    let _ = emit_upto(term, &mut state, end);
    *viewport_height_out = state.viewport_height.max(1);
    *mouse_capture_out = state.mouse_capture;
    *summary_out = Some(build_exit_summary(&state, session_started_at.elapsed()));
    Ok(())
}

/// One-line session recap printed to stdout after the TUI drops.
/// Survives in shell scrollback so the user can glance back at what
/// the session cost, how long it ran, and whether the goal landed.
///
/// Silent (returns "") for a session that never sent a turn — nothing
/// to report.
fn build_exit_summary(state: &TuiState, elapsed: Duration) -> String {
    let tool_calls = state
        .entries()
        .iter()
        .filter(|e| matches!(e, LogEntry::ToolCall { .. }))
        .count();
    let turns = state
        .entries()
        .iter()
        .filter(|e| matches!(e, LogEntry::TurnEnd { .. }))
        .count();
    if turns == 0 && tool_calls == 0 {
        return String::new();
    }

    let dur = format_session_duration(elapsed);
    let mut parts: Vec<String> = vec![
        format!("mira · {}", state.model),
        dur,
        format!("{turns} turn{}", if turns == 1 { "" } else { "s" }),
        format!("{tool_calls} tool{}", if tool_calls == 1 { "" } else { "s" }),
    ];

    let u = &state.usage;
    if !u.is_zero() {
        parts.push(format!(
            "↑{} ↓{}",
            crate::tui::components::status::short_num(u.prompt_tokens),
            crate::tui::components::status::short_num(u.completion_tokens),
        ));
    }
    if let Some(cost) = current_cost_usd(state) {
        parts.push(format_dollars_short(cost));
    }
    if let Some(goal) = state.goal.as_ref() {
        let word = match goal.status {
            GoalStatus::Met => "goal met",
            GoalStatus::Impossible => "goal impossible",
            GoalStatus::NeedsUser => "goal needs input",
            GoalStatus::Exhausted => "goal exhausted",
            GoalStatus::Cleared => "goal cleared",
            GoalStatus::Active => "goal active",
        };
        parts.push(word.to_owned());
    }
    parts.join(" · ")
}

/// Human-readable session duration for the exit summary. Round to the
/// nearest second under a minute, then minutes+seconds, then hours+minutes.
fn format_session_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Minimum inline viewport height — enough for the footer plus a
/// one-line composer body without hiding the composer chrome.
const MIN_INLINE_HEIGHT: u16 = 4;

/// Merge a new frame-request deadline into `next` — earliest wins, and
/// the earliest is floored at `last_draw + MIN_FRAME_INTERVAL` so a
/// burst of events can't drive the draw rate over ~60fps.
fn schedule_frame(
    next: &mut Option<std::time::Instant>,
    last_draw: std::time::Instant,
) {
    let now = std::time::Instant::now();
    let earliest = last_draw + MIN_FRAME_INTERVAL;
    let dl = now.max(earliest);
    *next = Some(next.map_or(dl, |cur| cur.min(dl)));
}

/// tokio `sleep_until` that treats `None` as "never" — pairs with a
/// `select!` `if deadline.is_some()` guard so a missing deadline is
/// simply a disabled branch.
async fn sleep_until(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}

/// Resize the inline viewport to fit `desired_viewport_height`, then
/// render one frame into it. Wrapping these two into a single call is
/// what gives the zero-gap look: no spacer row between the last settled
/// entry (in real scrollback) and the composer, because the viewport is
/// exactly as tall as the live content it holds.
///
/// `InlineTerm::set_height` moves the viewport boundary using its own
/// tracked state — no `get_cursor_position` call — so this is safe to
/// run alongside `EventStream`.
fn draw_frame(term: &mut InlineTerm, state: &mut TuiState) -> Result<()> {
    // Size the pane BEFORE handing the buffer to `render::draw`.
    // Overlays (palette, search) write at absolute Y coordinates inside
    // the pane's buffer, so if the pane is still at its old (small)
    // size when the closure runs, the overlay writes past the buffer's
    // bottom edge and ratatui panics with `index outside of buffer`.
    let width = term.width().max(1);
    let target = render::desired_pane_height(state, width).max(MIN_INLINE_HEIGHT);
    term.set_height(target)?;
    term.draw(|f| render::draw(f, state))?;
    Ok(())
}

/// Mirror newly-settled entries into real scrollback, stopping at the
/// emission frontier so the currently-streaming assistant tail stays
/// live in the viewport (its content is being pushed line-by-line by
/// `MarkdownStream`, not via this path). See [`TuiState::emission_frontier`].
///
/// The user message pushed at turn start emits immediately — before any
/// tokens arrive — because `emission_frontier` only holds back the
/// streaming assistant entry, not the "last entry" as a category.
fn emit_settled(term: &mut InlineTerm, state: &mut TuiState) -> Result<()> {
    emit_upto(term, state, state.emission_frontier())
}

/// Mirror entries in `[emitted_entries, end)` into the terminal's real
/// scrollback via `InlineTerm::insert_history`, then advance the
/// frontier.
fn emit_upto(term: &mut InlineTerm, state: &mut TuiState, end: usize) -> Result<()> {
    let end = end.min(state.entries().len());
    if state.emitted_entries >= end {
        return Ok(());
    }
    let width = term.width().max(1);
    let lines = render::transcript::settled_lines(state, width, end);
    if lines.is_empty() {
        state.emitted_entries = end;
        return Ok(());
    }
    render_lines_to_scrollback(term, lines, width)?;
    state.emitted_entries = end;
    Ok(())
}

/// Push a batch of already-rendered lines into real terminal scrollback
/// as one contiguous history block. Used by the streaming markdown path
/// (assistant tokens) where the caller owns the styling and doesn't
/// need to go through the transcript block pipeline.
fn push_lines_to_scrollback(term: &mut InlineTerm, lines: Vec<Line<'static>>) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let width = term.width().max(1);
    render_lines_to_scrollback(term, lines, width)
}

/// Wrap `lines` to `width`, size a buffer to the wrapped height, then
/// hand it to `InlineTerm::insert_history`. Consolidated helper so the
/// two callers (settled entries + streaming markdown) can't drift on
/// wrapping policy — a long assistant sentence that renders as one
/// visual row here is the same as a long tool-result snippet coming
/// through the block pipeline.
fn render_lines_to_scrollback(
    term: &mut InlineTerm,
    lines: Vec<Line<'static>>,
    width: u16,
) -> Result<()> {
    // Compute the post-wrap row count ourselves from the input Lines'
    // span widths. `Paragraph::line_count` (behind
    // `unstable-rendered-line-info`) under-reports for Vec<Line> input
    // — a 200-col line on an 80-col terminal returns 1 rather than 3,
    // so the buffer gets sized to a single row and the tail clips at
    // the right edge instead of wrapping into scrollback. Direct
    // display-width math sidesteps the bug and matches the wrapper
    // ratatui will actually apply (space-boundary greedy).
    let height = wrapped_row_count(&lines, width).max(1);
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    para.render(area, &mut buf);
    term.insert_history(&buf)?;
    Ok(())
}

/// Row count `lines` would occupy after wrapping to `width` columns.
/// Uses the same greedy-by-column policy `Wrap { trim: false }` does —
/// blank lines still cost one row so a `Line::from("")` separator
/// doesn't collapse.
fn wrapped_row_count(lines: &[Line<'_>], width: u16) -> u16 {
    use unicode_width::UnicodeWidthStr;
    let w = width.max(1) as usize;
    let mut total: u32 = 0;
    for line in lines {
        let display: usize = line
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        let rows = if display == 0 {
            1
        } else {
            (display + w - 1) / w
        };
        total = total.saturating_add(rows as u32);
    }
    total.min(u16::MAX as u32) as u16
}

/// Route one terminal event. Keys and mouse go to the input layer;
/// paste goes to its own router; resize is a state transition here.
async fn handle_terminal_event(
    evt: Event,
    term: &mut InlineTerm,
    state: &mut TuiState,
    session: &Session,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    cfg: &mut TuiConfig,
    file_index: &mut Option<Vec<String>>,
) {
    match evt {
        Event::Resize(width, height) => {
            // Reflow the viewport under the new terminal dimensions
            // before the next draw. `InlineTerm::resized` computes the
            // new position from tracked transcript rows — no cursor
            // query, no race with the event stream.
            let _ = term.resized();
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

/// Startup-banner tips — one shows per launch, deterministic
/// rotation so the same session picks the same one on resume.
const TIPS: &[&str] = &[
    "type while mira works — enter queues the next turn",
    "ctrl+p rewinds and re-edits the last prompt",
    "ctrl+z undoes the last file write",
    "1-9 expand or collapse recent tool groups",
    "ctrl+r searches the whole transcript",
    "/theme swaps the palette · /theme save keeps it",
    "scrollback is real — finished output selects and copies natively",
    "the footer names the keys that work right now — glance down when stuck",
    "/mode plan · /mode auto · tab completes every slash arg",
    "@ mentions a file without typing the whole path",
    "/budget caps session spend · /cost shows where it went",
    "shift+tab cycles permission modes without opening /mode",
    "ctrl+e expands the last tool result in place",
    "ctrl+y copies the last reply · ctrl+shift+c copies the transcript",
    "/sessions lists recent work · /resume prints the command to reopen one",
    "esc interrupts a turn · esc esc quits — the transcript stays behind",
];

/// Rebuild the visible transcript from a session's history. On a fresh
/// session (only a system message) we just show the welcome line; on
/// resume we replay user/assistant/tool entries so it's obvious the
/// conversation continued rather than starting empty.
async fn hydrate_from_history(session: &Session, state: &mut TuiState, cfg: &TuiConfig) {
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
        // Skill roster snapshot for the banner — sorted names; the
        // component caps the display with `+N more`.
        let mut skills: Vec<String> = cfg
            .skills
            .read()
            .await
            .skills
            .values()
            .map(|s| s.name.clone())
            .collect();
        skills.sort();
        state.push_welcome(
            state.model.clone(),
            state.mode,
            cwd_short,
            crate::tui::state::SessionMeta {
                provider: cfg.provider.clone(),
                branch: state.git_branch.clone(),
                skills,
            },
            tip,
        );
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
///
/// Returns the next queued prompt (if any) so the caller can start
/// the follow-up turn immediately. Without this the FIFO stalls
/// because `HarnessEvent::Done` only fires on natural completion.
pub(crate) fn interrupt_stream(
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
) -> Option<String> {
    if agent_stream.is_none() {
        return None;
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
    // Advance the FIFO so the next queued message doesn't wait
    // forever for a Done event that never arrives.
    state.take_next_queued()
}

async fn handle_harness_event(
    evt: HarnessEvent,
    term: &mut InlineTerm,
    state: &mut TuiState,
    agent_stream: &mut Option<BoxStream<'static, HarnessEvent>>,
    session: &Session,
    cfg: &mut TuiConfig,
    md_stream: &mut crate::tui::markdown::MarkdownStream,
    md_dot_emitted: &mut bool,
) {
    match evt {
        HarnessEvent::Token(t) => {
            state.append_token(&t);
            // Push every line the parser can now finalise straight into
            // real terminal scrollback. The assistant's log entry still
            // grows for search / turn-nav / history, but we mark its
            // index in `streamed_assistant_idx` so `emit_settled` skips
            // it on the next tick — otherwise the same reply would
            // print twice (once here, once through the block pipeline).
            let lines = md_stream.push(&t);
            if !lines.is_empty() {
                let out = if !*md_dot_emitted {
                    // Prefix the very first line of the reply with the
                    // inline `● ` marker — one row, not two, matching
                    // the batch-render assistant dot.
                    *md_dot_emitted = true;
                    crate::tui::components::message::prefix_assistant_dot(lines)
                } else {
                    lines
                };
                let _ = push_lines_to_scrollback(term, out);
            }
            if let Some(idx) = state
                .entries()
                .iter()
                .rposition(|e| matches!(e, LogEntry::Assistant(_)))
            {
                state.streamed_assistant_idx.insert(idx);
            }
        }
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
        HarnessEvent::Usage { totals, .. } => {
            state.usage = totals;
            // Arm the pre-compaction warning card once fill crosses 80 %.
            // Cleared when compaction fires (push_compacted) so we don't
            // keep nagging after the harness has already tidied up.
            if !state.context_warn_shown {
                if let Some(pct) = state.context_fill_pct() {
                    if pct >= 80.0 {
                        state.context_warn_shown = true;
                    }
                }
            }
        }
        HarnessEvent::MemoryLearned { count } => {
            state.push_warning(format!(
                "[memory] remembered {count} thing{}",
                if count == 1 { "" } else { "s" }
            ));
        }
        HarnessEvent::Compacted { messages_removed } => {
            state.push_compacted(messages_removed);
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
            // Flush any partial trailing line + open table / unclosed
            // fence in the streaming parser so the reply ends cleanly
            // in scrollback before the turn receipt lands.
            let tail_lines = md_stream.flush();
            if !tail_lines.is_empty() {
                let out = if !*md_dot_emitted {
                    // Edge case: a reply that ends without producing a
                    // single completed line (no `\n`) still deserves
                    // the inline `● ` marker so it isn't a naked run
                    // of text.
                    crate::tui::components::message::prefix_assistant_dot(tail_lines)
                } else {
                    tail_lines
                };
                let _ = push_lines_to_scrollback(term, out);
            }
            // Reset the dot latch so the next reply gets its own header.
            *md_dot_emitted = false;
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
                        prompt_tokens: usage
                            .prompt_tokens
                            .saturating_sub(base.prompt_tokens)
                            .min(u32::MAX as u64) as u32,
                        completion_tokens: usage
                            .completion_tokens
                            .saturating_sub(base.completion_tokens)
                            .min(u32::MAX as u64) as u32,
                        cached_input_tokens: usage
                            .cached_input_tokens
                            .saturating_sub(base.cached_input_tokens)
                            .min(u32::MAX as u64)
                            as u32,
                    },
                );
                state.push_turn_end(
                    elapsed_ms,
                    usage
                        .completion_tokens
                        .saturating_sub(base.completion_tokens),
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
            // branch snapshot so a later session starts accurate. The
            // banner shows the boot-time branch; live dirty tracking
            // left with the old header.
            state.git_branch = detect_git_branch(&cfg.cwd).await;
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
    } else if cfg.environments.is_switching() {
        // Tools would run against a half-moved tree. Hand the message back.
        state.push_warning("switching environments — send again when it's done".into());
        state.restore_input(text);
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
    state.turn_subagent_chars = 0;
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

/// Route one subagent broadcast event into the TUI state. The events
/// arrive on `cfg.subagent_events_rx` and update `state.agent_cells`
/// so the `● Agent(...)` tool-call renderer can show nested progress.
fn handle_subagent_event(
    msg: mira_server::protocol::ServerMsg,
    state: &mut TuiState,
) {
    use mira_server::protocol::ServerMsg;
    match msg {
        ServerMsg::SubagentStarted {
            parent_call_id,
            agent_id,
            model,
            ..
        } => {
            state.agent_started(&parent_call_id, agent_id, model);
        }
        ServerMsg::SubagentToolStart {
            parent_call_id,
            call,
        } => {
            // Each tool invocation = some model overhead: bump the char
            // counter so the working-indicator token count advances during
            // bash-heavy subagent runs, not just during text generation.
            state.turn_subagent_chars = state.turn_subagent_chars.saturating_add(200);
            // Model has stopped generating text and is now executing a
            // tool — clear the streaming tail so stale "thinking" text
            // doesn't stay frozen under the header while bash runs.
            state.agent_clear_streaming_text(&parent_call_id);
            let (label, summary) =
                summarize_tool(&call.function.name, &call.function.arguments);
            state.agent_tool_started(
                &parent_call_id,
                call.id.to_string(),
                label,
                summary,
            );
        }
        ServerMsg::SubagentToolEnd {
            parent_call_id,
            result,
        } => {
            state.turn_subagent_chars = state.turn_subagent_chars.saturating_add(100);
            state.agent_tool_ended(&parent_call_id, &result.call_id.to_string(), !result.is_error);
        }
        ServerMsg::SubagentWarning { parent_call_id, .. } => {
            state.agent_warning(&parent_call_id);
        }
        ServerMsg::SubagentProgress {
            parent_call_id,
            text,
        } => {
            // Tool output lines are real agent activity — count them even
            // though they aren't model tokens. Cap per line so a huge bash
            // dump doesn't balloon the display counter.
            state.turn_subagent_chars =
                state.turn_subagent_chars.saturating_add(text.len().min(80) as u64);
            state.agent_progress(&parent_call_id, text);
        }
        ServerMsg::SubagentDone { parent_call_id } => {
            state.agent_done(&parent_call_id);
        }
        // Count subagent characters as a token proxy — 4 chars ≈ 1 token
        // (standard heuristic). Also feeds the live streaming tail so
        // the agent header shows what the subagent is currently thinking.
        ServerMsg::SubagentToken { parent_call_id, text } => {
            state.turn_subagent_chars =
                state.turn_subagent_chars.saturating_add(text.len() as u64);
            state.agent_token(&parent_call_id, &text);
        }
        // Scratchpad note, review request, and all non-subagent variants
        // are intentionally ignored — the TUI doesn't need them.
        _ => {}
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
