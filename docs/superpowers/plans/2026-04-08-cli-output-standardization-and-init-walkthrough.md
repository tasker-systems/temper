# CLI Output Standardization & Init Walkthrough Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Standardize CLI output across all resource types with a schema-driven column registry and unified table renderer, add a guided `temper init` walkthrough using `dialoguer`, and introduce `temper config edit` with `validator`-based safe-write semantics.

**Architecture:** Introduce a three-variant `OutputFormat` enum (`Pretty`/`NoTty`/`Json`) with TTY auto-detection via `std::io::IsTerminal`. A `TableRenderer` and hardcoded `ColumnRegistry` live in `crates/temper-cli/src/output/`, driven by curated per-doc-type column lists that reference schema field names. `resource::list()` becomes the single pipeline that scans, parses, filters, sorts, and renders for all six doc types. `temper init` is rewritten as a `dialoguer` wizard with a non-interactive fallback, and `temper config edit` opens `$EDITOR` on a temp copy, loops until validation passes (using `validator` derives on `TemperConfig`), then atomically replaces the file.

**Tech Stack:** Rust, clap, dialoguer 0.12, validator 0.20 (derive), anstream, anstyle, serde_json, jsonschema, tracing

**Spec:** `docs/superpowers/specs/2026-04-08-cli-output-standardization-and-init-walkthrough-design.md`

**Branch:** `jct/cli-enhancements-and-init-walkthrough`

## Decisions Log (resolved during planning)

These resolutions override anything in the original plan body that conflicts. Task sections below have been updated to match; this log is the canonical record.

1. **Existing-vault detection in `temper init`** — warn only, do NOT offer to reconfigure. Point the user at `temper config edit` if they want to change anything. (Task 9)
2. **`load_config` validation warnings** — add `tracing` as a dep of `temper-core` and emit `tracing::warn!` from `load_config_from` when `TemperConfig::validate()` fails. Binary-size cost is zero because every downstream binary already pulls in `tracing`. (Task 7 or new Task 7b.)
3. **`auth.provider = "none"` runtime behavior** — `temper-client::config::build_oauth_config` already returns a `get(&provider)`-miss error. Clean break is acceptable; we only need to improve the error message so the user is told to run `temper config edit` or pick a real provider. (New task, see Task 11a.)
4. **`AuthProvider` shape refactor** — replace `AuthProviderConfig` + `HashMap<String, AuthProviderConfig>` with a new `AuthProvider { name: String, authorize_url, token_url, client_id, audience, callback_url, scopes }` struct and `providers: Vec<AuthProvider>`. Lookup becomes `providers.iter().find(|p| p.name == auth.provider)`. Validator `#[validate(nested)]` traverses vec elements natively. **This is a clean TOML format break** — existing configs using `[auth.providers.auth0]` will no longer parse; the new format is `[[auth.providers]] name = "auth0"`. Users re-run `temper init` or hand-edit. Accepted because this branch already wipes other cruft. (Task 7.)
5. **`TemperConfig::default()` vault path** — change from `"~/vault"` to `"~/Documents/temper-vault"` so `temper config edit` on first run seeds a sensible path. (Task 7 Step 5.)
6. **`TableRenderer` output shape** — `render_pretty()` and `render_no_tty()` return `String`; callers pass the string to `output::plain()`. Easier to unit test, matches the existing `output::*` helper pattern, and the allocation cost is trivial for table-sized output. (Task 2.)

---

**Test commands:**
- Unit tests (single): `cargo nextest run -p temper-cli <test_name>`
- Unit tests (all): `cargo make test`
- Clippy + fmt + machete: `cargo make check`
- Full suite: `cargo make test-all`

## Subagent Guidance (apply to every task)

### SG-1: Follow Existing Patterns
Before writing anything, read the file you're modifying AND a sibling in the same module. Match the style you find: naming, imports, structure, error handling. Don't invent new patterns.

### SG-2: Single Responsibility
Each function does one thing. If it constructs AND processes AND formats — split it. Follow the project's existing layering.

### SG-3: No Logic Duplication
Would two implementations drift independently over time? Extract. Otherwise leave inline. Don't create premature abstractions for one-time operations.

### SG-4: Test Strategy
Unit tests co-located with code. Integration tests separate. One behavior per test with descriptive names. Tests must actually run — verify, don't assume.

### SG-5: Don't Over-Build
Implement exactly what the task says. No speculative features, no defensive code for impossible cases, no "nice to have" extras.

### SG-6: Verify Before Claiming Done
Run the verification command. Read the output. Don't claim success based on what you think the code does.

### SG-7: Prefer Native Solutions
Don't invent when the framework, language, or platform provides. If a proper tool exists, use it over a hand-rolled alternative.

### SG-8: Front-Load Constraints
Before proposing anything: (1) existing abstractions for this? (2) platform/deployment limits? (3) async/performance requirements? List findings before writing code.

### SG-9: Don't Dismiss Owned Failures
If the user owns both sides of an interaction, debug the full stack. Never declare "not our problem" without proving external causation.

### SG-10: Checkpoint Before Continuing
After each major step, report: what's done, what's next, any concerns about approach drift.

## Project Fundamentals (apply to every task)

- Thin commands, fat actions (commands/ parses args and calls actions/, which own logic).
- Typed structs over `serde_json::json!()` — if a shape is known, define a struct.
- Functions must be decomposed and independently testable.
- Params struct if more than 5 related parameters.
- Unit tests co-located (`#[cfg(test)] mod tests {}`).
- All types implement `Debug`.
- Use `#[expect(lint, reason = "...")]` not `#[allow]`.
- Clippy/build use `--all-features`; the `cargo make check` target already does this.
- Commit after each verified task with a brief message.
- Never hardcode secrets; no env changes needed for this plan.
- Never use raw `Runtime::new()`; use `actions::runtime::with_client` wrapper if async needed (none of the tasks here need async).

## File Map

### Created
- `crates/temper-cli/src/output/table.rs`
- `crates/temper-cli/src/output/columns.rs`
- `crates/temper-cli/src/commands/config.rs`
- `crates/temper-cli/src/actions/config.rs`

### Modified
- `crates/temper-cli/src/format.rs`
- `crates/temper-cli/src/output/mod.rs`
- `crates/temper-cli/src/commands/resource.rs`
- `crates/temper-cli/src/commands/init.rs`
- `crates/temper-cli/src/commands/task.rs`
- `crates/temper-cli/src/commands/goal.rs`
- `crates/temper-cli/src/commands/session.rs`
- `crates/temper-cli/src/commands/mod.rs` (register `config` module)
- `crates/temper-cli/src/actions/mod.rs` (register `config` action module)
- `crates/temper-cli/src/cli.rs`
- `crates/temper-cli/src/main.rs`
- `crates/temper-cli/src/config.rs` (drop `skill_framework` field)
- `crates/temper-cli/Cargo.toml` (add `dialoguer`)
- `crates/temper-core/src/types/config.rs` (validator derives, AuthProvider refactor, default vault path, tracing warning)
- `crates/temper-core/src/schema.rs` (add `display_fields`)
- `crates/temper-core/Cargo.toml` (add `validator`, `tracing`)
- `crates/temper-client/src/config.rs` (Vec-based provider lookup, improved missing-provider error)
- `crates/temper-client/src/config.rs` tests — update TOML fixtures to `[[auth.providers]]` format

---

## Task 1: Expand `OutputFormat` with Pretty / NoTty / Json and TTY auto-detect

**Files:** `crates/temper-cli/src/format.rs`

- [ ] **Step 1: Write failing tests for the new `OutputFormat` API**

Replace the existing test module at the bottom of `crates/temper-cli/src/format.rs` (there is currently none) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pretty_lowercase() {
        assert_eq!(OutputFormat::parse("pretty"), OutputFormat::Pretty);
    }

    #[test]
    fn parse_no_tty_with_dash() {
        assert_eq!(OutputFormat::parse("no-tty"), OutputFormat::NoTty);
    }

    #[test]
    fn parse_json_lowercase() {
        assert_eq!(OutputFormat::parse("json"), OutputFormat::Json);
    }

    #[test]
    fn parse_unknown_defaults_to_auto() {
        // "text" is legacy and should resolve to auto-detect (Pretty in tests
        // depends on TTY; we only check that it is one of Pretty or NoTty).
        let v = OutputFormat::parse("text");
        assert!(matches!(v, OutputFormat::Pretty | OutputFormat::NoTty));
    }

    #[test]
    fn resolve_explicit_honors_value() {
        assert_eq!(
            OutputFormat::resolve(Some("json")),
            OutputFormat::Json
        );
    }

    #[test]
    fn resolve_none_picks_tty_or_no_tty() {
        let v = OutputFormat::resolve(None);
        assert!(matches!(v, OutputFormat::Pretty | OutputFormat::NoTty));
    }
}
```

- [ ] **Step 2: Implement the new enum and parser**

Replace the entire contents of `crates/temper-cli/src/format.rs` with:

```rust
use std::io::IsTerminal;

