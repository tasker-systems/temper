use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::SearchState;
use crate::tui::widgets::result_list::{render_result_list, ResultItem};

/// Render the search tab into `area`.
///
/// Layout:
///   [0] 1 line  — query input
///   [1] 1 line  — result count / loading indicator
///   [2] Min(1)  — result list
pub fn render_search(frame: &mut Frame, area: Rect, state: &SearchState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // input line
            Constraint::Length(1), // status / result count
            Constraint::Min(1),    // results
        ])
        .split(area);

    // -- Input line ----------------------------------------------------------
    let cursor_char = if state.input_focused { "\u{2502}" } else { "" };
    let input_text = format!("/ {}{}", state.query, cursor_char);
    let input_style = if state.input_focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let input_paragraph = Paragraph::new(Span::styled(input_text, input_style));
    frame.render_widget(input_paragraph, chunks[0]);

    // -- Status line ---------------------------------------------------------
    let status_text = if state.loading {
        "Searching...".to_string()
    } else if state.query.is_empty() {
        String::new()
    } else {
        format!(
            "{} result{}",
            state.results.len(),
            if state.results.len() == 1 { "" } else { "s" }
        )
    };
    let status_paragraph = Paragraph::new(Span::styled(
        status_text,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(status_paragraph, chunks[1]);

    // -- Result list ---------------------------------------------------------
    if state.results.is_empty() {
        if !state.query.is_empty() && !state.loading {
            let no_results = Paragraph::new(Span::styled(
                "No results",
                Style::default().fg(Color::DarkGray),
            ));
            frame.render_widget(no_results, chunks[2]);
        }
        return;
    }

    let items: Vec<ResultItem> = state
        .results
        .iter()
        .map(|hit| ResultItem {
            score: hit.score,
            path: &hit.file_path,
            note_type: &hit.note_type,
            snippet: &hit.content,
            depth: None,
        })
        .collect();

    render_result_list(frame, chunks[2], &items, state.selected);
}
