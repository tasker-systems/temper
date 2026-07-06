-- A2: cogmap-scoped R4 neighborhood stack — the cogmap-door analog of the team stack.
-- Scope = resources homed in THIS cogmap that are visible to the profile. Entry gate
-- (cogmap_readable_by_profile) is enforced in the service. Full edge-visibility
-- predicate reproduced conjunct-for-conjunct (both endpoints in cogmap scope ⊆
-- resources_visible_to, NOT is_folded, home anchor readable).

CREATE FUNCTION resources_in_cogmap_scope(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT h.resource_id
    FROM kb_resource_homes h
    JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
    WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap;
$$;

CREATE FUNCTION graph_traverse_cogmap_scoped(
    p_profile     uuid,
    p_cogmap      uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_cogmap_scope(p_profile, p_cogmap)
    ),
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

CREATE FUNCTION graph_atlas_nodes_cogmap(
    p_profile uuid, p_cogmap uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_cogmap_scope(p_profile, p_cogmap)
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
