use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem};

/// A single renderable item for search or context result lists.
pub struct ResultItem<'a> {
    pub score: f32,
    pub path: &'a str,
    pub note_type: &'a str,
    pub snippet: &'a str,
    pub depth: Option<usize>, // for context results; None for search
}

/// Render a list of `ResultItem`s into `area`, highlighting `selected`.
///
/// Each result occupies two lines:
///   [0.82] path/to/file.md
///     note_type · snippet text...
pub fn render_result_list(frame: &mut Frame, area: Rect, items: &[ResultItem], selected: usize) {
    if items.is_empty() {
        let empty = Block::default().borders(Borders::NONE);
        frame.render_widget(empty, area);
        return;
    }

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == selected;

            // Truncate snippet to keep lines reasonably short
            let max_snippet = 80usize;
            let snippet_truncated = if item.snippet.len() > max_snippet {
                format!("{}…", &item.snippet[..max_snippet])
            } else {
                item.snippet.to_string()
            };

            // Depth prefix for context results
            let depth_prefix = match item.depth {
                Some(d) if d > 0 => "  ".repeat(d),
                _ => String::new(),
            };

            let header_style = if is_selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default().fg(Color::White)
            };
            let meta_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let score_text = format!("[{:.2}]", item.score);
            let header_line = Line::from(vec![
                Span::styled(depth_prefix.clone(), Style::default()),
                Span::styled(score_text, meta_style),
                Span::styled(" ", Style::default()),
                Span::styled(item.path, header_style),
            ]);
            let detail_line = Line::from(vec![
                Span::styled(format!("{}  ", depth_prefix), Style::default()),
                Span::styled(item.note_type, meta_style),
                Span::styled(" \u{00b7} ", Style::default().fg(Color::DarkGray)),
                Span::styled(snippet_truncated, Style::default().fg(Color::Gray)),
            ]);

            let text = Text::from(vec![header_line, detail_line]);
            ListItem::new(text)
        })
        .collect();

    let list = List::new(list_items).block(Block::default().borders(Borders::NONE));
    frame.render_widget(list, area);
}
