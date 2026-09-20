use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use mira_harness::GoalStatus;
use mira_policy::Mode;
use mira_tools::{DiffKind, DiffLine, DiffPreview};

use crate::tui::markdown;
use crate::tui::state::{LogEntry, Palette, TuiState};

// ---- Palette ----
//
// Backed by `crate::tui::theme` — an `RwLock`-guarded global that
// `/theme <name>`, `/theme reload`, and `/theme save` mutate at
// runtime. Every callsite goes through one of the shim fns below;
// each is a single read-lock acquisition, cheap enough for the 100ms
// render tick.
//
// Truecolor-only — 16-color terminals fall back to their closest
// match automatically via ratatui/crossterm.

use crate::tui::theme;

/// Names stay uppercase to signal "palette constant" at every callsite
/// even though Rust convention normally wants snake_case for functions.
#[allow(non_snake_case)]
#[inline]
fn SALMON() -> Color {
    theme::current().salmon
}
#[allow(non_snake_case)]
#[inline]
fn CREAM() -> Color {
    theme::current().cream
}
#[allow(non_snake_case)]
#[inline]
fn MUTED() -> Color {
    theme::current().muted
}
#[allow(non_snake_case)]
#[inline]
fn DIM() -> Color {
    theme::current().dim
}
#[allow(non_snake_case)]
#[inline]
fn HAIRLINE() -> Color {
    theme::current().hairline
}

/// Mira's brand mark — the script-M is the closest Unicode analogue
/// of the flowing wave/M in the vector logo. Rendered wherever the
/// UI needs an app-identity glyph (header, welcome, streaming line).
const LOGO: &str = "ℳ";

/// Pulse the brand mark's color along a sine wave so the streaming
/// indicator "breathes". Elapsed-time driven — the render loop
/// already re-draws every 100ms while streaming, so this animates
/// for free.
fn pulsed_logo(secs: f32) -> Color {
    // Sine, 1.4s period, scaled into [0.55 .. 1.0] intensity — never
    // drops to invisible, but reads as a heartbeat.
    let phase = (secs * std::f32::consts::TAU / 1.4).sin() * 0.5 + 0.5;
    let scale = 0.55 + 0.45 * phase;
    Color::Rgb(
        (232.0 * scale) as u8,
        (156.0 * scale) as u8,
        (104.0 * scale) as u8,
    )
}

pub fn draw(f: &mut Frame, state: &mut TuiState) {
    // Input area grows with content up to 8 rows so a multi-line message
    // is visible while composing, then shrinks back after submit.
    // (#3, #4) Composer now uses a *top-only* coral hairline instead of
    // a full box border — 1 border row + N body rows, not 2 borders + N.
    let input_rows = input_display_rows(state.input()).min(6);
    let input_height = input_rows + 1;

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(1), // (#6) breathing space between header and transcript
            Constraint::Min(1),    // transcript
            Constraint::Length(input_height),
            Constraint::Length(1), // single unified footer (#3)
        ])
        .split(f.area());

    header(f, root[0], state);
    // root[1] is a deliberate blank row — matches the mockup's air
    // between the header rule and the first `> user` prompt.
    transcript(f, root[2], state);
    input(f, root[3], state);
    status(f, root[4], state);

    // Overlays: palette above the input, approval modal centered.
    if state.palette.kind != Palette::None && !state.palette.matches.is_empty() {
        palette(f, root[3], state);
    }
    if state.search.is_some() {
        search_overlay(f, root[3], state);
    }
    // Inline approval renders as a transcript block below the tool
    // call that triggered it — no centered modal.
}

fn input_display_rows(input: &str) -> u16 {
    let count = input.split('\n').count().max(1) as u16;
    count.max(1)
}

fn header(f: &mut Frame, area: Rect, state: &TuiState) {
    let mut spans = vec![
        Span::styled(
            format!("{LOGO} mira "),
            Style::default().fg(SALMON()).bold(),
        ),
        Span::styled("· ", Style::default().fg(DIM())),
        Span::styled(state.model.as_str(), Style::default().fg(CREAM())),
        Span::styled(" · ", Style::default().fg(DIM())),
        Span::styled(state.mode.as_str(), mode_style(state)),
    ];
    if let Some(branch) = state.git_branch.as_deref() {
        spans.push(Span::styled(" · ", Style::default().fg(DIM())));
        spans.push(Span::styled(
            format!("⎇ {branch}"),
            Style::default().fg(MUTED()),
        ));
    }
    if let Some(g) = state.goal.as_ref() {
        let (label, chip_style) = goal_chip(g.status);
        spans.push(Span::styled(" · ", Style::default().fg(DIM())));
        spans.push(Span::styled("goal ", chip_style));
        spans.push(Span::styled(
            format!("{}/{}", g.iterations, g.max_iterations),
            Style::default().fg(MUTED()),
        ));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(label, chip_style));
        // Trim the condition inline so it doesn't blow past the header.
        spans.push(Span::styled(" · ", Style::default().fg(DIM())));
        spans.push(Span::styled(
            truncate(&g.condition, 60),
            Style::default().fg(CREAM()),
        ));
    }
    let left = Line::from(spans);

    // Usage chip on the right side of the header, so the bottom status
    // row can stay purely a hint/flash area. Rendered as styled spans
    // (not a plain string) so the dollar figure can flip red past
    // `state.budget_usd` without repainting the whole chip.
    let usage_spans = format_usage_spans(state);
    if usage_spans.is_empty() {
        f.render_widget(Paragraph::new(left), area);
        return;
    }
    let right_len: usize = usage_spans.iter().map(|s| s.content.chars().count()).sum();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_len as u16)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(
        Paragraph::new(Line::from(usage_spans)).alignment(Alignment::Right),
        cols[1],
    );
}

