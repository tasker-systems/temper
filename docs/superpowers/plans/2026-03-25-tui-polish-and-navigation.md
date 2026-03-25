# TUI Polish and Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the Temper TUI into a visually consistent, spatially clear interface with shared visual widgets, Tab/Shift-Tab focus cycling, project-level navigation, and rendered markdown in the viewer.

**Architecture:** Build a shared visual toolkit (FocusableBlock, BreadcrumbBar, SectionSeparator, MarkdownRenderer) first, then apply these widgets across all tabs. Refactor app.rs into focused modules. Add FocusRegion model for Tab/Shift-Tab cycling. Rename Board→Projects with project list as default view. Use pulldown-cmark for markdown rendering in the viewer.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, pulldown-cmark (new), tokio

**Spec:** `docs/superpowers/specs/2026-03-25-tui-polish-and-navigation-design.md`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `src/tui/widgets/focusable_block.rs` | Border wrapper that changes style based on focus state |
| `src/tui/widgets/breadcrumb_bar.rs` | Pill-style breadcrumb segments with depth-based saturation |
| `src/tui/widgets/section_separator.rs` | Horizontal divider line with optional inline label |
| `src/tui/widgets/markdown_renderer.rs` | Converts markdown text to styled `Vec<Line>` via pulldown-cmark |
| `src/tui/app/mod.rs` | Slimmed App struct, render, dispatch, run — imports submodules |
| `src/tui/app/state.rs` | All state structs (SearchState, ContextState, etc.), enums (Tab, Screen, BoardLevel, PopupState, AppAction) |
| `src/tui/app/actions.rs` | handle_action() match arms, handle_enter(), move_selection(), confirm_popup() |
| `src/tui/app/queries.rs` | handle_query_result(), apply_swimlane_columns(), send_search_query() |
| `src/tui/app/focus.rs` | FocusRegion enum, focus cycling logic, region lists per screen type |
| `src/tui/tabs/projects.rs` | Renamed from board.rs — project list, milestones, swimlanes rendering with new widgets |
| `tests/tui_focus_test.rs` | Focus cycling tests |
| `tests/tui_markdown_test.rs` | Markdown renderer tests |

### Modified files
| File | Changes |
|------|---------|
| `Cargo.toml` | Add pulldown-cmark dependency |
| `src/tui/mod.rs` | Change `mod app;` to directory module, rename board references |
| `src/tui/widgets/mod.rs` | Add new widget module exports |
| `src/tui/tabs/mod.rs` | Rename `board` to `projects` |
| `src/tui/tabs/search.rs` | Use FocusableBlock + SectionSeparator |
| `src/tui/tabs/context.rs` | Use FocusableBlock + SectionSeparator |
| `src/tui/tabs/maintain.rs` | Use FocusableBlock + SectionSeparator |
| `src/tui/views/viewer.rs` | Add BreadcrumbBar, FocusableBlock, MarkdownRenderer, offset background |
| `src/tui/widgets/keyhints.rs` | Update hints for Tab/Shift-Tab, Projects rename |
| `src/tui/widgets/swimlane.rs` | Use FocusableBlock instead of direct Block |
| `src/tui/event.rs` | Add Tab/Shift-Tab mapping, update command parser for Projects rename |
| `tests/tui_state_test.rs` | Update Tab::Board → Tab::Projects, command parser tests |

### Deleted files
| File | Reason |
|------|--------|
| `src/tui/tabs/board.rs` | Renamed to `src/tui/tabs/projects.rs` |
| `src/tui/app.rs` | Replaced by `src/tui/app/` directory module |

---

## Task 1: Add pulldown-cmark dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add pulldown-cmark to Cargo.toml**

Add under `[dependencies]`:

```toml
pulldown-cmark = "0.13"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: Compiles successfully, pulldown-cmark downloaded and linked.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add pulldown-cmark for markdown rendering in TUI viewer"
```

---

## Task 2: FocusableBlock widget

**Files:**
- Create: `src/tui/widgets/focusable_block.rs`
- Modify: `src/tui/widgets/mod.rs`
- Test: inline unit tests in `focusable_block.rs`

- [ ] **Step 1: Write failing test**

Create `src/tui/widgets/focusable_block.rs` with test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn focused_input_has_yellow_border() {
        let block = FocusableBlock::new(FocusStyle::Input).focused(true);
        let rendered = block.to_block();
        // The block should have yellow border
        assert_eq!(rendered.style().fg, Some(Color::Yellow));
    }

    #[test]
    fn unfocused_interactive_has_dark_gray_border() {
        let block = FocusableBlock::new(FocusStyle::Content).focused(false);
        let rendered = block.to_block();
        assert_eq!(rendered.style().fg, Some(Color::DarkGray));
    }

    #[test]
    fn display_only_has_dim_border() {
        let block = FocusableBlock::new(FocusStyle::DisplayOnly);
        let rendered = block.to_block();
        assert_eq!(rendered.style().fg, Some(Color::DarkGray));
    }

    #[test]
    fn title_renders_in_border() {
        let block = FocusableBlock::new(FocusStyle::Input)
            .focused(true)
            .title("Search");
        let rendered = block.to_block();
        // Block should have title set
        assert!(format!("{:?}", rendered).contains("Search"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib focusable_block -- --nocapture 2>&1 | head -30`
Expected: FAIL — `FocusableBlock` not defined.

- [ ] **Step 3: Implement FocusableBlock**

Add above the test module in `src/tui/widgets/focusable_block.rs`:

```rust
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding};

/// Determines the color scheme for a focusable block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusStyle {
    /// Yellow border when focused (for input fields).
    Input,
    /// Cyan border when focused (for content regions like result lists).
    Content,
    /// Always dim DarkGray border (read-only display, not a tab-stop).
    DisplayOnly,
}

/// A wrapper around ratatui's `Block` that changes border style based on focus.
#[derive(Debug, Clone)]
pub struct FocusableBlock<'a> {
    style: FocusStyle,
    focused: bool,
    title: Option<&'a str>,
}

