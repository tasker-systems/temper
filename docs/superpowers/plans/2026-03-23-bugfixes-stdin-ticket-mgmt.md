# Bugfixes, Stdin Commands, Ticket Management — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix bugs in skill template and status counting, migrate milestones to project subdirectories, add `brainstorm` ticket stage, and add `events`/`warmup` commands for session context priming.

**Architecture:** Seven independent-ish tasks touching the skill generator template, status counter, milestone paths, ticket stages, and two new commands. The `warmup` command depends on `events` being implemented first. All other tasks are independent.

**Tech Stack:** Rust, clap (CLI), serde/serde_json/serde_yaml (serialization), chrono (timestamps), tempfile (tests)

**Spec:** `docs/superpowers/specs/2026-03-23-bugfixes-stdin-ticket-mgmt-design.md`

---

### Task 1: Skill template — invocation section, --stdin docs, ticket start shorthand

**Files:**
- Modify: `src/commands/skill.rs:29-67` (template string in `generate()`)
- Test: `tests/skill_test.rs`

- [ ] **Step 1: Write failing test for invocation section**

Add to `tests/skill_test.rs`:

```rust
#[test]
fn test_skill_generate_includes_invocation_section() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(
        content.contains("## Invocation"),
        "skill should contain invocation section"
    );
    assert!(
        content.contains("installed binary"),
        "skill should say temper is an installed binary"
    );
    assert!(
        content.contains("Never use `cargo run`"),
        "skill should warn against cargo run"
    );
}

#[test]
fn test_skill_generate_documents_stdin_flag() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(
        content.contains("--stdin"),
        "skill should document --stdin flag"
    );
}

#[test]
fn test_skill_generate_includes_ticket_start() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(
        content.contains("ticket start"),
        "skill should document ticket start shorthand"
    );
    assert!(
        content.contains("brainstorming skill"),
        "skill should reference brainstorming skill"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test skill_test -- --nocapture`
