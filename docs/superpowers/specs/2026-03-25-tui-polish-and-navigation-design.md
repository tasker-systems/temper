# TUI Polish and Navigation Improvements — Design Spec

**Ticket:** `2026-03-24-tui-polish-and-navigation-improvements`
**Branch:** `jcoletaylor/tui-polish-and-navigation-improvements`
**Date:** 2026-03-25

## Goal

Transform the Temper TUI from a functional prototype into a visually consistent, spatially clear interface. Every screen should use the same visual language so users never have to recalibrate when switching tabs or drilling into content.

## Approach

Visual Foundation First: build shared widgets that encode the boxing and breadcrumb rules, apply them across all tabs, then layer on navigation changes.

## 1. Shared Visual Toolkit

Three new widgets in `src/tui/widgets/` that encode all visual structure decisions. Tab renderers never make direct border or separator decisions — they use these widgets.

### 1.1 `focusable_block.rs`

Wraps content in a ratatui `Block` whose border style reflects focus state:

| State | Border | Color | Use |
|-------|--------|-------|-----|
| Focused | `Borders::ALL` | Yellow (inputs) or Cyan (content regions) | Currently active Tab-stop |
| Unfocused-interactive | `Borders::ALL` | DarkGray | Tab-stop but not currently focused |
| Display-only | `Borders::ALL` | DarkGray, dimmer | Bordered for visual consistency but not a Tab-stop (e.g., frontmatter) |

Accepts an optional title rendered in the top border. Focus color is parameterized so callers can distinguish input blocks (yellow) from content blocks (cyan).

### 1.2 `breadcrumb_bar.rs`

Renders a sequence of labeled segments as pills with increasing saturation:

| Depth | Background | Text Color | Weight |
|-------|-----------|------------|--------|
| 0 (root, e.g., "All") | Dark indigo (`#1e2040` / `Rgb(30,32,64)`) | DarkGray | Normal |
| 1 (e.g., project name) | Medium indigo (`#252550` / `Rgb(37,37,80)`) | Gray | Normal |
| 2+ (active segment) | Bright indigo (`#2a2a6a` / `Rgb(42,42,106)`) | Yellow | Bold |

Segments separated by `›` chevrons in DarkGray. Takes 1 line of vertical space. Input: `Vec<&str>` of segment labels — depth derived from position.

### 1.3 `section_separator.rs`

A 1-line horizontal divider using `─` characters in DarkGray. Accepts an optional left-aligned label (e.g., "4 results") rendered inline, breaking the line.

## 2. Tab Bar & Focus Cycling

### 2.1 FocusRegion Model

The app gains a `FocusRegion` enum tracking which section of the current screen has focus. Each screen defines its own region list:

```
Projects tab (per drill-down level — only one level visible at a time):
  Level 1:  TabBar → ProjectList
  Level 2:  TabBar → MilestoneList
  Level 3:  TabBar → SwimCol1 → SwimCol2 → SwimCol3
Search tab:    TabBar → SearchInput → ResultsList
Context tab:   TabBar → TopicInput → NeighborList
Maintain tab:  TabBar → ActionButtons
Viewer:        TabBar → DocumentBody
```

Tab/Shift-Tab cycles forward/backward through the region list for the current screen. The cycle wraps.

### 2.2 Key Binding Rules

| Key | Behavior |
|-----|----------|
| Tab | Move focus to next region |
| Shift-Tab | Move focus to previous region |
| h/l or ←/→ | When TabBar focused: switch between tabs. When in swimlanes: switch columns. |
| j/k or ↑/↓ | Navigate within the focused region (list items, scroll) |
| Enter | When TabBar focused: activate selected tab. Otherwise: drill down / open. |
| Escape | In drilled view: pop back one level. At top level: move focus to TabBar. |
| 1-4 | Direct tab switch (unchanged, works from any focus region) |

### 2.3 Tab Bar Rendering

- Active *tab*: white, bold, underlined (unchanged)
- Inactive tabs: DarkGray (unchanged)
- When TabBar *region* has focus: the entire tab bar line gets a subtle background highlight to indicate focus is "up there"
- When content has focus: tab bar renders normally (no background)

## 3. Projects Tab (formerly Board)

### 3.1 Rename

`Tab::Board` → `Tab::Projects` everywhere: enum variant, command parser (`:projects` / `:p`), key hints, tab bar label. The `1` key still activates it.

### 3.2 Three-Level Hierarchy