impl<'a> FocusableBlock<'a> {
    pub fn new(style: FocusStyle) -> Self {
        Self {
            style,
            focused: false,
            title: None,
        }
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Build a ratatui `Block` with the appropriate border styling.
    pub fn to_block(&self) -> Block<'a> {
        let border_color = match (self.style, self.focused) {
            (FocusStyle::Input, true) => Color::Yellow,
            (FocusStyle::Content, true) => Color::Cyan,
            (FocusStyle::DisplayOnly, _) => Color::DarkGray,
            (_, false) => Color::DarkGray,
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .padding(Padding::horizontal(1));

        if let Some(t) = self.title {
            block = block.title(t);
        }

        block
    }
}
```

- [ ] **Step 4: Add module export**

In `src/tui/widgets/mod.rs`, add:

```rust
pub mod focusable_block;
```

Making the full file:

```rust
pub mod command_line;
pub mod focusable_block;
pub mod frontmatter;
pub mod keyhints;
pub mod result_list;
pub mod swimlane;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib focusable_block -- --nocapture`
Expected: All 4 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/widgets/focusable_block.rs src/tui/widgets/mod.rs
git commit -m "feat(tui): add FocusableBlock widget with focus-aware border styling"
```

---

## Task 3: BreadcrumbBar widget

**Files:**
- Create: `src/tui/widgets/breadcrumb_bar.rs`
- Modify: `src/tui/widgets/mod.rs`
- Test: inline unit tests

- [ ] **Step 1: Write failing test**

Create `src/tui/widgets/breadcrumb_bar.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_segment_renders_root_style() {
        let bar = BreadcrumbBar::new(&["All"]);
        let line = bar.to_line();
        assert_eq!(line.spans.len(), 1); // just the one pill
    }

    #[test]
    fn three_segments_have_chevron_separators() {
        let bar = BreadcrumbBar::new(&["All", "temper", "visualization-qol"]);
        let line = bar.to_line();
        // segments + chevrons: "All" + " › " + "temper" + " › " + "viz-qol"
        assert_eq!(line.spans.len(), 5);
    }

    #[test]
    fn last_segment_is_yellow_bold() {
        let bar = BreadcrumbBar::new(&["All", "temper", "viz"]);
        let line = bar.to_line();
        let last = line.spans.last().unwrap();
        assert_eq!(last.style.fg, Some(Color::Yellow));
        assert!(last.style.add_modifier.contains(Modifier::BOLD));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib breadcrumb_bar -- --nocapture 2>&1 | head -20`
Expected: FAIL — `BreadcrumbBar` not defined.

- [ ] **Step 3: Implement BreadcrumbBar**

Add above tests in `src/tui/widgets/breadcrumb_bar.rs`:

```rust
use ratatui::prelude::*;

/// Depth-based background colors for breadcrumb pills.
const DEPTH_COLORS: &[(Color, Color)] = &[
    // (background, foreground) — background is for future use if ratatui supports cell bg
    (Color::Rgb(30, 32, 64), Color::DarkGray),   // depth 0: root
    (Color::Rgb(37, 37, 80), Color::Gray),        // depth 1: project
    (Color::Rgb(42, 42, 106), Color::Yellow),     // depth 2+: active
];

/// A breadcrumb bar with pill-style segments of increasing saturation.
pub struct BreadcrumbBar<'a> {
    segments: &'a [&'a str],
}

impl<'a> BreadcrumbBar<'a> {
    pub fn new(segments: &'a [&'a str]) -> Self {
        Self { segments }
    }

    /// Build a styled `Line` for rendering in a 1-line area.
    pub fn to_line(&self) -> Line<'a> {
        let mut spans: Vec<Span<'a>> = Vec::new();

        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)));
            }

            let is_last = i == self.segments.len() - 1;
            let depth_idx = i.min(DEPTH_COLORS.len() - 1);
            let (_bg, fg) = DEPTH_COLORS[depth_idx];

            let style = if is_last && self.segments.len() > 1 {
                // Active segment: always yellow + bold regardless of depth
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default().fg(fg)
            };

            spans.push(Span::styled(*segment, style));
        }

        Line::from(spans)
    }
}
```

- [ ] **Step 4: Add module export**

In `src/tui/widgets/mod.rs`, add `pub mod breadcrumb_bar;` (alphabetical order).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib breadcrumb_bar -- --nocapture`
Expected: All 3 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/widgets/breadcrumb_bar.rs src/tui/widgets/mod.rs
git commit -m "feat(tui): add BreadcrumbBar widget with depth-based pill styling"
```

---

## Task 4: SectionSeparator widget

**Files:**
- Create: `src/tui/widgets/section_separator.rs`
- Modify: `src/tui/widgets/mod.rs`
- Test: inline unit tests

- [ ] **Step 1: Write failing test**

Create `src/tui/widgets/section_separator.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_without_label_is_just_dashes() {
        let sep = SectionSeparator::new(30);
        let line = sep.to_line();
        assert_eq!(line.spans.len(), 1);
        assert!(line.spans[0].content.contains('─'));
    }

    #[test]
    fn separator_with_label_embeds_text() {
        let sep = SectionSeparator::new(40).label("4 results");
        let line = sep.to_line();
        // Should have: "── 4 results ──────..."
        let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(full.contains("4 results"));
        assert!(full.contains('─'));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib section_separator -- --nocapture 2>&1 | head -20`
Expected: FAIL — `SectionSeparator` not defined.

- [ ] **Step 3: Implement SectionSeparator**

```rust
use ratatui::prelude::*;

/// A horizontal divider line with optional inline label.
pub struct SectionSeparator<'a> {
    width: u16,
    label: Option<&'a str>,
}

impl<'a> SectionSeparator<'a> {
    pub fn new(width: u16) -> Self {
        Self { width, label: None }
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Build a styled `Line` representing the separator.
    pub fn to_line(&self) -> Line<'a> {
        let sep_style = Style::default().fg(Color::DarkGray);

        match self.label {
            None => {
                let dashes: String = "─".repeat(self.width as usize);
                Line::from(Span::styled(dashes, sep_style))
            }
            Some(label) => {
                let prefix = "── ";
                let suffix_pad = 1; // space after label
                let used = prefix.len() + label.len() + suffix_pad + 1; // +1 for space before trailing dashes
                let remaining = (self.width as usize).saturating_sub(used);
                let trailing: String = "─".repeat(remaining.max(1));

                Line::from(vec![
                    Span::styled(prefix, sep_style),
                    Span::styled(label, Style::default().fg(Color::DarkGray)),
                    Span::styled(format!(" {}", trailing), sep_style),
                ])
            }
        }
    }
}
```

- [ ] **Step 4: Add module export**

In `src/tui/widgets/mod.rs`, add `pub mod section_separator;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib section_separator -- --nocapture`
Expected: All 2 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/widgets/section_separator.rs src/tui/widgets/mod.rs
git commit -m "feat(tui): add SectionSeparator widget with optional inline label"
```

---

## Task 5: MarkdownRenderer widget

**Files:**
- Create: `src/tui/widgets/markdown_renderer.rs`
- Modify: `src/tui/widgets/mod.rs`
- Test: inline unit tests

- [ ] **Step 1: Write failing tests**

Create `src/tui/widgets/markdown_renderer.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn heading_renders_yellow_bold() {
        let lines = render_markdown("# Hello");
        assert!(!lines.is_empty());
        let first_content = lines.iter().find(|l| {
            l.spans.iter().any(|s| s.content.contains("Hello"))
        }).expect("should contain Hello");
        let span = first_content.spans.iter().find(|s| s.content.contains("Hello")).unwrap();
        assert_eq!(span.style.fg, Some(Color::Yellow));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn bold_text_renders_white_bold() {
        let lines = render_markdown("some **bold** text");
        let bold_span = lines.iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("bold"))
            .expect("should contain bold");
        assert_eq!(bold_span.style.fg, Some(Color::White));
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inline_code_renders_green() {
        let lines = render_markdown("use `foo` here");
        let code_span = lines.iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("foo"))
            .expect("should contain foo");
        assert_eq!(code_span.style.fg, Some(Color::Green));
    }

