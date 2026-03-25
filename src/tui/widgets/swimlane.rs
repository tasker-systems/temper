use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding, Paragraph};

use crate::actions::types::TicketInfo;

/// A stateless widget that renders a single kanban column.
pub struct Swimlane<'a> {
    pub title: &'a str,
    pub count: usize,
    pub tickets: &'a [TicketInfo],
    pub selected: Option<usize>,
    pub focused: bool,
}

impl<'a> Widget for Swimlane<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let header = format!("{} ({})", self.title, self.count);

        let border_style = if self.focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(header)
            .borders(Borders::ALL)
            .border_style(border_style)
            .padding(Padding::horizontal(1));

        let inner = block.inner(area);
        block.render(area, buf);

        // Render each ticket line inside the block
        for (i, ticket) in self.tickets.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let scope_label = ticket.scope.as_deref().unwrap_or("");

            let scope_style = match scope_label {
                "patch" => Style::default().fg(Color::Blue),
                "feature" => Style::default().fg(Color::Yellow),
                "epic" => Style::default().fg(Color::Magenta),
                _ => Style::default().fg(Color::DarkGray),
            };

            let is_selected = self.selected == Some(i);
            let base_style = if is_selected {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            } else {
                Style::default()
            };

            let row_area = Rect {
                x: inner.x,
                y: inner.y + i as u16,
                width: inner.width,
                height: 1,
            };

            // Build the line: [scope] title, truncated to fit
            let mut spans = Vec::new();

            if !scope_label.is_empty() {
                spans.push(Span::styled(
                    format!("[{}] ", scope_label),
                    if is_selected {
                        scope_style.bg(Color::DarkGray)
                    } else {
                        scope_style
                    },
                ));
            }

            // Calculate remaining width for title
            let prefix_len = if scope_label.is_empty() {
                0
            } else {
                scope_label.len() + 3 // "[scope] "
            };
            let available = (row_area.width as usize).saturating_sub(prefix_len);
            let title = if ticket.title.len() > available {
                let truncated: String = ticket
                    .title
                    .chars()
                    .take(available.saturating_sub(1))
                    .collect();
                format!("{}\u{2026}", truncated)
            } else {
                ticket.title.clone()
            };

            spans.push(Span::styled(title, base_style));

            let line = Line::from(spans);
            let para = Paragraph::new(line);
            para.render(row_area, buf);
        }
    }
}
