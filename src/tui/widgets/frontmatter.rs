use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render YAML frontmatter as a styled key-value block inside a "Metadata" border.
pub fn render_frontmatter(frame: &mut Frame, area: Rect, frontmatter: &serde_yaml::Value) {
    let mapping = match frontmatter {
        serde_yaml::Value::Mapping(m) => m,
        _ => {
            // Nothing to show — render an empty bordered block
            let block = Block::default()
                .title("Metadata")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));
            frame.render_widget(block, area);
            return;
        }
    };

    let mut lines: Vec<Line> = Vec::new();

    for (k, v) in mapping {
        let key_str = match k {
            serde_yaml::Value::String(s) => s.clone(),
            other => format!("{:?}", other),
        };

        // Skip null/empty values
        if matches!(v, serde_yaml::Value::Null) {
            continue;
        }
        let val_str = value_to_string(v);
        if val_str.trim().is_empty() {
            continue;
        }

        let key_span = Span::styled(format!("{}: ", key_str), Style::default().fg(Color::Cyan));

        let val_color = value_color(&key_str, &val_str);
        let val_span = Span::styled(val_str, Style::default().fg(val_color));

        lines.push(Line::from(vec![key_span, val_span]));
    }

    let block = Block::default()
        .title("Metadata")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn value_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>()
            .join(", "),
        serde_yaml::Value::Null => String::new(),
        other => format!("{:?}", other),
    }
}

fn value_color(key: &str, val: &str) -> Color {
    match key {
        "stage" => match val {
            "done" => Color::Green,
            "in-progress" | "in_progress" => Color::Yellow,
            _ => Color::Reset,
        },
        "scope" => match val {
            "patch" => Color::Blue,
            "feature" => Color::Yellow,
            "epic" => Color::Magenta,
            _ => Color::Reset,
        },
        "type" => match val {
            "bug" => Color::Red,
            "feature" | "feat" => Color::Cyan,
            "chore" | "task" => Color::DarkGray,
            "doc" | "docs" => Color::Blue,
            _ => Color::White,
        },
        _ => Color::White,
    }
}
