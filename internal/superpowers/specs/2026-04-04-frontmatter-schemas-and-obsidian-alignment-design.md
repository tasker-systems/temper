# Frontmatter Schemas & Obsidian Alignment — Design Spec

**Task:** `2026-04-03-enhance-and-fix-temper-init` (workstream e)
**Date:** 2026-04-04
**Mode:** build / large (multi-session)
**Depends on:** Workstreams A–C (completed), session-show-command (completed)
**Blocks:** Workstream d (temper doctor / doctor fix), knowledge graph edge extraction, manifest three-tier hashing

## Overview

Define a formal frontmatter schema for all six temper doctypes, aligned with Obsidian's property system, and establish the ownership model that governs which fields temper manages, which it shares, and which it preserves untouched. This spec is the **data contract** — upstream consumers (doctor, sync, ingest, knowledge graph) adopt fields as they become ready.

## Design Principles

1. **temper-id (UUID) is the identity; path is just location.** All sync, graph edges, and cross-references resolve to UUIDs.
2. **Flat YAML only.** Obsidian's Properties UI does not support nested YAML — nested objects render as uneditable JSON strings. The community consensus is flat keys with prefixes.
3. **Three-tier field ownership.** Every frontmatter field belongs to exactly one tier, and each tier has different governance rules.
4. **Schemas are the data contract.** They define what correct frontmatter looks like. Doctor validates against them. Implementation consumers adopt fields when ready.
5. **additionalProperties: true always.** User fields are never rejected or removed.

## Research Findings: Obsidian Compatibility

### Property Type Support

Obsidian supports exactly 7 property types: text, list, number, checkbox, date, date & time, tags. There is no object/map type. A property's type is inferred on first use and globally consistent across the vault.

### Nested YAML

Obsidian does NOT support nested YAML in its Properties UI. Nested objects display as "unknown" type with no editor support, serialized to ugly JSON strings. Users must switch to Source mode to view or edit. This is a heavily requested feature (283+ forum likes) with no official timeline. A beta community plugin exists but is not in the official registry.

### Reserved Property Names

- `tags` — list type, used for tagging
- `aliases` — list type, alternative names for the note
- `cssclasses` — list type, applies CSS classes to the note
- `publish`, `permalink`, `description`, `image`, `cover` — Obsidian Publish reserved

### Community Conventions

Flat with prefixes is the dominant pattern. Dot-notation keys (e.g., `book.title`) break Dataview compatibility. Underscore or hyphen prefixing (e.g., `temper-id`, `temper_status`) is the most compatible approach — works with Properties UI, Dataview, Bases, and all community plugins.

### Sources

- https://help.obsidian.md/Editing+and+formatting/Properties
- https://help.obsidian.md/Plugins/Templates
- https://forum.obsidian.md/t/properties-bases-support-multi-level-yaml-mapping-of-mappings-nested-attributes-nested-properties/63826

## Section 1: Three-Tier Field Ownership

### Tier 1: Temper System Fields (`temper-*` prefixed)

Governed exclusively by temper. Doctor enforces schema constraints (type, required/optional, allowed enum values). Sync tracks changes via `meta_hash`. No other tool should write these.

### Tier 2: Universal Fields (unprefixed)

Temper participates — reads, validates format, uses for display and graph edges — but shares ownership. Users and other tools (Obsidian, Dataview) can read and write these freely. Sync tracks changes via `open_hash`. Doctor validates format but does not reject unknown values.

### Tier 3: User Fields (anything else)

Temper preserves — never modifies, never removes, round-trips faithfully through parse/write cycles. Not tracked by any temper hash. Doctor ignores these entirely.

### Ownership Principle

- **`temper-*`**: temper governs — system of record, doctor enforces, sync tracks
- **Unprefixed universals**: temper participates — reads, validates format, shares ownership
- **Everything else**: temper preserves — never modifies, round-trips faithfully

## Section 2: Field Taxonomy

### Temper System Fields

| Field | Type | Applies To | Required? | Allowed Values / Format |
|-------|------|-----------|-----------|------------------------|
| `temper-id` | text | all | yes | UUIDv7 |
| `temper-type` | text | all | yes | `task`, `goal`, `session`, `research`, `decision`, `concept` |
| `temper-context` | text | all | yes | slug-format string |
| `temper-created` | datetime | all | yes | RFC3339 with timezone |
| `temper-updated` | datetime | all | no | RFC3339 with timezone |
| `temper-stage` | text | task | yes | `backlog`, `in-progress`, `done`, `cancelled` |
| `temper-mode` | text | task | no | `plan`, `build` |
| `temper-effort` | text | task | no | `small`, `medium`, `large` |
| `temper-goal` | text | task | no | slug of parent goal |
| `temper-seq` | number | task, goal | no | positive integer |
| `temper-branch` | text | task | no | git branch name |
| `temper-pr` | text | task | no | URL or identifier |
| `temper-status` | text | goal | no | `active`, `completed`, `paused`, `cancelled` |
| `temper-source` | text | all | no | ingestion source path or URL |

