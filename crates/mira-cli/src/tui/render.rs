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

// ---- Claude-style palette ----
//
// Warm salmon / cream + neutral grays. Kept as `const` RGBs (not
// `Style` constants) so callers can freely combine them with modifiers
// per-span. Truecolor-only — 16-color terminals fall back to their
// closest match automatically via ratatui/crossterm.
const SALMON: Color = Color::Rgb(232, 156, 104);
const CREAM: Color = Color::Rgb(226, 210, 182);
const MUTED: Color = Color::Rgb(140, 130, 118);
const DIM: Color = Color::Rgb(96, 90, 82);

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
    let input_rows = input_display_rows(state.input()).min(6);
    let input_height = input_rows + 2; // + top/bottom border

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .split(f.area());

    header(f, root[0], state);
    transcript(f, root[1], state);
    input(f, root[2], state);
    status(f, root[3], state);

    // Overlays: palette above the input, approval modal centered.
    if state.palette.kind != Palette::None && !state.palette.matches.is_empty() {
        palette(f, root[2], state);
    }
    if state.search.is_some() {
        search_overlay(f, root[2], state);
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
            Style::default().fg(SALMON).bold(),
        ),
        Span::styled("· ", Style::default().fg(DIM)),
        Span::styled(state.model.as_str(), Style::default().fg(CREAM)),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(state.mode.as_str(), mode_style(state)),
    ];
    if let Some(branch) = state.git_branch.as_deref() {
        spans.push(Span::styled(" · ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            format!("⎇ {branch}"),
            Style::default().fg(MUTED),
        ));
    }
    if let Some(g) = state.goal.as_ref() {
        let (label, chip_style) = goal_chip(g.status);
        spans.push(Span::styled(" · ", Style::default().fg(DIM)));
        spans.push(Span::styled("goal ", chip_style));
        spans.push(Span::styled(
            format!("{}/{}", g.iterations, g.max_iterations),
            Style::default().fg(MUTED),
        ));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(label, chip_style));
        // Trim the condition inline so it doesn't blow past the header.
        spans.push(Span::styled(" · ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            truncate(&g.condition, 60),
            Style::default().fg(CREAM),
        ));
    }
    let left = Line::from(spans);

    // Usage chip on the right side of the header, so the bottom status
    // row can stay purely a hint/flash area.
    let usage_text = format_usage(state);
    if usage_text.is_empty() {
        f.render_widget(Paragraph::new(left), area);
        return;
    }
    let right_len = usage_text.chars().count() as u16;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_len)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            usage_text,
            Style::default().fg(MUTED),
        )))
        .alignment(Alignment::Right),
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

    let mut lines: Vec<Line> = Vec::new();
    if state.dropped_entries > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  … {} earlier entr{} truncated",
                state.dropped_entries,
                if state.dropped_entries == 1 { "y" } else { "ies" },
            ),
            Style::default().fg(MUTED).italic(),
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
                    Some(LogEntry::ToolResult { expanded: false, .. }) => {
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
            let g = tool_group_lines(
                name,
                args,
                result,
                preview.as_ref(),
                &query,
                hit_this_entry || hit_next,
            );
            (g, if paired { 2 } else { 1 })
        } else {
            let mut ls = entry_to_lines_highlight(entry, &query, hit_this_entry);
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
        if i + consumed < entries.len() {
            lines.push(Line::from(""));
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
        lines.push(Line::from(Span::styled(
            "  (empty. type a message and press enter.)",
            Style::default().fg(MUTED),
        )));
    }

    let text = Text::from(lines);
    let para = Paragraph::new(text).wrap(Wrap { trim: false });

    // ratatui 0.29 exposes wrapped line count, which lets us compute the
    // tail offset and clamp scroll into a valid range.
    let total = para.line_count(area.width) as u16;
    let tail = total.saturating_sub(area.height);

    // Search override: scroll to the hit's entry so it lands in view.
    // Falls back to follow-tail behavior when no active hit.
    let scroll = if let Some((entry_idx, _)) = active_hit {
        let start = entry_row_starts
            .get(entry_idx)
            .copied()
            .unwrap_or(0) as u16;
        start
            .saturating_sub(area.height / 3)
            .min(tail)
    } else if state.follow_tail {
        tail
    } else {
        state.scroll.min(tail)
    };

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
                Style::default().fg(SALMON).bold(),
            )))
            .alignment(Alignment::Right),
            chip,
        );
    }

    f.render_widget(para.scroll((scroll, 0)), area);
}