/// Chip label + style for a given goal status. Active runs get a
/// pulsing-cyan feel via `SLOW_BLINK`; terminal statuses render solid
/// so the eye can distinguish "still going" from "done" at a glance.
fn goal_chip(status: GoalStatus) -> (&'static str, Style) {
    match status {
        GoalStatus::Active => (
            "▶ running",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        GoalStatus::Met => (
            "✓ met",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        GoalStatus::Impossible => (
            "✗ impossible",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        GoalStatus::NeedsUser => (
            "! needs you",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        GoalStatus::Exhausted => (
            "◐ exhausted",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        GoalStatus::Cleared => ("· cleared", Style::default().fg(Color::DarkGray)),
    }
}

fn transcript(f: &mut Frame, area: Rect, state: &mut TuiState) {
    // Track the row where each entry begins so we can scroll a search
    // hit into view even before the wrapped-line count is known.
    let query = state
        .search
        .as_ref()
        .map(|s| s.query.clone())
        .unwrap_or_default();
    let active_hit = state.active_hit();

    // (#2) Locate the most-recent successful edit_file/write_file/
    // apply_patch/create_file call. Only *that one* gets the
    // `/undo to revert` chip below its result, so the affordance
    // always points at what `/undo` will actually roll back.
    let undoable_call_idx = last_undoable_call_idx(state.entries());

    // Identity of the entry that is currently receiving live tokens —
    // used to render its trailing edge with a soft cursor and to skip
    // plan-card detection (which flickers if the model is mid-list).
    // Only when `state.streaming` is true; a completed reply that
    // happens to sit at the tail shouldn't be marked as live.
    let streaming_tail_idx: Option<usize> = if state.streaming {
        state
            .entries()
            .iter()
            .rposition(|e| matches!(e, LogEntry::Assistant(_)))
    } else {
        None
    };

    let mut lines: Vec<Line> = Vec::new();
    if state.dropped_entries > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  … {} earlier entr{} truncated",
                state.dropped_entries,
                if state.dropped_entries == 1 {
                    "y"
                } else {
                    "ies"
                },
            ),
            Style::default().fg(MUTED()).italic(),
        )));
        lines.push(Line::from(""));
    }
    let entries = state.entries();
    let mut entry_row_starts: Vec<usize> = Vec::with_capacity(entries.len());
    let mut i = 0;
    while i < entries.len() {
        let start = lines.len();
        let entry = &entries[i];
        let hit_this_entry = active_hit.map(|(idx, _)| idx) == Some(i);

        // Detect a run of consecutive tool call/result pairs of the same
        // family (all Read, all Edit, all Bash, …) and collapse them
        // into a single "Reading 3 files" block. Big density win when
        // the agent parallel-fires many reads on an exploration turn.
        if let LogEntry::ToolCall { name, args, .. } = entry {
            let (family, _) = summarize_tool(name, args);
            let mut batch_end = i;
            let mut batch = Vec::<(String, String)>::new(); // (family_label, summary)
            while let Some(LogEntry::ToolCall {
                name,
                args,
                preview,
                ..
            }) = entries.get(batch_end)
            {
                let (f, s) = summarize_tool(name, args);
                if f != family {
                    break;
                }
                // Don't batch calls that carry a diff preview — the
                // whole point of attaching it was to render the diff
                // inline, which would be lost in the collapsed batch
                // form.
                if preview.is_some() {
                    break;
                }
                // Must be followed by a *collapsed* result to batch —
                // expanded ones render individually so the user's Ctrl+E
                // toggle isn't silently swallowed.
                match entries.get(batch_end + 1) {
                    Some(LogEntry::ToolResult {
                        expanded: false, ..
                    }) => {
                        batch.push((f, s));
                        batch_end += 2;
                    }
                    _ => break,
                }
            }
            if batch.len() >= 2 {
                // Render batched summary block. Each entry (call + result)
                // maps to the same row start so search jumps still land
                // in the right batch.
                let lines_out = batched_tool_group_lines(&family, &batch);
                let trimmed = trim_empty(lines_out);
                for l in trimmed {
                    lines.push(l);
                }
                for _ in 0..(batch_end - i) {
                    entry_row_starts.push(start);
                }
                if batch_end < entries.len() {
                    lines.push(Line::from(""));
                    if matches!(entries.get(batch_end), Some(LogEntry::User(_))) {
                        lines.push(Line::from(""));
                    }
                }
                i = batch_end;
                continue;
            }
        }

        // Pair a ToolCall with its immediate ToolResult so they render as
        // one compact block ("● Read src/main.rs\n  └  fn main() { …")
        // instead of two loose lines. Falls through for in-flight calls
        // (no result yet) — they show the header with a spinner mark.
        let (group_lines, consumed) = if let LogEntry::ToolCall {
            name,
            args,
            preview,
            started_at,
            ..
        } = entry
        {
            let paired = matches!(entries.get(i + 1), Some(LogEntry::ToolResult { .. }));
            let result = if let Some(LogEntry::ToolResult {
                ok,
                snippet,
                full,
                expanded,
            }) = entries.get(i + 1)
            {
                Some(GroupResult {
                    ok: *ok,
                    snippet: snippet.as_str(),
                    full: full.as_str(),
                    expanded: *expanded,
                })
            } else {
                None
            };
            let hit_next = paired && active_hit.map(|(idx, _)| idx) == Some(i + 1);
            // Only feed the elapsed timer to the renderer for
            // in-flight calls — a completed call's ticker is noise.
            let elapsed = if result.is_none() {
                Some(started_at.elapsed())
            } else {
                None
            };
            let is_undoable_here = undoable_call_idx == Some(i);
            let g = tool_group_lines(
                name,
                args,
                result,
                preview.as_ref(),
                elapsed,
                &query,
                hit_this_entry || hit_next,
                is_undoable_here,
            );
            (g, if paired { 2 } else { 1 })
        } else {
            let is_streaming_here = streaming_tail_idx == Some(i);
            let mut ls = entry_to_lines_highlight_streaming(
                entry,
                &query,
                hit_this_entry,
                is_streaming_here,
            );
            // Plan mode: prepend a blue gutter to assistant lines so it's
            // obvious the model is brainstorming, not executing. Applies
            // only to assistant text; user/info/warn keep their normal
            // styling.
            if state.mode == Mode::Plan && matches!(entry, LogEntry::Assistant(_)) {
                ls = plan_gutter(ls);
            }
            (ls, 1)
        };

        // Strip leading/trailing empty lines. Markdown ending in `\n\n`
        // otherwise leaves 2 blank lines *plus* the separator below —
        // adding up to the huge vertical gaps Claude's TUI doesn't have.
        let trimmed = trim_empty(group_lines);

        // A group that renders to nothing (all-blank entry) still needs
        // an index in entry_row_starts to keep hit-scroll math correct.
        for l in trimmed {
            lines.push(l);
        }
        entry_row_starts.push(start);
        if consumed == 2 {
            entry_row_starts.push(start);
        }
        // One blank row between entries — anything more and it looks
        // like the transcript has scroll gaps.
        // (#7) When the *next* entry is a user prompt, add a second
        // blank row so the `> user` line gets breathing space above
        // and below (matches the landing mockup's paragraph rhythm).
        if i + consumed < entries.len() {
            lines.push(Line::from(""));
            if matches!(entries.get(i + consumed), Some(LogEntry::User(_))) {
                lines.push(Line::from(""));
            }
        }
        i += consumed;
    }

    // Ephemeral streaming indicator — appended to the transcript, below
    // the last entry (matches Claude Code). Not stored in `entries`, so
    // it disappears cleanly when the stream ends.
    if state.streaming {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(streaming_indicator_line(state));
    }

    // Ephemeral approval prompt — same trick as the streaming line, but
    // rendered as a small block below the pending tool call so the user
    // reads it inline with the call it's asking about (Claude Code
    // style). Keys are still routed to the approval handler as before.
    if let Some(pending) = state.pending_approval.as_ref() {
        lines.push(Line::from(""));
        for l in approval_prompt_lines(pending) {
            lines.push(l);
        }
    }
    if lines.is_empty() {
        // (#3) First-run onboarding card. Three lines that answer the
        // three "how do I…" questions first-timers actually have. Fades
        // out the moment the transcript has any entry — including our
        // own `push_info` boot line — so it never competes with real
        // content. Styled small and quiet so it reads as scaffolding.
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("▸▸", Style::default().fg(SALMON()).bold()),
            Span::styled("  ask anything", Style::default().fg(CREAM())),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("/ ", Style::default().fg(SALMON()).bold()),
            Span::styled(" commands  ·  ", Style::default().fg(MUTED())),
            Span::styled("@", Style::default().fg(SALMON()).bold()),
            Span::styled("  include a file", Style::default().fg(MUTED())),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("shift+tab ", Style::default().fg(SALMON()).bold()),
            Span::styled("cycle modes  ·  ", Style::default().fg(MUTED())),
            Span::styled("/help ", Style::default().fg(SALMON()).bold()),
            Span::styled("for everything else", Style::default().fg(MUTED())),
        ]));
    }

    let text = Text::from(lines);
    let para = Paragraph::new(text).wrap(Wrap { trim: false });

    // ratatui 0.29 exposes wrapped line count, which lets us compute the
    // tail offset and clamp scroll into a valid range.
    let total = para.line_count(area.width) as u16;
    let tail = total.saturating_sub(area.height);

    // Precedence: turn-nav jump > active search hit > follow-tail >
    // wherever the user last scrolled to.
    let scroll = if let Some(idx) = state.turn_scroll_target {
        let start = entry_row_starts.get(idx).copied().unwrap_or(0) as u16;
        // Anchor the user prompt line near the top so what comes after
        // (assistant reply, tool group) fills the viewport.
        start.saturating_sub(1).min(tail)
    } else if let Some((entry_idx, _)) = active_hit {
        let start = entry_row_starts.get(entry_idx).copied().unwrap_or(0) as u16;
        start.saturating_sub(area.height / 3).min(tail)
    } else if state.follow_tail {
        tail
    } else {
        state.scroll.min(tail)
    };
    // Turn-nav scroll is one-shot — consume it so a subsequent user
    // PgDn doesn't get snapped back to the anchor on next render.
    state.turn_scroll_target = None;

    // Persist the effective scroll so PgUp/PgDn work from where the user
    // is actually looking — not from a stale 0 they never chose.
    state.scroll = scroll;
    state.transcript_tail = tail;
    state.viewport_height = area.height;

    // Discovery hint: when the user has scrolled off the tail, show a
    // small `↑ pgdn to catch up` chip in the top-right of the
    // transcript so they know earlier content is preserved and how to
    // get back to live output.
    if !state.follow_tail && tail > 0 {
        let msg = "↑ scrolled  ·  pgdn to catch up";
        let w = (msg.chars().count() as u16).min(area.width.saturating_sub(2));
        let chip = Rect {
            x: area.x + area.width.saturating_sub(w + 1),
            y: area.y,
            width: w,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.to_owned(),
                Style::default().fg(SALMON()).bold(),
            )))
            .alignment(Alignment::Right),
            chip,
        );
    }

    f.render_widget(para.scroll((scroll, 0)), area);
}

