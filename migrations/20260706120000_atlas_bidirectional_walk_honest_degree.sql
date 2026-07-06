-- A1: make the neighborhood walk bidirectional and the node degree honest.
--
-- Body-only changes; RETURNS TABLE shapes are unchanged from their current defs
-- (graph_traverse_scoped: 20260704000009; graph_atlas_nodes: 20260706000001), so
-- CREATE OR REPLACE is valid. Shipped migrations stay immutable.
--
-- SECURITY INVARIANT preserved conjunct-for-conjunct: every edge still requires
-- both endpoints in team scope (⊆ resources_visible_to), NOT is_folded, and the
-- edge's home anchor readable. Widening direction does NOT widen the visibility
-- set — team scope is the security boundary and is unchanged.

CREATE OR REPLACE FUNCTION graph_traverse_scoped(
    p_profile     uuid,
    p_team        uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    -- BFS over the frontier NODE set, following in-scope visible edges in EITHER
    -- direction; the opposite endpoint becomes the next frontier. UNION dedups.
    reached AS (
        SELECT unnest(p_seed_ids) AS node_id, 0 AS depth
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END, r.depth + 1
        FROM reached r
        JOIN kb_edges e
          ON (e.source_id = r.node_id OR e.target_id = r.node_id)
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND r.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    -- Return every visible in-scope edge whose BOTH endpoints were reached.
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
    FROM kb_edges e
    JOIN reached rs ON rs.node_id = e.source_id
    JOIN reached rt ON rt.node_id = e.target_id
    JOIN scope ss ON ss.id = e.source_id
    JOIN scope st ON st.id = e.target_id
    WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
      AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
           OR e.edge_kind = ANY(p_edge_kinds));
$$;

CREATE OR REPLACE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk
    FROM ids
    JOIN scope s   ON s.id = ids.id
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    -- HONEST DEGREE: clamp both endpoints to team scope so the count equals what the
    -- (team-scoped) walk can render. Was: edges_visible_to only, which over-counted
    -- cross-scope / cross-team edges the walk can never show.
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        JOIN scope se  ON se.id  = e.source_id
        JOIN scope se2 ON se2.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