/// Render one entry with optional case-insensitive query highlighting.
/// The `focused` flag marks this entry as containing the *active* hit
/// (as opposed to any other match) — used to underline the row so the
/// user can tell which of N matches is current.
fn entry_to_lines_highlight(entry: &LogEntry, query: &str, focused: bool) -> Vec<Line<'static>> {
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
        LogEntry::Assistant(s) => markdown::render(s),
        LogEntry::ToolCall {
            name,
            args,
            preview,
            ..
        } => tool_group_lines(name, args, None, preview.as_ref(), "", false),
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
            tool_group_lines("(result)", "", Some(result), None, "", false)
        }
        LogEntry::Warning(s) => vec![Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Yellow)),
        ])],
        LogEntry::Info(s) => vec![Line::from(Span::styled(
            s.clone(),
            Style::default().fg(MUTED).italic(),
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
            Span::styled(mark, Style::default().fg(SALMON).bold()),
            Span::styled(line.to_owned(), Style::default().fg(CREAM)),
        ]));
        first = false;
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(
            "> ",
            Style::default().fg(SALMON).bold(),
        )));
    }
    out
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
    query: &str,
    focused: bool,
) -> Vec<Line<'static>> {
    let (label, summary) = summarize_tool(name, args);
    let (mark_glyph, mark_color) = match result {
        Some(r) if r.ok => ("● ", Color::Green),
        Some(_) => ("● ", Color::Red),
        None => ("◐ ", SALMON),
    };
    let mut header: Vec<Span<'static>> = vec![
        Span::styled(mark_glyph, Style::default().fg(mark_color).bold()),
        Span::styled(label, Style::default().fg(SALMON).bold()),
    ];
    if !summary.is_empty() {
        header.push(Span::styled(" ", Style::default()));
        header.push(Span::styled(
            truncate(&summary, 140),
            Style::default().fg(CREAM),
        ));
    }
    // Show `+N -M` diff stats when the call carries a preview so
    // successful edits get a scanable "how much changed" glance.
    if let Some(p) = preview {
        let (adds, dels) = diff_stats(p);
        header.push(Span::styled(
            format!("  +{adds}", ),
            Style::default().fg(Color::Green).bold(),
        ));
        header.push(Span::styled(
            format!(" -{dels}", ),
            Style::default().fg(Color::Red).bold(),
        ));
    }
    if let Some(r) = result {
        if !r.expanded && multi_line(r.full) {
            header.push(Span::styled(
                "  (ctrl+e to expand)",
                Style::default().fg(MUTED).italic(),
            ));
        } else if r.expanded {
            header.push(Span::styled(
                "  (ctrl+e to collapse)",
                Style::default().fg(MUTED).italic(),
            ));
        }
    }

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
                    Style::default().fg(MUTED).italic(),
                )));
            }
        }
        (_, None) => {}
        (_, Some(r)) if r.expanded && !r.full.is_empty() => {
            for line in r.full.lines() {
                out.push(Line::from(vec![
                    Span::styled("  │  ", Style::default().fg(DIM)),
                    Span::styled(line.to_owned(), Style::default().fg(MUTED)),
                ]));
            }
        }
        (_, Some(r)) => {
            let body = if r.snippet.is_empty() {
                "(no output)".to_owned()
            } else {
                truncate(r.snippet, 200)
            };
            out.push(Line::from(vec![
                Span::styled("  └  ", Style::default().fg(DIM)),
                Span::styled(body, Style::default().fg(MUTED)),
            ]));
        }
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
            Style::default().fg(SALMON).bold(),
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
            Span::styled("  └  ", Style::default().fg(DIM)),
            Span::styled(body, Style::default().fg(MUTED)),
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

