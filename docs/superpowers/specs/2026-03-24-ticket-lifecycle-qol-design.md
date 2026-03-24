# Ticket Lifecycle QoL — Design Specification

**Ticket:** `2026-03-23-temper-ticket-lifecycle-qol`
**Date:** 2026-03-24
**Status:** Draft

## Overview

A unified quality-of-life pass across the temper CLI covering lifecycle simplification, flag consistency, stdin ergonomics, global entity IDs, two new commands, and skill documentation improvements. No major architectural changes — these are targeted improvements to existing patterns.

## 1. UUIDv7 Frontmatter IDs

### What

Every vault entity (ticket, session, milestone, note, research note) gets an `id` field in its YAML frontmatter — a UUIDv7 generated at creation time.

### Schema

```yaml
---
id: 019e1a2b-3c4d-7000-8000-abcdef123456
type: ticket
title: "example ticket"
slug: "2026-03-24-example-ticket"
project: "temper"
# ... remaining type-specific fields
---
```

### Rules

- `id` is the first field in frontmatter by convention
- Filenames and directory structure are unchanged — slug-based paths remain the primary lookup
- `id` is used for cross-entity references (e.g., session-ticket links)
- Dependency: add `uuid` crate with `v7` feature

### Backfill

The `normalize` command (Section 8) backfills missing IDs. When a file has a parseable date (from slug `YYYY-MM-DD` prefix or frontmatter `date`/`created` field), the UUIDv7 is constructed from that timestamp to preserve temporal ordering. Otherwise, file mtime is used. The `uuid` crate's `new_v7` accepts a custom timestamp, so this is handled in Rust directly.

## 2. Lifecycle Stage Simplification

### What

Replace the 6 bespoke stages with 4 standard ones.

### Current → New

| Old Stage | New Stage |
|-----------|-----------|
| `backlog` | `backlog` |
| `brainstorm` | `in-progress` |
| `design` | `in-progress` |
| `plan` | `in-progress` |
| `implement` | `in-progress` |
| `done` | `done` |
| *(new)* | `cancelled` |

### Rationale

The superpowers workflow phases (brainstorm → design → plan → implement) are session-local context managed by skills. They are not reliably captured as ticket state transitions and don't convey useful information outside the active session. The ticket stage should reflect coarse workflow state only.

### Changes

- `ticket move --stage` validates against `["backlog", "in-progress", "done", "cancelled"]`
- `ticket start <slug>` moves to `in-progress` (was `brainstorm`)
- `ticket done <slug>` moves to `done` (unchanged behavior)
- Board view groups by the 4 new stages
- Migration of existing tickets handled by `normalize` (Section 8)

## 3. Stdin Auto-Detection

### What

Replace the explicit `--stdin` flag with TTY detection using `std::io::IsTerminal` (stable since Rust 1.70, no external dependency).

### Pattern

```rust
// Before
if stdin_flag { read_from_stdin() }

// After
if !std::io::stdin().is_terminal() { read_from_stdin() }
```

### Scope

Applied to all commands that accept body content:
- `ticket create`
- `note create`
- `session save`
- `research save` (new)

### Backward Compatibility

The `--stdin` flag remains in the CLI definition but becomes a no-op. Existing scripts and skill invocations that pass `--stdin` continue to work without modification. The flag can be removed in a future major version.

## 4. `--show-template` Flag

### What

A new flag on template-based commands that prints the raw template to stdout and exits without creating any file.

### Supported Commands

- `ticket create --show-template`
- `note create --show-template`
- `session save --show-template`
- `research save --show-template`

### Behavior

- Prints the template content (with placeholder frontmatter) to stdout
- Exits with code 0
- No file is created, no event is recorded
- Useful for agents constructing well-formed stdin content and humans learning template structure

## 5. Flag Consistency

### `--project`

Add to all entity-scoped commands that currently lack it:

| Command | Status |
|---------|--------|
| `ticket move` | **add** |
| `ticket done` | **add** |
| `ticket show` | **add** |
| `milestone update` | **add** |

Resolution order (matches existing pattern): explicit `--project` → CWD-based inference → error.

### `--format json|text`

Add to commands that produce structured output but currently lack the flag:

| Command | Status |
|---------|--------|
| `ticket list` | **add** |
| `ticket show` | **add** |
| `ticket board` | **add** |
| `milestone list` | **add** |
| `milestone create` | **add** |
| `session list` | **add** |
| `note create` | **add** |

Default: `text`. For query commands (list, show, board), JSON output emits the entity's frontmatter fields plus body. For create/save commands (ticket create, note create, research save, milestone create), JSON output emits the created entity's frontmatter as JSON to stdout — confirming what was created.

## 6. Session-Ticket Linking

### What

New flags on `session save` to connect sessions with tickets.

### CLI

```
temper session save [title] [--project p] [--ticket <slug>] [--state <state>]
```

### `--ticket <slug>`

Links the session to a ticket by:

1. Adding a `ticket` field to the session's frontmatter containing the ticket's UUIDv7
2. Updating the ticket's frontmatter:
   - `branch` — set to current git branch (from `git rev-parse --abbrev-ref HEAD`)
   - `sessions` — append this session's UUIDv7 to the list
