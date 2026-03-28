use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::state::{FocusRegion, SearchState};
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::result_list::{render_result_list, ResultItem};
use crate::tui::widgets::section_separator::SectionSeparator;

pub fn render_search(frame: &mut Frame, area: Rect, state: &SearchState, focus: FocusRegion) {
    let input_focused = focus == FocusRegion::Primary;
    let results_focused = focus == FocusRegion::Secondary;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // input with border
            Constraint::Length(1), // separator
            Constraint::Min(3),    // results with border
        ])
        .split(area);

    // Input block
    let input_focusable = FocusableBlock::new(FocusStyle::Input).focused(input_focused);
    let input_block = input_focusable.to_block();
    let cursor_char = if state.input_focused { "\u{2502}" } else { "" };
    let input_text = format!("/ {}{}", state.query, cursor_char);
    let input_style = if state.input_focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(input_text, input_style)).block(input_block),
        chunks[0],
    );

    // Separator with result count
    let status = if state.loading {
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
    let sep_widget = if status.is_empty() {
        SectionSeparator::new(area.width)
    } else {
        SectionSeparator::new(area.width).label(&status)
    };
    let sep = sep_widget.to_line();
    frame.render_widget(Paragraph::new(sep), chunks[1]);

    // Results block
    let results_focusable = FocusableBlock::new(FocusStyle::Content).focused(results_focused);
    let results_block = results_focusable.to_block();
    let results_inner = results_block.inner(chunks[2]);
    frame.render_widget(results_block, chunks[2]);

    if state.results.is_empty() {
        if !state.query.is_empty() && !state.loading {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No results",
                    Style::default().fg(Color::DarkGray),
                )),
                results_inner,
            );
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
    render_result_list(frame, results_inner, &items, state.selected);
}
