-- =============================================================================
-- R7 Phase 1: Knowledge Graph — Edge Tables
-- =============================================================================
-- Adds: edge_type enum, kb_resource_edges table, kb_deferred_edges table
--
-- Design: Native Postgres adjacency list. kb_resources rows are vertices.
-- Edges are directed with semantic types. Forward references (unresolvable
-- targets during bulk import) are held in kb_deferred_edges until the target
-- resource is created.
--
-- NOTE: DEFAULT gen_random_uuid() produces UUIDv4 which scatters on B-tree
-- indexes. All Rust insertion paths MUST set UUIDv7 explicitly via
-- Uuid::now_v7(). The default is a safety net for raw SQL only.

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
