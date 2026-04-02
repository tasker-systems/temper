# R7: Vertex-Edge Knowledge Graph in Native Postgres — Research & Proposal

**Date:** 2026-04-01
**Type:** Research (R-phase) + Implementation Proposal
**Scope:** Knowledge graph modeling via native Postgres vertex-edge tables, recursive CTEs, frontmatter-driven edge extraction, visibility-scoped graph traversal
**Depends on:** R2 (data model — done), R4 (access control — done), I6a (sync infrastructure — done), I5e (local KB restructure — done)
**Blocks:** Graph search mode implementation, knowledge graph visualization, advanced retrieval (combined vector + graph)

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Current State Analysis](#current-state-analysis)
3. [Graph Modeling Approach](#graph-modeling-approach)
4. [Frontmatter Schema Extension](#frontmatter-schema-extension)
5. [Recursive CTE Patterns](#recursive-cte-patterns)
6. [SQL Functions](#sql-functions)
7. [Ingest Pipeline Changes](#ingest-pipeline-changes)
8. [Migration Design](#migration-design)
9. [Interaction with Existing Systems](#interaction-with-existing-systems)
10. [Implementation Plan](#implementation-plan)
11. [Risk Analysis](#risk-analysis)
12. [Open Questions](#open-questions)
13. [Decision Log](#decision-log)
14. [Appendix A: Combined Vector + Graph Scoring Derivation](#appendix-a-combined-vector--graph-scoring-derivation)
15. [Appendix B: Rust Type Stubs](#appendix-b-rust-type-stubs)
16. [Appendix C: Related Tickets & Dependencies](#appendix-c-related-tickets--dependencies)

---

## Problem Statement

Vector similarity search alone is insufficient for knowledge base retrieval. Cosine distance over chunk embeddings finds *semantically similar* content, but semantic similarity is only one axis of relevance. Documents have explicit **structural relationships** — a research doc *extends* a prior investigation, a task *depends_on* another task, an architecture decision *references* five supporting documents — and these relationships represent **human-curated knowledge topology** that embeddings cannot capture.

Consider a concrete scenario: a user searches for "Neon deployment configuration." Semantic search returns the top-5 chunks by cosine distance. But the R3 deployment platform evaluation *extends* R2's data model design, which *depends_on* R1's workflow vision. A developer investigating deployment configuration almost certainly wants to traverse that chain. Today, they must manually follow `kb://` URIs or remember which documents are related. The knowledge graph makes these connections queryable.

Three specific deficiencies motivate this work:

1. **No structural traversal** — The `SearchMode::Graph` variant exists in `temper-core` but is unimplemented. The API stubs it to semantic search. There is no mechanism to answer "what does this document depend on?" or "show me everything connected to this concept within 3 hops."

2. **Frontmatter relationships are invisible** — Users already express relationships in YAML frontmatter (milestone references, project context, related docs). These are parsed for display but never stored in a queryable structure. The relationship information exists but is locked inside markdown files.

3. **Combined retrieval is impossible** — The most powerful retrieval strategy is hybrid: find semantically similar content via pgvector, then expand the result set by traversing graph edges from those seeds. This requires both vector embeddings and graph edges to be queryable in a single Postgres query. Without a graph structure, the vector search results are isolated — semantically relevant but structurally disconnected.

The R3 research phase established the constraint: **Neon does not support Apache AGE**. The R4 design spec confirmed the approach: **adjacency list + recursive CTEs, composing with pgvector in single queries**. R4 also provided the template for visibility-scoped graph traversal (§6, "Composition Patterns — With Graph Traversal"). This document designs the complete implementation.

---

## Current State Analysis

### What Exists

| Component | Status | Location |
|-----------|--------|----------|
| `SearchMode::Graph` enum variant | Defined, unimplemented | `crates/temper-core/src/types/search.rs` (type); `docs/superpowers/plans/2026-03-27-r5-indexing-sync-resource-management.md` (design) |
| `SearchMode` serde support | `"graph"` serializes/deserializes correctly | Tests in R5 plan confirm `serde_json::to_string(&SearchMode::Graph)` → `"graph"` |
| `SearchService::search()` | Stubs Graph mode to semantic (empty results) | `crates/temper-api` via I3 plan |
| `build_frontmatter()` | Emits: `temper-id`, `title`, `context`, `doc_type`, `ingestion_source`, `created` | `crates/temper-cli/src/actions/ingest.rs` L285–301 |
| `resources_visible_to()` | Composable SQL function — returns `(resource_id, access_level, via, team_role)` | `migrations/20260330000001_consolidated_schema.sql` |
| `can_modify_resource()` | Composes `resources_visible_to()` for write checks | Same migration |
| `contexts_visible_to()` | Scoped context listing | Same migration |
| `sync_diff_for_device()` | Composes `resources_visible_to()` inside CTE for sync | Same migration |
| `kb_resources` table | Full resource metadata with `slug`, `origin_uri`, context/doc_type FKs | Same migration |
| `kb_chunks` with HNSW index | 768-dim embeddings, `vector_cosine_ops`, `m=16`, `ef_construction=200` | Same migration |
| `kb_current_chunks` view | Filtered view over `is_current = true` chunks | Same migration |
| R4 graph traversal CTE sketch | Visibility-scoped recursive CTE over `kb_resource_edges` (table referenced but not created) | `specs/2026-03-27-r4-crate-architecture-auth-access-control-design.md` §6 |

### What Does NOT Exist

| Component | Status | Notes |
|-----------|--------|-------|
| `kb_resource_edges` table | **Not in schema** | R4 references it in example SQL but the DDL was never written. Noted as open question: "Graph table not in R2 DDL — add in I-phase or separate migration" |
| Edge type enum | Not defined | R4 CTE uses `e.source_id` / `e.target_id` column names as placeholder |
| Frontmatter relationship fields | Not parsed or stored | `build_frontmatter()` has no `relates_to`, `extends`, `depends_on`, `references` fields |
| Edge extraction in ingest pipeline | Not implemented | `ingest.rs` and `import_cmd.rs` handle frontmatter generation but not relationship extraction |
| Graph traversal SQL functions | Not implemented | R4 provides a sketch; no actual `CREATE FUNCTION` |
| Combined vector + graph search | Not implemented | R4 §6 provides a CTE pattern; not yet a callable function |
| CLI `--mode graph` implementation | Not wired | `SearchMode::Graph` exists but the API and CLI don't route to graph logic |

### Architectural Constraints

1. **Neon hosting** — No Apache AGE, no custom C extensions. Must use native Postgres features: tables, indexes, recursive CTEs, standard functions.
2. **Composable SQL functions** — All graph functions must follow the `LANGUAGE SQL STABLE` pattern established by `resources_visible_to()`. They must be callable inside CTEs by other functions (e.g., `sync_diff_for_device()` composes `resources_visible_to()`).
3. **Visibility-first** — Access control is enforced *before* traversal, not after. A resource the profile cannot see is never traversed through, preventing information leakage via graph structure (R4 §6 design principle).
4. **Single-query composition** — Graph traversal must compose with pgvector similarity search in a single SQL statement. This is the core advantage over AGE: AGE's Cypher queries cannot reference pgvector operators.

---

## Graph Modeling Approach

### Vertex = Existing Resource

There is no need for a separate vertex table. Each row in `kb_resources` is already a vertex:

- Has a stable UUID primary key (`kb_resources.id`)
- Has metadata (title, slug, context, doc_type)
- Has embeddings via `kb_chunks`
- Has access control via `resources_visible_to()`

Introducing a separate vertex table would duplicate identity, create synchronization problems, and break the composability of existing SQL functions that operate on `kb_resources.id`.

### Edge Table: `kb_resource_edges`

```sql
CREATE TYPE edge_type AS ENUM (
    'relates_to',     -- general bidirectional relationship
    'extends',        -- A extends B (A builds upon B's content)
    'depends_on',     -- A depends on B (A requires B to be complete/valid)
    'references',     -- A references B (citation, mention)
    'parent_of',      -- A is the parent of B (hierarchy: milestone→task, section→subsection)
    'tagged_with',    -- A is tagged with concept B (B may be a tag-resource or another resource)
    'preceded_by',    -- A is preceded by B in a sequence (temporal or logical ordering)
    'derived_from'    -- A was derived from B (B is source material for A)
);

CREATE TABLE kb_resource_edges (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    target_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    edge_type              edge_type NOT NULL,
    weight                 FLOAT NOT NULL DEFAULT 1.0,
    metadata               JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Prevent duplicate edges of the same type between the same resources
    CONSTRAINT uq_resource_edge UNIQUE (source_resource_id, target_resource_id, edge_type),

    -- Prevent self-referencing edges
    CONSTRAINT chk_no_self_edge CHECK (source_resource_id != target_resource_id)
);

-- Forward traversal: given a source, find all targets
CREATE INDEX idx_edges_source ON kb_resource_edges(source_resource_id);

-- Reverse traversal: given a target, find all sources
CREATE INDEX idx_edges_target ON kb_resource_edges(target_resource_id);

-- Type-filtered traversal: find all edges of a specific type from a source
CREATE INDEX idx_edges_source_type ON kb_resource_edges(source_resource_id, edge_type);

-- Type-filtered reverse: find all edges of a specific type pointing to a target
CREATE INDEX idx_edges_target_type ON kb_resource_edges(target_resource_id, edge_type);

-- Profile-scoped queries: find all edges created by a profile
CREATE INDEX idx_edges_created_by ON kb_resource_edges(created_by_profile_id);
```

### Directed vs Bidirectional Edges — Design Decision

**All edges are directed.** The `source_resource_id` → `target_resource_id` arrow has semantic meaning:

| Edge Type | Direction Semantics | Example |
|-----------|-------------------|---------|
| `relates_to` | A relates to B (symmetric in meaning but stored as directed) | R2 relates_to R1 |
| `extends` | A extends B (A builds on B) | R4 extends R2 |
| `depends_on` | A depends on B (A needs B) | I3 depends_on R4 |
| `references` | A references B (A cites B) | Session note references task |
| `parent_of` | A is parent of B | Milestone parent_of task |
| `tagged_with` | A is tagged with B | Research doc tagged_with "postgres" |
| `preceded_by` | A is preceded by B | R4 preceded_by R3 |
| `derived_from` | A is derived from B | Summary derived_from meeting notes |

**Rationale for directed over bidirectional:**

1. **Semantic clarity** — `extends` is not symmetric: if A extends B, B does not extend A. Storing a direction preserves this.
2. **Traversal control** — "Find everything this document depends on" (forward from source) is a different question than "find everything that depends on this document" (reverse from target). Directed edges let you choose.
3. **Symmetric queries when needed** — For `relates_to` or any query that wants both directions, the SQL simply unions forward and reverse traversals. This is a two-index-scan union, which Postgres handles efficiently.
4. **No storage duplication** — The alternative (storing two rows per symmetric edge) doubles storage and creates synchronization problems.

**Handling symmetric edge types:** When a user declares `relates_to: [B]` in document A's frontmatter, we store one directed edge `A → B` with type `relates_to`. The `graph_neighbors()` function queries both directions for `relates_to` edges by default. If document B also declares `relates_to: [A]`, the unique constraint prevents a duplicate — the edge already exists in one direction, and the reverse direction is a separate row that can also be inserted (B → A is a different `(source, target, type)` tuple from A → B).

### Edge Metadata (JSONB)

The `metadata` column supports edge-level properties without schema changes:

```json
{
  "provenance": "frontmatter",
  "confidence": 0.95,
  "extracted_at": "2026-04-01T12:00:00Z",
  "section": "## Dependencies",
  "notes": "User-annotated relationship"
}
```

Standard metadata fields:
- `provenance` — How the edge was created: `"frontmatter"` (extracted from YAML), `"manual"` (user-created via CLI/API), `"inferred"` (future: auto-detected by AI)
- `confidence` — Float 0.0–1.0, used for inferred edges
- `extracted_at` — When the edge was extracted/created
- `section` — If the relationship was declared in a specific document section

### Weight Semantics

Edge weight defaults to `1.0` and is used in combined search scoring. Weight heuristics:

| Provenance | Default Weight | Rationale |
|------------|---------------|-----------|
| Frontmatter-declared | 1.0 | Human-curated, highest signal |
| Manual via CLI/API | 1.0 | Explicit user intent |
| Inferred (future) | 0.5 | Machine-generated, lower confidence |

Weights are **not** distance — higher weight means stronger connection. In graph traversal scoring, weight is multiplicative along the path: a 2-hop path through edges with weights 0.8 and 0.6 has a combined score of 0.48.

---

## Frontmatter Schema Extension

### Current Frontmatter

Generated by `build_frontmatter()` in `crates/temper-cli/src/actions/ingest.rs`:

```yaml
---
temper-id: 019d1d24-2000-7379-8f26-ae4ae87bc5c6
title: "Some Document Title"
context: work
doc_type: research
ingestion_source: "/path/to/source.pdf"
created: 2026-04-01T12:00:00+00:00
---
```

### Extended Frontmatter

```yaml
---
temper-id: 019d1d24-2000-7379-8f26-ae4ae87bc5c6
title: "R4: Crate Architecture & Auth Design"
context: temper
doc_type: research

# ─── Relationship Fields (new) ───────────────────────────────────────────────
relates_to:
  - 019d1d24-1000-7379-8f26-ae4ae87bc5c6           # by temper-id (UUID)
  - r2-data-model-and-schema-design                  # by slug
extends: r2-data-model-and-schema-design              # single value or list
depends_on:
  - r2-data-model-and-schema-design
  - r3-deployment-platform-evaluation
references:
  - 019d1d24-3000-7379-8f26-ae4ae87bc5c6
  - https://neon.tech/docs/extensions/pgvector        # external URI (stored as metadata, not as edge)
tags:
  - architecture
  - postgres
  - access-control
# ─────────────────────────────────────────────────────────────────────────────

ingestion_source: "/path/to/source.pdf"
created: 2026-04-01T12:00:00+00:00
---
```

### Relationship Field Specification

| Field | Edge Type | Cardinality | Value Format |
|-------|-----------|-------------|-------------|
| `relates_to` | `relates_to` | List | UUID or slug |
| `extends` | `extends` | Single or list | UUID or slug |
| `depends_on` | `depends_on` | List | UUID or slug |
| `references` | `references` | List | UUID, slug, or external URI |
| `tags` | `tagged_with` | List | Tag name string |
| `parent` | `parent_of` (reverse) | Single | UUID or slug |
| `preceded_by` | `preceded_by` | Single or list | UUID or slug |
| `derived_from` | `derived_from` | Single or list | UUID or slug |

### Reference Resolution Rules

References in frontmatter can be specified as:

1. **UUID (temper-id)** — Canonical. Direct lookup against `kb_resources.id`. No ambiguity.
2. **Slug** — Convenience. Resolved via `kb_resources.slug` within the same context. If ambiguous (multiple contexts have the same slug), resolution fails and the edge is deferred with a warning.
3. **External URI** — For `references` only. Not stored as a graph edge (no target resource exists). Stored in the edge's `metadata` JSONB as `{"external_uri": "https://..."}` on a special self-referencing... **No** — external URIs are stored as resource-level metadata, not as edges. If the external content is later imported (creating a `kb_resource`), the edge can be created then.

Resolution happens at **ingest time** — when a document is added (`temper add`), imported (`temper import`), or pulled (`temper pull`). The ingest pipeline parses frontmatter, extracts relationship fields, resolves references, and upserts edges.

### Resolution Algorithm

```
for each relationship field (relates_to, extends, depends_on, ...):
    for each reference value:
        if value is a valid UUID:
            target_id = value
            if target_id exists in kb_resources:
                upsert edge (source=this_resource, target=target_id, type=field_type)
            else:
                log warning: "forward reference to non-existent resource {target_id}"
                store in kb_deferred_edges (or metadata) for later resolution
        else if value looks like a slug:
            candidates = SELECT id FROM kb_resources
                         WHERE slug = value AND kb_context_id = this_resource.kb_context_id
            if exactly one candidate:
                upsert edge (source=this_resource, target=candidate.id, type=field_type)
            else if zero candidates:
                # Try cross-context resolution
                candidates = SELECT id FROM kb_resources WHERE slug = value
                if exactly one:
                    upsert edge with cross-context note in metadata
                else:
                    log warning: "unresolved slug reference: {value}"
                    store as deferred
            else:
                log warning: "ambiguous slug reference: {value} matches {count} resources"
                store as deferred
        else:
            # External URI or unrecognized format — skip for edge creation
            log info: "skipping non-resolvable reference: {value}"
```

### Tag Resolution

Tags are a special case. The `tags` field creates `tagged_with` edges, but the target is not another document — it's a **tag concept**. Two approaches:

**Option A (Recommended): Tags as lightweight resources** — Create a `kb_resources` entry for each tag with `doc_type = "tag"` and `resource_mode = "system"`. This lets tags participate in the graph uniformly. A search for "everything tagged with postgres" is a single-hop graph traversal. Tags are visible to everyone (system-owned).

**Option B: Tags as JSONB on resource** — Store tags as a JSONB array on `kb_resources.metadata` and query with `@>` containment. Simpler but doesn't participate in graph traversal. This is what most knowledge bases do, but it creates a parallel query path that doesn't compose with graph search.

**Decision: Option A for phase 2+, JSONB tag column for phase 1.** Phase 1 focuses on document-to-document edges via `relates_to`, `extends`, `depends_on`, `references`, `parent_of`, `preceded_by`, `derived_from`. Tag-as-resource requires a `doc_type` seed and a tag management surface. Defer to phase 2 but design the schema to support it.

---

## Recursive CTE Patterns

All CTEs below follow the R4 design principle: **visibility is the outermost boundary, computed once, applied at every traversal hop.** The `visible` CTE calls `resources_visible_to()` and all subsequent joins reference it.

### Pattern 1: N-Hop Traversal with Path Tracking

Given a seed resource, find all resources reachable within N hops. Track the full path for cycle detection and provenance.

```sql
WITH RECURSIVE
  visible AS (
    SELECT resource_id FROM resources_visible_to($1)  -- $1 = profile_id
  ),
  traversal AS (
    -- Base case: seed resources (must be visible)
    SELECT
      v.resource_id,
      0 AS depth,
      ARRAY[v.resource_id] AS path,
      NULL::edge_type AS edge_type,
      NULL::UUID AS from_resource_id,
      1.0::FLOAT AS path_weight
    FROM visible v
    WHERE v.resource_id = ANY($2)  -- $2 = seed resource IDs

    UNION ALL

    -- Recursive case: expand one hop
    SELECT
      e.target_resource_id AS resource_id,
      t.depth + 1 AS depth,
      t.path || e.target_resource_id,
      e.edge_type,
      t.resource_id AS from_resource_id,
      t.path_weight * e.weight AS path_weight
    FROM traversal t
    JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
    JOIN visible v ON v.resource_id = e.target_resource_id  -- visibility at every hop
    WHERE t.depth < $3                                       -- $3 = max_depth
      AND NOT e.target_resource_id = ANY(t.path)             -- cycle prevention
      AND ($4 = '{}' OR e.edge_type = ANY($4::edge_type[]))  -- $4 = edge type filter
  )
SELECT DISTINCT ON (resource_id)
  t.resource_id,
  t.depth,
  t.path,
  t.edge_type,
  t.from_resource_id,
  t.path_weight
FROM traversal t
ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC;
```

**Notes:**
- `ARRAY[v.resource_id]` accumulates the full traversal path for cycle detection
- `NOT e.target_resource_id = ANY(t.path)` prevents infinite loops in cyclic graphs
- `DISTINCT ON (resource_id) ... ORDER BY depth ASC` returns the shortest path to each reachable resource
- `path_weight` is multiplicative: weight decays along the path, so directly connected resources score higher
- Edge type filter `$4` allows typed traversal (e.g., only follow `extends` chains)

### Pattern 2: Bidirectional N-Hop Traversal

For symmetric edge types like `relates_to`, or when the user wants "everything connected regardless of direction":

```sql
WITH RECURSIVE
  visible AS (
    SELECT resource_id FROM resources_visible_to($1)
  ),
  traversal AS (
    SELECT
      v.resource_id,
      0 AS depth,
      ARRAY[v.resource_id] AS path,
      NULL::edge_type AS edge_type,
      1.0::FLOAT AS path_weight
    FROM visible v
    WHERE v.resource_id = ANY($2)

    UNION ALL

    -- Forward direction
    SELECT
      e.target_resource_id,
      t.depth + 1,
      t.path || e.target_resource_id,
      e.edge_type,
      t.path_weight * e.weight
    FROM traversal t
    JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
    JOIN visible v ON v.resource_id = e.target_resource_id
    WHERE t.depth < $3
      AND NOT e.target_resource_id = ANY(t.path)
      AND ($4 = '{}' OR e.edge_type = ANY($4::edge_type[]))

    UNION ALL

    -- Reverse direction
    SELECT
      e.source_resource_id,
      t.depth + 1,
      t.path || e.source_resource_id,
      e.edge_type,
      t.path_weight * e.weight
    FROM traversal t
    JOIN kb_resource_edges e ON e.target_resource_id = t.resource_id
    JOIN visible v ON v.resource_id = e.source_resource_id
    WHERE t.depth < $3
      AND NOT e.source_resource_id = ANY(t.path)
      AND ($4 = '{}' OR e.edge_type = ANY($4::edge_type[]))
  )
SELECT DISTINCT ON (resource_id)
  resource_id, depth, path, edge_type, path_weight
FROM traversal
ORDER BY resource_id, depth ASC, path_weight DESC;
```

### Pattern 3: Shortest Path Between Two Resources

```sql
WITH RECURSIVE
  visible AS (
    SELECT resource_id FROM resources_visible_to($1)
  ),
  search AS (
    SELECT
      v.resource_id,
      0 AS depth,
      ARRAY[v.resource_id] AS path,
      1.0::FLOAT AS path_weight,
      (v.resource_id = $3) AS found  -- $3 = target resource ID
    FROM visible v
    WHERE v.resource_id = $2          -- $2 = source resource ID

    UNION ALL

    SELECT
      e.target_resource_id,
      s.depth + 1,
      s.path || e.target_resource_id,
      s.path_weight * e.weight,
      (e.target_resource_id = $3) AS found
    FROM search s
    JOIN kb_resource_edges e ON e.source_resource_id = s.resource_id
    JOIN visible v ON v.resource_id = e.target_resource_id
    WHERE s.depth < $4                -- $4 = max_depth (safety limit)
      AND NOT s.found                 -- stop expanding once found
      AND NOT e.target_resource_id = ANY(s.path)
  )
SELECT path, depth, path_weight
FROM search
WHERE found = true
ORDER BY depth ASC, path_weight DESC
LIMIT 1;
```

**Note:** This is BFS shortest path, not Dijkstra. For knowledge graphs at our expected scale (<100K resources), BFS with a reasonable depth limit (6–8) is sufficient. Postgres recursive CTEs evaluate breadth-first by default.

### Pattern 4: Subgraph Extraction

Given a set of seed resources, extract the complete connected subgraph within the visibility boundary:

```sql
WITH RECURSIVE
  visible AS (
    SELECT resource_id FROM resources_visible_to($1)
  ),
  subgraph AS (
    SELECT v.resource_id, 0 AS depth, ARRAY[v.resource_id] AS path
    FROM visible v
    WHERE v.resource_id = ANY($2)

    UNION ALL

    SELECT e.target_resource_id, sg.depth + 1, sg.path || e.target_resource_id
    FROM subgraph sg
    JOIN kb_resource_edges e ON e.source_resource_id = sg.resource_id
    JOIN visible v ON v.resource_id = e.target_resource_id
    WHERE sg.depth < $3
      AND NOT e.target_resource_id = ANY(sg.path)

    UNION ALL

    SELECT e.source_resource_id, sg.depth + 1, sg.path || e.source_resource_id
    FROM subgraph sg
    JOIN kb_resource_edges e ON e.target_resource_id = sg.resource_id
    JOIN visible v ON v.resource_id = e.source_resource_id
    WHERE sg.depth < $3
      AND NOT e.source_resource_id = ANY(sg.path)
  )
SELECT DISTINCT sg.resource_id, MIN(sg.depth) AS min_depth
FROM subgraph sg
GROUP BY sg.resource_id
ORDER BY min_depth ASC;
```

### Pattern 5: Typed Traversal (Follow Only Specific Edge Types)

This is a specialization of Pattern 1 with the edge type filter active. Example: follow only `extends` chains to find the full lineage of a document.

```sql
-- Find the full "extends" ancestry of a document
WITH RECURSIVE
  visible AS (
    SELECT resource_id FROM resources_visible_to($1)
  ),
  lineage AS (
    SELECT v.resource_id, 0 AS depth, ARRAY[v.resource_id] AS path
    FROM visible v
    WHERE v.resource_id = $2

    UNION ALL

    SELECT e.target_resource_id, l.depth + 1, l.path || e.target_resource_id
    FROM lineage l
    JOIN kb_resource_edges e ON e.source_resource_id = l.resource_id
    JOIN visible v ON v.resource_id = e.target_resource_id
    WHERE l.depth < 10
      AND e.edge_type = 'extends'
      AND NOT e.target_resource_id = ANY(l.path)
  )
SELECT resource_id, depth, path FROM lineage ORDER BY depth;
```

---

## SQL Functions

All functions follow the established pattern: `LANGUAGE SQL STABLE`, composable inside CTEs, visibility-scoped via `resources_visible_to()`.

### `graph_traverse()`

The primary traversal function. Returns all resources reachable from a set of seed resources within N hops, optionally filtered by edge type.

```sql
CREATE FUNCTION graph_traverse(
    p_profile_id  UUID,
    p_seed_ids    UUID[],
    p_max_depth   INT DEFAULT 3,
    p_edge_types  TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id       UUID,
    depth             INT,
    path              UUID[],
    edge_type         edge_type,
    from_resource_id  UUID,
    path_weight       FLOAT
)
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE
      visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
      ),
      traversal AS (
        SELECT
          v.resource_id,
          0 AS depth,
          ARRAY[v.resource_id] AS path,
          NULL::edge_type AS edge_type,
          NULL::UUID AS from_resource_id,
          1.0::FLOAT AS path_weight
        FROM visible v
        WHERE v.resource_id = ANY(p_seed_ids)

        UNION ALL

        SELECT
          e.target_resource_id,
          t.depth + 1,
          t.path || e.target_resource_id,
          e.edge_type,
          t.resource_id,
          t.path_weight * e.weight
        FROM traversal t
        JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
        JOIN visible v ON v.resource_id = e.target_resource_id
        WHERE t.depth < p_max_depth
          AND NOT e.target_resource_id = ANY(t.path)
          AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
      )
    SELECT DISTINCT ON (t.resource_id)
      t.resource_id,
      t.depth,
      t.path,
      t.edge_type,
      t.from_resource_id,
      t.path_weight
    FROM traversal t
    ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC
$$;
```

### `graph_neighbors()`

Immediate neighbors of a resource (1-hop). Simpler and faster than full traversal — no recursion needed.

```sql
CREATE FUNCTION graph_neighbors(
    p_profile_id   UUID,
    p_resource_id  UUID,
    p_direction    VARCHAR DEFAULT 'both',
    p_edge_types   TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id  UUID,
    edge_type    edge_type,
    direction    VARCHAR,
    weight       FLOAT,
    metadata     JSONB
)
LANGUAGE SQL STABLE AS $$
    -- Outgoing edges (source → target)
    SELECT
      e.target_resource_id AS resource_id,
      e.edge_type,
      'outgoing'::VARCHAR AS direction,
      e.weight,
      e.metadata
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id
      AND (p_direction IN ('both', 'outgoing'))
      AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))

    UNION ALL

    -- Incoming edges (target ← source)
    SELECT
      e.source_resource_id AS resource_id,
      e.edge_type,
      'incoming'::VARCHAR AS direction,
      e.weight,
      e.metadata
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id
      AND (p_direction IN ('both', 'incoming'))
      AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
$$;
```

### `combined_search()`

The hybrid search function: find semantically similar chunks via pgvector, then expand the result set by traversing graph edges from the top semantic matches. Returns a unified score that blends vector similarity and graph proximity.

```sql
CREATE FUNCTION combined_search(
    p_profile_id     UUID,
    p_query_embedding vector(768),
    p_seed_ids       UUID[] DEFAULT '{}',
    p_vector_weight  FLOAT DEFAULT 0.7,
    p_graph_weight   FLOAT DEFAULT 0.3,
    p_vector_limit   INT DEFAULT 20,
    p_graph_depth    INT DEFAULT 2,
    p_edge_types     TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id    UUID,
    title          TEXT,
    slug           VARCHAR(256),
    vector_score   FLOAT,
    graph_score    FLOAT,
    combined_score FLOAT,
    origin         VARCHAR(16)   -- 'vector', 'graph', 'both'
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
      SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),

    -- Stage 1: Vector similarity search
    vector_hits AS (
      SELECT
        c.resource_id,
        1.0 - (c.embedding <=> p_query_embedding) AS similarity  -- cosine similarity (1 = identical)
      FROM kb_current_chunks c
      JOIN visible v ON v.resource_id = c.resource_id
      ORDER BY c.embedding <=> p_query_embedding
      LIMIT p_vector_limit
    ),

    -- Deduplicate: best similarity per resource
    vector_resources AS (
      SELECT resource_id, MAX(similarity) AS similarity
      FROM vector_hits
      GROUP BY resource_id
    ),

    -- Stage 2: Graph expansion from vector seeds + explicit seeds
    seed_ids AS (
      SELECT resource_id FROM vector_resources
      UNION
      SELECT unnest(p_seed_ids) AS resource_id
    ),
    graph_hits AS (
      SELECT
        gt.resource_id,
        gt.depth,
        gt.path_weight
      FROM graph_traverse(
        p_profile_id,
        ARRAY(SELECT resource_id FROM seed_ids),
        p_graph_depth,
        p_edge_types
      ) gt
      WHERE gt.depth > 0  -- exclude seeds themselves (they're already in vector_resources)
    ),

    -- Normalize graph scores: path_weight / (depth + 1) so closer = higher
    graph_resources AS (
      SELECT
        resource_id,
        MAX(path_weight / (depth + 1)::FLOAT) AS graph_proximity
      FROM graph_hits
      GROUP BY resource_id
    ),

    -- Stage 3: Combine scores
    combined AS (
      SELECT
        COALESCE(vr.resource_id, gr.resource_id) AS resource_id,
        COALESCE(vr.similarity, 0.0) AS vector_score,
        COALESCE(gr.graph_proximity, 0.0) AS graph_score,
        (p_vector_weight * COALESCE(vr.similarity, 0.0))
          + (p_graph_weight * COALESCE(gr.graph_proximity, 0.0)) AS combined_score,
        CASE
          WHEN vr.resource_id IS NOT NULL AND gr.resource_id IS NOT NULL THEN 'both'
          WHEN vr.resource_id IS NOT NULL THEN 'vector'
          ELSE 'graph'
        END AS origin
      FROM vector_resources vr
      FULL OUTER JOIN graph_resources gr ON gr.resource_id = vr.resource_id
    )

    SELECT
      c.resource_id,
      r.title,
      r.slug,
      c.vector_score::FLOAT,
      c.graph_score::FLOAT,
      c.combined_score::FLOAT,
      c.origin::VARCHAR(16)
    FROM combined c
    JOIN kb_resources r ON r.id = c.resource_id
    ORDER BY c.combined_score DESC
$$;
```

### `graph_resource_edges()`

Utility function for inspecting edges of a specific resource. Used by CLI `temper show` and API detail endpoints.

```sql
CREATE FUNCTION graph_resource_edges(
    p_profile_id   UUID,
    p_resource_id  UUID
) RETURNS TABLE (
    edge_id           UUID,
    peer_resource_id  UUID,
    peer_title        TEXT,
    peer_slug         VARCHAR(256),
    edge_type         edge_type,
    direction         VARCHAR,
    weight            FLOAT,
    metadata          JSONB,
    created           TIMESTAMPTZ
)
LANGUAGE SQL STABLE AS $$
    -- Outgoing
    SELECT
      e.id AS edge_id,
      e.target_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_type,
      'outgoing'::VARCHAR AS direction,
      e.weight,
      e.metadata,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.target_resource_id
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id

    UNION ALL

    -- Incoming
    SELECT
      e.id AS edge_id,
      e.source_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_type,
      'incoming'::VARCHAR AS direction,
      e.weight,
      e.metadata,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.source_resource_id
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id

    ORDER BY edge_type, direction, created
$$;
```

---

## Ingest Pipeline Changes

### Current Ingest Flow

```
User runs `temper add <file>` or `temper import <url>`
  → parse/convert to markdown
  → build_frontmatter() generates YAML header
  → write_vault_file_and_register() writes to vault + manifest
  → POST /api/resources (cloud ingest)
  → Cloud: chunk → embed → store in kb_chunks
```

### Extended Ingest Flow

```
User runs `temper add <file>` or `temper import <url>`
  → parse/convert to markdown
  → build_frontmatter() generates YAML header (now with relationship fields)
  → write_vault_file_and_register() writes to vault + manifest
  → POST /api/resources (cloud ingest)
  → Cloud: chunk → embed → store in kb_chunks
  → NEW: extract_edges_from_frontmatter()
  → NEW: resolve references (UUID / slug lookup)
  → NEW: upsert edges in kb_resource_edges
  → NEW: attempt deferred edge resolution for any edges pointing TO this resource
```

### Edge Extraction Function (Rust)

```rust
/// Extract graph edge declarations from parsed YAML frontmatter.
///
/// Returns a list of (edge_type, target_ref) pairs where target_ref
/// is either a UUID string or a slug string.
pub fn extract_edge_declarations(
    frontmatter: &serde_yaml::Value,
) -> Vec<(EdgeType, TargetRef)> {
    let mut edges = Vec::new();

    let field_mappings: &[(&str, EdgeType)] = &[
        ("relates_to", EdgeType::RelatesTo),
        ("extends", EdgeType::Extends),
        ("depends_on", EdgeType::DependsOn),
        ("references", EdgeType::References),
        ("preceded_by", EdgeType::PrecededBy),
        ("derived_from", EdgeType::DerivedFrom),
        ("parent", EdgeType::ParentOf),  // stored as reverse: parent→child
    ];

    for (field_name, edge_type) in field_mappings {
        if let Some(value) = frontmatter.get(field_name) {
            match value {
                serde_yaml::Value::String(s) => {
                    if let Some(target) = parse_target_ref(s) {
                        edges.push((*edge_type, target));
                    }
                }
                serde_yaml::Value::Sequence(seq) => {
                    for item in seq {
                        if let serde_yaml::Value::String(s) = item {
                            if let Some(target) = parse_target_ref(s) {
                                edges.push((*edge_type, target));
                            }
                        }
                    }
                }
                _ => {} // skip non-string/non-list values
            }
        }
    }

    edges
}

/// Parse a frontmatter reference value into a TargetRef.
fn parse_target_ref(value: &str) -> Option<TargetRef> {
    // Try UUID first
    if let Ok(uuid) = Uuid::parse_str(value) {
        return Some(TargetRef::Id(uuid));
    }
    // Otherwise treat as slug (must be non-empty, no whitespace-only)
    let trimmed = value.trim();
    if !trimmed.is_empty() && !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Some(TargetRef::Slug(trimmed.to_string()));
    }
    // External URIs are not resolvable to edges
    None
}
```

### Edge Upsert Logic

```sql
-- Upsert a single edge (used by ingest pipeline)
INSERT INTO kb_resource_edges (
    source_resource_id, target_resource_id, edge_type,
    weight, metadata, created_by_profile_id
)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT ON CONSTRAINT uq_resource_edge
DO UPDATE SET
    weight = EXCLUDED.weight,
    metadata = kb_resource_edges.metadata || EXCLUDED.metadata,
    updated = now();
```

### Handling Forward References

When document A declares `depends_on: [B-slug]` but B doesn't exist yet (e.g., bulk import where B is imported after A):

**Strategy: Deferred edge table**

```sql
CREATE TABLE kb_deferred_edges (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    target_ref             TEXT NOT NULL,       -- slug or UUID string that couldn't be resolved
    target_context_id      UUID,                -- context hint for slug resolution
    edge_type              edge_type NOT NULL,
    weight                 FLOAT NOT NULL DEFAULT 1.0,
    metadata               JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts               INT NOT NULL DEFAULT 0,
    last_attempt           TIMESTAMPTZ
);

CREATE INDEX idx_deferred_edges_target_ref ON kb_deferred_edges(target_ref);
```

**Resolution trigger:** After every successful resource creation/import, attempt to resolve deferred edges:

```sql
-- Attempt to resolve deferred edges targeting a newly created resource
WITH resolved AS (
    SELECT de.id AS deferred_id,
           de.source_resource_id,
           r.id AS target_resource_id,
           de.edge_type,
           de.weight,
           de.metadata,
           de.created_by_profile_id
    FROM kb_deferred_edges de
    JOIN kb_resources r ON (
        r.id::TEXT = de.target_ref        -- UUID match
        OR (r.slug = de.target_ref        -- slug match
            AND (de.target_context_id IS NULL OR r.kb_context_id = de.target_context_id))
    )
),
inserted AS (
    INSERT INTO kb_resource_edges (
        source_resource_id, target_resource_id, edge_type,
        weight, metadata, created_by_profile_id
    )
    SELECT source_resource_id, target_resource_id, edge_type,
           weight, metadata, created_by_profile_id
    FROM resolved
    ON CONFLICT ON CONSTRAINT uq_resource_edge DO NOTHING
    RETURNING id
)
DELETE FROM kb_deferred_edges
WHERE id IN (SELECT deferred_id FROM resolved);
```

### Edge Diffing on Update

When a document's frontmatter is updated (e.g., user edits the `depends_on` list), the ingest pipeline must diff the old and new edge sets:

```rust
/// Reconcile edges after a frontmatter update.
///
/// - New declarations: upsert edges
/// - Removed declarations: delete edges with provenance="frontmatter"
/// - Unchanged: no-op
pub async fn reconcile_edges(
    pool: &PgPool,
    resource_id: Uuid,
    profile_id: Uuid,
    new_declarations: &[(EdgeType, TargetRef)],
) -> Result<EdgeReconciliation> {
    // 1. Fetch existing frontmatter-provenance edges for this source
    let existing = sqlx::query!(
        r#"SELECT id, target_resource_id, edge_type as "edge_type: EdgeType"
           FROM kb_resource_edges
           WHERE source_resource_id = $1
             AND metadata->>'provenance' = 'frontmatter'"#,
        resource_id
    )
    .fetch_all(pool)
    .await?;

    // 2. Resolve new declarations to target IDs
    let resolved_new = resolve_all_targets(pool, resource_id, new_declarations).await?;

    // 3. Compute diff
    let existing_set: HashSet<(Uuid, EdgeType)> = existing.iter()
        .map(|e| (e.target_resource_id, e.edge_type))
        .collect();
    let new_set: HashSet<(Uuid, EdgeType)> = resolved_new.iter()
        .map(|(target_id, edge_type)| (*target_id, *edge_type))
        .collect();

    let to_add = &new_set - &existing_set;
    let to_remove = &existing_set - &new_set;

    // 4. Execute diff
    for (target_id, edge_type) in &to_add {
        upsert_edge(pool, resource_id, *target_id, *edge_type, profile_id).await?;
    }
    for (target_id, edge_type) in &to_remove {
        delete_frontmatter_edge(pool, resource_id, *target_id, *edge_type).await?;
    }

    Ok(EdgeReconciliation {
        added: to_add.len(),
        removed: to_remove.len(),
        unchanged: existing_set.intersection(&new_set).count(),
    })
}
```

### Batch Edge Creation for Bulk Imports

During `temper import --directory` or initial vault indexing, we may process hundreds of documents. Edge resolution should be batched:

1. **Phase 1**: Import all documents, creating `kb_resources` entries. Skip edge resolution.
2. **Phase 2**: Extract edge declarations from all imported documents' frontmatter.
3. **Phase 3**: Resolve all targets in a single batch query (join slugs against `kb_resources`).
4. **Phase 4**: Batch `INSERT` resolved edges with `ON CONFLICT DO NOTHING`.
5. **Phase 5**: Store any unresolved references in `kb_deferred_edges`.

This avoids N+1 queries during bulk import.

---

## Migration Design

### Migration File

Following the consolidated schema pattern, this would be a new migration file:

```sql
-- =============================================================================
-- R7: Knowledge Graph — Edge Table + Traversal Functions
-- =============================================================================
-- Adds: kb_resource_edges, kb_deferred_edges, edge_type enum,
--        graph_traverse(), graph_neighbors(), combined_search(),
--        graph_resource_edges()

-- ─── Edge Type Enum ──────────────────────────────────────────────────────────

CREATE TYPE edge_type AS ENUM (
    'relates_to',
    'extends',
    'depends_on',
    'references',
    'parent_of',
    'tagged_with',
    'preceded_by',
    'derived_from'
);

-- ─── Edge Table ──────────────────────────────────────────────────────────────

CREATE TABLE kb_resource_edges (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    target_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    edge_type              edge_type NOT NULL,
    weight                 FLOAT NOT NULL DEFAULT 1.0,
    metadata               JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_resource_edge UNIQUE (source_resource_id, target_resource_id, edge_type),
    CONSTRAINT chk_no_self_edge CHECK (source_resource_id != target_resource_id)
);

CREATE INDEX idx_edges_source ON kb_resource_edges(source_resource_id);
CREATE INDEX idx_edges_target ON kb_resource_edges(target_resource_id);
CREATE INDEX idx_edges_source_type ON kb_resource_edges(source_resource_id, edge_type);
CREATE INDEX idx_edges_target_type ON kb_resource_edges(target_resource_id, edge_type);
CREATE INDEX idx_edges_created_by ON kb_resource_edges(created_by_profile_id);

-- ─── Deferred Edges (forward references) ─────────────────────────────────────

CREATE TABLE kb_deferred_edges (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    target_ref             TEXT NOT NULL,
    target_context_id      UUID REFERENCES kb_contexts(id),
    edge_type              edge_type NOT NULL,
    weight                 FLOAT NOT NULL DEFAULT 1.0,
    metadata               JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts               INT NOT NULL DEFAULT 0,
    last_attempt           TIMESTAMPTZ
);

CREATE INDEX idx_deferred_edges_target_ref ON kb_deferred_edges(target_ref);
CREATE INDEX idx_deferred_edges_source ON kb_deferred_edges(source_resource_id);

-- ─── Graph Traversal Functions ───────────────────────────────────────────────

-- N-hop forward traversal with visibility scoping
CREATE FUNCTION graph_traverse(
    p_profile_id  UUID,
    p_seed_ids    UUID[],
    p_max_depth   INT DEFAULT 3,
    p_edge_types  TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id       UUID,
    depth             INT,
    path              UUID[],
    edge_type         edge_type,
    from_resource_id  UUID,
    path_weight       FLOAT
)
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE
      visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
      ),
      traversal AS (
        SELECT
          v.resource_id,
          0 AS depth,
          ARRAY[v.resource_id] AS path,
          NULL::edge_type AS edge_type,
          NULL::UUID AS from_resource_id,
          1.0::FLOAT AS path_weight
        FROM visible v
        WHERE v.resource_id = ANY(p_seed_ids)

        UNION ALL

        SELECT
          e.target_resource_id,
          t.depth + 1,
          t.path || e.target_resource_id,
          e.edge_type,
          t.resource_id,
          t.path_weight * e.weight
        FROM traversal t
        JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
        JOIN visible v ON v.resource_id = e.target_resource_id
        WHERE t.depth < p_max_depth
          AND NOT e.target_resource_id = ANY(t.path)
          AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
      )
    SELECT DISTINCT ON (t.resource_id)
      t.resource_id,
      t.depth,
      t.path,
      t.edge_type,
      t.from_resource_id,
      t.path_weight
    FROM traversal t
    ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC
$$;

-- Immediate neighbors (1-hop, bidirectional)
CREATE FUNCTION graph_neighbors(
    p_profile_id   UUID,
    p_resource_id  UUID,
    p_direction    VARCHAR DEFAULT 'both',
    p_edge_types   TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id  UUID,
    edge_type    edge_type,
    direction    VARCHAR,
    weight       FLOAT,
    metadata     JSONB
)
LANGUAGE SQL STABLE AS $$
    SELECT
      e.target_resource_id AS resource_id,
      e.edge_type,
      'outgoing'::VARCHAR AS direction,
      e.weight,
      e.metadata
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id
      AND (p_direction IN ('both', 'outgoing'))
      AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))

    UNION ALL

    SELECT
      e.source_resource_id AS resource_id,
      e.edge_type,
      'incoming'::VARCHAR AS direction,
      e.weight,
      e.metadata
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id
      AND (p_direction IN ('both', 'incoming'))
      AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
$$;

-- Hybrid vector + graph search
CREATE FUNCTION combined_search(
    p_profile_id      UUID,
    p_query_embedding vector(768),
    p_seed_ids        UUID[] DEFAULT '{}',
    p_vector_weight   FLOAT DEFAULT 0.7,
    p_graph_weight    FLOAT DEFAULT 0.3,
    p_vector_limit    INT DEFAULT 20,
    p_graph_depth     INT DEFAULT 2,
    p_edge_types      TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id    UUID,
    title          TEXT,
    slug           VARCHAR(256),
    vector_score   FLOAT,
    graph_score    FLOAT,
    combined_score FLOAT,
    origin         VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
      SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),
    vector_hits AS (
      SELECT
        c.resource_id,
        1.0 - (c.embedding <=> p_query_embedding) AS similarity
      FROM kb_current_chunks c
      JOIN visible v ON v.resource_id = c.resource_id
      ORDER BY c.embedding <=> p_query_embedding
      LIMIT p_vector_limit
    ),
    vector_resources AS (
      SELECT resource_id, MAX(similarity) AS similarity
      FROM vector_hits
      GROUP BY resource_id
    ),
    seed_ids AS (
      SELECT resource_id FROM vector_resources
      UNION
      SELECT unnest(p_seed_ids) AS resource_id
    ),
    graph_hits AS (
      SELECT
        gt.resource_id,
        gt.depth,
        gt.path_weight
      FROM graph_traverse(
        p_profile_id,
        ARRAY(SELECT resource_id FROM seed_ids),
        p_graph_depth,
        p_edge_types
      ) gt
      WHERE gt.depth > 0
    ),
    graph_resources AS (
      SELECT
        resource_id,
        MAX(path_weight / (depth + 1)::FLOAT) AS graph_proximity
      FROM graph_hits
      GROUP BY resource_id
    ),
    combined AS (
      SELECT
        COALESCE(vr.resource_id, gr.resource_id) AS resource_id,
        COALESCE(vr.similarity, 0.0) AS vector_score,
        COALESCE(gr.graph_proximity, 0.0) AS graph_score,
        (p_vector_weight * COALESCE(vr.similarity, 0.0))
          + (p_graph_weight * COALESCE(gr.graph_proximity, 0.0)) AS combined_score,
        CASE
          WHEN vr.resource_id IS NOT NULL AND gr.resource_id IS NOT NULL THEN 'both'
          WHEN vr.resource_id IS NOT NULL THEN 'vector'
          ELSE 'graph'
        END AS origin
      FROM vector_resources vr
      FULL OUTER JOIN graph_resources gr ON gr.resource_id = vr.resource_id
    )
    SELECT
      c.resource_id,
      r.title,
      r.slug,
      c.vector_score::FLOAT,
      c.graph_score::FLOAT,
      c.combined_score::FLOAT,
      c.origin::VARCHAR(16)
    FROM combined c
    JOIN kb_resources r ON r.id = c.resource_id
    ORDER BY c.combined_score DESC
$$;

-- Edge listing for a specific resource
CREATE FUNCTION graph_resource_edges(
    p_profile_id   UUID,
    p_resource_id  UUID
) RETURNS TABLE (
    edge_id           UUID,
    peer_resource_id  UUID,
    peer_title        TEXT,
    peer_slug         VARCHAR(256),
    edge_type         edge_type,
    direction         VARCHAR,
    weight            FLOAT,
    metadata          JSONB,
    created           TIMESTAMPTZ
)
LANGUAGE SQL STABLE AS $$
    SELECT
      e.id AS edge_id,
      e.target_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_type,
      'outgoing'::VARCHAR AS direction,
      e.weight,
      e.metadata,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.target_resource_id
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id

    UNION ALL

    SELECT
      e.id AS edge_id,
      e.source_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_type,
      'incoming'::VARCHAR AS direction,
      e.weight,
      e.metadata,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.source_resource_id
    JOIN resources_visible_to(p_profile_id) v ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id

    ORDER BY edge_type, direction, created
$$;
```

---

## Interaction with Existing Systems

### Sync Protocol

Edges are **server-side only**. The local vault stores relationship declarations in YAML frontmatter; the server materializes these into `kb_resource_edges`. This division follows the existing pattern:

| Layer | Content | Authority |
|-------|---------|-----------|
| **Vault (local)** | Markdown files with YAML frontmatter declaring relationships | Portable declaration format |
| **Postgres (server)** | `kb_resource_edges` rows with resolved UUIDs, weights, metadata | Queryable materialized graph |

When a user edits a document's `depends_on` field locally and runs `temper sync push`:
1. The sync pipeline detects the content hash change
2. The updated document is pushed to the server
3. The server's ingest pipeline re-extracts edges from the updated frontmatter
4. Edge reconciliation (diff old vs new) updates `kb_resource_edges`

This means:
- Edges don't appear in the sync manifest or diff
- Edge conflicts are impossible (they're always derived from frontmatter, which is synced via the existing push/pull mechanism)
- Offline editing of relationship fields works: the user adds `depends_on: [foo]` locally, syncs later, edges are created server-side

### `sync_diff_for_device()` Composition

The existing `sync_diff_for_device()` function does not need modification. It operates on resources and content hashes, not edges. Edge extraction is a post-sync side-effect on the server.

### SearchMode Integration

The `SearchMode` enum already has the `Graph` variant:

```rust
pub enum SearchMode {
    Semantic,
    Keyword,
    Graph,
}
```

Implementation routing in `SearchService::search()`:

```rust
impl SearchService {
    pub async fn search(
        pool: &PgPool,
        profile_id: Uuid,
        params: SearchParams,
    ) -> ApiResult<Vec<SearchResultRow>> {
        match params.mode {
            SearchMode::Semantic => {
                // Existing: embed query → cosine similarity search
                Self::semantic_search(pool, profile_id, &params).await
            }
            SearchMode::Keyword => {
                // Future: tsvector full-text search
                Self::keyword_search(pool, profile_id, &params).await
            }
            SearchMode::Graph => {
                // NEW: combined vector + graph search
                // If query is empty, do pure graph traversal from seed_ids
                // If query is present, do combined_search()
                if params.q.is_empty() && !params.seed_ids.is_empty() {
                    Self::graph_traverse_search(pool, profile_id, &params).await
                } else {
                    Self::combined_search(pool, profile_id, &params).await
                }
            }
        }
    }
}
```

### SearchParams Extension

```rust
pub struct SearchParams {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    pub context: Option<String>,
    pub doc_type: Option<String>,
    pub team: Option<String>,
    pub depth: Option<u32>,
    pub limit: Option<i64>,
    // NEW fields for graph search
    #[serde(default)]
    pub seed_ids: Vec<Uuid>,          // explicit seed resources for graph expansion
    #[serde(default)]
    pub edge_types: Vec<String>,       // filter to specific edge types
    pub vector_weight: Option<f64>,    // override default 0.7
    pub graph_weight: Option<f64>,     // override default 0.3
}
```

### CLI Surface Changes

```
# Semantic search (existing, default)
temper search "neon deployment" --mode semantic

# Graph search — combined vector + graph
temper search "neon deployment" --mode graph

# Graph search with explicit seeds
temper search --mode graph --seed 019d1d24-2000-7379-8f26-ae4ae87bc5c6

# Graph search with edge type filter
temper search "neon deployment" --mode graph --edge-type extends --edge-type depends_on

# Pure graph traversal (no query embedding needed)
temper search --mode graph --seed 019d1d24-2000-7379-8f26-ae4ae87bc5c6 --depth 4

# Show resource edges (detail view)
temper show <resource-id> --edges

# Context command — potentially graph-first
temper context show work --graph   # show the work context as a graph of related resources
```

### `build_frontmatter()` Extension

```rust
pub fn build_frontmatter(
    id: Uuid,
    title: &str,
    context: &str,
    doc_type: &str,
    ingestion_source: Option<&str>,
    relationships: Option<&ResourceRelationships>,  // NEW parameter
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = format!(
        "---\ntemper-id: {id}\ntitle: \"{title}\"\ncontext: {context}\ndoc_type: {doc_type}\n"
    );
    if let Some(source) = ingestion_source {
        fm.push_str(&format!("ingestion_source: \"{source}\"\n"));
    }
    if let Some(rels) = relationships {
        if !rels.relates_to.is_empty() {
            fm.push_str("relates_to:\n");
            for r in &rels.relates_to {
                fm.push_str(&format!("  - {r}\n"));
            }
        }
        if !rels.extends.is_empty() {
            fm.push_str("extends:\n");
            for r in &rels.extends {
                fm.push_str(&format!("  - {r}\n"));
            }
        }
        if !rels.depends_on.is_empty() {
            fm.push_str("depends_on:\n");
            for r in &rels.depends_on {
                fm.push_str(&format!("  - {r}\n"));
            }
        }
        if !rels.references.is_empty() {
            fm.push_str("references:\n");
            for r in &rels.references {
                fm.push_str(&format!("  - {r}\n"));
            }
        }
        if !rels.tags.is_empty() {
            fm.push_str("tags:\n");
            for t in &rels.tags {
                fm.push_str(&format!("  - {t}\n"));
            }
        }
    }
    fm.push_str(&format!("created: {now}\n---\n\n"));
    fm
}
```

---

## Implementation Plan

### Phase 1: Schema Migration + Edge Table + Basic Traversal Functions

**Scope:** DDL only. No Rust code changes. Establish the schema foundation.

**Deliverables:**
- [ ] New migration file: `20260401000001_knowledge_graph_edges.sql`
- [ ] `edge_type` enum creation
- [ ] `kb_resource_edges` table with indexes and constraints
- [ ] `kb_deferred_edges` table for forward references
- [ ] `graph_traverse()` SQL function
- [ ] `graph_neighbors()` SQL function
- [ ] `graph_resource_edges()` SQL function
- [ ] Integration tests: create resources, insert edges, verify traversal returns correct results
- [ ] Integration tests: verify visibility scoping (profile A can't traverse through resources owned by profile B)
- [ ] Integration tests: verify cycle detection (A→B→C→A doesn't infinite loop)

**Estimated effort:** 1–2 days

### Phase 2: Frontmatter Extraction in Ingest Pipeline

**Scope:** Parse relationship fields from YAML frontmatter and upsert edges during ingest.

**Deliverables:**
- [ ] `extract_edge_declarations()` function in `temper-core` or `temper-cli/src/actions/ingest.rs`
- [ ] `TargetRef` enum (`Id(Uuid)`, `Slug(String)`)
- [ ] `EdgeType` Rust enum mirroring the SQL `edge_type`
- [ ] `ResourceRelationships` struct for parsed frontmatter relationships
- [ ] Reference resolution logic (UUID direct, slug lookup)
- [ ] Edge upsert in cloud ingest pipeline (API handler)
- [ ] Deferred edge storage for unresolved references
- [ ] Deferred edge resolution trigger on new resource creation
- [ ] Edge reconciliation on document update (diff old vs new)
- [ ] Batch edge creation for `temper import --directory`
- [ ] Unit tests for `extract_edge_declarations()`
- [ ] Unit tests for reference resolution
- [ ] Integration test for full ingest → edge creation flow

**Estimated effort:** 3–4 days

### Phase 3: Combined Vector + Graph Search

**Scope:** Implement `combined_search()` SQL function and wire `SearchMode::Graph` through the API.

**Deliverables:**
- [ ] `combined_search()` SQL function (added to migration or as separate migration)
- [ ] `SearchService::combined_search()` Rust implementation
- [ ] `SearchService::graph_traverse_search()` for pure graph traversal (no query embedding)
- [ ] Extend `SearchParams` with `seed_ids`, `edge_types`, `vector_weight`, `graph_weight`
- [ ] Wire `SearchMode::Graph` routing in `SearchService::search()`
- [ ] API response includes `origin` field (vector/graph/both) and `graph_score`/`vector_score`
- [ ] Integration tests: combined search returns results from both vector and graph paths
- [ ] Performance benchmarks at representative scale

**Estimated effort:** 2–3 days

### Phase 4: CLI Integration and Search Mode

**Scope:** CLI surface changes for graph search and edge display.

**Deliverables:**
- [ ] `temper search --mode graph` with `--seed`, `--edge-type`, `--depth` flags
- [ ] `temper show <resource> --edges` to display resource edges
- [ ] `build_frontmatter()` updated to accept optional `ResourceRelationships`
- [ ] `temper add` and `temper import` parse and store relationship fields from user-authored frontmatter
- [ ] User documentation for frontmatter relationship fields
- [ ] End-to-end test: create document with `depends_on`, search with `--mode graph`, verify relationship appears

**Estimated effort:** 2–3 days

### Total Estimated Effort: 8–12 days across 4 phases

---

## Risk Analysis

### Technical Risks

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| **Recursive CTE performance at scale** — Deep traversals over large graphs can be slow | Medium | Medium | Default `max_depth=3` limits expansion. Visibility CTE prunes the graph early. Worst case: 10K resources × avg 5 edges = 50K edge rows, 3-hop BFS touches ≤5³=125 nodes. Benchmark at 100K edges and tune. |
| **Cycle detection overhead** — `NOT ... = ANY(path)` array containment check scales with path length | Low | Low | Path length is bounded by `max_depth` (default 3, max ~10). Array containment on 10-element UUID arrays is negligible. |
| **Orphaned edges on resource deletion** — `ON DELETE CASCADE` handles this, but bulk deletes could be slow | Low | Low | `ON DELETE CASCADE` on both `source_resource_id` and `target_resource_id` FKs ensures automatic cleanup. The cascading delete is index-assisted (both columns are indexed). |
| **Deferred edge table grows unboundedly** — If slugs never resolve, deferred edges accumulate | Low | Medium | Add `attempts` counter and `last_attempt` timestamp. After 10 failed resolution attempts, log warning and stop retrying. Periodic cleanup job removes deferred edges older than 30 days. |
| **Edge type enum evolution** — Adding new edge types requires `ALTER TYPE ... ADD VALUE` | Low | Low | Postgres supports `ALTER TYPE edge_type ADD VALUE 'new_type'`. This is DDL but non-destructive. Plan for it in migration patterns. |
| **Combined search query plan instability** — Complex CTEs with `FULL OUTER JOIN` may get suboptimal plans | Medium | Medium | Use `EXPLAIN ANALYZE` during development. Consider materialized CTEs (`AS MATERIALIZED`) for the `visible` and `vector_hits` subqueries if the planner misestimates cardinality. |
| **Slug collision across contexts** — Same slug in different contexts creates ambiguous resolution | Medium | Medium | Resolution prioritizes same-context matches. Cross-context resolution requires exactly one match. If ambiguous, defer the edge and warn the user. UUIDs are always unambiguous. |

### Architectural Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **Schema coupling** — Edge table creates tight coupling between `kb_resources` and a new table | Low | This is intentional — edges are a fundamental property of resources. `ON DELETE CASCADE` ensures lifecycle consistency. |
| **Frontmatter as source of truth for edges** — If users edit edges via API without updating frontmatter, the declarations drift | Medium | Edges have a `provenance` metadata field. Frontmatter-derived edges (`"provenance": "frontmatter"`) are reconciled on sync. API-created edges (`"provenance": "manual"`) are never overwritten by frontmatter reconciliation. Two provenance tracks, no conflict. |
| **Migration ordering** — This migration depends on tables from the consolidated schema | Low | Use sqlx migration ordering (timestamp prefix). This migration's timestamp is after `20260330000001_consolidated_schema.sql`. |

---

## Open Questions

### Q1: Edge Weight Heuristics

**Question:** Should different edge types have different default weights? E.g., `extends` (strong structural connection, weight 1.0) vs `references` (weak citation, weight 0.5)?

**Current decision:** All edge types default to 1.0. Users can override per-edge via frontmatter metadata (future extension). Revisit after observing real usage patterns.

**Future option:** Type-based weight defaults configurable in `kb_doc_types` or a `kb_edge_type_config` table.

### Q2: Maximum Traversal Depth Defaults

**Question:** What should the default and maximum allowed `max_depth` be?

**Current decision:**
- Default: 3 (covers most practical use cases — "show me what this depends on, and what those depend on, and one more hop")
- Maximum: 10 (safety limit — beyond this, the result set is likely too large to be useful and the CTE cost is high)
- Configurable per-request via API parameter

### Q3: User-Defined Edge Types

**Question:** Should users be able to create custom edge types beyond the predefined enum?

**Current decision:** No. The `edge_type` enum is fixed at the schema level. Custom relationships can be expressed via the `metadata` JSONB column (e.g., `{"custom_type": "influences"}`). This keeps traversal queries simple (enum comparison) and prevents type proliferation.

**Revisit trigger:** If three or more users request custom edge types, consider adding a `kb_edge_type_definitions` table and switching from enum to varchar with FK validation.

### Q4: Should `temper context` Become Graph-First?

**Question:** The `temper context` command currently shows resources within a context. Should it default to a graph view that shows relationships between resources?

**Recommendation:** Not in v1. Add a `--graph` flag that renders the context's resources as a graph (nodes = resources, edges = relationships). Default remains the current list view. Evaluate whether graph view should become default after user feedback.

### Q5: Edge Deletion Semantics

**Question:** When a resource is soft-deleted (`is_active = false`), should its edges be preserved, soft-deleted, or hard-deleted?

**Current decision:** Edges reference `kb_resources(id)` with `ON DELETE CASCADE`, which triggers on hard delete. For soft-delete (`is_active = false`), edges remain but the resource won't appear in `resources_visible_to()` results (which filters `is_active = true`), so the edges are effectively invisible. This preserves graph structure for potential undelete.

### Q6: How Should the TUI Neighbor Tab Use Graph Data?

**Question:** The TUI design spec (2026-03-24) describes a "Neighbors" tab showing cross-references. Should this be powered by graph edges instead of (or in addition to) wiki-link parsing?

**Recommendation:** Yes. The Neighbors tab should query `graph_neighbors()` for the selected resource. This gives a richer view than wiki-link parsing alone, since it includes all edge types and directions. Wiki-links that are also in frontmatter `references` would be deduplicated. Future phase: infer edges from wiki-link `[[notation]]` in document body (not just frontmatter).

### Q7: Should External URIs Get Stub Resources?

**Question:** When `references: [https://neon.tech/docs/extensions/pgvector]` appears in frontmatter, should we create a stub `kb_resources` entry for the external URL to enable graph edges?

**Recommendation:** Not yet. External URIs are metadata, not graph participants, until they are imported (`temper import <url>`). At import time, the resource is created and any deferred edges resolve naturally. Creating stub resources for every external reference would pollute the resource namespace.

---

## Decision Log

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Graph implementation strategy | Native Postgres adjacency list + recursive CTEs | Neon doesn't support AGE. CTEs compose with pgvector in single queries. R3 and R4 established this direction. |
| D2 | Vertex table | No separate vertex table — `kb_resources` rows are vertices | Avoids identity duplication, preserves composability with existing SQL functions |
| D3 | Edge direction | All edges are directed | Preserves semantic meaning (`extends` is not symmetric). Bidirectional queries union forward + reverse scans. |
| D4 | Duplicate prevention | `UNIQUE(source_resource_id, target_resource_id, edge_type)` | One edge per type per direction between any two resources |
| D5 | Visibility enforcement | Scoped at every traversal hop, not post-filtered | Prevents information leakage through graph structure (R4 design principle) |
| D6 | Edge provenance tracking | `metadata->>'provenance'` field: `frontmatter`, `manual`, `inferred` | Enables reconciliation: frontmatter-derived edges are synced with document content; manual edges are preserved independently |
| D7 | Forward reference handling | Deferred edge table with resolution on new resource creation | Supports bulk import where documents reference not-yet-imported resources |
| D8 | Tag handling | JSONB for phase 1, tag-as-resource for phase 2+ | Avoids premature complexity; designs for eventual graph participation |
| D9 | Default traversal depth | 3 hops, maximum 10 | Covers practical use cases without excessive CTE cost |
| D10 | Custom edge types | Not supported; use `metadata` JSONB | Keeps enum-based traversal fast and simple |
| D11 | Sync model | Edges are server-side only; frontmatter is the portable declaration | Follows existing pattern: vault is portable markdown, Postgres is the queryable materialization |
| D12 | Weight semantics | Higher = stronger (not distance), multiplicative along path, default 1.0 | Intuitive for scoring: directly connected resources always score higher than distant ones |
| D13 | Combined search scoring | `p_vector_weight × similarity + p_graph_weight × (path_weight / (depth + 1))` | Tunable blend; defaults favor vector (0.7) over graph (0.3) since semantic relevance is primary |

---

## Appendix A: Combined Vector + Graph Scoring Derivation

### Scoring Model

The combined search score for a resource is:

```
combined_score = w_v × vector_score + w_g × graph_score
```

Where:
- `w_v` = vector weight (default 0.7)
- `w_g` = graph weight (default 0.3)
- `vector_score` = `1.0 - cosine_distance` (range: [0, 1], where 1 = identical)
- `graph_score` = `max(path_weight / (depth + 1))` across all paths to this resource

### Graph Score Derivation

For a resource reached via multiple paths, we take the maximum score across all paths:

```
graph_score = max over all paths P: (∏ weights along P) / (len(P) + 1)
```

The `+1` in the denominator ensures:
- A directly connected resource (depth=1) with weight 1.0 gets score `1.0 / 2 = 0.5`
- A 2-hop resource with all weights 1.0 gets score `1.0 / 3 = 0.33`
- A directly connected resource with weight 0.5 gets `0.5 / 2 = 0.25`

This decay function ensures that graph proximity contributes meaningfully but doesn't overwhelm vector similarity for distant resources.

### Score Ranges

| Scenario | Vector Score | Graph Score | Combined (0.7/0.3) |
|----------|-------------|-------------|---------------------|
| Highly similar, directly connected | 0.9 | 0.5 | 0.78 |
| Highly similar, no graph connection | 0.9 | 0.0 | 0.63 |
| Low similarity, directly connected | 0.3 | 0.5 | 0.36 |
| No vector match, 1-hop graph | 0.0 | 0.5 | 0.15 |
| No vector match, 2-hop graph | 0.0 | 0.33 | 0.10 |

The default weights (0.7/0.3) ensure that a resource must be at least moderately similar via vector search to rank highly, even with strong graph connections. This prevents the graph from surfacing irrelevant but structurally connected documents.

---

## Appendix B: Rust Type Stubs

### Edge Types (temper-core)

```rust
// crates/temper-core/src/types/graph.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Edge type enum — mirrors the Postgres `edge_type` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    RelatesTo,
    Extends,
    DependsOn,
    References,
    ParentOf,
    TaggedWith,
    PrecededBy,
    DerivedFrom,
}

/// Reference target in frontmatter — either a resolved UUID or a slug string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetRef {
    Id(Uuid),
    Slug(String),
}

/// Parsed relationship declarations from YAML frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRelationships {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relates_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preceded_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<String>,
}

/// A resolved edge ready for database insertion.
#[derive(Debug, Clone)]
pub struct ResolvedEdge {
    pub source_resource_id: Uuid,
    pub target_resource_id: Uuid,
    pub edge_type: EdgeType,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

/// Result of edge reconciliation after a frontmatter update.
#[derive(Debug, Clone)]
pub struct EdgeReconciliation {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub deferred: usize,
}

/// Graph traversal result row — mirrors the SQL function return type.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphTraversalRow {
    pub resource_id: Uuid,
    pub depth: i32,
    pub path: Vec<Uuid>,
    pub edge_type: Option<EdgeType>,
    pub from_resource_id: Option<Uuid>,
    pub path_weight: f64,
}

/// Graph neighbor row — mirrors the SQL function return type.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphNeighborRow {
    pub resource_id: Uuid,
    pub edge_type: EdgeType,
    pub direction: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

/// Combined search result row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CombinedSearchRow {
    pub resource_id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub vector_score: f64,
    pub graph_score: f64,
    pub combined_score: f64,
    pub origin: String,  // "vector", "graph", "both"
}
```

### Access-Scoped Graph Queries (temper-core)

```rust
// Extension to existing AccessScoped pattern from R4

use crate::types::graph::{EdgeType, GraphTraversalRow, GraphNeighborRow, CombinedSearchRow};

/// Graph query operations — all visibility-scoped.
pub trait GraphQueries {
    /// Traverse the graph from seed resources.
    async fn graph_traverse(
        &self,
        profile_id: Uuid,
        seed_ids: &[Uuid],
        max_depth: i32,
        edge_types: &[EdgeType],
    ) -> Result<Vec<GraphTraversalRow>>;

    /// Get immediate neighbors of a resource.
    async fn graph_neighbors(
        &self,
        profile_id: Uuid,
        resource_id: Uuid,
        direction: &str,
        edge_types: &[EdgeType],
    ) -> Result<Vec<GraphNeighborRow>>;

    /// Combined vector + graph search.
    async fn combined_search(
        &self,
        profile_id: Uuid,
        query_embedding: &[f32],
        seed_ids: &[Uuid],
        vector_weight: f64,
        graph_weight: f64,
    ) -> Result<Vec<CombinedSearchRow>>;
}
```

---

## Appendix C: Related Tickets & Dependencies

| Ticket | Relationship | Status |
|--------|-------------|--------|
| **R2** — Data Model & Schema Design | Foundation — established `kb_resources`, `kb_chunks`, pgvector | ✅ Done |
| **R4** — Crate Architecture, Auth & Access Control | Foundation — established access control SQL functions, graph CTE sketch | ✅ Done |
| **R5** — Indexing, Sync & Resource Management | Foundation — established `SearchMode::Graph`, `SearchParams`, ingest pipeline | ✅ Done |
| **I5e** — Local KB Restructure | Foundation — vault layout, manifest, `build_frontmatter()` | ✅ Done |
| **I6a** — Sync Infrastructure | Foundation — sync protocol, `sync_diff_for_device()` | ✅ Done |
| **I5d** — Cloud-Routed Search | Parallel — search API that `SearchMode::Graph` will extend | ✅ Done |
| **R6** — Filesystem Watcher | Parallel — watcher detects frontmatter changes, could trigger edge re-extraction | Research done |
| **I9** — Search Unification & CLI Evolution | Consumer — graph search mode is part of the search unification scope | Backlog |
| **I10** — temper-mcp | Consumer — MCP agent access to graph queries | Backlog |

### New Tickets to Create

| Ticket | Scope | Phase |
|--------|-------|-------|
| **I7a** — Knowledge Graph Schema Migration | `kb_resource_edges` table, `kb_deferred_edges`, `edge_type` enum, graph SQL functions | Phase 1 |
| **I7b** — Frontmatter Edge Extraction Pipeline | `extract_edge_declarations()`, reference resolution, edge upsert/reconciliation in ingest | Phase 2 |
| **I7c** — Combined Vector + Graph Search | `combined_search()` function, `SearchMode::Graph` wiring, API extension | Phase 3 |
| **I7d** — Graph CLI & Frontend Integration | `--mode graph` CLI flags, `--edges` display, `build_frontmatter()` extension, TUI updates | Phase 4 |