fn entry_to_lines_highlight_streaming(
    entry: &LogEntry,
    query: &str,
    focused: bool,
    streaming: bool,
) -> Vec<Line<'static>> {
    // For an in-flight assistant entry, render via the streaming path:
    //   - Suppress the plan-card conversion (flickers as steps stream
    //     in mid-list).
    //   - Add a soft `▍` cursor after the last non-empty span so it's
    //     obvious tokens are still landing, even during a silent gap.
    if streaming {
        if let LogEntry::Assistant(s) = entry {
            let mut out = markdown::render(s);
            append_streaming_cursor(&mut out);
            return out;
        }
    }
    if query.is_empty() {
        return entry_to_lines(entry);
    }
    // In search mode, render Assistant entries as plain text so the
    // highlight overlay is visible. Markdown styling and yellow-on-
    // black highlights would fight each other otherwise.
    let base = match entry {
        LogEntry::Assistant(s) => s
            .lines()
            .map(|l| Line::from(Span::raw(l.to_owned())))
            .collect(),
        _ => entry_to_lines(entry),
    };
    base.into_iter()
        .map(|line| highlight_line(line, query, focused))
        .collect()
}

fn highlight_line<'a>(line: Line<'a>, query: &str, focused: bool) -> Line<'a> {
    // Build a single string from the line, then rescan for the needle
    // and rebuild spans with match runs styled. Loses per-span coloring
    // but that's acceptable in search mode — the highlight is the point.
    let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let needle = query.to_ascii_lowercase();
    let hay = raw.to_ascii_lowercase();
    let mut out: Vec<Span> = Vec::new();
    let mut cursor = 0;
    while let Some(off) = hay[cursor..].find(&needle) {
        let start = cursor + off;
        if start > cursor {
            out.push(Span::raw(raw[cursor..start].to_owned()));
        }
        let end = start + needle.len();
        let mut style = Style::default().bg(Color::Yellow).fg(Color::Black);
        if focused {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        out.push(Span::styled(raw[start..end].to_owned(), style));
        cursor = end;
    }
    if cursor < raw.len() {
        out.push(Span::raw(raw[cursor..].to_owned()));
    }
    if out.is_empty() {
        line
    } else {
        Line::from(out)
    }
}

fn entry_to_lines(entry: &LogEntry) -> Vec<Line<'static>> {
    match entry {
        LogEntry::User(s) => user_lines(s),
        LogEntry::TurnEnd { elapsed_ms } => turn_end_lines(*elapsed_ms),
        LogEntry::Assistant(s) => {
            // (#5) When the assistant reply is a "Plan:" doc, render
            // as a bordered card with checkbox steps — matches the
            // approval card's visual weight so a plan lands as a
            // discrete object in the transcript rather than a wall
            // of markdown. Any other assistant reply falls through
            // to the standard markdown renderer.
            if let Some(plan) = try_parse_plan(s) {
                render_plan_card(&plan)
            } else {
                markdown::render(s)
            }
        }
        LogEntry::ToolCall {
            name,
            args,
            preview,
            ..
        } => tool_group_lines(name, args, None, preview.as_ref(), None, "", false, false),
        LogEntry::ToolResult {
            ok,
            snippet,
            full,
            expanded,
        } => {
            // Orphan result (no preceding call) — should be rare, but
            // render defensively as a stand-alone block.
            let result = GroupResult {
                ok: *ok,
                snippet: snippet.as_str(),
                full: full.as_str(),
                expanded: *expanded,
            };
            tool_group_lines("(result)", "", Some(result), None, None, "", false, false)
        }
        LogEntry::Warning(s) => vec![Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Yellow)),
        ])],
        LogEntry::Info(s) => vec![Line::from(Span::styled(
            s.clone(),
            Style::default().fg(MUTED()).italic(),
        ))],
    }
}

/// Claude-style user turn: `> ` prompt in salmon, message in cream on
/// its own row(s). Multi-line messages (Ctrl+J) get the `> ` marker on
/// the first row only and a hanging indent on the rest so paragraphs
/// read cleanly.
fn user_lines(s: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut first = true;
    for line in s.lines() {
        let mark = if first { "> " } else { "  " };
        out.push(Line::from(vec![
            Span::styled(mark, Style::default().fg(SALMON()).bold()),
            Span::styled(line.to_owned(), Style::default().fg(CREAM())),
        ]));
        first = false;
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(
            "> ",
            Style::default().fg(SALMON()).bold(),
        )));
    }
    out
}

/// Turn-end marker rendered under the assistant reply:
///
///     ✳ Baked for 12.4s
///
/// Muted italic so it reads as a full stop, not a headline. Verb is
/// picked from [`turn_verb_past`] against the millisecond bucket so
/// the same duration always reads the same word.
fn turn_end_lines(elapsed_ms: u32) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("✳ ", Style::default().fg(SALMON())),
        Span::styled(
            turn_end_label(elapsed_ms),
            Style::default().fg(MUTED()).italic(),
        ),
    ])]
}

/// Shared label so the transcript-plaintext export and the on-screen
/// renderer agree on wording (avoids "Baked for 4s" in the TUI but
/// "12300ms" in the markdown save).
pub fn turn_end_label(elapsed_ms: u32) -> String {
    format!(
        "{} for {}",
        turn_verb_past(elapsed_ms),
        format_turn_elapsed(elapsed_ms)
    )
}

