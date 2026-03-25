# TUI Visual Fidelity & Navigation Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three visual fidelity gaps (breadcrumb pill backgrounds, border visibility, selected-item highlights) and redesign the Projects tab navigation model to use master-detail split instead of push-based full-screen replacement.

**Architecture:** Priority 1 changes are isolated widget/renderer fixes — each modifies a single file with no structural impact. Priority 2 is a navigation redesign affecting `BoardLevel` state, `projects.rs` rendering, `focus.rs` region lists, and `actions.rs` enter/escape handling. The existing push-based stack model is preserved for swimlanes and viewer — only the Projects→Milestones transition changes to a side-by-side split.

**Tech Stack:** Rust, ratatui, crossterm

**Spec:** `docs/superpowers/specs/2026-03-25-tui-polish-and-navigation-design.md`
**Prior plan:** `docs/superpowers/plans/2026-03-25-tui-polish-and-navigation.md`
**Branch:** `jcoletaylor/tui-polish-and-navigation-improvements`

---

## Priority 1: Visual Fidelity Fixes

### Task 1: BreadcrumbBar background-colored pills

**Files:**
- Modify: `src/tui/widgets/breadcrumb_bar.rs:55-72` (the `segment_style` function)
- Test: `src/tui/widgets/breadcrumb_bar.rs` (inline tests)

The spec defines depth-based background colors for breadcrumb segments:
- Depth 0 (root): `Rgb(30, 32, 64)` bg, DarkGray fg
- Depth 1 (intermediate): `Rgb(37, 37, 80)` bg, Gray fg
- Depth 2+ (active): `Rgb(42, 42, 106)` bg, Yellow fg, Bold

Currently `segment_style()` only sets `fg()` — no `bg()` is applied.

- [ ] **Step 1: Update test expectations to include background colors**

Add a new test and update the existing `last_segment_is_yellow_bold` test to also assert background color:

```rust
#[test]
fn segments_have_depth_based_backgrounds() {
    let bar = BreadcrumbBar::new(&["All", "temper", "viz"]);
    let line = bar.to_line();

    // Depth 0: "All" at span index 0
    assert_eq!(
        line.spans[0].style.bg,
        Some(Color::Rgb(30, 32, 64)),
        "depth 0 should have dark indigo background"
    );

    // Depth 1: "temper" at span index 2
    assert_eq!(
        line.spans[2].style.bg,
        Some(Color::Rgb(37, 37, 80)),
        "depth 1 should have medium indigo background"
    );

    // Depth 2: "viz" at span index 4
    assert_eq!(
        line.spans[4].style.bg,
        Some(Color::Rgb(42, 42, 106)),
        "depth 2+ should have bright indigo background"
    );
}

#[test]
fn single_segment_has_root_background() {
    let bar = BreadcrumbBar::new(&["All"]);
    let line = bar.to_line();
    assert_eq!(
        line.spans[0].style.bg,
        Some(Color::Rgb(30, 32, 64)),
        "single segment should have root background"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib breadcrumb_bar -- --nocapture`
Expected: FAIL — `bg` is `None`, not `Some(Rgb(...))`

- [ ] **Step 3: Add background colors to `segment_style`**

Replace the `segment_style` function body:

```rust
fn segment_style(idx: usize, last_idx: usize) -> Style {
    // Single segment — always root style
    if last_idx == 0 {
        return Style::default()
            .fg(Color::DarkGray)
            .bg(Color::Rgb(30, 32, 64));
    }

    // Last segment of a multi-segment bar — always active (Yellow+Bold)
    if idx == last_idx {
        return Style::default()
            .fg(Color::Yellow)
            .bg(Color::Rgb(42, 42, 106))
            .bold();
    }

    // Intermediate segments: depth 0 → DarkGray, depth 1+ → Gray
    if idx == 0 {
        Style::default()
            .fg(Color::DarkGray)
            .bg(Color::Rgb(30, 32, 64))
    } else {
        Style::default()
            .fg(Color::Gray)
            .bg(Color::Rgb(37, 37, 80))
    }
}
```

