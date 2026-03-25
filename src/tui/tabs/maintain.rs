use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::MaintainState;

/// Render the maintain tab into `area`.
///
/// Layout (vertical chunks):
///   Header, index section, normalize section, progress, key hints.
pub fn render_maintain(frame: &mut Frame, area: Rect, state: &MaintainState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header + blank
            Constraint::Length(2), // index section
            Constraint::Length(5), // normalize section
            Constraint::Length(1), // progress / status
            Constraint::Min(1),    // spacer
        ])
        .split(area);

    // -- Header ---------------------------------------------------------------
    let header = Paragraph::new(Span::styled(
        "Maintenance",
        Style::default().fg(Color::Yellow).bold(),
    ));
    frame.render_widget(header, chunks[0]);

    // -- Index section --------------------------------------------------------
    let index_text = match &state.index_stats {
        Some(stats) => format!(
            "Last index: {} documents, {} chunks ({:.1}s)",
            stats.documents, stats.chunks, stats.duration_secs
        ),
        None => "No index data".to_string(),
    };
    let index_label = Paragraph::new(vec![
        Line::from(Span::styled(
            "Index",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(Span::styled(index_text, Style::default().fg(Color::White))),
    ]);
    frame.render_widget(index_label, chunks[1]);

    // -- Normalize section ----------------------------------------------------
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
    let normalize_widget = Paragraph::new(normalize_lines);
    frame.render_widget(normalize_widget, chunks[2]);

    // -- Progress / status line -----------------------------------------------
    let progress_text = if state.running {
        let msg = state.progress_message.as_deref().unwrap_or("Running...");
        Span::styled(msg, Style::default().fg(Color::Yellow))
    } else {
        Span::styled("idle", Style::default().fg(Color::DarkGray))
    };
    let progress = Paragraph::new(progress_text);
    frame.render_widget(progress, chunks[3]);
}
