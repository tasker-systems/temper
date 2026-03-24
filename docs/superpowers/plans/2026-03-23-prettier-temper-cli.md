# Prettier Temper CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add styled terminal output to all temper CLI commands using anstream/anstyle, following the tasker-ctl output module pattern.

**Architecture:** Create `src/output/` module with style constants (`styles.rs`) and semantic helper functions (`mod.rs`). Then convert every command file's `println!`/`eprintln!` calls to use the appropriate output helper. Warmup and skill generate are excluded (machine-consumed output).

**Tech Stack:** anstream 0.6, anstyle 1.0 (both already in Cargo.toml), clap (styles integration)

**Spec:** `docs/superpowers/specs/2026-03-23-prettier-temper-cli-design.md`

---

### Task 1: Create output module — styles.rs

**Files:**
- Create: `src/output/styles.rs`

- [ ] **Step 1: Create `src/output/styles.rs` with style constants and clap_styles**

```rust
//! Style constants and clap help styling configuration.

use anstyle::{AnsiColor, Effects, Style};

/// Green — success messages, healthy status.
pub(crate) const SUCCESS: Style =
    Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green)));

/// Red — errors, unhealthy status.
pub(crate) const ERROR: Style =
    Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)));

/// Yellow — warnings, caution messages.
pub(crate) const WARNING: Style =
    Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Yellow)));

/// Bold — section headers.
pub(crate) const HEADER: Style = Style::new().effects(Effects::BOLD);

/// Bold — label names in "Label: value" pairs.
pub(crate) const LABEL: Style = Style::new().effects(Effects::BOLD);

/// Dimmed — secondary/muted information.
pub(crate) const DIM: Style = Style::new().effects(Effects::DIMMED);

/// Dimmed — hints and guidance text.
pub(crate) const HINT: Style = Style::new().effects(Effects::DIMMED);

/// Custom clap styles for help output, matching our CLI palette.
pub(crate) fn clap_styles() -> clap::builder::Styles {
    clap::builder::Styles::styled()
        .header(
            Style::new()
                .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green)))
                .effects(Effects::BOLD),
        )
        .usage(
            Style::new()
                .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green)))
                .effects(Effects::BOLD),
        )
        .literal(Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Cyan))))
        .placeholder(Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Cyan))))
        .error(
            Style::new()
                .fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)))
                .effects(Effects::BOLD),
        )
        .valid(Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green))))
        .invalid(Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Yellow))))
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features 2>&1 | tail -5`
Expected: This will fail because the module isn't declared yet — that's fine, proceed to next step.

---

### Task 2: Create output module — mod.rs and wire up

**Files:**
- Create: `src/output/mod.rs`
- Modify: `src/lib.rs:1` (add `pub mod output;`)

- [ ] **Step 1: Create `src/output/mod.rs` with all semantic helpers**

