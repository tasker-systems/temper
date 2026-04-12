# Knowledge Graph Phase 1: Schema, SQL Functions & Rust Types

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the database foundation for the knowledge graph — edge tables, SQL traversal functions, and matching Rust types — with integration tests proving visibility scoping, cycle detection, and typed traversal all work against real Postgres.

**Architecture:** Native Postgres adjacency list with recursive CTEs. No separate vertex table — `kb_resources` rows ARE vertices. Edges are directed with semantic types (`extends`, `depends_on`, etc.). All SQL functions compose with the existing `resources_visible_to()` function to enforce visibility at every traversal hop. A `kb_deferred_edges` table handles forward references during bulk import.

**Tech Stack:** Postgres 18, sqlx 0.8, serde, chrono, uuid v7, ts-rs (feature-gated)

**Source Research:** `R7: Vertex-Edge Knowledge Graph in Native Postgres` (vault research doc `2026-04-01-r7-vertex-edge-knowledge-graph-native-postgres`)

**Important codebase corrections vs R7:**
- `resources_visible_to()` signature is `(p_profile_id UUID, p_team_id UUID DEFAULT NULL, p_resource_ids UUID[] DEFAULT '{}')` — graph functions must pass all 3 params
- No `SearchMode` enum exists — search is param-driven via `SearchParams`. Graph search mode is Phase 3 scope, not this phase.
- `temper import` → `temper add` — the CLI command was renamed during I5a
- Ingest is client-side (CLI chunks+embeds, POSTs `IngestPayload`) with server-side fallback

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `migrations/20260411000001_knowledge_graph_edges.sql` | DDL: `edge_type` enum, `kb_resource_edges` table, `kb_deferred_edges` table, `graph_traverse()`, `graph_neighbors()`, `graph_resource_edges()` SQL functions |
| `crates/temper-core/src/types/graph.rs` | Rust types: `EdgeType`, `TargetRef`, `ResourceRelationships`, `GraphTraversalRow`, `GraphNeighborRow`, `GraphEdgeRow`, `ResolvedEdge`, `EdgeReconciliation` |
| `crates/temper-api/tests/graph_test.rs` | Integration tests: edge CRUD, traversal, visibility scoping, cycle detection |

### Modified Files
| File | Change |
|------|--------|
| `crates/temper-core/src/types/mod.rs` | Add `pub mod graph;` and re-exports |
| `crates/temper-api/tests/common/fixtures.rs` | Add edge cleanup to `clean_and_seed()`, add graph-specific seed helpers |

---

## Task 1: Migration — Edge Type Enum and Tables

**Files:**
- Create: `migrations/20260411000001_knowledge_graph_edges.sql`

- [ ] **Step 1: Write the migration file**

```sql
-- =============================================================================
-- R7 Phase 1: Knowledge Graph — Edge Tables
-- =============================================================================
-- Adds: edge_type enum, kb_resource_edges table, kb_deferred_edges table
--
-- Design: Native Postgres adjacency list. kb_resources rows are vertices.
-- Edges are directed with semantic types. Forward references (unresolvable
-- targets during bulk import) are held in kb_deferred_edges until the target
-- resource is created.

-- ─── Edge Type Enum ──────────────────────────────────────────────────────────

CREATE TYPE edge_type AS ENUM (
    'relates_to',     -- general relationship (symmetric in meaning, stored directed)
    'extends',        -- A builds upon B's content
    'depends_on',     -- A requires B to be complete/valid
    'references',     -- A cites or mentions B
    'parent_of',      -- A is the parent of B (hierarchy)
    'tagged_with',    -- A is tagged with concept B (future: tag-as-resource)
    'preceded_by',    -- A is preceded by B in sequence
    'derived_from'    -- A was derived from B (source material)
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

-- Forward traversal: given a source, find all targets
CREATE INDEX idx_edges_source ON kb_resource_edges(source_resource_id);
-- Reverse traversal: given a target, find all sources
CREATE INDEX idx_edges_target ON kb_resource_edges(target_resource_id);
-- Type-filtered forward traversal
CREATE INDEX idx_edges_source_type ON kb_resource_edges(source_resource_id, edge_type);
-- Type-filtered reverse traversal
CREATE INDEX idx_edges_target_type ON kb_resource_edges(target_resource_id, edge_type);
-- Profile-scoped queries
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
```

- [ ] **Step 2: Verify the migration applies against local Docker Postgres**

Run: `cargo make docker-up && DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run`

Expected: Migration applies cleanly with no errors.

- [ ] **Step 3: Verify tables exist**

Run: `psql postgresql://temper:temper@localhost:5437/temper_development -c "\dt kb_resource_edges" -c "\dt kb_deferred_edges" -c "\dT+ edge_type"`

