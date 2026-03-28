use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::tui::app::{BoardLevel, BoardState, DetailPanel, FocusRegion, MilestoneWithCounts};
use crate::tui::widgets::breadcrumb_bar::BreadcrumbBar;
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::swimlane::Swimlane;

/// Render the projects tab content into `area` based on the current board state.
pub fn render_projects_tab(frame: &mut Frame, area: Rect, state: &BoardState, focus: FocusRegion) {
    match &state.level {
        BoardLevel::ProjectList {
            projects,
            selected,
            detail,
        } => render_project_list(frame, area, projects, *selected, detail.as_ref(), focus),
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

fn render_project_list(
    frame: &mut Frame,
    area: Rect,
    projects: &[String],
    selected: usize,
    detail: Option<&DetailPanel>,
    focus: FocusRegion,
) {
    // Layout: breadcrumb (1 line) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb
    let breadcrumb_segments: Vec<&str> = if let Some(d) = detail {
        vec!["All", &d.project]
    } else {
        vec!["All"]
    };
    let breadcrumb = Paragraph::new(BreadcrumbBar::new(&breadcrumb_segments).to_line());
    frame.render_widget(breadcrumb, chunks[0]);

    // Horizontal split: left 35% projects, right 65% milestones
    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);

    // Left panel: project list
    if projects.is_empty() {
        let msg =
            Paragraph::new("No projects configured").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, panels[0]);
    } else {
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

        let fb = FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Primary);
        let block = fb.to_block();
        let list = List::new(items).block(block);
        frame.render_widget(list, panels[0]);
    }

    // Right panel: milestone detail or placeholder
    if let Some(d) = detail {
        render_milestone_detail(frame, panels[1], d, focus);
    } else {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(false);
        let block = fb.to_block();
        let msg = Paragraph::new("Select a project")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(msg, panels[1]);
    }
}

fn render_milestone_detail(
    frame: &mut Frame,
    area: Rect,
    detail: &DetailPanel,
    focus: FocusRegion,
) {
    if detail.milestones.is_empty() {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Secondary);
        let block = fb.to_block();
        let msg = Paragraph::new("No milestones")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = detail
        .milestones
        .iter()
        .enumerate()
        .map(|(i, ms)| {
            let marker = if i == detail.selected {
                "\u{25b8} "
            } else {
                "  "
            };
            let style = if i == detail.selected {
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

    let fb = FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Secondary);
    let block = fb.to_block();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
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
