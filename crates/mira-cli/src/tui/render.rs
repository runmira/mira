use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::state::{LogEntry, TuiState};

pub fn draw(f: &mut Frame, state: &TuiState) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    header(f, root[0], state);
    transcript(f, root[1], state);
    input(f, root[2], state);
    status(f, root[3], state);

    if state.pending_approval.is_some() {
        approval_modal(f, state);
    }
}

fn header(f: &mut Frame, area: Rect, state: &TuiState) {
    let line = Line::from(vec![
        Span::styled("mira ", Style::default().fg(Color::Magenta).bold()),
        Span::styled("· ", Style::default().fg(Color::DarkGray)),
        Span::styled(state.model.as_str(), Style::default().fg(Color::Cyan)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(state.mode.as_str(), mode_style(state)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn transcript(f: &mut Frame, area: Rect, state: &TuiState) {
    let mut lines: Vec<Line> = Vec::new();
    for entry in state.entries() {
        for l in entry_to_lines(entry) {
            lines.push(l);
        }
        lines.push(Line::from(""));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (empty. type a message and press enter.)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let text = Text::from(lines);
    let para = Paragraph::new(text).wrap(Wrap { trim: false });

    // ratatui 0.29 exposes wrapped line count, which lets us compute a
    // scroll offset that pins the tail in view when `follow_tail` is on.
    let total = para.line_count(area.width) as u16;
    let scroll = if state.follow_tail {
        total.saturating_sub(area.height)
    } else {
        state.scroll.min(total.saturating_sub(1))
    };
    f.render_widget(para.scroll((scroll, 0)), area);
}

fn entry_to_lines(entry: &LogEntry) -> Vec<Line<'_>> {
    match entry {
        LogEntry::User(s) => vec![Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::Cyan).bold()),
            Span::styled(s.as_str(), Style::default().fg(Color::Cyan)),
        ])],
        LogEntry::Assistant(s) => s
            .lines()
            .map(|l| Line::from(Span::raw(l.to_owned())))
            .collect(),
        LogEntry::ToolCall { name, args } => vec![Line::from(vec![
            Span::styled("▶ ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(name.clone(), Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("({})", truncate(args, 120)),
                Style::default().fg(Color::DarkGray),
            ),
        ])],
        LogEntry::ToolResult { ok, snippet } => {
            let (mark, color) = if *ok {
                ("✓ ", Color::Green)
            } else {
                ("✗ ", Color::Red)
            };
            vec![Line::from(vec![
                Span::styled(mark, Style::default().fg(color).bold()),
                Span::styled(snippet.clone(), Style::default().fg(Color::DarkGray)),
            ])]
        }
        LogEntry::Warning(s) => vec![Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(s.as_str(), Style::default().fg(Color::Yellow)),
        ])],
        LogEntry::Info(s) => vec![Line::from(Span::styled(
            s.as_str(),
            Style::default().fg(Color::DarkGray).italic(),
        ))],
    }
}

fn input(f: &mut Frame, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let text = if state.input().is_empty() {
        Line::from(Span::styled(
            "type a message · enter to send · esc esc to quit",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(vec![
            Span::raw(state.input().to_owned()),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
        ])
    };
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn status(f: &mut Frame, area: Rect, state: &TuiState) {
    let left = if state.streaming {
        "⏳ thinking · ctrl-c to interrupt"
    } else if state.esc_pending {
        "press esc again to quit"
    } else {
        "enter send · /help commands · esc esc quit"
    };
    let hint = if let Some(flash) = &state.flash {
        Span::styled(
            format!("  {flash}"),
            Style::default().fg(Color::Green).italic(),
        )
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::DarkGray)),
        hint,
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn approval_modal(f: &mut Frame, state: &TuiState) {
    let Some(req) = state.pending_approval.as_ref() else {
        return;
    };
    let area = centered(f.area(), 70, 40);
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " approve tool call ",
            Style::default().fg(Color::Yellow).bold(),
        ));

    let args = pretty_args(&req.call.function.arguments);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("tool: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                req.call.function.name.clone(),
                Style::default().fg(Color::Yellow).bold(),
            ),
        ]),
        Line::from(Span::styled("args:", Style::default().fg(Color::DarkGray))),
    ];
    for l in args.lines().take(12) {
        lines.push(Line::from(Span::raw(format!("  {l}"))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("y", Style::default().fg(Color::Green).bold()),
        Span::raw(" allow · "),
        Span::styled("n", Style::default().fg(Color::Red).bold()),
        Span::raw(" deny · "),
        Span::styled("esc", Style::default().fg(Color::DarkGray)),
        Span::raw(" deny"),
    ]));

    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left);
    f.render_widget(para, area);
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

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - pct_h) / 2),
        Constraint::Percentage(pct_h),
        Constraint::Percentage((100 - pct_h) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_w) / 2),
        Constraint::Percentage(pct_w),
        Constraint::Percentage((100 - pct_w) / 2),
    ])
    .split(v[1])[1]
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