    #[test]
    fn list_items_get_bullet_prefix() {
        let lines = render_markdown("- item one\n- item two");
        let bullet_line = lines.iter().find(|l| {
            l.spans.iter().any(|s| s.content.contains("•") || s.content.contains("item one"))
        }).expect("should have bullet line");
        let full: String = bullet_line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(full.contains('•') || full.contains('-'));
    }

    #[test]
    fn link_renders_cyan() {
        let lines = render_markdown("[click](url)");
        let link_span = lines.iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("click"))
            .expect("should contain link text");
        assert_eq!(link_span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn code_block_has_left_bar_indicator() {
        let md = "```rust\nlet x = 1;\n```";
        let lines = render_markdown(md);
        // Code block lines should have some visual indicator
        let code_line = lines.iter().find(|l| {
            l.spans.iter().any(|s| s.content.contains("let x"))
        }).expect("should contain code");
        // Should have a prefix span for the left bar
        assert!(code_line.spans.len() >= 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        let lines = render_markdown("");
        assert!(lines.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib markdown_renderer -- --nocapture 2>&1 | head -20`
Expected: FAIL — `render_markdown` not defined.

- [ ] **Step 3: Implement MarkdownRenderer**

```rust
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, HeadingLevel, CodeBlockKind};
use ratatui::prelude::*;

/// Render markdown text into a vector of styled ratatui Lines.
pub fn render_markdown(input: &str) -> Vec<Line<'static>> {
    if input.is_empty() {
        return Vec::new();
    }

    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(input, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut in_code_block = false;
    let mut in_heading = false;
    let mut in_list_item = false;
    let mut list_item_started = false;

    let code_block_style = Style::default().fg(Color::Green);
    let code_block_prefix = Span::styled("  │ ", Style::default().fg(Color::DarkGray));

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut lines, &mut current_spans);
                    // Blank line before headings (unless it's the first line)
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    in_heading = true;
                    let heading_style = match level {
                        HeadingLevel::H1 | HeadingLevel::H2 => {
                            Style::default().fg(Color::Yellow).bold()
                        }
                        _ => Style::default().fg(Color::Yellow),
                    };
                    style_stack.push(heading_style);
                }
                Tag::Strong => {
                    style_stack.push(Style::default().fg(Color::White).bold());
                }
                Tag::Emphasis => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.add_modifier(Modifier::ITALIC));
                }
                Tag::CodeBlock(kind) => {
                    flush_line(&mut lines, &mut current_spans);
                    in_code_block = true;
                    // Show language label if available
                    if let CodeBlockKind::Fenced(lang) = &kind {
                        let lang_str = lang.to_string();
                        if !lang_str.is_empty() {
                            lines.push(Line::from(vec![
                                code_block_prefix.clone(),
                                Span::styled(lang_str, Style::default().fg(Color::DarkGray)),
                            ]));
                        }
                    }
                }
                Tag::Link { dest_url, .. } => {
                    style_stack.push(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                    // Store URL for potential display — for now we just style the text
                    let _ = dest_url;
                }
                Tag::List(_) => {}
                Tag::Item => {
                    flush_line(&mut lines, &mut current_spans);
                    in_list_item = true;
                    list_item_started = false;
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans);
                    style_stack.push(Style::default().fg(Color::Gray).italic());
                }
                Tag::Paragraph => {
                    // Add blank line between paragraphs (not after headings)
                    if !lines.is_empty() && !in_heading && !in_list_item {
                        lines.push(Line::from(""));
                    }
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    flush_line(&mut lines, &mut current_spans);
                    in_heading = false;
                    style_stack.pop();
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link | TagEnd::BlockQuote => {
                    style_stack.pop();
                    if matches!(tag_end, TagEnd::BlockQuote) {
                        flush_line(&mut lines, &mut current_spans);
                    }
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans);
                    in_list_item = false;
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut current_spans);
                }
                _ => {}
            },
            Event::Text(text) => {
                let text_str = text.to_string();
                if in_code_block {
                    // Each line in a code block gets the left-bar prefix
                    for code_line in text_str.lines() {
                        lines.push(Line::from(vec![
                            code_block_prefix.clone(),
                            Span::styled(code_line.to_string(), code_block_style),
                        ]));
                    }
                } else {
                    let style = current_style(&style_stack);
                    if in_list_item && !list_item_started {
                        current_spans.push(Span::styled(
                            "  • ",
                            Style::default().fg(Color::DarkGray),
                        ));
                        list_item_started = true;
                    }
                    current_spans.push(Span::styled(text_str, style));
                }
            }
            Event::Code(code) => {
                current_spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Green),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            _ => {}
        }
    }

    flush_line(&mut lines, &mut current_spans);
    lines
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}
```

- [ ] **Step 4: Add module export**

In `src/tui/widgets/mod.rs`, add `pub mod markdown_renderer;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib markdown_renderer -- --nocapture`
Expected: All 7 tests PASS.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-features`
Expected: No warnings.

