//! Tool-call components — the compact `● Read src/main.rs` group header,
//! batching of same-family calls, and the family summarizer.
//!
//! A `ToolView` decides its own visual representation from its status
//! (in-flight / ok / failed), so new tool UI states attach here rather
//! than in the transcript layer.

use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};

use crate::tui::state::AgentCell;
use super::tool_result;
use super::{highlight_line, truncate, CREAM, DIM, MUTED, SALMON};

/// One tool call (optionally paired with its result) as the renderer
/// sees it. Borrows from the state entries.
pub(crate) struct ToolView<'a> {
    pub name: &'a str,
    pub args: &'a str,
    /// Diff preview computed at approval time (or, in auto-approve
    /// modes, when the tool call arrived). Rendered after a successful
    /// Edit/Write so the user sees exactly what changed.
    pub preview: Option<&'a mira_tools::DiffPreview>,
    pub result: Option<tool_result::ResultView<'a>>,
    /// Wall-clock elapsed for in-flight calls — drives the `· 1.2s`
    /// ticker so long reads / bash calls stop looking hung. `None`
    /// once a result has landed.
    pub elapsed: Option<std::time::Duration>,
    /// True for the most-recent successful undoable write — renders
    /// the `/undo to revert` chip pointing at what `/undo` rolls back.
    pub undoable: bool,
    /// Header-only rendering: tool groups from turns before the last
    /// user prompt auto-collapse so long sessions read as a narrative.
    /// The undo chip stays visible — it's the one actionable affordance.
    pub collapsed: bool,
    /// Newest output line of the in-flight call (ToolProgress live
    /// tail) — renders dim under the `◐` header so a 90-second
    /// command doesn't look hung. `None` once a result lands.
    pub tail: Option<String>,
    /// Live cell data for `agent` tool calls — child tool uses,
    /// progress notes, done status. `None` for all other tools.
    pub agent_cell: Option<&'a AgentCell>,
}

/// A batch of same-family call/result pairs, collapsed into one block.
pub(crate) struct BatchView {
    /// Friendly family name from [`summarize_tool`] (e.g. "Read").
    pub family: String,
    /// One arg summary per call.
    pub summaries: Vec<String>,
    /// Header-only rendering for batches from earlier turns.
    pub collapsed: bool,
}

