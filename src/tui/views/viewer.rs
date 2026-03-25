use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::tui::app::ViewerState;
use crate::tui::widgets::frontmatter::render_frontmatter;

/// Compute the height (in rows) the frontmatter block should occupy.
/// Counts non-null, non-empty fields plus 2 for the border.
fn frontmatter_height(fm: &serde_yaml::Value) -> u16 {
    let field_count = match fm {
        serde_yaml::Value::Mapping(m) => m
            .iter()
            .filter(|(_, v)| {
                !matches!(v, serde_yaml::Value::Null) && {
                    let s = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => format!("{:?}", other),
                    };
                    !s.trim().is_empty()
                }
            })
            .count(),
        _ => 0,
    };
    // 2 lines for border top/bottom, 1 per field; minimum 2 so the block renders
    (field_count.max(1) as u16) + 2
}

/// Render the full-screen document viewer.
pub fn render_viewer(frame: &mut Frame, area: Rect, state: &ViewerState) {
    let fm_height = frontmatter_height(&state.document.frontmatter);

    // Layout: breadcrumb (1 line) | frontmatter | body (fills rest)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),         // breadcrumb
            Constraint::Length(fm_height), // frontmatter
            Constraint::Min(1),            // body
        ])
        .split(area);

    // Breadcrumb
    let crumb = Paragraph::new(Line::from(vec![
        Span::styled(
            "← ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(
            state.source_label.as_str(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(
            state.document.title.as_str(),
            Style::default().fg(Color::White),
        ),
    ]));
    frame.render_widget(crumb, chunks[0]);

    // Frontmatter
    render_frontmatter(frame, chunks[1], &state.document.frontmatter);

    // Body — scrollable paragraph
    let body_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let body = Paragraph::new(state.document.body.as_str())
        .block(body_block)
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset as u16, 0));

    frame.render_widget(body, chunks[2]);
}