/// Past-tense counterpart to the streaming vocabulary — picked from a
/// deterministic bucket of `elapsed_ms` so an identical reply time
/// always reads the same. The set is curated for the cooking /
/// "gently applied thought" register that reads as playful without
/// undermining a serious reply.
fn turn_verb_past(elapsed_ms: u32) -> &'static str {
    const WORDS: &[&str] = &[
        "Baked",
        "Brewed",
        "Cooked",
        "Simmered",
        "Steeped",
        "Percolated",
        "Wrangled",
        "Pondered",
        "Cogitated",
        "Mused",
        "Sizzled",
        "Roasted",
        "Whisked",
        "Stirred",
        "Seasoned",
        "Marinated",
        "Chopped",
        "Kneaded",
        "Blended",
        "Fermented",
        "Concocted",
        "Crafted",
        "Forged",
        "Tinkered",
        "Hacked",
        "Wrestled",
        "Untangled",
        "Investigated",
        "Devised",
        "Dreamed",
        "Schemed",
        "Calculated",
        "Reasoned",
        "Explored",
        "Assembled",
        "Refined",
        "Polished",
        "Orchestrated",
        "Discovered",
        "Conjured",
        "Analyzed",
        "Decoded",
        "Debugged",
        "Prototyped",
        "Architected",
        "Engineered",
        "Compiled",
        "Rendered",
        "Optimized",
        "Configured",
        "Refactored",
        "Researched",
        "Surveyed",
        "Scanned",
        "Traced",
        "Mapped",
        "Indexed",
        "Linked",
        "Connected",
        "Shaped",
        "Invented",
        "Imagined",
        "Envisioned",
        "Experimented",
        "Improvised",
        "Solved",
        "Cracked",
        "Unraveled",
        "Untwisted",
        "Unfolded",
        "Sautéed",
        "Grilled",
        "Basted",
        "Broiled",
        "Glazed",
    ];

    let idx = ((elapsed_ms / 173) as usize) % WORDS.len();
    WORDS[idx]
}

/// Compact "wall time between user submit and stream done":
/// - Under 10s: one decimal ("6.4s").
/// - Under a minute: whole seconds ("42s").
/// - Above: minutes+seconds ("1m 07s").
fn format_turn_elapsed(ms: u32) -> String {
    let secs = ms as f32 / 1000.0;
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{}s", secs as u32)
    } else {
        let m = (secs as u32) / 60;
        let s = (secs as u32) % 60;
        format!("{m}m {s:02}s")
    }
}

/// A rendered ToolResult passed alongside its ToolCall so the group can
/// show as one visual block. Borrowed strings, since the paired entries
/// outlive the render pass.
#[derive(Clone, Copy)]
struct GroupResult<'a> {
    ok: bool,
    snippet: &'a str,
    full: &'a str,
    expanded: bool,
}

/// Compact Claude-style block for a tool call + result:
///
///     ● Read src/main.rs
///       └  fn main() { println!("hi") ...
///
/// Expanded form ("ctrl+e") replaces the `└  snippet` line with the
/// full output prefixed by a `│` gutter. When called for an in-flight
/// call (no result yet), the header shows a dim spinner mark.
fn tool_group_lines(
    name: &str,
    args: &str,
    result: Option<GroupResult<'_>>,
    preview: Option<&DiffPreview>,
    elapsed: Option<std::time::Duration>,
    query: &str,
    focused: bool,
    is_last_undoable: bool,
) -> Vec<Line<'static>> {
    let (label, summary) = summarize_tool(name, args);
    let (mark_glyph, mark_color) = match result {
        Some(r) if r.ok => ("● ", Color::Green),
        Some(_) => ("● ", Color::Red),
        None => ("◐ ", SALMON()),
    };
    let mut header: Vec<Span<'static>> = vec![
        Span::styled(mark_glyph, Style::default().fg(mark_color).bold()),
        Span::styled(label, Style::default().fg(SALMON()).bold()),
    ];
    if !summary.is_empty() {
        header.push(Span::styled(" ", Style::default()));
        header.push(Span::styled(
            truncate(&summary, 140),
            Style::default().fg(CREAM()),
        ));
    }
    // Show `+N -M` diff stats when the call carries a preview so
    // successful edits get a scanable "how much changed" glance.
    if let Some(p) = preview {
        let (adds, dels) = diff_stats(p);
        header.push(Span::styled(
            format!("  +{adds}",),
            Style::default().fg(Color::Green).bold(),
        ));
        header.push(Span::styled(
            format!(" -{dels}",),
            Style::default().fg(Color::Red).bold(),
        ));
    }
    // (#4) In-flight elapsed ticker — only when there's no result yet.
    // Render pass fires every 100ms while streaming, so this updates
    // continuously with no extra timer plumbing. Sub-second calls stay
    // silent to avoid flicker on the common fast-path.
    if let Some(d) = elapsed {
        let secs = d.as_secs_f32();
        if secs >= 0.6 {
            header.push(Span::styled(
                format!(" · {}", format_elapsed(secs)),
                Style::default().fg(MUTED()),
            ));
        }
    }
    // Expandability is signaled implicitly by the truncated `└` snippet
    // below the header — no chevron. The previous `⌄` suffix was getting
    // orphaned by `Paragraph::wrap` when the header exceeded terminal
    // width (ratatui broke on the leading whitespace and dropped the
    // lone glyph onto its own row, reading as a stray `;`-ish mark in
    // fonts without U+2304). `Ctrl+E` is still discoverable via `/help`.

    let mut out: Vec<Line<'static>> = vec![Line::from(header)];

    // Body: prefer the diff preview when the call succeeded — that's
    // the feedback loop users actually want ("what did the edit
    // change?"). Fall back to the tool's own output otherwise.
    let show_diff = preview.is_some() && result.map(|r| r.ok).unwrap_or(false);
    match (show_diff, result) {
        (true, Some(_)) => {
            let p = preview.unwrap();
            let cap = 12;
            for dl in p.lines.iter().take(cap) {
                out.push(indented_diff_line(dl));
            }
            if p.lines.len() > cap {
                out.push(Line::from(Span::styled(
                    format!("     … {} more diff lines", p.lines.len() - cap),
                    Style::default().fg(MUTED()).italic(),
                )));
            }
        }
        (_, None) => {}
        (_, Some(r)) if r.expanded && !r.full.is_empty() => {
            for line in r.full.lines() {
                out.push(Line::from(vec![
                    Span::styled("  │  ", Style::default().fg(DIM())),
                    Span::styled(line.to_owned(), Style::default().fg(MUTED())),
                ]));
            }
        }
        (_, Some(r)) => {
            // (#1, #2) A green-dot success + a bare "exit=0" snippet is
            // pure noise — the dot already says "worked." Suppress the
            // body row in that specific case; if Bash produced real
            // output the user still sees the chevron and can Ctrl+E.
            let bash_success_no_output = r.ok
                && r.snippet.trim() == "exit=0"
                && !r
                    .full
                    .split("--- output ---\n")
                    .nth(1)
                    .map(|o| !o.trim().is_empty())
                    .unwrap_or(false);
            if !bash_success_no_output {
                let body = if r.snippet.is_empty() {
                    "(no output)".to_owned()
                } else {
                    truncate(r.snippet, 200)
                };
                out.push(Line::from(vec![
                    Span::styled("  └  ", Style::default().fg(DIM())),
                    Span::styled(body, Style::default().fg(MUTED())),
                ]));
            }
        }
    }

    // (#2) Undo affordance on the most-recent successful write.
    // Rendered as a small dim chip so it points at `/undo` without
    // shouting for attention on every edit.
    if is_last_undoable {
        out.push(Line::from(vec![
            Span::styled("     ", Style::default()),
            Span::styled("/undo", Style::default().fg(SALMON()).bold()),
            Span::styled(" to revert", Style::default().fg(MUTED()).italic()),
        ]));
    }

    if query.is_empty() {
        out
    } else {
        out.into_iter()
            .map(|l| highlight_line(l, query, focused))
            .collect()
    }
}