fn multi_line(s: &str) -> bool {
    s.lines().count() > 1 || s.chars().count() > 200
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
        Span::styled(label, Style::default().fg(SALMON).bold()),
        Span::styled(
            format!("… ({}", fmt_secs(secs)),
            Style::default().fg(MUTED),
        ),
    ];
    if state.usage.completion_tokens > 0 {
        spans.push(Span::styled(
            format!(" · ↓{} tokens", short_num(state.usage.completion_tokens)),
            Style::default().fg(MUTED),
        ));
    }
    spans.push(Span::styled(
        " · esc to interrupt)",
        Style::default().fg(MUTED),
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
    let get = |k: &str| -> Option<String> {
        v.get(k).and_then(|x| x.as_str()).map(|s| s.to_owned())
    };
    let one_line = |s: String| -> String {
        s.lines().next().unwrap_or("").to_owned()
    };
    match name {
        "read_file" | "view_file" => (
            "Read".to_owned(),
            get("path").unwrap_or_default(),
        ),
        "edit_file" | "apply_patch" => (
            "Edit".to_owned(),
            get("path").or_else(|| get("target")).unwrap_or_default(),
        ),
        "write_file" | "create_file" => (
            "Write".to_owned(),
            get("path").unwrap_or_default(),
        ),
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
        "web_fetch" | "fetch" => (
            "Fetch".to_owned(),
            get("url").unwrap_or_default(),
        ),
        "web_search" => (
            "Search".to_owned(),
            get("query").unwrap_or_default(),
        ),
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

fn input(f: &mut Frame, area: Rect, state: &TuiState) {
    let border_color = if state.streaming { MUTED } else { SALMON };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            " ▸▸ ",
            Style::default().fg(SALMON).bold(),
        ))
        // Bottom-right chip: `[edit · shift+tab to cycle]`. Matches
        // Claude's "accept edits on (shift+tab to cycle)" affordance.
        .title_bottom(
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(state.mode.chip_label(), mode_style(state)),
                Span::styled(
                    "  · shift+tab to cycle  ",
                    Style::default().fg(MUTED),
                ),
            ])
            .right_aligned(),
        );

    let body: Text = if state.input().is_empty() {
        Line::from(Span::styled(
            "message · enter send · ctrl+j newline · / cmd · @ file · esc esc quit",
            Style::default().fg(MUTED),
        ))
        .into()
    } else {
        // Split on '\n' rather than .lines() so a trailing newline shows
        // as an empty last row (the cursor lands there after Ctrl+J).
        let segments: Vec<Line> = state
            .input()
            .split('\n')
            .map(|l| Line::from(Span::styled(l.to_owned(), Style::default().fg(CREAM))))
            .collect();
        Text::from(segments)
    };
    f.render_widget(Paragraph::new(body).block(block), area);

    // Position the terminal caret at the cursor byte offset. Column is
    // char-based (breaks visually for wide CJK, but at least keys land
    // at the right byte). +1 skips the box border.
    let (row, col) = cursor_visual(state.input(), state.cursor());
    f.set_cursor_position((area.x + 1 + col, area.y + 1 + row));
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
        Palette::None => "",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SALMON))
        .title(Span::styled(title, Style::default().fg(SALMON).bold()));

    let items: Vec<ListItem> = state
        .palette
        .matches
        .iter()
        .map(|m| {
            let mut spans = vec![Span::styled(
                m.title.clone(),
                Style::default().fg(CREAM),
            )];
            if !m.detail.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    m.detail.clone(),
                    Style::default().fg(MUTED).italic(),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(DIM)
                .fg(SALMON)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.palette.cursor));
    f.render_stateful_widget(list, area, &mut list_state);
}

fn status(f: &mut Frame, area: Rect, state: &TuiState) {
    // Streaming indicator lives in the transcript and the usage chip
    // is in the header — the status row is pure hints/flash.
    let line: Line<'static> = if state.esc_pending {
        Line::from(Span::styled(
            "press esc again to quit",
            Style::default().fg(SALMON).bold(),
        ))
    } else if let Some(flash) = &state.flash {
        Line::from(Span::styled(
            flash.clone(),
            Style::default().fg(Color::Green).italic(),
        ))
    } else {
        Line::from(Span::styled(
            "enter send · / cmd · @ file · shift+tab mode · pgup/dn or wheel scroll · ctrl+r search · ctrl+e expand · esc esc quit",
            Style::default().fg(MUTED),
        ))
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Rotate through a small vocabulary of streaming-verbs based on
/// elapsed-time buckets. Keeps the status line feeling alive during
/// long silent gaps instead of just repeating "thinking".
fn streaming_label(secs: f32) -> &'static str {
    const WORDS: &[&str] = &[
        "Wrangling", "Thinking", "Pondering", "Cogitating", "Musing", "Simmering", "Brewing",
        "Percolating",
    ];
    let idx = ((secs / 4.0) as usize) % WORDS.len();
    WORDS[idx]
}

