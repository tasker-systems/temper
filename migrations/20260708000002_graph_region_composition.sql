-- Beat D: region → resources COMPOSITION read. Unlike graph_traverse_cogmap_scoped
-- (20260706120200), the walk is NOT fenced to a cogmap's homed resources — it is
-- seeded by region members (facets) and follows visible edges out to context-homed
-- resources (the builder axis). The full edge-visibility predicate is reproduced
-- conjunct-for-conjunct: both endpoints in resources_visible_to, NOT is_folded, and
-- the edge's home anchor readable. This is the read that makes "the ideas + the work
-- they were derived_from" one graph.

CREATE FUNCTION graph_region_composition_edges(
    p_profile    uuid,
    p_region_ids uuid[],
    p_depth      int
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    seeds AS (  -- region members visible to the caller = the knowledge-axis seeds
        SELECT DISTINCT m.member_id AS id
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.id = m.member_id
        WHERE m.region_id = ANY(p_region_ids)
    ),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END, r.depth + 1
        FROM reached r
        JOIN kb_edges e ON (e.source_id = r.node_id OR e.target_id = r.node_id)
        JOIN vis vs ON vs.id = e.source_id
        JOIN vis vt ON vt.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND r.depth < LEAST(p_depth, 3)
    )
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
    FROM kb_edges e
    JOIN reached rs ON rs.node_id = e.source_id
    JOIN reached rt ON rt.node_id = e.target_id
    JOIN vis vs ON vs.id = e.source_id
    JOIN vis vt ON vt.id = e.target_id
    WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id);
$$;

-- Node rows for an arbitrary id set, each gated through resources_visible_to (a
-- non-cogmap-scoped analog of graph_atlas_nodes_cogmap). home = 'cogmap' if the
-- resource has any kb_cogmaps home, else 'context' — this drives the mark shape.
CREATE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
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
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
