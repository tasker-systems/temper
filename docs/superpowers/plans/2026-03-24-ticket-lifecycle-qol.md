# Ticket Lifecycle QoL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve temper CLI ergonomics with simplified lifecycle stages, UUIDv7 entity IDs, stdin auto-detection, flag consistency, session-ticket linking, and two new commands (research save, normalize).

**Architecture:** All changes modify existing patterns — no new architectural concepts. UUIDv7 is added as a frontmatter field via the `uuid` crate. Stdin detection uses `std::io::IsTerminal`. New commands follow the established command handler pattern (cli.rs enum + commands/*.rs handler + main.rs routing + tests/*.rs).

**Tech Stack:** Rust, clap (CLI), uuid (v7), serde_yaml, chrono, tempfile (tests)

**Spec:** `docs/superpowers/specs/2026-03-24-ticket-lifecycle-qol-design.md`

---

### Task 1: Add uuid dependency and UUIDv7 helper

**Files:**
- Modify: `Cargo.toml:11-33`
- Create: `src/ids.rs`
- Modify: `src/lib.rs:1-12`
- Create: `tests/ids_test.rs`

- [ ] **Step 1: Write failing test for UUIDv7 generation**

```rust
// tests/ids_test.rs
#[test]
fn test_generate_id_returns_valid_uuidv7() {
    let id = temper_cli::ids::generate_id();
    // UUIDv7 format: 8-4-4-4-12 hex with version nibble = 7
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "7", "version nibble should be 7");
}

#[test]
fn test_generate_id_from_date_uses_timestamp() {
    let id1 = temper_cli::ids::generate_id_from_date("2026-01-01");
    let id2 = temper_cli::ids::generate_id_from_date("2026-06-15");
    // id2 should sort after id1 since UUIDv7 encodes timestamp in high bits
    assert!(id2 > id1, "later date should produce lexically greater UUID");
}

#[test]
fn test_generate_id_from_date_invalid_falls_back() {
    let id = temper_cli::ids::generate_id_from_date("not-a-date");
    // Should still produce a valid UUID (falls back to now)
    assert_eq!(id.len(), 36);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test ids_test`
Expected: FAIL — module `ids` not found

- [ ] **Step 3: Add uuid dependency**

Add to `Cargo.toml` under `[dependencies]`:
```toml
uuid = { version = "1", features = ["v7"] }
```

- [ ] **Step 4: Write ids module**

```rust
// src/ids.rs
use uuid::{Uuid, timestamp::Timestamp};

/// Generate a new UUIDv7 using the current timestamp.
pub fn generate_id() -> String {
    Uuid::now_v7().to_string()
}

/// Generate a UUIDv7 from a date string (YYYY-MM-DD).
/// Falls back to current timestamp if parsing fails.
pub fn generate_id_from_date(date_str: &str) -> String {
    if let Some(ts) = parse_date_to_timestamp(date_str) {
        Uuid::new_v7(ts).to_string()
    } else {
        generate_id()
    }
}

fn parse_date_to_timestamp(date_str: &str) -> Option<Timestamp> {
    let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let secs = datetime.and_utc().timestamp() as u64;
    Some(Timestamp::from_unix(uuid::NoContext, secs, 0))
}
```

- [ ] **Step 5: Register ids module in lib.rs**

Add `pub mod ids;` to `src/lib.rs`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test ids_test`
Expected: PASS (3 tests)

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/ids.rs src/lib.rs tests/ids_test.rs
git commit -m "feat: add uuid crate and ids module for UUIDv7 generation"
```

---

### Task 2: Update templates to include id field

**Files:**
- Modify: `src/templates/ticket.md`
- Modify: `src/templates/session.md`
- Modify: `src/templates/milestone.md`
- Create: `src/templates/research.md`

- [ ] **Step 1: Add `id` placeholder to ticket template**

Update `src/templates/ticket.md` — add `id: "{{id}}"` as the first field after `---`:

```markdown
---
id: "{{id}}"
type: ticket
title: "{{title}}"
slug: "{{slug}}"
project: "{{project}}"
milestone: "{{milestone}}"
stage: backlog
seq: {{seq}}
created: {{datetime}}
updated: {{datetime}}
branch: null
pr: null
---

# {{title}}
```

- [ ] **Step 2: Add `id` placeholder to session template**

Update `src/templates/session.md`:

```markdown
---
id: "{{id}}"
type: session
date: {{date}}
project: ""
---

# Session: {{title}}

## Goal

What this session set out to accomplish.

## What happened

What was attempted, what worked, what didn't.

## Decisions

Significant choices made and why (alternatives considered).

## What connected

Concepts, patterns, or cross-project links noticed.

## To pick up

Next steps, open threads, things to investigate.
```

- [ ] **Step 3: Add `id` placeholder to milestone template**

Update `src/templates/milestone.md`:

```markdown
---
id: "{{id}}"
type: milestone
title: "{{title}}"
slug: "{{slug}}"
project: "{{project}}"
seq: {{seq}}
status: active
created: {{date}}
---

# {{title}}
```

- [ ] **Step 4: Create research template**

Create `src/templates/research.md`:

```markdown
---
id: "{{id}}"
type: research
date: {{date}}
project: "{{project}}"
title: "{{title}}"
slug: "{{slug}}"
---

# {{title}}

## Topic

What question or area is being investigated.

## Findings

Key discoveries, data points, and conclusions.

## Sources

References, links, documentation consulted.

## Implications

How this affects current or planned work.

## Open Questions

What remains unknown or needs further investigation.
```

- [ ] **Step 5: Register research template as embedded**

In `src/vault.rs`, add the embedded template constant and match arm:

```rust
const EMBEDDED_RESEARCH: &str = include_str!("templates/research.md");

fn embedded_template(note_type: &str) -> Option<&'static str> {
    match note_type {
        "session" => Some(EMBEDDED_SESSION),
        "ticket" => Some(EMBEDDED_TICKET),
        "milestone" => Some(EMBEDDED_MILESTONE),
        "research" => Some(EMBEDDED_RESEARCH),
        _ => None,
    }
}
```

- [ ] **Step 6: Run existing tests to verify nothing broke**

Run: `cargo test`
Expected: All existing tests pass. Some ticket tests may fail due to the new `{{id}}` placeholder not being substituted — that will be fixed in Task 3.

- [ ] **Step 7: Commit**

```bash
git add src/templates/ src/vault.rs
git commit -m "feat: add id placeholder to templates and create research template"
```

---

### Task 3: Wire UUIDv7 into entity creation paths

**Files:**
- Modify: `src/commands/ticket.rs:127-204` (create function)
- Modify: `src/commands/session.rs:20-99` (save function)
- Modify: `src/commands/milestone.rs` (create function)
- Modify: `tests/ticket_test.rs`

- [ ] **Step 1: Write test for UUIDv7 in ticket frontmatter**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_create_includes_uuid_id() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "ID Test", Some(&ms_slug), false,
    ).unwrap();

    let content = std::fs::read_to_string(
        dir.path().join("tickets/myapp").join(format!("{slug}.md")),
    ).unwrap();
    // Should have an id field with a UUIDv7 (36 chars, version nibble 7)
    assert!(content.contains("id: \"0"), "should contain a UUIDv7 id field");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test ticket_test test_ticket_create_includes_uuid_id`
Expected: FAIL — id field contains `{{id}}` placeholder, not an actual UUID

- [ ] **Step 3: Wire UUIDv7 into ticket create**

In `src/commands/ticket.rs`, in the `create` function, add the id generation and pass it as a template variable:

After line 157 (`let seq_str = seq.to_string();`), add:
```rust
let id = crate::ids::generate_id();
```

Update the `vars` vector to include `("id", id.as_str())`.

- [ ] **Step 4: Wire UUIDv7 into milestone create**

In `src/commands/milestone.rs`, in the `create` function, generate an id and pass it as a template variable (same pattern: `let id = crate::ids::generate_id();` and add to vars).

- [ ] **Step 5: Wire UUIDv7 into session save**

In `src/commands/session.rs`, in the `save` function, generate an id and pass it as a template variable when rendering the session template. Add `("id", &id)` to the vars in `render_template_with_vars`.

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: All tests pass, including the new UUID test

- [ ] **Step 7: Commit**

```bash
git add src/commands/ticket.rs src/commands/session.rs src/commands/milestone.rs tests/ticket_test.rs
git commit -m "feat: generate UUIDv7 for all entity creation paths"
```

---

### Task 4: Simplify lifecycle stages

**Files:**
- Modify: `src/commands/ticket.rs:216-231` (valid_stages in move_ticket)
- Modify: `src/commands/ticket.rs:376-383` (stages in board)
- Modify: `src/commands/ticket.rs:402-414` (board column headers)
- Modify: `src/commands/ticket.rs:491` (markdown board headers)
- Modify: `tests/ticket_test.rs`

- [ ] **Step 1: Update test for new stages**

Update `test_ticket_move_to_brainstorm` in `tests/ticket_test.rs` to test `in-progress` instead:

```rust
#[test]
fn test_ticket_move_to_in_progress() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false)
            .unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("in-progress"), None).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: in-progress"));
}

#[test]
fn test_ticket_move_rejects_old_stages() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false)
            .unwrap();

    let result = temper_cli::commands::ticket::move_ticket(&config, &slug, Some("brainstorm"), None);
    assert!(result.is_err(), "old stage 'brainstorm' should be rejected");
}

#[test]
fn test_ticket_move_to_cancelled() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false)
            .unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("cancelled"), None).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: cancelled"));
}
```

- [ ] **Step 2: Update `test_ticket_move_and_done` to use new stages**

Change line 70 from `Some("implement")` to `Some("in-progress")`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test ticket_test`
Expected: FAIL — old `brainstorm`/`implement` stages still in valid_stages

- [ ] **Step 4: Update valid_stages in move_ticket**

In `src/commands/ticket.rs`, replace the `valid_stages` array (lines 216-223):

```rust
let valid_stages = ["backlog", "in-progress", "done", "cancelled"];
```

- [ ] **Step 5: Update board view for 4 stages**

In `src/commands/ticket.rs`, update the `board` function:

Replace the `stages` array (line 376-383):
```rust
let stages = ["backlog", "in-progress", "done", "cancelled"];
```

Replace the terminal column header (line 402-404):
```rust
output::plain(format!(
    " {:<20}│ {:<20}│ {:<20}│ {:<16}",
    "Backlog", "In Progress", "Done", "Cancelled"
));
```

Update the separator line widths, cell widths, and row rendering to use 4 columns instead of 6.

Replace the markdown board header (line 491):
```rust
md.push_str("| Backlog | In Progress | Done | Cancelled |\n");
md.push_str("|---------|-------------|------|----------|\n");
```

Update the corresponding cell rendering for 4 columns.

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add src/commands/ticket.rs tests/ticket_test.rs
git commit -m "feat: simplify lifecycle to backlog/in-progress/done/cancelled"
```

---

### Task 5: Stdin auto-detection

**Files:**
- Modify: `src/commands/ticket.rs:176-185` (create)
- Modify: `src/commands/note.rs:46-52` (create)
- Modify: `src/main.rs:115-122` (session save stdin)
- Modify: `src/cli.rs` (keep --stdin but make it hidden/deprecated)

- [ ] **Step 1: Write test for stdin auto-detection helper**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_create_without_stdin_flag() {
    // Verify that create works without the stdin flag
    // (stdin auto-detection will be a no-op in test context since stdin IS a terminal)
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "No stdin", Some(&ms_slug), false,
    ).unwrap();

    let content = std::fs::read_to_string(
        dir.path().join("tickets/myapp").join(format!("{slug}.md")),
    ).unwrap();
    assert!(content.contains("No stdin"));
}
```

- [ ] **Step 2: Create stdin helper in vault.rs**

Add to `src/vault.rs`:

```rust
/// Read stdin content if stdin is not a terminal (piped input).
/// Returns None if stdin is a terminal or if reading fails.
pub fn read_stdin_if_piped() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).ok()?;
    if buf.is_empty() { None } else { Some(buf) }
}
```

- [ ] **Step 3: Update ticket create to use auto-detection**

In `src/commands/ticket.rs`, replace the stdin block (lines 176-185):

```rust
if let Some(stdin_content) = vault::read_stdin_if_piped() {
    content.push_str(&stdin_content);
    content.push('\n');
}
```

Remove the `stdin: bool` parameter from the `create` function signature. Update callers in `main.rs`.

- [ ] **Step 4: Update note create to use auto-detection**

In `src/commands/note.rs`, replace the stdin block (lines 46-52) and the `read_stdin` function with `vault::read_stdin_if_piped()`. Remove the `from_stdin: bool` parameter.

- [ ] **Step 5: Update session save to use auto-detection**

In `main.rs` (lines 115-122), replace the stdin handling for session save. Instead of checking the `stdin` flag, call `vault::read_stdin_if_piped()` and pass the result to `session::save`.

- [ ] **Step 6: Hide --stdin flag in CLI**

In `src/cli.rs`, add `#[arg(long, hide = true)]` to each `stdin` field to keep backward compat but hide from help.

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/vault.rs src/commands/ticket.rs src/commands/note.rs src/main.rs src/cli.rs tests/ticket_test.rs
git commit -m "feat: auto-detect stdin via TTY check, deprecate --stdin flag"
```

---

### Task 6: Add --show-template flag

**Files:**
- Modify: `src/cli.rs` (add flag to TicketAction::Create, NoteAction::Create, SessionAction::Save)
- Modify: `src/main.rs` (handle flag before calling create/save)
- Modify: `src/vault.rs` (expose template rendering for preview)

- [ ] **Step 1: Add --show-template to CLI definitions**

In `src/cli.rs`, add `#[arg(long)] show_template: bool` to:
- `TicketAction::Create`
- `NoteAction::Create`
- `SessionAction::Save`

- [ ] **Step 2: Handle --show-template in main.rs routing**

For each command, check `show_template` before calling the handler. If true, print the template and return:

```rust
// Example for ticket create
if show_template {
    let content = temper_cli::vault::get_template("ticket")?;
    print!("{content}");
    return Ok(());
}
```

- [ ] **Step 3: Add get_template helper to vault.rs**

```rust
/// Return the raw template content for a note type.
pub fn get_template(note_type: &str) -> Result<String> {
    embedded_template(note_type)
        .map(String::from)
        .ok_or_else(|| TemperError::Vault(format!("No template found for '{note_type}'")))
}
```

Make `embedded_template` and `get_template` both `pub`.

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs src/vault.rs
git commit -m "feat: add --show-template flag for template-based commands"
```

---

### Task 7: Add --project flag to ticket move/done/show and milestone update

**Files:**
- Modify: `src/cli.rs:152-176` (TicketAction::Move, Done, Show)
- Modify: `src/cli.rs:202-207` (MilestoneAction::Update)
- Modify: `src/commands/ticket.rs` (find_ticket to accept optional project filter)
- Modify: `src/main.rs:163-185` (routing for move/done/show)

- [ ] **Step 1: Add --project to CLI definitions**

In `src/cli.rs`, add `#[arg(long)] project: Option<String>` to:
- `TicketAction::Move`
- `TicketAction::Done`
- `TicketAction::Show`
- `MilestoneAction::Update`

- [ ] **Step 2: Update find_ticket to accept optional project filter**

In `src/commands/ticket.rs`, update `find_ticket` signature:

```rust
pub fn find_ticket(config: &Config, slug_or_suffix: &str, project: Option<&str>) -> Result<Option<TicketInfo>> {
    let all = load_tickets(config, project, None)?;
    // ... rest unchanged
}
```

- [ ] **Step 3: Update all callers of find_ticket**

Update `move_ticket`, `done`, `show` to accept `project: Option<&str>` and pass it through to `find_ticket`. Update their call sites in `main.rs` to resolve project from flag or CWD (using the existing pattern from ticket create/board).

- [ ] **Step 4: Update milestone update similarly**

Add `project: Option<&str>` parameter to `milestone::update` and use it for lookup.

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: All tests pass (test callers pass `None` for project)

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/commands/ticket.rs src/commands/milestone.rs src/main.rs
git commit -m "feat: add --project flag to ticket move/done/show and milestone update"
```

---

### Task 8: Add --format json|text to remaining commands

**Files:**
- Modify: `src/cli.rs` (add format field)
- Modify: `src/commands/ticket.rs` (list, show, board — add JSON output paths)
- Modify: `src/commands/session.rs` (list — add JSON output path)
- Modify: `src/commands/milestone.rs` (list, create — add JSON output path)
- Modify: `src/commands/note.rs` (create — add JSON output path)
- Modify: `src/main.rs` (pass format through)

- [ ] **Step 1: Add --format to CLI definitions**

Add `#[arg(long, default_value = "text")] format: String` to:
- `TicketAction::List`
- `TicketAction::Show`
- `TicketAction::Board`
- `MilestoneAction::List`
- `MilestoneAction::Create`
- `SessionAction::List`
- `NoteAction::Create`

- [ ] **Step 2: Write test for JSON output**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_list_json() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(
        &config, "myapp", "JSON Test", Some(&ms_slug), false,
    ).unwrap();

    // list with json format should not error
    let result = temper_cli::commands::ticket::list(&config, Some("myapp"), None, "json");
    assert!(result.is_ok());
}
```

- [ ] **Step 3: Add format parameter to handler functions**

Update function signatures to accept `format: &str`. For `"json"` format:
- **list/show**: Serialize the loaded entity data as JSON array/object and print
- **create**: Serialize the created entity's frontmatter fields as JSON and print

For text format, keep existing output unchanged.

- [ ] **Step 4: Update main.rs routing to pass format through**

Thread the `format` field from CLI to handler functions.

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/commands/ticket.rs src/commands/session.rs src/commands/milestone.rs src/commands/note.rs src/main.rs tests/ticket_test.rs
git commit -m "feat: add --format json|text to ticket/session/milestone/note commands"
```

---

### Task 9: Session-ticket linking

**Files:**
- Modify: `src/cli.rs:228-241` (SessionAction::Save)
- Modify: `src/commands/session.rs:20-99` (save function)
- Modify: `src/main.rs:107-133` (session routing)
- Create: `tests/session_ticket_test.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/session_ticket_test.rs
use tempfile::TempDir;

#[test]
fn test_session_save_with_ticket_links_entities() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let ticket_slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Linked ticket", Some(&ms_slug), false,
    ).unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Linked Session"),
        Some("myapp"),
        Some("Session body content"),
        Some(&ticket_slug),
        None, // no state change
    ).unwrap();

    // Verify ticket now has a sessions field
    let ticket_content = std::fs::read_to_string(
        dir.path().join("tickets/myapp").join(format!("{ticket_slug}.md")),
    ).unwrap();
    assert!(ticket_content.contains("sessions:"), "ticket should have sessions field");
}

#[test]
fn test_session_save_with_ticket_and_state_moves_ticket() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let ticket_slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Done ticket", Some(&ms_slug), false,
    ).unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Final Session"),
        Some("myapp"),
        Some("Wrapping up"),
        Some(&ticket_slug),
        Some("done"),
    ).unwrap();

    let ticket_content = std::fs::read_to_string(
        dir.path().join("tickets/myapp").join(format!("{ticket_slug}.md")),
    ).unwrap();
    assert!(ticket_content.contains("stage: done"), "ticket should be marked done");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test session_ticket_test`
Expected: FAIL — save function doesn't have ticket/state parameters

- [ ] **Step 3: Add CLI flags**

In `src/cli.rs`, add to `SessionAction::Save`:
```rust
#[arg(long)]
ticket: Option<String>,
#[arg(long)]
state: Option<String>,
```

- [ ] **Step 4: Update session save function**

Update `session::save` signature to add `ticket: Option<&str>` and `state: Option<&str>` parameters.

After creating/updating the session note, if `ticket` is provided:
1. Find the ticket via `ticket::find_ticket`
2. Read the ticket file
3. Parse the session's UUIDv7 from its frontmatter
4. Append session ID to the ticket's `sessions` list in frontmatter (or create the field)
5. Set ticket's `branch` from `git rev-parse --abbrev-ref HEAD`
6. Look for matching spec/plan docs and add to ticket's `docs` field
7. If `state` is provided, update the ticket's `stage` field
8. Write the updated ticket

- [ ] **Step 5: Update main.rs routing**

Pass the new `ticket` and `state` fields through from CLI to `session::save`.

- [ ] **Step 6: Update existing session tests**

Update existing calls to `session::save` in `tests/session_test.rs` to pass `None, None` for the new parameters.

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/cli.rs src/commands/session.rs src/main.rs tests/session_ticket_test.rs tests/session_test.rs
git commit -m "feat: add --ticket and --state flags to session save for ticket linking"
```

---

### Task 10: New command — temper research save

**Files:**
- Create: `src/commands/research.rs`
- Modify: `src/commands/mod.rs`
- Modify: `src/cli.rs` (add Research command with Save subcommand)
- Modify: `src/main.rs` (add routing)
- Create: `tests/research_test.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/research_test.rs
use tempfile::TempDir;

#[test]
fn test_research_save_creates_note() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let result = temper_cli::commands::research::save(
        &config,
        "LLM Context Windows",
        Some("myapp"),
        None, // no stdin
    );
    assert!(result.is_ok());

    let research_dir = dir.path().join("research/myapp");
    assert!(research_dir.is_dir());
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("type: research"));
    assert!(content.contains("LLM Context Windows"));
    assert!(content.contains("id: \"0")); // UUIDv7 starts with 0
}

#[test]
fn test_research_save_idempotent_without_stdin() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None).unwrap();

    let research_dir = dir.path().join("research/myapp");
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let before = std::fs::read_to_string(&path).unwrap();

    // Second save without stdin should be idempotent
    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn test_research_save_with_stdin_replaces_body() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None).unwrap();

    temper_cli::commands::research::save(
        &config,
        "Topic",
        Some("myapp"),
        Some("Updated research findings"),
    ).unwrap();

    let research_dir = dir.path().join("research/myapp");
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("Updated research findings"));
    assert!(content.starts_with("---"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test research_test`
Expected: FAIL — module `research` not found

- [ ] **Step 3: Implement research command**

Create `src/commands/research.rs` following the session.rs pattern:

```rust
use chrono::Local;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::project;
use crate::vault;

/// Create or update a research note.
///
/// Path: `<vault_root>/research/<project>/<date> — <title>.md`
///
/// Follows session save semantics: idempotent without stdin, replaces body with stdin.
pub fn save(
    config: &Config,
    title: &str,
    project: Option<&str>,
    stdin_content: Option<&str>,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    let project_name: String = if let Some(p) = project {
        p.to_string()
    } else if let Ok(cwd) = std::env::current_dir() {
        project::resolve_from_cwd(&cwd, &config.projects)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "general".to_string())
    } else {
        "general".to_string()
    };

    let slug = format!("{today}-{}", vault::slugify(title));
    let filename = format!("{today} \u{2014} {title}.md");
    let research_dir = config.vault_root.join("research").join(&project_name);
    let note_path = research_dir.join(&filename);

    if note_path.exists() {
        if let Some(body) = stdin_content {
            let existing = std::fs::read_to_string(&note_path)?;
            let updated = replace_body(&existing, body);
            std::fs::write(&note_path, updated)?;
            let relative = note_path.strip_prefix(&config.vault_root).unwrap_or(&note_path);
            output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    let id = crate::ids::generate_id();
    let templates_rel = config.templates_dir
        .strip_prefix(&config.vault_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "templates".to_string());

    let mut content = vault::render_template_with_vars(
        &config.vault_root,
        &templates_rel,
        "research",
        title,
        &[("project", &project_name), ("id", &id), ("slug", &slug)],
    )?;

    content = vault::set_frontmatter_field(&content, "project", &project_name);

    if let Some(body) = stdin_content {
        content = replace_body(&content, body);
    }

    vault::write_note(&note_path, &content)?;

    let relative = note_path.strip_prefix(&config.vault_root).unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();
    output::success(format!("Created: {relative_str}"));

    let ts = Local::now().to_rfc3339();
    let event = Event::NoteCreate {
        ts,
        note_type: "research".to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        project: project_name,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

fn replace_body(existing: &str, new_body: &str) -> String {
    let trimmed = existing.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let frontmatter_end = 3 + end + 3;
            let frontmatter = &trimmed[..frontmatter_end];
            return format!("{frontmatter}\n\n{new_body}");
        }
    }
    new_body.to_string()
}
```

- [ ] **Step 4: Register module and add CLI/routing**

Add `pub mod research;` to `src/commands/mod.rs`.

Add `Research` subcommand to `src/cli.rs`:
```rust
/// Research notes
Research {
    #[command(subcommand)]
    action: ResearchAction,
},
```

```rust
#[derive(Subcommand)]
pub enum ResearchAction {
    /// Create or update a research note
    Save {
        title: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
        #[arg(long)]
        show_template: bool,
        #[arg(long, hide = true)]
        stdin: bool,
    },
}
```

Add routing in `main.rs`.

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/commands/research.rs src/commands/mod.rs src/cli.rs src/main.rs tests/research_test.rs
git commit -m "feat: add temper research save command for high-fidelity research notes"
```

---

### Task 11: New command — temper normalize

**Files:**
- Create: `src/commands/normalize.rs`
- Modify: `src/commands/mod.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/discovery.rs` (add Normalize event)
- Create: `tests/normalize_test.rs`

- [ ] **Step 1: Add Normalize event variant**

In `src/discovery.rs`, add:

```rust
#[serde(rename = "normalize")]
Normalize {
    ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    ids_backfilled: u32,
    files_moved: u32,
    stages_migrated: u32,
    slugs_fixed: u32,
    frontmatter_fixed: u32,
},
```

- [ ] **Step 2: Write failing tests**

```rust
// tests/normalize_test.rs
use tempfile::TempDir;

#[test]
fn test_normalize_backfills_missing_ids() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Create a ticket normally (it will have an id)
    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Test", Some(&ms_slug), false,
    ).unwrap();

    // Strip the id field to simulate a pre-UUIDv7 ticket
    let path = dir.path().join("tickets/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let stripped = content.lines()
        .filter(|l| !l.starts_with("id:"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{stripped}\n")).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.ids_backfilled > 0, "should backfill at least one ID");

    // Verify the id was added
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("id:"), "ticket should now have an id field");
}

#[test]
fn test_normalize_migrates_old_stages() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Old Stage", Some(&ms_slug), false,
    ).unwrap();

    // Manually set stage to an old value
    let path = dir.path().join("tickets/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let modified = content.replace("stage: backlog", "stage: brainstorm");
    std::fs::write(&path, &modified).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.stages_migrated > 0);

    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("stage: in-progress"));
}

#[test]
fn test_normalize_dry_run_makes_no_changes() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Dry run", Some(&ms_slug), false,
    ).unwrap();

    // Strip id to create something to normalize
    let path = dir.path().join("tickets/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let stripped = content.lines()
        .filter(|l| !l.starts_with("id:"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{stripped}\n")).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    temper_cli::commands::normalize::run(&config, None, true, false).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "dry-run should not modify files");
}

#[test]
fn test_normalize_moves_misplaced_files() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Misplaced", Some(&ms_slug), false,
    ).unwrap();

    // Move file to wrong project directory
    let correct_path = dir.path().join("tickets/myapp").join(format!("{slug}.md"));
    let wrong_dir = dir.path().join("tickets/wrong");
    std::fs::create_dir_all(&wrong_dir).unwrap();
    let wrong_path = wrong_dir.join(format!("{slug}.md"));
    std::fs::rename(&correct_path, &wrong_path).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.files_moved > 0);
    assert!(correct_path.exists(), "file should be moved back to correct project dir");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test normalize_test`
Expected: FAIL — module not found

- [ ] **Step 4: Implement normalize command**

Create `src/commands/normalize.rs`. The command should:

1. Scan all entity directories (tickets, sessions, milestones, research)
2. For each markdown file, parse frontmatter
3. Apply repairs based on what's missing/wrong
4. Track counts in a `NormalizeSummary` struct
5. If not `--dry-run`, write changes and record event
6. Print summary

Return a `NormalizeSummary` struct:
```rust
pub struct NormalizeSummary {
    pub ids_backfilled: u32,
    pub files_moved: u32,
    pub stages_migrated: u32,
    pub slugs_fixed: u32,
    pub frontmatter_fixed: u32,
}
```

The `run` function signature:
```rust
pub fn run(config: &Config, project: Option<&str>, dry_run: bool, fix_slugs: bool) -> Result<NormalizeSummary>
```

Key implementation details:
- Use `vault::collect_md_files_recursive` to find all files
- Parse frontmatter with `vault::parse_frontmatter`
- For missing ids, use `ids::generate_id_from_date` with the date from slug or frontmatter
- For old stages, map `brainstorm|design|plan|implement` → `in-progress`
- For wrong directories, compare frontmatter `project` against parent directory name
- Add the `id` field by inserting a line after the opening `---`

- [ ] **Step 5: Register module and add CLI/routing**

Add `pub mod normalize;` to `src/commands/mod.rs`.

Add `Normalize` command to `src/cli.rs`:
```rust
/// Normalize vault structure and repair drift
Normalize {
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    fix_slugs: bool,
},
```

Add routing in `main.rs`.

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add src/commands/normalize.rs src/commands/mod.rs src/cli.rs src/main.rs src/discovery.rs tests/normalize_test.rs
git commit -m "feat: add temper normalize command for vault repair and consistency"
```

---

### Task 12: Update skill generation

**Files:**
- Modify: `src/commands/skill.rs:30-86` (generate function template)

- [ ] **Step 1: Update the skill template string**

In `src/commands/skill.rs`, update the `format!` template in the `generate` function to:

1. Replace `--stdin` references with auto-detection note
2. Add `research save` and `normalize` commands
3. Update `session save` with `--ticket` and `--state` flags
4. Change `ticket start` from "brainstorm" to "in-progress"
5. Remove "brainstorm → design → plan → implement" superpowers stage references
6. Add search/context guidance section
7. Add `--show-template` mention for template commands
8. Document 4-stage lifecycle

Updated commands section should include:
```
- `temper search <query>` — Semantic search across indexed content (use before grep/find)
- `temper context <topic> [--depth N]` — Traverse nearest neighbors for related content
- `temper session save [<title>] [--ticket <slug>] [--state <state>]` — Create/update session note, optionally link to ticket
- `temper research save <title>` — Create high-fidelity research note (stdin auto-detected)
- `temper normalize [--dry-run]` — Repair vault structure drift
```

Updated workflow section should reference `in-progress` instead of `brainstorm`, and add search/context discovery guidance.

- [ ] **Step 2: Run skill test**

Run: `cargo test --test skill_test`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/commands/skill.rs
git commit -m "feat: update skill generation with new commands and search guidance"
```

---

### Task 13: Final verification and cleanup

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run formatting check**

Run: `cargo fmt -- --check`
Expected: No formatting issues

- [ ] **Step 4: Build release binary**

Run: `cargo build --release`
Expected: Clean build

- [ ] **Step 5: Smoke test new commands**

Run:
```bash
temper normalize --dry-run
temper research save --show-template
temper ticket create --show-template
temper ticket board
```
Expected: All commands produce expected output

- [ ] **Step 6: Reinstall skill**

Run: `temper skill install`
Expected: Skill installed with updated content