use serde::Serialize;

/// Output format selector for CLI commands.
///
/// `Pretty` renders markdown-style pipe tables with bold headers; used when
/// stdout is a TTY and the user did not override via `--format`. `NoTty` is
/// tab-delimited with no borders and no ANSI, suited for pipes and scripts.
/// `Json` always outputs full JSON (including all frontmatter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Pretty,
    NoTty,
    Json,
}

impl OutputFormat {
    /// Parse a `--format` string. Unknown / legacy values auto-detect via TTY.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pretty" => Self::Pretty,
            "no-tty" | "notty" => Self::NoTty,
            "json" => Self::Json,
            // Legacy "text" or anything else: auto-detect
            _ => Self::auto(),
        }
    }

    /// Resolve the effective format given an optional explicit CLI value.
    ///
    /// `None` auto-detects; `Some("text")` is treated as auto-detect for
    /// backward compatibility.
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some(s) => Self::parse(s),
            None => Self::auto(),
        }
    }

    /// Pick a format based on whether stdout is a terminal.
    fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Pretty
        } else {
            Self::NoTty
        }
    }
}

/// Print a serializable value in the requested format.
pub fn output<T: Serialize + std::fmt::Display>(value: &T, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
        OutputFormat::Pretty | OutputFormat::NoTty => {
            println!("{value}");
        }
    }
}
```

- [ ] **Step 3: Verify tests pass**

```
cargo nextest run -p temper-cli format::tests
```

- [ ] **Step 4: Fix any callers that matched against `Text`**

```
cargo build -p temper-cli --all-features 2>&1 | head -60
```

Replace any `OutputFormat::Text` arms with `OutputFormat::Pretty | OutputFormat::NoTty`. Do not change behavior of any call site beyond renaming — later tasks will thread the new variants end-to-end.

- [ ] **Step 5: Commit**

```
git add crates/temper-cli/src/format.rs
git commit -m "Expand OutputFormat with Pretty/NoTty/Json and TTY auto-detect"
```

---

## Task 2: Introduce `TableRenderer` with pretty/no-tty rendering

**Files:**
- Create: `crates/temper-cli/src/output/table.rs`
- Modify: `crates/temper-cli/src/output/mod.rs`

- [ ] **Step 1: Create skeleton module and register it**

Create `crates/temper-cli/src/output/table.rs` with only the type shells (enough to compile) and add `pub mod table;` to `crates/temper-cli/src/output/mod.rs`. Export the public types via `pub use table::{Alignment, Column, TableRenderer};`.

```rust
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
}
```

- [ ] **Step 2: Write failing tests for `render_pretty`**

Add this test module at the bottom of `crates/temper-cli/src/output/table.rs`:

```rust
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
        assert!(out.contains("| writing |"), "writing should be padded to 7: {out}");
        assert!(out.contains("| second |"), "second should be padded to 6: {out}");
    }

    #[test]
    fn pretty_right_aligns_numeric_column() {
        let out = sample().render_pretty();
        assert!(out.contains("|   1 |"), "seq '1' should right-align in width 3: {out}");
        assert!(out.contains("|  12 |"), "seq '12' should right-align in width 3: {out}");
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
```

- [ ] **Step 3: Implement `render_pretty` and `render_no_tty`**

Add these methods inside `impl TableRenderer` in `crates/temper-cli/src/output/table.rs`:

```rust
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
```

- [ ] **Step 4: Verify tests pass**

```
cargo nextest run -p temper-cli output::table::tests
```

- [ ] **Step 5: Commit**

```
git add crates/temper-cli/src/output/table.rs crates/temper-cli/src/output/mod.rs
git commit -m "Add TableRenderer with pretty and no-tty formats"
```

---

## Task 3: Add `display_fields()` helper to `temper-core::schema`

**Files:** `crates/temper-core/src/schema.rs`

- [ ] **Step 1: Write failing tests for `display_fields`**

Add to the existing `#[cfg(test)] mod tests` block in `crates/temper-core/src/schema.rs`:

```rust
    // -------------------------------------------------------------------------
    // display_fields tests
    // -------------------------------------------------------------------------

    #[test]
    fn display_fields_task_includes_extra_columns() {
        let fields = display_fields("task").expect("task display fields");
        let names: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        // Universal first
        assert_eq!(names[0], "temper-context");
        assert_eq!(names[1], "temper-type");
        assert_eq!(names[2], "slug");
        assert_eq!(names[3], "temper-updated");
        // Task extras
        assert!(names.contains(&"temper-stage"));
        assert!(names.contains(&"temper-mode"));
        assert!(names.contains(&"temper-effort"));
        assert!(names.contains(&"temper-goal"));
    }

    #[test]
    fn display_fields_goal_has_status_and_seq() {
        let fields = display_fields("goal").expect("goal display fields");
        assert!(fields.contains(&"temper-status".to_string()));
        assert!(fields.contains(&"temper-seq".to_string()));
    }

    #[test]
    fn display_fields_session_universal_only() {
        let fields = display_fields("session").expect("session display fields");
        assert_eq!(
            fields,
            vec![
                "temper-context".to_string(),
                "temper-type".to_string(),
                "slug".to_string(),
                "temper-updated".to_string(),
            ]
        );
    }

    #[test]
    fn display_fields_unknown_type_errors() {
        assert!(display_fields("widget").is_err());
    }
```

- [ ] **Step 2: Implement `display_fields`**

Add this function immediately after `updatable_fields` in `crates/temper-core/src/schema.rs`:

```rust
/// Field names to display in table output for a doctype, in order.
///
/// Unlike `updatable_fields`, this is a curated set used only for
/// human-readable table rendering in the CLI. JSON output is unaffected and
/// still contains the full frontmatter.
///
/// Universal columns come first (context, type, slug, updated), followed by
/// per-type extras. This is the single point of curation — adding a new
/// doctype means extending the match here.
pub fn display_fields(doc_type: &str) -> Result<Vec<String>> {
    const UNIVERSAL: &[&str] = &["temper-context", "temper-type", "slug", "temper-updated"];

    let extras: &[&str] = match doc_type {
        "task" => &["temper-stage", "temper-mode", "temper-effort", "temper-goal"],
        "goal" => &["temper-status", "temper-seq"],
        "session" | "research" | "concept" | "decision" => &[],
        other => {
            return Err(TemperError::Config(format!(
                "unknown doctype '{other}' for display_fields"
            )));
        }
    };

    Ok(UNIVERSAL
        .iter()
        .chain(extras.iter())
        .map(|s| s.to_string())
        .collect())
}
```

- [ ] **Step 3: Verify tests pass**

```
cargo nextest run -p temper-core schema::tests::display_fields
```

- [ ] **Step 4: Commit**

```
git add crates/temper-core/src/schema.rs
git commit -m "Add display_fields helper for schema-driven column curation"
```

---

## Task 4: `ColumnRegistry` module — `display_columns()` and `extract_row()`

**Files:**
- Create: `crates/temper-cli/src/output/columns.rs`
- Modify: `crates/temper-cli/src/output/mod.rs`

- [ ] **Step 1: Create skeleton and register module**

Add `pub mod columns;` to `crates/temper-cli/src/output/mod.rs` with `pub use columns::{display_columns, extract_row};`. Create `crates/temper-cli/src/output/columns.rs` with imports and function stubs returning `Vec::new()`.

```rust
//! Hardcoded display column registry for CLI table output.
//!
//! Columns are curated per doc type — which to display, in what order, with
//! what width and alignment. The set of fields that *exist* comes from the
//! schema; the selection/ordering is code-owned for layout stability.

use serde_json::Value;

use super::table::{Alignment, Column};

/// Ordered list of columns to render for `doc_type` in table formats.
pub fn display_columns(doc_type: &str) -> Vec<Column> {
    // Stub — implementation in step 3.
    let _ = doc_type;
    Vec::new()
}

/// Extract stringified cells from frontmatter for the given columns.
///
/// Missing fields render as empty strings. `temper-updated` is formatted as
/// YYYY-MM-DD (date-only) from an RFC3339 timestamp.
pub fn extract_row(frontmatter: &Value, columns: &[Column]) -> Vec<String> {
    let _ = (frontmatter, columns);
    Vec::new()
}
```

- [ ] **Step 2: Write failing tests**

Add at the bottom of `crates/temper-cli/src/output/columns.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn task_columns_include_universal_and_extras() {
        let cols = display_columns("task");
        let headers: Vec<&str> = cols.iter().map(|c| c.header.as_str()).collect();
        assert_eq!(
            headers,
            vec!["Context", "Type", "Slug", "Updated", "Stage", "Mode", "Effort", "Goal"]
        );
    }

    #[test]
    fn goal_columns_have_status_and_seq() {
        let cols = display_columns("goal");
        let headers: Vec<&str> = cols.iter().map(|c| c.header.as_str()).collect();
        assert_eq!(
            headers,
            vec!["Context", "Type", "Slug", "Updated", "Status", "Seq"]
        );
    }

    #[test]
    fn session_columns_universal_only() {
        let cols = display_columns("session");
        assert_eq!(cols.len(), 4);
    }

    #[test]
    fn unknown_type_returns_empty() {
        let cols = display_columns("widget");
        assert!(cols.is_empty());
    }

    #[test]
    fn extract_row_populates_known_fields() {
        let cols = display_columns("task");
        let fm = json!({
            "temper-context": "temper",
            "temper-type": "task",
            "slug": "2026-04-07-thing",
            "temper-updated": "2026-04-07T12:34:56Z",
            "temper-stage": "in-progress",
            "temper-mode": "build",
            "temper-effort": "medium",
            "temper-goal": "core",
        });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[0], "temper");
        assert_eq!(row[1], "task");
        assert_eq!(row[2], "2026-04-07-thing");
        assert_eq!(row[3], "2026-04-07", "RFC3339 should be truncated to YYYY-MM-DD");
        assert_eq!(row[4], "in-progress");
    }

    #[test]
    fn extract_row_missing_fields_render_empty() {
        let cols = display_columns("task");
        let fm = json!({ "temper-context": "temper", "slug": "x" });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[0], "temper");
        assert_eq!(row[1], "");
        assert_eq!(row[2], "x");
        assert_eq!(row[3], "");
    }

    #[test]
    fn extract_row_non_rfc3339_updated_left_as_is() {
        let cols = display_columns("task");
        let fm = json!({ "temper-updated": "unknown" });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[3], "unknown");
    }

    #[test]
    fn seq_numeric_column_is_right_aligned() {
        let cols = display_columns("goal");
        let seq_col = cols.iter().find(|c| c.header == "Seq").unwrap();
        assert_eq!(seq_col.alignment, Alignment::Right);
    }
}
```

- [ ] **Step 3: Implement `display_columns` and `extract_row`**

Replace the function bodies in `crates/temper-cli/src/output/columns.rs`:

```rust
pub fn display_columns(doc_type: &str) -> Vec<Column> {
    let mut cols = vec![
        Column::new("Context", 16, Alignment::Left),
        Column::new("Type", 10, Alignment::Left),
        Column::new("Slug", 40, Alignment::Left),
        Column::new("Updated", 12, Alignment::Left),
    ];
    match doc_type {
        "task" => {
            cols.push(Column::new("Stage", 12, Alignment::Left));
            cols.push(Column::new("Mode", 6, Alignment::Left));
            cols.push(Column::new("Effort", 7, Alignment::Left));
            cols.push(Column::new("Goal", 16, Alignment::Left));
        }
        "goal" => {
            cols.push(Column::new("Status", 10, Alignment::Left));
            cols.push(Column::new("Seq", 4, Alignment::Right));
        }
        "session" | "research" | "concept" | "decision" => {}
        _ => return Vec::new(),
    }
    cols
}

/// Map a column header back to the frontmatter key it reads from.
fn field_key_for(header: &str) -> &'static str {
    match header {
        "Context" => "temper-context",
        "Type" => "temper-type",
        "Slug" => "slug",
        "Updated" => "temper-updated",
        "Stage" => "temper-stage",
        "Mode" => "temper-mode",
        "Effort" => "temper-effort",
        "Goal" => "temper-goal",
        "Status" => "temper-status",
        "Seq" => "temper-seq",
        _ => "",
    }
}

pub fn extract_row(frontmatter: &Value, columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .map(|col| {
            let key = field_key_for(&col.header);
            let raw = frontmatter.get(key);
            let text = match raw {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            if col.header == "Updated" {
                date_only(&text)
            } else {
                text
            }
        })
        .collect()
}

/// Truncate an RFC3339 timestamp to YYYY-MM-DD. If the string doesn't look
/// like a date, return it unchanged.
fn date_only(s: &str) -> String {
    if s.len() >= 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s[..4].chars().all(|c| c.is_ascii_digit())
        && s[5..7].chars().all(|c| c.is_ascii_digit())
        && s[8..10].chars().all(|c| c.is_ascii_digit())
    {
        return s[..10].to_string();
    }
    s.to_string()
}
```

- [ ] **Step 4: Verify tests pass**

```
cargo nextest run -p temper-cli output::columns::tests
```

- [ ] **Step 5: Commit**

```
git add crates/temper-cli/src/output/columns.rs crates/temper-cli/src/output/mod.rs
git commit -m "Add ColumnRegistry with display_columns and extract_row"
```

---

## Task 5: Unified `resource::list()` pipeline — scan/parse/filter/sort/render

**Files:** `crates/temper-cli/src/commands/resource.rs`

- [ ] **Step 1: Write failing integration tests for the unified pipeline**

Add a new `#[cfg(test)] mod list_pipeline_tests { ... }` block at the bottom of `crates/temper-cli/src/commands/resource.rs` that builds a temp vault and asserts output shape for each format. Use `tempfile::TempDir`:

```rust
#[cfg(test)]
mod list_pipeline_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_config(tmp: &TempDir) -> Config {
        let vault_root = tmp.path().to_path_buf();
        fs::create_dir_all(vault_root.join(".temper")).unwrap();
        Config {
            state_dir: vault_root.join(".temper"),
            vault_root,
            contexts: vec!["temper".into(), "default".into()],
            skill_output: PathBuf::from("/tmp/skill"),
        }
    }

    fn write_resource(config: &Config, ctx: &str, doc_type: &str, slug: &str, updated: &str, extras: &str) {
        let dir = config.doc_type_dir(ctx, doc_type);
        fs::create_dir_all(&dir).unwrap();
        let content = format!(
            "---\ntemper-id: \"id-{slug}\"\ntemper-type: {doc_type}\ntemper-context: {ctx}\nslug: {slug}\ntitle: \"Title {slug}\"\ntemper-updated: \"{updated}\"\n{extras}---\n\nbody\n"
        );
        fs::write(dir.join(format!("{slug}.md")), content).unwrap();
    }

    #[test]
    fn scan_rows_sorts_descending_by_updated() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(&config, "temper", "task", "a", "2026-04-01T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n");
        write_resource(&config, "temper", "task", "b", "2026-04-07T00:00:00Z",
            "temper-stage: done\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n");

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let mut sorted = rows;
        sort_rows(&mut sorted);
        assert_eq!(sorted[0].slug_for_tests(), "b");
        assert_eq!(sorted[1].slug_for_tests(), "a");
    }

    #[test]
    fn filter_rows_respects_stage() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(&config, "temper", "task", "x", "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n");
        write_resource(&config, "temper", "task", "y", "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n");

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let filtered = filter_rows(rows, ListFilters {
            stage: Some("in-progress"),
            goal: None,
            status: None,
        });
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "y");
    }

    #[test]
    fn truncate_to_limit() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        for i in 0..5 {
            write_resource(&config, "temper", "session", &format!("s{i}"),
                &format!("2026-04-0{}T00:00:00Z", i + 1), "");
        }
        let mut rows = scan_rows(&config, "session", Some("temper")).unwrap();
        sort_rows(&mut rows);
        rows.truncate(2);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn render_no_tty_emits_tab_header_and_rows() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(&config, "temper", "task", "only", "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n");

        let out = render_list("task", &config, Some("temper"), None, None, None, None,
            OutputFormat::NoTty).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "Context\tType\tSlug\tUpdated\tStage\tMode\tEffort\tGoal");
        assert!(lines[1].starts_with("temper\ttask\tonly\t2026-04-07"));
    }

    #[test]
    fn render_json_emits_full_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(&config, "temper", "research", "note", "2026-04-07T00:00:00Z", "");
        let out = render_list("research", &config, Some("temper"), None, None, None, None,
            OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["slug"], "note");
        assert_eq!(arr[0]["title"], "Title note");
    }
}
```

Note: This test block references `ResourceRow::slug_for_tests()` for ergonomic assertions — add `#[cfg(test)] pub fn slug_for_tests(&self) -> String { ... }` to the `ResourceRow` struct in the next step.

- [ ] **Step 2: Implement `ResourceRow`, `ListFilters`, `scan_rows`, `sort_rows`, `filter_rows`, and `render_list`**

Delete `list_simple_resources`, `collect_simple_resources`, `parse_simple_resource`, `SimpleResourceEntry` from `resource.rs`. Replace the current `list()` function and add the supporting helpers. Full replacement of the `list` path and helpers:

```rust
use crate::format::OutputFormat;
use crate::output::table::TableRenderer;
use crate::output::{self, columns as col_registry};

/// A single resource row for the unified list pipeline.
#[derive(Debug, Clone)]
pub struct ResourceRow {
    /// Full frontmatter (used for JSON output and row extraction).
    pub frontmatter: serde_json::Value,
    /// Vault-relative path to the markdown file.
    pub path: String,
}

impl ResourceRow {
    fn updated_at(&self) -> &str {
        self.frontmatter
            .get("temper-updated")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    #[cfg(test)]
    pub fn slug_for_tests(&self) -> String {
        self.frontmatter
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
}

/// Filters applied after scanning.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilters<'a> {
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Scan disk for all resources of `doc_type`, optionally restricted to one context.
pub fn scan_rows(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
) -> Result<Vec<ResourceRow>> {
    let contexts_to_scan: Vec<String> = match context {
        Some(c) => vec![c.to_string()],
        None => config.contexts.clone(),
    };

    let mut rows = Vec::new();
    for ctx in &contexts_to_scan {
        let dir = config.doc_type_dir(ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            if let Some(row) = parse_row(&path, &config.vault_root)? {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn parse_row(path: &std::path::Path, vault_root: &std::path::Path) -> Result<Option<ResourceRow>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Vault(e.to_string()))?;
    let fm = match vault::parse_frontmatter(&content) {
        Some(v) => v,
        None => return Ok(None),
    };
    let relative = path.strip_prefix(vault_root).unwrap_or(path);
    Ok(Some(ResourceRow {
        frontmatter: fm,
        path: relative.to_string_lossy().to_string(),
    }))
}

/// Sort rows by `temper-updated` descending (most recent first).
pub fn sort_rows(rows: &mut [ResourceRow]) {
    rows.sort_by(|a, b| b.updated_at().cmp(a.updated_at()));
}

/// Apply stage/goal/status filters, dropping rows that don't match.
pub fn filter_rows(rows: Vec<ResourceRow>, filters: ListFilters<'_>) -> Vec<ResourceRow> {
    rows.into_iter()
        .filter(|row| match_filters(row, &filters))
        .collect()
}

fn match_filters(row: &ResourceRow, filters: &ListFilters<'_>) -> bool {
    if let Some(stage) = filters.stage {
        if row.frontmatter.get("temper-stage").and_then(|v| v.as_str()) != Some(stage) {
            return false;
        }
    }
    if let Some(goal) = filters.goal {
        if row.frontmatter.get("temper-goal").and_then(|v| v.as_str()) != Some(goal) {
            return false;
        }
    }
    if let Some(status) = filters.status {
        if row.frontmatter.get("temper-status").and_then(|v| v.as_str()) != Some(status) {
            return false;
        }
    }
    true
}

/// Render a list to a String (used by tests and by `list` on stdout).
#[expect(clippy::too_many_arguments, reason = "thin passthrough for list command")]
pub fn render_list(
    doc_type: &str,
    config: &Config,
    context: Option<&str>,
    limit: Option<usize>,
    stage: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
    format: OutputFormat,
) -> Result<String> {
    validate_doc_type(doc_type)?;
    let mut rows = scan_rows(config, doc_type, context)?;
    sort_rows(&mut rows);
    rows = filter_rows(rows, ListFilters { stage, goal, status });
    rows.truncate(limit.unwrap_or(20));

    match format {
        OutputFormat::Json => {
            let frontmatters: Vec<&serde_json::Value> =
                rows.iter().map(|r| &r.frontmatter).collect();
            Ok(serde_json::to_string_pretty(&frontmatters).unwrap_or_default())
        }
        OutputFormat::Pretty | OutputFormat::NoTty => {
            let columns = col_registry::display_columns(doc_type);
            let mut renderer = TableRenderer::new(columns.clone());
            for row in &rows {
                renderer.push_row(col_registry::extract_row(&row.frontmatter, &columns));
            }
            if format == OutputFormat::Pretty {
                Ok(renderer.render_pretty())
            } else {
                Ok(renderer.render_no_tty())
            }
        }
    }
}

/// List resources of a given type (unified pipeline for all doc types).
#[expect(clippy::too_many_arguments, reason = "pipeline fan-out from CLI")]
pub fn list(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
    limit: Option<usize>,
    stage: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
    format: &str,
) -> Result<()> {
    if let Some(s) = stage {
        vault::validate_stage(s)?;
    }
    let format = OutputFormat::parse(format);
    let body = render_list(doc_type, config, context, limit, stage, goal, status, format)?;
    if body.is_empty() {
        output::hint(format!("No {doc_type} resources found."));
        return Ok(());
    }
    // anstream handles TTY / no-TTY ANSI stripping based on the real stdout.
    output::plain(body.trim_end());
    Ok(())
}
```

Also add `use crate::vault;` if not already present.

- [ ] **Step 3: Verify tests pass**

```
cargo nextest run -p temper-cli list_pipeline_tests
```

- [ ] **Step 4: Commit**

```
git add crates/temper-cli/src/commands/resource.rs
git commit -m "Unify resource::list pipeline with ResourceRow and TableRenderer"
```

---

## Task 6: Remove dead per-type list formatters

**Files:**
- Modify: `crates/temper-cli/src/commands/task.rs`
- Modify: `crates/temper-cli/src/commands/goal.rs`
- Modify: `crates/temper-cli/src/commands/session.rs`

- [ ] **Step 1: Delete the dead list functions**

Remove:
- `pub fn list(...)` from `task.rs` (the goal-grouped version)
- `pub fn list(...)` from `goal.rs` (the roadmap view)
- `pub fn list(...)`, `collect_sessions`, `add_session_entry`, `parse_date_from_file`, `extract_date_from_stem` from `session.rs` — only the versions used exclusively by the list path. Keep `show()` helpers that still consume `parse_date_from_file`/`extract_date_from_stem` for `show` unchanged; if they overlap, keep the helpers.

Also update `session.rs` tests — delete `list_returns_sessions_sorted_by_date_desc` and `list_empty_context_shows_hint`. Delete the unused `SessionEntry` struct fields if the struct is no longer referenced anywhere else.

- [ ] **Step 2: Verify build succeeds**

```
cargo build -p temper-cli --all-features 2>&1 | head -80
```

Fix any compile errors (unused imports especially).

- [ ] **Step 3: Run all unit tests**

```
cargo make test
```

- [ ] **Step 4: Commit**

```
git add crates/temper-cli/src/commands/task.rs crates/temper-cli/src/commands/goal.rs crates/temper-cli/src/commands/session.rs
git commit -m "Remove per-type list formatters superseded by unified pipeline"
```

---

## Task 7: Refactor AuthProvider to Vec, add `validator` derives, add `tracing` warnings

**Files:**
- Modify: `crates/temper-core/Cargo.toml`
- Modify: `crates/temper-core/src/types/config.rs`

This task covers Decisions 2, 4, and the validator portion of 5. It is larger than most tasks because the refactors are tightly coupled — validator nested traversal is the whole reason we're moving off HashMap, and the tracing warning hangs off the same `load_config_from` path.

- [ ] **Step 1: Add dependencies**

Add to `[dependencies]` in `crates/temper-core/Cargo.toml`:

```toml
validator = { version = "0.20", features = ["derive"] }
tracing = "0.1"
```

- [ ] **Step 2: Write failing tests for the new AuthProvider shape + validator rules**

Replace the existing `test_temper_config_toml_roundtrip` test in `crates/temper-core/src/types/config.rs` and add new tests. Put all of this inside the existing `#[cfg(test)] mod tests { ... }` block:

```rust
    use validator::Validate;

    // --- new auth provider shape ---

    #[test]
    fn auth_providers_parse_as_array_of_tables() {
        let toml_str = r#"
[vault]
path = "~/projects/kb-vault"

[sync.subscriptions]
contexts = ["temper"]

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

[cloud]
api_url = "https://temperkb.io"
"#;
        let cfg: TemperConfig = toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.vault.path, "~/projects/kb-vault");
        assert_eq!(cfg.auth.provider, "auth0");
        assert_eq!(cfg.auth.providers.len(), 1);
        assert_eq!(cfg.auth.providers[0].name, "auth0");
        assert_eq!(
            cfg.auth.providers[0].authorize_url,
            "https://temperkb.us.auth0.com/authorize"
        );
    }

    #[test]
    fn auth_providers_lookup_by_name() {
        let cfg = TemperConfig::default();
        let active = cfg
            .auth
            .providers
            .iter()
            .find(|p| p.name == cfg.auth.provider);
        assert!(active.is_some(), "default config should have its active provider");
        assert_eq!(active.unwrap().name, "auth0");
    }

    #[test]
    fn default_vault_path_is_documents_temper_vault() {
        let cfg = TemperConfig::default();
        assert_eq!(cfg.vault.path, "~/Documents/temper-vault");
    }

    // --- validator rules ---

    #[test]
    fn validate_accepts_default_config() {
        let cfg = TemperConfig::default();
        cfg.validate().expect("default config should validate");
    }

    #[test]
    fn validate_rejects_empty_vault_path() {
        let mut cfg = TemperConfig::default();
        cfg.vault.path = String::new();
        let err = cfg.validate().unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("vault") || s.contains("path"), "got: {s}");
    }

    #[test]
    fn validate_rejects_malformed_api_url() {
        let mut cfg = TemperConfig::default();
        cfg.cloud.api_url = "not a url".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_malformed_authorize_url_in_provider_vec() {
        let mut cfg = TemperConfig::default();
        cfg.auth.providers[0].authorize_url = "nope".to_string();
        let err = cfg.validate().unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("authorize_url") || s.contains("provider"),
            "got: {s}"
        );
    }

    #[test]
    fn validate_rejects_empty_provider_client_id() {
        let mut cfg = TemperConfig::default();
        cfg.auth.providers[0].client_id = String::new();
        assert!(cfg.validate().is_err());
    }
```

Delete the old `test_temper_config_toml_roundtrip` test (the one using `[auth.providers.auth0]` dotted-form) — the replacement above covers it with the new format.

- [ ] **Step 3: Refactor types — `AuthProvider` replaces `AuthProviderConfig`, `providers` becomes `Vec<AuthProvider>`**

In `crates/temper-core/src/types/config.rs`:

1. Delete `use std::collections::HashMap;` at the top of the file (no longer needed once the refactor lands).
2. Add `use validator::Validate;` near the other imports.
3. Delete `AuthProviderConfig` entirely and replace with:

```rust
/// A single auth provider entry. Stored in `[[auth.providers]]` arrays in TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AuthProvider {
    /// Provider name — referenced by `auth.provider` to pick the active entry.
    #[validate(length(min = 1, message = "provider name cannot be empty"))]
    pub name: String,
    #[validate(url(message = "authorize_url must be a valid URL"))]
    pub authorize_url: String,
    #[validate(url(message = "token_url must be a valid URL"))]
    pub token_url: String,
    #[validate(length(min = 1, message = "client_id cannot be empty"))]
    pub client_id: String,
    #[validate(url(message = "audience must be a valid URL"))]
    pub audience: String,
    #[serde(default = "default_callback_url")]
    pub callback_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}
```

4. Replace `AuthConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AuthConfig {
    #[serde(default = "default_auth_provider")]
    pub provider: String,
    #[serde(default)]
    #[validate(nested)]
    pub providers: Vec<AuthProvider>,
}
```

5. Replace the `Default` impl for `AuthConfig`:

```rust
impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: default_auth_provider(),
            providers: vec![AuthProvider {
                name: "auth0".to_string(),
                authorize_url: "https://temperkb.us.auth0.com/authorize".to_string(),
                token_url: "https://temperkb.us.auth0.com/oauth/token".to_string(),
                client_id: "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF".to_string(),
                audience: "https://temperkb.io/api".to_string(),
                callback_url: default_callback_url(),
                scopes: vec![
                    "openid".to_string(),
                    "profile".to_string(),
                    "email".to_string(),
                    "offline_access".to_string(),
                ],
            }],
        }
    }
}
```

- [ ] **Step 4: Add `Validate` derives to the remaining config types**

In the same file, add `Validate` to `TemperConfig`, `CloudVaultConfig`, `SkillConfig`, and `CloudSection`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct TemperConfig {
    #[validate(nested)]
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: UnifiedSyncConfig,
    #[serde(default)]
    #[validate(nested)]
    pub skill: SkillConfig,
    #[serde(default)]
    #[validate(nested)]
    pub auth: AuthConfig,
    #[serde(default)]
    #[validate(nested)]
    pub cloud: CloudSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CloudVaultConfig {
    #[validate(length(min = 1, message = "vault path cannot be empty"))]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct SkillConfig {
    #[serde(default = "default_skill_output")]
    #[validate(length(min = 1, message = "skill output path cannot be empty"))]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CloudSection {
    #[serde(default = "default_api_url")]
    #[validate(url(message = "api_url must be a valid URL"))]
    pub api_url: String,
}
```

- [ ] **Step 5: Update the `TemperConfig::default()` vault path**

Change the `Default` impl:

```rust
impl Default for TemperConfig {
    fn default() -> Self {
        Self {
            vault: CloudVaultConfig {
                path: "~/Documents/temper-vault".to_string(),
            },
            sync: Default::default(),
            cli: Default::default(),
            skill: Default::default(),
            auth: Default::default(),
            cloud: Default::default(),
        }
    }
}
```

Note: the `cli: Default::default()` line is removed in Task 8 — leave it here for now so this task compiles in isolation.

- [ ] **Step 6: Emit a tracing warning when `load_config_from` loads an invalid config**

Edit `load_config_from` in the same file to run validation after parsing and emit a warning (without blocking startup):

```rust
/// Load config from a specific path (useful for tests).
pub fn load_config_from(path: &std::path::Path) -> Result<TemperConfig, String> {
    if !path.exists() {
        return Ok(TemperConfig::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let cfg: TemperConfig = toml::from_str(&content)
        .map_err(|e| format!("config parse error in {}: {}", path.display(), e))?;
    if let Err(e) = cfg.validate() {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "config at {} has validation issues — run `temper config edit` to fix",
            path.display()
        );
    }
    Ok(cfg)
}
```

Add `use validator::Validate;` at the top of the file if it's not already present for the impl paths.

- [ ] **Step 7: Verify tests pass**

```
cargo nextest run -p temper-core
cargo clippy -p temper-core --all-features -- -D warnings
```

- [ ] **Step 8: Commit**

```
git add crates/temper-core/Cargo.toml crates/temper-core/src/types/config.rs
git commit -m "Refactor AuthProvider to Vec, add validator derives, warn on invalid config load"
```

Note: temper-client will stop compiling after this commit because it still references `AuthProviderConfig` and `HashMap`. Task 7b immediately fixes it — do NOT land Task 7 without Task 7b.

---

## Task 7b: Update `temper-client` for the new `AuthProvider` Vec shape

**Files:**
- Modify: `crates/temper-client/src/config.rs`

This task restores `temper-client` to compile after the Task 7 refactor. It also implements the improved missing-provider error message (Decision 3).

- [ ] **Step 1: Read `crates/temper-client/src/config.rs` end-to-end**

Understand the current `build_oauth_config` function (~line 37), its tests, and the TOML fixtures embedded in tests (they currently use `[auth.providers.auth0]` dotted form).

- [ ] **Step 2: Write failing tests for the new lookup + improved error**

Replace the existing test fixtures (which use the old TOML format) with new ones using `[[auth.providers]]`. Add an error-message assertion test:

```rust
    #[test]
    fn oauth_config_missing_provider_returns_helpful_error() {
        use temper_core::types::config::{AuthConfig, CloudSection, CloudVaultConfig, TemperConfig, UnifiedSyncConfig, SkillConfig};
        let cfg = TemperConfig {
            vault: CloudVaultConfig { path: "~/vault".into() },
            sync: UnifiedSyncConfig::default(),
            skill: SkillConfig::default(),
            auth: AuthConfig {
                provider: "none".to_string(),
                providers: Vec::new(),
            },
            cloud: CloudSection::default(),
        };
        let err = build_oauth_config(&cfg).expect_err("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("cloud sync is disabled") || msg.contains("temper config edit"),
            "error should guide the user: {msg}"
        );
    }
```

Also update any existing TOML-fixture tests in this file from the dotted form (`[auth.providers.auth0]`) to the array form (`[[auth.providers]]` with `name = "auth0"`). Read the existing tests at lines ~120-280 to find all fixtures that need updating.

- [ ] **Step 3: Update `build_oauth_config` to use Vec lookup and emit a helpful error**

Replace the body of `build_oauth_config`:

```rust
pub fn build_oauth_config(config: &TemperConfig) -> Result<OAuthConfig, String> {
    let provider: &AuthProvider = config
        .auth
        .providers
        .iter()
        .find(|p| p.name == config.auth.provider)
        .ok_or_else(|| {
            if config.auth.provider == "none" || config.auth.providers.is_empty() {
                "cloud sync is disabled for this vault — run `temper config edit` and \
                 set `auth.provider` to a configured provider, or re-run `temper init` \
                 and pick an auth provider"
                    .to_string()
            } else {
                format!(
                    "auth provider '{}' not found in [[auth.providers]] — run `temper config edit` to fix",
                    config.auth.provider
                )
            }
        })?;
    Ok(OAuthConfig {
        authorize_url: provider.authorize_url.clone(),
        token_url: provider.token_url.clone(),
        client_id: provider.client_id.clone(),
        audience: Some(provider.audience.clone()),
        callback_url: provider.callback_url.clone(),
        scopes: provider.scopes.clone(),
    })
}
```

Update the import at the top of the file:

```rust
use temper_core::types::config::{AuthProvider, TemperConfig};
```

(Remove `AuthProviderConfig` from the import.)

- [ ] **Step 4: Update remaining test bodies that construct `AuthProviderConfig` directly**

Any test that hand-builds `AuthProviderConfig { ... }` must be changed to `AuthProvider { name: "...".into(), ... }`. Search the file for `AuthProviderConfig` and replace. Any test that constructs `providers: HashMap::new()` becomes `providers: Vec::new()`.

- [ ] **Step 5: Verify build and tests**

```
cargo build -p temper-client --all-features
cargo nextest run -p temper-client
```

- [ ] **Step 6: Commit**

```
git add crates/temper-client/src/config.rs
git commit -m "Adapt temper-client to AuthProvider Vec shape with helpful missing-provider error"
```

---

## Task 8: Drop `CliConfig` and `skill.framework`; tolerate stale configs

**Files:**
- Modify: `crates/temper-core/src/types/config.rs`
- Modify: `crates/temper-cli/src/config.rs`
- Modify: `crates/temper-cli/src/commands/skill.rs`
- Modify: `crates/temper-cli/src/commands/session.rs` (test fixture)

- [ ] **Step 1: Write failing test that proves legacy configs still parse**

Add to the test module in `crates/temper-core/src/types/config.rs`. Note: Task 7 already changed the default vault path; this task only removes the `[cli]` and `skill.framework` fields. The stale-config test must use the NEW `[[auth.providers]]` format (Task 7) plus the dropped `[cli]` and `skill.framework` fields:

```rust
    #[test]
    fn stale_cli_section_and_skill_framework_parse_without_error() {
        // This config contains fields we no longer define — the expectation
        // is that serde drops them silently rather than failing to parse.
        let toml_str = r#"
[vault]
path = "~/Documents/temper-vault"

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"
"#;
        let cfg: TemperConfig = toml::from_str(toml_str).expect("stale config must parse");
        assert_eq!(cfg.vault.path, "~/Documents/temper-vault");
        assert_eq!(cfg.skill.output, "~/.claude/skills/temper");
    }
```

- [ ] **Step 2: Remove `CliConfig`, `default_progress`, `skill.framework`**

In `crates/temper-core/src/types/config.rs`:

1. Delete the `CliConfig` struct, its `Default` impl, and `default_progress()`.
2. In `TemperConfig`, delete `pub cli: CliConfig,`.
3. Update the `Default` impl for `TemperConfig` — remove the `cli: Default::default(),` line (it was left in Task 7 Step 5 for that task's isolated compile to work):

```rust
impl Default for TemperConfig {
    fn default() -> Self {
        Self {
            vault: CloudVaultConfig {
                path: "~/Documents/temper-vault".to_string(),
            },
            sync: Default::default(),
            skill: Default::default(),
            auth: Default::default(),
            cloud: Default::default(),
        }
    }
}
```

4. In `SkillConfig`, delete `framework` field and `default_skill_framework()`.
5. Update the `Default` impl for `SkillConfig`:

```rust
impl Default for SkillConfig {
    fn default() -> Self {
        Self { output: default_skill_output() }
    }
}
```

6. Update existing tests in the file that reference `config.cli.progress` or `config.skill.framework` — delete those assertions.

- [ ] **Step 3: Update `temper-cli::config::Config`**

In `crates/temper-cli/src/config.rs`:

1. Remove `pub skill_framework: String,` from the `Config` struct.
2. Remove the two `skill_framework: global.skill.framework.clone()` lines in `load_from` and `load`.

- [ ] **Step 4: Update `skill.rs` to drop framework branch**

In `crates/temper-cli/src/commands/skill.rs`, replace `if config.skill_framework == "superpowers"` block with an unconditional check (the temper skill dynamically detects plugins now):

```rust
// Check superpowers plugin (best-effort)
let superpowers_path = dirs::home_dir()
    .unwrap_or_else(|| std::path::PathBuf::from("~"))
    .join(".claude/plugins/cache/claude-plugins-official/superpowers");
if superpowers_path.exists() {
    output::status_icon(true, format!("Superpowers: {}", superpowers_path.display()));
} else {
    output::status_icon(
        false,
        format!("Superpowers: NOT FOUND ({})", superpowers_path.display()),
    );
}
```

Also remove `skill_framework: "superpowers".to_string(),` from the test fixture at line ~501.

- [ ] **Step 5: Update `session.rs` test fixture**

Remove `skill_framework: "superpowers".to_string(),` from the `test_vault()` fixture around line 481.

- [ ] **Step 6: Fix `init.rs` template string**

Update the `register_default_config` format string in `crates/temper-cli/src/commands/init.rs` to drop the `[cli]` section and the `framework = "superpowers"` line. Update the corresponding test. (The init rewrite in Task 9 will replace this entirely, but we need the file to compile before then.)

- [ ] **Step 7: Verify build and tests**

```
cargo build --all-features 2>&1 | head -40
cargo make test
```

- [ ] **Step 8: Commit**

```
git add crates/temper-core/src/types/config.rs crates/temper-cli/src/config.rs crates/temper-cli/src/commands/skill.rs crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/init.rs
git commit -m "Remove CliConfig and skill.framework; tolerate stale configs"
```

---

## Task 9: Rewrite `temper init` as a `dialoguer` walkthrough

**Files:**
- Modify: `crates/temper-cli/Cargo.toml`
- Modify: `crates/temper-cli/src/commands/init.rs`

- [ ] **Step 1: Add `dialoguer` dependency**

Add to `[dependencies]` in `crates/temper-cli/Cargo.toml`:

```toml
dialoguer = "0.12"
```

- [ ] **Step 2: Write failing tests for wizard answer → config generation**

Add test cases to the existing `#[cfg(test)] mod tests` in `crates/temper-cli/src/commands/init.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_answers_generate_complete_config() {
        let answers = WizardAnswers {
            vault_path: "/tmp/my-vault".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"path = "/tmp/my-vault""#));
        assert!(toml.contains("[auth]"));
        assert!(toml.contains(r#"provider = "auth0""#));
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(toml.contains("[cloud]"));
        assert!(toml.contains(r#"api_url = "https://temperkb.io""#));
        // Must NOT contain removed fields
        assert!(!toml.contains("[cli]"), "cli section should not be written");
        assert!(!toml.contains("framework ="), "skill.framework should not be written");
    }

    #[test]
    fn auth_none_writes_provider_none_marker() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"provider = "none""#));
        // no auth0 provider entry when none chosen
        assert!(!toml.contains("[[auth.providers]]"));
    }

    #[test]
    fn auth0_writes_array_of_tables_format() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        // Must use the new array-of-tables format, NOT the old dotted-map form
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(!toml.contains("[auth.providers.auth0]"), "must not use old dotted form");
    }

    #[test]
    fn apply_answers_warns_on_existing_vault_but_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("existing");
        // Pre-create a .temper/ marker to simulate an existing vault
        std::fs::create_dir_all(vault.join(".temper")).unwrap();
        let answers = WizardAnswers {
            vault_path: vault.to_string_lossy().to_string(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        // Should succeed (no error) — the existing-vault warning is emitted via output::warning
        // and does not block. Test verifies idempotent behavior.
        apply_answers(&answers, false).expect("should warn but succeed");
        assert!(vault.join(".temper/manifest.json").exists());
    }

    #[test]
    fn extra_contexts_go_into_subscriptions() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec!["temper".into(), "writing".into()],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"contexts = ["default", "temper", "writing"]"#));
    }

    #[test]
    fn apply_answers_creates_vault_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_path = tmp.path().join("vault");
        let answers = WizardAnswers {
            vault_path: vault_path.to_string_lossy().to_string(),
            extra_contexts: vec!["writing".into()],
            auth_choice: AuthChoice::None,
        };
        apply_answers(&answers, false).expect("apply should succeed");
        assert!(vault_path.join(".temper/manifest.json").exists());
        assert!(vault_path.join(".temper/events.jsonl").exists());
        assert!(vault_path.join("default").exists());
        assert!(vault_path.join("writing").exists());
    }

    #[test]
    fn no_interactive_defaults_and_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("v");
        run_non_interactive(&vault, false).expect("non-interactive run should succeed");
        assert!(vault.join(".temper").exists());
        assert!(vault.join("default").exists());
    }
}
```

- [ ] **Step 3: Implement the wizard, types, and apply logic**

Replace the entire contents of `crates/temper-cli/src/commands/init.rs`:

```rust
//! `temper init` — guided vault + config setup.
//!
//! The wizard is split into two parts so that tests can drive the apply
//! step without touching dialoguer: `gather_answers` (interactive) and
//! `apply_answers` (pure disk work).

use std::path::{Path, PathBuf};

use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};

use crate::config::global_config_path;
use crate::error::{Result, TemperError};
use crate::output;

/// User selection for auth provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthChoice {
    Auth0,
    None,
}