/// Compact right-side accounting: `↑12.3k ↓4.1k · 15% · $0.024`. Empty
/// when the provider hasn't reported any usage yet.
fn format_usage(state: &TuiState) -> String {
    let u = &state.usage;
    if u.is_zero() {
        return String::new();
    }
    let mut out = format!(
        "↑{} ↓{}",
        short_num(u.prompt_tokens),
        short_num(u.completion_tokens),
    );
    if let Some(ctx) = model_context_len(&state.model) {
        let pct = ((u.prompt_tokens as f64 / ctx as f64) * 100.0).min(999.0);
        out.push_str(&format!(" · {pct:.0}%"));
    }
    if let Some(dollars) = mira_ai::cost_usd(
        &state.model,
        mira_ai::TokenUsage {
            prompt_tokens: u.prompt_tokens.min(u32::MAX as u64) as u32,
            completion_tokens: u.completion_tokens.min(u32::MAX as u64) as u32,
            cached_input_tokens: u.cached_input_tokens.min(u32::MAX as u64) as u32,
        },
    ) {
        out.push_str(&format!(" · {}", format_dollars(dollars)));
    }
    out
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
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
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
        Span::styled(
            "  ⚠  ",
            Style::default().fg(Color::Yellow).bold(),
        ),
        Span::styled(
            friendly.clone(),
            Style::default().fg(SALMON).bold(),
        ),
        Span::styled(
            "  ·  approval required",
            Style::default().fg(CREAM),
        ),
    ];
    if let Some(preview) = &pending.preview {
        let kind_label = match preview.kind {
            DiffKind::Edit => "edit",
            DiffKind::Overwrite => "overwrite",
            DiffKind::Create => "create",
        };
        header.push(Span::styled("  ·  ", Style::default().fg(DIM)));
        header.push(Span::styled(
            preview.path.clone(),
            Style::default().fg(CREAM).bold(),
        ));
        header.push(Span::styled(
            format!("  ({kind_label})"),
            Style::default().fg(MUTED),
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
                Style::default().fg(MUTED).italic(),
            )));
        }
    } else if !primary.is_empty() {
        // A shell command / path / URL, whatever `summarize_tool` picked.
        // Keep it on one line — the outer Paragraph::wrap handles reflow
        // gracefully at any terminal width.
        out.push(Line::from(vec![
            Span::styled("     $  ", Style::default().fg(DIM)),
            Span::styled(primary, Style::default().fg(CREAM)),
        ]));
    } else {
        for l in pretty_args(args).lines().take(6) {
            out.push(Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(l.to_owned(), Style::default().fg(CREAM)),
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
        Span::styled("  approve       ", Style::default().fg(CREAM)),
        Span::styled(
            " a ",
            Style::default()
                .fg(Color::Black)
                .bg(SALMON)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  always this session       ",
            Style::default().fg(CREAM),
        ),
        Span::styled(
            " n ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  deny", Style::default().fg(CREAM)),
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
        Style::default().fg(DIM),
    ))
}

/// Owned-string diff line for the inline approval block. `diff_line`
/// below returns a borrowed `Line<'_>` for the modal path and doesn't
/// fit `Vec<Line<'static>>`; this version clones + indents in one go.
fn indented_diff_line(d: &DiffLine) -> Line<'static> {
    match d {
        DiffLine::Ctx(s) => Line::from(vec![
            Span::raw("   "),
            Span::styled(s.clone(), Style::default().fg(MUTED)),
        ]),
        DiffLine::Add(s) => Line::from(vec![
            Span::styled("  +", Style::default().fg(Color::Green).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Green)),
        ]),
        DiffLine::Del(s) => Line::from(vec![
            Span::styled("  -", Style::default().fg(Color::Red).bold()),
            Span::styled(s.clone(), Style::default().fg(Color::Red)),
        ]),
        DiffLine::HunkGap => Line::from(Span::styled(
            "    ⋯",
            Style::default().fg(DIM).italic(),
        )),
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
            let mut spans: Vec<Span<'static>> =
                vec![Span::styled("│ ", gutter_style)];
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