```rust
//! Styled terminal output for temper.
//!
//! Uses `anstyle` for ANSI style definitions and `anstream` for auto-detecting
//! terminal capabilities. Output gracefully degrades to plain text when piped
//! or when the terminal doesn't support colors.

mod styles;

use std::io::Write;

pub use styles::clap_styles;

use styles::{DIM, ERROR, HEADER, HINT, LABEL, SUCCESS, WARNING};

/// Print a success message (green checkmark prefix).
pub fn success(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{SUCCESS}✓{SUCCESS:#} {SUCCESS}{msg}{SUCCESS:#}").ok();
}

/// Print an error message to stderr (red X prefix).
pub fn error(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{ERROR}✗ {msg}{ERROR:#}").ok();
}

/// Print a warning message (yellow exclamation prefix).
pub fn warning(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{WARNING}! {msg}{WARNING:#}").ok();
}

/// Print a section header (bold).
pub fn header(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{HEADER}{msg}{HEADER:#}").ok();
}

/// Print a labeled value ("  Label: value" with the label bolded).
pub fn label(name: impl std::fmt::Display, value: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "  {LABEL}{name}:{LABEL:#} {value}").ok();
}

/// Print dimmed/muted text (for secondary information).
pub fn dim(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{DIM}{msg}{DIM:#}").ok();
}

/// Print a hint/suggestion (dimmed, for guidance text).
pub fn hint(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{HINT}{msg}{HINT:#}").ok();
}

/// Print a status line with a colored icon based on health/status.
pub fn status_icon(healthy: bool, msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    if healthy {
        writeln!(out, "  {SUCCESS}✓{SUCCESS:#} {msg}").ok();
    } else {
        writeln!(out, "  {ERROR}✗{ERROR:#} {msg}").ok();
    }
}

/// Print a list item with a bullet prefix.
pub fn item(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "  • {msg}").ok();
}

/// Print a blank line.
pub fn blank() {
    let mut out = anstream::stdout().lock();
    writeln!(out).ok();
}

/// Print plain text to stdout (for output that doesn't need styling).
pub fn plain(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{msg}").ok();
}

/// Print inline progress to stderr (no trailing newline).
pub fn progress(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    write!(out, "{DIM}{msg}{DIM:#}").ok();
}
```

- [ ] **Step 2: Add `pub mod output;` to `src/lib.rs`**

Add after line 1 (after `pub mod chunker;`):
```rust
pub mod output;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS — no errors

- [ ] **Step 4: Commit**

```bash
git add src/output/styles.rs src/output/mod.rs src/lib.rs
git commit -m "feat: add output module with styled terminal helpers"
```

---

### Task 3: Wire clap_styles into CLI and style main.rs error

**Files:**
- Modify: `src/cli.rs:4-7` (add styles attribute)
- Modify: `src/main.rs:18-20` (use output::error for top-level errors)

- [ ] **Step 1: Add `styles` attribute to `Cli` struct**

In `src/cli.rs`, change:
```rust
#[command(
    name = "temper",
    about = "Developer workflow tool for agent-assisted development"
)]
```
to:
```rust
#[command(
    name = "temper",
    about = "Developer workflow tool for agent-assisted development",
    styles = temper_cli::output::clap_styles()
)]
```

- [ ] **Step 2: Style the top-level error in `src/main.rs`**

Change:
```rust
    if let Err(e) = run(cli) {
        eprintln!("temper: {e}");
        std::process::exit(1);
    }
```
to:
```rust
    if let Err(e) = run(cli) {
        temper_cli::output::error(format!("temper: {e}"));
        std::process::exit(1);
    }
```

- [ ] **Step 3: Verify it compiles and help output is styled**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: wire clap_styles and styled error output"
```

---

### Task 4: Convert init.rs

**Files:**
- Modify: `src/commands/init.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map (all `eprintln!` → output helpers):

| Line | Old | New |
|------|-----|-----|
| 38 | `eprintln!("temper: creating vault at {}", path.display())` | `output::dim(format!("Creating vault at {}", path.display()))` |
| 44 | `eprintln!("temper: temper.toml already exists, skipping")` | `output::dim("temper.toml already exists, skipping")` |
| 47 | `eprintln!("temper: wrote temper.toml")` | `output::success("Wrote temper.toml")` |
| 54 | `eprintln!("temper: created {dir}/")` | `output::item(format!("Created {dir}/"))` |
| 71 | `eprintln!()` | `output::blank()` |
| 72 | `eprintln!("temper: vault initialized successfully.")` | `output::success("Vault initialized successfully")` |
| 73 | `eprintln!()` | `output::blank()` |
| 74-80 | Next steps block | `output::header("Next steps")` + `output::hint(...)` for each line |
| 91-94 | `eprintln!("temper: wrote {}")` | `output::item(format!("Wrote {}", ...))` |
| 106 | `eprintln!("temper: global config already has default_vault set, skipping")` | `output::dim("Global config already has default_vault set, skipping")` |
| 129-133 | `eprintln!("temper: registered {} as default vault in {}")` | `output::dim(format!("Registered {} as default vault in {}", ...))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/init.rs
git commit -m "feat: style init command output"
```

---

### Task 5: Convert check.rs

**Files:**
- Modify: `src/commands/check.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map:

| Line | Old | New |
|------|-----|-----|
| 12 | `eprintln!("temper: {msg}")` (vault quiet) | `output::error(msg)` |
| 15 | `eprintln!("temper: {msg}")` (dirs quiet) | `output::error(msg)` |
| 18 | `eprintln!("temper: embedding model: {msg}")` | `output::error(format!("Embedding: {msg}"))` |
| 21 | `eprintln!("temper: state: {msg}")` | `output::error(format!("State: {msg}"))` |
| 27 | `eprintln!("Vault:     OK ({})")` | `output::status_icon(true, format!("Vault: {}", config.vault_root.display()))` |
| 28 | `eprintln!("Vault:     FAIL ({msg})")` | `output::status_icon(false, format!("Vault: {msg}"))` |
| 32 | `eprintln!("Dirs:      OK (...)")` | `output::status_icon(true, "Dirs: sessions, tickets, milestones, templates")` |
| 33 | `eprintln!("Dirs:      WARN ({msg})")` | `output::warning(format!("Dirs: {msg}"))` |
| 37 | `eprintln!("Embedding: OK (...)")` | `output::status_icon(true, format!("Embedding: model cached, {size_mb:.1}MB"))` |
| 38 | `eprintln!("Embedding: {msg}")` | `output::status_icon(false, format!("Embedding: {msg}"))` |
| 42 | `eprintln!("State:     OK (...)")` | `output::status_icon(true, format!("State: {}", config.state_dir.display()))` |
| 43 | `eprintln!("State:     {msg}")` | `output::status_icon(false, format!("State: {msg}"))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/check.rs
git commit -m "feat: style check command output"
```

---

### Task 6: Convert status.rs

**Files:**
- Modify: `src/commands/status.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map:

| Line | Old | New |
|------|-----|-----|
| 7 | `println!("Temper Vault")` | `output::header("Temper Vault")` |
| 8 | `println!("  Root:       {}", ...)` | `output::label("Root", config.vault_root.display())` |
| 9 | `println!()` | `output::blank()` |
| 17 | `println!("Files")` | `output::header("Files")` |
| 18 | `println!("  Sessions:   {}", sessions)` | `output::label("Sessions", sessions)` |
| 19 | `println!("  Tickets:    {}", tickets)` | `output::label("Tickets", tickets)` |
| 20 | `println!("  Milestones: {}", milestones)` | `output::label("Milestones", milestones)` |
| 21 | `println!("  Templates:  {}", templates)` | `output::label("Templates", templates)` |
| 22 | `println!()` | `output::blank()` |
| 25 | `println!("Index")` | `output::header("Index")` |
| 36-44 | Index stats `print!`/`println!` block | `output::label("Chunks", format!(...))` for the stats line, keeping conditional last-indexed suffix |
| 46-48 | `Err` arm: `println!("  not built ...")` | `output::hint("  not built — run 'temper index embed'")` |
| 50 | `println!()` | `output::blank()` |
| 53 | `println!("Projects")` | `output::header("Projects")` |
| 55 | `println!("  (none configured)")` | `output::hint("  (none configured)")` |
| 62 | `println!("  {} — {} ({})", ...)` (verbose) | `output::plain(format!("  {} — {} ({})", ...))` |
| 64 | `println!("  {}", name)` | `output::plain(format!("  {}", name))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/status.rs
git commit -m "feat: style status command output"
```

---

### Task 7: Convert ticket.rs

**Files:**
- Modify: `src/commands/ticket.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map:

| Line | Old | New |
|------|-----|-----|
| 201 | `eprintln!("Created ticket: {slug}")` | `output::success(format!("Created ticket: {slug}"))` |
| 281 | `eprintln!("Moved ticket {}: ...")` | `output::success(format!("Moved ticket {}: {from_stage} → {to_stage}", ticket.slug))` |
| 322 | `eprintln!("Completed ticket: {}", ...)` | `output::success(format!("Completed ticket: {}", ticket.slug))` |
| 335 | `print!("{content}")` | `output::plain(content.trim_end())` — note: use `print!` via anstream if content has no trailing newline, or use `plain` which adds one. Since ticket markdown typically ends with newline, `print!` should stay as-is to avoid double newline. Keep as `print!("{content}")`. |
| 343 | `println!("No tickets found.")` | `output::hint("No tickets found.")` |
| 356 | `println!("\n## {ms}")` | `output::blank()` then `output::header(format!("## {ms}"))` |
| 358-361 | `println!("  {:>3}  [{:<10}] ...")` | `output::plain(format!("  {:>3}  [{:<10}]  {} ({})", t.seq, t.stage, t.title, t.project))` |
| 383 | `println!("{project_title} Board")` | `output::header(format!("{project_title} Board"))` |
| 384 | `println!("{}", "═".repeat(68))` | `output::plain("═".repeat(68))` |
| 397-408 | Board column headers and separator | `output::plain(format!(...))` for both lines |
| 450-453 | Board row cells | `output::plain(format!(...))` |
| 456-464 | Board footer separator | `output::plain(format!(...))` |
| 473 | `println!(" Milestone: ...")` | `output::plain(format!(" Milestone: {} ({counts_str})", ms.title))` |
| 474 | `println!()` | `output::blank()` |
| 520 | `eprintln!("Board written to boards/{project}.md")` | `output::dim(format!("Board written to boards/{project}.md"))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/ticket.rs
git commit -m "feat: style ticket command output"
```

---

### Task 8: Convert milestone.rs

**Files:**
- Modify: `src/commands/milestone.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map:

| Line | Old | New |
|------|-----|-----|
| 159 | `eprintln!("Created milestone: {slug}")` | `output::success(format!("Created milestone: {slug}"))` |
| 206 | `println!("No milestones for project: {project}")` | `output::hint(format!("No milestones for project: {project}"))` |
| 211 | `println!("{project_title} Roadmap")` | `output::header(format!("{project_title} Roadmap"))` |
| 212 | `println!("{}", "─".repeat(14))` | `output::plain("─".repeat(14))` |
| 242-245 | `println!(" {seq_display}  {:<24} ...")` | `output::plain(format!(...))` |
| 280 | `eprintln!("Updated milestone {slug} → {status}")` | `output::success(format!("Updated milestone: {slug} → {status}"))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/milestone.rs
git commit -m "feat: style milestone command output"
```

---

### Task 9: Convert session.rs

**Files:**
- Modify: `src/commands/session.rs`

- [ ] **Step 1: Add output import and convert all output calls**

Add at top of file:
```rust
use crate::output;
```

Conversion map:

| Line | Old | New |
|------|-----|-----|
| 54 | `println!("Updated: {}", ...)` | `output::success(format!("Updated: {}", relative.display()))` |
| 83 | `println!("Created: {relative_str}")` | `output::success(format!("Created: {relative_str}"))` |
| 108 | `println!("No sessions directory found.")` | `output::warning("No sessions directory found.")` |
| 148 | `println!("No sessions found.")` | `output::hint("No sessions found.")` |
| 152 | `println!("{:<12} {:<20} Title", "Date", "Project")` | `output::plain(format!("{:<12} {:<20} Title", "Date", "Project"))` |
| 153 | `println!("{}", "-".repeat(60))` | `output::dim("-".repeat(60))` |
| 155 | `println!("{:<12} {:<20} {}", ...)` | `output::plain(format!("{:<12} {:<20} {}", entry.date, entry.project, entry.title))` |

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/session.rs
git commit -m "feat: style session command output"
```

