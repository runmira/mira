//! Status components — the animated working indicator, the turn-end
//! full-stop, and the Goal panel.
//!
//! The Goal panel is Mira's "goal-directed agent" signature: instead of
//! the standing goal scrolling past as `[goal]` info lines, it renders
//! as a persistent card pinned under the header for the whole run —
//! condition, loop progress, and the evaluator's latest reason — and
//! flips to a terminal state the moment the goal is met (or blocked).

use ratatui::style::{Stylize, Color, Modifier, Style};
use ratatui::text::{Line, Span};

use mira_harness::{Goal, GoalStatus};

use super::{CREAM, LOGO, MUTED, SALMON};

/// The tool call currently executing, resolved at block-build time —
/// the working indicator names the actual work instead of a generic
/// verb while a tool runs.
#[derive(Clone)]
pub struct InFlightTool {
    /// Friendly family label ("Read", "Bash", …).
    pub label: String,
    /// One-line arg summary ("src/main.rs", "cargo check").
    pub summary: String,
    /// Seconds since this specific call started (the header ticker's
    /// clock, not the turn's).
    pub elapsed_secs: f32,
}

/// Data for the ephemeral working indicator, computed at block-build
/// time so the renderer stays pure.
#[derive(Clone)]
pub struct StatusView {
    /// Seconds since the provider stream started.
    pub elapsed_secs: f32,
    /// Completion tokens produced this turn (delta over the baseline).
    pub tokens: u64,
    /// Tool call in flight, if any. `None` while the model is
    /// thinking/streaming with nothing running — the heartbeat form.
    pub tool: Option<InFlightTool>,
}

