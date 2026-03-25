use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding},
};

/// Determines visual style and focus behavior for a `FocusableBlock`.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusStyle {
    /// Yellow border when focused — used for interactive input fields.
    Input,
    /// Cyan border when focused — used for content regions like result lists.
    Content,
    /// Always dim DarkGray border — read-only display, not a tab-stop.
    #[allow(dead_code)]
    DisplayOnly,
}

/// A configurable bordered region whose border color reflects focus state.
///
/// All blocks use [`Borders::ALL`] and [`Padding::horizontal`](1).
///
/// # Border colors
///
/// | State                  | Border Color |
/// |------------------------|-------------|
/// | Focused + Input        | Yellow      |
/// | Focused + Content      | Cyan        |
/// | DisplayOnly (any)      | DarkGray    |
/// | Unfocused (any)        | DarkGray    |
#[derive(Debug, Clone)]
pub struct FocusableBlock {
    style: FocusStyle,
    focused: bool,
    title: Option<String>,
}

impl FocusableBlock {
    /// Create a new `FocusableBlock` with the given focus style.
    pub fn new(style: FocusStyle) -> Self {
        Self {
            style,
            focused: false,
            title: None,
        }
    }

    /// Set whether this block is currently focused.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Set the title rendered in the top border.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    /// Build a ratatui [`Block`] with border styling appropriate for this block's state.
    pub fn to_block(&self) -> Block<'_> {
        let border_color = match (&self.style, self.focused) {
            (FocusStyle::Input, true) => Color::Yellow,
            (FocusStyle::Content, true) => Color::Cyan,
            _ => Color::DarkGray,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .padding(Padding::horizontal(1));

        match &self.title {
            Some(t) => block.title(t.as_str()),
            None => block,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_block(block: Block) -> Buffer {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);
        buf
    }

    /// Helper: get the style of the top-left corner character (0,0) which carries border_style.
    fn border_fg_at(buf: &Buffer, x: u16, y: u16) -> Color {
        buf.cell(Position::new(x, y))
            .map(|c| c.fg)
            .unwrap_or(Color::Reset)
    }

    #[test]
    fn focused_input_has_yellow_border() {
        let fb = FocusableBlock::new(FocusStyle::Input).focused(true);
        let block = fb.to_block();
        let buf = render_block(block);
        assert_eq!(
            border_fg_at(&buf, 0, 0),
            Color::Yellow,
            "focused Input block should have Yellow border"
        );
    }

    #[test]
    fn unfocused_interactive_has_dark_gray_border() {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(false);
        let block = fb.to_block();
        let buf = render_block(block);
        assert_eq!(
            border_fg_at(&buf, 0, 0),
            Color::DarkGray,
            "unfocused Content block should have DarkGray border"
        );
    }

    #[test]
    fn display_only_has_dim_border() {
        let fb = FocusableBlock::new(FocusStyle::DisplayOnly);
        let block = fb.to_block();
        let buf = render_block(block);
        assert_eq!(
            border_fg_at(&buf, 0, 0),
            Color::DarkGray,
            "DisplayOnly block should always have DarkGray border"
        );
    }

    #[test]
    fn title_renders_in_border() {
        let fb = FocusableBlock::new(FocusStyle::Content)
            .focused(true)
            .title("Search");
        let block = fb.to_block();
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);

        // Collect all text from the buffer's top row to check for title presence
        let top_row: String = (0..area.width)
            .map(|x| {
                buf.cell(Position::new(x, 0))
                    .map(|c| c.symbol().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();

        assert!(
            top_row.contains("Search"),
            "title 'Search' should appear in the top border row, got: {top_row:?}"
        );
    }

    #[test]
    fn focused_content_has_cyan_border() {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(true);
        let block = fb.to_block();
        let buf = render_block(block);
        assert_eq!(
            border_fg_at(&buf, 0, 0),
            Color::Cyan,
            "focused Content block should have Cyan border"
        );
    }

    #[test]
    fn display_only_focused_still_dark_gray() {
        // DisplayOnly should ignore focus state
        let fb = FocusableBlock::new(FocusStyle::DisplayOnly).focused(true);
        let block = fb.to_block();
        let buf = render_block(block);
        assert_eq!(
            border_fg_at(&buf, 0, 0),
            Color::DarkGray,
            "DisplayOnly focused block should still have DarkGray border"
        );
    }

    #[test]
    fn terminal_render_smoke_test() {
        // Smoke test: ensure it renders without panicking via a Terminal
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let fb = FocusableBlock::new(FocusStyle::Input)
                    .focused(true)
                    .title("Test");
                let block = fb.to_block();
                frame.render_widget(block, frame.area());
            })
            .unwrap();
    }
}