/// Compact Claude-style block for a tool call + result:
///
///     ● Read src/main.rs
///       └  fn main() { println!("hi") ...
///
/// Expanded form ("ctrl+e") replaces the `└  snippet` line with the
/// full output prefixed by a `│` gutter. When called for an in-flight
/// call (no result yet), the header shows a dim spinner mark.
pub(crate) fn render(
    v: &ToolView<'_>,
    query: &str,
    focused: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let (label, summary) = summarize_tool(v.name, v.args);
    let (mark_glyph, mark_color) = match v.result {
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
    if let Some(p) = v.preview {
        let (adds, dels) = tool_result::diff_stats(p);
        header.push(Span::styled(
            format!("  +{adds}"),
            Style::default().fg(Color::Green).bold(),
        ));
        header.push(Span::styled(
            format!(" -{dels}"),
            Style::default().fg(Color::Red).bold(),
        ));
    }
    // In-flight elapsed ticker — only when there's no result yet.
    // Render pass fires every 100ms while streaming, so this updates
    // continuously with no extra timer plumbing. Sub-second calls stay
    // silent to avoid flicker on the common fast-path.
    if let Some(d) = v.elapsed {
        let secs = d.as_secs_f32();
        if secs >= 0.6 {
            header.push(Span::styled(
                format!(" · {}", format_elapsed(secs)),
                Style::default().fg(MUTED()),
            ));
        }
    }
    // Model caption inline on the header for agent calls — dim so it
    // doesn't compete with the prompt text, but saves a whole row vs a
    // separate line.
    if let Some(cell) = v.agent_cell {
        if let Some(model) = &cell.model {
            header.push(Span::styled("  ·  ", Style::default().fg(DIM())));
            header.push(Span::styled(model_short(model), Style::default().fg(DIM())));
        }
    }
    // Expandability is signaled implicitly by the truncated `└` snippet
    // below the header — no chevron. The previous `⌄` suffix was getting
    // orphaned by `Paragraph::wrap` when the header exceeded terminal
    // width (ratatui broke on the leading whitespace and dropped the
    // lone glyph onto its own row, reading as a stray `;`-ish mark in
    // fonts without U+2304). `Ctrl+E` is still discoverable via `/help`.

    let mut out: Vec<Line<'static>> = vec![Line::from(header)];

    // Live tail: newest stdout line from the still-running call. One
    // dim row — enough to tell "hung" from "working", never competing
    // with the full output that lands on completion.
    if v.result.is_none() {
        if let Some(line) = &v.tail {
            out.push(Line::from(vec![
                Span::styled("  └ ⋯ ", Style::default().fg(super::DIM())),
                Span::styled(line.clone(), Style::default().fg(MUTED())),
            ]));
        }
    }

    // Body: prefer the diff preview when the call succeeded — that's
    // the feedback loop users actually want ("what did the edit
    // change?"). Fall back to the tool's own output otherwise.
    // Auto-collapsed groups (older turns) render header-only.
    if !v.collapsed {
        let show_diff = v.preview.is_some() && v.result.map(|r| r.ok).unwrap_or(false);
        if show_diff {
            let p = v.preview.expect("checked above");
            out.extend(tool_result::diff_preview_lines(p, width));
        } else if let Some(r) = v.result.as_ref() {
            // Pass the file path so read_file / view_file results get
            // language-aware syntax highlighting instead of the generic
            // heuristic.
            let file_path = match v.name {
                "read_file" | "view_file" => Some(summary.as_str()),
                _ => None,
            };
            out.extend(tool_result::body_lines(r, file_path));
        }
    }

    // Agent cell: nested child tool uses and progress notes.
    if let Some(cell) = v.agent_cell {
        // Live streaming tail — the last line the subagent was generating
        // before it called a tool or finished. Shows what the model was
        // "thinking" and clears the moment a tool fires. Never shown once
        // the agent is done.
        if !cell.done && !cell.streaming_text.trim().is_empty() {
            let last = cell
                .streaming_text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            if !last.is_empty() {
                out.push(Line::from(vec![
                    Span::styled("  ↳ ", Style::default().fg(DIM())),
                    Span::styled(
                        truncate(last, 90),
                        Style::default().fg(super::DIM()).italic(),
                    ),
                ]));
            }
        }
        // Live progress note — only while the agent is still running.
        // Cleared visually on completion so it doesn't linger as stale text.
        if !cell.done {
            if let Some(note) = cell.progress.last() {
                out.push(Line::from(vec![
                    Span::styled("  ↻ ", Style::default().fg(DIM())),
                    Span::styled(note.clone(), Style::default().fg(MUTED()).italic()),
                ]));
            }
        }
        // Child tool uses — styled like mini tool calls so they read
        // consistently with the parent transcript (salmon label, cream
        // summary, colored status dot).
        if !cell.tool_uses.is_empty() {
            let last_idx = cell.tool_uses.len().saturating_sub(1);
            for (i, tu) in cell.tool_uses.iter().enumerate() {
                let is_last = i == last_idx && cell.done;
                let connector = if is_last { "  └  " } else { "  ├  " };
                let (dot, dot_color) = match tu.ok {
                    Some(true) => ("● ", Color::Green),
                    Some(false) => ("● ", Color::Red),
                    None => ("◐ ", SALMON()),
                };
                let mut spans = vec![
                    Span::styled(connector, Style::default().fg(DIM())),
                    Span::styled(dot, Style::default().fg(dot_color)),
                    Span::styled(tu.label.clone(), Style::default().fg(SALMON())),
                ];
                if !tu.summary.is_empty() {
                    spans.push(Span::styled(" ", Style::default()));
                    spans.push(Span::styled(
                        truncate(&tu.summary, 80),
                        Style::default().fg(CREAM()),
                    ));
                }
                out.push(Line::from(spans));
            }
        }
        // Warnings chip.
        if cell.warnings > 0 {
            out.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("!{} warning{}", cell.warnings, if cell.warnings == 1 { "" } else { "s" }),
                    Style::default().fg(Color::Yellow).bold(),
                ),
            ]));
        }
    }

    // Undo affordance on the most-recent successful write. Rendered as
    // a small dim chip so it points at `/undo` without shouting for
    // attention on every edit.
    if v.undoable {
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

/// Collapsed multi-call rendering, Claude-style. `family` is the
/// friendly name from `summarize_tool` (e.g. "Read"). Renders as:
///
///     ● Reading 3 files
///       └  src/main.rs
///       └  src/lib.rs
///       └  src/foo.rs
pub(crate) fn render_batch(v: &BatchView) -> Vec<Line<'static>> {
    let (verb, noun) = batch_verb_noun(&v.family);
    let n = v.summaries.len();
    let header = Line::from(vec![
        Span::styled("● ", Style::default().fg(Color::Green).bold()),
        Span::styled(
            format!("{verb} {n} {noun}"),
            Style::default().fg(SALMON()).bold(),
        ),
    ]);

    let mut out = vec![header];
    if v.collapsed {
        return out;
    }
    let last_idx = v.summaries.len().saturating_sub(1);
    for (i, summary) in v.summaries.iter().enumerate() {
        let body = if summary.is_empty() {
            "(no arg)".to_owned()
        } else {
            truncate(summary, 140)
        };
        // `├` on every row except the last, `└` on the last — a proper
        // tree so the eye reads the group as one thing rather than
        // three disconnected `└` corners stacked on top of each other.
        let connector = if i == last_idx { "  └  " } else { "  ├  " };
        out.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(super::DIM())),
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
        "Agent" => ("Spawning", "subagents"),
        _ => ("Calling", "tools"),
    }
}

