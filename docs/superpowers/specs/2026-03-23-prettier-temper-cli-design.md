# Prettier Temper CLI — Design Spec

**Ticket:** `2026-03-23-prettier-temper-cli`
**Date:** 2026-03-23
**Status:** Approved

## Goal

Add styled terminal output to temper using `anstream` + `anstyle` (already in Cargo.toml but unused), following the same module pattern as `tasker-ctl`. All existing `println!`/`eprintln!` calls get converted to semantic output helpers in a single pass. Output degrades gracefully to plain text when piped or in non-TTY contexts.

## Output Module: `src/output/`

### `src/output/styles.rs`

Style constants using `anstyle`, matching the tasker-ctl palette for ecosystem consistency:

| Constant  | Definition                     | Usage                          |
|-----------|--------------------------------|--------------------------------|
| `SUCCESS` | Green foreground               | Success messages, healthy status |
| `ERROR`   | Red foreground                 | Errors, unhealthy status       |
| `WARNING` | Yellow foreground              | Warnings, caution messages     |
| `HEADER`  | Bold                           | Section headers                |
| `LABEL`   | Bold                           | Label names in key-value pairs |
| `DIM`     | Dimmed                         | Secondary/muted information    |
| `HINT`    | Dimmed                         | Hints and guidance text        |

Includes `clap_styles()` function returning a `clap::builder::Styles` that themes help output (green bold headers/usage, cyan literals/placeholders, red bold errors).

### `src/output/mod.rs`

Semantic helper functions, each writing to `anstream::stdout().lock()` (or `stderr` for errors):

| Function                        | Output                                    |
|---------------------------------|-------------------------------------------|
| `success(msg)`                  | `✓ {msg}` in green                       |
| `error(msg)`                    | `✗ {msg}` in red, to stderr              |
| `warning(msg)`                  | `! {msg}` in yellow                      |
| `header(msg)`                   | `{msg}` in bold                          |
| `label(name, value)`            | `  {name}: {value}` with name bolded     |
| `dim(msg)`                      | `{msg}` dimmed                           |
| `hint(msg)`                     | `{msg}` dimmed                           |
| `status_icon(healthy, msg)`     | `  ✓ {msg}` green or `  ✗ {msg}` red    |
| `item(msg)`                     | `  • {msg}`                              |
| `blank()`                       | Empty line                               |
| `plain(msg)`                    | Unstyled text                            |
| `progress(msg)`                 | `{msg}` dimmed, to stderr, no newline    |

Additionally, a `progress(msg)` helper writes to stderr without a trailing newline (using `eprint!` via `anstream::stderr()`) for inline progress indicators like `[1/50] file.md`.

`DIM` and `HINT` are visually identical (both dimmed) but semantically distinct: `dim` is for secondary/progress information, `hint` is for actionable guidance. This allows the styles to diverge later without changing call sites.

All functions use `anstream` for automatic TTY detection — no explicit fallback code needed.

## Clap Integration

Add `#[command(styles = output::clap_styles())]` to the `Cli` struct in `src/cli.rs`.

## Command Conversion Map

Each command's `println!`/`eprintln!` calls are replaced with the appropriate semantic helper:

### `init.rs`
Currently uses `eprintln!` throughout — migrates to stdout via helpers (acceptable since init is interactive, not piped).
- `"creating vault at ..."` → `output::dim(...)`
- `"wrote temper.toml"` → `output::success(...)`
- `"created {dir}/"` → `output::item(...)` (progress, not final success)
- `"already exists, skipping"` → `output::dim(...)`
- `"vault initialized successfully"` → `output::success(...)`
- "Next steps" block → `output::header("Next steps")` + `output::hint(...)` lines
- `"added project '...'"` → `output::success(...)`
- Global config messages → `output::dim(...)`

### `check.rs`
Currently uses `eprintln!` throughout — migrates to stdout via helpers (check output is for human reading, not script parsing).
- `"Vault: OK ..."` → `output::status_icon(true, ...)`
- `"Vault: FAIL ..."` → `output::status_icon(false, ...)`
- `"Dirs: WARN ..."` → `output::warning(...)` (not a binary pass/fail)
- Embedding, State OK/FAIL → `output::status_icon(true/false, ...)`