/// Pulse the brand mark's color along a sine wave so the streaming
/// indicator "breathes". Elapsed-time driven — the render loop
/// already re-draws every 100ms while streaming, so this animates
/// for free.
pub(crate) fn pulsed_logo(secs: f32) -> Color {
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

/// Rotate through a small vocabulary of streaming verbs based on
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

/// The in-transcript working indicator — replaces the status-bar
/// spinner so users see it inline with the assistant reply (Claude
/// Code's convention). Two forms:
///
///     ℳ Wrangling… (12s · ↓ 1.2k tokens · esc to interrupt)   ← thinking
///     ◐ Reading src/main.rs · 3.2s · esc to interrupt          ← tool running
///
/// The tool form names the actual work (the thing people scan for
/// during a long call) and keeps the breathing glyph for liveness.
pub(crate) fn working_line(v: &StatusView) -> Line<'static> {
    let secs = v.elapsed_secs;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut closing = "";
    match &v.tool {
        Some(t) => {
            spans.push(Span::styled(
                "◐ ",
                Style::default().fg(pulsed_logo(secs)).bold(),
            ));
            spans.push(Span::styled(t.label.clone(), Style::default().fg(SALMON()).bold()));
            if !t.summary.is_empty() {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    super::truncate(&t.summary, 100),
                    Style::default().fg(CREAM()),
                ));
            }
            // The tool's own clock, gated like the header ticker so
            // fast calls don't flicker a time that's already gone.
            if t.elapsed_secs >= 0.6 {
                spans.push(Span::styled(
                    format!(" · {}", fmt_secs(t.elapsed_secs)),
                    Style::default().fg(MUTED()),
                ));
            }
        }
        None => {
            closing = ")";
            spans.push(Span::styled(
                format!("{LOGO} "),
                Style::default().fg(pulsed_logo(secs)).bold(),
            ));
            spans.push(Span::styled(
                streaming_label(secs),
                Style::default().fg(SALMON()).bold(),
            ));
            spans.push(Span::styled(
                format!("… ({}", fmt_secs(secs)),
                Style::default().fg(MUTED()),
            ));
        }
    }
    if v.tokens > 0 {
        spans.push(Span::styled(
            format!(" · ↓{} tokens", short_num(v.tokens)),
            Style::default().fg(MUTED()),
        ));
    }
    spans.push(Span::styled(
        format!(" · esc to interrupt{closing}"),
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

/// What one turn did, captured at push time — the receipt data on the
/// `✳` marker.
pub struct TurnEndView {
    pub elapsed_ms: u32,
    pub files: u16,
    pub adds: u32,
    pub dels: u32,
    pub tools: u16,
    pub cost: Option<f64>,
}

/// Turn-end marker rendered under the assistant reply — a quiet
/// full-stop that doubles as the turn's receipt:
///
///     ✳ Baked for 12.4s
///     ✳ Baked for 34.1s · 3 files +18 −4 · 7 tools · $0.021
///
/// Muted italic so it reads as a full stop, not a headline. Verb is
/// picked from [`turn_verb_past`] against the millisecond bucket so
/// the same duration always reads the same word.
pub(crate) fn turn_end_lines(v: &TurnEndView) -> Vec<Line<'static>> {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("✳ ", Style::default().fg(SALMON())),
        Span::styled(
            turn_end_label(v.elapsed_ms),
            Style::default().fg(MUTED()).italic(),
        ),
    ];
    if v.files > 0 {
        spans.push(Span::styled(
            format!(
                " · {} file{}",
                v.files,
                if v.files == 1 { "" } else { "s" }
            ),
            Style::default().fg(CREAM()),
        ));
        if v.adds > 0 || v.dels > 0 {
            spans.push(Span::styled(
                format!(" +{}", v.adds),
                Style::default().fg(ratatui::style::Color::Green),
            ));
            spans.push(Span::styled(
                format!(" −{}", v.dels),
                Style::default().fg(ratatui::style::Color::Red),
            ));
        }
    }
    if v.tools > 0 {
        spans.push(Span::styled(
            format!(
                " · {} tool{}",
                v.tools,
                if v.tools == 1 { "" } else { "s" }
            ),
            Style::default().fg(MUTED()),
        ));
    }
    if let Some(c) = v.cost {
        let label = if c < 0.01 {
            format!("${:.4}", c)
        } else {
            format!("${:.2}", c)
        };
        spans.push(Span::styled(
            format!(" · {label}"),
            Style::default().fg(MUTED()).italic(),
        ));
    }
    vec![Line::from(spans)]
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
pub(crate) fn format_turn_elapsed(ms: u32) -> String {
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

pub(crate) fn short_num(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    }
}

// ---- Goal panel ----

/// Rows the goal panel occupies (0 when no goal is set): rule +
/// condition + status, plus one spacer row so the transcript below
/// keeps its breathing room.
pub(crate) fn goal_rows(goal: Option<&Goal>) -> u16 {
    if goal.is_some() { 4 } else { 0 }
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

/// The persistent Goal card. Shape:
///
///     GOAL ─────────────────────────────────────
///     Implement authentication
///     ▶ running · iteration 2/10 · still working — tests failing
///
/// and when the loop resolves:
///
///     GOAL ─────────────────────────────────────
///     Implement authentication
///     ✓ goal met — all tests passing
///
/// Rendered under the header every frame a goal exists, so it stays
/// visible no matter how far the transcript has scrolled.
pub(crate) fn goal_panel_lines(goal: &Goal, pulse_secs: f32) -> Vec<Line<'static>> {
    let rule_style = Style::default().fg(super::HAIRLINE());
    let mut out = vec![
        Line::from(vec![
            Span::styled(" GOAL ", Style::default().fg(SALMON()).bold()),
            Span::styled("─────────────────────────────────────────────────────", rule_style),
        ]),
        Line::from(Span::styled(
            format!(" {}", super::truncate(&goal.condition, 120)),
            Style::default().fg(CREAM()).bold(),
        )),
    ];

    let (chip, chip_style) = goal_chip(goal.status);
    let mut status_spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    if goal.status == GoalStatus::Active {
        // The running marker breathes with the same heartbeat as the
        // streaming indicator — subtle "agent at work" animation.
        status_spans.push(Span::styled(
            format!("{LOGO} "),
            Style::default().fg(pulsed_logo(pulse_secs)).bold(),
        ));
        status_spans.push(Span::styled("running".to_owned(), chip_style));
    } else {
        status_spans.push(Span::styled(chip.to_owned(), chip_style));
    }
    status_spans.push(Span::styled(
        format!(" · iteration {}/{}", goal.iterations, goal.max_iterations),
        Style::default().fg(MUTED()),
    ));
    if let Some(reason) = goal.last_reason.as_deref() {
        let reason = reason.trim();
        if !reason.is_empty() {
            status_spans.push(Span::styled(
                format!(" — {}", super::truncate(reason, 100)),
                Style::default().fg(MUTED()).italic(),
            ));
        }
    }
    out.push(Line::from(status_spans));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_end_label_is_deterministic() {
        assert_eq!(turn_end_label(4200), turn_end_label(4200));
        assert!(turn_end_label(6400).contains("6.4s"));
        assert!(turn_end_label(42_000).ends_with("42s"));
        assert!(turn_end_label(67_000).ends_with("1m 07s"));
    }

    #[test]
    fn goal_rows_zero_without_goal() {
        assert_eq!(goal_rows(None), 0);
        let g = Goal::new("ship it");
        assert_eq!(goal_rows(Some(&g)), 4);
    }

    #[test]
    fn goal_panel_shows_condition_and_iterations() {
        let mut g = Goal::new("Implement authentication");
        g.iterations = 2;
        g.max_iterations = 10;
        g.last_reason = Some("tests still failing".into());
        let ls = goal_panel_lines(&g, 0.0);
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("GOAL"));
        assert!(joined.contains("Implement authentication"));
        assert!(joined.contains("2/10"));
        assert!(joined.contains("tests still failing"));
    }

    #[test]
    fn working_line_names_in_flight_tool() {
        let v = StatusView {
            elapsed_secs: 12.0,
            tokens: 1_900,
            tool: Some(InFlightTool {
                label: "Read".into(),
                summary: "src/main.rs".into(),
                elapsed_secs: 3.2,
            }),
        };
        let line = working_line(&v);
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.contains("◐ Read src/main.rs"), "{text}");
        assert!(text.contains("3.2s"), "tool's own clock: {text}");
        assert!(!text.contains("12s"), "turn clock must not shadow tool: {text}");
        assert!(text.contains("↓1.9k tokens"));
        assert!(text.contains("esc to interrupt"));
        assert!(!text.contains("… ("), "no heartbeat form while a tool runs");
    }

    #[test]
    fn working_line_without_tool_keeps_heartbeat_form() {
        let v = StatusView {
            elapsed_secs: 12.0,
            tokens: 0,
            tool: None,
        };
        let line = working_line(&v);
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.starts_with('ℳ'), "{text}");
        assert!(text.contains("… (12s"), "{text}");
        assert!(text.ends_with("esc to interrupt)"), "{text}");
    }

    #[test]
    fn working_line_fast_tool_hides_ticker() {
        let v = StatusView {
            elapsed_secs: 5.0,
            tokens: 0,
            tool: Some(InFlightTool {
                label: "Bash".into(),
                summary: "ls".into(),
                elapsed_secs: 0.2,
            }),
        };
        let text: String = working_line(&v).spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.contains("◐ Bash ls"), "{text}");
        assert!(!text.contains("0.2s"), "{text}");
    }

    #[test]
    fn short_num_scales() {
        assert_eq!(short_num(999), "999");
        assert_eq!(short_num(1_234), "1.2k");
        assert_eq!(short_num(2_500_000), "2.50M");
    }
}
