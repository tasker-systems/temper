use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::app::{FocusRegion, ViewerState};
use crate::tui::widgets::breadcrumb_bar::BreadcrumbBar;
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::frontmatter::render_frontmatter;
use crate::tui::widgets::markdown_renderer::render_markdown;
use crate::tui::widgets::section_separator::SectionSeparator;

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
pub fn render_viewer(frame: &mut Frame, area: Rect, state: &ViewerState, focus: FocusRegion) {
    let fm_height = frontmatter_height(&state.document.frontmatter);

    // Layout: breadcrumb (1 line) | frontmatter | separator (1 line) | body (fills rest)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),         // breadcrumb
            Constraint::Length(fm_height), // frontmatter
            Constraint::Length(1),         // separator
            Constraint::Min(1),            // body
        ])
        .split(area);

    // Breadcrumb bar
    let segs: Vec<&str> = state
        .breadcrumb_segments
        .iter()
        .map(|s| s.as_str())
        .collect();
    let crumb_line = BreadcrumbBar::new(&segs).to_line();
    frame.render_widget(Paragraph::new(crumb_line), chunks[0]);

    // Frontmatter — DisplayOnly aesthetic (dim DarkGray border, not a focus target).
    // render_frontmatter draws its own bordered Block internally with DarkGray styling,
    // which matches the DisplayOnly convention, so we render it directly.
    render_frontmatter(frame, chunks[1], &state.document.frontmatter);

    // Section separator
    let sep = SectionSeparator::new(area.width);
    let sep_line = sep.to_line();
    frame.render_widget(Paragraph::new(sep_line), chunks[2]);

    // Body — scrollable paragraph with markdown rendering
    let body_focusable =
        FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Primary);
    let body_block = body_focusable.to_block();

    let md_lines = render_markdown(&state.document.body);
    let body = Paragraph::new(md_lines)
        .block(body_block)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(Color::Rgb(22, 22, 42)))
        .scroll((state.scroll_offset as u16, 0));

    frame.render_widget(body, chunks[3]);
}