- [ ] **Step 7: Commit**

```bash
git add src/tui/widgets/markdown_renderer.rs src/tui/widgets/mod.rs
git commit -m "feat(tui): add markdown renderer using pulldown-cmark for viewer"
```

---

## Task 6: Extract state types from app.rs

This is a pure structural refactor — move type definitions out of the monolithic `app.rs` into `app/state.rs`. No behavior changes.

**Files:**
- Create: `src/tui/app/` directory
- Create: `src/tui/app/state.rs`
- Create: `src/tui/app/mod.rs` (from existing `src/tui/app.rs`)
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Create app directory and move app.rs**

```bash
mkdir -p src/tui/app
mv src/tui/app.rs src/tui/app/mod.rs
```

- [ ] **Step 2: Verify it compiles unchanged**

Run: `cargo check --all-features`
Expected: Compiles (module resolution unchanged for a `mod.rs` file).

- [ ] **Step 3: Extract state types into state.rs**

Create `src/tui/app/state.rs` containing all type definitions currently at lines 23-179 of app/mod.rs:
- `Tab` enum (rename `Board` variant to `Projects`)
- `BoardLevel` enum
- `MilestoneWithCounts` struct
- `BoardState` struct
- `SearchState` struct
- `ContextNeighbor` struct
- `ContextState` struct
- `MaintainState` struct
- `ViewerState` struct
- `Screen` enum (rename `Board` variant to `Projects`)
- `PopupState` enum
- `AppAction` enum

Add necessary imports at the top of `state.rs`:
```rust
use crate::actions::types::{
    IndexStats, MilestoneInfo, NormalizeSummary, SearchHit, TicketInfo, VaultDocument,
};
```

- [ ] **Step 4: Update app/mod.rs to import from state.rs**

In `src/tui/app/mod.rs`:
- Add `pub mod state;` near the top
- Replace the type definitions block (lines 23-179) with `pub use state::*;`
- Remove the now-redundant imports that `state.rs` owns

- [ ] **Step 5: Rename Tab::Board to Tab::Projects**

In `src/tui/app/state.rs`, change:
```rust
pub enum Tab {
    Projects,  // was Board
    Search,
    Context,
    Maintain,
}
```

And `Screen::Board` to `Screen::Projects`:
```rust
pub enum Screen {
    Projects(BoardState),  // was Board
    Search(SearchState),
    Context(ContextState),
    Maintain(MaintainState),
    Viewer(ViewerState),
}
```

Then fix all references in `app/mod.rs` — search for `Tab::Board` and replace with `Tab::Projects`, search for `Screen::Board` and replace with `Screen::Projects`.

- [ ] **Step 6: Update event.rs for the rename**

In `src/tui/event.rs`:
- `parse_command`: change `"b" | "board"` to `"p" | "projects"`, keep `"b"` as alias for backward compat
- All `Tab::Board` references → `Tab::Projects`

- [ ] **Step 7: Update tests for the rename**

In `tests/tui_state_test.rs`:
- `Tab::Board` → `Tab::Projects`
- `parse_command("board")` → keep as backward-compat alias test, add `parse_command("projects")` test
- Add `parse_command("p")` test

- [ ] **Step 8: Run all tests**

Run: `cargo test --all-features`
Expected: All tests PASS.

- [ ] **Step 9: Commit**

```bash
git add src/tui/app/ src/tui/mod.rs src/tui/event.rs tests/tui_state_test.rs
git commit -m "refactor(tui): extract state types to app/state.rs, rename Board to Projects"
```

---

## Task 7: Extract actions and queries from app/mod.rs

Continue decomposing app/mod.rs into focused modules.

**Files:**
- Create: `src/tui/app/actions.rs`
- Create: `src/tui/app/queries.rs`
- Modify: `src/tui/app/mod.rs`

- [ ] **Step 1: Create actions.rs**