- [ ] **Step 4: Update the existing `single_segment_renders_root_style` test**

The existing test asserts only `fg`. It still passes since we only added `bg`. No change needed — but verify.

- [ ] **Step 5: Run all breadcrumb tests**

Run: `cargo test --lib breadcrumb_bar`
Expected: all PASS

- [ ] **Step 6: Add padding spaces to pill segments for visual weight**

Wrap each segment label with a leading and trailing space so the background color creates a visible pill effect. In `BreadcrumbBar::to_line()`, change the span creation:

```rust
// Before:
spans.push(Span::styled(segment.clone(), style));

// After:
spans.push(Span::styled(format!(" {} ", segment), style));
```

Update tests that check `span.content` for exact matches (e.g., `"All"` becomes `" All "`).

- [ ] **Step 7: Run all tests**

Run: `cargo test --lib breadcrumb_bar`
Expected: all PASS (after updating content assertions)

- [ ] **Step 8: Commit**

```bash
git add src/tui/widgets/breadcrumb_bar.rs
git commit -m "fix(tui): add background colors and padding to breadcrumb pills"
```

---

### Task 2: FocusableBlock border visibility

**Files:**
- Modify: `src/tui/widgets/focusable_block.rs:61-65` (the border color match)
- Test: `src/tui/widgets/focusable_block.rs` (inline tests)

The unfocused border color is `DarkGray` which is invisible on dark terminal backgrounds. Change unfocused-interactive borders to `Color::Rgb(60, 60, 80)` — visible but still clearly dimmer than focused borders (Yellow/Cyan).

- [ ] **Step 1: Add test for unfocused border visibility**

```rust
#[test]
fn unfocused_input_has_visible_border() {
    let fb = FocusableBlock::new(FocusStyle::Input).focused(false);
    let block = fb.to_block();
    let buf = render_block(block);
    let fg = border_fg_at(&buf, 0, 0);
    // Should NOT be DarkGray — should be a visible-but-dim color
    assert_ne!(
        fg,
        Color::DarkGray,
        "unfocused interactive border should be brighter than DarkGray"
    );
    assert_eq!(
        fg,
        Color::Rgb(60, 60, 80),
        "unfocused interactive border should be subtle indigo"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib focusable_block::tests::unfocused_input_has_visible_border`
Expected: FAIL — currently returns `DarkGray`

- [ ] **Step 3: Update border color logic**

Replace the border_color match in `to_block()`:

```rust
let border_color = match (&self.style, self.focused) {
    (FocusStyle::Input, true) => Color::Yellow,
    (FocusStyle::Content, true) => Color::Cyan,
    (FocusStyle::DisplayOnly, _) => Color::DarkGray,
    // Unfocused interactive — visible but dim
    (_, false) => Color::Rgb(60, 60, 80),
};
```

- [ ] **Step 4: Update existing test `unfocused_interactive_has_dark_gray_border`**

This test now needs to expect `Rgb(60, 60, 80)` instead of `DarkGray`. Rename it to `unfocused_interactive_has_dim_border` and update assertion:

```rust
#[test]
fn unfocused_interactive_has_dim_border() {
    let fb = FocusableBlock::new(FocusStyle::Content).focused(false);
    let block = fb.to_block();
    let buf = render_block(block);
    assert_eq!(
        border_fg_at(&buf, 0, 0),
        Color::Rgb(60, 60, 80),
        "unfocused Content block should have dim indigo border"
    );
}
```

- [ ] **Step 5: Run all focusable_block tests**