/// Count `+`/`-` lines in a diff preview for the header stat chip.
fn diff_stats(p: &DiffPreview) -> (usize, usize) {
    let mut adds = 0;
    let mut dels = 0;
    for l in &p.lines {
        match l {
            DiffLine::Add(_) => adds += 1,
            DiffLine::Del(_) => dels += 1,
            _ => {}
        }
    }
    (adds, dels)
}

/// Collapsed multi-call rendering, Claude-style. `family` is the
/// friendly name from `summarize_tool` (e.g. "Read"), `entries` is
/// each call's `(family_label, arg_summary)`. Renders as:
///
///     ● Reading 3 files
///       └  src/main.rs
///       └  src/lib.rs
///       └  src/foo.rs
fn batched_tool_group_lines(family: &str, entries: &[(String, String)]) -> Vec<Line<'static>> {
    let (verb, noun) = batch_verb_noun(family);
    let n = entries.len();
    let header = Line::from(vec![
        Span::styled("● ", Style::default().fg(Color::Green).bold()),
        Span::styled(
            format!("{verb} {n} {noun}"),
            Style::default().fg(SALMON()).bold(),
        ),
    ]);

    let mut out = vec![header];
    for (_, summary) in entries {
        let body = if summary.is_empty() {
            "(no arg)".to_owned()
        } else {
            truncate(summary, 140)
        };
        out.push(Line::from(vec![
            Span::styled("  └  ", Style::default().fg(DIM())),
            Span::styled(body, Style::default().fg(MUTED())),
        ]));
    }
    out
}

/// Family → ("Reading", "files") style pair. Falls back to
/// ("Calling", "tools") for unknown families so we always render
/// something readable.
fn batch_verb_noun(family: &str) -> (&'static str, &'static str) {
    match family {
        "Read" => ("Reading", "files"),
        "Edit" => ("Editing", "files"),
        "Write" => ("Writing", "files"),
        "Bash" => ("Running", "commands"),
        "Glob" => ("Globbing", "patterns"),
        "Grep" => ("Grepping", "patterns"),
        "Fetch" => ("Fetching", "URLs"),
        "Search" => ("Searching", "queries"),
        _ => ("Calling", "tools"),
    }
}

/// A `Line` is "empty" when every span it holds is blank. Used to
/// trim leading/trailing blank rows from an entry's rendering so the
/// transcript separator stays a single row.
fn line_is_empty(l: &Line<'_>) -> bool {
    l.spans.iter().all(|s| s.content.trim().is_empty())
}

fn trim_empty(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines.first().map(line_is_empty).unwrap_or(false) {
        lines.remove(0);
    }
    while lines.last().map(line_is_empty).unwrap_or(false) {
        lines.pop();
    }
    lines
}

/// The in-transcript streaming indicator — replaces the status-bar
/// spinner so users see it inline with the assistant reply (Claude
/// Code's convention). Format:
///
///     ✻ Wrangling… (12s · ↓ 1.2k tokens · esc to interrupt)
fn streaming_indicator_line(state: &TuiState) -> Line<'static> {
    let secs = state
        .stream_started_at
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let label = streaming_label(secs);
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            format!("{LOGO} "),
            Style::default().fg(pulsed_logo(secs)).bold(),
        ),
        Span::styled(label, Style::default().fg(SALMON()).bold()),
        Span::styled(
            format!("… ({}", fmt_secs(secs)),
            Style::default().fg(MUTED()),
        ),
    ];
    let turn_completion = state
        .usage
        .completion_tokens
        .saturating_sub(state.turn_usage_baseline.completion_tokens);
    if turn_completion > 0 {
        spans.push(Span::styled(
            format!(" · ↓{} tokens", short_num(turn_completion)),
            Style::default().fg(MUTED()),
        ));
    }
    spans.push(Span::styled(
        " · esc to interrupt)",
        Style::default().fg(MUTED()),
    ));
    Line::from(spans)
}

/// `12s`, `1m 7s` — mirrors Claude's status format so long runs read
/// naturally instead of `67.4s`.
fn fmt_secs(secs: f32) -> String {
    let total = secs as u64;
    if total < 60 {
        format!("{total}s")
    } else {
        let m = total / 60;
        let s = total % 60;
        format!("{m}m {s}s")
    }
}

/// Extract a friendly name + one-line arg summary from a tool call so we
/// can render `Read src/foo.rs` instead of `read_file({"path":"..."})`.
/// Unknown tools keep their raw name and get the first string-valued arg
/// as a summary.
fn summarize_tool(name: &str, args: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let get =
        |k: &str| -> Option<String> { v.get(k).and_then(|x| x.as_str()).map(|s| s.to_owned()) };
    let one_line = |s: String| -> String { s.lines().next().unwrap_or("").to_owned() };
    match name {
        "read_file" | "view_file" => ("Read".to_owned(), get("path").unwrap_or_default()),
        "edit_file" | "apply_patch" => (
            "Edit".to_owned(),
            get("path").or_else(|| get("target")).unwrap_or_default(),
        ),
        "write_file" | "create_file" => ("Write".to_owned(), get("path").unwrap_or_default()),
        "shell" | "bash" => (
            "Bash".to_owned(),
            one_line(get("cmd").or_else(|| get("command")).unwrap_or_default()),
        ),
        "list_files" | "glob" => (
            "Glob".to_owned(),
            get("pattern").or_else(|| get("path")).unwrap_or_default(),
        ),
        "grep" | "search" | "ripgrep" => (
            "Grep".to_owned(),
            get("pattern").or_else(|| get("query")).unwrap_or_default(),
        ),
        "web_fetch" | "fetch" => ("Fetch".to_owned(), get("url").unwrap_or_default()),
        "web_search" => ("Search".to_owned(), get("query").unwrap_or_default()),
        _ => {
            // Fallback: keep the raw name; surface the first string arg
            // as the summary so unknown tools still show something.
            let first_str = v
                .as_object()
                .and_then(|o| o.values().find_map(|x| x.as_str()))
                .map(str::to_owned)
                .unwrap_or_default();
            (name.to_owned(), one_line(first_str))
        }
    }
}

/// Prompt prefix rendered inline on the first input line. `▸` is
/// U+25B8 (narrow triangle) so it lays out as one column each in a
/// monospace font — the whole prefix is three columns wide.
const PROMPT: &str = "▸▸ ";
const PROMPT_COLS: u16 = 3;
/// Continuation-line indent so multi-line input aligns under the first
/// character after `▸▸ ` instead of hanging out into the margin.
const PROMPT_CONT: &str = "   ";

fn input(f: &mut Frame, area: Rect, state: &TuiState) {
    // (asks 1 & 2) Composer top border is a full-width coral hairline —
    // no title interrupting it. The `▸▸` prompt lives inside the body,
    // inline with the text the user is typing (matches the landing
    // mockup where the arrow sits on the same line as the message).
    let hairline_color = if state.streaming { MUTED() } else { HAIRLINE() };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(hairline_color));

    let prompt_style = Style::default().fg(SALMON()).bold();
    let body: Text = if state.input().is_empty() {
        Line::from(vec![
            Span::styled(PROMPT, prompt_style),
            Span::styled("message", Style::default().fg(DIM()).italic()),
        ])
        .into()
    } else {
        // Split on '\n' rather than .lines() so a trailing newline shows
        // as an empty last row (the cursor lands there after Ctrl+J).
        let segments: Vec<Line> = state
            .input()
            .split('\n')
            .enumerate()
            .map(|(i, l)| {
                let (prefix, style) = if i == 0 {
                    (PROMPT, prompt_style)
                } else {
                    (PROMPT_CONT, Style::default())
                };
                let mut spans: Vec<Span<'static>> = vec![Span::styled(prefix, style)];
                spans.extend(render_composer_line(l, state));
                Line::from(spans)
            })
            .collect();
        Text::from(segments)
    };
    f.render_widget(Paragraph::new(body).block(block), area);

    // Position the terminal caret. Body starts +1 row below the top
    // hairline; every line carries the PROMPT_COLS-wide prefix, so the
    // caret is offset by that width regardless of which row it's on.
    let (row, col) = cursor_visual(state.input(), state.cursor());
    f.set_cursor_position((area.x + PROMPT_COLS + col, area.y + 1 + row));
}

