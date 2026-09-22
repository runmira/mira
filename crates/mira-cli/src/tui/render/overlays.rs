//! Overlays — floating surfaces anchored above the composer: the
//! slash/@/model/theme palette and the Ctrl+R search bar. They render
//! over the transcript's last rows and never take over the screen.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::tui::components::{CREAM, DIM, MUTED, SALMON};
use crate::tui::state::{Palette, TuiState};

/// Draw the palette as a floating list *above* the input area. Height
/// caps at 8 rows to avoid taking over the screen; wider palettes just
/// scroll internally with the selection cursor.
pub(crate) fn palette(f: &mut Frame, input_area: Rect, state: &TuiState) {
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

/// Small search bar rendered just above the composer while Ctrl+R is
/// active. Shows the query, hit count, and cursor position. Enter/n
/// cycles; Esc closes.
pub(crate) fn search(f: &mut Frame, input_area: Rect, state: &TuiState) {
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

// The palette's highlight background needs the theme's `dim`/cream
// colors; go through the same shims the components use (imported above).