Extract these methods from `App` impl block in `app/mod.rs` into `app/actions.rs`:
- `handle_enter()` (lines ~1072-1198)
- `move_selection()` (lines ~1200-1275)
- `confirm_popup()` (lines ~1001-1048)
- `selected_ticket_identity()` (lines ~962-981)
- `current_swimlane_milestone()` (lines ~984-998)

Structure as an `impl App` block in `actions.rs` with `use super::*;` to access types.

The exact structure:
```rust
use super::state::*;
use super::App;
use crate::tui::query_actor::QueryRequest;

impl App {
    // paste extracted methods here
}
```

- [ ] **Step 2: Create queries.rs**

Extract:
- `handle_query_result()` (lines ~852-923)
- `apply_swimlane_columns()` (lines ~926-956)
- `send_search_query()` (lines ~1052-1070)

Structure similarly with `impl App` block.

- [ ] **Step 3: Add module declarations in app/mod.rs**

```rust
mod actions;
mod queries;
pub mod state;
```

Remove the extracted method bodies from `app/mod.rs`, keeping only `new*`, `from_config`, accessors, `dispatch`, `render`, `make_root_screen`, and `run`.

- [ ] **Step 4: Run all tests**

Run: `cargo test --all-features`
Expected: All tests PASS — purely structural move.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app/
git commit -m "refactor(tui): extract action handlers and query processing into submodules"
```

---

## Task 8: FocusRegion model

**Files:**
- Create: `src/tui/app/focus.rs`
- Modify: `src/tui/app/mod.rs` (add focus field to App)
- Modify: `src/tui/app/state.rs` (add FocusRegion enum)
- Test: `tests/tui_focus_test.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/tui_focus_test.rs`:

```rust
use temper_cli::tui::app::{App, AppAction, Tab};
use temper_cli::tui::app::state::FocusRegion;

#[test]
fn initial_focus_is_content() {
    let app = App::new_for_test();
    assert_ne!(app.focus_region(), FocusRegion::TabBar);
}

#[test]
fn tab_cycles_focus_forward() {
    let mut app = App::new_for_test();
    // Start somewhere in content, Tab should advance
    let initial = app.focus_region();
    app.dispatch(AppAction::FocusNext);
    let after = app.focus_region();
    assert_ne!(initial, after);
}

#[test]
fn shift_tab_cycles_focus_backward() {
    let mut app = App::new_for_test();
    // Move forward then backward should return to start
    let initial = app.focus_region();
    app.dispatch(AppAction::FocusNext);
    app.dispatch(AppAction::FocusPrev);
    assert_eq!(app.focus_region(), initial);
}

#[test]
fn focus_wraps_around() {
    let mut app = App::new_for_test();
    // Cycle through all regions — should eventually return to start
    let initial = app.focus_region();
    for _ in 0..20 {
        app.dispatch(AppAction::FocusNext);
        if app.focus_region() == initial {
            return; // wrapped successfully
        }
    }
    panic!("focus did not wrap after 20 tabs");
}

#[test]
fn tab_switch_resets_focus_to_first_content() {
    let mut app = App::new_for_test();
    app.dispatch(AppAction::FocusNext); // move focus
    app.dispatch(AppAction::SwitchTab(Tab::Search));
    // After switching tabs, focus should be on first content region, not tab bar
    assert_ne!(app.focus_region(), FocusRegion::TabBar);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test tui_focus_test -- --nocapture 2>&1 | head -20`
Expected: FAIL — `FocusRegion` and `FocusNext`/`FocusPrev` not defined.

- [ ] **Step 3: Define FocusRegion in state.rs**

Add to `src/tui/app/state.rs`:

```rust
/// Identifies which UI region currently has focus.
/// Tab/Shift-Tab cycles through regions; arrows navigate within.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusRegion {
    TabBar,
    /// First content region (e.g., project list, search input, topic input, action buttons)
    Primary,
    /// Second content region (e.g., results list, neighbor list)
    Secondary,
    /// Third+ content region (e.g., swimlane columns — 0-indexed)
    Tertiary(usize),
}
```

Add `FocusNext` and `FocusPrev` to `AppAction`:

```rust
// Focus cycling
FocusNext,
FocusPrev,
```

- [ ] **Step 4: Create focus.rs with cycling logic**

Create `src/tui/app/focus.rs`:

```rust
use super::state::*;
use super::App;

impl App {
    /// Get the current focus region.
    pub fn focus_region(&self) -> FocusRegion {
        self.focus
    }

    /// Returns the ordered list of focus regions for the current screen.
    pub fn focus_regions(&self) -> Vec<FocusRegion> {
        match self.current_screen() {
            Screen::Projects(board) => match &board.level {
                BoardLevel::Projects { .. } => {
                    vec![FocusRegion::TabBar, FocusRegion::Primary]
                }
                BoardLevel::Milestones { .. } => {
                    vec![FocusRegion::TabBar, FocusRegion::Primary]
                }
                BoardLevel::Swimlanes { .. } => {
                    vec![
                        FocusRegion::TabBar,
                        FocusRegion::Tertiary(0),
                        FocusRegion::Tertiary(1),
                        FocusRegion::Tertiary(2),
                    ]
                }
            },
            Screen::Search(_) => {
                vec![FocusRegion::TabBar, FocusRegion::Primary, FocusRegion::Secondary]
            }
            Screen::Context(_) => {
                vec![FocusRegion::TabBar, FocusRegion::Primary, FocusRegion::Secondary]
            }
            Screen::Maintain(_) => {
                vec![FocusRegion::TabBar, FocusRegion::Primary]
            }
            Screen::Viewer(_) => {
                vec![FocusRegion::TabBar, FocusRegion::Primary]
            }
        }
    }