---

### Task 10: Convert project.rs, note.rs, events.rs, search.rs, context.rs

**Files:**
- Modify: `src/commands/project.rs`
- Modify: `src/commands/note.rs`
- Modify: `src/commands/events.rs`
- Modify: `src/commands/search.rs`
- Modify: `src/commands/context.rs`

These are small files with few output calls each, batched into one task.

- [ ] **Step 1: Convert project.rs**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 36-39 | `eprintln!("temper: added project '{}' ...")` | `output::success(format!("Added project '{}' (path={}, repo={})", name, path, resolved_repo))` |
| 71 | `eprintln!("temper: removed project '{}'", name)` | `output::success(format!("Removed project '{}'", name))` |
| 78 | `println!("No projects configured.")` | `output::hint("No projects configured.")` |
| 85 | `println!("{:<20} {:<40} REPO", ...)` | `output::plain(format!("{:<20} {:<40} REPO", "NAME", "PATH"))` |
| 86 | `println!("{}", "-".repeat(80))` | `output::dim("-".repeat(80))` |
| 89 | `println!("{:<20} {:<40} {}", ...)` | `output::plain(format!("{:<20} {:<40} {}", name, p.path.display(), p.repo))` |

- [ ] **Step 2: Convert note.rs**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 59 | `println!("Created: {relative_str}")` | `output::success(format!("Created: {relative_str}"))` |

- [ ] **Step 3: Convert events.rs**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 98 | `println!("No events found.")` | `output::hint("No events found.")` |
| 110 | `println!("{}", format_event(event))` | `output::plain(format_event(event))` |

Leave JSON path (`println!` with serde_json) unchanged.

- [ ] **Step 4: Convert search.rs**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 87 | `println!("No search index found. ...")` | `output::warning("No search index found. Run 'temper index' to build it.")` |
| 93 | `println!("Index is empty. ...")` | `output::warning("Index is empty. Run 'temper index' to populate it.")` |

- [ ] **Step 5: Convert context.rs**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 171 | `println!("No search index found. ...")` | `output::warning("No search index found. Run 'temper index' to build it.")` |

- [ ] **Step 6: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/commands/project.rs src/commands/note.rs src/commands/events.rs src/commands/search.rs src/commands/context.rs
git commit -m "feat: style project, note, events, search, context output"
```

---

### Task 11: Convert index.rs

**Files:**
- Modify: `src/commands/index.rs`

This is the most complex conversion due to the inline progress pattern.

- [ ] **Step 1: Add output import and convert all output calls**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 99 | `eprintln!("Collecting files: {} total", ...)` | `output::dim(format!("Collecting files: {} total", all_files.len()))` |
| 110 | `eprintln!("  Warning: could not hash ...")` | `output::warning(format!("Could not hash {}: {e}", file.display()))` |
| 143-147 | `eprintln!("Files to embed: ...")` | `output::dim(format!("Files to embed: {} new/changed, {} unchanged", ...))` |
| 156 | `eprintln!("  Note: no existing index ...")` | `output::dim("  No existing index found; all files will be re-embedded")` |
| 201 | `eprintln!("  Warning: could not read ...")` | `output::warning(format!("Could not read {rel_path}: {e}"))` |
| 231 | `eprintln!("  Note: large file ...")` | `output::dim(format!("  Large file ({}): {}", meta.len(), rel_path))` |
| 239 | `eprintln!("  Warning: skipping ...")` | `output::warning(format!("Skipping {rel_path} (read error: {e})"))` |
| 244 | `eprint!("  [{}/{}] {} ", ...)` | `output::progress(format!("  [{}/{}] {} ", i + 1, total_to_embed, rel_path))` |
| 248 | `eprintln!("— skipped (no chunks)")` | `eprintln!("— skipped (no chunks)")` (continuation of progress line, keep as-is) |
| 254 | `eprintln!("— {} chunks", ...)` | `eprintln!("— {} chunks", chunks.len())` (continuation, keep as-is) |
| 293 | `eprintln!("— ERROR: {e}")` | `eprintln!("— ERROR: {e}")` (continuation of progress line, keep as-is — adding red would require exposing style constants for inline use, not worth the complexity for this edge case) |
| 299-305 | `eprintln!()` + `eprintln!("Building HNSW index: ...")` | `output::blank()` (via stderr: `eprintln!()`) then `output::dim(format!("Building HNSW index: ..."))` |
| 343-344 | `println!()` + `println!("Index complete: ...")` | `output::blank()` then `output::success(format!("Index complete: ..."))` |
| 351 | `println!("Index saved to: {}")` | `output::plain(format!("Index saved to: {}", ...))` |
| 352-355 | `println!("Registry saved to: {}")` | `output::plain(format!("Registry saved to: {}", ...))` |
| 357 | `println!("Cleaned {} orphaned ...")` | `output::dim(format!("Cleaned {} orphaned external entries.", orphaned.len()))` |

Note: Lines 248, 254, 293 are continuations of the `progress` line (they complete it with a newline), so they stay as `eprintln!`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/index.rs
git commit -m "feat: style index command output"
```