/// Extract a friendly name + one-line arg summary from a tool call so we
/// can render `Read src/foo.rs` instead of `read_file({"path":"..."})`.
/// Unknown tools keep their raw name and get the first string-valued arg
/// as a summary.
pub(crate) fn summarize_tool(name: &str, args: &str) -> (String, String) {
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
        "agent" => {
            let type_prefix = get("type")
                .filter(|t| !t.is_empty())
                .map(|t| format!("({t}) "))
                .unwrap_or_default();
            let prompt = one_line(get("prompt").unwrap_or_default());
            ("Agent".to_owned(), format!("{type_prefix}{prompt}"))
        }
        "plan" => ("Plan".to_owned(), get("title").unwrap_or_default()),
        "ask_user" => ("Ask".to_owned(), {
            // First question text as the summary — that's the thing
            // the user is being asked about.
            v.get("questions")
                .and_then(|qs| qs.get(0))
                .and_then(|q| q.get("question"))
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_owned()
        }),
        "task_create" => ("Task".to_owned(), get("subject").unwrap_or_default()),
        "task_get" => (
            "Task".to_owned(),
            v.get("task_id")
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_u64())
                .map(|id| format!("#{id}"))
                .unwrap_or_default(),
        ),
        "task_update" => {
            let id = v
                .get("task_id")
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_u64())
                .map(|id| format!("#{id}"))
                .unwrap_or_default();
            // Prefer the status transition in the summary — "#3 →
            // in_progress" reads as an event, which is what it is.
            let summary = match get("status") {
                Some(status) => format!("{id} → {status}"),
                None => id,
            };
            ("Task".to_owned(), summary)
        }
        "task_list" => ("Tasks".to_owned(), String::new()),
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

/// True when `name` is one of the tools that mira-tools journals into
/// `.mira/.undo/` — i.e. exactly the set `/undo` can roll back. Same
/// list `summarize_tool` gives an "Edit" / "Write" label to.
pub(crate) fn is_undoable_tool(name: &str) -> bool {
    matches!(
        name,
        "edit_file" | "write_file" | "apply_patch" | "create_file"
    )
}

