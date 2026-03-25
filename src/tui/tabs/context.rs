use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::ContextState;
use crate::tui::widgets::result_list::{render_result_list, ResultItem};

/// Render the context tab into `area`.
///
/// Layout:
///   [0] 1 line  — topic input (only when input_active) or center indicator
///   [1] 1 line  — status / neighbor count / loading
///   [2] Min(1)  — neighbor list grouped by depth
pub fn render_context(frame: &mut Frame, area: Rect, state: &ContextState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // input or center indicator
            Constraint::Length(1), // status line
            Constraint::Min(1),    // neighbor list
        ])
        .split(area);

    // -- Top line: input field or center indicator ---------------------------
    if state.input_active {
        let cursor = "\u{2502}";
        let input_text = format!("/ {}{}", state.input_text, cursor);
        let input_para =
            Paragraph::new(Span::styled(input_text, Style::default().fg(Color::Yellow)));
        frame.render_widget(input_para, chunks[0]);
    } else if state.current_center.is_empty() {
        // No center set yet — show prompt
        let prompt = Paragraph::new(Span::styled(
            "Press / to enter a topic",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(prompt, chunks[0]);
    } else {
        // Center indicator with depth and navigation hint
        let center_text = format!(
            "\u{2299} {}  depth: {}  [+/-] adjust  [c] re-center  [Esc] back",
            state.current_center, state.depth
        );
        let center_para =
            Paragraph::new(Span::styled(center_text, Style::default().fg(Color::Cyan)));
        frame.render_widget(center_para, chunks[0]);
    }

    // -- Status line ---------------------------------------------------------
    let status_text = if state.loading {
        "Loading neighbors...".to_string()
    } else if state.current_center.is_empty() {
        String::new()
    } else {
        let stack_hint = if state.center_stack.is_empty() {
            String::new()
        } else {
            format!("  (stack: {})", state.center_stack.len())
        };
        format!(
            "{} neighbor{}{}",
            state.neighbors.len(),
            if state.neighbors.len() == 1 { "" } else { "s" },
            stack_hint,
        )
    };
    let status_para = Paragraph::new(Span::styled(
        status_text,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(status_para, chunks[1]);

    // -- Neighbor list -------------------------------------------------------
    if state.neighbors.is_empty() {
        if !state.loading && !state.current_center.is_empty() {
            let no_results = Paragraph::new(Span::styled(
                "No neighbors found",
                Style::default().fg(Color::DarkGray),
            ));
            frame.render_widget(no_results, chunks[2]);
        }
        return;
    }

    let items: Vec<ResultItem> = state
        .neighbors
        .iter()
        .map(|n| ResultItem {
            score: n.score,
            path: &n.file_path,
            note_type: &n.note_type,
            snippet: &n.label,
            depth: Some(n.depth),
        })
        .collect();

    render_result_list(frame, chunks[2], &items, state.selected);
}