/// Split one composer line around any `[[paste:N]]` placeholders and
/// render each stash as a compact `[pasted N lines]` chip. Non-paste
/// text renders in the usual cream. The visual char-count changes,
/// but `cursor_visual` still works off the raw byte cursor so caret
/// placement lands where the user is typing — worst case the caret
/// sits inside the placeholder text, which reads as a natural
/// "you're editing this token" affordance.
fn render_composer_line(line: &str, state: &TuiState) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut cursor = 0;
    while let Some(hit) = line[cursor..].find("[[paste:") {
        let start = cursor + hit;
        if start > cursor {
            out.push(Span::styled(
                line[cursor..start].to_owned(),
                Style::default().fg(CREAM()),
            ));
        }
        let after_open = start + "[[paste:".len();
        let Some(rel_end) = line[after_open..].find("]]") else {
            // Malformed — treat the rest of the line as plain text.
            out.push(Span::styled(
                line[start..].to_owned(),
                Style::default().fg(CREAM()),
            ));
            return out;
        };
        let end = after_open + rel_end;
        let id_part = &line[after_open..end];
        let after_close = end + 2;
        let label = match id_part
            .parse::<u32>()
            .ok()
            .and_then(|id| state.pastes.iter().find(|p| p.id == id))
        {
            Some(p) => format!(
                " [pasted {} line{}] ",
                p.lines,
                if p.lines == 1 { "" } else { "s" }
            ),
            None => format!(" [pasted ?] "),
        };
        out.push(Span::styled(
            label,
            Style::default()
                .fg(Color::Black)
                .bg(SALMON())
                .add_modifier(Modifier::BOLD),
        ));
        cursor = after_close;
    }
    if cursor < line.len() {
        out.push(Span::styled(
            line[cursor..].to_owned(),
            Style::default().fg(CREAM()),
        ));
    }
    if out.is_empty() {
        out.push(Span::raw(String::new()));
    }
    out
}

/// Wrap the input into (row, col) cell coordinates for the caret.
/// `col` counts chars, not display width — sufficient for ASCII/latin
/// and a passable approximation for most other text.
fn cursor_visual(input: &str, cursor_byte: usize) -> (u16, u16) {
    let mut row: u16 = 0;
    let mut col: u16 = 0;
    let mut byte = 0;
    for c in input.chars() {
        if byte >= cursor_byte {
            break;
        }
        if c == '\n' {
            row = row.saturating_add(1);
            col = 0;
        } else {
            col = col.saturating_add(1);
        }
        byte += c.len_utf8();
    }
    (row, col)
}

/// Draw the palette as a floating list *above* the input area. Height
/// caps at 8 rows to avoid taking over the screen; wider palettes just
/// scroll internally with the selection cursor.
fn palette(f: &mut Frame, input_area: Rect, state: &TuiState) {
    let width = input_area.width.min(60);
    let visible = (state.palette.matches.len() as u16).min(8);
    let height = visible + 2; // + borders

    // Anchor to the input's left edge, floating just above it.
    let anchor_x = input_area.x;
    // If there's no room above, drop the overlay below.
    let anchor_y = input_area.y.saturating_sub(height);
    let area = Rect {
        x: anchor_x,
        y: anchor_y,
        width,
        height,
    };
    f.render_widget(Clear, area);

    let title = match state.palette.kind {
        Palette::Slash => " commands ",
        Palette::AtFile => " files (rg --files) ",
        Palette::Model => " models ",
        Palette::Theme => " themes ",
        Palette::SavePath => " save transcript to… ",
        Palette::None => "",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SALMON()))
        .title(Span::styled(title, Style::default().fg(SALMON()).bold()));

    let items: Vec<ListItem> = state
        .palette
        .matches
        .iter()
        .map(|m| {
            let mut spans = vec![Span::styled(m.title.clone(), Style::default().fg(CREAM()))];
            if !m.detail.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    m.detail.clone(),
                    Style::default().fg(MUTED()).italic(),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(DIM())
                .fg(SALMON())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.palette.cursor));
    f.render_stateful_widget(list, area, &mut list_state);
}

fn status(f: &mut Frame, area: Rect, state: &TuiState) {
    // Error flash trumps everything — a red-bg row for the full
    // duration of `state.error_flash_until`. Renders the row solid
    // so the label reads as an alert bar rather than a floating chip
    // that could get lost against the transcript.
    if state.error_flash_active() {
        let label = state
            .error_flash_label
            .as_deref()
            .unwrap_or("provider error");
        let bar = format!(" ⚠  {label}");
        let bar = pad_to_width(&bar, area.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }

    // (#3 + #9) Unified single-row footer:
    //   left  = hint / flash / esc-pending
    //   right = mode chip in its own color + `shift+tab to cycle`
    // Split with a Layout so the right side is always right-anchored
    // regardless of hint length.
    let left: Line<'static> = if state.esc_pending {
        let hint = if !state.is_input_empty() {
            "press esc again to clear input"
        } else {
            "press esc again to quit"
        };
        Line::from(Span::styled(
            format!(" {hint}"),
            Style::default().fg(SALMON()).bold(),
        ))
    } else if let Some(flash) = &state.flash {
        Line::from(Span::styled(
            format!(" {flash}"),
            Style::default().fg(Color::Green).italic(),
        ))
    } else {
        // (ask 3) Trimmed to the five actions the mockup shows —
        // `ctrl+r search` and `ctrl+e expand` were pushing the hint
        // wider than the composer and doubling as noise. Both are still
        // discoverable via `/help`.
        Line::from(Span::styled(
            " enter send · / cmd · @ file · shift+tab mode · esc esc quit",
            Style::default().fg(MUTED()),
        ))
    };

    let right = Line::from(vec![
        Span::styled(state.mode.chip_label(), mode_style(state)),
        Span::styled(" · shift+tab to cycle ", Style::default().fg(MUTED())),
    ]);
    let right_len = right
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_len)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), cols[1]);
}

/// Rotate through a small vocabulary of streaming-verbs based on
/// elapsed-time buckets. Keeps the status line feeling alive during
/// long silent gaps instead of just repeating "thinking".
fn streaming_label(secs: f32) -> &'static str {
    const WORDS: &[&str] = &[
        "Wrangling",
        "Thinking",
        "Pondering",
        "Cogitating",
        "Musing",
        "Simmering",
        "Brewing",
        "Percolating",
    ];
    let idx = ((secs / 4.0) as usize) % WORDS.len();
    WORDS[idx]
}

