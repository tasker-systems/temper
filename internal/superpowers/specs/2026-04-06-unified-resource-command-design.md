# Unified Resource Command Design

**Date:** 2026-04-06
**Task:** 2026-04-06-standardize-doc-creation-commands-across-the-board-and-fix-the-research-save-bug
**Context:** temper
**Mode/Effort:** build/medium

## Summary

Replace all per-doctype CLI commands (task, goal, session, research, note) with a single
`temper resource {create,list,show,update}` command. Use jsonschemas from
`crates/temper-core/schemas/` as the source of truth for which flags each doctype supports.
Add `concept` and `decision` as first-class doctypes with templates and create support.
Remove dead config and code.

## CLI Surface — Before and After

### Removed Commands

| Old Command | Replacement |
|---|---|
| `temper task create --title T --context C` | `temper resource create --type task --title T --context C` |
| `temper task list --context C --stage S` | `temper resource list --type task --context C --stage S` |
| `temper task show <slug>` | `temper resource show --type task <slug>` |
| `temper task move <slug> --stage S` | `temper resource update --type task <slug> --stage S` |
| `temper task done <slug> --branch B --pr P` | `temper resource update --type task <slug> --stage done --branch B --pr P` |
| `temper goal create --title T --context C` | `temper resource create --type goal --title T --context C` |
| `temper goal list --context C` | `temper resource list --type goal --context C` |
| `temper goal update <slug> --status S` | `temper resource update --type goal <slug> --status S` |
| `temper session save --title T --context C` | `temper resource create --type session --title T --context C` |
| `temper session list --context C --limit N` | `temper resource list --type session --context C --limit N` |
| `temper session show <slug>` | `temper resource show --type session <slug>` |
| `temper research save --title T --context C` | `temper resource create --type research --title T --context C` |
| `temper note create concept "title"` | `temper resource create --type concept --title T --context C` |

### New Commands

```
temper resource create --type concept --title T --context C
temper resource create --type decision --title T --context C
temper resource list --type concept --context C --limit N
temper resource list --type decision --context C --limit N
temper resource show --type <any> <slug> [--context C]
temper resource update --type <any> <slug> [--context C] [schema-driven flags...]
```

### Unchanged Commands

`search`, `sync`, `doctor`, `auth`, `add`, `pull`, `remove`, `context`, `warmup`,
`skill`, `events`, `init`, `check`, `status`

## `resource create`

```
temper resource create --type <TYPE> --title <TITLE> --context <CTX> [flags...] [< stdin]
```

**Common flags (all types):** `--type`, `--title`, `--context`, `--format`, `--show-template`

