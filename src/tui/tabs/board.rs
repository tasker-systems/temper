use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::tui::app::{BoardLevel, BoardState, MilestoneWithCounts};
use crate::tui::widgets::swimlane::Swimlane;

/// Render the board tab content into `area` based on the current board state.
pub fn render_board(frame: &mut Frame, area: Rect, state: &BoardState) {
    match &state.level {
        BoardLevel::Projects {
            projects, selected, ..
        } => render_projects(frame, area, projects, *selected),
        BoardLevel::Milestones {
            project,
            milestones,
            selected,
        } => render_milestones(frame, area, project, milestones, *selected),
        BoardLevel::Swimlanes {
            project,
            milestone,
            columns,
            column,
            row,
            ..
        } => render_swimlanes(frame, area, project, milestone, columns, *column, *row),
    }
}

fn render_projects(frame: &mut Frame, area: Rect, projects: &[String], selected: usize) {
    if projects.is_empty() {
        let msg =
            Paragraph::new("No projects configured").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == selected { "\u{25b8} " } else { "  " };
            let style = if i == selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default()
            };
            ListItem::new(format!("{}{}", marker, name)).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Projects").borders(Borders::NONE));
    frame.render_widget(list, area);
}

fn render_milestones(
    frame: &mut Frame,
    area: Rect,
    project: &str,
    milestones: &[MilestoneWithCounts],
    selected: usize,
) {
    // Layout: breadcrumb (1 line) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(Line::from(vec![
        Span::styled("All", Style::default().fg(Color::DarkGray)),
        Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)),
        Span::styled(project, Style::default().fg(Color::White)),
    ]));
    frame.render_widget(breadcrumb, chunks[0]);

    if milestones.is_empty() {
        let msg =
            Paragraph::new("Loading milestones...").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = milestones
        .iter()
        .enumerate()
        .map(|(i, ms)| {
            let marker = if i == selected { "\u{25b8} " } else { "  " };
            let style = if i == selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default()
            };

            let counts = format_milestone_counts(ms);
            let label = format!("{}{:<26}{}", marker, ms.info.title, counts);
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::NONE));
    frame.render_widget(list, chunks[1]);
}

fn format_milestone_counts(ms: &MilestoneWithCounts) -> String {
    let mut parts = Vec::new();
    if ms.in_progress > 0 {
        parts.push(format!("{} in-progress", ms.in_progress));
    }
    if ms.backlog > 0 {
        parts.push(format!("{} backlog", ms.backlog));
    }
    if ms.done > 0 {
        parts.push(format!("{} done", ms.done));
    }
    if parts.is_empty() {
        "empty".to_string()
    } else {
        parts.join(" \u{00b7} ")
    }
}

fn render_swimlanes(
    frame: &mut Frame,
    area: Rect,
    project: &str,
    milestone: &str,
    columns: &[Vec<crate::actions::types::TicketInfo>; 3],
    active_column: usize,
    active_row: usize,
) {
    // Layout: breadcrumb (1 line) + columns
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(Line::from(vec![
        Span::styled("All", Style::default().fg(Color::DarkGray)),
        Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)),
        Span::styled(project, Style::default().fg(Color::DarkGray)),
        Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)),
        Span::styled(milestone, Style::default().fg(Color::White)),
    ]));
    frame.render_widget(breadcrumb, chunks[0]);

    // Three equal columns
    let col_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(chunks[1]);

    let titles = ["BACKLOG", "IN-PROGRESS", "DONE"];

    for (i, title) in titles.iter().enumerate() {
        let is_focused = i == active_column;
        let selected_row = if is_focused { Some(active_row) } else { None };

        let swimlane = Swimlane {
            title,
            count: columns[i].len(),
            tickets: &columns[i],
            selected: selected_row,
            focused: is_focused,
        };

        frame.render_widget(swimlane, col_areas[i]);
    }
}
