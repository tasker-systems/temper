use ratatui::prelude::*;

/// A breadcrumb navigation bar with pill-style segments of increasing saturation.
///
/// Depth-based styling:
/// - Depth 0 (root, e.g. "All"): DarkGray, Normal weight
/// - Depth 1 (e.g. project name): Gray, Normal weight
/// - Depth 2+ (active segment): Yellow, Bold
///
/// The last segment is always Yellow+Bold when there is more than one segment.
/// Segments are separated by ` › ` (U+203A) chevrons in DarkGray.
pub struct BreadcrumbBar {
    segments: Vec<String>,
}

#[allow(dead_code)]
impl BreadcrumbBar {
    /// Create a new `BreadcrumbBar` from a slice of segment label strings.
    pub fn new(segments: &[&str]) -> Self {
        Self {
            segments: segments.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Build a styled ratatui `Line` suitable for rendering in a 1-line area.
    pub fn to_line(&self) -> Line<'static> {
        if self.segments.is_empty() {
            return Line::default();
        }

        let chevron = Span::styled(
            " \u{203a} ".to_string(),
            Style::default().fg(Color::DarkGray),
        );

        let last_idx = self.segments.len() - 1;
        let mut spans: Vec<Span<'static>> = Vec::new();

        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                spans.push(chevron.clone());
            }

            let style = segment_style(i, last_idx);
            spans.push(Span::styled(segment.clone(), style));
        }

        Line::from(spans)
    }
}

/// Determine the style for a segment at position `idx` out of `last_idx`.
///
/// If there is only one segment (last_idx == 0), the root style (DarkGray) is used.
/// When there are multiple segments, the last segment is always Yellow+Bold.
fn segment_style(idx: usize, last_idx: usize) -> Style {
    // Single segment — always root style
    if last_idx == 0 {
        return Style::default().fg(Color::DarkGray);
    }

    // Last segment of a multi-segment bar — always active (Yellow+Bold)
    if idx == last_idx {
        return Style::default().fg(Color::Yellow).bold();
    }

    // Intermediate segments: depth 0 → DarkGray, depth 1+ → Gray
    if idx == 0 {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Gray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    #[test]
    fn single_segment_renders_root_style() {
        let bar = BreadcrumbBar::new(&["All"]);
        let line = bar.to_line();
        // Only 1 span — no chevrons
        assert_eq!(
            line.spans.len(),
            1,
            "expected exactly 1 span for single segment"
        );
        let span = &line.spans[0];
        assert_eq!(span.content, "All");
        assert_eq!(
            span.style.fg,
            Some(Color::DarkGray),
            "single segment should use DarkGray (root style)"
        );
    }

    #[test]
    fn three_segments_have_chevron_separators() {
        let bar = BreadcrumbBar::new(&["All", "temper", "viz"]);
        let line = bar.to_line();
        // 3 segments + 2 chevrons = 5 spans
        assert_eq!(
            line.spans.len(),
            5,
            "expected 5 spans: 3 segments + 2 chevrons; got {}",
            line.spans.len()
        );
        // Verify chevron positions (indices 1 and 3)
        assert!(
            line.spans[1].content.contains('\u{203a}'),
            "span[1] should be a › chevron"
        );
        assert!(
            line.spans[3].content.contains('\u{203a}'),
            "span[3] should be a › chevron"
        );
        // Verify segment label positions
        assert_eq!(line.spans[0].content, "All");
        assert_eq!(line.spans[2].content, "temper");
        assert_eq!(line.spans[4].content, "viz");
    }

    #[test]
    fn last_segment_is_yellow_bold() {
        let bar = BreadcrumbBar::new(&["All", "temper", "viz"]);
        let line = bar.to_line();
        // Last segment is at index 4 (span index for "viz")
        let last_span = &line.spans[4];
        assert_eq!(
            last_span.style.fg,
            Some(Color::Yellow),
            "last segment should have Yellow foreground"
        );
        assert!(
            last_span.style.add_modifier.contains(Modifier::BOLD),
            "last segment should have BOLD modifier"
        );
    }
}
