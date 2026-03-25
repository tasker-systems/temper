use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::Screen;

pub fn render_keyhints(frame: &mut Frame, area: Rect, screen: &Screen) {
    let hints = match screen {
        Screen::Board(_) => {
            "j/k move · h/l columns · Enter open · e edit · s stage · S scope · : cmd"
        }
        Screen::Search(state) => {
            if state.input_focused {
                "Type to search · Tab results · Esc cancel"
            } else {
                "j/k move · Enter view · c context · / search · e edit · : cmd"
            }
        }
        Screen::Context(_) => "j/k move · Enter view · c re-center · +/- depth · / topic · : cmd",
        Screen::Maintain(_) => "i index · n normalize · : cmd",
        Screen::Viewer(_) => {
            "j/k scroll · e edit · c context · s stage · S scope · Esc back · : cmd"
        }
    };

    let paragraph = Paragraph::new(hints).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    frame.render_widget(paragraph, area);
}