    /// Cycle focus to the next region.
    pub fn focus_next(&mut self) {
        let regions = self.focus_regions();
        let current_idx = regions.iter().position(|r| *r == self.focus).unwrap_or(0);
        let next_idx = (current_idx + 1) % regions.len();
        self.focus = regions[next_idx];
        self.sync_focus_to_state();
    }

    /// Cycle focus to the previous region.
    pub fn focus_prev(&mut self) {
        let regions = self.focus_regions();
        let current_idx = regions.iter().position(|r| *r == self.focus).unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            regions.len() - 1
        } else {
            current_idx - 1
        };
        self.focus = regions[prev_idx];
        self.sync_focus_to_state();
    }

    /// After focus changes, synchronize the legacy state flags
    /// (input_focused, input_active, column) so rendering and existing
    /// key handlers work correctly during the migration.
    fn sync_focus_to_state(&mut self) {
        match self.current_screen_mut() {
            Screen::Search(s) => {
                s.input_focused = self.focus == FocusRegion::Primary;
            }
            Screen::Context(s) => {
                s.input_active = self.focus == FocusRegion::Primary;
            }
            Screen::Projects(board) => {
                if let BoardLevel::Swimlanes { column, .. } = &mut board.level {
                    if let FocusRegion::Tertiary(col) = self.focus {
                        *column = col;
                    }
                }
            }
            _ => {}
        }
    }

    /// Reset focus to the first content region (used after tab switches).
    pub fn reset_focus(&mut self) {
        let regions = self.focus_regions();
        // First content region (skip TabBar which is always index 0)
        self.focus = regions.get(1).copied().unwrap_or(FocusRegion::Primary);
        self.sync_focus_to_state();
    }
}
```

- [ ] **Step 5: Add focus field to App struct**

In `src/tui/app/mod.rs`, add to the `App` struct:

```rust
pub focus: FocusRegion,
```

Initialize in `new()`, `new_for_test()`, and `from_config()` as `FocusRegion::Primary`.

Add `mod focus;` to module declarations.

- [ ] **Step 6: Wire FocusNext/FocusPrev into dispatch**

In the `dispatch` method, add cases:

```rust
AppAction::FocusNext => self.focus_next(),
AppAction::FocusPrev => self.focus_prev(),
```

In `SwitchTab` handler, add `self.reset_focus();` after creating the new root screen.

- [ ] **Step 7: Wire Tab/Shift-Tab in event.rs**

In `src/tui/event.rs`, add at the top of `map_key()` (after Ctrl-C but before mode-specific handlers):

```rust
KeyCode::Tab if key.modifiers.is_empty() => return Some(AppAction::FocusNext),
KeyCode::BackTab => return Some(AppAction::FocusPrev),
```

This ensures Tab/Shift-Tab works globally regardless of current mode.

- [ ] **Step 8: Run all tests**

Run: `cargo test --all-features`
Expected: All tests PASS including the new focus tests.

- [ ] **Step 9: Commit**

```bash
git add src/tui/app/focus.rs src/tui/app/state.rs src/tui/app/mod.rs src/tui/event.rs tests/tui_focus_test.rs
git commit -m "feat(tui): add FocusRegion model with Tab/Shift-Tab cycling"
```

---

## Task 9: Apply visual widgets to Search tab

**Files:**
- Modify: `src/tui/tabs/search.rs`

- [ ] **Step 1: Rewrite search tab rendering**

Replace the contents of `src/tui/tabs/search.rs` to use the new widgets:

```rust
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::app::state::{FocusRegion, SearchState};
use crate::tui::widgets::focusable_block::{FocusableBlock, FocusStyle};
use crate::tui::widgets::result_list::{render_result_list, ResultItem};
use crate::tui::widgets::section_separator::SectionSeparator;

/// Render the search tab into `area`.
pub fn render_search(frame: &mut Frame, area: Rect, state: &SearchState, focus: FocusRegion) {
    let input_focused = focus == FocusRegion::Primary;
    let results_focused = focus == FocusRegion::Secondary;

    // Layout: input block (3 lines with border), separator (1), results block (fills)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // input with border
            Constraint::Length(1), // separator
            Constraint::Min(3),   // results with border
        ])
        .split(area);

    // -- Input block (FocusableBlock) ----------------------------------------
    let input_block = FocusableBlock::new(FocusStyle::Input)
        .focused(input_focused)
        .to_block();
    let cursor_char = if input_focused { "\u{2502}" } else { "" };
    let input_text = format!("/ {}{}", state.query, cursor_char);
    let input_style = if input_focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let input_paragraph = Paragraph::new(Span::styled(input_text, input_style)).block(input_block);
    frame.render_widget(input_paragraph, chunks[0]);

    // -- Section separator with result count ---------------------------------
    let status_label = if state.loading {
        "Searching..."
    } else if state.query.is_empty() {
        ""
    } else {
        // Build the label dynamically
        &format!(
            "{} result{}",
            state.results.len(),
            if state.results.len() == 1 { "" } else { "s" }
        )
    };
    // We need to handle the lifetime — build separator inline
    let sep = SectionSeparator::new(area.width);
    let sep_line = if status_label.is_empty() {
        SectionSeparator::new(area.width).to_line()
    } else {
        // Reconstruct with label to avoid lifetime issues
        sep.label(status_label).to_line()
    };
    frame.render_widget(Paragraph::new(sep_line), chunks[1]);

    // -- Results block (FocusableBlock) --------------------------------------
    let results_block = FocusableBlock::new(FocusStyle::Content)
        .focused(results_focused)
        .to_block();
    let results_inner = results_block.inner(chunks[2]);
    frame.render_widget(results_block, chunks[2]);

    if state.results.is_empty() {
        if !state.query.is_empty() && !state.loading {
            let no_results = Paragraph::new(Span::styled(
                "No results",
                Style::default().fg(Color::DarkGray),
            ));
            frame.render_widget(no_results, results_inner);
        }
        return;
    }

    let items: Vec<ResultItem> = state
        .results
        .iter()
        .map(|hit| ResultItem {
            score: hit.score,
            path: &hit.file_path,
            note_type: &hit.note_type,
            snippet: &hit.content,
            depth: None,
        })
        .collect();

    render_result_list(frame, results_inner, &items, state.selected);
}
```

Note: The render function signature gains a `focus: FocusRegion` parameter. Update the call site in `app/mod.rs` render method to pass `self.focus`.

- [ ] **Step 2: Update call site in app/mod.rs**

In the render method, change:
```rust
Screen::Search(search_state) => {
    search::render_search(frame, chunks[1], search_state);
}
```
to:
```rust
Screen::Search(search_state) => {
    search::render_search(frame, chunks[1], search_state, self.focus);
}
```

- [ ] **Step 3: Fix lifetime issue in separator**

The `status_label` format string has a temporary lifetime. Resolve by computing the string before the separator call and storing it in a local variable. Adjust the code as needed so it compiles cleanly.

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: All tests PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tui/tabs/search.rs src/tui/app/mod.rs
git commit -m "feat(tui): apply FocusableBlock and SectionSeparator to Search tab"
```