**Level 1 — Project List** (new, always the default starting view):
- `FocusableBlock` around a list of all projects from config
- Each entry: `project name · N in-progress · N backlog · N done`
- `BreadcrumbBar`: single "All" pill
- Enter: push to milestones for selected project

**Level 2 — Milestones** (existing logic, new visual treatment):
- `FocusableBlock` around milestone list
- `BreadcrumbBar`: `All › ProjectName`
- "(All Tickets)" synthetic entry remains first
- Enter: push to swimlanes, triggers `QueryRequest::LoadTickets`

**Level 3 — Swimlanes** (existing logic, new visual treatment):
- `BreadcrumbBar`: `All › ProjectName › MilestoneName`
- Three columns, each in its own `FocusableBlock`
- Focused column: cyan border. Other columns: dim border.
- Tab/Shift-Tab cycles between the three columns as sibling regions
- j/k navigates within the focused column

### 3.3 Escape Behavior

Swimlanes → Milestones → Project List. From Project List, Escape moves focus to TabBar.

## 4. Search Tab

### 4.1 Layout

```
┌─ TabBar ──────────────────────────────────────┐
│  Projects · Search · Context · Maintain        │
├───────────────────────────────────────────────-┤
│ ┌─ Search Input (FocusableBlock) ────────────┐ │
│ │ / embeddings│                               │ │
│ └────────────────────────────────────────────┘ │
│ ── 4 results ──────────────────── (separator)  │
│ ┌─ Results (FocusableBlock) ─────────────────┐ │
│ │ ▸ 0.92  concepts/embeddings.md   [concept] │ │
│ │   Vector embeddings for semantic...         │ │
│ │   0.87  concepts/hnsw-index.md   [concept]  │ │
│ │   Hierarchical navigable small world...     │ │
│ └────────────────────────────────────────────┘ │
│ j/k move · Enter view · / search · : cmd       │
└────────────────────────────────────────────────┘
```

### 4.2 Focus

- Input block: yellow border when focused, dim when not
- Results block: cyan border when focused, dim when not
- Tab/Shift-Tab: TabBar → SearchInput → ResultsList (wraps)
- Selected result: left border accent (yellow) + subtle background highlight

### 4.3 No Breadcrumb

Search has no navigation hierarchy — no `BreadcrumbBar` rendered.

## 5. Context Tab

### 5.1 Layout

Identical structure to Search tab:
- `FocusableBlock` around topic input / center indicator
- `SectionSeparator` with neighbor count + stack depth
- `FocusableBlock` around neighbor list

### 5.2 Input Modes

The input region has two visual states:
- **Input active**: editable text field, yellow border, cursor shown
- **Center indicator**: read-only display (`⊙ topic  depth: 2  [+/-] adjust  [c] re-center`), dim border but still a Tab-stop (Tab into it, then `/` to activate input)

### 5.3 Focus

Tab/Shift-Tab: TabBar → TopicInput → NeighborList (wraps). Same selected-item treatment as Search.

## 6. Maintain Tab

### 6.1 Layout

```
┌─ TabBar ──────────────────────────────────────┐
│  Projects · Search · Context · Maintain        │
├────────────────────────────────────────────────┤
│ ┌─ Actions (FocusableBlock) ─────────────────┐ │
│ │ Index                                       │ │
│ │   Last: 42 documents, 318 chunks (1.2s)     │ │
│ │                                             │ │
│ │ Normalize                                   │ │
│ │   IDs backfilled: 0 | Files moved: 2 | ... │ │
│ └────────────────────────────────────────────┘ │
│ ── idle ──────────────────────── (separator)   │
│ Progress output area (no border, static)       │
│                                                │
│ i index · n normalize · : cmd                  │
└────────────────────────────────────────────────┘
```

### 6.2 Focus

Tab/Shift-Tab: TabBar → ActionButtons. Only two focus regions. `i` and `n` trigger actions when ActionButtons is focused.

## 7. Viewer

### 7.1 Layout

```
┌─ TabBar ──────────────────────────────────────┐
│  Projects · Search · Context · Maintain        │
├────────────────────────────────────────────────┤
│  All › temper › visualization-qol › ticket     │  (BreadcrumbBar)
│ ┌─ Frontmatter (display-only border) ────────┐ │
│ │ type: ticket  project: temper  stage: ...   │ │
│ └────────────────────────────────────────────┘ │
│ ─────────────────────────────── (separator)    │
│ ┌─ Document Body (FocusableBlock) ───────────┐ │
│ │                                             │ │
│ │  [rendered markdown on offset background]   │ │
│ │                                             │ │
│ └────────────────────────────────────────────┘ │
│ j/k scroll · e edit · Esc back · : cmd         │
└────────────────────────────────────────────────┘
```