3. If design/plan docs exist in `docs/superpowers/specs/` or `docs/superpowers/plans/` whose filename contains the ticket slug (prefix match, e.g., `2026-03-24-auth-refactor-design.md` matches ticket slug `2026-03-24-auth-refactor`), adds their relative paths to a `docs` list on the ticket frontmatter

### `--state <state>`

Requires `--ticket`. After linking, moves the ticket to the specified stage. Validates against the 4 allowed stages (`backlog`, `in-progress`, `done`, `cancelled`).

Example: `temper session save "Completed auth refactor" --ticket 2026-03-24-auth-refactor --state done`

### Without flags

`session save` behaves exactly as it does today.

## 7. New Command: `temper research save`

### What

A new note type for durable research artifacts — higher fidelity than session notes, organized by project.

### CLI

```
temper research save <title> [--project p] [--format json|text] [--show-template]
```

### Storage

- Path: `research/{project}/YYYY-MM-DD — {title}.md` (same convention as sessions)
- Slug: `YYYY-MM-DD-{slugified-title}` (same convention as tickets)

### Frontmatter

```yaml
---
id: <uuidv7>
type: research
date: YYYY-MM-DD
project: <project>
title: <title>
slug: <slug>
---
```

### Template

A new `research.md` template focused on:
- Topic / question under investigation
- Findings and key takeaways
- Sources and references
- Implications for current work
- Open questions

### Behavior

- Stdin auto-detected via TTY check
- Records a `NoteCreate` event with note type `research`
- Idempotent: if file exists without piped stdin, no-op; with stdin, replaces body and preserves frontmatter (same as session save)

## 8. New Command: `temper normalize`

### What

A vault repair and consistency tool that scans entities and fixes structural drift.

### CLI

```
temper normalize [--project p] [--dry-run] [--fix-slugs]
```

### Repairs

1. **Missing `id`** — backfills UUIDv7 using file date for timestamp when available, mtime otherwise
2. **Missing/malformed frontmatter** — adds required fields using template defaults (e.g., missing `project`, `type`, `slug`)
3. **Slug consistency** — re-slugifies the title and compares against filename; reports mismatches. With `--fix-slugs`, renames files to match the canonical slug
4. **Wrong directory** — if frontmatter `project` doesn't match the file's directory, moves file to `{entity_type}/{project}/`
5. **Stage migration** — maps old stages (`brainstorm`, `design`, `plan`, `implement`) to `in-progress`
6. **Stale directory cleanup** — removes empty directories left behind by file moves

### Flags

- `--dry-run` — reports what would change without modifying anything
- `--fix-slugs` — enables file renaming for slug mismatches (off by default to avoid surprises)
- `--project` — scopes to one project; without it, normalizes entire vault

### Output

Prints a summary of changes grouped by repair type:
```
Normalize complete:
  3 IDs backfilled
  1 file moved (tickets/general/ → tickets/temper/)
  2 stages migrated (brainstorm → in-progress)
  0 slug mismatches
```

## 9. Skill & Documentation Updates

### Skill Generation Changes

The skill file generated by `temper skill generate` must reflect:

- **New commands:** `research save` and `normalize` added to command list
- **Updated commands:** `session save` with `--ticket` and `--state` flags
- **Stage vocabulary:** Document the 4 stages; clarify that superpowers phases are session-local
- **Stdin:** Remove `--stdin` flag mentions; document auto-detection; mention `--show-template`
- **`ticket start`:** Updated to say "move to in-progress"

### Search & Context Guidance

The skill should actively position `temper search` and `temper context` as first-reach tools for vault discovery:

- **`temper search "<query>"`** — semantic search using embeddings; finds conceptually related content, not just keyword matches. Reach for this before launching subagents to grep/find across the vault.
- **`temper context <topic> --depth N`** — traverses nearest neighbors in the HNSW index to surface related entities. Use this for understanding how a topic connects to other work.
- Include concrete workflow examples: `search → context → targeted reads` rather than jumping to filesystem commands.

### Template

The skill workflow guidance for `ticket start` changes from:
```
1. Run `temper ticket move <slug> --stage brainstorm --project <p>`
```
to:
```
1. Run `temper ticket move <slug> --stage in-progress --project <p>`
```

## Implementation Notes

### Dependencies

- `uuid` crate with `v7` feature (for UUIDv7 generation)
- No other new dependencies — `std::io::IsTerminal` is in std

### Entity Types Affected

| Entity | UUIDv7 | Stdin Auto | `--show-template` | `--format` | `--project` gap |
|--------|:------:|:----------:|:-----------------:|:----------:|:---------------:|
| Ticket | yes | yes (create) | yes (create) | yes (list/show/board) | yes (move/done/show) |
| Session | yes | yes (save) | yes (save) | yes (list) | — |
| Milestone | yes | — | — | yes (list/create) | yes (update) |
| Note | yes | yes (create) | yes (create) | yes (create) | — |
| Research | yes | yes (save) | yes (save) | yes (save) | — |

### Test Strategy

- Unit tests for UUIDv7 generation with custom timestamps
- Unit tests for stage validation (reject old stages, accept new ones)
- Unit tests for TTY detection path (mock stdin as terminal vs pipe)
- Integration tests for normalize (create malformed fixtures, run normalize, verify repairs)
- Existing tests updated for new stage names
- `--show-template` tested by checking stdout output matches template content
- `--format json` tested by parsing output as valid JSON