---

## Task 10: Apply visual widgets to Context tab

**Files:**
- Modify: `src/tui/tabs/context.rs`

- [ ] **Step 1: Rewrite context tab rendering**

Apply the same pattern as Search: `FocusableBlock` around input and neighbor list, `SectionSeparator` with neighbor count. Add `focus: FocusRegion` parameter. The input region shows either the editable input field or the center indicator, both within the same `FocusableBlock`.

Follow the same structure as the Search tab rewrite in Task 9 — bordered input block (3 lines), separator (1 line), bordered neighbor list (fills rest).

- [ ] **Step 2: Update call site in app/mod.rs**

Pass `self.focus` to `render_context()`.

- [ ] **Step 3: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/tui/tabs/context.rs src/tui/app/mod.rs
git commit -m "feat(tui): apply FocusableBlock and SectionSeparator to Context tab"
```

---

## Task 11: Apply visual widgets to Maintain tab

**Files:**
- Modify: `src/tui/tabs/maintain.rs`

- [ ] **Step 1: Rewrite maintain tab rendering**

Wrap the index + normalize sections in a single `FocusableBlock` (the "Actions" region). Add `SectionSeparator` with status label. The progress area below has no border. Add `focus: FocusRegion` parameter — `FocusRegion::Primary` highlights the actions block.

- [ ] **Step 2: Update call site in app/mod.rs**

Pass `self.focus` to `render_maintain()`.

- [ ] **Step 3: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/tui/tabs/maintain.rs src/tui/app/mod.rs
git commit -m "feat(tui): apply FocusableBlock and SectionSeparator to Maintain tab"
```

---

## Task 12: Apply visual widgets to Projects tab (formerly Board)

**Files:**
- Create: `src/tui/tabs/projects.rs` (rename from board.rs)
- Delete: `src/tui/tabs/board.rs`
- Modify: `src/tui/tabs/mod.rs`
- Modify: `src/tui/app/mod.rs`

- [ ] **Step 1: Rename board.rs to projects.rs**

```bash
mv src/tui/tabs/board.rs src/tui/tabs/projects.rs
```

- [ ] **Step 2: Update module references**

In `src/tui/tabs/mod.rs`, change `pub mod board;` to `pub mod projects;`.

In `src/tui/app/mod.rs`, change `use super::tabs::board;` to `use super::tabs::projects;` and update all call sites (`board::render_board` → `projects::render_projects_tab`).

- [ ] **Step 3: Rewrite projects.rs**

Apply `BreadcrumbBar` to milestones and swimlanes views. Apply `FocusableBlock` around the project list, milestone list, and swimlane columns. Add `focus: FocusRegion` parameter.

Key changes:
- `render_projects()` (Level 1): `BreadcrumbBar::new(&["All"])`, `FocusableBlock` around project list
- `render_milestones()`: `BreadcrumbBar::new(&["All", project])`, `FocusableBlock` around milestone list
- `render_swimlanes()`: `BreadcrumbBar::new(&["All", project, milestone])`, each column in its own `FocusableBlock` — focused column gets `FocusStyle::Content` with `focused(true)`, others get `focused(false)`
- For swimlanes, focus maps: `FocusRegion::Tertiary(0)` = first column, etc.

- [ ] **Step 4: Update call site in app/mod.rs render method**

```rust
Screen::Projects(board_state) => {
    projects::render_projects_tab(frame, chunks[1], board_state, self.focus);
}
```