### 7.2 Breadcrumb Path

Source-aware breadcrumbs:
- From Projects drill-down: `All › project › milestone › document-title`
- From Search: `Search › document-title`
- From Context: `Context › center-topic › document-title`

### 7.3 Frontmatter Block

Bordered with dim DarkGray border (display-only — not a Tab-stop). Renders key-value pairs from YAML frontmatter inline.

### 7.4 Rendered Markdown Body

**New dependency:** `pulldown-cmark` for markdown parsing.

**New widget:** `markdown_renderer.rs` in `src/tui/widgets/`. Parses markdown with `pulldown-cmark` into a streaming event sequence, walks events to build `Vec<Line<'_>>` with ratatui styles:

| Markdown Element | Rendering |
|-----------------|-----------|
| `# H1` | Yellow, bold, large (preceded by blank line) |
| `## H2` | Yellow, bold (preceded by blank line) |
| `### H3+` | Yellow (preceded by blank line) |
| `**bold**` | White, bold |
| `*italic*` | Italic modifier |
| `` `inline code` `` | Green on slightly darker background |
| Code blocks | Indented, left accent bar (DarkGray), darker inset background |
| `- list items` | Bullet prefix (`•`), indented |
| `[links](url)` | Cyan, underlined |
| `> blockquotes` | DarkGray left bar, italic text |
| Regular text | Default terminal foreground |

**Offset background:** The document body `FocusableBlock` uses a slightly darker background than the rest of the TUI (`Rgb(22,22,42)` vs the app's default) to create a distinct reading surface.

### 7.5 Focus

Tab/Shift-Tab: TabBar → DocumentBody. Frontmatter is not a focus target. j/k scrolls the body. `e` opens `$EDITOR`. Escape pops back.

## 8. app.rs Decomposition

The current `app.rs` is 1400+ lines. This work naturally splits it:

- **`app.rs`**: Core `App` struct, state enums, `FocusRegion`, screen stack, action dispatch
- **`app/state.rs`** (extract): Per-tab state structs (`SearchState`, `ContextState`, `MaintainState`, `BoardLevel`, etc.)
- **`app/actions.rs`** (extract): `AppAction` enum and `handle_action()` match arms
- **`app/queries.rs`** (extract): `handle_query_result()` and query dispatch logic

This is a structural refactor with no behavior change — existing tests continue to pass.

## 9. New Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `pulldown-cmark` | latest stable | Markdown parsing for viewer body rendering |

## 10. Files Changed / Created

### New files
- `src/tui/widgets/focusable_block.rs`
- `src/tui/widgets/breadcrumb_bar.rs`
- `src/tui/widgets/section_separator.rs`
- `src/tui/widgets/markdown_renderer.rs`
- `src/tui/app/state.rs` (extracted from app.rs)
- `src/tui/app/actions.rs` (extracted from app.rs)
- `src/tui/app/queries.rs` (extracted from app.rs)

### Modified files
- `src/tui/app.rs` → `src/tui/app/mod.rs` (slimmed, imports extracted modules)
- `src/tui/event.rs` (Tab/Shift-Tab handling, FocusRegion integration)
- `src/tui/tabs/board.rs` → renamed to `src/tui/tabs/projects.rs` (project list level, breadcrumbs, focus blocks)
- `src/tui/tabs/search.rs` (focus blocks, separator, selected-item highlight)
- `src/tui/tabs/context.rs` (focus blocks, separator, selected-item highlight)
- `src/tui/tabs/maintain.rs` (focus blocks, separator)
- `src/tui/views/viewer.rs` (breadcrumbs, frontmatter block, markdown renderer, offset background)
- `src/tui/widgets/mod.rs` (new module exports)
- `src/tui/widgets/keyhints.rs` (updated for Tab/Shift-Tab hints, Projects rename)
- `src/tui/widgets/swimlane.rs` (use FocusableBlock instead of direct Block)
- `Cargo.toml` (add pulldown-cmark)

## 11. Testing Strategy

- **Existing tests**: All 128+ existing tests must continue to pass after the app.rs decomposition (pure structural refactor)
- **Widget unit tests**: Each new widget gets basic render tests verifying correct border styles and content
- **Focus cycling tests**: Test that Tab/Shift-Tab correctly cycles through regions for each screen type
- **Markdown renderer tests**: Test rendering of each markdown element type (headers, bold, code blocks, lists, links)
- **Integration**: Manual verification with `temper tui` against an indexed vault
