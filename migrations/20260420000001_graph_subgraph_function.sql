-- =============================================================================
-- C1: graph_subgraph_nodes() — CTE-aggregated aggregator subgraph
-- =============================================================================
-- Replaces the four correlated subqueries in
-- `graph_service::aggregator_subgraph` Query 1 with CTE aggregations and
-- LEFT JOINs. Packaged as a SQL function so the planner caches the plan
-- across calls (this runs on every graph-panel refresh).
--
-- Semantics match the prior inline query exactly:
--   * edge_count: every row in kb_resource_edges incident to the node
--     (source OR target), direction-agnostic. Not visibility-filtered —
--     matches the existing "edge_count reflects total not subgraph"
--     contract used by graph_subgraph_test.rs.
--   * session_count: distinct *active* session-typed peers sharing any
--     edge with the node. Not visibility-filtered — matches the existing
--     "sessions are annotations, not participants" contract.
--   * first_chunk: body text of the chunk with the lowest chunk_index on
--     the current version.
--   * stage_raw: managed_meta->>'temper-stage' from the manifest; the
--     Rust caller gates this to the 'task' doctype.
--
-- Session and inactive resources are filtered out of the returned node set
-- in the final SELECT, so the Rust-side edge query can safely assume every
-- returned id is a valid, non-session node.

CREATE FUNCTION graph_subgraph_nodes(
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
    -- Emit one row per edge incident to a candidate node — outgoing
    -- (source = node) and incoming (target = node) unioned so a node's
    -- total degree is COUNT(*) over this CTE.
    peer_edges AS (
        SELECT e.source_resource_id AS node_id,
               e.target_resource_id AS peer_id
          FROM kb_resource_edges e
          JOIN candidate_ids c ON c.id = e.source_resource_id
        UNION ALL
        SELECT e.target_resource_id AS node_id,
               e.source_resource_id AS peer_id
          FROM kb_resource_edges e
          JOIN candidate_ids c ON c.id = e.target_resource_id
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
