# CLI Output Standardization & Init Walkthrough Design

**Date**: 2026-04-08
**Task**: 2026-04-07-temper-cli-enhancements-for-common-output-and-init-walkthrough
**Branch**: jct/cli-enhancements-and-init-walkthrough

## Overview

Standardize CLI output across all resource types using a schema-driven column registry and
unified table renderer, replace the auto-pilot `temper init` with a guided interactive
walkthrough using `dialoguer`, add `temper config edit` with safe-write validation using the
`validator` crate, and clean up dead config fields.

## 1. Output Format System

### OutputFormat Enum

Replace the current `Text`/`Json` enum in `crates/temper-cli/src/format.rs` with three variants:

```rust
pub enum OutputFormat {
    Pretty,  // Markdown-style pipe tables (default when stdout is a TTY)
    NoTty,   // Tab-delimited, no borders (default when stdout is not a TTY)
    Json,    // Full JSON with all frontmatter
}
```

**Auto-detection**: When `--format` is not explicitly passed, check
`std::io::stdout().is_terminal()` (from `std::io::IsTerminal`, stable since Rust 1.70).
TTY → `Pretty`, no TTY → `NoTty`.

**CLI flag**: `--format <pretty|no-tty|json>` overrides auto-detection.

### TableRenderer

New struct in `crates/temper-cli/src/output/table.rs`:

```rust
pub struct TableRenderer {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
}

pub struct Column {
    header: String,
    min_width: usize,
    alignment: Alignment, // Left or Right
}

pub enum Alignment {
    Left,
    Right,
}
```

Three render methods:
- `render_pretty(&self)` — pipe-delimited with `|` column separators and `---` header
  separator line. Headers are bold via `anstyle`. Example:
  ```
  | Context          | Type       | Slug                                     | Updated      | Stage       |
  |------------------|------------|------------------------------------------|--------------|-------------|
  | temper           | task       | 2026-04-07-cli-enhancements              | 2026-04-07   | in-progress |
  ```
- `render_no_tty(&self)` — tab-delimited, headers on first line, no borders, no ANSI:
  ```
  context	type	slug	updated	stage
  temper	task	2026-04-07-cli-enhancements	2026-04-07	in-progress
  ```
- JSON is handled separately via serde, not through the renderer.

The renderer uses `anstream` for color capability detection, matching the existing
`output::header()` / `output::dim()` pattern.

## 2. Schema-Driven Column Registry

New module in `crates/temper-cli/src/output/columns.rs`.

The registry maps doc types to curated display columns for human-readable formats. JSON
always outputs all frontmatter — the registry only governs table views.

### Universal Columns (all doc types, always first)

| Column    | Source Key       | Width | Alignment |
|-----------|------------------|-------|-----------|
| Context   | `temper-context`  | 16    | Left      |
| Type      | `temper-type`     | 10    | Left      |
| Slug      | `slug`           | 40    | Left      |
| Updated   | `temper-updated`  | 12    | Left      |

`Updated` displays date-only (YYYY-MM-DD) extracted from the RFC3339 timestamp.

### Per-Type Extra Columns

| Doc Type  | Extra Columns                    |
|-----------|----------------------------------|
| task      | stage, mode, effort, goal        |
| goal      | status, seq                      |
| session   | (none)                           |
| research  | (none)                           |
| concept   | (none)                           |
| decision  | (none)                           |

Title is excluded from table output — slugs are the operational handle and titles can be
arbitrarily long. Title is present in JSON output.

### API

```rust
/// Returns the ordered display columns for a doc type in table formats.
pub fn display_columns(doc_type: &str) -> Vec<Column>

/// Extract a row of string values from frontmatter for the given columns.
pub fn extract_row(frontmatter: &serde_json::Value, columns: &[Column]) -> Vec<String>
```

The column definitions reference schema field names. When a new doc type is added with a
new schema, adding its per-type columns to the registry is the only change needed.

## 3. Unified List Pipeline

### Current State

Each doc type has its own list path: `task.rs::list()`, `goal.rs::list()`,
`list_simple_resources()` in `resource.rs`. They read files, parse frontmatter, and format
output independently with inconsistent columns and styles.

### New Flow

`resource::list()` becomes the single entry point for all doc types:

1. **Scan** — walk `{context}/{doc_type}/` directories, collect all `.md` files
2. **Parse** — extract frontmatter from each file into a uniform `ResourceRow`:
   ```rust
   struct ResourceRow {
       frontmatter: serde_json::Value,  // all frontmatter (for JSON output)
       path: String,                     // vault-relative path
   }
   ```