Expected: 3 new tests FAIL (content doesn't contain invocation/stdin/ticket start)

- [ ] **Step 3: Update skill template in generate()**

Replace the template string in `src/commands/skill.rs:29-67`. The new template adds:
1. `## Invocation` section between vault path and `## Projects`
2. `--stdin` flags on ticket create, session save, note create command lines
3. `ticket start` in command reference
4. `ticket start` workflow in Workflow Integration section

```rust
    let content = format!(
        r#"<!-- config-hash: {hash} -->
---
name: temper
description: Knowledge vault operations — context lookup, session notes, ticket management, semantic search
---

# Temper — Vault Workflow Tool

Vault: {vault_path}

## Invocation

`temper` is an installed binary in `$PATH`. Always run it directly as `temper <subcommand>`.
Never use `cargo run`, `python`, full binary paths, or any other indirect method — even when
working inside the temper source repo.

## Projects
{project_list}

## Commands

- `temper search <query>` — Semantic search across indexed content
- `temper context <topic>` — Show topic with related context
- `temper session save [<title>] [--stdin]` — Create/update session note (pipe body via stdin)
- `temper session list` — List recent sessions
- `temper ticket create --title <t> --project <p> [--stdin]` — Create ticket (pipe body via stdin)
- `temper ticket list` — List tickets
- `temper ticket board` — Board view
- `temper milestone list` — Roadmap view
- `temper note create <type> <title> [--stdin]` — Create note from template (pipe body via stdin)
- `temper events [--project <p>] [--limit <n>]` — Show recent vault events
- `temper warmup [--project <p>]` — Context primer for new sessions
- `temper index` — Rebuild search index
- `temper status` — Vault overview

## Workflow Integration

When starting a session:
- Check for recent sessions: `temper session list --project <current>`
- Search for relevant context: `temper search "<topic>"`

When ending a session:
- Suggest: `temper session save`

When the user says `/temper ticket start <slug>`:
1. Run `temper ticket move <slug> --stage brainstorm --project <p>`
2. Run `temper ticket show <slug>`
3. Invoke the brainstorming skill with the ticket content as context

This tool uses the superpowers workflow: brainstorm → design → plan → implement → finish.
"#,
        hash = hash,
        vault_path = vault_path,
        project_list = project_list,
    );
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test skill_test -- --nocapture`
Expected: All skill tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/commands/skill.rs tests/skill_test.rs
git commit -m "fix: update skill template with invocation guidance, --stdin docs, ticket start"
```

---

### Task 2: Add `brainstorm` as valid ticket stage

**Files:**
- Modify: `src/commands/ticket.rs:215` (valid_stages in `move_ticket`)
- Modify: `src/commands/ticket.rs:364` (stages array in `board`)
- Modify: `src/commands/ticket.rs:384-386` (terminal board header)
- Modify: `src/commands/ticket.rs:387-394` (terminal board divider)
- Modify: `src/commands/ticket.rs:418-422` (column width mappings — indices shift with new column)
- Modify: `src/commands/ticket.rs:435-438` (terminal cell join — now 6 cells)
- Modify: `src/commands/ticket.rs:441-448` (terminal bottom divider — now 6 columns)
- Modify: `src/commands/ticket.rs:470-471` (markdown table header and divider)
- Modify: `src/commands/milestone.rs:205` (stages in `list`)
- Test: `tests/ticket_test.rs`

- [ ] **Step 1: Write failing test for brainstorm stage**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_move_to_brainstorm() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false)
            .unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("brainstorm"), None).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: brainstorm"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test ticket_test test_ticket_move_to_brainstorm -- --nocapture`
Expected: FAIL with "invalid stage: brainstorm"

- [ ] **Step 3: Add brainstorm to valid_stages and board stage arrays**

In `src/commands/ticket.rs:215` (`move_ticket` validation):
```rust
    let valid_stages = ["backlog", "brainstorm", "design", "plan", "implement", "done"];
```

In `src/commands/ticket.rs:364` (`board` stages array):
```rust
    let stages = ["backlog", "brainstorm", "design", "plan", "implement", "done"];
```

In `src/commands/milestone.rs:205` (`list` stage iteration):
```rust
        for stage in &["backlog", "brainstorm", "design", "plan", "implement", "done"] {
```

- [ ] **Step 3a: Update board terminal and markdown headers for 6 columns**

The `board()` function in `ticket.rs` has hardcoded stage headers for both terminal and markdown output. All must be updated to include "Brainstorm" as the second column.

**Terminal header** (ticket.rs:384-386) — add Brainstorm column:
```rust
        println!(
            " {:<16}│ {:<16}│ {:<16}│ {:<8}│ {:<16}│ Done",
            "Backlog", "Brainstorm", "Design", "Plan", "Implement"
        );
```

**Terminal divider** (ticket.rs:387-394) — add 6th column:
```rust
        println!(
            "{}┼{}┼{}┼{}┼{}┼{}",
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(9),
            "─".repeat(17),
            "─".repeat(9)
        );
```

**Column width mappings** (ticket.rs:418-422) — update indices for 6 columns:
```rust
                    let width = match i {
                        3 => 8,   // Plan (was index 2)
                        5 => 9,   // Done (was index 4)
                        _ => 16,  // Backlog, Brainstorm, Design, Implement
                    };
```

**Terminal cell join** (ticket.rs:435-438) — now 6 cells:
```rust
            println!(
                "{}│{}│{}│{}│{}│{}",
                cells[0], cells[1], cells[2], cells[3], cells[4], cells[5]
            );
```

**Terminal bottom divider** (ticket.rs:441-448) — 6 columns:
```rust
        println!(
            "{}┴{}┴{}┴{}┴{}┴{}",
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(9),
            "─".repeat(17),
            "─".repeat(9)
        );
```

**Markdown table header** (ticket.rs:470-471):
```rust
        md.push_str("| Backlog | Brainstorm | Design | Plan | Implement | Done |\n");
        md.push_str("|---------|------------|--------|------|-----------|------|\n");
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test ticket_test -- --nocapture`
Expected: All ticket tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/commands/ticket.rs src/commands/milestone.rs tests/ticket_test.rs
git commit -m "feat: add brainstorm as valid ticket stage"
```

---

### Task 3: Fix recursive counting in `temper status`

**Files:**
- Modify: `src/commands/status.rs:72-90` (replace `count_md_files`)
- Test: `tests/check_test.rs` (or new `tests/status_test.rs`)

- [ ] **Step 1: Write failing test for recursive counting**

Create `tests/status_test.rs`:

```rust
use tempfile::TempDir;

#[test]
fn test_count_md_files_recursive() {
    let dir = TempDir::new().unwrap();

    // Create nested structure: dir/project_a/file1.md, dir/project_a/file2.md, dir/project_b/file3.md
    let project_a = dir.path().join("project_a");
    let project_b = dir.path().join("project_b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();

    std::fs::write(project_a.join("file1.md"), "# File 1").unwrap();
    std::fs::write(project_a.join("file2.md"), "# File 2").unwrap();
    std::fs::write(project_b.join("file3.md"), "# File 3").unwrap();
    std::fs::write(project_b.join("not-md.txt"), "skip me").unwrap();

    let count = temper_cli::commands::status::count_md_files(dir.path());
    assert_eq!(count, 3, "should count all .md files recursively");
}

#[test]
fn test_count_md_files_empty_dir() {
    let dir = TempDir::new().unwrap();
    let count = temper_cli::commands::status::count_md_files(dir.path());
    assert_eq!(count, 0);
}

#[test]
fn test_count_md_files_nonexistent_dir() {
    let count = temper_cli::commands::status::count_md_files(std::path::Path::new("/nonexistent"));
    assert_eq!(count, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test status_test -- --nocapture`
Expected: FAIL — `count_md_files` is not public

- [ ] **Step 3: Make count_md_files public and replace with recursive version**

In `src/commands/status.rs`, replace lines 72-90. Change `fn` to `pub fn` and add recursion:

```rust
pub fn count_md_files(dir: &std::path::Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                count += count_md_files(&path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                count += 1;
            }
        }
    }
    count
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test status_test -- --nocapture`
Expected: PASS

- [ ] **Step 5: Verify with live vault**

Run: `temper status`
Expected: Tickets count should now show the actual number (250+), not 0

- [ ] **Step 6: Commit**

```bash
git add src/commands/status.rs tests/status_test.rs
git commit -m "fix: recursive counting in temper status for nested directories"
```

---

### Task 4: Migrate milestones to project subdirectories

**Files:**
- Modify: `src/commands/milestone.rs` (all path-constructing functions)
- Create: `scripts/migrate-milestones.sh`
- Test: `tests/ticket_test.rs` (existing test assertions for milestone paths need updating)

- [ ] **Step 1: Write failing test for project-scoped milestone paths**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_milestone_creates_in_project_subdir() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.2", None).unwrap();

    // Should be in milestones/myapp/<slug>.md, NOT milestones/<slug>.md
    let expected_path = dir
        .path()
        .join("milestones/myapp")
        .join(format!("{ms_slug}.md"));
    assert!(
        expected_path.exists(),
        "milestone should be in project subdir: {}",
        expected_path.display()
    );

    let flat_path = dir.path().join("milestones").join(format!("{ms_slug}.md"));
    assert!(
        !flat_path.exists(),
        "milestone should NOT be at flat path: {}",
        flat_path.display()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test ticket_test test_milestone_creates_in_project_subdir -- --nocapture`
Expected: FAIL — milestone is at flat path, not project subdir

- [ ] **Step 3: Update milestone create to use project subdir**

In `src/commands/milestone.rs`, update `create()` (around line 108):

```rust
    let dir = config.milestones_dir.join(project);
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Err(TemperError::Vault(format!(
            "milestone already exists: {slug}"
        )));
    }
    // ... (template rendering stays the same) ...
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
```

- [ ] **Step 4: Update ensure_maintenance to use project subdir**

In `src/commands/milestone.rs`, update `ensure_maintenance()` (around line 70):

```rust
    let slug = format!("{project}-maintenance");
    let dir = config.milestones_dir.join(project);
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Ok(slug);
    }
    // ... (template rendering stays the same) ...
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
```

- [ ] **Step 5: Update load_milestones to scan project subdirectories**

Replace `load_milestones` to match the pattern used by `load_tickets`:

```rust
pub fn load_milestones(config: &Config, project: Option<&str>) -> Result<Vec<MilestoneInfo>> {
    let base = &config.milestones_dir;
    if !base.is_dir() {
        return Ok(vec![]);
    }
    let mut milestones = Vec::new();
    let dirs: Vec<_> = if let Some(p) = project {
        let d = base.join(p);
        if d.is_dir() { vec![d] } else { vec![] }
    } else {
        fs::read_dir(base)
            .map_err(|e| TemperError::Vault(e.to_string()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    };
    for dir in dirs {
        for entry in fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
            let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let content = fs::read_to_string(&path)
                .map_err(|e| TemperError::Vault(format!("reading {}: {e}", path.display())))?;
            let fm = match vault::parse_frontmatter(&content) {
                Some(fm) => fm,
                None => continue,
            };
            let info: MilestoneInfo = match serde_yaml::from_value(fm) {
                Ok(i) => i,
                Err(_) => continue,
            };
            milestones.push(info);
        }
    }
    milestones.sort_by_key(|m| m.seq);
    Ok(milestones)
}
```

- [ ] **Step 6: Update `update()` to use project subdir**

In `src/commands/milestone.rs`, update `update()`. The function needs to find the milestone first to get its project:

```rust
pub fn update(config: &Config, slug: &str, status: &str) -> Result<()> {
    let valid_statuses = ["active", "completed", "paused", "cancelled"];
    if !valid_statuses.contains(&status) {
        return Err(TemperError::Vault(format!(
            "invalid status: {status}. Must be one of: {}",
            valid_statuses.join(", ")
        )));
    }
    let info = find_milestone(config, slug)?
        .ok_or_else(|| TemperError::Vault(format!("milestone not found: {slug}")))?;
    let path = config.milestones_dir.join(&info.project).join(format!("{slug}.md"));
    if !path.exists() {
        return Err(TemperError::Vault(format!("milestone not found: {slug}")));
    }
    let content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let updated = vault::set_frontmatter_field(&content, "status", status);
    fs::write(&path, updated).map_err(|e| TemperError::Vault(e.to_string()))?;
    let event = discovery::Event::MilestoneUpdate {
        ts: Local::now().to_rfc3339(),
        project: info.project,
        milestone: slug.to_string(),
        status: status.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    eprintln!("Updated milestone {slug} → {status}");
    Ok(())
}
```

- [ ] **Step 7: Update existing test assertion for milestone path**

In `tests/ticket_test.rs`, update `test_milestone_create_and_ticket_create` (line 10-14):

```rust
    assert!(dir
        .path()
        .join("milestones/myapp")
        .join(format!("{ms_slug}.md"))
        .exists());
```

- [ ] **Step 8: Run all tests**

Run: `cargo test -- --nocapture`
Expected: All tests PASS

- [ ] **Step 9: Create migration script**

Create `scripts/migrate-milestones.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

VAULT="${TEMPER_VAULT:-$(temper status 2>/dev/null | grep 'Root:' | awk '{print $2}')}"
MS_DIR="$VAULT/milestones"

if [ ! -d "$MS_DIR" ]; then
    echo "No milestones directory found at $MS_DIR"
    exit 0
fi

moved=0
skipped=0

for f in "$MS_DIR"/*.md; do
    [ -f "$f" ] || continue

    # Extract project from frontmatter
    project=$(awk '/^---$/{n++; next} n==1 && /^project:/{print $2; exit}' "$f")

    if [ -z "$project" ]; then
        echo "SKIP (no project): $(basename "$f")"
        skipped=$((skipped + 1))
        continue
    fi

    dest="$MS_DIR/$project"
    mkdir -p "$dest"
    mv "$f" "$dest/"
    echo "MOVED: $(basename "$f") → $project/"
    moved=$((moved + 1))
done

echo ""
echo "Done: $moved moved, $skipped skipped"
```

- [ ] **Step 10: Make script executable**

Run: `chmod +x scripts/migrate-milestones.sh`

- [ ] **Step 11: Commit**

```bash
git add src/commands/milestone.rs tests/ticket_test.rs scripts/migrate-milestones.sh
git commit -m "feat: migrate milestones to project subdirectories"
```

---

### Task 5: New command — `temper events`

**Files:**
- Create: `src/commands/events.rs`
- Modify: `src/commands/mod.rs` (add `pub mod events;`)
- Modify: `src/cli.rs` (add Events variant)
- Modify: `src/main.rs` (add dispatch)
- Test: `tests/discovery_test.rs` (extend existing)

- [ ] **Step 1: Write failing test**

Add to `tests/discovery_test.rs`:

```rust
#[test]
fn test_events_list_returns_recent_events() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Create some events via ticket operations
    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Test ticket", Some(&ms_slug), false)
        .unwrap();

    // Load events
    let events = temper_cli::commands::events::load_events(&config, None, 20).unwrap();
    assert!(events.len() >= 2, "should have at least 2 events (milestone create + ticket create)");
}

#[test]
fn test_events_filter_by_project() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    temper_cli::commands::project::add(dir.path(), "other", "/tmp/other", Some("org/other"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms1 = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let ms2 = temper_cli::commands::milestone::create(&config, "other", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Ticket A", Some(&ms1), false).unwrap();
    temper_cli::commands::ticket::create(&config, "other", "Ticket B", Some(&ms2), false).unwrap();

    let myapp_events = temper_cli::commands::events::load_events(&config, Some("myapp"), 20).unwrap();
    for event in &myapp_events {
        let project = temper_cli::commands::events::event_project(event);
        assert_eq!(project, "myapp", "filtered events should only be from myapp");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test discovery_test -- --nocapture`
Expected: FAIL — `events` module doesn't exist

- [ ] **Step 3: Create events command module**

Create `src/commands/events.rs`:

```rust
use crate::config::Config;
use crate::discovery::Event;
use crate::error::{Result, TemperError};

/// Extract the project field from any Event variant.
pub fn event_project(event: &Event) -> &str {
    match event {
        Event::NoteCreate { project, .. }
        | Event::TicketCreate { project, .. }
        | Event::TicketMove { project, .. }
        | Event::TicketDone { project, .. }
        | Event::MilestoneCreate { project, .. }
        | Event::MilestoneUpdate { project, .. } => project,
    }
}

/// Load events from events.jsonl, newest first, with optional project filter and limit.
pub fn load_events(
    config: &Config,
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<Event>> {
    let log_path = config.state_dir.join("events.jsonl");
    if !log_path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&log_path)
        .map_err(|e| TemperError::Vault(format!("reading events.jsonl: {e}")))?;

    let mut events: Vec<Event> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    // Reverse for newest-first
    events.reverse();

    // Filter by project if specified
    if let Some(p) = project {
        events.retain(|e| event_project(e) == p);
    }

    events.truncate(limit);
    Ok(events)
}

/// Format a single event as a human-readable line.
fn format_event(event: &Event) -> String {
    match event {
        Event::NoteCreate {
            ts,
            note_type,
            title,
            project,
            ..
        } => format!("{ts}  {project:<12}  note_create     {note_type}: {title}"),
        Event::TicketCreate {
            ts,
            project,
            ticket,
            title,
            ..
        } => format!("{ts}  {project:<12}  ticket_create   {ticket}: {title}"),
        Event::TicketMove {
            ts,
            project,
            ticket,
            from_stage,
            to_stage,
            ..
        } => format!("{ts}  {project:<12}  ticket_move     {ticket}: {from_stage} → {to_stage}"),
        Event::TicketDone {
            ts,
            project,
            ticket,
            ..
        } => format!("{ts}  {project:<12}  ticket_done     {ticket}"),
        Event::MilestoneCreate {
            ts,
            project,
            milestone,
            title,
        } => format!("{ts}  {project:<12}  ms_create       {milestone}: {title}"),
        Event::MilestoneUpdate {
            ts,
            project,
            milestone,
            status,
        } => format!("{ts}  {project:<12}  ms_update       {milestone} → {status}"),
    }
}

/// Run the events command — print events to stdout.
pub fn run(config: &Config, project: Option<&str>, limit: usize, format: &str) -> Result<()> {
    let events = load_events(config, project, limit)?;

    if events.is_empty() {
        println!("No events found.");
        return Ok(());
    }

    match format {
        "json" => {
            for event in &events {
                println!("{}", serde_json::to_string(event).unwrap_or_default());
            }
        }
        _ => {
            for event in &events {
                println!("{}", format_event(event));
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Register the module**

In `src/commands/mod.rs`, add:
```rust
pub mod events;
```

- [ ] **Step 5: Add CLI definition**

In `src/cli.rs`, add to the `Commands` enum (after `Status`):

```rust
    /// Show recent vault events
    Events {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long, default_value = "text")]
        format: String,
    },
```

- [ ] **Step 6: Add dispatch in main.rs**

In `src/main.rs`, add to the `run()` match (after the Status arm):

```rust
        Commands::Events {
            project,
            limit,
            format,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            let project = project
                .as_deref()
                .or_else(|| resolved.map(|r| r.name.as_str()));
            temper_cli::commands::events::run(&config, project, limit, &format)
        }
```

Note: also add `Commands::Events { .. }` to the import if destructured.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --test discovery_test -- --nocapture`
Expected: All tests PASS

- [ ] **Step 8: Manual smoke test**

Run: `temper events --limit 5`
Expected: Shows 5 most recent events in human-readable format

Run: `temper events --format json --limit 2`
Expected: Shows 2 JSON lines

- [ ] **Step 9: Commit**

```bash
git add src/commands/events.rs src/commands/mod.rs src/cli.rs src/main.rs tests/discovery_test.rs
git commit -m "feat: add temper events command for viewing vault activity"
```

---

### Task 6: New command — `temper warmup`

**Files:**
- Create: `src/commands/warmup.rs`
- Modify: `src/commands/mod.rs` (add `pub mod warmup;`)
- Modify: `src/cli.rs` (add Warmup variant)
- Modify: `src/main.rs` (add dispatch)
- Test: New `tests/warmup_test.rs`

**Depends on:** Task 5 (events command)

- [ ] **Step 1: Write failing test**

Create `tests/warmup_test.rs`:

```rust
use tempfile::TempDir;

#[test]
fn test_warmup_produces_output() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Create some data
    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false)
        .unwrap();

    // warmup should succeed even with minimal data
    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test warmup_test -- --nocapture`
Expected: FAIL — `warmup` module doesn't exist

- [ ] **Step 3: Create warmup command module**

Create `src/commands/warmup.rs`:

```rust
use crate::commands::events;
use crate::commands::session;
use crate::config::Config;
use crate::error::Result;
use crate::vault;

const MAX_SESSION_LINES: usize = 500;

/// Run the warmup command — output a context primer for a new session.
pub fn run(config: &Config, project: Option<&str>, format: &str) -> Result<()> {
    let project_name = project.unwrap_or("general");

    match format {
        "json" => run_json(config, project_name),
        _ => run_text(config, project_name),
    }
}

fn run_text(config: &Config, project: &str) -> Result<()> {
    println!("# Session Context: {project}");
    println!();

    // Section 1: Recent sessions
    println!("## Recent Sessions");
    println!();
    let sessions = collect_recent_sessions(config, project, 3);
    if sessions.is_empty() {
        println!("No recent sessions.");
    } else {
        for (date, title, _path) in &sessions {
            println!("- {date}: {title}");
        }
    }
    println!();

    // Section 2: Last session content
    if let Some((_date, _title, path)) = sessions.first() {
        println!("## Last Session");
        println!();
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                for line in &lines[..MAX_SESSION_LINES] {
                    println!("{line}");
                }
                println!();
                println!("... (truncated at {MAX_SESSION_LINES} lines, see full note at {})", path.display());
            } else {
                print!("{content}");
            }
        }
        println!();
    }

    // Section 3: Recent events
    println!("## Recent Events");
    println!();
    let recent_events = events::load_events(config, Some(project), 15)?;
    if recent_events.is_empty() {
        println!("No recent events.");
    } else {
        for event in &recent_events {
            println!("{}", format_event_brief(event));
        }
    }

    Ok(())
}

fn run_json(config: &Config, project: &str) -> Result<()> {
    let sessions = collect_recent_sessions(config, project, 3);
    let recent_events = events::load_events(config, Some(project), 15)?;

    let last_session_content = sessions.first().and_then(|(_, _, path)| {
        std::fs::read_to_string(path).ok().map(|content| {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                lines[..MAX_SESSION_LINES].join("\n")
            } else {
                content
            }
        })
    });

    let output = serde_json::json!({
        "project": project,
        "recent_sessions": sessions.iter().map(|(date, title, _)| {
            serde_json::json!({"date": date, "title": title})
        }).collect::<Vec<_>>(),
        "last_session_content": last_session_content,
        "recent_events": recent_events.iter().map(|e| {
            serde_json::to_value(e).unwrap_or_default()
        }).collect::<Vec<_>>(),
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
    Ok(())
}

/// Collect recent session files for a project, sorted by date descending.
/// Returns (date, title, path) tuples.
fn collect_recent_sessions(
    config: &Config,
    project: &str,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let sessions_dir = config.sessions_dir.join(project);
    if !sessions_dir.exists() {
        return vec![];
    }

    let mut entries: Vec<(String, String, std::path::PathBuf)> = Vec::new();

    if let Ok(dir) = std::fs::read_dir(&sessions_dir) {
        for entry in dir.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Parse date from filename: "YYYY-MM-DD — Title"
            let date = if stem.len() >= 10 {
                stem[..10].to_string()
            } else {
                "unknown".to_string()
            };

            let title = if let Some(pos) = stem.find(" \u{2014} ") {
                stem[pos + " \u{2014} ".len()..].to_string()
            } else {
                stem.clone()
            };

            entries.push((date, title, path));
        }
    }

    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.truncate(limit);
    entries
}

/// Brief event formatting for warmup output.
fn format_event_brief(event: &crate::discovery::Event) -> String {
    use crate::discovery::Event;
    match event {
        Event::NoteCreate { ts, note_type, title, .. } => {
            let date = &ts[..10];
            format!("  {date}  created {note_type}: {title}")
        }
        Event::TicketCreate { ts, ticket, title, .. } => {
            let date = &ts[..10];
            format!("  {date}  created ticket: {title} ({ticket})")
        }
        Event::TicketMove { ts, ticket, from_stage, to_stage, .. } => {
            let date = &ts[..10];
            format!("  {date}  moved {ticket}: {from_stage} → {to_stage}")
        }
        Event::TicketDone { ts, ticket, .. } => {
            let date = &ts[..10];
            format!("  {date}  completed {ticket}")
        }
        Event::MilestoneCreate { ts, milestone, title, .. } => {
            let date = &ts[..10];
            format!("  {date}  created milestone: {title} ({milestone})")
        }
        Event::MilestoneUpdate { ts, milestone, status, .. } => {
            let date = &ts[..10];
            format!("  {date}  milestone {milestone} → {status}")
        }
    }
}
```

- [ ] **Step 4: Register the module**

In `src/commands/mod.rs`, add:
```rust
pub mod warmup;
```

- [ ] **Step 5: Add CLI definition**

In `src/cli.rs`, add to the `Commands` enum:

```rust
    /// Context primer for new sessions
    Warmup {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
```

- [ ] **Step 6: Add dispatch in main.rs**

In `src/main.rs`, add to the `run()` match:

```rust
        Commands::Warmup { project, format } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
            let project = project
                .as_deref()
                .or_else(|| resolved.map(|r| r.name.as_str()));
            temper_cli::commands::warmup::run(&config, project, &format)
        }
```

- [ ] **Step 7: Run tests**

Run: `cargo test -- --nocapture`
Expected: All tests PASS including new warmup_test

- [ ] **Step 8: Manual smoke test**

Run: `temper warmup --project temper`
Expected: Shows recent sessions, last session content, and recent events for temper project

- [ ] **Step 9: Commit**

```bash
git add src/commands/warmup.rs src/commands/mod.rs src/cli.rs src/main.rs tests/warmup_test.rs
git commit -m "feat: add temper warmup command for session context priming"
```

---

### Task 7: Documentation and final deployment

**Files:**
- Modify: `README.md` (add hook setup docs)

- [ ] **Step 1: Add hook setup documentation to README**

Add a section to the README documenting how to wire up `temper warmup` as a Claude Code hook:

```markdown
## Session Pre-Warming

To automatically prime new Claude Code sessions with recent context, add a `SessionStart` hook
to your project's `.claude/settings.local.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "startup",
      "hooks": [{
        "type": "command",
        "command": "temper warmup --project <your-project>"
      }]
    }]
  }
}
```

This runs `temper warmup` on every new session, injecting:
- Last 3 session summaries
- Full content of the most recent session note
- Last 15 project events (ticket/milestone activity)
```

- [ ] **Step 2: Re-install the skill file**

Run: `temper skill install`
Expected: "Skill installed: ~/.claude/commands/temper.md"

- [ ] **Step 3: Run migration script on live vault**

Run: `scripts/migrate-milestones.sh`
Expected: Milestones moved to project subdirectories, count reported

- [ ] **Step 4: Verify everything works end-to-end**

Run: `temper status`
Expected: Correct counts for tickets, sessions, milestones

Run: `temper events --limit 5`
Expected: Recent events shown

Run: `temper warmup`
Expected: Full context primer output

- [ ] **Step 5: Final commit**

```bash
git add README.md
git commit -m "docs: add session pre-warming hook setup guide"
```

- [ ] **Step 6: Run full test suite**

Run: `cargo test --locked`
Expected: All tests PASS, 0 ignored