### `status.rs`
- `"Temper Vault"` → `output::header(...)`
- `"  Root: ..."` → `output::label("Root", ...)`
- Section headers ("Files", "Index", "Projects") → `output::header(...)`
- Key-value lines → `output::label(...)`
- "not built" hint → `output::hint(...)`

### `ticket.rs`
- `"Created ticket: ..."` → `output::success(...)`
- `"Moved ticket ..."` → `output::success(...)`
- `"Completed ticket: ..."` → `output::success(...)`
- `"No tickets found."` → `output::hint(...)`
- `ticket show` raw markdown content → `output::plain(...)` (passthrough)
- `ticket list` milestone headers → `output::header(...)`
- `ticket list` ticket lines → `output::plain(...)`
- Board title/separators → `output::header(...)` / `output::plain(...)`
- Board cell content → `output::plain(...)` (preserving fixed-width formatting)
- `"Board written to ..."` → `output::dim(...)`

### `milestone.rs`
- `"Created milestone: ..."` → `output::success(...)`
- `"Updated milestone ..."` → `output::success(...)`
- Roadmap header → `output::header(...)`
- `"No milestones ..."` → `output::hint(...)`

### `session.rs`
- `"Created: ..."`, `"Updated: ..."` → `output::success(...)`
- `"No sessions directory found."` → `output::warning(...)`
- `"No sessions found."` → `output::hint(...)`
- Table header/separator → `output::plain(...)` or `output::dim(...)`

### `project.rs`
- `"added project '...'"` → `output::success(...)`
- `"removed project ..."` → `output::success(...)`
- `"No projects configured."` → `output::hint(...)`
- Table header → `output::plain(...)`

### `note.rs`
- `"Created: ..."` → `output::success(...)`

### `events.rs`
- `"No events found."` → `output::hint(...)`
- Event lines → `output::plain(...)`

### `search.rs`
- `"No search index found."` → `output::warning(...)` with hint
- `"Index is empty."` → `output::warning(...)`

### `index.rs`
- `"Collecting files: ..."` → `output::dim(...)`
- `"Files to embed: ..."` summary → `output::dim(...)`
- `"Note: no existing index..."` → `output::dim(...)`
- `"Note: large file..."` → `output::dim(...)`
- Per-file progress `[1/50] file.md` → `output::progress(...)` (no newline, stderr)
- Per-file result `— N chunks` / `— skipped` → `eprintln!` continuation (completes the progress line)
- Per-file error `— ERROR: ...` → `eprintln!` continuation in red
- `"Warning: could not hash/read ..."` → `output::warning(...)`
- `"Building HNSW index: ..."` → `output::dim(...)`
- Final summary (total entries, index path) → `output::success(...)`
- Orphaned entries cleaned → `output::dim(...)`

### `skill.rs`
- `"OK (...)"` lines → `output::status_icon(true, ...)`
- `"NOT FOUND"` / `"STALE"` lines → `output::status_icon(false, ...)`
- `"Run: temper skill install"` → `output::hint(...)`

### `context.rs`
- `"No search index found."` → `output::warning(...)`

## Exclusions

### Warmup command (`warmup.rs`)
Outputs markdown consumed by Claude Code SessionStart hooks. Stays as raw `println!` — no ANSI codes in machine-consumed output.

### Skill generate (`skill.rs` generate subcommand)
Outputs raw skill file content to stdout for piping/redirection. Stays as raw `println!`.

### JSON output paths
The `--format json` paths in search, events, context already use `serde_json` serialization and are unaffected.

### `format.rs`
The `OutputFormat` enum and JSON/text rendering logic are unchanged. The text path uses `println!("{value}")` via Display impls which produce plain text — routing through `anstream` is unnecessary since no styling is applied to Display output.

### Test assertions
Tests don't capture styled output, so no test changes needed.

### Command logic
No changes to any data flow, business logic, or error handling.