- [ ] **Step 5: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/tabs/projects.rs src/tui/tabs/mod.rs src/tui/app/mod.rs
git rm src/tui/tabs/board.rs
git commit -m "feat(tui): rename Board to Projects, apply BreadcrumbBar and FocusableBlock"
```

---

## Task 13: Viewer with breadcrumbs and markdown rendering

**Files:**
- Modify: `src/tui/views/viewer.rs`
- Modify: `src/tui/app/mod.rs` (viewer call site)

- [ ] **Step 1: Rewrite viewer rendering**

Replace `src/tui/views/viewer.rs` to use:
- `BreadcrumbBar` at the top (source-aware: build segments from `source_label`)
- `FocusableBlock` with `FocusStyle::DisplayOnly` around frontmatter
- `SectionSeparator` between frontmatter and body
- `FocusableBlock` with `FocusStyle::Content` around document body, with offset background color (use `Style::default().bg(Color::Rgb(22, 22, 42))` on the body block)
- `render_markdown()` to convert the document body text to styled lines

Layout:
```
BreadcrumbBar (1 line)
FocusableBlock [frontmatter] (variable)
SectionSeparator (1 line)
FocusableBlock [markdown body] (fills)
```

The markdown body block needs scroll support — use `Paragraph::new(rendered_lines).scroll((state.scroll_offset as u16, 0))`.

Add `focus: FocusRegion` parameter.

- [ ] **Step 2: Update ViewerState source_label**

The current `source_label` is a flat string like `"Board > project > milestone"`. For breadcrumbs, we need structured segments. Add a `breadcrumb_segments: Vec<String>` field to `ViewerState` in `state.rs`. Populate it when pushing a Viewer screen in `handle_enter()`:
- From Projects: `["All", project, milestone, title]`
- From Search: `["Search", title]`
- From Context: `["Context", center_topic, title]`

- [ ] **Step 3: Update call site in app/mod.rs**

Pass `self.focus` to `render_viewer()`.

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/views/viewer.rs src/tui/app/
git commit -m "feat(tui): viewer with BreadcrumbBar, markdown rendering, and offset background"
```

---

## Task 14: Tab bar focus highlight

**Files:**
- Modify: `src/tui/app/mod.rs` (render method, tab bar section)
- Modify: `src/tui/widgets/keyhints.rs`

- [ ] **Step 1: Add tab bar focus highlight**

In the render method's tab bar section (around line 775 of the original), when `self.focus == FocusRegion::TabBar`, add a subtle background to the tab bar line:

```rust
let tab_bar_style = if self.focus == FocusRegion::TabBar {
    Style::default().bg(Color::Rgb(30, 30, 50))
} else {
    Style::default()
};
let tab_bar = Paragraph::new(Line::from(spans)).style(tab_bar_style);
```

When TabBar is focused, left/right (h/l) should switch tabs. Add this to the dispatch method: when `self.focus == FocusRegion::TabBar`, `MoveLeft`/`MoveRight` cycle through tabs, `Enter` activates the highlighted tab and moves focus to first content region.

- [ ] **Step 2: Update keyhints for Tab/Shift-Tab**

In `src/tui/widgets/keyhints.rs`, add `Tab/S-Tab focus` to each hint string. Update "Board" references to "Projects".

- [ ] **Step 3: Update the swimlane widget**

In `src/tui/widgets/swimlane.rs`, replace the direct `Block` usage with `FocusableBlock`:

```rust
use crate::tui::widgets::focusable_block::{FocusableBlock, FocusStyle};

// Replace the block creation:
let block = FocusableBlock::new(FocusStyle::Content)
    .focused(self.focused)
    .title(&format!("{} ({})", self.title, self.count))
    .to_block();
```

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app/mod.rs src/tui/widgets/keyhints.rs src/tui/widgets/swimlane.rs
git commit -m "feat(tui): tab bar focus highlight, updated keyhints, swimlane uses FocusableBlock"
```

---

## Task 15: Update help overlay and final integration

**Files:**
- Modify: `src/tui/app/mod.rs` (help overlay text)
- Modify: `src/tui/event.rs` (ensure Tab handling doesn't conflict with search input Tab)

- [ ] **Step 1: Update help overlay**

In `render_help_overlay()`, update:
- "Board" → "Projects" in all references
- Add "Tab / S-Tab" to the Navigation section
- Update command shortcuts: `:p` / `:projects`

- [ ] **Step 2: Fix Tab key conflict in search/context input**

The global Tab handler from Task 8 must not fire when search or context input is active — those modes should keep Tab for their own use (focus results). Adjust the `map_key` function in `event.rs` so that:
- When `in_search_input`: Tab maps to `SearchFocusResults` (existing behavior)
- When `in_context_input`: Tab is ignored (no action)
- Otherwise: Tab maps to `FocusNext`

The existing `map_search_input` already handles Tab → `SearchFocusResults`. Ensure the global Tab handler has lower priority by placing it after the mode-specific checks.

- [ ] **Step 3: Escape behavior for focus**

When at top-level (no drill-down), Escape should move focus to TabBar instead of doing nothing. Adjust the Escape handler in dispatch: if `self.focus != FocusRegion::TabBar` and we're at root level (stack depth 1, not drilled into milestones/swimlanes), set `self.focus = FocusRegion::TabBar`.

- [ ] **Step 4: Full test suite**

Run: `cargo test --all-features`
Expected: All tests PASS.

- [ ] **Step 5: Clippy and fmt**

Run: `cargo clippy --all-features && cargo fmt --check`
Expected: No warnings, formatting clean.

- [ ] **Step 6: Commit**

```bash
git add src/tui/
git commit -m "feat(tui): help overlay update, Tab key conflict resolution, Escape focus behavior"
```

---

## Task 16: Manual verification and polish

**Files:** None (testing only)

- [ ] **Step 1: Run the TUI**

Run: `temper tui`

Verify:
- Projects tab shows project list as default view
- Enter drills into milestones, then swimlanes
- Escape walks back through the hierarchy
- Breadcrumb pills appear with increasing saturation
- Tab/Shift-Tab cycles between sections
- Tab bar highlights when focused
- Search and Context tabs have bordered input + results
- Maintain tab has bordered actions section
- Viewer shows breadcrumbs, bordered frontmatter, rendered markdown on offset background

- [ ] **Step 2: Fix any visual issues found**

Address rendering bugs, spacing issues, or color adjustments discovered during manual testing.

- [ ] **Step 3: Final commit if changes made**

```bash
git add -A
git commit -m "fix(tui): visual polish from manual testing"
```

- [ ] **Step 4: Run full test suite one last time**

Run: `cargo test --all-features && cargo clippy --all-features`
Expected: All PASS.