3. **Filter** — apply `--stage`, `--status`, `--goal` filters (validated against schema enums)
4. **Sort** — by `temper-updated` descending (most recent first)
5. **Truncate** — apply `--limit` (default 20)
6. **Render** — dispatch by format:
   - `Json` → serialize `Vec<serde_json::Value>` (full frontmatter per row)
   - `Pretty` / `NoTty` → get `display_columns(doc_type)`, extract rows, pass to
     `TableRenderer`

### What Changes

- Per-type list formatting in `task::list()`, `goal::list()`, `session::list()` is removed
- Goal grouping in task list is removed — the task description explicitly says
  "line-over-line consistency is preferable to goal annotation breakers". Goal is a column.
- Sort is unified: all types sort by `temper-updated` descending
- These command files keep their `create`, `show`, `update` functions

## 4. Init Walkthrough

Replace the current auto-pilot `init.rs` with a `dialoguer`-driven interactive wizard.

### Prompt Flow

**Step 1 — Vault path**
```
Where should your vault live? [~/Documents/temper-vault]
  ? Your vault is the directory where all knowledge files are stored.
```
`dialoguer::Input` with default `~/Documents/temper-vault`. Validates: path is writable,
not inside an existing vault (check for `.temper/` ancestor).

**Step 2 — Initial contexts**
```
Create any contexts now? (comma-separated, or Enter for just 'default')
  ? Contexts are top-level scopes like project names: temper, tasker, writing
```
`dialoguer::Input`. Always creates `default/` plus whatever the user enters.

**Step 3 — Auth provider**
```
Auth provider:
  > auth0 (recommended — temperkb.io cloud sync)
    none (local-only, no sync)
  ? Auth enables cloud sync and search. You can change this later.
```
`dialoguer::Select`. If `auth0`, use existing Auth0 defaults. If `none`, set
`auth.provider = "none"` in the generated config. `AuthConfig` gains a `provider` value
of `"none"` which suppresses the auth flow in `temper-client`. This is an explicit marker,
not an omission — omitting `[auth]` would silently fall back to Auth0 defaults via serde's
`#[serde(default)]`.

**Step 4 — Confirm and create**
```
Ready to initialize:
  Vault:      ~/Documents/temper-vault
  Contexts:   default, temper, writing
  Auth:       auth0
  Skill:      ~/.claude/skills/temper
  Config:     ~/.config/temper/config.toml

Proceed? [Y/n]
```
`dialoguer::Confirm`. On yes, create vault + config. On no, restart or abort.

### Non-Interactive Mode

`--no-interactive` uses all defaults (preserves current behavior).

### Existing Vault Detection

If the target path already contains `.temper/`, warn and offer to reconfigure (re-generate
config.toml) rather than re-initializing the vault structure.

## 5. Config Edit Command

New subcommand: `temper config edit`

### Flow

1. Load current config from `global_config_path()` (`~/.config/temper/config.toml`)
2. If no config exists, generate from `TemperConfig::default()` serialized to TOML
3. Copy to temp file in same directory (`.config.toml.edit`) for same-filesystem atomic rename
4. Open with `$EDITOR` — error if `$EDITOR` is unset:
   `"Set $EDITOR to use config edit, e.g. export EDITOR=vim"`
5. On editor close:
   - Parse edited TOML into `TemperConfig` (serde structural check)
   - Run `validator` semantic checks
   - **Valid**: atomically replace config file, clean up temp file, print success
   - **Invalid**: print errors, offer `Select` prompt: "Re-edit" or "Discard"
     - Re-edit → reopen editor with the invalid file
     - Discard → clean up temp file, no changes made

### Implementation Location

- `crates/temper-cli/src/commands/config.rs` — command entry point
- `crates/temper-cli/src/actions/config.rs` — edit logic (temp file, editor, validate loop)

## 6. Validator Integration

Add `validator` crate (with `derive` feature) to `temper-core` dependencies.

### Validation Rules

