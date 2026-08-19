# Enhance and Fix temper init — Design Spec

**Task:** `2026-04-03-enhance-and-fix-temper-init`
**Date:** 2026-04-04
**Mode:** build / large (multi-session)

## Overview

Pre-alpha launch readiness: fix the broken `temper init` config, clean up production data hygiene, and enhance the temper skill with resume/start/create flows. This is session 1 of a multi-session effort covering the first three workstreams.

## Workstream A: Fix temper init config

### Problem

`temper init` generates `~/.config/temper/config.toml` with `[skill] output = "~/.claude/commands/temper.md"`. This path is stale — the skill system now lives at `~/.claude/skills/temper/SKILL.md`. New users who run `temper init` get a broken config out of the gate.

### Changes

**File:** `crates/temper-cli/src/commands/init.rs` — `register_default_config()` function (lines 60-112)

1. Update `[skill] output` from `~/.claude/commands/temper.md` to `~/.claude/skills/temper/SKILL.md`
2. Update `[sync.subscriptions] contexts` to be empty by default (`contexts = []`) with a TOML comment explaining how to add subscriptions
3. Verify `framework = "superpowers"` is still correct (it is)

### Scope

~15 lines changed in one file. No tests needed beyond verifying the generated config content is correct — add a unit test for `register_default_config()` output if one doesn't exist.

## Workstream B: Data Hygiene

### B1: kb_doc_types Migration

**Goal:** Reconcile `kb_doc_types` to the canonical six types that express temper's opinionated taxonomy:

| Type | Purpose |
|------|---------|
| `task` | What we're working on — actionable work items |
| `goal` | What we're working on — directional objectives |
| `session` | What we're working on — daily work records |
| `research` | What matters — investigation and findings |
| `decision` | What matters — choices made and rationale |
| `concept` | What matters — reusable ideas and patterns |

**Current state in seeds:**
- Base seed (20260330000002): `ticket`, `session`, `milestone`, `research`, `board`, `concept`, `source` (7 types)
- Extended seed (20260401000001): `task`, `goal`, `resource` (3 types)
- Production: User has already manually deleted `ticket`, `milestone`, `board`, `source`, `resource` from Neon

**Migration actions (new file: `migrations/20260404000001_consolidate_doc_types.sql`):**

1. **Insert** `decision` with well-known UUID `00000000-0000-0000-0001-00000000000b`
2. **Reclassify** any `kb_resources.doc_type_id` pointing to `resource` type → reassign to `research`
3. **Reclassify** any `kb_resources.doc_type_id` pointing to `ticket`, `milestone`, `board`, `source` → reassign to `research` as safe default
4. **Delete** removed types: `ticket`, `milestone`, `board`, `source`, `resource`
5. All operations use `WHERE EXISTS` / `IF EXISTS` guards — idempotent on both fresh DBs and already-cleaned prod

**Well-known UUIDs (established in seeds):**

| Name | UUID |
|------|------|
| ticket | `00000000-0000-0000-0001-000000000001` |
| session | `00000000-0000-0000-0001-000000000002` |
| milestone | `00000000-0000-0000-0001-000000000003` |
| research | `00000000-0000-0000-0001-000000000004` |
| board | `00000000-0000-0000-0001-000000000005` |
| concept | `00000000-0000-0000-0001-000000000006` |
| source | `00000000-0000-0000-0001-000000000007` |
| task | `00000000-0000-0000-0001-000000000008` |
| goal | `00000000-0000-0000-0001-000000000009` |
| resource | `00000000-0000-0000-0001-00000000000a` |
| decision (new) | `00000000-0000-0000-0001-00000000000b` |

### B2: Production Orphan Audit

Separate from the migration, we need to audit Neon prod for orphaned references:

```sql
-- Find kb_resources whose doc_type_id no longer exists in kb_doc_types
SELECT r.id, r.uri, r.doc_type_id, r.title
FROM kb_resources r
LEFT JOIN kb_doc_types dt ON r.doc_type_id = dt.id
WHERE dt.id IS NULL;
```

Review results together. If orphans exist, reclassify to `research` with targeted UPDATEs.

### B3: Default Context Auto-Provisioning

**Server-side** (`crates/temper-api/src/services/profile_service.rs`):

In `resolve_from_claims()`, after creating a new profile (lines 95-127), also create a `default` context:

```rust
// After profile creation
context_service::create(pool, profile_id, "default").await?;
```

This ensures every new profile starts with at least one usable context.

**Client-side** (`crates/temper-cli`):

When context resolution fails (e.g., the CLI can't match a `--context` flag or infer one), warn and fall back to `default` rather than crashing with a 404-style error. The warning should say something like:

```
⚠ Context "foo" not found. Using "default" context.
  To create this context: temper context create foo
```

This applies to any CLI command that takes `--context`.

## Workstream C: Skill Enhancements

All changes in this workstream are to the temper skill files at `~/.claude/skills/temper/`. No Rust CLI changes needed — the underlying CLI commands already exist.

### C1: `task resume <slug>`

New routing path in SKILL.md under "On Task Resume":

1. `temper task show <slug>` — load task content, extract mode/effort
2. `temper session list --context <ctx>` — find most recent session
3. Read last session's "Next Steps" to understand pickup point
4. Re-read `workflows/{mode}-{effort}.md`
5. Continue from where the last session ended

Add to SKILL.md argument parsing: recognize `task resume <slug>` as a distinct command.

### C2: `session start [--context <ctx>]`

New routing path in SKILL.md under "On Session Start":

1. Load temper skill and context
2. `temper task list --stage in-progress --context <ctx>` — show active tasks
3. Ask: "Working on one of these, or something new?"
4. If existing task: pivot to `task resume <slug>`
5. If new: proceed as open session, save via `temper session save` at end

### C3: Guided `task create [--context <ctx>]`

New routing path in SKILL.md under "On Task Create":

1. If `--context` provided, use it. Otherwise list contexts and ask.
2. Ask for title / problem statement
3. Infer or ask: mode (plan/build), effort (small/medium/large)
4. Ask for goal linkage (`temper goal list --context <ctx>`, or none)
5. Ask for acceptance criteria
6. Run `temper task create` with gathered info
7. Offer to start the task immediately

## Future Workstreams (Not This Session)

These are documented for continuity across sessions:

| ID | Title | Effort |
|----|-------|--------|
| d | Unify normalize/check → `temper doctor` / `temper doctor fix` | 1 session |
| e | Frontmatter JSON schemas + Obsidian alignment | 1-2 sessions |
| f | Auth login → auto-provision profile hook | small |
| g | Interactive `temper init` flow (vault path prompt, config explanation, flags) | medium |
| h | Rename `doctype` → `type` across user-facing surfaces | small |
| i | `temper move` command + sync-by-UUID path reconciliation | medium |

### Key Design Principle for Future Work

**temper-id (UUID) is the identity; path is just location.** Sync reconciliation must key on UUID, not path. Currently, moving a file results in delete + create with a new UUID — this needs to be fixed in workstream (i) so that `temper move` and manual file relocations are detected as moves by the sync protocol. The manifest already keys by UUID (`HashMap<Uuid, ManifestEntry>`), but the vault scanner assigns new UUIDs to files not in `known_paths` rather than matching by temper-id in frontmatter.

## Session Plan

1. Workstream A (fix init config) — implement, test, commit
2. Workstream B1 (migration) — write migration, test locally
3. Workstream B2 (prod audit) — run orphan query against Neon, review, fix
4. Workstream B3 (default context) — server + client changes, test
5. Workstream C (skill enhancements) — update SKILL.md and reference.md, if time permits
