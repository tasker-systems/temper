# Temper TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `temper ticket board` with an interactive ratatui-based TUI (`temper tui`) providing navigable swimlanes, semantic search, context graph walking, and inline ticket mutation.

**Architecture:** Three-layer split — `src/actions/` (pure data logic), `src/commands/` (CLI wrappers), `src/tui/` (ratatui app). Query actor on a dedicated std::thread with tokio mpsc channels for non-blocking search/context/index/normalize. Single-pane-focus UX with vim-style keybindings and `:` command mode.

**Tech Stack:** ratatui, crossterm, tokio (rt, macros, sync), existing candle/HNSW pipeline.

**Spec:** `docs/superpowers/specs/2026-03-24-temper-tui-design.md`

---

## File Structure

### New Files

```
src/actions/mod.rs          — Module declarations
src/actions/ticket.rs       — Ticket data operations (extracted from commands/ticket.rs)
src/actions/milestone.rs    — Milestone data operations (extracted from commands/milestone.rs)
src/actions/search.rs       — Search execution (extracted from commands/search.rs)
src/actions/context.rs      — Context graph traversal (extracted from commands/context.rs)
src/actions/index.rs        — Index rebuild (extracted from commands/index.rs)
src/actions/normalize.rs    — Normalize execution (extracted from commands/normalize.rs)
src/actions/types.rs        — Shared return types (SearchHit, ContextResult, IndexStats, etc.)
src/actions/vault.rs        — Vault document reader (read any vault file into VaultDocument)

src/tui/mod.rs              — TUI entry point, terminal setup/teardown
src/tui/app.rs              — App struct, Screen enum, navigation stack, state management
src/tui/event.rs            — Crossterm event handling, AppAction enum, command mode parsing
src/tui/query_actor.rs      — Dedicated thread actor: QueryRequest/QueryResult, debounce, dispatch
src/tui/tabs/mod.rs         — Tab module declarations
src/tui/tabs/board.rs       — Board tab: project list → milestones → swimlanes rendering + state
src/tui/tabs/search.rs      — Search tab: input widget + results list rendering + state
src/tui/tabs/context.rs     — Context tab: centered topic + neighbor list rendering + state
src/tui/tabs/maintain.rs    — Maintain tab: index stats + normalize status rendering + state
src/tui/views/mod.rs        — View module declarations
src/tui/views/viewer.rs     — Full-screen document viewer rendering + state
src/tui/views/popup.rs      — Stage picker, scope picker popup rendering
src/tui/widgets/mod.rs      — Widget module declarations
src/tui/widgets/swimlane.rs — Kanban column widget (stateless ratatui widget)
src/tui/widgets/result_list.rs  — Ranked result list widget (shared by search + context)
src/tui/widgets/frontmatter.rs  — Frontmatter header renderer widget
src/tui/widgets/keyhints.rs     — Context-sensitive bottom bar key hints widget
src/tui/widgets/command_line.rs — Command mode input widget

tests/actions_ticket_test.rs    — Tests for actions::ticket
tests/actions_milestone_test.rs — Tests for actions::milestone
tests/actions_search_test.rs    — Tests for actions::search
tests/actions_context_test.rs   — Tests for actions::context
tests/tui_state_test.rs         — Tests for app state transitions, navigation stack, command parsing
tests/tui_query_actor_test.rs   — Tests for query actor request/response
```

### Modified Files

```
Cargo.toml                  — Add ratatui, crossterm, tokio dependencies
src/lib.rs                  — Add `pub mod actions;` and `pub mod tui;`
src/cli.rs                  — Add Tui subcommand, remove TicketAction::Board
src/main.rs                 — Route Tui command, remove board dispatch
src/commands/ticket.rs      — Replace data functions with re-exports from actions::ticket
src/commands/milestone.rs   — Replace data functions with re-exports from actions::milestone
src/commands/search.rs      — Call actions::search, format result
src/commands/context.rs     — Call actions::context, format result
src/commands/index.rs       — Call actions::index, format progress
src/commands/normalize.rs   — Call actions::normalize, format summary
```

---

### Implementation Notes

**Test setup pattern:** Test code in this plan uses simplified signatures for readability. See `tests/ticket_test.rs` for the correct init + config setup pattern — `init::run()` takes `(path, no_interactive, register_global)` and `config::load()` takes `Option<&str>` not `Option<&Path>`. Follow the existing test conventions.

**Type moves:** When moving types like `TicketInfo`, `MilestoneInfo`, and `NormalizeSummary` to `actions::types`, remove the original definition from `commands/` and re-export from the new location. Don't duplicate structs.

**Crossterm + ratatui version compatibility:** Verify that the crossterm version is compatible with the ratatui version at implementation time. Ratatui's `Cargo.toml` specifies its crossterm requirement.

---

### Task 1: Add Dependencies and Module Scaffolding

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Create: `src/actions/mod.rs`
- Create: `src/tui/mod.rs`
- Create: `src/tui/tabs/mod.rs`
- Create: `src/tui/views/mod.rs`
- Create: `src/tui/widgets/mod.rs`

- [ ] **Step 1: Add ratatui, crossterm, and tokio to Cargo.toml**

Add to `[dependencies]`:
```toml
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["rt", "macros", "sync"] }
```

- [ ] **Step 2: Create empty module files**

Create `src/actions/mod.rs`:
```rust
pub mod types;
pub mod ticket;
pub mod milestone;
pub mod search;
pub mod context;
pub mod index;
pub mod normalize;
pub mod vault;
```

Create `src/tui/mod.rs`:
```rust
mod app;
mod event;
mod query_actor;
mod tabs;
mod views;
mod widgets;

pub use app::run;
```

Create `src/tui/tabs/mod.rs`:
```rust
pub mod board;
pub mod search;
pub mod context;
pub mod maintain;
```

Create `src/tui/views/mod.rs`:
```rust
pub mod viewer;
pub mod popup;
```