**Type-specific create flags (from schemas):**
- task: `--goal`, `--mode`, `--effort`
- goal: `--slug` (override auto-generated slug)
- session: (none beyond common — title defaults to today's date if omitted)
- research: (none beyond common)
- concept: (none beyond common)
- decision: (none beyond common)

**Stdin body:** All types accept piped body content that replaces the template body.

**Internal dispatch:** The `resource create` command delegates to type-specific action
functions because each type has distinct setup logic:
- task: calculates `temper-seq` from sibling tasks under the same goal
- goal: generates slug, ensures maintenance goal exists
- session: defaults title to today's date, infers context from task if `--task` was
  previously how context was inferred (now just require `--context`)
- research/concept/decision: straightforward template render

**File paths:** All types use `config.doc_type_dir(context, type)` which produces
`{vault_root}/{context}/{type}/`. Filename patterns:
- task: `{slug}.md`
- goal: `{slug}.md`
- session: `{date}-{slug}.md`
- research: `{date}-{slug}.md`
- concept: `{slug}.md`
- decision: `{date}-{slug}.md`

## `resource list`

```
temper resource list --type <TYPE> [--context <CTX>] [--limit N] [--format text|json]
```

**Filtering flags (type-specific, schema-derived):**
- task: `--stage`, `--goal`
- goal: `--status`
- Others: no additional filters currently

All types get `--limit` (default 20). All types get `--context` (optional; scans all
configured contexts if omitted). `--type` is required — listing all types at once is
not supported (too noisy, and filtering flags are type-specific).

## `resource show`

```
temper resource show --type <TYPE> <slug> [--context <CTX>] [--format text|json]
```

Finds the resource by slug (with partial/suffix matching as session show does today) and
prints its raw markdown content.

## `resource update`

```
temper resource update --type <TYPE> <slug> [--context <CTX>] [schema-driven flags...]
```

### Schema-Driven Flag Validation

1. Load the embedded jsonschema for the given `--type`
2. Extract the `properties` map from the type-specific schema (not base)
3. Each property key that isn't in the system-managed blocklist becomes a valid flag
4. Flag name = property key with `temper-` prefix stripped (e.g., `temper-stage` -> `--stage`)
5. Validate values against schema constraints (enum values, type checks)
6. Apply via `vault::set_frontmatter_field()` for each validated field

**System-managed fields (not updatable via update):**
`temper-id`, `temper-provisional-id`, `temper-type`, `temper-created`, `temper-updated`,
`temper-source`, `temper-legacy-id`, `slug`

**Base schema fields updatable on all types:**
- `--title` (string)
- `--tags` (array, append)
- `--aliases` (array, append)
- `--relates-to` (array, append — accepts UUIDs, slugs, or context/type/slug paths)
- `--references` (array, append)
- `--depends-on` (array, append)
- `--extends` (string or array)
- `--preceded-by` (string or array)
- `--derived-from` (string or array)

**Type-specific updatable fields:**
- task: `--stage` (enum), `--mode` (enum), `--effort` (enum), `--goal` (string),
  `--seq` (integer), `--branch` (string), `--pr` (string)
- goal: `--status` (enum), `--seq` (integer)
- session/research/concept/decision: base fields only

### Context and Type Mutation

Two special flags handle identity changes:

**`--context-to <new-context>`** — moves the resource to a different context:
1. Find file at `{old-context}/{type}/{filename}`
2. Update `temper-context` field in frontmatter
3. Move file to `{new-context}/{type}/{filename}` (create dir if needed)
4. Update `temper-updated` timestamp

**`--type-to <new-type>`** (requires `--type-from` instead of `--type`):
```
temper resource update <slug> --type-from research --type-to decision --context C
```
1. Find file at `{context}/research/{filename}`
2. Update `temper-type` field in frontmatter
3. Move file to `{context}/decision/{filename}`
4. Validate frontmatter against new type's schema — warn if required fields are missing
5. Update `temper-updated` timestamp

The managed hash excludes identity fields, so sync picks up these changes naturally
as frontmatter modifications.

### Array Field Behavior

Array fields (`--relates-to`, `--references`, `--depends-on`, `--tags`, `--aliases`)
append by default. Repeated flags add multiple entries:

```
temper resource update --type task my-task --relates-to uuid1 --relates-to uuid2
```

## New Doctypes

### Concept

Evergreen named ideas, patterns, or domain terms.

**Schema additions to `concept.schema.json`:**
```json
"date": {
  "type": "string",
  "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
  "description": "Date concept was recorded (YYYY-MM-DD)"
}
```
Add `"date"` to `"required"` array.

**Template (`templates/concept.md`):**
```yaml
---
temper-provisional-id: "{{ id }}"
temper-type: concept
temper-context: "{{ project }}"
date: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
---

# {{ title }}
```

**Path:** `{context}/concept/{slug}.md` (no date prefix — concepts are identified by name)

### Decision

Point-in-time choices with rationale. ADR-like.

**Schema:** already has `slug` + `date` — no changes needed.

**Template (`templates/decision.md`):**
```yaml
---
temper-provisional-id: "{{ id }}"
temper-type: decision
temper-context: "{{ project }}"
date: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
---

# {{ title }}

## Context

## Decision

## Consequences
```

**Path:** `{context}/decision/{date}-{slug}.md`

## Cleanup

### 1. Remove `[sync.auto]` Config

- Delete `SyncAutoConfig` struct from `crates/temper-core/src/types/config.rs`
- Remove `auto` field from `SyncConfig`
- Remove `[sync.auto]` from init template in `crates/temper-cli/src/commands/init.rs`
- Existing configs with the section are silently ignored (serde default behavior)
- Remove related tests

### 2. Remove LEGACY_FIELD_MAP from doctor_fix.rs

- Delete `fix_legacy_fields()` function and the `LEGACY_FIELD_MAP` constant
- Remove callers
- Keep `infer_temper_id` (still needed for files with no ID)

### 3. Remove Stale Template Copies

Delete from `crates/temper-cli/src/templates/`:
- `research.md`
- `task.md`
- `session.md`
- `goal.md`

Keep `crates/temper-cli/src/templates.rs` (Askama struct definitions — needs new structs
for concept and decision templates).

### 4. Remove `temper-legacy-id`

- Remove from `base.schema.json`
- Remove from `KNOWN_TEMPER_FIELDS` in `schema.rs`
- Remove from `LEGACY_FIELDS` mapping
- Remove from `ManagedMeta` struct
- Keep in the system-managed blocklist for `resource update` (so old files with it
  don't get accidentally modified)

### 5. Fix Research Path Bug

Inherently fixed — `research save` is replaced by `resource create --type research`
which uses `config.doc_type_dir(context, "research")` producing the correct
`{context}/research/` path.

### 6. Fix Note Create Path Bug

Same fix — `note create` wrote to `{type}s/` at vault root. Replaced by
`resource create` using `config.doc_type_dir()`.

## Downstream Updates

### Skill Generate/Install

`temper skill generate` and `temper skill install` produce a `reference.md` that
documents CLI commands. This must be updated to reflect the `resource` command surface.
The skill's `SKILL.md` router and `session-lifecycle.md` reference old command names
(`temper task show`, `temper session save`, etc.) — these need updating too.

### temper-ui Docs Page

`packages/temper-ui/src/routes/docs/+page.svelte` — update CLI reference documentation.

### Discovery Events

The existing `Event::NoteCreate`, `Event::TaskCreate`, `Event::TaskMove`,
`Event::TaskDone`, `Event::GoalCreate`, `Event::GoalUpdate` events should be
consolidated to `Event::ResourceCreate` and `Event::ResourceUpdate` with a `doc_type`
field.

## Implementation Notes

- The clap `Commands` enum in `cli.rs` replaces `TaskAction`, `GoalAction`,
  `SessionAction`, `ResearchAction`, `NoteAction` with a single `ResourceAction` enum
- Type-specific flags on `resource update` can use clap's dynamic approach or a flat
  struct with `Option<String>` for each known field — the schema validation happens
  after parsing, not during
- The `resource create` dispatch can use a match on type string to call existing action
  functions (refactored to accept a common `CreateParams` struct)
- Existing tests in `session.rs`, `task.rs`, `goal.rs` action modules still test the
  underlying logic — just need CLI integration tests updated for the new command surface
