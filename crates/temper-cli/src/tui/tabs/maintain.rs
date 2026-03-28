use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::state::FocusRegion;
use crate::tui::app::MaintainState;
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::section_separator::SectionSeparator;

/// Render the maintain tab into `area`.
///
/// Layout (vertical chunks):
///   Actions block (index + normalize), separator, progress area.
pub fn render_maintain(frame: &mut Frame, area: Rect, state: &MaintainState, focus: FocusRegion) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // actions block (index + normalize)
            Constraint::Length(1), // separator
            Constraint::Min(1),    // progress area
        ])
        .split(area);

    // -- Actions block (index + normalize) ------------------------------------
    let focusable = FocusableBlock::new(FocusStyle::Content)
        .focused(focus == FocusRegion::Primary)
        .title(" Maintenance ");
    let block = focusable.to_block();

    let index_text = match &state.index_stats {
        Some(stats) => format!(
            "  {} documents, {} chunks ({:.1}s)",
            stats.documents, stats.chunks, stats.duration_secs
        ),
        None => "  No index data".to_string(),
    };

    let normalize_lines: Vec<Line> = match &state.last_normalize {
        Some(summary) => vec![
            Line::from(Span::styled(
                "Normalize",
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(Span::styled(
                format!(
                    "  IDs backfilled: {}  |  Files moved: {}  |  Stages migrated: {}",
                    summary.ids_backfilled, summary.files_moved, summary.stages_migrated,
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!(
                    "  Slugs fixed: {}  |  Frontmatter fixed: {}  |  Unscoped tickets: {}",
                    summary.slugs_fixed, summary.frontmatter_fixed, summary.unscoped_tickets,
                ),
                Style::default().fg(Color::White),
            )),
        ],
        None => vec![
            Line::from(Span::styled(
                "Normalize",
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(Span::styled(
                "  Not run",
                Style::default().fg(Color::DarkGray),
            )),
        ],
    };

    let mut content_lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Index",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(Span::styled(index_text, Style::default().fg(Color::White))),
        Line::from(""),
    ];
    content_lines.extend(normalize_lines);

    let actions_widget = Paragraph::new(content_lines).block(block);
    frame.render_widget(actions_widget, chunks[0]);

    // -- Separator ------------------------------------------------------------
    let status_label = if state.running {
        state
            .progress_message
            .as_deref()
            .unwrap_or("Running...")
            .to_string()
    } else {
        "idle".to_string()
    };
    let separator = SectionSeparator::new(area.width).label(&status_label);
    let separator_widget = Paragraph::new(separator.to_line());
    frame.render_widget(separator_widget, chunks[1]);

    // -- Progress area --------------------------------------------------------
    let progress_text = if state.running {
        let msg = state.progress_message.as_deref().unwrap_or("Running...");
        Paragraph::new(Span::styled(msg, Style::default().fg(Color::Yellow)))
    } else {
        Paragraph::new(Span::styled("", Style::default()))
    };
    frame.render_widget(progress_text, chunks[2]);
}
