//! Table renderer for `Pretty` and `NoTty` output formats.
//!
//! `Pretty` uses markdown-style pipe tables with a `---` header separator
//! and bold headers via `anstyle`. `NoTty` uses tab-delimited output with
//! no borders, no ANSI, one line per row.

use std::fmt::Write;

use anstyle::{Effects, Style};

/// Column alignment for the pretty renderer. `NoTty` ignores alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}

/// A single column definition.
#[derive(Debug, Clone)]
pub struct Column {
    pub header: String,
    pub min_width: usize,
    pub alignment: Alignment,
}

impl Column {
    pub fn new(header: impl Into<String>, min_width: usize, alignment: Alignment) -> Self {
        Self {
            header: header.into(),
            min_width,
            alignment,
        }
    }
}

/// Renders a table of string cells to `Pretty` or `NoTty` output.
#[derive(Debug, Default)]
pub struct TableRenderer {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
}

impl TableRenderer {
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    pub fn push_row(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Compute the actual width of each column based on header + cell lengths.
    fn column_widths(&self) -> Vec<usize> {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let max_cell = self
                    .rows
                    .iter()
                    .map(|r| r.get(i).map(|c| c.len()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                col.min_width.max(col.header.len()).max(max_cell)
            })
            .collect()
    }

    /// Render a markdown-style pipe table with bold headers.
    ///
    /// Bold is applied via `anstyle`. `anstream` at the callsite strips the
    /// escapes on non-terminal stdout, so this output is safe to always
    /// produce when the caller requested `Pretty`.
    pub fn render_pretty(&self) -> String {
        let widths = self.column_widths();
        let bold: Style = Style::new().effects(Effects::BOLD);
        let mut out = String::new();

        // Header row
        out.push('|');
        for (col, w) in self.columns.iter().zip(widths.iter()) {
            let padded = pad(&col.header, *w, col.alignment);
            let _ = write!(out, " {bold}{padded}{bold:#} |");
        }
        out.push('\n');

        // Separator row (uses dashes of the full width per column)
        out.push('|');
        for w in &widths {
            let _ = write!(out, "{}|", "-".repeat(w + 2));
        }
        out.push('\n');

        // Data rows
        for row in &self.rows {
            out.push('|');
            for (i, col) in self.columns.iter().enumerate() {
                let empty = String::new();
                let cell = row.get(i).unwrap_or(&empty);
                let padded = pad(cell, widths[i], col.alignment);
                let _ = write!(out, " {padded} |");
            }
            out.push('\n');
        }

        out
    }

    /// Render a tab-delimited table with headers on the first line.
    ///
    /// No ANSI, no padding, no borders.
    pub fn render_no_tty(&self) -> String {
        let mut out = String::new();
        let headers: Vec<&str> = self.columns.iter().map(|c| c.header.as_str()).collect();
        out.push_str(&headers.join("\t"));
        out.push('\n');
        for row in &self.rows {
            out.push_str(&row.join("\t"));
            out.push('\n');
        }
        out
    }
}

fn pad(text: &str, width: usize, align: Alignment) -> String {
    if text.len() >= width {
        return text.to_string();
    }
    let pad = " ".repeat(width - text.len());
    match align {
        Alignment::Left => format!("{text}{pad}"),
        Alignment::Right => format!("{pad}{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TableRenderer {
        let cols = vec![
            Column::new("Context", 7, Alignment::Left),
            Column::new("Slug", 4, Alignment::Left),
            Column::new("Seq", 3, Alignment::Right),
        ];
        let mut t = TableRenderer::new(cols);
        t.push_row(vec!["temper".into(), "first".into(), "1".into()]);
        t.push_row(vec!["writing".into(), "second".into(), "12".into()]);
        t
    }

    #[test]
    fn pretty_has_header_and_separator() {
        let out = sample().render_pretty();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "header + separator + 2 rows: {out}");
        assert!(lines[0].starts_with("|"), "header starts with pipe");
        assert!(lines[1].contains("---"), "separator contains dashes");
    }

    #[test]
    fn pretty_pads_to_longest_cell_in_column() {
        let out = sample().render_pretty();
        // "writing" is 7 chars; "Context" is 7; column width should be max(7, 7) = 7
        // "second" is 6 chars; "Slug" is 4; column width should be 6
        assert!(
            out.contains("| writing |"),
            "writing should be padded to 7: {out}"
        );
        assert!(
            out.contains("| second |"),
            "second should be padded to 6: {out}"
        );
    }

    #[test]
    fn pretty_right_aligns_numeric_column() {
        let out = sample().render_pretty();
        assert!(
            out.contains("|   1 |"),
            "seq '1' should right-align in width 3: {out}"
        );
        assert!(
            out.contains("|  12 |"),
            "seq '12' should right-align in width 3: {out}"
        );
    }

    #[test]
    fn pretty_empty_rows_still_renders_header() {
        let t = TableRenderer::new(vec![Column::new("A", 1, Alignment::Left)]);
        let out = t.render_pretty();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "header + separator only");
    }

    #[test]
    fn no_tty_uses_tabs_with_header_line() {
        let out = sample().render_no_tty();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 rows");
        assert_eq!(lines[0], "Context\tSlug\tSeq");
        assert_eq!(lines[1], "temper\tfirst\t1");
        assert_eq!(lines[2], "writing\tsecond\t12");
    }

    #[test]
    fn no_tty_empty_rows_only_emits_header() {
        let t = TableRenderer::new(vec![Column::new("A", 1, Alignment::Left)]);
        let out = t.render_no_tty();
        assert_eq!(out, "A\n");
    }
}
