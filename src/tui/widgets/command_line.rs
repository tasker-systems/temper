use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn render_command_line(frame: &mut Frame, area: Rect, input: &str) {
    let line = Line::from(vec![
        Span::styled(
            ":",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(input, Style::default().fg(Color::White)),
        Span::styled("█", Style::default().fg(Color::Yellow)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
