use ratatui::prelude::*;

const DASH: &str = "─";

pub struct SectionSeparator {
    width: u16,
    label: Option<String>,
}

impl SectionSeparator {
    pub fn new(width: u16) -> Self {
        Self { width, label: None }
    }

    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    pub fn to_line(&self) -> Line<'_> {
        let style = Style::default().fg(Color::DarkGray);

        match &self.label {
            None => {
                let dashes = DASH.repeat(self.width as usize);
                Line::from(Span::styled(dashes, style))
            }
            Some(label) => {
                // Format: "── <label> ────────────"
                // "── " (3 chars) + label + " " (1 char) + trailing dashes
                let label_len = label.chars().count();
                let used = 3 + label_len + 1;
                let trailing_count = (self.width as usize).saturating_sub(used);

                let text = format!(
                    "{} {} {}",
                    DASH.repeat(2),
                    label,
                    DASH.repeat(trailing_count)
                );
                Line::from(Span::styled(text, style))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_without_label_is_just_dashes() {
        let sep = SectionSeparator::new(30);
        let line = sep.to_line();
        assert_eq!(line.spans.len(), 1);
        let span_content = &line.spans[0].content;
        assert!(
            span_content.contains('─'),
            "expected dashes, got: {span_content:?}"
        );
        assert!(
            !span_content.contains(' '),
            "no spaces expected without label"
        );
    }

    #[test]
    fn separator_with_label_embeds_text() {
        let sep = SectionSeparator::new(40).label("4 results");
        let line = sep.to_line();
        let full_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            full_text.contains("4 results"),
            "expected label in output, got: {full_text:?}"
        );
        assert!(
            full_text.contains('─'),
            "expected dashes in output, got: {full_text:?}"
        );
    }
}