---

### Task 12: Convert skill.rs and main.rs skill install

**Files:**
- Modify: `src/commands/skill.rs`
- Modify: `src/main.rs:288` (skill install success message)

- [ ] **Step 1: Add output import to skill.rs and convert check() output**

Add `use crate::output;` and convert:

| Line | Old | New |
|------|-----|-----|
| 126 | `println!("Superpowers: OK (...)")` | `output::status_icon(true, format!("Superpowers: {}", superpowers_path.display()))` |
| 128 | `println!("Superpowers: NOT FOUND (...)")` | `output::status_icon(false, format!("Superpowers: NOT FOUND ({})", superpowers_path.display()))` |
| 134 | `println!("Skill file:  NOT FOUND (...)")` | `output::status_icon(false, format!("Skill file: NOT FOUND ({})", skill_path.display()))` |
| 135 | `println!("  Run: temper skill install")` | `output::hint("  Run: temper skill install")` |
| 139 | `println!("Skill file:  OK (...)")` | `output::status_icon(true, format!("Skill file: {}", skill_path.display()))` |
| 155 | `println!("Hash:        OK (up to date)")` | `output::status_icon(true, "Hash: up to date")` |
| 158 | `println!("Hash:        STALE")` | `output::status_icon(false, "Hash: STALE")` |
| 159-161 | Embedded/Current/Run lines | `output::plain(format!("  Embedded: {}", h))`, `output::plain(format!("  Current:  {}", current_hash))`, `output::hint("  Run: temper skill install")` |
| 164 | `println!("Hash:        UNKNOWN (...)")` | `output::warning("Hash: UNKNOWN (no config-hash comment found)")` |

- [ ] **Step 2: Convert skill install message in main.rs**

In `src/main.rs` line 288, change:
```rust
println!("Skill installed: {}", output_path.display());
```
to:
```rust
temper_cli::output::success(format!("Skill installed: {}", output_path.display()));
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/commands/skill.rs src/main.rs
git commit -m "feat: style skill command output"
```

---

### Task 13: Run full test suite and verify

**Files:** None (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --all-features`
Expected: All tests pass (76+ tests)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings or errors

- [ ] **Step 3: Build release binary and smoke test**

Run: `cargo build --release && ./target/release/temper status`
Expected: Styled output with bold headers, labeled values

Run: `./target/release/temper --help`
Expected: Colored help output (green headers, cyan literals)

Run: `./target/release/temper status | cat`
Expected: Plain text (no ANSI codes) — verifies anstream auto-detection

- [ ] **Step 4: Install updated binary**

Run: `cargo install --path . --force`