### Universal Fields

| Field | Type | Applies To | Required? | Notes |
|-------|------|-----------|-----------|-------|
| `title` | text | all | yes | Display name, Obsidian search |
| `slug` | text | task, goal, research, decision, concept | yes | URL-safe identifier |
| `date` | date | session, research, decision | yes | YYYY-MM-DD, Obsidian native date type |
| `tags` | tags | all | no | Obsidian native tag type |
| `aliases` | list | all | no | Obsidian native alternative names |
| `relates_to` | list | all | no | UUIDs, slugs, or `{context}/{type}/{slug}` paths |
| `depends_on` | list | all | no | same format |
| `extends` | list or text | all | no | same format |
| `references` | list | all | no | UUIDs, slugs, paths, or external URIs |
| `preceded_by` | list or text | all | no | same format |
| `derived_from` | list or text | all | no | same format |

### Legacy Field Migration Map

| Old Field | New Field | Transformation |
|-----------|-----------|---------------|
| `id` | `temper-id` | Direct rename |
| `type` | `temper-type` | Direct rename |
| `doc_type` | `temper-type` | Direct rename |
| `context` | `temper-context` | Direct rename |
| `project` | `temper-context` | Direct rename (session/research used `project`) |
| `ingestion_source` | `temper-source` | Direct rename |
| `created` | `temper-created` | Direct rename (already RFC3339) |
| `updated` | `temper-updated` | Direct rename |
| `stage` | `temper-stage` | Direct rename |
| `mode` | `temper-mode` | Direct rename |
| `effort` | `temper-effort` | Direct rename |
| `goal` | `temper-goal` | Direct rename |
| `seq` | `temper-seq` | Direct rename |
| `branch` | `temper-branch` | Direct rename |
| `pr` | `temper-pr` | Direct rename |
| `status` | `temper-status` | Direct rename |
| `legacy_id` | `temper-legacy-id` | Preserve for history (optional, can drop) |

## Section 3: Per-Doctype Schema Examples

### Task

```yaml
---
temper-id: 019d5616-8e3c-7432-9867-222b36e46ea1
temper-type: task
temper-context: temper
temper-stage: in-progress
temper-mode: build
temper-effort: large
temper-goal: temper-maintenance
temper-seq: 130
temper-branch: jcoletaylor/2026-04-03-enhance-and-fix-temper-init
temper-pr: null
temper-source: /path/to/original.md
temper-created: 2026-04-03T21:23:32.026022-04:00
temper-updated: 2026-04-04T09:09:43.841549-04:00
title: "Enhance and fix temper init"
slug: 2026-04-03-enhance-and-fix-temper-init
tags:
  - cli
  - onboarding
depends_on:
  - 2026-04-02-local-integration-and-e2e-testing
relates_to:
  - temper-maintenance
---
```

### Goal

```yaml
---
temper-id: 019d5038-ce94-7661-8869-8711545e9678
temper-type: goal
temper-context: temper
temper-status: active
temper-seq: 10
temper-source: /path/to/original.md
temper-created: 2026-04-02T22:03:13.543398+00:00
title: "Temper Cloud"
slug: temper-cloud
tags:
  - infrastructure
---
```

### Session

```yaml
---
temper-id: 019d5977-f476-7e41-b4aa-fc4bd2b24426
temper-type: session
temper-context: temper
temper-created: 2026-04-04T12:00:00-04:00
title: "Session show command and skill guidance fixes"
date: 2026-04-04
tags:
  - cli
  - skill
relates_to:
  - 2026-04-03-enhance-and-fix-temper-init
---
```

### Research

```yaml
---
temper-id: 019d503b-25c9-7172-b1b1-356bf9442b68
temper-type: research
temper-context: temper
temper-source: /path/to/original.md
temper-created: 2026-04-02T22:05:47.617879+00:00
title: "R7: Vertex-Edge Knowledge Graph in Native Postgres"
slug: 2026-04-01-r7-vertex-edge-knowledge-graph-native-postgres
date: 2026-04-01
tags:
  - architecture
  - postgres
extends: r2-data-model-and-schema-design
depends_on:
  - r2-data-model-and-schema-design
  - r4-crate-architecture-auth-access-control
references:
  - https://neon.tech/docs/extensions/pgvector
---
```

### Decision