/// Index of the last entry that's an undoable, *successful* tool call.
/// Scans backwards, stops at the first paired `(call, result)` where
/// the tool is a writer and the result is ok. Returns `None` when no
/// such call is in the visible transcript.
pub(crate) fn last_undoable_call_idx(entries: &[crate::tui::state::LogEntry]) -> Option<usize> {
    use crate::tui::state::LogEntry;
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

/// Strip the `claude-` prefix and trailing date suffix from a model id
/// for the inline header caption. `claude-haiku-4-5-20251001` → `haiku-4-5`.
fn model_short(model: &str) -> String {
    let s = model.strip_prefix("claude-").unwrap_or(model);
    // Strip trailing `-YYYYMMDD` (8 digits preceded by a dash).
    if s.len() > 9 {
        let tail = &s[s.len() - 8..];
        if tail.chars().all(|c| c.is_ascii_digit()) {
            return s[..s.len() - 9].to_owned();
        }
    }
    s.to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_known_tools() {
        assert_eq!(
            summarize_tool("read_file", r#"{"path":"src/main.rs"}"#),
            ("Read".to_owned(), "src/main.rs".to_owned())
        );
        assert_eq!(
            summarize_tool("shell", r#"{"cmd":"cargo check\nmore"}"#),
            ("Bash".to_owned(), "cargo check".to_owned())
        );
        assert_eq!(
            summarize_tool("edit_file", r#"{"target":"x.rs"}"#),
            ("Edit".to_owned(), "x.rs".to_owned())
        );
    }

    #[test]
    fn summarize_unknown_keeps_raw_name_and_first_string_arg() {
        assert_eq!(
            summarize_tool("weird_tool", r#"{"note":"hello world"}"#),
            ("weird_tool".to_owned(), "hello world".to_owned())
        );
        // Malformed args don't panic.
        assert_eq!(summarize_tool("weird_tool", "not json").0, "weird_tool");
    }

    #[test]
    fn batch_verb_noun_falls_back() {
        assert_eq!(batch_verb_noun("Read"), ("Reading", "files"));
        assert_eq!(batch_verb_noun("??"), ("Calling", "tools"));
    }

    #[test]
    fn undoable_scan_finds_last_writer_pair() {
        use crate::tui::state::LogEntry;
        use std::time::Instant;
        let entries = vec![
            LogEntry::ToolCall {
                name: "edit_file".into(),
                args: "{}".into(),
                preview: None,
                call_id: "1".into(),
                started_at: Instant::now(),
            },
            LogEntry::ToolResult {
                ok: true,
                snippet: "ok".into(),
                full: String::new(),
                expanded: false,
                collapsed_override: None,
            },
            LogEntry::ToolCall {
                name: "read_file".into(),
                args: "{}".into(),
                preview: None,
                call_id: "2".into(),
                started_at: Instant::now(),
            },
            LogEntry::ToolResult {
                ok: true,
                snippet: "ok".into(),
                full: String::new(),
                expanded: false,
                collapsed_override: None,
            },
            LogEntry::ToolCall {
                name: "write_file".into(),
                args: "{}".into(),
                preview: None,
                call_id: "3".into(),
                started_at: Instant::now(),
            },
            LogEntry::ToolResult {
                ok: false,
                snippet: "denied".into(),
                full: String::new(),
                expanded: false,
                collapsed_override: None,
            },
        ];
        // The failed write at the tail doesn't qualify; the earlier
        // successful edit does.
        assert_eq!(last_undoable_call_idx(&entries), Some(0));
    }

    #[test]
    fn elapsed_ticker_format() {
        assert_eq!(format_elapsed(1.4), "1.4s");
        assert_eq!(format_elapsed(12.0), "12s");
        assert_eq!(format_elapsed(63.0), "1m03s");
    }
}