Expected: Both tables and the enum type are listed.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260411000001_knowledge_graph_edges.sql
git commit -m "feat(schema): add kb_resource_edges and kb_deferred_edges tables

R7 Phase 1 — knowledge graph edge storage. Directed edges between
kb_resources with semantic types, weight, JSONB metadata, and
provenance tracking. Forward references stored in kb_deferred_edges
for resolution during bulk import."
```

---

## Task 2: Migration — SQL Traversal Functions

**Files:**
- Modify: `migrations/20260411000001_knowledge_graph_edges.sql` (append functions)

All functions follow the established pattern: `LANGUAGE SQL STABLE`, composable inside CTEs, visibility-scoped via `resources_visible_to()`. The real signature is `resources_visible_to(p_profile_id, p_team_id, p_resource_ids)` — we pass `NULL` for team_id and `'{}'` for resource_ids to get the full visibility set.

- [ ] **Step 1: Append graph_traverse() to the migration**

Append to `migrations/20260411000001_knowledge_graph_edges.sql`:

```sql
-- ─── Graph Traversal Functions ───────────────────────────────────────────────

-- N-hop forward traversal with visibility scoping, cycle detection, and
-- path-weight decay. Composes with resources_visible_to() at every hop.
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
        SELECT v.resource_id
          FROM resources_visible_to(p_profile_id, NULL, '{}') v
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
        WHERE v.resource_id = ANY(p_seed_ids)

        UNION ALL

        -- Recursive case: expand one hop forward
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
    WHERE t.depth > 0   -- exclude seeds from result set
    ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC
$$;
```

- [ ] **Step 2: Append graph_neighbors() to the migration**

Append to the same migration file:

```sql
-- Immediate neighbors (1-hop, optionally directional).
-- Simpler and faster than full traversal — no recursion needed.
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
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id
      AND p_direction IN ('both', 'outgoing')
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
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id
      AND p_direction IN ('both', 'incoming')
      AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
$$;
```

- [ ] **Step 3: Append graph_resource_edges() to the migration**

Append to the same migration file:

```sql
-- Edge listing for a specific resource — returns all edges with peer metadata.
-- Used by CLI `temper show --edges` and API detail endpoints.
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
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.target_resource_id
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
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id

    ORDER BY edge_type, direction, created
$$;
```

- [ ] **Step 4: Re-run migration and verify functions exist**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run`

Then verify:

Run: `psql postgresql://temper:temper@localhost:5437/temper_development -c "\df graph_*"`

Expected: Three functions listed: `graph_traverse`, `graph_neighbors`, `graph_resource_edges`.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260411000001_knowledge_graph_edges.sql
git commit -m "feat(schema): add graph_traverse, graph_neighbors, graph_resource_edges SQL functions

Visibility-scoped recursive CTE traversal composing with
resources_visible_to(). Cycle detection via path array.
Edge type filtering. Path weight decay for scoring."
```

---

## Task 3: Rust Types — EdgeType Enum and Core Graph Types

**Files:**
- Create: `crates/temper-core/src/types/graph.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

Follow existing enum patterns from `crates/temper-core/src/types/access.rs` (lines 10-29): `#[sqlx(type_name = "...", rename_all = "snake_case")]`, feature-gated `ts_rs::TS` derives, `#[cfg_attr(feature = "typescript", ...)]`.

- [ ] **Step 1: Create graph.rs with EdgeType enum**

Create `crates/temper-core/src/types/graph.rs`:

```rust
//! Knowledge graph types — edge types, traversal results, and relationship
//! declarations for the R7 vertex-edge graph stored in `kb_resource_edges`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Edge Type ──────────────────────────────────────────────────────────────

/// Edge type enum — mirrors the Postgres `edge_type` enum exactly.
///
/// All edges are directed: `source_resource_id → target_resource_id`.
/// Symmetric queries (e.g., `relates_to`) union forward + reverse scans.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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

impl std::fmt::Display for EdgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelatesTo => write!(f, "relates_to"),
            Self::Extends => write!(f, "extends"),
            Self::DependsOn => write!(f, "depends_on"),
            Self::References => write!(f, "references"),
            Self::ParentOf => write!(f, "parent_of"),
            Self::TaggedWith => write!(f, "tagged_with"),
            Self::PrecededBy => write!(f, "preceded_by"),
            Self::DerivedFrom => write!(f, "derived_from"),
        }
    }
}

// ─── Target Reference ───────────────────────────────────────────────────────

/// A reference target in frontmatter — either a resolved UUID or a slug string.
/// Used during edge extraction before resolution against the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetRef {
    Id(Uuid),
    Slug(String),
}

impl TargetRef {
    /// Parse a frontmatter reference value into a TargetRef.
    ///
    /// - Valid UUID string → `TargetRef::Id`
    /// - Non-empty, non-URL string → `TargetRef::Slug`
    /// - Empty or URL strings → `None` (external URIs aren't graph edges)
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(uuid) = Uuid::parse_str(trimmed) {
            return Some(Self::Id(uuid));
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return None;
        }
        Some(Self::Slug(trimmed.to_string()))
    }
}

// ─── Relationship Declarations ──────────────────────────────────────────────

/// Parsed relationship declarations from YAML frontmatter.
///
/// Each field maps to an edge type. Values are raw strings — either UUIDs
/// or slugs — that get resolved to `kb_resources.id` at ingest time.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
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

impl ResourceRelationships {
    /// Returns true if no relationships are declared.
    pub fn is_empty(&self) -> bool {
        self.relates_to.is_empty()
            && self.extends.is_empty()
            && self.depends_on.is_empty()
            && self.references.is_empty()
            && self.tags.is_empty()
            && self.parent.is_none()
            && self.preceded_by.is_empty()
            && self.derived_from.is_empty()
    }

    /// Extract (EdgeType, TargetRef) pairs from all declared relationships.
    ///
    /// Skips values that don't parse as UUID or slug (e.g., external URLs).
    /// The `parent` field produces a reversed edge: the parent resource gets
    /// a `ParentOf` edge pointing to this resource (handled by the caller).
    pub fn to_edge_declarations(&self) -> Vec<(EdgeType, TargetRef)> {
        let mut edges = Vec::new();
        let field_mappings: &[(&[String], EdgeType)] = &[
            (&self.relates_to, EdgeType::RelatesTo),
            (&self.extends, EdgeType::Extends),
            (&self.depends_on, EdgeType::DependsOn),
            (&self.references, EdgeType::References),
            (&self.tags, EdgeType::TaggedWith),
            (&self.preceded_by, EdgeType::PrecededBy),
            (&self.derived_from, EdgeType::DerivedFrom),
        ];
        for (values, edge_type) in field_mappings {
            for value in *values {
                if let Some(target) = TargetRef::parse(value) {
                    edges.push((*edge_type, target));
                }
            }
        }
        // Parent is a single value producing a reversed ParentOf edge
        if let Some(ref parent_val) = self.parent {
            if let Some(target) = TargetRef::parse(parent_val) {
                edges.push((EdgeType::ParentOf, target));
            }
        }
        edges
    }
}

// ─── Database Result Types ──────────────────────────────────────────────────

/// Graph traversal result row — mirrors the `graph_traverse()` SQL function.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphTraversalRow {
    pub resource_id: Uuid,
    pub depth: i32,
    pub path: Vec<Uuid>,
    pub edge_type: Option<EdgeType>,
    pub from_resource_id: Option<Uuid>,
    pub path_weight: f64,
}

/// Graph neighbor row — mirrors the `graph_neighbors()` SQL function.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphNeighborRow {
    pub resource_id: Uuid,
    pub edge_type: EdgeType,
    pub direction: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

/// Edge listing row — mirrors the `graph_resource_edges()` SQL function.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphEdgeRow {
    pub edge_id: Uuid,
    pub peer_resource_id: Uuid,
    pub peer_title: String,
    pub peer_slug: String,
    pub edge_type: EdgeType,
    pub direction: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
    pub created: chrono::DateTime<chrono::Utc>,
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
```

- [ ] **Step 2: Register the module and add re-exports in mod.rs**

In `crates/temper-core/src/types/mod.rs`, add the module declaration after line 29 (`pub mod search;`):

```rust
pub mod graph;
```

Add re-exports after line 34 (`pub mod vault_config;`), before the existing `pub use` block:

```rust
pub use graph::{
    EdgeReconciliation, EdgeType, GraphEdgeRow, GraphNeighborRow, GraphTraversalRow, ResolvedEdge,
    ResourceRelationships, TargetRef,
};
```

- [ ] **Step 3: Verify temper-core compiles**

Run: `cargo check -p temper-core --all-features 2>&1 | head -30`

Expected: Compiles cleanly with no errors. Warnings about unused items are acceptable — these types will be consumed by later phases.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/graph.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(temper-core): add graph types — EdgeType, traversal rows, relationships