Run: `cargo test --lib focusable_block`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add src/tui/widgets/focusable_block.rs
git commit -m "fix(tui): make unfocused borders visible with subtle indigo color"
```

---

### Task 3: Selected item background highlight in result lists

**Files:**
- Modify: `src/tui/widgets/result_list.rs:45-54` (selected item styling)
- Test: `src/tui/widgets/result_list.rs` (add inline tests)

The spec calls for a background-color bar on selected items: `Rgb(42, 42, 74)`. Currently selected items only have yellow text, no background.

- [ ] **Step 1: Add background to selected item styles in `render_result_list`**

In `result_list.rs`, update the style logic:

```rust
let header_style = if is_selected {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Rgb(42, 42, 74))
        .bold()
} else {
    Style::default().fg(Color::White)
};
let meta_style = if is_selected {
    Style::default()
        .fg(Color::Cyan)
        .bg(Color::Rgb(42, 42, 74))
} else {
    Style::default().fg(Color::DarkGray)
};
```

Also apply background to the depth prefix and snippet spans when selected:

```rust
let depth_style = if is_selected {
    Style::default().bg(Color::Rgb(42, 42, 74))
} else {
    Style::default()
};
let snippet_style = if is_selected {
    Style::default()
        .fg(Color::Gray)
        .bg(Color::Rgb(42, 42, 74))
} else {
    Style::default().fg(Color::Gray)
};
let dot_style = if is_selected {
    Style::default()
        .fg(Color::DarkGray)
        .bg(Color::Rgb(42, 42, 74))
} else {
    Style::default().fg(Color::DarkGray)
};
```

Then use these styles in the span construction:

```rust
let header_line = Line::from(vec![
    Span::styled(depth_prefix.clone(), depth_style),
    Span::styled(score_text, meta_style),
    Span::styled(" ", depth_style),
    Span::styled(item.path, header_style),
]);
let detail_line = Line::from(vec![
    Span::styled(format!("{}  ", depth_prefix), depth_style),
    Span::styled(item.note_type, meta_style),
    Span::styled(" \u{00b7} ", dot_style),
    Span::styled(snippet_truncated, snippet_style),
]);
```

- [ ] **Step 2: Run full test suite to verify no regressions**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 3: Commit**

```bash
git add src/tui/widgets/result_list.rs
git commit -m "fix(tui): add background highlight to selected result items"
```

---

### Task 4: Selected item highlight in project and milestone lists

**Files:**
- Modify: `src/tui/tabs/projects.rs:57-68` (project list items) and `src/tui/tabs/projects.rs:102-116` (milestone list items)

Apply the same `Rgb(42, 42, 74)` background to selected items in the Projects and Milestones list views.

- [ ] **Step 1: Update project list selected style**

In `render_projects()`, update the style block:

```rust
let style = if i == selected {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Rgb(42, 42, 74))
        .bold()
} else {
    Style::default()
};
```

- [ ] **Step 2: Update milestone list selected style**

In `render_milestones()`, update the style block:

```rust
let style = if i == selected {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Rgb(42, 42, 74))
        .bold()
} else {
    Style::default()
};
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add src/tui/tabs/projects.rs
git commit -m "fix(tui): add background highlight to selected projects and milestones"
```

---

## Priority 2: Projects Tab Navigation Redesign

### Task 5: Change BoardLevel to always-start-at-projects and add master-detail state

**Files:**
- Modify: `src/tui/app/state.rs` (BoardLevel and BoardState)
- Modify: `src/tui/app/mod.rs` (`from_config`, `make_root_screen`)
- Test: `tests/tui_state_test.rs`

The current model auto-drills into CWD project milestones. The spec says Projects tab always starts at project list with an optional right-panel showing milestones for the selected project.

- [ ] **Step 1: Restructure BoardState to hold master-detail state**

In `state.rs`, change `BoardState` and `BoardLevel`:

```rust
#[derive(Debug, Clone)]
pub struct BoardState {
    pub level: BoardLevel,
    /// When at project list level, tracks milestones for the selected project
    /// (shown in right panel). None until a project is selected or data loads.
    pub detail_milestones: Option<DetailPanel>,
}

