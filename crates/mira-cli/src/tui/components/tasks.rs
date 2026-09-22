//! Task panel — the agent's live plan, rendered with checkboxes under
//! the streaming indicator.
//!
//! Hydrated from the `task_create` / `task_update` / `task_list` tool
//! results' structured `data` payloads (see `TuiState::apply_task_payload`),
//! so the panel always mirrors the task store the agent itself sees.
//! Like Claude Code's todo list, it turns a long tool-calling turn into
//! a legible "here's the plan, here's where we are" readout.

use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use super::{CREAM, DIM, MUTED, SALMON};
use crate::tui::state::{TaskItem, TaskStatus};

/// How many task rows to show before collapsing into "+N more".
const MAX_VISIBLE: usize = 8;

pub(crate) struct TaskListView<'a> {
    pub items: &'a [TaskItem],
}

pub(crate) fn render(v: &TaskListView<'_>) -> Vec<Line<'static>> {
    let total = v.items.len();
    let done = v
        .items
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();

    let mut out = vec![Line::from(vec![
        Span::styled("  tasks ", Style::default().fg(MUTED()).italic()),
        Span::styled(
            format!("{done}/{total}"),
            Style::default().fg(MUTED()).bold(),
        ),
    ])];

    for item in v.items.iter().take(MAX_VISIBLE) {
        out.push(task_line(item));
    }
    if total > MAX_VISIBLE {
        out.push(Line::from(Span::styled(
            format!("    … {} more", total - MAX_VISIBLE),
            Style::default().fg(DIM()).italic(),
        )));
    }
    out
}

fn task_line(item: &TaskItem) -> Line<'static> {
    let label = item
        .active_form
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&item.subject);
    match item.status {
        TaskStatus::Pending => Line::from(vec![
            Span::styled("  ○ ", Style::default().fg(DIM())),
            Span::styled(label.to_owned(), Style::default().fg(MUTED())),
        ]),
        TaskStatus::InProgress => Line::from(vec![
            Span::styled("  ◐ ", Style::default().fg(SALMON()).bold()),
            Span::styled(label.to_owned(), Style::default().fg(CREAM()).bold()),
        ]),
        TaskStatus::Completed => Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(ratatui::style::Color::Green)),
            Span::styled(
                item.subject.clone(),
                Style::default().fg(MUTED()).add_modifier(Modifier::DIM),
            ),
        ]),
        // Deleted tasks are filtered at hydration; render defensively.
        TaskStatus::Deleted => Line::from(Span::raw("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, subject: &str, status: TaskStatus) -> TaskItem {
        TaskItem {
            id,
            subject: subject.into(),
            active_form: None,
            status,
        }
    }

    #[test]
    fn renders_header_and_glyphs() {
        let items = vec![
            item(1, "Investigate", TaskStatus::Completed),
            item(2, "Run tests", TaskStatus::InProgress),
            item(3, "Write docs", TaskStatus::Pending),
        ];
        let ls = render(&TaskListView { items: &items });
        assert_eq!(ls.len(), 4); // header + 3
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("tasks 1/3"));
        assert!(joined.contains("✓"));
        assert!(joined.contains("◐"));
        assert!(joined.contains("○"));
    }

    #[test]
    fn in_progress_prefers_active_form() {
        let mut it = item(2, "Run tests", TaskStatus::InProgress);
        it.active_form = Some("Running tests".into());
        let ls = render(&TaskListView { items: &[it] });
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("Running tests"));
        assert!(!joined.contains("Run tests"));
    }

    #[test]
    fn caps_at_max_visible() {
        let items: Vec<TaskItem> = (1..=12)
            .map(|i| item(i, &format!("task {i}"), TaskStatus::Pending))
            .collect();
        let ls = render(&TaskListView { items: &items });
        assert_eq!(ls.len(), MAX_VISIBLE + 2); // header + 8 + more-line
        let joined: String = ls
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(joined.contains("4 more"));
    }
}