/// Compact right-side accounting as styled spans: `↑12.3k ↓4.1k · 15% · $0.024`.
/// Dollar figure paints red when `state.budget_usd` is set and the
/// running cost has met or exceeded it. Empty when the provider hasn't
/// reported any usage yet.
fn format_usage_spans(state: &TuiState) -> Vec<Span<'static>> {
    let u = &state.usage;
    if u.is_zero() {
        return Vec::new();
    }
    let muted = Style::default().fg(MUTED());
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        format!(
            "↑{} ↓{}",
            short_num(u.prompt_tokens),
            short_num(u.completion_tokens)
        ),
        muted,
    ));
    if let Some(ctx) = model_context_len(&state.model) {
        let pct = ((u.prompt_tokens as f64 / ctx as f64) * 100.0).min(999.0);
        spans.push(Span::styled(format!(" · {pct:.0}%"), muted));
    }
    if let Some(dollars) = mira_ai::cost_usd(
        &state.model,
        mira_ai::TokenUsage {
            prompt_tokens: u.prompt_tokens.min(u32::MAX as u64) as u32,
            completion_tokens: u.completion_tokens.min(u32::MAX as u64) as u32,
            cached_input_tokens: u.cached_input_tokens.min(u32::MAX as u64) as u32,
        },
    ) {
        spans.push(Span::styled(" · ".to_owned(), muted));
        let over = state.budget_usd.map(|cap| dollars >= cap).unwrap_or(false);
        let dollar_style = if over {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        let mut label = format_dollars(dollars);
        if let Some(cap) = state.budget_usd {
            // Show the cap alongside so the user sees the runway shrink
            // without having to `/cost`.
            label.push_str(&format!("/${cap:.2}"));
        }
        spans.push(Span::styled(label, dollar_style));
    }
    spans
}

/// Best-effort context window size for common model IDs. Substring
/// match on the model string covers OpenRouter's `provider/name` form
/// as well as bare provider names. Returns `None` for unknowns → the
/// status bar just omits the `%` column.
fn model_context_len(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    // Order matters: check more specific matches first.
    if m.contains("gpt-4.1") || m.contains("gpt-5") {
        Some(1_000_000)
    } else if m.contains("claude-3-5-sonnet") || m.contains("claude-sonnet-4") {
        Some(200_000)
    } else if m.contains("claude-3-5-haiku") || m.contains("claude-haiku-4") {
        Some(200_000)
    } else if m.contains("claude-3-opus") || m.contains("claude-opus-4") {
        Some(200_000)
    } else if m.contains("gemini-2.5") || m.contains("gemini-1.5") {
        Some(1_000_000)
    } else if m.contains("gpt-4o") {
        Some(128_000)
    } else if m.contains("llama-3.3") || m.contains("llama-3.1") {
        Some(128_000)
    } else if m.contains("deepseek") {
        Some(64_000)
    } else if m.contains("grok") {
        Some(128_000)
    } else {
        None
    }
}

fn short_num(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    }
}

fn format_dollars(d: f64) -> String {
    if d < 0.01 {
        format!("${d:.4}")
    } else if d < 1.0 {
        format!("${d:.3}")
    } else {
        format!("${d:.2}")
    }
}

/// (#5) A model reply that reads as a plan proposal. Detected by a
/// heading line starting with `# Plan`, `## Plan`, or a first line of
/// `Plan:` (case-insensitive), followed by a numbered or bulleted
/// list. Returns the parsed shape so [`render_plan_card`] can lay it
/// out as a bordered card.
struct Plan {
    title: String,
    steps: Vec<String>,
}

fn try_parse_plan(s: &str) -> Option<Plan> {
    let mut lines = s.lines().peekable();
    let first = lines.next()?.trim();
    // Accept `# Plan …`, `## Plan …`, or `Plan: …` / `Plan …` / plain `Plan`
    let title = if let Some(rest) = first.strip_prefix("# Plan") {
        rest.trim_start_matches(':').trim().to_owned()
    } else if let Some(rest) = first.strip_prefix("## Plan") {
        rest.trim_start_matches(':').trim().to_owned()
    } else if let Some(rest) = first.to_ascii_lowercase().strip_prefix("plan") {
        // Accept `Plan`, `Plan:`, `Plan — foo`. Anything after is the title.
        let rest = rest.trim_start_matches(':').trim();
        if rest.is_empty() && first.eq_ignore_ascii_case("plan") {
            String::new()
        } else if !rest.is_empty() {
            rest.to_owned()
        } else {
            return None;
        }
    } else {
        return None;
    };

    // Collect numbered- or bulleted-list steps. Blank lines end the
    // step run; anything not-a-list-item after that ends parsing.
    let mut steps: Vec<String> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() && steps.is_empty() {
            continue; // skip blanks before the list starts
        }
        if line.is_empty() {
            break; // stop at the blank line after the list
        }
        // `1. do X`, `12. do Y`, `- do X`, `* do Y`
        let step = if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            rest.to_owned()
        } else {
            let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                break;
            }
            let after = &line[digits.len()..];
            if let Some(rest) = after
                .strip_prefix(". ")
                .or_else(|| after.strip_prefix(") "))
            {
                rest.to_owned()
            } else {
                break;
            }
        };
        steps.push(step);
    }

    if steps.len() < 2 {
        return None; // one-step "plan" is just a sentence, don't box it
    }
    Some(Plan { title, steps })
}

fn render_plan_card(plan: &Plan) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let bar_style = Style::default().fg(SALMON());
    // Top rule with inline title chip
    let mut header: Vec<Span<'static>> = vec![
        Span::styled("╭─ ", bar_style),
        Span::styled("plan", Style::default().fg(SALMON()).bold()),
    ];
    if !plan.title.is_empty() {
        header.push(Span::styled("  ", Style::default()));
        header.push(Span::styled(
            plan.title.clone(),
            Style::default().fg(CREAM()).bold(),
        ));
    }
    out.push(Line::from(header));

    for step in &plan.steps {
        out.push(Line::from(vec![
            Span::styled("│  ", bar_style),
            Span::styled("[ ] ", Style::default().fg(MUTED())),
            Span::styled(step.clone(), Style::default().fg(CREAM())),
        ]));
    }
    // Footer hint — no interaction yet, but signal the card affordance.
    out.push(Line::from(vec![
        Span::styled("╰─ ", bar_style),
        Span::styled(
            "review the plan · say `go` / `edit` / `cancel`",
            Style::default().fg(MUTED()).italic(),
        ),
    ]));
    out
}

/// True when `name` is one of the tools that mira-tools journals into
/// `.mira/.undo/` — i.e. exactly the set `/undo` can roll back. Same
/// list `summarize_tool` gives an "Edit" / "Write" label to.
fn is_undoable_tool(name: &str) -> bool {
    matches!(
        name,
        "edit_file" | "write_file" | "apply_patch" | "create_file"
    )
}

/// Index of the last entry that's an undoable, *successful* tool call.
/// Scans backwards, stops at the first paired `(call, result)` where
/// the tool is a writer and the result is ok. Returns `None` when no
/// such call is in the visible transcript.
fn last_undoable_call_idx(entries: &[LogEntry]) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let mut i = entries.len();
    while i > 0 {
        i -= 1;
        if let LogEntry::ToolCall { name, .. } = &entries[i] {
            if !is_undoable_tool(name) {
                continue;
            }
            match entries.get(i + 1) {
                Some(LogEntry::ToolResult { ok: true, .. }) => return Some(i),
                _ => continue,
            }
        }
    }
    None
}

/// `1.4s` under 10s, `12s` under a minute, `1m03s` above. Keeps the
/// in-flight tool ticker compact whether the call takes a beat or
/// half a minute (rare — usually a Bash that hasn't crashed yet).
fn format_elapsed(secs: f32) -> String {
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{}s", secs as u32)
    } else {
        let mins = (secs as u32) / 60;
        let rem = (secs as u32) % 60;
        format!("{mins}m{rem:02}s")
    }
}

