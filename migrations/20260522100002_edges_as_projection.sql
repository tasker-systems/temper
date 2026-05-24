-- Edges as projection — phase-2 schema cutover for limb 1.
-- kb_resource_edges becomes a projection of the relationship-event stream.
-- Spec: docs/superpowers/specs/2026-05-22-limb1-relationship-events-edge-projection-design.md

-- ─── 1. Synthesize a genesis relationship_asserted event per existing edge ──
-- Pre-existing edges must become real ledger history, or a full rebuild loses
-- them. emitter = created_by_profile_id; occurred_at = the edge's created time.
-- edge_kind / polarity / label come from the 8->4 legacy mapping; the payload
-- shape matches temper_core::types::relationship_events::RelationshipAsserted.
INSERT INTO kb_events (
    id, event_type_id, profile_id, device_id, topic_id, scope_id,
    payload, metadata, "references", correlation_id, occurred_at, created
)
SELECT
    public.uuid_generate_v7(),
    (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
    e.created_by_profile_id,
    'migration',
    '019e3d6f-2300-7000-8000-000000000050',  -- declaration topic
    '019e3d6f-2300-7000-8000-000000000010',  -- public scope
    jsonb_build_object(
        'source_resource_id', e.source_resource_id,
        'target', jsonb_build_object('kind', 'resource', 'value', e.target_resource_id),
        'edge_kind', m.edge_kind,
        'polarity',  m.polarity,
        'label',     m.label,
        'weight',    e.weight
    ),
    jsonb_build_object('intent', 'migration'),
    '[]'::jsonb,
    public.uuid_generate_v7(),
    e.created,
    e.created
FROM kb_resource_edges e
CROSS JOIN LATERAL (
    -- 8->4 mapping, matching EdgeType::legacy_mapping() in temper-core.
    -- `tagged_with` is a defensive branch: the enum has the label but no live
    -- rows or Rust variant use it; map to express/forward to be safe.
    SELECT
        CASE e.edge_type
            WHEN 'parent_of'    THEN 'contains'
            WHEN 'tagged_with'  THEN 'express'
            WHEN 'depends_on'   THEN 'leads_to'
            WHEN 'preceded_by'  THEN 'leads_to'
            WHEN 'derived_from' THEN 'leads_to'
            WHEN 'extends'      THEN 'leads_to'
            WHEN 'relates_to'   THEN 'near'
            WHEN 'references'   THEN 'near'
        END AS edge_kind,
        CASE e.edge_type
            WHEN 'depends_on'   THEN 'inverse'
            WHEN 'preceded_by'  THEN 'inverse'
            WHEN 'derived_from' THEN 'inverse'
            WHEN 'extends'      THEN 'inverse'
            ELSE 'forward'
        END AS polarity,
        e.edge_type::text AS label
) m;

-- ─── 2. Evolve kb_resource_edges into the projection shape ──────────────────
ALTER TABLE kb_resource_edges
    ADD COLUMN edge_kind            edge_kind,
    ADD COLUMN polarity             edge_polarity,
    ADD COLUMN label                text,
    ADD COLUMN asserted_by_event_id uuid REFERENCES kb_events(id),
    ADD COLUMN last_event_id        uuid REFERENCES kb_events(id),
    ADD COLUMN is_folded            boolean NOT NULL DEFAULT false;

-- Backfill the new columns from the legacy edge_type before making them NOT NULL.
UPDATE kb_resource_edges e SET
    edge_kind = (CASE edge_type
        WHEN 'parent_of' THEN 'contains' WHEN 'tagged_with' THEN 'express'
        WHEN 'depends_on' THEN 'leads_to' WHEN 'preceded_by' THEN 'leads_to'
        WHEN 'derived_from' THEN 'leads_to' WHEN 'extends' THEN 'leads_to'
        WHEN 'relates_to' THEN 'near' WHEN 'references' THEN 'near' END)::edge_kind,
    polarity = (CASE edge_type
        WHEN 'depends_on' THEN 'inverse' WHEN 'preceded_by' THEN 'inverse'
        WHEN 'derived_from' THEN 'inverse' WHEN 'extends' THEN 'inverse'
        ELSE 'forward' END)::edge_polarity,
    label = edge_type::text;

-- asserted_by_event_id / last_event_id link each surviving edge row to its
-- genesis event. Match on the synthesized payload's source+target+label.
UPDATE kb_resource_edges e SET
    asserted_by_event_id = ev.id,
    last_event_id        = ev.id
FROM kb_events ev
WHERE ev.event_type_id = (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted')
  AND ev.device_id = 'migration'
  AND (ev.payload->>'source_resource_id')::uuid = e.source_resource_id
  AND (ev.payload->'target'->>'value')::uuid     = e.target_resource_id
  AND ev.payload->>'label'                       = e.edge_type::text;

ALTER TABLE kb_resource_edges
    ALTER COLUMN edge_kind            SET NOT NULL,
    ALTER COLUMN polarity             SET NOT NULL,
    ALTER COLUMN label                SET NOT NULL,
    ALTER COLUMN asserted_by_event_id SET NOT NULL,
    ALTER COLUMN last_event_id        SET NOT NULL;

-- ─── 3. Drop graph functions referencing the legacy enum / columns ──────────
-- These must drop BEFORE the enum/columns they depend on. Recreated below
-- against the new column shape.
DROP FUNCTION IF EXISTS graph_traverse(UUID, UUID[], INT, TEXT[]);
DROP FUNCTION IF EXISTS graph_neighbors(UUID, UUID, VARCHAR, TEXT[]);
DROP FUNCTION IF EXISTS graph_resource_edges(UUID, UUID);

-- ─── 4. Drop kb_deferred_edges (Gate 3 — replaced by slug-target assertions) ─
-- Must happen before DROP TYPE edge_type because the table has a column of
-- that type.
DROP TABLE kb_deferred_edges;

-- ─── 5. Drop the legacy columns / constraint / enum ─────────────────────────
ALTER TABLE kb_resource_edges DROP CONSTRAINT uq_resource_edge;
ALTER TABLE kb_resource_edges
    DROP COLUMN edge_type,
    DROP COLUMN created_by_profile_id,
    DROP COLUMN metadata;
DROP TYPE edge_type;

ALTER TABLE kb_resource_edges
    ADD CONSTRAINT uq_resource_edge
    UNIQUE (source_resource_id, target_resource_id, edge_kind, label, polarity);

CREATE INDEX idx_edges_asserted_by_event ON kb_resource_edges(asserted_by_event_id);
CREATE INDEX idx_edges_not_folded ON kb_resource_edges(source_resource_id, target_resource_id)
    WHERE NOT is_folded;

-- ─── 6. Recreate graph functions for the new column shape ──────────────────
-- graph_traverse: edge_type column → edge_kind. RETURNS edge_kind/polarity/label
-- (drop edge_type). Add NOT is_folded predicate. The p_edge_types filter
-- compares edge_kind::text.
CREATE FUNCTION graph_traverse(
    p_profile_id  UUID,
    p_seed_ids    UUID[],
    p_max_depth   INT DEFAULT 3,
    p_edge_types  TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id       UUID,
    depth             INT,
    path              UUID[],
    edge_kind         edge_kind,
    polarity          edge_polarity,
    label             TEXT,
    from_resource_id  UUID,
    path_weight       FLOAT
)
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE
      seed_visible AS (
        SELECT v.resource_id
          FROM resources_visible_to(p_profile_id, NULL, p_seed_ids) v
         WHERE v.resource_id = ANY(p_seed_ids)
      ),
      visible AS (
        SELECT v.resource_id
          FROM resources_visible_to(p_profile_id, NULL, '{}') v
      ),
      traversal AS (
        -- Base case: seed resources (must be visible)
        SELECT
          sv.resource_id,
          0 AS depth,
          ARRAY[sv.resource_id] AS path,
          NULL::edge_kind AS edge_kind,
          NULL::edge_polarity AS polarity,
          NULL::TEXT AS label,
          NULL::UUID AS from_resource_id,
          1.0::FLOAT AS path_weight
        FROM seed_visible sv

        UNION ALL

        -- Recursive case: expand one hop forward
        SELECT
          e.target_resource_id,
          t.depth + 1,
          t.path || e.target_resource_id,
          e.edge_kind,
          e.polarity,
          e.label,
          t.resource_id,
          t.path_weight * e.weight
        FROM traversal t
        JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
        JOIN visible v ON v.resource_id = e.target_resource_id
        WHERE t.depth < p_max_depth
          AND NOT e.is_folded
          AND NOT e.target_resource_id = ANY(t.path)
          AND (p_edge_types = '{}' OR e.edge_kind::TEXT = ANY(p_edge_types))
      )
    SELECT DISTINCT ON (t.resource_id)
      t.resource_id,
      t.depth,
      t.path,
      t.edge_kind,
      t.polarity,
      t.label,
      t.from_resource_id,
      t.path_weight
    FROM traversal t
    WHERE t.depth > 0
    ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC
$$;

-- graph_neighbors: edge_type → edge_kind; drop metadata; add polarity+label;
-- exclude folded.
CREATE FUNCTION graph_neighbors(
    p_profile_id   UUID,
    p_resource_id  UUID,
    p_direction    VARCHAR DEFAULT 'both',
    p_edge_types   TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id  UUID,
    edge_kind    edge_kind,
    polarity     edge_polarity,
    label        TEXT,
    direction    VARCHAR,
    weight       FLOAT
)
LANGUAGE SQL STABLE AS $$
    SELECT
      e.target_resource_id AS resource_id,
      e.edge_kind,
      e.polarity,
      e.label,
      'outgoing'::VARCHAR AS direction,
      e.weight
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id
      AND NOT e.is_folded
      AND p_direction IN ('both', 'outgoing')
      AND (p_edge_types = '{}' OR e.edge_kind::TEXT = ANY(p_edge_types))

    UNION ALL

    SELECT
      e.source_resource_id AS resource_id,
      e.edge_kind,
      e.polarity,
      e.label,
      'incoming'::VARCHAR AS direction,
      e.weight
    FROM kb_resource_edges e
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id
      AND NOT e.is_folded
      AND p_direction IN ('both', 'incoming')
      AND (p_edge_types = '{}' OR e.edge_kind::TEXT = ANY(p_edge_types))
$$;

-- graph_resource_edges: edge_type → edge_kind; drop metadata; add polarity+label;
-- exclude folded.
CREATE FUNCTION graph_resource_edges(
    p_profile_id   UUID,
    p_resource_id  UUID
) RETURNS TABLE (
    edge_id           UUID,
    peer_resource_id  UUID,
    peer_title        TEXT,
    peer_slug         VARCHAR(256),
    edge_kind         edge_kind,
    polarity          edge_polarity,
    label             TEXT,
    direction         VARCHAR,
    weight            FLOAT,
    created           TIMESTAMPTZ
)
LANGUAGE SQL STABLE AS $$
    SELECT
      e.id AS edge_id,
      e.target_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_kind,
      e.polarity,
      e.label,
      'outgoing'::VARCHAR AS direction,
      e.weight,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.target_resource_id
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.target_resource_id
    WHERE e.source_resource_id = p_resource_id
      AND NOT e.is_folded

    UNION ALL

    SELECT
      e.id AS edge_id,
      e.source_resource_id AS peer_resource_id,
      r.title AS peer_title,
      r.slug AS peer_slug,
      e.edge_kind,
      e.polarity,
      e.label,
      'incoming'::VARCHAR AS direction,
      e.weight,
      e.created
    FROM kb_resource_edges e
    JOIN kb_resources r ON r.id = e.source_resource_id
    JOIN resources_visible_to(p_profile_id, NULL, '{}') v
      ON v.resource_id = e.source_resource_id
    WHERE e.target_resource_id = p_resource_id
      AND NOT e.is_folded

    ORDER BY edge_kind, direction, created
$$;

-- graph_subgraph_nodes: only needs NOT is_folded on the peer_edges scans.
CREATE OR REPLACE FUNCTION graph_subgraph_nodes(
    p_profile_id        UUID,
    p_context_name      VARCHAR,
    p_aggregator_types  TEXT[],
    p_depth             INT
) RETURNS TABLE (
    resource_id    UUID,
    slug           VARCHAR(256),
    title          TEXT,
    doc_type       VARCHAR(64),
    edge_count     INT,
    session_count  INT,
    first_chunk    TEXT,
    stage_raw      TEXT
)
LANGUAGE SQL STABLE AS $$
    WITH seed_concepts AS (
        SELECT r.id
          FROM kb_resources r
          JOIN resources_visible_to(p_profile_id, NULL, '{}') v ON v.resource_id = r.id
          JOIN kb_contexts c   ON c.id  = r.kb_context_id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         WHERE c.name = p_context_name
           AND dt.name = ANY(p_aggregator_types)
           AND r.is_active = true
    ),
    traversed AS (
        SELECT gt.resource_id AS id
          FROM graph_traverse(
              p_profile_id,
              ARRAY(SELECT id FROM seed_concepts),
              p_depth,
              '{}'::text[]
          ) gt
    ),
    candidate_ids AS (
        SELECT id FROM seed_concepts
        UNION
        SELECT id FROM traversed
    ),
    peer_edges AS (
        SELECT e.source_resource_id AS node_id,
               e.target_resource_id AS peer_id
          FROM kb_resource_edges e
          JOIN candidate_ids c ON c.id = e.source_resource_id
         WHERE NOT e.is_folded
        UNION ALL
        SELECT e.target_resource_id AS node_id,
               e.source_resource_id AS peer_id
          FROM kb_resource_edges e
          JOIN candidate_ids c ON c.id = e.target_resource_id
         WHERE NOT e.is_folded
    ),
    peer_edges_annotated AS (
        SELECT pe.node_id,
               pe.peer_id,
               peer_dt.name AS peer_type,
               peer_r.is_active AS peer_active
          FROM peer_edges pe
          JOIN kb_resources peer_r  ON peer_r.id = pe.peer_id
          JOIN kb_doc_types peer_dt ON peer_dt.id = peer_r.kb_doc_type_id
    ),
    edge_counts AS (
        SELECT pea.node_id,
               COUNT(*)::int AS edge_count,
               COUNT(DISTINCT pea.peer_id)
                 FILTER (WHERE pea.peer_type = 'session' AND pea.peer_active)::int
                 AS session_count
          FROM peer_edges_annotated pea
         GROUP BY pea.node_id
    ),
    first_chunks AS (
        SELECT DISTINCT ON (cc.resource_id)
               cc.resource_id,
               cc.content
          FROM kb_current_chunks cc
          JOIN candidate_ids c ON c.id = cc.resource_id
         ORDER BY cc.resource_id, cc.chunk_index
    )
    SELECT
        r.id                         AS resource_id,
        r.slug,
        r.title,
        dt.name::VARCHAR(64)         AS doc_type,
        COALESCE(ec.edge_count, 0)   AS edge_count,
        COALESCE(ec.session_count,0) AS session_count,
        fc.content                   AS first_chunk,
        m.managed_meta->>'temper-stage' AS stage_raw
      FROM kb_resources r
      JOIN kb_doc_types dt   ON dt.id = r.kb_doc_type_id
      JOIN candidate_ids c   ON c.id = r.id
      LEFT JOIN edge_counts ec  ON ec.node_id = r.id
      LEFT JOIN first_chunks fc ON fc.resource_id = r.id
      LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
     WHERE r.is_active = true
       AND dt.name <> 'session'
$$;

-- graph_search: depends on graph_traverse's return shape via columns it
-- references — it only uses resource_id, depth, path_weight, so its body
-- needs no change. CREATE OR REPLACE'd in 20260420000003 with the same body
-- we want, so no work needed here.