R7 Phase 1 Rust types mirroring the Postgres edge_type enum and
SQL function return shapes. ResourceRelationships parses frontmatter
relationship declarations into typed edge targets. Feature-gated
ts-rs and schemars derives for TypeScript and MCP compatibility."
```

---

## Task 4: Unit Tests for TargetRef and ResourceRelationships

**Files:**
- Modify: `crates/temper-core/src/types/graph.rs` (append test module)

- [ ] **Step 1: Add unit tests to graph.rs**

Append to the bottom of `crates/temper-core/src/types/graph.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ── TargetRef::parse ────────────────────────────────────────────────

    #[test]
    fn parse_valid_uuid() {
        let input = "019d1d24-2000-7379-8f26-ae4ae87bc5c6";
        let result = TargetRef::parse(input);
        assert_eq!(
            result,
            Some(TargetRef::Id(
                Uuid::parse_str("019d1d24-2000-7379-8f26-ae4ae87bc5c6").unwrap()
            ))
        );
    }

    #[test]
    fn parse_slug() {
        assert_eq!(
            TargetRef::parse("r2-data-model"),
            Some(TargetRef::Slug("r2-data-model".to_string()))
        );
    }

    #[test]
    fn parse_slug_with_whitespace_trimming() {
        assert_eq!(
            TargetRef::parse("  some-slug  "),
            Some(TargetRef::Slug("some-slug".to_string()))
        );
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(TargetRef::parse(""), None);
        assert_eq!(TargetRef::parse("   "), None);
    }

    #[test]
    fn parse_http_url_returns_none() {
        assert_eq!(TargetRef::parse("https://neon.tech/docs"), None);
        assert_eq!(TargetRef::parse("http://example.com"), None);
    }

    // ── ResourceRelationships ───────────────────────────────────────────

    #[test]
    fn empty_relationships() {
        let rels = ResourceRelationships::default();
        assert!(rels.is_empty());
        assert!(rels.to_edge_declarations().is_empty());
    }

    #[test]
    fn to_edge_declarations_extracts_all_types() {
        let rels = ResourceRelationships {
            extends: vec!["r2-data-model".to_string()],
            depends_on: vec![
                "019d1d24-2000-7379-8f26-ae4ae87bc5c6".to_string(),
                "r3-platform-eval".to_string(),
            ],
            references: vec![
                "r1-workflow-vision".to_string(),
                "https://external.com/doc".to_string(), // should be skipped
            ],
            parent: Some("milestone-q2".to_string()),
            ..Default::default()
        };

        assert!(!rels.is_empty());

        let declarations = rels.to_edge_declarations();
        // extends: 1, depends_on: 2, references: 1 (URL skipped), parent: 1 = 5
        assert_eq!(declarations.len(), 5);

        // Verify edge types
        assert!(declarations
            .iter()
            .any(|(t, _)| *t == EdgeType::Extends));
        assert_eq!(
            declarations
                .iter()
                .filter(|(t, _)| *t == EdgeType::DependsOn)
                .count(),
            2
        );
        // URL reference should be filtered out
        assert_eq!(
            declarations
                .iter()
                .filter(|(t, _)| *t == EdgeType::References)
                .count(),
            1
        );
        // Parent produces ParentOf
        assert!(declarations
            .iter()
            .any(|(t, _)| *t == EdgeType::ParentOf));
    }

    // ── EdgeType serde ──────────────────────────────────────────────────

    #[test]
    fn edge_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&EdgeType::DependsOn).unwrap(),
            "\"depends_on\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeType::RelatesTo).unwrap(),
            "\"relates_to\""
        );
    }

    #[test]
    fn edge_type_deserializes_from_snake_case() {
        let result: EdgeType = serde_json::from_str("\"derived_from\"").unwrap();
        assert_eq!(result, EdgeType::DerivedFrom);
    }

    #[test]
    fn edge_type_display() {
        assert_eq!(EdgeType::ParentOf.to_string(), "parent_of");
        assert_eq!(EdgeType::TaggedWith.to_string(), "tagged_with");
    }

    // ── ResourceRelationships serde round-trip ──────────────────────────

    #[test]
    fn relationships_serde_round_trip() {
        let rels = ResourceRelationships {
            extends: vec!["r2-data-model".to_string()],
            depends_on: vec!["r3-platform".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&rels).unwrap();
        let parsed: ResourceRelationships = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.extends, rels.extends);
        assert_eq!(parsed.depends_on, rels.depends_on);
        assert!(parsed.relates_to.is_empty());
    }

    #[test]
    fn relationships_deserialize_skips_missing_fields() {
        let json = r#"{"extends": ["foo"]}"#;
        let parsed: ResourceRelationships = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.extends, vec!["foo"]);
        assert!(parsed.depends_on.is_empty());
        assert!(parsed.parent.is_none());
    }
}
```

- [ ] **Step 2: Run the unit tests**

Run: `cargo nextest run -p temper-core -E 'test(graph::tests)' 2>&1 | tail -20`

Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/graph.rs
git commit -m "test(temper-core): unit tests for TargetRef parsing and ResourceRelationships

Covers UUID/slug/URL parsing, edge declaration extraction,
serde round-trip, and empty-state checks."
```

---

## Task 5: Test Fixtures — Edge Cleanup and Graph Seed Helpers

**Files:**
- Modify: `crates/temper-api/tests/common/fixtures.rs`

