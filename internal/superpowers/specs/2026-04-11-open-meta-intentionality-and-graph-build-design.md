# Open Meta Intentionality + `temper graph build` — Design Spec

**Date:** 2026-04-11
**Task:** `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships`
**Related specs:**
- `2026-04-11-sync-metadata-only-patch-design.md` (prerequisite)
- `2026-04-11-knowledge-graph-ui-design.md` (downstream consumer)
**Depends on:** Sync metadata-only PATCH (Spec 1) — without it, writing
relationship frontmatter across the vault triggers full re-ingest on sync
**Mode:** build
**Effort:** large (multi-session)

---

## Problem

Two intertwined problems:

1. **open_meta is a grab-bag.** Relationship fields (`relates_to`,
   `depends_on`, `extends`) land in open_meta because they're not
   `temper-*` prefixed. But they're not truly arbitrary — temper extracts
   edges from them, the seeder writes them, and downstream consumers
   (search filtering, doctor validation) need to query them. There's no
   formal distinction between "open fields temper knows about" and "truly
   arbitrary user fields."

2. **The vault has no relationships.** The knowledge graph infrastructure
   (R7 Phases 1-4) is built — schema, edge extraction, graph traversal,
   combined search — but the vault files have almost no relationship
   frontmatter. Without seeding, the graph is empty and the graph UI has
   nothing to display.

## Part A: Known Open Fields Registry

### Design

A `KNOWN_OPEN_FIELDS` constant in `temper-core` that enumerates the field
names temper recognizes in open_meta, with expected types and aliases.

```rust
pub struct KnownOpenField {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub expected_type: OpenFieldType,
    pub category: FieldCategory,
}

pub enum OpenFieldType {
    StringList,  // Vec<String> — relates_to, depends_on, etc.
    String,      // Single string — parent
    Tags,        // Vec<String> with Obsidian tag semantics
}

pub enum FieldCategory {
    Relationship,  // Drives edge extraction
    Metadata,      // Obsidian-compatible universal fields
}
```

### Field Inventory

**Relationship fields** (drive edge extraction via `ResourceRelationships`):

| Canonical Name | Aliases | Type | Edge Type |
|---------------|---------|------|-----------|
| `relates_to` | `relates-to` | StringList | RelatesTo |
| `depends_on` | `depends-on` | StringList | DependsOn |
| `extends` | — | StringList | Extends |
| `references` | — | StringList | References |
| `preceded_by` | `preceded-by` | StringList | PrecededBy |
| `derived_from` | `derived-from` | StringList | DerivedFrom |
| `parent` | — | String | ParentOf (reversed) |

**Metadata fields** (Obsidian-compatible universals):

| Canonical Name | Aliases | Type | Notes |
|---------------|---------|------|-------|
| `tags` | — | Tags | Obsidian native tag type |
| `aliases` | — | StringList | Obsidian native alternative names |
| `date` | — | String | YYYY-MM-DD, Obsidian native date type |

### Naming Convention

- **Canonical form uses underscores**: `relates_to`, `depends_on`,
  `derived_from`. This matches the existing `ResourceRelationships` struct
  fields and all deployed code.
- **Hyphenated aliases are accepted**: `relates-to`, `depends-on`,
  `derived-from`. Normalized to canonical form at parse boundaries.
- **Inconsistency with `temper-*` hyphens is acceptable**: the `temper-`
  prefix signals the namespace difference. System fields use hyphens,
  user-owned open fields use underscores.

### Integration Points

1. **`extract_declarations_from_open_meta()`** in `edge_service.rs` —
   already scans keys; add alias normalization (hyphen → underscore) before
   matching. Trivial change.
2. **`temper doctor`** — warn on misspelled relationship fields (e.g.,
   `dependson`, `relates-with`). Suggest canonical form. Not an error.
3. **`temper graph build`** — writes canonical underscore form.
4. **`open_meta` field filtering in search** (task
   `2026-04-11-open-meta-field-filtering-in-search`) — the registry
   provides the set of meaningful field names for `meta_filters`.

### What This Does NOT Do

- Does not change `split_frontmatter_tiers()` routing — known open fields
  still land in `open_meta`, not `managed_meta`. They remain user-editable.
- Does not validate values (e.g., checking that a `depends_on` target
  actually exists) — that's edge resolution at ingest time, already built.
- Does not restrict what can go in `open_meta` — unknown fields are
  preserved as always. `additionalProperties: true` forever.

---

## Part B: `temper graph build`

### First Principle: Owner Boundary Enforcement

**No relationship may cross an owner boundary.** This is a hard constraint,
not a tunable parameter.

- `@me/` resources can relate to `@me/` resources
- `@team-x/` resources can relate to `@team-x/` resources
- `@me/` to `@team-x/` is forbidden, even if the user has visibility into
  both

Even a title or slug exposed across an owner boundary is a data leak. The
seeder treats owner boundaries as hard walls. Every relationship discovered
in every pass is validated against this constraint before being written.

### Command Interface

```
temper graph build [--context <ctx>] [--dry-run] [--offline]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--context <ctx>` | all contexts | Scope to a single context |
| `--dry-run` | false | Print report without writing files |
| `--offline` | false | Skip heuristic search pass (no API needed) |

### Pipeline — Four Passes

**Pass 1: Structural extraction (deterministic, local)**

Derives relationships from existing temper metadata that implies connections
but isn't expressed as relationship frontmatter:

| Source | Relationship | Example |
|--------|-------------|---------|
| `temper-goal: <slug>` on tasks | `parent` → the goal resource | Task "fix auth" with `temper-goal: temper-cloud` → `parent: temper-cloud` |
| Session `date` near task `temper-updated` | `relates_to` task ↔ session | Session dated 2026-04-11 + task updated same day in same context |

Does NOT create edges for implicit context grouping — context membership
is already expressed through `temper-context`.

All candidates validated against owner boundary before proceeding.

**Pass 2: Content scanning (deterministic, local)**

Parses the markdown body for explicit references:

| Pattern | Relationship | Resolution |
|---------|-------------|------------|
| `[text](path)` markdown links | `references` | Resolve path relative to vault root |
| `[[slug]]` or `[[slug\|display]]` wikilinks | `references` | Match against known slug set within same owner |
| Bare UUIDs matching a `temper-id` | `references` | Direct UUID lookup within same owner |
| Words matching known slugs (exact match) | `references` | Match against slug set within same owner; body text only, not frontmatter |

Slug matching requires care to avoid false positives. Only match against
the known slug set for the same owner. Do not do fuzzy or substring
matching — exact slug matches only.

All candidates validated against owner boundary.

**Pass 3: Heuristic discovery (API-dependent, skipped with `--offline`)**

For each resource:
1. Call `temper search` with the resource's title as query, scoped to the
   same context and owner
2. Top-N results (N=5 initially) above a score threshold (0.6 initially)
   become `relates_to` candidates
3. Deduplicate against relationships already discovered in passes 1-2
4. Validate against owner boundary (search results from
   `resources_visible_to()` may include cross-owner resources the profile
   can see but the resource cannot reference)

The score threshold and top-N are hardcoded defaults. Can be promoted to
CLI flags later if tuning is needed.

**Pass 4: Merge + Write**

For each resource with new relationships discovered:

1. Read the vault file
2. Parse existing frontmatter
3. Read existing relationship fields (both underscore and hyphen forms,
   normalized to canonical)
4. Merge: union new discoveries with existing values, deduplicate by
   resolved target (same slug or UUID = same target)
5. Write updated frontmatter back to the file using canonical underscore
   form for field names
6. Preserve all other frontmatter fields and body content untouched
7. Track changes for the report

**Idempotency**: Re-running produces the same result. Existing
relationships are not duplicated. If a user manually removed a relationship,
the seeder will re-add it on the next run — this is intentional for
deterministic passes (structural connections are real). No suppression
mechanism in v1.

### Report Output

```
temper graph build — 847 resources scanned (owner: @me)

Pass 1 (structural):  142 relationships found (89 new)
Pass 2 (content):      67 relationships found (52 new)
Pass 3 (heuristic):   203 relationships found (118 new)
                       ---
Files modified:        194
Relationships added:   259
Already present:       153

Modified files:
  @me/temper/task/2026-04-10-knowledge-graph-foundations.md  (+3 relates_to)
  @me/temper/research/2026-04-01-r7-vertex-edge...md        (+2 references)
  ...
```

With `--dry-run`, same output but "Would modify" instead of "Modified."

### Implementation Location

The command lives in `temper-cli`:
- Command definition: `crates/temper-cli/src/commands/graph.rs` (new
  subcommand group: `temper graph build`, leaving room for future
  `temper graph show`, etc.)
- Pipeline logic: `crates/temper-cli/src/actions/graph_build.rs`
- Vault file reading/writing: uses existing `temper-core` vault and
  frontmatter utilities

### After Build

After `temper graph build` modifies vault files, the user runs
`temper sync run` to push changes to the server. With the metadata-only
PATCH (Spec 1), only `open_hash` differs — the sync uses the PATCH path,
no re-chunking or re-embedding.

The server's ingest pipeline already calls `reconcile_edges()` on metadata
updates (as specified in Spec 1), so edges in `kb_resource_edges` are
updated from the new frontmatter automatically.

## Scope Boundary

- **In scope**: Known fields registry, alias normalization, deterministic +
  heuristic seeding, owner boundary enforcement, dry-run, offline mode
- **Out of scope**: LLM-inferred relationships (tier 3 — separate future
  command, likely `temper graph infer`, intentionally limited to local
  Ollama models), suppression mechanism, cross-owner relationship
  allowlisting
- **Adjacent**: `2026-04-11-open-meta-field-filtering-in-search` benefits
  from the known fields registry but is a separate implementation task

## Testing Strategy

### Known Fields Registry
- Unit tests: alias normalization (hyphen → underscore) for all fields
- Unit tests: category and type classification

### Graph Build Pipeline
- **Unit tests**: each pass in isolation — structural extraction, content
  scanning (wikilinks, markdown links, UUID/slug matching)
- **Unit tests**: owner boundary validation rejects cross-owner candidates
- **Unit tests**: merge deduplication preserves existing, adds new, doesn't
  duplicate
- **Integration tests**: full pipeline against a test vault with known
  structure — verify correct relationships discovered and written
- **E2E test**: `temper graph build --dry-run` on a seeded vault, verify
  report accuracy; then run without `--dry-run`, verify files modified,
  `temper sync run` succeeds with metadata-only PATCH

## Dependencies

- Sync metadata-only PATCH (Spec 1) — required for practical operation
  at scale
- R7 Phases 1-4 (done) — edge extraction, graph traversal, combined search
- Frontmatter schemas (done) — field definitions and tier model

## Estimated Effort

Large — multi-session:
1. Known fields registry + alias normalization (~1 session)
2. Graph build command scaffold + Pass 1-2 (~1-2 sessions)
3. Pass 3 (heuristic search) + Pass 4 (merge/write) (~1 session)
4. Owner boundary enforcement + testing (~1 session)