#[derive(Debug, Clone)]
pub struct DetailPanel {
    pub project: String,
    pub milestones: Vec<MilestoneWithCounts>,
    pub selected: usize,
}
```

Keep `BoardLevel` as-is but make `Projects` the default starting level. The `Milestones` variant is no longer used as a standalone level — it only exists in `detail_milestones`. However, to minimize disruption, we can repurpose the existing variants:

Actually, simpler approach — keep `BoardLevel::Projects` as the left panel, store the detail panel alongside. `BoardLevel::Milestones` is removed as a top-level concept (milestones are always the right side of the split). `BoardLevel::Swimlanes` remains as a full-screen push.

```rust
#[derive(Debug, Clone)]
pub enum BoardLevel {
    /// Two-panel split: left = project list, right = milestones for selected project
    ProjectList {
        selected: usize,
        projects: Vec<String>,
        detail: Option<DetailPanel>,
    },
    /// Full-screen swimlanes (pushed onto stack from milestone selection)
    Swimlanes {
        project: String,
        milestone: String,
        load_project: String,
        load_milestone: String,
        column: usize,
        row: usize,
        columns: [Vec<TicketInfo>; 3],
    },
}
```

- [ ] **Step 2: Update BoardState**

```rust
#[derive(Debug, Clone)]
pub struct BoardState {
    pub level: BoardLevel,
}
```

(No change to `BoardState` itself — the detail is embedded in `BoardLevel::ProjectList`.)

- [ ] **Step 3: Fix all compilation errors from variant changes**

Two enum variants change: `BoardLevel::Projects` → `BoardLevel::ProjectList`, and `BoardLevel::Milestones` is removed entirely. Every match arm referencing either variant must be updated.

Files that match on `BoardLevel::Projects { .. }`:
- `src/tui/tabs/projects.rs:12-14` — match arm in `render_projects_tab`
- `src/tui/app/actions.rs:10` — `handle_enter` match arm
- `src/tui/app/actions.rs:152` — `move_selection` match arm
- `src/tui/app/focus.rs:14` — combined `Projects | Milestones` pattern in `focus_regions`

Files that match on `BoardLevel::Milestones { .. }`:
- `src/tui/tabs/projects.rs:15-19` — `render_milestones` dispatch
- `src/tui/app/actions.rs:27-63` — `handle_enter` milestones arm
- `src/tui/app/actions.rs:161-173` — `move_selection` milestones arm
- `src/tui/app/focus.rs:14` — combined pattern with `Projects`

Also add the `DetailPanel` struct declaration to `state.rs` (it's re-exported via `pub use state::*` in `mod.rs`):

```rust
#[derive(Debug, Clone)]
pub struct DetailPanel {
    pub project: String,
    pub milestones: Vec<MilestoneWithCounts>,
    pub selected: usize,
}
```

This step stubs all match arms to compile — full rendering comes in Task 6, full action handling in Task 8.

- [ ] **Step 4: Update `from_config` to always start at ProjectList**

In `mod.rs`, change `from_config`:

```rust
let inferred_project = project.as_ref().map(|p| p.name.clone());

// Always start at project list; if we have an inferred project,
// pre-load its milestones into the detail panel
let detail = if let Some(proj) = &project {
    let milestones = load_milestones_with_counts(config, &proj.name);
    Some(DetailPanel {
        project: proj.name.clone(),
        milestones,
        selected: 0,
    })
} else {
    None
};

// Pre-select the inferred project in the list
let selected = project
    .as_ref()
    .and_then(|p| project_names.iter().position(|n| n == &p.name))
    .unwrap_or(0);