The existing `clean_and_seed()` deletes in reverse FK order. We need to add `kb_resource_edges` and `kb_deferred_edges` cleanup, and add helpers for creating test resources and edges.

- [ ] **Step 1: Add edge table cleanup to clean_and_seed()**

In `crates/temper-api/tests/common/fixtures.rs`, add these two cleanup statements at the top of the function body (before the `kb_events` deletion on line 21), since edges FK to resources which FK to profiles:

```rust
    sqlx::query("DELETE FROM kb_deferred_edges")
        .execute(pool)
        .await
        .expect("clean kb_deferred_edges");

    sqlx::query("DELETE FROM kb_resource_edges")
        .execute(pool)
        .await
        .expect("clean kb_resource_edges");
```

- [ ] **Step 2: Add graph test helper functions**

Append to the bottom of `crates/temper-api/tests/common/fixtures.rs`:

```rust
/// Create a test profile and return its UUID.
pub async fn create_test_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    let sub = format!("test|{id}");
    sqlx::query(
        r#"INSERT INTO kb_profiles (id, display_name, email)
           VALUES ($1, $2, $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(email)
    .bind(email)
    .execute(pool)
    .await
    .expect("create test profile");

    // Link auth so JWT resolution works
    sqlx::query(
        r#"INSERT INTO kb_profile_auth_links (id, profile_id, provider_name, provider_sub)
           VALUES ($1, $2, 'test-provider', $3)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(uuid::Uuid::now_v7())
    .bind(id)
    .bind(&sub)
    .execute(pool)
    .await
    .expect("create test auth link");

    id
}

/// Create a test resource owned by the given profile and return its UUID.
pub async fn create_test_resource(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    title: &str,
    slug: &str,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())"#,
    )
    .bind(id)
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(format!("test://{slug}"))
    .bind(title)
    .bind(slug)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create test resource");

    id
}

/// Insert a directed edge between two resources.
pub async fn create_test_edge(
    pool: &PgPool,
    source_id: uuid::Uuid,
    target_id: uuid::Uuid,
    edge_type: &str,
    profile_id: uuid::Uuid,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id, edge_type,
             weight, metadata, created_by_profile_id)
           VALUES ($1, $2, $3, $4::edge_type, 1.0, '{}', $5)"#,
    )
    .bind(id)
    .bind(source_id)
    .bind(target_id)
    .bind(edge_type)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("create test edge");

    id
}
```

- [ ] **Step 3: Verify existing tests still pass with the fixture changes**

Run: `cargo nextest run -p temper-api --features test-db 2>&1 | tail -20`

Expected: All existing tests pass. The new cleanup statements delete from empty tables, so no change in behavior.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/tests/common/fixtures.rs
git commit -m "test(fixtures): add edge table cleanup and graph test helpers

Extends clean_and_seed() with kb_resource_edges and
kb_deferred_edges cleanup. Adds create_test_profile,
create_test_resource, and create_test_edge helpers for
graph integration tests."
```

---

## Task 6: Integration Tests — Edge CRUD and Basic Traversal

**Files:**
- Create: `crates/temper-api/tests/graph_test.rs`

These tests verify the SQL functions work correctly against real Postgres. They use `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` which applies all migrations (including our new one) to an isolated per-test database.

- [ ] **Step 1: Create graph_test.rs with edge insertion and neighbor tests**

Create `crates/temper-api/tests/graph_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;

/// Inserting an edge and querying neighbors returns the expected peer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_insert_edge_and_query_neighbors(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "graph@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Query outgoing neighbors of r1
    let rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT resource_id, edge_type::TEXT, direction FROM graph_neighbors($1, $2, 'outgoing', '{}')",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
    assert_eq!(rows[0].1, "extends");
    assert_eq!(rows[0].2, "outgoing");

    // Query incoming neighbors of r2
    let rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT resource_id, edge_type::TEXT, direction FROM graph_neighbors($1, $2, 'incoming', '{}')",
    )
    .bind(profile)
    .bind(r2)
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors incoming");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r1);
    assert_eq!(rows[0].1, "extends");
    assert_eq!(rows[0].2, "incoming");
}

/// Bidirectional neighbor query returns both directions.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_both_directions(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "both@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    // r1 → r2 (extends), r3 → r1 (depends_on)
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r1, "depends_on", profile).await;

    // Both directions from r1: should see r2 (outgoing) and r3 (incoming)
    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT resource_id, direction FROM graph_neighbors($1, $2, 'both', '{}')",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors both");

    assert_eq!(rows.len(), 2);
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2));
    assert!(ids.contains(&r3));
}

/// Edge type filter restricts neighbor results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_edge_type_filter(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "filter@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r1, r3, "references", profile).await;

    // Filter to only 'extends' edges
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT resource_id FROM graph_neighbors($1, $2, 'both', $3)",
    )
    .bind(profile)
    .bind(r1)
    .bind(vec!["extends"])
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors filtered");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
}

