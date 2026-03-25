use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::{ContextState, FocusRegion};
use crate::tui::widgets::focusable_block::{FocusStyle, FocusableBlock};
use crate::tui::widgets::result_list::{render_result_list, ResultItem};
use crate::tui::widgets::section_separator::SectionSeparator;

/// Render the context tab into `area`.
///
/// Layout: topic input or center indicator (height 3), separator (height 1), neighbor list (fills rest).
pub fn render_context(frame: &mut Frame, area: Rect, state: &ContextState, focus: FocusRegion) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // input or center indicator block
            Constraint::Length(1), // section separator
            Constraint::Min(1),    // neighbor list block
        ])
        .split(area);

    // -- Input / indicator block ---------------------------------------------
    let input_focusable =
        FocusableBlock::new(FocusStyle::Input).focused(focus == FocusRegion::Primary);
    let input_block = input_focusable.to_block();

    if state.input_active {
        let cursor = "\u{2502}";
        let input_text = format!("/ {}{}", state.input_text, cursor);
        let input_para =
            Paragraph::new(Span::styled(input_text, Style::default().fg(Color::Yellow)))
                .block(input_block);
        frame.render_widget(input_para, chunks[0]);
    } else if state.current_center.is_empty() {
        // No center set yet — show prompt
        let prompt = Paragraph::new(Span::styled(
            "Press / to enter a topic",
            Style::default().fg(Color::DarkGray),
        ))
        .block(input_block);
        frame.render_widget(prompt, chunks[0]);
    } else {
        // Center indicator with depth and navigation hint
        let center_text = format!(
            "\u{2299} {}  depth: {}  [+/-] adjust  [c] re-center  [Esc] back",
            state.current_center, state.depth
        );
        let center_para =
            Paragraph::new(Span::styled(center_text, Style::default().fg(Color::Cyan)))
                .block(input_block);
        frame.render_widget(center_para, chunks[0]);
    }

    // -- Section separator ---------------------------------------------------
    let separator_label = if state.loading {
        "Loading...".to_string()
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

    let sep_owned;
    let sep_line = if separator_label.is_empty() {
        sep_owned = SectionSeparator::new(chunks[1].width);
        sep_owned.to_line()
    } else {
        sep_owned = SectionSeparator::new(chunks[1].width).label(&separator_label);
        sep_owned.to_line()
    };
    frame.render_widget(Paragraph::new(sep_line), chunks[1]);

    // -- Neighbor list block -------------------------------------------------
    let content_focusable =
        FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Secondary);
    let content_block = content_focusable.to_block();

    if state.neighbors.is_empty() {
        if !state.loading && !state.current_center.is_empty() {
            let no_results = Paragraph::new(Span::styled(
                "No neighbors found",
                Style::default().fg(Color::DarkGray),
            ))
            .block(content_block);
            frame.render_widget(no_results, chunks[2]);
        } else {
            frame.render_widget(content_block, chunks[2]);
        }
        return;
    }

    let inner_content = content_block.inner(chunks[2]);
    frame.render_widget(content_block, chunks[2]);

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

    render_result_list(frame, inner_content, &items, state.selected);
}