```yaml
---
temper-id: 019d6000-0000-0000-0000-000000000001
temper-type: decision
temper-context: temper
temper-created: 2026-04-04T15:00:00-04:00
title: "Use flat prefixed frontmatter over nested YAML"
slug: 2026-04-04-flat-prefixed-frontmatter
date: 2026-04-04
tags:
  - architecture
  - obsidian
relates_to:
  - 2026-04-01-r7-vertex-edge-knowledge-graph-native-postgres
references:
  - https://help.obsidian.md/Editing+and+formatting/Properties
---
```

### Concept

```yaml
---
temper-id: 019d6000-0000-0000-0000-000000000002
temper-type: concept
temper-context: temper
temper-created: 2026-04-04T15:00:00-04:00
title: "Three-tier frontmatter ownership"
slug: three-tier-frontmatter-ownership
tags:
  - architecture
derived_from:
  - 2026-04-04-flat-prefixed-frontmatter
---
```

## Section 4: JSON Schema Structure

Schemas live at `crates/temper-core/schemas/` and compose via `$ref`:

| File | Purpose |
|------|---------|
| `base.schema.json` | Common required fields (`temper-id`, `temper-type`, `temper-context`, `temper-created`, `title`) plus optional universals (`tags`, `aliases`, relationship fields) |
| `task.schema.json` | Extends base. Adds `temper-stage` (required), `temper-mode`, `temper-effort`, `temper-goal`, `temper-seq`, `temper-branch`, `temper-pr`, `slug` (required) |
| `goal.schema.json` | Extends base. Adds `temper-status`, `temper-seq`, `slug` (required) |
| `session.schema.json` | Extends base. Adds `date` (required) |
| `research.schema.json` | Extends base. Adds `slug` (required), `date` (required) |
| `decision.schema.json` | Extends base. Adds `slug` (required), `date` (required) |
| `concept.schema.json` | Extends base. Adds `slug` (required) |

All schemas specify `additionalProperties: true` — user fields are always preserved.

### Relationship Field Format

Relationship fields (`relates_to`, `depends_on`, `extends`, `references`, `preceded_by`, `derived_from`) accept references in three formats:

1. **UUID (temper-id)** — canonical, direct lookup
2. **Slug** — convenience, resolved within context (or cross-context via `{context}/{type}/{slug}`)
3. **External URI** — for `references` only, stored as resource-level metadata

Resolution happens at ingest time. Unresolvable references are preserved in frontmatter and logged as warnings — they may resolve later when the target document is created or synced.

### Relationship to Knowledge Graph (R7)

The `kb_resource_edges` table (designed in R7) stores edges with `weight FLOAT DEFAULT 1.0` and `metadata JSONB` including provenance. Frontmatter relationship fields are the **declaration surface** — the ingest pipeline extracts them into edges.

Weight is a database-layer concern, not a frontmatter concern. Default weights by provenance:

| Provenance | Default Weight |
|------------|---------------|
| Frontmatter-declared | 1.0 |
| Manual (CLI/API) | 1.0 |
| Inferred (future AI) | 0.5 |

Topological analysis (degree centrality, connection density) provides implicit relevance weighting without requiring users to declare weights in frontmatter. Highly connected resources surface naturally through graph traversal scoring.

## Section 5: Manifest Hash Model (Target Architecture)

### Current State

The manifest tracks one hash pair per resource:

```
content_hash  — SHA-256 of body (frontmatter stripped)
remote_hash   — SHA-256 from server at last sync
```

Frontmatter changes are invisible to sync.

### Target State

The manifest entry expands to three hash pairs, one per ownership tier:

```rust
pub struct ManifestEntry {
    pub path: String,

    // Tier 1: Body content (existing, renamed for clarity)
    pub body_hash: String,
    pub body_remote_hash: String,

    // Tier 2: Temper system fields (new)
    pub meta_hash: String,
    pub meta_remote_hash: String,

    // Tier 3: Open fields (new)
    pub open_hash: String,
    pub open_remote_hash: String,

    pub synced_at: DateTime<Utc>,
    pub state: ManifestEntryState,
    pub mtime_secs: Option<i64>,
}
```

### Hash Computation

- **body_hash**: SHA-256 of content below `---` frontmatter delimiter (unchanged from today)
- **meta_hash**: SHA-256 of all `temper-*` fields, sorted alphabetically by key, serialized as canonical YAML
- **open_hash**: SHA-256 of all non-`temper-*` frontmatter fields, sorted alphabetically by key, serialized as canonical YAML

### Conflict Resolution Strategy (per tier)