/// graph_resource_edges returns edges with peer metadata.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_resource_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "edges@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "depends_on", profile).await;

    let rows: Vec<(uuid::Uuid, String, String, String)> = sqlx::query_as(
        "SELECT peer_resource_id, peer_title, edge_type::TEXT, direction FROM graph_resource_edges($1, $2)",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("graph_resource_edges");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
    assert_eq!(rows[0].1, "Doc B");
    assert_eq!(rows[0].2, "depends_on");
    assert_eq!(rows[0].3, "outgoing");
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(graph_test)' 2>&1 | tail -30`

Expected: All 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/graph_test.rs
git commit -m "test(graph): integration tests for edge insertion and neighbor queries

Verifies graph_neighbors() with directional filtering, edge type
filtering, and graph_resource_edges() with peer metadata joins.
All tests run against real Postgres via sqlx::test."
```

---

## Task 7: Integration Tests — Multi-Hop Traversal and Cycle Detection

**Files:**
- Modify: `crates/temper-api/tests/graph_test.rs` (append tests)

- [ ] **Step 1: Add multi-hop traversal test**

Append to `crates/temper-api/tests/graph_test.rs`:

```rust
/// graph_traverse follows a chain of edges up to max_depth.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_multi_hop(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "traverse@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;
    let r4 = common::fixtures::create_test_resource(&pool, profile, "R4", "r4").await;

    // Chain: r1 → r2 → r3 → r4
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r4, "extends", profile).await;

    // Traverse from r1 with depth=2 — should reach r2 (depth 1) and r3 (depth 2), NOT r4
    let rows: Vec<(uuid::Uuid, i32)> = sqlx::query_as(
        "SELECT resource_id, depth FROM graph_traverse($1, $2, $3, '{}')",
    )
    .bind(profile)
    .bind(vec![r1])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("graph_traverse depth=2");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2), "r2 should be reachable at depth 1");
    assert!(ids.contains(&r3), "r3 should be reachable at depth 2");
    assert!(!ids.contains(&r4), "r4 should NOT be reachable at depth 2");
    assert!(!ids.contains(&r1), "seed r1 should be excluded from results");

    // Depth=3 should now include r4
    let rows: Vec<(uuid::Uuid, i32)> = sqlx::query_as(
        "SELECT resource_id, depth FROM graph_traverse($1, $2, $3, '{}')",
    )
    .bind(profile)
    .bind(vec![r1])
    .bind(3_i32)
    .fetch_all(&pool)
    .await
    .expect("graph_traverse depth=3");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r4), "r4 should be reachable at depth 3");
}

/// graph_traverse handles cycles without infinite recursion.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_cycle_detection(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "cycle@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;

    // Cycle: r1 → r2 → r3 → r1
    common::fixtures::create_test_edge(&pool, r1, r2, "relates_to", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "relates_to", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r1, "relates_to", profile).await;

    // Traverse from r1 — should terminate, not loop forever
    let rows: Vec<(uuid::Uuid, i32)> = sqlx::query_as(
        "SELECT resource_id, depth FROM graph_traverse($1, $2, $3, '{}')",
    )
    .bind(profile)
    .bind(vec![r1])
    .bind(10_i32)  // High depth to prove cycle detection works
    .fetch_all(&pool)
    .await
    .expect("graph_traverse with cycle");

    // Should find r2 and r3 but NOT revisit r1 (cycle detected)
    assert_eq!(rows.len(), 2);
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2));
    assert!(ids.contains(&r3));
}

/// graph_traverse with edge type filter only follows matching edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_typed_filter(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "typed@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;
    let r4 = common::fixtures::create_test_resource(&pool, profile, "R4", "r4").await;

    // r1 →(extends) r2 →(extends) r3
    // r1 →(references) r4
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r1, r4, "references", profile).await;

    // Filter to 'extends' only — should see r2, r3 but NOT r4
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT resource_id FROM graph_traverse($1, $2, $3, $4)",
    )
    .bind(profile)
    .bind(vec![r1])
    .bind(5_i32)
    .bind(vec!["extends"])
    .fetch_all(&pool)
    .await
    .expect("graph_traverse typed");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2));
    assert!(ids.contains(&r3));
    assert!(!ids.contains(&r4), "r4 connected via 'references' should be excluded");
}