/// Collected wizard answers — produced by `gather_answers` (interactive) or
/// `default_answers` (`--no-interactive`).
#[derive(Debug, Clone)]
pub struct WizardAnswers {
    pub vault_path: String,
    pub extra_contexts: Vec<String>,
    pub auth_choice: AuthChoice,
}

fn default_vault_path() -> String {
    dirs::home_dir()
        .map(|h| h.join("Documents/temper-vault").to_string_lossy().to_string())
        .unwrap_or_else(|| "./temper-vault".to_string())
}

/// CLI entry point dispatched from `main.rs`.
pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()> {
    if no_interactive {
        return run_non_interactive(path, register_global);
    }
    let initial_vault = if path.as_os_str().is_empty() {
        default_vault_path()
    } else {
        path.to_string_lossy().to_string()
    };
    let answers = gather_answers(&initial_vault)?;
    print_summary(&answers, register_global);
    let proceed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Proceed?")
        .default(true)
        .interact()
        .map_err(|e| TemperError::Vault(format!("prompt error: {e}")))?;
    if !proceed {
        output::warning("Init cancelled");
        return Ok(());
    }
    apply_answers(&answers, register_global)
}

/// Non-interactive path — uses all defaults.
pub fn run_non_interactive(path: &Path, register_global: bool) -> Result<()> {
    let answers = WizardAnswers {
        vault_path: path.to_string_lossy().to_string(),
        extra_contexts: Vec::new(),
        auth_choice: AuthChoice::Auth0,
    };
    apply_answers(&answers, register_global)
}