let root_screen = Screen::Projects(BoardState {
    level: BoardLevel::ProjectList {
        selected,
        projects: project_names.clone(),
        detail,
    },
});
```

- [ ] **Step 5: Update `make_root_screen` for Projects tab**

```rust
Tab::Projects => {
    let detail = if let (Some(proj), Some(config)) = (&self.inferred_project, &self.config) {
        let milestones = load_milestones_with_counts(config, proj);
        Some(DetailPanel {
            project: proj.clone(),
            milestones,
            selected: 0,
        })
    } else {
        None
    };

    let selected = self.inferred_project.as_ref()
        .and_then(|p| self.project_names.iter().position(|n| n == p))
        .unwrap_or(0);

    Screen::Projects(BoardState {
        level: BoardLevel::ProjectList {
            selected,
            projects: self.project_names.clone(),
            detail,
        },
    })
}
```

- [ ] **Step 6: Update `new_for_test` to use new BoardLevel**

```rust
pub fn new_for_test() -> Self {
    let milestone = MilestoneWithCounts {
        info: MilestoneInfo {
            title: "v0.1".into(),
            slug: "v0-1".into(),
            project: "demo".into(),
            seq: 1,
            status: "active".into(),
        },
        backlog: 0,
        in_progress: 0,
        done: 0,
    };
    let board = BoardState {
        level: BoardLevel::ProjectList {
            selected: 0,
            projects: vec!["demo".into()],
            detail: Some(DetailPanel {
                project: "demo".into(),
                milestones: vec![milestone],
                selected: 0,
            }),
        },
    };
    Self {
        stack: vec![Screen::Projects(board)],
        // ... rest unchanged
    }
}
```

- [ ] **Step 7: Update existing tests**

In `tests/tui_state_test.rs`, `test_esc_pops_stack` currently expects Enter to push swimlanes from milestones. With the new model, `new_for_test` starts at `BoardLevel::ProjectList` with focus on `Primary`. The first Enter moves focus to `Secondary` (detail panel) without pushing to the stack. A second Enter on `Secondary` pushes Swimlanes. Update:

```rust
#[test]
fn test_esc_pops_stack() {
    let mut app = App::new_for_test();
    // new_for_test starts at ProjectList with detail panel, focus on Primary
    assert_eq!(app.stack_depth(), 1);

    // First Enter focuses the detail panel (Secondary), stack unchanged
    app.dispatch(AppAction::Enter);
    assert_eq!(app.stack_depth(), 1, "Enter on Primary should focus Secondary, not push");

    // Second Enter on milestone detail pushes Swimlanes
    app.dispatch(AppAction::Enter);
    assert_eq!(app.stack_depth(), 2, "Enter on Secondary should push Swimlanes");

    // Esc pops back to ProjectList
    app.dispatch(AppAction::Escape);
    assert_eq!(app.stack_depth(), 1);
}
```

- [ ] **Step 8: Run tests**

Run: `cargo test`
Expected: all PASS (compilation + tests)

- [ ] **Step 9: Commit**

```bash
git add src/tui/app/state.rs src/tui/app/mod.rs src/tui/app/actions.rs src/tui/app/focus.rs tests/tui_state_test.rs
git commit -m "refactor(tui): restructure BoardLevel for master-detail project navigation"
```

---

### Task 6: Render Projects tab as master-detail split

**Files:**
- Modify: `src/tui/tabs/projects.rs` (complete rewrite of project/milestone rendering)

Replace the separate `render_projects` and `render_milestones` functions with a single split-panel renderer for `BoardLevel::ProjectList`.

- [ ] **Step 1: Implement split layout renderer**

Replace the `BoardLevel::Projects` and `BoardLevel::Milestones` match arms in `render_projects_tab` with a single `BoardLevel::ProjectList` arm:

```rust
pub fn render_projects_tab(frame: &mut Frame, area: Rect, state: &BoardState, focus: FocusRegion) {
    match &state.level {
        BoardLevel::ProjectList {
            projects,
            selected,
            detail,
        } => render_project_list(frame, area, projects, *selected, detail.as_ref(), focus),
        BoardLevel::Swimlanes {
            project,
            milestone,
            columns,
            column,
            row,
            ..
        } => render_swimlanes(frame, area, project, milestone, columns, *column, *row, focus),
    }
}
```

- [ ] **Step 2: Implement `render_project_list` with horizontal split**

```rust
fn render_project_list(
    frame: &mut Frame,
    area: Rect,
    projects: &[String],
    selected: usize,
    detail: Option<&DetailPanel>,
    focus: FocusRegion,
) {
    // Layout: breadcrumb (1 line) + split content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb — shows project name if detail is open
    let breadcrumb = if let Some(d) = detail {
        BreadcrumbBar::new(&["All", &d.project])
    } else {
        BreadcrumbBar::new(&["All"])
    };
    frame.render_widget(Paragraph::new(breadcrumb.to_line()), chunks[0]);

    // Horizontal split: left panel (projects) + right panel (milestones)
    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);

    // Left panel: project list
    render_left_panel(frame, panels[0], projects, selected, focus);

    // Right panel: milestone list (or empty)
    if let Some(d) = detail {
        render_right_panel(frame, panels[1], d, focus);
    } else {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(false);
        let block = fb.to_block();
        let msg = Paragraph::new(Span::styled(
            "Select a project",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        frame.render_widget(msg, panels[1]);
    }
}

fn render_left_panel(
    frame: &mut Frame,
    area: Rect,
    projects: &[String],
    selected: usize,
    focus: FocusRegion,
) {
    if projects.is_empty() {
        let msg =
            Paragraph::new("No projects configured").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == selected { "\u{25b8} " } else { "  " };
            let style = if i == selected {
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(42, 42, 74))
                    .bold()
            } else {
                Style::default()
            };
            ListItem::new(format!("{}{}", marker, name)).style(style)
        })
        .collect();

    let fb = FocusableBlock::new(FocusStyle::Content)
        .focused(focus == FocusRegion::Primary)
        .title(" Projects ");
    let block = fb.to_block();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_right_panel(
    frame: &mut Frame,
    area: Rect,
    detail: &DetailPanel,
    focus: FocusRegion,
) {
    if detail.milestones.is_empty() {
        let fb = FocusableBlock::new(FocusStyle::Content).focused(focus == FocusRegion::Secondary);
        let block = fb.to_block();
        let msg = Paragraph::new(Span::styled(
            "No milestones",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = detail
        .milestones
        .iter()
        .enumerate()
        .map(|(i, ms)| {
            let marker = if i == detail.selected { "\u{25b8} " } else { "  " };
            let style = if i == detail.selected {
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(42, 42, 74))
                    .bold()
            } else {
                Style::default()
            };
            let counts = format_milestone_counts(ms);
            let label = format!("{}{:<26}{}", marker, ms.info.title, counts);
            ListItem::new(label).style(style)
        })
        .collect();

    let fb = FocusableBlock::new(FocusStyle::Content)
        .focused(focus == FocusRegion::Secondary)
        .title(&format!(" {} Milestones ", detail.project));
    let block = fb.to_block();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
```

- [ ] **Step 3: Add necessary imports**

Add `use crate::tui::app::DetailPanel;` and the `List`, `ListItem` imports at top of `projects.rs`.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test && cargo clippy --all-features`
Expected: all PASS, no warnings

- [ ] **Step 5: Commit**

```bash
git add src/tui/tabs/projects.rs
git commit -m "feat(tui): render Projects tab as master-detail split layout"
```

---

### Task 7: Update focus regions for master-detail Projects tab

**Files:**
- Modify: `src/tui/app/focus.rs` (focus_regions for ProjectList, sync_focus_to_state)

The master-detail split introduces two focus regions for `ProjectList`: `Primary` (left panel / project list) and `Secondary` (right panel / milestones).

- [ ] **Step 1: Write test for focus cycling through both panels**

In `tests/tui_focus_test.rs`:

```rust
#[test]
fn project_list_has_two_content_regions() {
    let app = App::new_for_test();
    // new_for_test has ProjectList with detail
    let regions = app.focus_regions();
    assert_eq!(
        regions,
        vec![FocusRegion::TabBar, FocusRegion::Primary, FocusRegion::Secondary],
        "ProjectList with detail should have TabBar + Primary + Secondary"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test project_list_has_two_content_regions`
Expected: FAIL — currently only has `[TabBar, Primary]`

- [ ] **Step 3: Update `focus_regions` for ProjectList**

In `focus.rs`, update the `BoardLevel` match:

```rust
Screen::Projects(board) => match &board.level {
    BoardLevel::ProjectList { detail, .. } => {
        if detail.is_some() {
            vec![FocusRegion::TabBar, FocusRegion::Primary, FocusRegion::Secondary]
        } else {
            vec![FocusRegion::TabBar, FocusRegion::Primary]
        }
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
```

- [ ] **Step 4: Update `sync_focus_to_state` — no ProjectList-specific sync needed**

The `sync_focus_to_state` for `Projects` currently syncs swimlane column. That still works for Swimlanes. No additional sync needed for ProjectList since `selected` is managed differently (MoveUp/Down on Primary moves project selection, on Secondary moves milestone selection).

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add src/tui/app/focus.rs tests/tui_focus_test.rs
git commit -m "feat(tui): add Secondary focus region for master-detail milestone panel"
```

---

### Task 8: Update action handlers for master-detail navigation

**Files:**
- Modify: `src/tui/app/actions.rs` (handle_enter, move_selection)
- Modify: `src/tui/app/mod.rs` (Escape handling)

Key behavior changes:
- **j/k on Primary**: move project selection, load milestones for newly selected project into detail panel
- **j/k on Secondary**: move milestone selection within detail panel
- **Enter on Primary**: same as j/k — selects project and loads milestones (or could just focus Secondary)
- **Enter on Secondary**: push Swimlanes full-screen for selected milestone
- **Escape from Swimlanes**: pop back to ProjectList split view
- **h/l at ProjectList level**: move focus between Primary and Secondary

- [ ] **Step 1: Update `move_selection` for ProjectList**

In `actions.rs`, replace the `BoardLevel::Projects` and `BoardLevel::Milestones` arms.

**Important — borrow checker:** The existing `move_selection` takes `&mut self` and destructures via `self.current_screen_mut()`. Calling another `&mut self` method (like a helper) from within that match arm would create a double mutable borrow. Instead, for the Primary panel case, we update `selected` inside the match, then set a flag to reload the detail panel *after* the match drops the borrow.

```rust
// At the top of move_selection, add a flag:
let mut reload_project_detail = false;

// Then in the match:
BoardLevel::ProjectList {
    selected,
    projects,
    detail,
} => {
    if self.focus == FocusRegion::Secondary {
        // Moving within milestone list
        if let Some(d) = detail {
            match dir {
                Direction::Up => d.selected = d.selected.saturating_sub(1),
                Direction::Down => {
                    if !d.milestones.is_empty() {
                        d.selected = (d.selected + 1).min(d.milestones.len() - 1);
                    }
                }
                _ => {}
            }
        }
    } else {
        // Moving within project list
        match dir {
            Direction::Up => {
                *selected = selected.saturating_sub(1);
                reload_project_detail = true;
            }
            Direction::Down => {
                if !projects.is_empty() {
                    *selected = (*selected + 1).min(projects.len() - 1);
                    reload_project_detail = true;
                }
            }
            _ => {}
        }
    }
}
```

Then after the outer `match self.current_screen_mut()` block closes:

```rust
// After the match — borrow on self is released, safe to call helper
if reload_project_detail {
    self.load_detail_for_selected_project();
}
```

- [ ] **Step 2: Add `load_detail_for_selected_project` helper**

In `actions.rs`. This method takes `&mut self` and is called only after the `move_selection` match borrow is dropped:

```rust
pub(super) fn load_detail_for_selected_project(&mut self) {
    if let Screen::Projects(board) = self.current_screen_mut() {
        if let BoardLevel::ProjectList {
            selected, projects, detail, ..
        } = &mut board.level
        {
            if let Some(proj_name) = projects.get(*selected) {
                let milestones = self
                    .config
                    .as_ref()
                    .map(|c| load_milestones_with_counts(c, proj_name))
                    .unwrap_or_default();
                *detail = Some(DetailPanel {
                    project: proj_name.clone(),
                    milestones,
                    selected: 0,
                });
            }
        }
    }
}
```

**Wait — same issue.** `self.current_screen_mut()` borrows `self`, but then `self.config.as_ref()` also borrows `self`. Fix: extract what we need first.

```rust
pub(super) fn load_detail_for_selected_project(&mut self) {
    // Extract project name first to avoid overlapping borrows
    let proj_name = if let Screen::Projects(board) = self.current_screen() {
        if let BoardLevel::ProjectList { selected, projects, .. } = &board.level {
            projects.get(*selected).cloned()
        } else {
            None
        }
    } else {
        None
    };

    let Some(proj_name) = proj_name else { return };

    let milestones = self
        .config
        .as_ref()
        .map(|c| load_milestones_with_counts(c, &proj_name))
        .unwrap_or_default();

    if let Screen::Projects(board) = self.current_screen_mut() {
        if let BoardLevel::ProjectList { detail, .. } = &mut board.level {
            *detail = Some(DetailPanel {
                project: proj_name,
                milestones,
                selected: 0,
            });
        }
    }
}
```

- [ ] **Step 3: Update `handle_enter` for ProjectList**

Replace the `BoardLevel::Projects` and `BoardLevel::Milestones` arms with:

```rust
BoardLevel::ProjectList { detail, .. } => {
    if self.focus == FocusRegion::Secondary {
        // Enter on milestone — push swimlanes
        if let Some(d) = detail {
            if let Some(ms) = d.milestones.get(d.selected) {
                let ms_slug = ms.info.slug.clone();
                let ms_title = ms.info.title.clone();
                let load_project = ms.info.project.clone();
                let display_project = if load_project == "__all__" {
                    "All".to_string()
                } else {
                    d.project.clone()
                };
                let next = Screen::Projects(BoardState {
                    level: BoardLevel::Swimlanes {
                        project: display_project,
                        milestone: ms_title,
                        load_project: load_project.clone(),
                        load_milestone: ms_slug.clone(),
                        column: 0,
                        row: 0,
                        columns: [vec![], vec![], vec![]],
                    },
                });
                self.stack.push(next);
                self.reset_focus();
                if let Some(tx) = &self.req_tx {
                    let _ = tx.try_send(
                        super::super::query_actor::QueryRequest::LoadTickets {
                            project: load_project,
                            milestone: ms_slug,
                        },
                    );
                }
            }
        }
    } else {
        // Enter on project — focus the detail panel
        self.focus = FocusRegion::Secondary;
        self.sync_focus_to_state();
    }
}
```

- [ ] **Step 4: Update Escape handling in `mod.rs`**

The existing Escape handler at root level (stack depth 1) moves focus to TabBar. When popping from Swimlanes, it should reset focus to Primary (left panel). No special handling needed beyond what exists — `self.reset_focus()` after pop already sets focus to first content region.

- [ ] **Step 5: Update breadcrumb segments in swimlane Enter handler**

In `handle_enter` for `BoardLevel::Swimlanes`, the breadcrumb_segments still reference "All" as root which is correct.

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 7: Commit**

```bash
git add src/tui/app/actions.rs src/tui/app/mod.rs
git commit -m "feat(tui): implement master-detail navigation for Projects tab"
```

---

### Task 9: Final cleanup and full verification

**Files:**
- Verify: all modified files compile cleanly
- Run: full test suite + clippy

- [ ] **Step 1: Run clippy**

Run: `cargo clippy --all-features -- -D warnings`
Expected: clean

- [ ] **Step 2: Run full test suite**

Run: `cargo test`
Expected: all tests pass (existing + new)

- [ ] **Step 3: Fix any remaining dead code warnings**

Remove unused `render_milestones` function from `projects.rs` if it wasn't already removed. Remove the `BoardLevel::Milestones` import from any file that no longer references it.

- [ ] **Step 4: Run `temper tui` manually for visual verification**

Run: `temper tui`
Verify:
- Breadcrumb pills have visible background colors
- FocusableBlock borders are visible against dark terminal
- Selected items have background highlight bar
- Projects tab shows split layout: projects left, milestones right
- j/k on left panel changes project and right panel updates
- Tab/Shift-Tab cycles between left and right panels
- Enter on milestone pushes full-screen swimlanes
- Escape from swimlanes returns to split view

- [ ] **Step 5: Commit any cleanup**

```bash
git add -A
git commit -m "chore(tui): cleanup dead code and final polish"
```
