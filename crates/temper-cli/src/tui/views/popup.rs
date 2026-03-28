use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

/// Render a centered popup overlay with a list of selectable options.
///
/// `options` is a list of `(value, optional_description)` pairs.
/// The selected item is highlighted.
pub fn render_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    options: &[(String, Option<String>)],
    selected: usize,
) {
    let popup_width = 50u16;
    let popup_height = (options.len() as u16) + 2; // border top + bottom

    // Centre the popup within `area`
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_width.min(area.width),
        height: popup_height.min(area.height),
    };

    // Clear the background behind the popup
    frame.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, (value, desc))| {
            let text = if let Some(d) = desc {
                format!("  {} \u{2014} {}", value, d)
            } else {
                format!("  {}", value)
            };
            let style = if i == selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default()
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::White));

    let list = List::new(items).block(block);
    frame.render_widget(list, popup_area);
}

/// Stage picker options.
pub fn stage_options() -> Vec<(String, Option<String>)> {
    vec![
        ("backlog".to_string(), None),
        ("in-progress".to_string(), None),
        ("done".to_string(), None),
        ("cancelled".to_string(), None),
    ]
}

/// Scope picker options.
pub fn scope_options() -> Vec<(String, Option<String>)> {
    vec![
        (
            "patch".to_string(),
            Some("tactical, no ceremony".to_string()),
        ),
        ("feature".to_string(), Some("full pipeline".to_string())),
        ("epic".to_string(), Some("strategic roadmap".to_string())),
    ]
}