/// Run the interactive prompts and return collected answers.
fn gather_answers(initial_vault: &str) -> Result<WizardAnswers> {
    let theme = ColorfulTheme::default();

    let vault_path: String = Input::with_theme(&theme)
        .with_prompt("Where should your vault live?")
        .default(initial_vault.to_string())
        .interact_text()
        .map_err(|e| TemperError::Vault(format!("prompt error: {e}")))?;

    let contexts_raw: String = Input::with_theme(&theme)
        .with_prompt("Create any contexts now? (comma-separated, or Enter for just 'default')")
        .default(String::new())
        .allow_empty(true)
        .interact_text()
        .map_err(|e| TemperError::Vault(format!("prompt error: {e}")))?;

    let extra_contexts: Vec<String> = contexts_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "default")
        .collect();

    let items = vec![
        "auth0 (recommended — temperkb.io cloud sync)",
        "none (local-only, no sync)",
    ];
    let idx = Select::with_theme(&theme)
        .with_prompt("Auth provider")
        .default(0)
        .items(&items)
        .interact()
        .map_err(|e| TemperError::Vault(format!("prompt error: {e}")))?;

    let auth_choice = if idx == 0 { AuthChoice::Auth0 } else { AuthChoice::None };

    Ok(WizardAnswers {
        vault_path,
        extra_contexts,
        auth_choice,
    })
}