Create `src/tui/widgets/mod.rs`:
```rust
pub mod swimlane;
pub mod result_list;
pub mod frontmatter;
pub mod keyhints;
pub mod command_line;
```

- [ ] **Step 3: Add module declarations to lib.rs**

Add to `src/lib.rs`:
```rust
pub mod actions;
pub mod tui;
```

- [ ] **Step 4: Create placeholder files so the module tree compiles**

Each submodule file (app.rs, event.rs, query_actor.rs, each tab, view, widget) needs to exist as an empty file or with minimal content so `cargo check` passes.

- [ ] **Step 5: Run cargo check to verify module tree compiles**

Run: `cargo check`
Expected: Compiles with no errors (may have unused warnings).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: scaffold actions and tui module structure with new dependencies"
```

---

### Task 2: Extract Actions — Ticket

**Files:**
- Create: `src/actions/types.rs`
- Create: `src/actions/ticket.rs`
- Modify: `src/commands/ticket.rs`
- Create: `tests/actions_ticket_test.rs`

- [ ] **Step 1: Write failing test for actions::ticket::load_tickets**

Create `tests/actions_ticket_test.rs`:
```rust
use temper_cli::actions::ticket;
use temper_cli::commands::init;
use tempfile::TempDir;

#[test]
fn test_load_tickets_returns_all_tickets_for_project() {
    let tmp = TempDir::new().unwrap();
    let config = temper_cli::config::load(Some(tmp.path())).unwrap();
    init::run(&config).unwrap();

    // Create a milestone and two tickets
    temper_cli::actions::milestone::create(&config, "test-project", "test-ms", None).unwrap();
    ticket::create(&config, "test-project", "First ticket", Some("test-ms"), None).unwrap();
    ticket::create(&config, "test-project", "Second ticket", Some("test-ms"), None).unwrap();

    let tickets = ticket::load_tickets(&config, Some("test-project"), None).unwrap();
    assert_eq!(tickets.len(), 2);
    assert!(tickets.iter().any(|t| t.title == "First ticket"));
    assert!(tickets.iter().any(|t| t.title == "Second ticket"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test actions_ticket_test -- --nocapture`
Expected: FAIL — `actions::ticket` module doesn't export these functions yet.

- [ ] **Step 3: Create src/actions/types.rs with shared types**

Move `TicketInfo` to `src/actions/types.rs`. This struct currently lives in `src/commands/ticket.rs`. Move it and re-export from the old location for backward compatibility:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub milestone: String,
    pub stage: String,
    pub scope: Option<String>,
    pub seq: u32,
    pub branch: Option<String>,
    pub pr: Option<String>,
}
```

Also add placeholder types for later tasks:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub seq: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub score: f32,
    pub file_path: String,
    pub content: String,
    pub chunk_index: usize,
    pub note_type: String,
    pub cluster: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextNeighbor {
    pub score: f32,
    pub file_path: String,
    pub title: String,
    pub note_type: String,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextResults {
    pub center: String,
    pub neighbors: Vec<ContextNeighbor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub documents: usize,
    pub chunks: usize,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NormalizeSummary {
    pub ids_backfilled: u32,
    pub files_moved: u32,
    pub stages_migrated: u32,
    pub slugs_fixed: u32,
    pub frontmatter_fixed: u32,
    pub unscoped_tickets: u32,
}
```

- [ ] **Step 4: Create src/actions/ticket.rs by extracting data functions**

Move `load_tickets`, `find_ticket`, `next_seq`, `create`, `move_ticket`, and `done` from `src/commands/ticket.rs` into `src/actions/ticket.rs`. These functions are already pure data operations (lines 1-116 in commands/ticket.rs). Use `TicketInfo` from `actions::types`.

The functions keep the same signatures. Import paths change from `crate::commands::ticket` to `crate::actions::ticket`.

```rust
use crate::actions::types::TicketInfo;
use crate::config::Config;
use crate::error::Result;
// ... rest of imports from commands/ticket.rs

pub fn load_tickets(config: &Config, project: Option<&str>, milestone_slug: Option<&str>) -> Result<Vec<TicketInfo>> {
    // Move existing implementation from commands/ticket.rs
}

pub fn find_ticket(config: &Config, slug_or_suffix: &str, project: Option<&str>) -> Result<Option<TicketInfo>> {
    // Move existing implementation
}

pub fn next_seq(config: &Config, project: &str, milestone_slug: &str) -> Result<u32> {
    // Move existing implementation
}

pub fn create(config: &Config, project: &str, title: &str, milestone_slug: Option<&str>, scope: Option<&str>) -> Result<String> {
    // Move existing implementation (includes discovery events)
}

pub fn move_ticket(config: &Config, slug_or_suffix: &str, stage: Option<&str>, new_milestone: Option<&str>, project: Option<&str>, scope: Option<&str>) -> Result<()> {
    // Move existing implementation (includes discovery events)
}

pub fn done(config: &Config, slug_or_suffix: &str, branch: Option<&str>, pr: Option<&str>, project: Option<&str>) -> Result<()> {
    // Move existing implementation
}
```

Also add a new function the TUI needs for reading full document content:
```rust
pub fn read_ticket_content(config: &Config, slug_or_suffix: &str, project: Option<&str>) -> Result<Option<(TicketInfo, String)>> {
    // find_ticket + read file content, return (info, raw_markdown_body)
}
```

- [ ] **Step 5: Update commands/ticket.rs to use actions**

Replace the data function bodies in `src/commands/ticket.rs` with re-exports or delegation:

```rust
// At top of file:
pub use crate::actions::types::TicketInfo;
pub use crate::actions::ticket::{load_tickets, find_ticket, next_seq, create, move_ticket, done};

// Keep list(), show(), board() formatting functions in commands/ticket.rs
// They now call the re-exported action functions internally
```

- [ ] **Step 6: Run tests to verify nothing broke**

Run: `cargo test`
Expected: All existing tests pass. New test passes.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "refactor: extract ticket data operations to actions layer"
```

---

### Task 3: Extract Actions — Milestone

**Files:**
- Create: `src/actions/milestone.rs`
- Modify: `src/commands/milestone.rs`
- Create: `tests/actions_milestone_test.rs`

- [ ] **Step 1: Write failing test for actions::milestone::load_milestones**

Create `tests/actions_milestone_test.rs`:
```rust
use temper_cli::actions::milestone;
use temper_cli::commands::init;
use tempfile::TempDir;

#[test]
fn test_load_milestones_returns_all_for_project() {
    let tmp = TempDir::new().unwrap();
    let config = temper_cli::config::load(Some(tmp.path())).unwrap();
    init::run(&config).unwrap();

    milestone::create(&config, "test-project", "Alpha", None).unwrap();
    milestone::create(&config, "test-project", "Beta", None).unwrap();

    let milestones = milestone::load_milestones(&config, Some("test-project")).unwrap();
    assert!(milestones.len() >= 3); // 2 created + auto-created Maintenance
    assert!(milestones.iter().any(|m| m.title == "Alpha"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test actions_milestone_test -- --nocapture`
Expected: FAIL — module path doesn't resolve yet.

- [ ] **Step 3: Create src/actions/milestone.rs by extracting data functions**

Move `load_milestones`, `find_milestone`, `next_seq`, `ensure_maintenance`, `create`, `update`, and `count_tickets_by_stage` from `src/commands/milestone.rs` into `src/actions/milestone.rs`. Use `MilestoneInfo` from `actions::types`.

```rust
use crate::actions::types::MilestoneInfo;
use crate::config::Config;
use crate::error::Result;
use std::collections::HashMap;

pub fn load_milestones(config: &Config, project: Option<&str>) -> Result<Vec<MilestoneInfo>> { ... }
pub fn find_milestone(config: &Config, slug: &str, project: Option<&str>) -> Result<Option<MilestoneInfo>> { ... }
pub fn next_seq(config: &Config, project: &str) -> Result<u32> { ... }
pub fn ensure_maintenance(config: &Config, project: &str) -> Result<String> { ... }
pub fn create(config: &Config, project: &str, title: &str, slug: Option<&str>) -> Result<String> { ... }
pub fn update(config: &Config, slug: &str, status: &str, project: Option<&str>) -> Result<()> { ... }
pub fn count_tickets_by_stage(config: &Config, project: &str) -> Result<HashMap<String, HashMap<String, usize>>> { ... }
```

- [ ] **Step 4: Update commands/milestone.rs to use actions**

Replace data functions with re-exports. Keep `list()` and formatting in commands/milestone.rs. Note: `create` in commands had a `format` parameter for output — split into `actions::milestone::create` (returns slug) and formatting in commands wrapper.

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor: extract milestone data operations to actions layer"
```

---

### Task 4: Extract Actions — Search and Context

**Files:**
- Create: `src/actions/search.rs`
- Create: `src/actions/context.rs`
- Modify: `src/commands/search.rs`
- Modify: `src/commands/context.rs`
- Create: `tests/actions_search_test.rs`
- Create: `tests/actions_context_test.rs`

- [ ] **Step 1: Write failing test for actions::search::run**

Create `tests/actions_search_test.rs`. This test requires an initialized vault with indexed content, so it will be a higher-level integration test:

```rust
use temper_cli::actions::search;
use temper_cli::actions::types::SearchResults;
use temper_cli::commands::init;
use tempfile::TempDir;

#[test]
fn test_search_returns_results_struct() {
    let tmp = TempDir::new().unwrap();
    let config = temper_cli::config::load(Some(tmp.path())).unwrap();
    init::run(&config).unwrap();

    // Search on empty vault returns empty results, not an error
    let results = search::run(&config, "test query", None, None, 10).unwrap();
    assert_eq!(results.query, "test query");
    assert!(results.hits.is_empty());
}
```

- [ ] **Step 2: Create src/actions/search.rs**

Extract the data-fetching logic from `commands/search.rs`. The function loads the embedder and index, embeds the query, runs the HNSW search, and returns `SearchResults` instead of printing:

```rust
use crate::actions::types::{SearchHit, SearchResults};
use crate::config::Config;
use crate::error::Result;

pub fn run(
    config: &Config,
    query: &str,
    note_type: Option<&str>,
    project: Option<&str>,
    limit: usize,
) -> Result<SearchResults> {
    // Load embedder and index (from commands/search.rs lines 87-97)
    // Embed query, search HNSW (lines 98-113)
    // Build SearchHit vec (lines 115-126)
    // Return SearchResults { query, hits } instead of calling format::output()
}
```

- [ ] **Step 3: Update commands/search.rs to use actions**

```rust
pub fn run(config: &Config, query: &str, format: &str, note_type: Option<&str>, project: Option<&str>, limit: usize) -> Result<()> {
    let results = crate::actions::search::run(config, query, note_type, project, limit)?;
    crate::format::output(&results, format.into());
    Ok(())
}
```

Keep the `Display` impl for `SearchResults` (or a wrapper) in commands/search.rs for text formatting.

- [ ] **Step 4: Write failing test for actions::context::run**

Create `tests/actions_context_test.rs`:
```rust
use temper_cli::actions::context;
use temper_cli::commands::init;
use tempfile::TempDir;

#[test]
fn test_context_returns_results_struct() {
    let tmp = TempDir::new().unwrap();
    let config = temper_cli::config::load(Some(tmp.path())).unwrap();
    init::run(&config).unwrap();

    let results = context::run(&config, "test topic", 1, 10).unwrap();
    assert_eq!(results.center, "test topic");
    assert!(results.neighbors.is_empty());
}
```

- [ ] **Step 5: Create src/actions/context.rs**

Extract data logic from `commands/context.rs`. Move `resolve_topic` and `group_hits` helpers. Return `ContextResults`:

```rust
use crate::actions::types::{ContextNeighbor, ContextResults};
use crate::config::Config;
use crate::error::Result;

pub fn run(
    config: &Config,
    topic: &str,
    depth: usize,
    limit: usize,
) -> Result<ContextResults> {
    // Load embedder and index
    // resolve_topic (moved here)
    // Multi-hop traversal (from commands/context.rs)
    // Build ContextNeighbor vec with depth annotations
    // Return ContextResults { center, neighbors }
}
```

- [ ] **Step 6: Update commands/context.rs to use actions**

Same pattern — call action, format result.

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "refactor: extract search and context data operations to actions layer"
```

---

### Task 5: Extract Actions — Index and Normalize

**Files:**
- Create: `src/actions/index.rs`
- Create: `src/actions/normalize.rs`
- Modify: `src/commands/index.rs`
- Modify: `src/commands/normalize.rs`

- [ ] **Step 1: Create src/actions/normalize.rs**

`normalize::run` already returns `NormalizeSummary`. Move the struct to `actions::types` (done in Task 2) and update the function to use it. The function itself moves to `actions/normalize.rs`, with the CLI wrapper in `commands/normalize.rs` handling the output formatting:

```rust
use crate::actions::types::NormalizeSummary;
use crate::config::Config;
use crate::error::Result;

pub fn run(config: &Config, project: Option<&str>, dry_run: bool, fix_slugs: bool) -> Result<NormalizeSummary> {
    // Move existing implementation from commands/normalize.rs
    // Remove output::* calls — return summary only
    // Keep output::warning() calls as tracing::warn!() instead
}
```

- [ ] **Step 2: Update commands/normalize.rs to format the summary**

```rust
pub fn run(config: &Config, project: Option<&str>, dry_run: bool, fix_slugs: bool) -> Result<()> {
    let summary = crate::actions::normalize::run(config, project, dry_run, fix_slugs)?;
    // Format and print summary using output::* helpers (existing lines 67-85)
    Ok(())
}
```

- [ ] **Step 3: Create src/actions/index.rs with progress callback**

The index command is monolithic. Extract it with a progress callback so the TUI can receive updates:

```rust
use crate::actions::types::IndexStats;
use crate::config::Config;
use crate::error::Result;

pub fn run<F>(
    config: &Config,
    force: bool,
    paths_filter: Option<&str>,
    sources_override: Option<&str>,
    on_progress: F,
) -> Result<IndexStats>
where
    F: Fn(&str),
{
    // Move existing implementation from commands/index.rs
    // Replace output::progress(msg) calls with on_progress(msg)
    // Replace output::dim/warning calls with on_progress or tracing
    // Return IndexStats { documents, chunks, duration_secs }
}
```

- [ ] **Step 4: Update commands/index.rs to use actions with progress**

```rust
pub fn run(config: &Config, force: bool, paths_filter: Option<&str>, sources_override: Option<&str>) -> Result<()> {
    let stats = crate::actions::index::run(config, force, paths_filter, sources_override, |msg| {
        output::progress(msg);
    })?;
    output::success(&format!("Indexed {} documents ({} chunks) in {:.1}s", stats.documents, stats.chunks, stats.duration_secs));
    Ok(())
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: All pass. Existing normalize tests should continue working through the commands layer.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor: extract index and normalize to actions layer with progress callback"
```

---

### Task 6: Add vault content reader to actions

**Files:**
- Modify: `src/actions/ticket.rs`
- Modify: `src/actions/types.rs`

The TUI viewer needs to read full file content for any vault document. Add a general-purpose reader:

- [ ] **Step 1: Add VaultDocument type to actions/types.rs**

```rust
#[derive(Debug, Clone)]
pub struct VaultDocument {
    pub path: String,
    pub note_type: String,
    pub title: String,
    pub frontmatter: serde_yaml::Value,
    pub body: String,
}
```

- [ ] **Step 2: Add read_document function to a new src/actions/vault.rs**

Create `src/actions/vault.rs` and add to `src/actions/mod.rs`:

```rust
use crate::config::Config;
use crate::error::Result;
use crate::actions::types::VaultDocument;
use std::path::Path;

pub fn read_document(path: &Path) -> Result<VaultDocument> {
    // Read file, split frontmatter from body (use vault:: helpers)
    // Parse frontmatter YAML, extract type/title
    // Return VaultDocument
}
```

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add vault document reader to actions layer"
```

---

### Task 7: CLI Changes — Add `tui` Subcommand, Remove `ticket board`

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add Tui variant to Commands enum in cli.rs**

```rust
/// Launch interactive TUI
Tui,
```

- [ ] **Step 2: Remove Board variant from TicketAction enum in cli.rs**

Remove the `Board` variant and its fields from `TicketAction`.

- [ ] **Step 3: Route Tui command in main.rs**

```rust
Commands::Tui => {
    crate::tui::run(&config)?;
}
```

Remove the `TicketAction::Board` match arm.

- [ ] **Step 4: Run cargo check**

Run: `cargo check`
Expected: Compiles (tui::run is a placeholder at this point).

- [ ] **Step 5: Remove board-related test code if any exists**

Check `tests/ticket_test.rs` for board-specific tests and remove them.

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: add tui subcommand, remove ticket board command"
```

---

### Task 8: TUI Core — App State, Event Loop, Terminal Setup

**Files:**
- Modify: `src/tui/mod.rs`
- Create: `src/tui/app.rs`
- Create: `src/tui/event.rs`
- Create: `tests/tui_state_test.rs`

- [ ] **Step 1: Write failing tests for app state transitions**

Create `tests/tui_state_test.rs`:
```rust
use temper_cli::tui::app::{App, AppAction, Tab};

#[test]
fn test_tab_switch_replaces_stack() {
    let mut app = App::new_for_test();
    app.dispatch(AppAction::SwitchTab(Tab::Search));
    assert_eq!(app.active_tab(), Tab::Search);
    assert_eq!(app.stack_depth(), 1);
}

#[test]
fn test_esc_pops_stack() {
    let mut app = App::new_for_test();
    // Simulate drilling into a milestone
    app.dispatch(AppAction::SwitchTab(Tab::Board));
    app.dispatch(AppAction::Enter); // drill into milestone
    assert_eq!(app.stack_depth(), 2);
    app.dispatch(AppAction::Escape);
    assert_eq!(app.stack_depth(), 1);
}

#[test]
fn test_esc_on_root_does_nothing() {
    let mut app = App::new_for_test();
    app.dispatch(AppAction::SwitchTab(Tab::Board));
    assert_eq!(app.stack_depth(), 1);
    app.dispatch(AppAction::Escape);
    assert_eq!(app.stack_depth(), 1); // can't pop below root
}

#[test]
fn test_command_mode_parsing() {
    use temper_cli::tui::event::parse_command;
    assert_eq!(parse_command("q"), Some(AppAction::Quit));
    assert_eq!(parse_command("quit"), Some(AppAction::Quit));
    assert_eq!(parse_command("b"), Some(AppAction::SwitchTab(Tab::Board)));
    assert_eq!(parse_command("search"), Some(AppAction::SwitchTab(Tab::Search)));
    assert_eq!(parse_command("c"), Some(AppAction::SwitchTab(Tab::Context)));
    assert_eq!(parse_command("m"), Some(AppAction::SwitchTab(Tab::Maintain)));
    assert_eq!(parse_command("xyz"), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test tui_state_test -- --nocapture`
Expected: FAIL — types don't exist yet.

- [ ] **Step 3: Implement App struct and state management in src/tui/app.rs**

```rust
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Board,
    Search,
    Context,
    Maintain,
}

#[derive(Debug, Clone)]
pub enum Screen {
    Board(BoardState),
    Search(SearchState),
    Context(ContextState),
    Maintain(MaintainState),
    Viewer(ViewerState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    SwitchTab(Tab),
    Enter,
    Escape,
    Quit,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    OpenEditor,
    StagePicker,
    ScopePicker,
    PivotContext,
    DepthIncrease,
    DepthDecrease,
    FocusSearch,
    TabToResults,
    EnterCommandMode,
    // ... state-specific actions
}

pub struct App {
    stack: Vec<Screen>,
    command_mode: bool,
    command_input: String,
    should_quit: bool,
    // ... TUI-specific state
}

impl App {
    pub fn new(config: &Config) -> Self { ... }
    pub fn new_for_test() -> Self { ... } // test helper with minimal config
    pub fn dispatch(&mut self, action: AppAction) { ... }
    pub fn active_tab(&self) -> Tab { ... }
    pub fn stack_depth(&self) -> usize { self.stack.len() }
    pub fn should_quit(&self) -> bool { self.should_quit }
    pub fn current_screen(&self) -> &Screen { self.stack.last().unwrap() }
}
```

Define `BoardState`, `SearchState`, `ContextState`, `MaintainState`, `ViewerState` structs to hold tab-specific state (selected indices, query text, results, etc.).

- [ ] **Step 4: Implement event mapping in src/tui/event.rs**

```rust
use crate::tui::app::{AppAction, Tab};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn map_key(key: KeyEvent, in_command_mode: bool, in_search_input: bool) -> Option<AppAction> {
    // Map crossterm KeyEvents to AppActions
    // Handle command mode, search input mode, and normal mode
}

pub fn parse_command(input: &str) -> Option<AppAction> {
    // Parse vim-style commands with abbreviation support
    match input {
        "q" | "quit" => Some(AppAction::Quit),
        "b" | "board" => Some(AppAction::SwitchTab(Tab::Board)),
        "s" | "search" => Some(AppAction::SwitchTab(Tab::Search)),
        "c" | "context" => Some(AppAction::SwitchTab(Tab::Context)),
        "m" | "maintain" => Some(AppAction::SwitchTab(Tab::Maintain)),
        "?" | "h" | "help" => Some(AppAction::ToggleHelp),
        _ => None,
    }
}
```

- [ ] **Step 5: Implement TUI entry point in src/tui/mod.rs**

```rust
use crate::config::Config;
use crate::error::Result;

pub fn run(config: &Config) -> Result<()> {
    // 1. Initialize tokio runtime
    // 2. Enter alternate screen, enable raw mode
    // 3. Create App
    // 4. Spawn query actor thread
    // 5. Main event loop: select! on crossterm events and query results
    // 6. On quit: restore terminal, drop runtime
    todo!("TUI event loop — implemented in later tasks")
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test --test tui_state_test -- --nocapture`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: implement TUI app state, event mapping, and command mode parsing"
```

---

### Task 9: Query Actor

**Files:**
- Create: `src/tui/query_actor.rs`
- Create: `tests/tui_query_actor_test.rs`

- [ ] **Step 1: Write failing test for query actor**

Create `tests/tui_query_actor_test.rs`:
```rust
use temper_cli::tui::query_actor::{QueryRequest, QueryResult, spawn_query_actor};
use temper_cli::commands::init;
use tempfile::TempDir;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_query_actor_handles_search_request() {
    let tmp = TempDir::new().unwrap();
    let config = temper_cli::config::load(Some(tmp.path())).unwrap();
    init::run(&config).unwrap();

    let (req_tx, req_rx) = mpsc::channel(16);
    let (res_tx, mut res_rx) = mpsc::channel(16);

    spawn_query_actor(config.clone(), req_rx, res_tx);

    req_tx.send(QueryRequest::Search { query: "test".to_string() }).await.unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        res_rx.recv()
    ).await.unwrap().unwrap();

    assert!(matches!(result, QueryResult::SearchResults(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test tui_query_actor_test -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Implement query actor**

```rust
use crate::actions;
use crate::config::Config;
use tokio::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum QueryRequest {
    Search { query: String },
    Context { topic: String, depth: usize, limit: usize },
    Index { force: bool },
    Normalize { project: Option<String>, dry_run: bool, fix_slugs: bool },
}

#[derive(Debug)]
pub enum QueryResult {
    SearchResults(crate::actions::types::SearchResults),
    ContextResults(crate::actions::types::ContextResults),
    IndexComplete(crate::actions::types::IndexStats),
    NormalizeComplete(crate::actions::types::NormalizeSummary),
    Progress { message: String },
    Error(String),
}

pub fn spawn_query_actor(
    config: Config,
    mut rx: mpsc::Receiver<QueryRequest>,
    tx: mpsc::Sender<QueryResult>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // Block on receiving requests
        while let Some(req) = rx.blocking_recv() {
            // Drain channel for debounce — if more requests queued, skip to latest
            let req = drain_to_latest(req, &mut rx);

            let result = match req {
                QueryRequest::Search { query } => {
                    match actions::search::run(&config, &query, None, None, 20) {
                        Ok(results) => QueryResult::SearchResults(results),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::Context { topic, depth, limit } => {
                    match actions::context::run(&config, &topic, depth, limit) {
                        Ok(results) => QueryResult::ContextResults(results),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::Index { force } => {
                    match actions::index::run(&config, force, None, None, |msg| {
                        let _ = tx.blocking_send(QueryResult::Progress { message: msg.to_string() });
                    }) {
                        Ok(stats) => QueryResult::IndexComplete(stats),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
                QueryRequest::Normalize { project, dry_run, fix_slugs } => {
                    match actions::normalize::run(&config, project.as_deref(), dry_run, fix_slugs) {
                        Ok(summary) => QueryResult::NormalizeComplete(summary),
                        Err(e) => QueryResult::Error(e.to_string()),
                    }
                }
            };

            if tx.blocking_send(result).is_err() {
                break; // TUI closed
            }
        }
    })
}

fn drain_to_latest(current: QueryRequest, rx: &mut mpsc::Receiver<QueryRequest>) -> QueryRequest {
    let mut latest = current;
    while let Ok(newer) = rx.try_recv() {
        latest = newer;
    }
    latest
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test tui_query_actor_test -- --nocapture`
Expected: Pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: implement query actor with debounce on dedicated thread"
```

---

### Task 10: TUI Event Loop and Terminal Setup

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/app.rs`

This task wires up the actual terminal event loop — the crossterm + tokio integration that makes the TUI run.

- [ ] **Step 1: Implement terminal setup/teardown in tui/mod.rs**

```rust
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

pub fn run(config: &Config) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Terminal setup
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Create channels
        let (req_tx, req_rx) = mpsc::channel(16);
        let (res_tx, mut res_rx) = mpsc::channel(16);

        // Spawn query actor
        let _actor_handle = query_actor::spawn_query_actor(config.clone(), req_rx, res_tx);

        // Create app
        let mut app = App::new(config, req_tx);
        app.load_initial_data()?; // Load tickets/milestones for board tab

        // Main loop
        loop {
            terminal.draw(|f| app.render(f))?;

            tokio::select! {
                // Crossterm events
                _ = tokio::task::spawn_blocking(|| event::read()) => {
                    if let Ok(Event::Key(key)) = event::read() {
                        if key.kind == KeyEventKind::Press {
                            if let Some(action) = event::map_key(key, app.in_command_mode(), app.in_search_input()) {
                                app.dispatch(action);
                            }
                        }
                    }
                }
                // Query results
                Some(result) = res_rx.recv() => {
                    app.handle_query_result(result);
                }
            }

            if app.should_quit() {
                break;
            }
        }

        // Teardown
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(())
    })
}
```

Note: The crossterm event reading needs careful handling — `event::read()` is blocking, so it runs in `spawn_blocking`. The actual implementation should use crossterm's `event::poll` with a timeout or the `EventStream` feature for async-friendly event reading.

- [ ] **Step 2: Add render method to App**

Add a `render` method to `App` that dispatches to the appropriate tab/view renderer based on the current screen:

```rust
impl App {
    pub fn render(&self, frame: &mut Frame) {
        // Render tab bar at top
        // Render current screen content
        // Render key hints at bottom
        // Render command line if in command mode
    }
}
```

This is a placeholder — the actual tab renderers are implemented in Tasks 11-15.

- [ ] **Step 3: Test manually**

Run: `cargo run -- tui`
Expected: TUI launches, shows placeholder content, tab switching works, `:q` quits.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: implement TUI event loop with crossterm and tokio select"
```

---

### Task 11: Board Tab — Milestone List and Swimlanes

**Files:**
- Modify: `src/tui/tabs/board.rs`
- Modify: `src/tui/app.rs`
- Create: `src/tui/widgets/swimlane.rs`

- [ ] **Step 1: Implement BoardState in app.rs**

```rust
#[derive(Debug, Clone)]
pub enum BoardLevel {
    Projects { selected: usize, projects: Vec<String> },
    Milestones { project: String, selected: usize, milestones: Vec<MilestoneWithCounts> },
    Swimlanes { project: String, milestone: String, column: usize, row: usize, columns: [Vec<TicketInfo>; 3] },
}

#[derive(Debug, Clone)]
pub struct MilestoneWithCounts {
    pub info: MilestoneInfo,
    pub backlog: usize,
    pub in_progress: usize,
    pub done: usize,
}

#[derive(Debug, Clone)]
pub struct BoardState {
    pub level: BoardLevel,
}
```

- [ ] **Step 2: Implement board tab rendering in tui/tabs/board.rs**

Render based on `BoardLevel`:
- `Projects`: simple list of project names with ticket counts
- `Milestones`: list with summary counts per milestone, breadcrumb showing project
- `Swimlanes`: three columns using the swimlane widget, breadcrumb showing project > milestone

- [ ] **Step 3: Implement swimlane widget in tui/widgets/swimlane.rs**

A stateless ratatui widget that renders a single kanban column:
- Header with stage name and count
- List of ticket cards showing scope badge + truncated title
- Highlight for selected row

```rust
use ratatui::prelude::*;

pub struct Swimlane<'a> {
    pub title: &'a str,
    pub tickets: &'a [TicketInfo],
    pub selected: Option<usize>,
    pub focused: bool,
}

impl Widget for Swimlane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) { ... }
}
```

- [ ] **Step 4: Wire board navigation in App::dispatch**

Handle `MoveUp`, `MoveDown`, `MoveLeft`, `MoveRight`, `Enter`, `Escape` for each board level. `Enter` at milestone level loads tickets and pushes swimlane screen. `Escape` pops back up. `h`/`l` at milestone level cycles projects.

- [ ] **Step 5: Load initial board data on startup**

In `App::load_initial_data()`, resolve project from CWD, load milestones, prepare `BoardState` at the Milestones level.

- [ ] **Step 6: Test manually**

Run: `cargo run -- tui` from within a project directory.
Expected: Board tab shows milestones for inferred project. Arrow keys navigate. Enter drills into swimlanes. Esc goes back.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: implement board tab with milestone list and swimlane view"
```

---

### Task 12: Keyhints and Command Line Widgets

**Files:**
- Create: `src/tui/widgets/keyhints.rs`
- Create: `src/tui/widgets/command_line.rs`

- [ ] **Step 1: Implement keyhints widget**

Context-sensitive key hints rendered at the bottom of the screen. Takes the current screen type and returns the appropriate hint bar:

```rust
pub struct KeyHints<'a> {
    pub screen: &'a Screen,
    pub in_command_mode: bool,
}

impl Widget for KeyHints<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render key hints based on screen type
        // e.g., Board: "j/k move · Enter open · h/l columns · :q quit"
    }
}
```

- [ ] **Step 2: Implement command line widget**

Rendered when `:` is pressed. Shows `:` prefix with input text and cursor:

```rust
pub struct CommandLine<'a> {
    pub input: &'a str,
    pub cursor_pos: usize,
}

impl Widget for CommandLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) { ... }
}
```

- [ ] **Step 3: Integrate into App::render**

Render keyhints at bottom of every screen. When command mode is active, replace keyhints with command line widget.

- [ ] **Step 4: Test manually**

Expected: Key hints appear at bottom, change based on active tab. `:` opens command input. `:q` quits. `:b`, `:s`, `:c`, `:m` switch tabs.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: implement keyhints and command line widgets"
```

---

### Task 13: Search Tab

**Files:**
- Modify: `src/tui/tabs/search.rs`
- Create: `src/tui/widgets/result_list.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement SearchState in app.rs**

```rust
#[derive(Debug, Clone)]
pub struct SearchState {
    pub query: String,
    pub cursor_pos: usize,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub input_focused: bool,
    pub loading: bool,
}
```

- [ ] **Step 2: Implement result_list widget**

Shared widget for both search and context results:

```rust
pub struct ResultList<'a> {
    pub items: &'a [ResultItem<'a>],
    pub selected: usize,
}

pub struct ResultItem<'a> {
    pub score: f32,
    pub path: &'a str,
    pub note_type: &'a str,
    pub snippet: &'a str,
    pub depth: Option<usize>, // for context results
}
```

- [ ] **Step 3: Implement search tab rendering**

Search input at top, result_list below. When loading, show a spinner or "Searching..." indicator.

- [ ] **Step 4: Wire search interaction in App::dispatch**

- When search tab is active and input focused: capture keystrokes as text input
- On each keystroke: send `QueryRequest::Search` to actor (debounce happens in actor)
- `Tab` or `↓` moves focus to results
- `/` from results refocuses input
- `Enter` on result pushes Viewer screen
- `c` on result pushes Context screen centered on that item

- [ ] **Step 5: Handle QueryResult::SearchResults in App**

```rust
pub fn handle_query_result(&mut self, result: QueryResult) {
    match result {
        QueryResult::SearchResults(results) => {
            if let Screen::Search(state) = self.current_screen_mut() {
                state.results = results.hits;
                state.loading = false;
            }
        }
        // ... other variants
    }
}
```

- [ ] **Step 6: Test manually**

Run: `cargo run -- tui` → press `2` or `:s` → type a query → see results → navigate with j/k → Enter to view.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: implement search tab with live query and result navigation"
```

---

### Task 14: Context Tab

**Files:**
- Modify: `src/tui/tabs/context.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement ContextState in app.rs**

```rust
#[derive(Debug, Clone)]
pub struct ContextState {
    pub center_stack: Vec<String>, // stack of centers for Esc navigation
    pub current_center: String,
    pub depth: usize,
    pub neighbors: Vec<ContextNeighbor>,
    pub selected: usize,
    pub loading: bool,
    pub input_active: bool,
    pub input_text: String,
}
```

- [ ] **Step 2: Implement context tab rendering**

Show current center at top with depth indicator. Reuse `result_list` widget for neighbors, grouped by depth with separators. `/` enters topic input.

- [ ] **Step 3: Wire context interaction**

- `c` on a neighbor: push current center to center_stack, send `QueryRequest::Context` for selected neighbor
- `Esc`: pop center_stack, re-query previous center (or exit to tab root if stack empty)
- `+`/`-`: adjust depth (1-3), re-query
- `Enter`: open selected neighbor in Viewer
- `/`: activate topic input mode

- [ ] **Step 4: Test manually**

Run TUI → `:c` → type a topic → see neighbors → `c` to re-center → `Esc` to walk back → `Enter` to view a doc.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: implement context tab with graph walking and center stack"
```

---

### Task 15: Document Viewer

**Files:**
- Modify: `src/tui/views/viewer.rs`
- Create: `src/tui/widgets/frontmatter.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement ViewerState in app.rs**

```rust
#[derive(Debug, Clone)]
pub struct ViewerState {
    pub document: VaultDocument,
    pub scroll_offset: usize,
    pub source_label: String, // breadcrumb back label
}
```

- [ ] **Step 2: Implement frontmatter widget**

Renders key-value pairs from frontmatter YAML in a styled block:

```rust
pub struct FrontmatterWidget<'a> {
    pub frontmatter: &'a serde_yaml::Value,
}

impl Widget for FrontmatterWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render each field as "key: value" with colored values for stage/scope
    }
}
```

- [ ] **Step 3: Implement viewer rendering**

- Breadcrumb at top ("← Search results" etc.)
- Frontmatter block
- Scrollable body text
- Context-sensitive keyhints at bottom (e for editor, c for context, s/S for stage/scope if ticket)

- [ ] **Step 4: Implement $EDITOR handoff**

When `e` is pressed in viewer:
```rust
fn open_in_editor(terminal: &mut Terminal<impl Backend>, path: &str) -> Result<()> {
    // Leave alternate screen, disable raw mode
    // Spawn $EDITOR as child process, wait
    // Re-enable raw mode, enter alternate screen
    // Caller reloads document content after return
}
```

- [ ] **Step 5: Implement scroll with j/k in viewer**

j/k scrolls the body text up/down. Page Up/Page Down for larger jumps.

- [ ] **Step 6: Test manually**

From board or search, Enter on an item → viewer shows frontmatter + body → scroll works → `e` opens editor → `Esc` returns.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: implement document viewer with frontmatter display and editor handoff"
```

---

### Task 16: Mutation Popups — Stage and Scope Pickers

**Files:**
- Modify: `src/tui/views/popup.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement popup rendering**

```rust
pub struct PickerPopup<'a> {
    pub title: &'a str,
    pub options: &'a [(String, Option<String>)], // (value, description)
    pub selected: usize,
}

impl Widget for PickerPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Center a small rect on screen
        // Render bordered box with title
        // List options with highlight on selected
    }
}
```

- [ ] **Step 2: Add popup state to App**

```rust
pub enum PopupState {
    None,
    StagePicker { slug: String, project: String, selected: usize },
    ScopePicker { slug: String, project: String, selected: usize },
}
```

- [ ] **Step 3: Wire popup interaction**

- `s` on a ticket (in swimlanes or viewer): open StagePicker popup
- `S` on a ticket: open ScopePicker popup
- In popup: j/k to select, Enter to confirm, Esc to cancel
- On confirm: call `actions::ticket::move_ticket()` with the selected value
- After mutation: reload board data, update viewer if open

- [ ] **Step 4: Test manually**

From swimlane view, press `s` → stage picker appears → select new stage → ticket moves. Same for `S` with scope.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: implement stage and scope picker popups for ticket mutation"
```

---

### Task 17: Maintain Tab

**Files:**
- Modify: `src/tui/tabs/maintain.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement MaintainState in app.rs**

```rust
#[derive(Debug, Clone)]
pub struct MaintainState {
    pub index_stats: Option<IndexStats>,
    pub last_normalize: Option<NormalizeSummary>,
    pub progress_message: Option<String>,
    pub running: bool,
}
```

- [ ] **Step 2: Implement maintain tab rendering**

Show index stats (if available), normalize summary (if run), and current progress. Key hints: `i` for index, `n` for normalize.

- [ ] **Step 3: Wire maintain actions**

- `i`: send `QueryRequest::Index { force: true }` to actor
- `n`: send `QueryRequest::Normalize { project: inferred, dry_run: false, fix_slugs: false }`
- Handle `QueryResult::Progress` to update progress message
- Handle `QueryResult::IndexComplete` and `NormalizeComplete` to update stats

- [ ] **Step 4: Test manually**

Switch to maintain tab → press `i` → see progress → see completion stats. Same for `n`.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: implement maintain tab with index and normalize triggers"
```

---

### Task 18: Tab Bar Widget and Layout Polish

**Files:**
- Modify: `src/tui/app.rs` (render method)
- Modify: all tab files (layout adjustments)

- [ ] **Step 1: Implement tab bar rendering**

Render four tabs at the top of every screen. Active tab highlighted. Format: `Board · Search · Context · Maintain` with the active one in bold/inverted.

- [ ] **Step 2: Implement breadcrumb rendering for board tab**

Show "All › project › milestone" breadcrumb below tab bar on board screens.

- [ ] **Step 3: Layout polish**

- Ensure all screens use consistent margins
- Tab bar takes 1 row, keyhints take 1 row, content fills the middle
- Breadcrumbs take 1 row when present
- Handle narrow terminals gracefully (truncate titles)

- [ ] **Step 4: Test manually at various terminal sizes**

Resize terminal to 80x24, 120x40, 200x60. Verify nothing overflows or panics.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add tab bar widget and polish layout across all screens"
```

---

### Task 19: Help Overlay

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Implement help overlay**

When `:help` or `:?` is entered, render a centered overlay listing all keybindings organized by category (Navigation, Mutation, Search/Context, Maintenance, Command Mode). Any key press dismisses the overlay.

- [ ] **Step 2: Wire help toggle in dispatch**

Add `AppAction::ToggleHelp` and `show_help: bool` to App state.

- [ ] **Step 3: Test manually**

`:?` → help overlay shows → any key dismisses.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add help overlay with keybinding reference"
```

---

### Task 20: Final Integration and Cleanup

**Files:**
- Modify: various files for cleanup

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-features`
Expected: No warnings.

- [ ] **Step 3: Run fmt**

Run: `cargo fmt --check`
Expected: Clean.

- [ ] **Step 4: Manual end-to-end test**

Launch `cargo run -- tui` from temper project directory. Walk through:
1. Board tab → navigate milestones → drill into swimlanes → view a ticket
2. Change scope via `S` popup
3. Change stage via `s` popup
4. Open in editor via `e`
5. Search tab → query → browse results → view a doc → pivot to context via `c`
6. Context tab → walk the graph → re-center → Esc back through centers
7. Maintain tab → trigger index → trigger normalize
8. `:help` → dismiss
9. `:q` → clean exit

- [ ] **Step 5: Remove any dead code from board command removal**

Check for unused board-related code in `commands/ticket.rs` and remove.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "chore: final cleanup and dead code removal for TUI integration"
```