```rust
#[derive(Validate)]
pub struct TemperConfig {
    #[validate(nested)]
    pub vault: CloudVaultConfig,
    #[validate(nested)]
    pub skill: SkillConfig,
    #[validate(nested)]
    pub auth: AuthConfig,
    #[validate(nested)]
    pub cloud: CloudSection,
}

#[derive(Validate)]
pub struct CloudVaultConfig {
    #[validate(length(min = 1, message = "vault path cannot be empty"))]
    pub path: String,
}

#[derive(Validate)]
pub struct SkillConfig {
    #[validate(length(min = 1, message = "skill output path cannot be empty"))]
    pub output: String,
}

#[derive(Validate)]
pub struct AuthProviderConfig {
    #[validate(url(message = "authorize_url must be a valid URL"))]
    pub authorize_url: String,
    #[validate(url(message = "token_url must be a valid URL"))]
    pub token_url: String,
    #[validate(length(min = 1, message = "client_id cannot be empty"))]
    pub client_id: String,
    #[validate(url(message = "audience must be a valid URL"))]
    pub audience: String,
}

#[derive(Validate)]
pub struct CloudSection {
    #[validate(url(message = "api_url must be a valid URL"))]
    pub api_url: String,
}
```

### Where Validation Runs

- **`config edit`** — after re-parsing edited TOML, before overwriting
- **`init`** — after building config from wizard answers, before writing
- **`load_config()`** — as a warning only (don't block startup; a partially broken config
  is better than no config)

## 7. Config Cleanup

### Removed Fields

- `[cli]` section entirely — `progress` was never implemented, `temper sync status`
  uses `--format` now
- `skill.framework` — the temper skill now dynamically detects installed skills/plugins
  and asks the user at session start

### Final Config Shape

```toml
[vault]
path = "~/Documents/temper-vault"

[sync.subscriptions]
contexts = []

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

[cloud]
api_url = "https://temperkb.io"
```

## 8. Dependencies

| Crate | Dependency | Feature | Purpose |
|-------|-----------|---------|---------|
| temper-cli | `dialoguer` | — | Interactive prompts for init wizard and config edit |
| temper-core | `validator` | `derive` | Semantic config validation |

## 9. Files Changed

### Created
- `crates/temper-cli/src/output/table.rs` — `TableRenderer` struct and render methods
- `crates/temper-cli/src/output/columns.rs` — `ColumnRegistry`, `display_columns()`, `extract_row()`
- `crates/temper-cli/src/commands/config.rs` — `temper config edit` command
- `crates/temper-cli/src/actions/config.rs` — safe-edit logic (temp file, editor, validate)

### Modified
- `crates/temper-cli/src/format.rs` — `OutputFormat` → `Pretty`/`NoTty`/`Json` with TTY auto-detect
- `crates/temper-cli/src/output/mod.rs` — re-export `table` and `columns` modules
- `crates/temper-cli/src/commands/resource.rs` — unified list pipeline using registry + renderer
- `crates/temper-cli/src/commands/init.rs` — rewritten with `dialoguer` wizard
- `crates/temper-core/src/types/config.rs` — remove `CliConfig`, remove `skill.framework`, add `Validate` derives
- `crates/temper-core/src/schema.rs` — add `display_fields()` helper for column metadata
- CLI arg parser — add `config edit` subcommand, expand `--format` accepted values

### Dead Code Removed
- Per-type list formatting in `task.rs::list()`, `goal.rs::list()`, `session.rs::list()`
  (list rendering consolidates; these files retain create/show/update)
- `CliConfig` struct and `default_progress()` in config.rs
- `skill.framework` field and `default_skill_framework()` in config.rs

## 10. Testing Strategy

- **TableRenderer**: unit tests for each render method — verify column alignment, header
  separators, tab delimiters, empty table, single row, long values
- **ColumnRegistry**: unit tests — `display_columns()` returns correct columns per type,
  `extract_row()` extracts values from sample frontmatter, missing fields produce empty strings
- **Unified list pipeline**: integration tests with temp vault — create resources, verify
  list output matches expected format for each `OutputFormat` variant
- **Init wizard**: unit tests for config generation from wizard answers, integration test
  with `--no-interactive` flag (exercisable without TTY)
- **Config edit**: unit tests for validate-then-write flow — valid config succeeds, invalid
  config returns errors with field-level messages
- **Validator rules**: unit tests for each semantic rule — empty paths, malformed URLs,
  valid configs pass
- **Config cleanup**: verify existing configs containing the now-removed `[cli]` section
  or `skill.framework` field still parse correctly. `TemperConfig` already uses
  `#[serde(default)]` on its fields; the concern here is that unknown fields in TOML are
  accepted by default. Test: load a TOML string containing `[cli]\nprogress = "bar"` and
  `skill.framework = "superpowers"` into the new `TemperConfig` — it should parse without
  error and those values should simply be discarded.