fn print_summary(answers: &WizardAnswers, register_global: bool) {
    output::blank();
    output::header("Ready to initialize:");
    output::label("Vault", &answers.vault_path);
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    output::label("Contexts", ctxs.join(", "));
    let auth_label = match answers.auth_choice {
        AuthChoice::Auth0 => "auth0",
        AuthChoice::None => "none",
    };
    output::label("Auth", auth_label);
    if register_global {
        output::label("Config", global_config_path().display().to_string());
    }
    output::blank();
}

/// Write vault dirs and (optionally) the global config file.
pub fn apply_answers(answers: &WizardAnswers, register_global: bool) -> Result<()> {
    let vault = PathBuf::from(&answers.vault_path);

    // Warn if a .temper/ marker already exists — per Decision 1 we do not
    // offer to reconfigure, we just point the user at `temper config edit`.
    let marker = vault.join(".temper");
    if marker.exists() {
        output::warning(format!(
            "vault already exists at {}; re-running init is idempotent. \
             To change settings, run `temper config edit`.",
            vault.display()
        ));
    }

    std::fs::create_dir_all(&vault)?;

    let state_dir = vault.join(".temper");
    std::fs::create_dir_all(&state_dir)?;
    let manifest_path = state_dir.join("manifest.json");
    if !manifest_path.exists() {
        std::fs::write(&manifest_path, "{}\n")?;
    }
    let events_path = state_dir.join("events.jsonl");
    if !events_path.exists() {
        std::fs::write(&events_path, "")?;
    }

    // Create default/ and any extra contexts
    std::fs::create_dir_all(vault.join("default"))?;
    for ctx in &answers.extra_contexts {
        std::fs::create_dir_all(vault.join(ctx))?;
    }

    if register_global {
        let config_path = global_config_path();
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let toml = render_config_toml(answers);
            std::fs::write(&config_path, toml)?;
            output::dim(format!("Wrote global config to {}", config_path.display()));
        } else {
            output::dim("Global config already exists, skipping");
        }
    }

    output::success("Vault initialized successfully");
    Ok(())
}

