# Bugfixes, Stdin Commands, Ticket Management

**Date:** 2026-03-23
**Ticket:** 2026-03-23-bugfixes-stdin-commands-ticket-management
**Status:** approved

## Overview

A collection of QoL improvements and bug fixes for temper, covering skill template accuracy, status reporting, milestone organization, a new `events` command, a new `warmup` command for session pre-warming, and skill-level shorthand for starting tickets.

## Items

### 1. Skill template: add invocation section

**Problem:** Claude repeatedly tries to run temper via `cargo run`, `python`, or full binary paths instead of calling it directly. The generated skill file at `~/.claude/commands/temper.md` has no guidance on how to invoke the binary.

**Fix:** Add an `## Invocation` section to the template in `src/commands/skill.rs:generate()`, placed between the header and `## Projects`:

```markdown
## Invocation

`temper` is an installed binary in `$PATH`. Always run it directly as `temper <subcommand>`.
Never use `cargo run`, `python`, full binary paths, or any other indirect method — even when
working inside the temper source repo.
```

**Files:** `src/commands/skill.rs`

### 2. Skill template: document --stdin flag

**Problem:** Commands that accept `--stdin` (ticket create, session save, note create) don't show this in the skill command reference. Claude agents don't know to pass the flag when piping content.

**Fix:** Update the command reference lines in the `generate()` template:

```markdown
- `temper ticket create --title <t> --project <p> [--stdin]` — Create ticket (pipe body via stdin)
- `temper session save [<title>] [--stdin]` — Create/update session note (pipe body via stdin)
- `temper note create <type> <title> [--stdin]` — Create note from template (pipe body via stdin)
```

**Files:** `src/commands/skill.rs`

### 3. Add `brainstorm` as a valid ticket stage

**Problem:** The superpowers workflow uses brainstorm as a phase (brainstorm → design → plan → implement → finish), but the valid ticket stages in `ticket.rs` are `[backlog, design, plan, implement, done]`. Starting a ticket with the brainstorming skill requires a `brainstorm` stage.

**Fix:** Add `"brainstorm"` to the `valid_stages` array in `ticket.rs:move_ticket()`. The stage ordering becomes: `backlog → brainstorm → design → plan → implement → done`.

**Files:** `src/commands/ticket.rs`

### 3a. Skill template: ticket start shorthand

**Problem:** No direct way to move a ticket to brainstorm and invoke the brainstorming skill in one step from Claude.

**Fix:** Add to the skill template's command reference:

```markdown
- `temper ticket start <slug> --project <p>` — Move to brainstorm, show content, then invoke brainstorming skill
```

And add to the workflow section:

```markdown
When the user says `/temper ticket start <slug>`:
1. Run `temper ticket move <slug> --stage brainstorm --project <p>`
2. Run `temper ticket show <slug>`
3. Invoke the brainstorming skill with the ticket content as context
```

This is skill-level composition only — no new CLI subcommand. The CLI stays clean; `ticket move` + `ticket show` remain the primitives.

**Files:** `src/commands/skill.rs`

### 4. Fix recursive counting in `temper status`

**Problem:** `count_md_files()` in `status.rs` does a flat `read_dir` scan, but tickets and sessions are stored in project subdirectories (`tickets/<project>/`, `sessions/<project>/`). Status reports 0 tickets despite hundreds existing.

**Fix:** Replace `count_md_files()` with a recursive version that walks subdirectories. No new dependencies — a simple recursive function suffices.

```rust
fn count_md_files(dir: &Path) -> usize {
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

**Files:** `src/commands/status.rs`

### 5. Migrate milestones to project subdirectories

**Problem:** Tickets and sessions use `<type>/<project>/<slug>.md` structure, but milestones are flat in `milestones/<slug>.md`. This is inconsistent and means project-scoped queries require frontmatter parsing instead of directory filtering.

**Fix — runtime:** Update milestone commands to use `milestones/<project>/` subdirectory structure, matching tickets and sessions. Functions requiring path updates:
- `create` — writes to `milestones_dir.join(format!("{slug}.md"))`, needs project subdir
- `load_milestones` — flat `read_dir`, needs recursive or project-scoped scanning
- `find_milestone` — delegates to `load_milestones`, inherits the fix
- `ensure_maintenance` — writes to flat path, needs project subdir
- `update` — reads from flat path, needs project subdir

**Fix — migration:** A `scripts/migrate-milestones.sh` script that:
1. Parses each milestone's frontmatter as raw YAML (not through `MilestoneInfo` struct) to handle any files with missing fields
2. Reads the `project` field if present
3. Falls back to cross-referencing ticket slugs if no project field
4. Moves files into `milestones/<project>/` subdirectories
5. Reports any files it couldn't classify for manual review

**Files:** `src/commands/milestone.rs`, `scripts/migrate-milestones.sh`

### 6. New command: `temper events`

**Problem:** No way to view recent vault activity from `.temper/events.jsonl` directly.

**Subcommand:**

```
temper events [--project <p>] [--limit <n>] [--format json|text]
```

- Reads `.temper/events.jsonl` from the vault state directory
- Default limit: 20
- Default format: human-readable text (timestamp, event type, summary)
- `--project` filters to events matching that project
- `--format json` outputs raw JSONL records
- Malformed JSONL lines are silently skipped (append-only file may have partial writes)

**Files:** New `src/commands/events.rs`, updates to `src/cli.rs`, `src/main.rs`

### 7. New command: `temper warmup`

**Problem:** New Claude sessions start with no context about recent work. There's no automated way to prime a session with recent activity.

**Subcommand:**

```
temper warmup [--project <p>] [--format json|text]
```

Outputs a structured context primer combining:
1. **Recent sessions** (last 3): title, date, project — from session list logic
2. **Last session content**: full body of the most recent session note for the project
3. **Recent events** (last ~15): from the `events` logic above

Default output is markdown. `--project` defaults to current directory inference; if inference fails, falls back to `"general"` (matching `session save` behavior).

If the most recent session note exceeds ~500 lines, truncate with a note pointing to the full file path.

**Hook documentation:** The README (or a setup/configuration section) documents how to wire warmup as a Claude Code `SessionStart` hook. Using the `startup` matcher ensures it only fires on new sessions (not resumes, which already have context):

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "startup",
      "hooks": [{
        "type": "command",
        "command": "temper warmup --project <p>"
      }]
    }]
  }
}
```

Temper does not write this config — the user sets it up per-project in their `settings.local.json`.

**Files:** New `src/commands/warmup.rs`, updates to `src/cli.rs`, `src/main.rs`, README update

## What's not changing

- No new CLI subcommand for `ticket start` — skill composition only
- No auto-writing `settings.local.json` — user wires hooks themselves
- No new crate dependencies — recursive dir walk is hand-rolled
- Existing `--stdin` behavior unchanged — just documenting it in the skill

## Implementation order

1. Skill template fixes (items 1-3) — lowest risk, immediate value
2. Recursive counting fix (item 4) — bug fix, no API changes
3. Milestone migration (item 5) — script + command updates
4. `temper events` (item 6) — new command, independent
5. `temper warmup` (item 7) — builds on events, depends on item 6
6. Re-run `temper skill install` to deploy updated template