| Tier | Strategy | Rationale |
|------|----------|-----------|
| Body | Paragraph-level auto-merge (existing `similar` crate logic) | Content is prose, mergeable |
| Temper meta | Last-write-wins by timestamp | Discrete enum/scalar values, no meaningful merge |
| Open fields | Field-level merge with union semantics for lists, last-write-wins for scalars | Lists like `tags` and `depends_on` merge naturally via union |

### Event Ledger Bridge

For temper meta and open field conflict resolution, timestamps per change are required. The existing `kb_events` table has the right shape:

```sql
CREATE TABLE kb_events (
    id            UUID PRIMARY KEY,
    profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    device_id     VARCHAR(64) NOT NULL,
    kb_context_id UUID REFERENCES kb_contexts(id),
    resource_id   UUID REFERENCES kb_resources(id),
    event_type    VARCHAR(64) NOT NULL,
    payload       JSONB NOT NULL DEFAULT '{}',
    created       TIMESTAMPTZ NOT NULL
);
```

The local `events.jsonl` tracks events per device. The open design question in the migration — *"how to bridge local events.jsonl with cloud kb_events, and whether event IDs should annotate document changes for merge reconciliation"* — is answered by this design:

1. **Local**: when a `temper-*` field or open field changes, append a `frontmatter_update` event to `events.jsonl` with field name, old value, new value, and UTC timestamp
2. **On sync push**: ship unsynced `frontmatter_update` events to `kb_events`
3. **On sync pull**: the server uses the event ledger to determine which device's change wins (latest `created` timestamp per field)

### Implementation Scope

This section describes the **target architecture**. Implementation is phased:

- **This workstream (e)**: Define schemas, implement hash computation in doctor (so it can detect drift)
- **Sync workstream (future)**: Manifest struct expansion, event ledger bridge, conflict resolution logic
- **API workstream (future)**: Server-side support for three-tier hashes, frontmatter-aware sync endpoints

## Section 6: Doctor Commands

### `temper doctor`

Validates every vault file against its doctype schema. Reports:

- Missing required fields (e.g., task without `temper-stage`)
- Invalid enum values (e.g., `temper-stage: active` — not a valid stage)
- Malformed UUIDs or dates
- Unrecognized `temper-*` fields (possible typos)
- Legacy field names that should be migrated
- Hash drift between tiers (if manifest exists)

Output is a structured report: `N files checked, M issues found (X auto-fixable, Y manual)`

### `temper doctor fix`

Applies auto-fixable transformations:

1. Rename legacy fields per migration map (Section 2)
2. Backfill `temper-created` from `date` field if missing (as `{date}T00:00:00Z`)
3. Normalize enum values (e.g., legacy stage names like `brainstorm` → `in-progress`)
4. Generate `temper-id` if missing (new UUIDv7)
5. Preserve all user fields and body content

Reports what it fixed and what requires manual intervention.

### Rollout Constraint

**Do NOT run `temper doctor fix` on the local kb-vault until:**

1. The API is updated to understand the new `temper-*` field names
2. The manifest.json migration strategy is defined and implemented
3. The sync protocol can handle the field rename without treating every file as a conflict

Running doctor fix prematurely would rename all local frontmatter fields while the server still expects the old names, causing sync to see every file as locally modified with no remote counterpart for the new field structure.

The safe sequence is:

1. Ship schemas and doctor (read-only validation) — **this workstream**
2. Update API to accept both old and new field names — **API workstream**
3. Expand manifest to three-tier hashes — **sync workstream**
4. Run `temper doctor fix` on the vault — **coordinated migration**
5. Sync the migrated vault — old names are gone, new names are canonical

## Section 7: Schema File Location & Integration

### Location

`crates/temper-core/schemas/` — JSON Schema files:

```
schemas/
  base.schema.json
  task.schema.json
  goal.schema.json
  session.schema.json
  research.schema.json
  decision.schema.json
  concept.schema.json
```

### Integration Points

- **Doctor**: schemas compiled into binary via `include_str!`, validated using a JSON Schema library (e.g., `jsonschema` crate)
- **OpenAPI**: schemas referenced in utoipa spec generation for API consumers
- **Skill**: schemas inform agent guidance about what fields are expected
- **Templates (Askama)**: updated to emit `temper-*` field names (future workstream, after API readiness)

## Future Workstreams Unblocked

| ID | Title | Dependency on This Spec |
|----|-------|------------------------|
| d | `temper doctor` / `temper doctor fix` | Schemas define what doctor validates against |
| R7 | Knowledge graph edge extraction | Relationship field format is now defined |
| sync | Three-tier manifest hashing | Hash model and conflict resolution strategy defined |
| h | Rename `doctype` → `type` across surfaces | Subsumed — migration map covers this as `doc_type` → `temper-type` |
| i | `temper move` + sync-by-UUID | `temper-id` as canonical identity is reinforced |