/// path_weight decays multiplicatively along the traversal path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_path_weight_decay(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "weight@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;

    // r1 →(w=0.8) r2 →(w=0.6) r3
    // Insert with custom weights
    sqlx::query(
        "INSERT INTO kb_resource_edges (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         VALUES (gen_random_uuid(), $1, $2, 'extends', 0.8, '{}', $3)",
    )
    .bind(r1).bind(r2).bind(profile)
    .execute(&pool).await.expect("edge r1→r2");

    sqlx::query(
        "INSERT INTO kb_resource_edges (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         VALUES (gen_random_uuid(), $1, $2, 'extends', 0.6, '{}', $3)",
    )
    .bind(r2).bind(r3).bind(profile)
    .execute(&pool).await.expect("edge r2→r3");

    let rows: Vec<(uuid::Uuid, i32, f64)> = sqlx::query_as(
        "SELECT resource_id, depth, path_weight FROM graph_traverse($1, $2, 3, '{}')",
    )
    .bind(profile)
    .bind(vec![r1])
    .fetch_all(&pool)
    .await
    .expect("path weight query");

    for row in &rows {
        if row.0 == r2 {
            assert!((row.2 - 0.8).abs() < 0.001, "r2 path_weight should be 0.8, got {}", row.2);
        } else if row.0 == r3 {
            assert!((row.2 - 0.48).abs() < 0.001, "r3 path_weight should be 0.48, got {}", row.2);
        }
    }
}
```

- [ ] **Step 2: Run the traversal tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(graph_test)' 2>&1 | tail -30`

Expected: All 8 tests pass (4 from Task 6 + 4 new).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/graph_test.rs
git commit -m "test(graph): multi-hop traversal, cycle detection, typed filter, weight decay

Verifies graph_traverse() handles depth limits, cycles via path
array containment, edge type filtering, and multiplicative weight
decay. All against real Postgres."
```

---

## Task 8: Integration Tests — Visibility Scoping

**Files:**
- Modify: `crates/temper-api/tests/graph_test.rs` (append tests)

This is the most critical test: proving that profile A cannot traverse through resources owned by profile B. The graph functions compose with `resources_visible_to()`, so an invisible resource should block traversal — not just be omitted from results, but prevent further hops through it.

- [ ] **Step 1: Add visibility scoping tests**

Append to `crates/temper-api/tests/graph_test.rs`:

```rust
/// Traversal cannot see resources owned by another profile.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_visibility_blocks_other_profiles(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob@test.com").await;

    // Alice's resources
    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 1", "alice-1").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 2", "alice-2").await;

    // Bob's resource
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob Doc 1", "bob-1").await;

    // Alice's resource behind Bob's: a1 → b1 → a2
    // Alice can see a1, cannot see b1, so cannot reach a2 via this path
    common::fixtures::create_test_edge(&pool, a1, b1, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, b1, a2, "extends", alice).await;

    // Also give Alice a direct edge to a2 to verify she CAN see it directly
    common::fixtures::create_test_edge(&pool, a1, a2, "relates_to", alice).await;

    // Traverse from a1 as Alice with only 'extends' filter
    let rows: Vec<(uuid::Uuid, i32)> = sqlx::query_as(
        "SELECT resource_id, depth FROM graph_traverse($1, $2, 5, $3)",
    )
    .bind(alice)
    .bind(vec![a1])
    .bind(vec!["extends"])
    .fetch_all(&pool)
    .await
    .expect("alice traversal");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    // Alice should NOT see b1 (Bob's resource)
    assert!(!ids.contains(&b1), "Alice should not see Bob's resource");
    // Alice should NOT reach a2 via extends chain (blocked by b1)
    assert!(!ids.contains(&a2), "Alice should not reach a2 via extends through Bob's resource");

    // But with 'both' edge types including relates_to, a1→a2 direct edge works
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT resource_id FROM graph_traverse($1, $2, 5, '{}')",
    )
    .bind(alice)
    .bind(vec![a1])
    .fetch_all(&pool)
    .await
    .expect("alice traversal unfiltered");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&a2), "Alice should see a2 via direct relates_to edge");
    assert!(!ids.contains(&b1), "Alice still cannot see Bob's resource");
}

/// Neighbors function also respects visibility.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_visibility(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob@test.com").await;

    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc", "alice-doc").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 2", "alice-doc-2").await;
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob Doc", "bob-doc").await;

    common::fixtures::create_test_edge(&pool, a1, a2, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, a1, b1, "references", alice).await;

    // Alice's neighbors of a1 should NOT include b1
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT resource_id FROM graph_neighbors($1, $2, 'both', '{}')",
    )
    .bind(alice)
    .bind(a1)
    .fetch_all(&pool)
    .await
    .expect("alice neighbors");

    assert_eq!(rows.len(), 1, "only a2 should be visible");
    assert_eq!(rows[0].0, a2);
}

