use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::tui::app::{BoardLevel, BoardState, FocusRegion, MilestoneWithCounts};
use crate::tui::widgets::breadcrumb_bar::BreadcrumbBar;
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::swimlane::Swimlane;

/// Render the projects tab content into `area` based on the current board state.
pub fn render_projects_tab(frame: &mut Frame, area: Rect, state: &BoardState, focus: FocusRegion) {
    match &state.level {
        BoardLevel::Projects {
            projects, selected, ..
        } => render_projects(frame, area, projects, *selected, focus),
        BoardLevel::Milestones {
            project,
            milestones,
            selected,
        } => render_milestones(frame, area, project, milestones, *selected, focus),
        BoardLevel::Swimlanes {
            project,
            milestone,
            columns,
            column,
            row,
            ..
        } => render_swimlanes(
            frame, area, project, milestone, columns, *column, *row, focus,
        ),
    }
}

fn render_projects(
    frame: &mut Frame,
    area: Rect,
    projects: &[String],
    selected: usize,
    focus: FocusRegion,
) {
    // Layout: breadcrumb (1 line) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(BreadcrumbBar::new(&["All"]).to_line());
    frame.render_widget(breadcrumb, chunks[0]);

    if projects.is_empty() {
        let msg =
            Paragraph::new("No projects configured").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == selected { "\u{25b8} " } else { "  " };
            let style = if i == selected {
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(42, 42, 74))
                    .bold()
            } else {
                Style::default()
            };
            ListItem::new(format!("{}{}", marker, name)).style(style)
        })
        .collect();

    let fb = FocusableBlock::new(FocusStyle::Content).focused(focus != FocusRegion::TabBar);
    let block = fb.to_block();
    let list = List::new(items).block(block);
    frame.render_widget(list, chunks[1]);
}

fn render_milestones(
    frame: &mut Frame,
    area: Rect,
    project: &str,
    milestones: &[MilestoneWithCounts],
    selected: usize,
    focus: FocusRegion,
) {
    // Layout: breadcrumb (1 line) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(BreadcrumbBar::new(&["All", project]).to_line());
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
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(42, 42, 74))
                    .bold()
            } else {
                Style::default()
            };

            let counts = format_milestone_counts(ms);
            let label = format!("{}{:<26}{}", marker, ms.info.title, counts);
            ListItem::new(label).style(style)
        })
        .collect();

    let fb = FocusableBlock::new(FocusStyle::Content).focused(focus != FocusRegion::TabBar);
    let block = fb.to_block();
    let list = List::new(items).block(block);
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

#[expect(
    clippy::too_many_arguments,
    reason = "swimlane render takes all view params"
)]
fn render_swimlanes(
    frame: &mut Frame,
    area: Rect,
    project: &str,
    milestone: &str,
    columns: &[Vec<crate::actions::types::TicketInfo>; 3],
    active_column: usize,
    active_row: usize,
    focus: FocusRegion,
) {
    // Layout: breadcrumb (1 line) + columns
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb = Paragraph::new(BreadcrumbBar::new(&["All", project, milestone]).to_line());
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
        let col_focused = focus == FocusRegion::Tertiary(i);
        let selected_row = if i == active_column {
            Some(active_row)
        } else {
            None
        };

        let _block = FocusableBlock::new(FocusStyle::Content)
            .focused(col_focused)
            .to_block();

        let swimlane = Swimlane {
            title,
            count: columns[i].len(),
            tickets: &columns[i],
            selected: selected_row,
            focused: i == active_column,
        };

        frame.render_widget(swimlane, col_areas[i]);
    }
}