/// Small search bar rendered just above the composer while Ctrl+R is
/// active. Shows the query, hit count, and cursor position. Enter/n
/// cycles; Esc closes.
fn search_overlay(f: &mut Frame, input_area: Rect, state: &TuiState) {
    let Some(s) = state.search.as_ref() else {
        return;
    };
    let height: u16 = 3;
    let width = input_area.width;
    let anchor_y = input_area.y.saturating_sub(height);
    let area = Rect {
        x: input_area.x,
        y: anchor_y,
        width,
        height,
    };
    f.render_widget(Clear, area);

    let hits = s.hits.len();
    let cursor = if hits == 0 { 0 } else { s.cursor + 1 };
    let title = format!(" search · {cursor}/{hits} · enter next · esc close ");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(Span::styled(
            title,
            Style::default().fg(Color::Magenta).bold(),
        ));

    let body = Line::from(vec![
        Span::styled("/", Style::default().fg(Color::DarkGray)),
        Span::styled(
            s.query.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]);

    let para = Paragraph::new(body).block(block);
    f.render_widget(para, area);
}

/// Inline approval block. Redesign goals:
///
/// - **One-glance clarity**: `⚠  Bash  ·  approval required` — no
///   more raw JSON dumps, no "approve X?" as a header line.
/// - **Show the meaningful thing, not the wrapper**: extract the
///   command / path from the tool call and format it like a code
///   snippet, not a `{"command":"..."}` blob.
/// - **Boxed key badges**: `[ y ] approve` reads faster than
///   `y allow · a always · n deny` and looks like buttons.
fn approval_prompt_lines(pending: &crate::tui::state::PendingApproval) -> Vec<Line<'static>> {
    let name = pending.request.call.function.name.as_str();
    let args = pending.request.call.function.arguments.as_str();
    let (friendly, primary) = summarize_tool(name, args);
    let mut out: Vec<Line<'static>> = Vec::new();

    // Top rule so the block stands out from the surrounding transcript.
    out.push(rule_line());

    // Header: warning glyph + friendly tool name + (optional) target.
    let mut header: Vec<Span<'static>> = vec![
        Span::styled("  ⚠  ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(friendly.clone(), Style::default().fg(SALMON()).bold()),
        Span::styled("  ·  approval required", Style::default().fg(CREAM())),
    ];
    if let Some(preview) = &pending.preview {
        let kind_label = match preview.kind {
            DiffKind::Edit => "edit",
            DiffKind::Overwrite => "overwrite",
            DiffKind::Create => "create",
        };
        header.push(Span::styled("  ·  ", Style::default().fg(DIM())));
        header.push(Span::styled(
            preview.path.clone(),
            Style::default().fg(CREAM()).bold(),
        ));
        header.push(Span::styled(
            format!("  ({kind_label})"),
            Style::default().fg(MUTED()),
        ));
    }
    out.push(Line::from(header));
    out.push(Line::from(""));

    // Body: diff preview when available, otherwise the primary arg
    // (command / path / pattern) as a clean code-style snippet. Falls
    // back to a compact JSON view only when everything else is empty.
    if let Some(preview) = &pending.preview {
        for dl in preview.lines.iter().take(24) {
            out.push(indented_diff_line(dl));
        }
        if preview.lines.len() > 24 {
            out.push(Line::from(Span::styled(
                format!("     … {} more diff lines", preview.lines.len() - 24),
                Style::default().fg(MUTED()).italic(),
            )));
        }
    } else if !primary.is_empty() {
        // A shell command / path / URL, whatever `summarize_tool` picked.
        // Keep it on one line — the outer Paragraph::wrap handles reflow
        // gracefully at any terminal width.
        out.push(Line::from(vec![
            Span::styled("     $  ", Style::default().fg(DIM())),
            Span::styled(primary, Style::default().fg(CREAM())),
        ]));
    } else {
        for l in pretty_args(args).lines().take(6) {
            out.push(Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(l.to_owned(), Style::default().fg(CREAM())),
            ]));
        }
    }
    out.push(Line::from(""));

    // Key badges — filled backgrounds so they look like buttons.
    out.push(Line::from(vec![
        Span::styled("     ", Style::default()),
        Span::styled(
            " y ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  approve       ", Style::default().fg(CREAM())),
        Span::styled(
            " a ",
            Style::default()
                .fg(Color::Black)
                .bg(SALMON())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  always this session       ", Style::default().fg(CREAM())),
        Span::styled(
            " n ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  deny", Style::default().fg(CREAM())),
    ]));

    // Bottom rule closes the block.
    out.push(rule_line());

    out
}

/// Horizontal rule used to bracket the approval block. Long enough
/// for most terminal widths; ratatui's wrap won't split it because
/// it's a single Span and the transcript uses `Wrap { trim: false }`.
fn rule_line() -> Line<'static> {
    Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────────",
        Style::default().fg(DIM()),
    ))
}

/// Owned-string diff line for the inline approval block. `diff_line`
/// below returns a borrowed `Line<'_>` for the modal path and doesn't
/// fit `Vec<Line<'static>>`; this version clones + indents in one go.
fn indented_diff_line(d: &DiffLine) -> Line<'static> {
    match d {
        DiffLine::Ctx(s) => Line::from(vec![
            Span::raw("   "),
            Span::styled(s.clone(), Style::default().fg(MUTED())),
        ]),
        DiffLine::Add(s) => Line::from(vec![
            Span::styled("  +", Style::default().fg(Color::Green).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Green)),
        ]),
        DiffLine::Del(s) => Line::from(vec![
            Span::styled("  -", Style::default().fg(Color::Red).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Red)),
        ]),
        DiffLine::HunkGap => Line::from(Span::styled("    ⋯", Style::default().fg(DIM()).italic())),
    }
}

/// Prepend a soft blue `│ ` gutter to every line — used to mark
/// assistant text as "planning" output when the session is in
/// [`Mode::Plan`]. Stateless, so tests + future callers can reuse it.
fn plan_gutter(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let gutter_style = Style::default().fg(Color::LightBlue);
    lines
        .into_iter()
        .map(|l| {
            let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", gutter_style)];
            spans.extend(l.spans);
            Line::from(spans)
        })
        .collect()
}

fn mode_style(state: &TuiState) -> Style {
    use mira_policy::Mode::*;
    let color = match state.mode {
        Plan => Color::Blue,
        Manual => Color::Green,
        Auto => Color::Yellow,
        Edit => Color::LightYellow,
        Yolo => Color::Red,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// Append a subtle `▍` cursor to the last non-empty line of a
/// streaming assistant entry. Uses the salmon accent + bold so the
/// eye can see progress even during a silent provider pause. Empty
/// input (assistant just started) gets a single cursor row so the
/// user isn't looking at nothing.
fn append_streaming_cursor(lines: &mut Vec<Line<'static>>) {
    let cursor = Span::styled(
        "▍",
        Style::default().fg(SALMON()).add_modifier(Modifier::BOLD),
    );
    if let Some(last) = lines.iter_mut().rev().find(|l| !line_is_empty(l)) {
        last.spans.push(Span::raw(" "));
        last.spans.push(cursor);
        return;
    }
    lines.push(Line::from(cursor));
}

/// Pad `s` on the right with spaces so its char-count reaches `width`,
/// or truncate with `…` if it already exceeds. Used by the status bar
/// so a solid-color background (error flash) covers the whole row —
/// otherwise the terminal shows the transcript underneath the tail
/// end of the row.
fn pad_to_width(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count == width {
        return s.to_owned();
    }
    if count > width {
        let cut: String = s.chars().take(width.saturating_sub(1)).collect();
        return format!("{cut}…");
    }
    let mut out = s.to_owned();
    for _ in count..width {
        out.push(' ');
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

fn pretty_args(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| raw.to_owned())
}
