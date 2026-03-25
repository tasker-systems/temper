use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::prelude::*;
use std::mem;

/// Parse markdown input and produce styled ratatui `Line`s for display.
#[allow(dead_code)]
pub fn render_markdown(input: &str) -> Vec<Line<'static>> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code_block = false;
    let mut in_heading = false;
    let mut in_list_item = false;
    let mut is_first_block = true;

    let options = Options::empty();
    let parser = Parser::new_ext(input, options);

    let current_style = |stack: &Vec<Style>| -> Style { stack.last().copied().unwrap_or_default() };

    let flush_line = |current_spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>| {
        if !current_spans.is_empty() {
            lines.push(Line::from(mem::take(current_spans)));
        }
    };

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_line(&mut current_spans, &mut lines);
                if !is_first_block {
                    lines.push(Line::from(vec![]));
                }
                is_first_block = false;
                in_heading = true;
                let heading_style = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(Color::Yellow),
                };
                style_stack.push(heading_style);
            }

            Event::End(TagEnd::Heading(_)) => {
                flush_line(&mut current_spans, &mut lines);
                style_stack.pop();
                in_heading = false;
            }

            Event::Start(Tag::Strong) => {
                let strong_style = Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD);
                style_stack.push(strong_style);
            }

            Event::End(TagEnd::Strong) => {
                style_stack.pop();
            }

            Event::Start(Tag::Emphasis) => {
                let em_style = Style::default().add_modifier(Modifier::ITALIC);
                style_stack.push(em_style);
            }

            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }

            Event::Start(Tag::Link { .. }) => {
                let link_style = Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED);
                style_stack.push(link_style);
            }

            Event::End(TagEnd::Link) => {
                style_stack.pop();
            }

            Event::Start(Tag::BlockQuote(_)) => {
                let quote_style = Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::ITALIC);
                style_stack.push(quote_style);
            }

            Event::End(TagEnd::BlockQuote(_)) => {
                flush_line(&mut current_spans, &mut lines);
                style_stack.pop();
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                flush_line(&mut current_spans, &mut lines);
                if !is_first_block {
                    lines.push(Line::from(vec![]));
                }
                is_first_block = false;
                in_code_block = true;

                // Show language label if fenced
                if let CodeBlockKind::Fenced(lang) = &kind {
                    if !lang.is_empty() {
                        let lang_owned = lang.to_string();
                        let label_span = Span::styled(
                            format!("│ {}", lang_owned),
                            Style::default().fg(Color::DarkGray),
                        );
                        lines.push(Line::from(vec![label_span]));
                    }
                }
            }

            Event::End(TagEnd::CodeBlock) => {
                flush_line(&mut current_spans, &mut lines);
                in_code_block = false;
            }

            Event::Start(Tag::Item) => {
                flush_line(&mut current_spans, &mut lines);
                in_list_item = true;
            }

            Event::End(TagEnd::Item) => {
                flush_line(&mut current_spans, &mut lines);
                in_list_item = false;
            }

            Event::Start(Tag::Paragraph) => {
                flush_line(&mut current_spans, &mut lines);
                if !is_first_block {
                    lines.push(Line::from(vec![]));
                }
            }

            Event::End(TagEnd::Paragraph) => {
                flush_line(&mut current_spans, &mut lines);
                is_first_block = false;
            }

            Event::Start(_) | Event::End(_) => {
                // Ignore other start/end events
            }

            Event::Text(text) => {
                if in_code_block {
                    // Split multi-line code blocks into individual lines with │ prefix
                    let text_str = text.to_string();
                    let mut code_lines: Vec<&str> = text_str.split('\n').collect();
                    // Remove trailing empty line that pulldown-cmark adds
                    if code_lines.last() == Some(&"") {
                        code_lines.pop();
                    }
                    for code_line in code_lines {
                        let prefix = Span::styled("│ ", Style::default().fg(Color::DarkGray));
                        let code_span =
                            Span::styled(code_line.to_string(), Style::default().fg(Color::Green));
                        lines.push(Line::from(vec![prefix, code_span]));
                    }
                } else {
                    let style = current_style(&style_stack);
                    let text_owned = text.to_string();
                    if in_list_item && current_spans.is_empty() {
                        // Add bullet prefix before the first span in a list item
                        let bullet = Span::styled("• ", Style::default().fg(Color::DarkGray));
                        current_spans.push(bullet);
                    }
                    current_spans.push(Span::styled(text_owned, style));
                }
            }

            Event::Code(code) => {
                let code_owned = code.to_string();
                current_spans.push(Span::styled(code_owned, Style::default().fg(Color::Green)));
            }

            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut current_spans, &mut lines);
            }

            Event::Rule => {
                flush_line(&mut current_spans, &mut lines);
                lines.push(Line::from(vec![Span::styled(
                    "────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )]));
            }

            _ => {}
        }

        // Silence unused variable warning; `in_heading` is maintained for potential future use
        let _ = in_heading;
    }

    flush_line(&mut current_spans, &mut lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_renders_yellow_bold() {
        let lines = render_markdown("# Hello");
        let span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("Hello"))
            .expect("should contain Hello");
        assert_eq!(span.style.fg, Some(Color::Yellow));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn bold_text_renders_white_bold() {
        let lines = render_markdown("some **bold** text");
        let span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("bold"))
            .expect("should contain bold");
        assert_eq!(span.style.fg, Some(Color::White));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inline_code_renders_green() {
        let lines = render_markdown("use `foo` here");
        let span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("foo"))
            .expect("should contain foo");
        assert_eq!(span.style.fg, Some(Color::Green));
    }

    #[test]
    fn list_items_get_bullet_prefix() {
        let lines = render_markdown("- item one\n- item two");
        let line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("item one")))
            .expect("should have item one");
        let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(full.contains('•'));
    }

    #[test]
    fn link_renders_cyan() {
        let lines = render_markdown("[click](http://example.com)");
        let span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("click"))
            .expect("should contain link text");
        assert_eq!(span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn code_block_has_left_bar() {
        let lines = render_markdown("```rust\nlet x = 1;\n```");
        let code_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("let x")))
            .expect("should contain code");
        assert!(code_line.spans.len() >= 2); // prefix + code
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(render_markdown("").is_empty());
    }
}