/// Produce the TOML body for `config.toml` from the collected answers.
pub fn render_config_toml(answers: &WizardAnswers) -> String {
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    let ctx_list = ctxs
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let auth_section = match answers.auth_choice {
        AuthChoice::None => "[auth]\nprovider = \"none\"\n".to_string(),
        AuthChoice::Auth0 => r#"[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
        .to_string(),
    };

    format!(
        r#"[vault]
path = "{path}"

[sync.subscriptions]
contexts = [{ctx_list}]

[skill]
output = "~/.claude/skills/temper"

{auth_section}
[cloud]
api_url = "https://temperkb.io"
"#,
        path = answers.vault_path,
    )
}
```

Note: `auth.provider = "none"` is handled by Task 7b, which makes `temper-client::build_oauth_config` return a helpful error instead of panicking when no matching provider exists.

- [ ] **Step 4: Verify tests pass**

```
cargo nextest run -p temper-cli commands::init::tests
```

- [ ] **Step 5: Commit**

```
git add crates/temper-cli/Cargo.toml crates/temper-cli/src/commands/init.rs
git commit -m "Rewrite temper init as dialoguer walkthrough with testable apply step"
```

---

## Task 10: `temper config edit` — validated safe-write with `$EDITOR`

**Files:**
- Create: `crates/temper-cli/src/actions/config.rs`
- Create: `crates/temper-cli/src/commands/config.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Add clap subcommand**