/// graph_resource_edges respects visibility — hidden peers are excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resource_edges_visibility(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob@test.com").await;

    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc", "alice-doc").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 2", "alice-doc-2").await;
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob Doc", "bob-doc").await;

    common::fixtures::create_test_edge(&pool, a1, a2, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, a1, b1, "references", alice).await;

    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT peer_resource_id, direction FROM graph_resource_edges($1, $2)",
    )
    .bind(alice)
    .bind(a1)
    .fetch_all(&pool)
    .await
    .expect("resource edges");

    assert_eq!(rows.len(), 1, "only a2 edge should be visible");
    assert_eq!(rows[0].0, a2);
}
```

- [ ] **Step 2: Run all graph tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(graph_test)' 2>&1 | tail -30`

Expected: All 11 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/graph_test.rs
git commit -m "test(graph): visibility scoping — traversal blocked by invisible resources

Proves that graph_traverse(), graph_neighbors(), and
graph_resource_edges() all compose correctly with
resources_visible_to(). Profile A cannot traverse through or see
edges to resources owned by profile B."
```

---

## Task 9: Integration Tests — Constraint Enforcement

**Files:**
- Modify: `crates/temper-api/tests/graph_test.rs` (append tests)

- [ ] **Step 1: Add constraint enforcement tests**

Append to `crates/temper-api/tests/graph_test.rs`:

```rust
/// Self-referencing edges are rejected by the check constraint.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_self_edge_rejected(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "self@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc", "doc").await;

    let result = sqlx::query(
        "INSERT INTO kb_resource_edges (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         VALUES (gen_random_uuid(), $1, $1, 'relates_to', 1.0, '{}', $2)",
    )
    .bind(r1)
    .bind(profile)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "self-edge should be rejected by chk_no_self_edge");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("chk_no_self_edge"), "error should mention the constraint: {err}");
}

/// Duplicate edges (same source, target, type) are rejected by unique constraint.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_duplicate_edge_rejected(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "dupe@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Same source → target → type should fail
    let result = sqlx::query(
        "INSERT INTO kb_resource_edges (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         VALUES (gen_random_uuid(), $1, $2, 'extends', 1.0, '{}', $3)",
    )
    .bind(r1)
    .bind(r2)
    .bind(profile)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "duplicate edge should be rejected");

    // Different edge type between same resources should succeed
    common::fixtures::create_test_edge(&pool, r1, r2, "references", profile).await;
}

/// Edges cascade-delete when a resource is deleted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_edge_cascade_on_resource_delete(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "cascade@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "depends_on", profile).await;

    // Delete r2 — both edges touching r2 should cascade
    sqlx::query("DELETE FROM kb_resources WHERE id = $1")
        .bind(r2)
        .execute(&pool)
        .await
        .expect("delete resource");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kb_resource_edges")
        .fetch_one(&pool)
        .await
        .expect("count edges");

    assert_eq!(count.0, 0, "all edges touching deleted resource should be gone");
}
```

- [ ] **Step 2: Run all graph tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(graph_test)' 2>&1 | tail -30`

Expected: All 14 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/graph_test.rs
git commit -m "test(graph): constraint enforcement — self-edge, duplicate, cascade delete

Verifies chk_no_self_edge, uq_resource_edge, and ON DELETE CASCADE
all work as designed."
```

---

## Task 10: Regenerate sqlx Cache and Full Verification

**Files:**
- Modify: `.sqlx/` cache files (auto-generated)

- [ ] **Step 1: Regenerate the sqlx offline cache**

Run: `cargo sqlx prepare --workspace -- --all-features 2>&1 | tail -10`

Expected: Cache regenerated successfully.

- [ ] **Step 2: Run full check suite**

Run: `cargo make check 2>&1 | tail -30`

Expected: Clean — no clippy warnings, formatting correct, docs build.

- [ ] **Step 3: Run all tests (Rust unit + integration)**

Run: `cargo make test 2>&1 | tail -20`

Expected: All unit tests pass.

Run: `cargo nextest run -p temper-api --features test-db 2>&1 | tail -30`

Expected: All integration tests pass (existing + new graph tests).

- [ ] **Step 4: Commit the sqlx cache**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx offline cache for graph migration"
```

- [ ] **Step 5: Final commit — squash or leave as-is based on preference**

All Phase 1 work is complete. The branch `jct/knowledge-graph-foundations` has:
- Migration: `edge_type` enum, `kb_resource_edges`, `kb_deferred_edges` tables
- Migration: `graph_traverse()`, `graph_neighbors()`, `graph_resource_edges()` SQL functions
- Rust types: `EdgeType`, `TargetRef`, `ResourceRelationships`, `GraphTraversalRow`, `GraphNeighborRow`, `GraphEdgeRow`, `ResolvedEdge`, `EdgeReconciliation`
- Unit tests: `TargetRef::parse`, `ResourceRelationships` serde + edge extraction
- Integration tests: edge CRUD, neighbor queries, multi-hop traversal, cycle detection, typed filtering, path weight decay, visibility scoping, constraint enforcement