In `crates/temper-cli/src/cli.rs`, add a new variant to `Commands`:

```rust
    /// Manage temper global config
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
```

And define:

```rust
#[derive(Subcommand)]
pub enum ConfigAction {
    /// Open config.toml in $EDITOR with validate-then-save semantics
    Edit,
}
```

- [ ] **Step 2: Write failing tests for the pure validation loop**

Create `crates/temper-cli/src/actions/config.rs`:

```rust
//! `temper config edit` actions — temp-file editor workflow with validation.

use std::path::{Path, PathBuf};

use temper_core::types::config::TemperConfig;
use validator::Validate;

use crate::error::{Result, TemperError};

/// Outcome of parsing + validating edited TOML content.
#[derive(Debug)]
pub enum ParseOutcome {
    Valid(TemperConfig),
    Invalid(String),
}

/// Parse TOML text into `TemperConfig` and run validator rules.
pub fn parse_and_validate(content: &str) -> ParseOutcome {
    let parsed: TemperConfig = match toml::from_str(content) {
        Ok(c) => c,
        Err(e) => return ParseOutcome::Invalid(format!("TOML parse error: {e}")),
    };
    if let Err(errors) = parsed.validate() {
        return ParseOutcome::Invalid(format_errors(&errors));
    }
    ParseOutcome::Valid(parsed)
}

fn format_errors(errors: &validator::ValidationErrors) -> String {
    let mut out = String::from("Configuration is invalid:\n");
    for (field, field_errors) in errors.field_errors() {
        for err in field_errors {
            let msg = err
                .message
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|| err.code.to_string());
            out.push_str(&format!("  - {field}: {msg}\n"));
        }
    }
    out
}

/// Build the temp edit-file path (sibling of the target file).
pub fn temp_edit_path(target: &Path) -> PathBuf {
    let mut file_name = target.file_name().unwrap_or_default().to_os_string();
    file_name.push(".edit");
    target.with_file_name(file_name)
}

/// Atomically replace `target` with the contents currently in `edit_path`.
///
/// Uses `std::fs::rename` which is atomic on the same filesystem.
pub fn commit_edit(edit_path: &Path, target: &Path) -> Result<()> {
    std::fs::rename(edit_path, target)
        .map_err(|e| TemperError::Config(format!("cannot commit config edit: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[vault]
path = "/tmp/v"

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://example.com/a"
token_url = "https://example.com/t"
client_id = "cid"
audience = "https://example.com/api"

[cloud]
api_url = "https://example.com"
"#;

    #[test]
    fn parse_valid_config_returns_valid() {
        match parse_and_validate(VALID) {
            ParseOutcome::Valid(_) => {}
            ParseOutcome::Invalid(msg) => panic!("expected valid, got: {msg}"),
        }
    }

    #[test]
    fn parse_invalid_toml_returns_invalid() {
        match parse_and_validate("not = toml =") {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("TOML parse error")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn parse_empty_vault_path_returns_invalid() {
        let broken = VALID.replace(r#"path = "/tmp/v""#, r#"path = """#);
        match parse_and_validate(&broken) {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("vault") || msg.contains("path")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn parse_bad_url_returns_invalid() {
        let broken = VALID.replace(r#"api_url = "https://example.com""#, r#"api_url = "not a url""#);
        match parse_and_validate(&broken) {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("api_url") || msg.contains("url")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn temp_edit_path_is_sibling_with_dot_edit() {
        let p = std::path::PathBuf::from("/a/b/config.toml");
        assert_eq!(temp_edit_path(&p), std::path::PathBuf::from("/a/b/config.toml.edit"));
    }

    #[test]
    fn commit_edit_moves_file_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("config.toml");
        let edit = tmp.path().join("config.toml.edit");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&edit, "new").unwrap();
        commit_edit(&edit, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!edit.exists());
    }
}
```

Register by adding `pub mod config;` to `crates/temper-cli/src/actions/mod.rs`.

- [ ] **Step 3: Create the command wrapper**

Create `crates/temper-cli/src/commands/config.rs`:

```rust
//! `temper config edit` command entry point.

use std::path::Path;

use dialoguer::{theme::ColorfulTheme, Select};

use crate::actions::config as action;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::types::config::{global_config_path, TemperConfig};

/// Open `$EDITOR` against a temp copy of the global config and loop until
/// the edited TOML is both structurally and semantically valid, or the user
/// chooses to discard.
pub fn edit() -> Result<()> {
    let target = global_config_path();
    ensure_config_exists(&target)?;

    let edit_path = action::temp_edit_path(&target);
    std::fs::copy(&target, &edit_path)
        .map_err(|e| TemperError::Config(format!("cannot copy for edit: {e}")))?;

    loop {
        open_in_editor(&edit_path)?;
        let content = std::fs::read_to_string(&edit_path)
            .map_err(|e| TemperError::Config(format!("cannot read edit file: {e}")))?;

        match action::parse_and_validate(&content) {
            action::ParseOutcome::Valid(_) => {
                action::commit_edit(&edit_path, &target)?;
                output::success(format!("Config saved: {}", target.display()));
                return Ok(());
            }
            action::ParseOutcome::Invalid(msg) => {
                output::error(msg);
                let choices = vec!["Re-edit", "Discard changes"];
                let idx = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("What now?")
                    .default(0)
                    .items(&choices)
                    .interact()
                    .map_err(|e| TemperError::Config(format!("prompt error: {e}")))?;
                if idx == 1 {
                    let _ = std::fs::remove_file(&edit_path);
                    output::warning("Discarded changes");
                    return Ok(());
                }
            }
        }
    }
}

fn ensure_config_exists(target: &Path) -> Result<()> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TemperError::Config(format!("cannot create config dir: {e}")))?;
    }
    let default_config = TemperConfig::default();
    let toml = toml::to_string_pretty(&default_config)
        .map_err(|e| TemperError::Config(format!("default config serialize: {e}")))?;
    std::fs::write(target, toml)
        .map_err(|e| TemperError::Config(format!("cannot write default config: {e}")))?;
    output::dim(format!("Seeded default config at {}", target.display()));
    Ok(())
}

fn open_in_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR").map_err(|_| {
        TemperError::Config("Set $EDITOR to use config edit, e.g. export EDITOR=vim".into())
    })?;
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .map_err(|e| TemperError::Config(format!("failed to launch {editor}: {e}")))?;
    if !status.success() {
        return Err(TemperError::Config(format!(
            "{editor} exited with status {status}"
        )));
    }
    Ok(())
}
```

Register by adding `pub mod config;` to `crates/temper-cli/src/commands/mod.rs`.

- [ ] **Step 4: Wire into `main.rs`**

Add a `Commands::Config { action }` arm in `crates/temper-cli/src/main.rs`:

```rust
        Commands::Config { action } => match action {
            temper_cli::cli::ConfigAction::Edit => temper_cli::commands::config::edit(),
        },
```

Also import `ConfigAction` in the `use` list at the top.

- [ ] **Step 5: Verify tests pass**

```
cargo nextest run -p temper-cli actions::config::tests
cargo build -p temper-cli --all-features
```

- [ ] **Step 6: Commit**

```
git add crates/temper-cli/src/actions/config.rs crates/temper-cli/src/commands/config.rs crates/temper-cli/src/actions/mod.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "Add temper config edit with validator-based safe-write loop"
```

---

## Task 11: Thread `--format` auto-detect through `main.rs` dispatch

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Change `--format` defaults**

In `crates/temper-cli/src/cli.rs`, change every `#[arg(long, default_value = "text")]` on a `format: String` field to drop the default and use `Option<String>`:

```rust
#[arg(long)]
format: Option<String>,
```

This applies to: `Events`, `Resource::{Create,List,Show}`, `Doctor`, `Warmup`, `Sync::*`, `Search`. Do not change command internals yet.

- [ ] **Step 2: Resolve format at dispatch in `main.rs`**

At the top of each arm that uses `format`, change:

```rust
let format = temper_cli::format::OutputFormat::resolve(format.as_deref());
// Pass as string to existing call sites (they will call parse again, which is idempotent)
let format_str = match format {
    temper_cli::format::OutputFormat::Pretty => "pretty",
    temper_cli::format::OutputFormat::NoTty => "no-tty",
    temper_cli::format::OutputFormat::Json => "json",
};
```

Then pass `format_str` to the existing `&str` parameter. This preserves the string-threading interface while making TTY detection happen exactly once at dispatch.

- [ ] **Step 3: Verify build and run smoke test**

```
cargo build -p temper-cli --all-features
cargo nextest run -p temper-cli
```

- [ ] **Step 4: Commit**

```
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "Resolve --format with TTY auto-detect at dispatch"
```

---

## Task 12: End-to-end verification

**Files:** none

- [ ] **Step 1: Clippy + fmt + machete**

```
cargo make check
```

- [ ] **Step 2: Full unit test run**

```
cargo make test
```

- [ ] **Step 3: Manual smoke test each new command**

Run against a scratch vault under `/tmp`:

```
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml ./target/debug/temper init /tmp/smoke-vault --no-interactive
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml TEMPER_VAULT=/tmp/smoke-vault ./target/debug/temper resource create --type task --title "First" --context default
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml TEMPER_VAULT=/tmp/smoke-vault ./target/debug/temper resource list --type task
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml TEMPER_VAULT=/tmp/smoke-vault ./target/debug/temper resource list --type task --format json
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml TEMPER_VAULT=/tmp/smoke-vault ./target/debug/temper resource list --type task --format no-tty
TEMPER_GLOBAL_CONFIG=/tmp/smoke-config.toml EDITOR=true ./target/debug/temper config edit
```

Expected: pretty table by default on TTY, tab-delimited via pipe or explicit `--format no-tty`, JSON array of full frontmatter via `--format json`, config edit round-trips cleanly with `EDITOR=true` (a shell built-in that exits 0 without modification).

- [ ] **Step 4: Final commit if any small fixes emerged**

```
git add -A
git commit -m "CLI output standardization and init walkthrough: verification pass"
```

---

## Notes for subagents

- Do not re-order tasks. Tests in Task 6 depend on Task 5 compiling. Task 8 deletes `skill_framework` which Task 5 never reads anyway, but fixture code in session.rs must be fixed in lockstep. Task 9's init rewrite depends on Task 8 removing the stale config fields so the generated TOML is correct.
- The `pad` helper in Task 2 counts bytes, not grapheme clusters. That's acceptable for ASCII column data (which is what our fields are) — do not pull in `unicode-width` without evidence that it's needed.
- The `anstream::stdout` writer in `output::plain` already strips ANSI on non-TTY stdout, so the Pretty renderer's bold escapes are safe to emit unconditionally when the caller selected `Pretty`.
- `TemperConfig` serialization for the default config (Task 10) requires every field to round-trip. Verify by running the test `default_answers_generate_complete_config` after implementation.
- `run_non_interactive` in init.rs creates a vault at the given path without prompting. Match the existing behavior from the current `run()` for any edge cases (e.g. don't fail if manifest.json already exists).

---

**END OF PLAN**
