-- C3: expose kb_edges.id from graph_traverse_scoped so rendered AtlasEdges can be
-- addressed for R5 edge trails (readTrail('edge', id)). Additive `id` column; body
-- otherwise unchanged from the shipped 20260703130000 definition (shipped migrations
-- stay immutable — this DROP/CREATEs a fresh function since RETURNS TABLE shape
-- cannot be altered via CREATE OR REPLACE).
DROP FUNCTION IF EXISTS graph_traverse_scoped(uuid, uuid, uuid[], int, edge_kind[]);

CREATE FUNCTION graph_traverse_scoped(
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
    walk AS (
        SELECT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, 1 AS depth
        FROM kb_edges e
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND e.source_id = ANY(p_seed_ids)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
        UNION
        SELECT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, w.depth + 1
        FROM kb_edges e
        JOIN walk w ON e.source_id = w.target_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND w.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    -- DISTINCT: `walk`'s UNION dedups on the full row *including* depth, so an
    -- edge reachable at two different depths (realistic with multiple seeds)
    -- would otherwise survive as two rows once `depth` is dropped here. `id` is
    -- functionally determined by the same kb_edges row as every other selected
    -- column, so adding it does not change this dedup's behavior.
    SELECT DISTINCT id, source_id, target_id, edge_kind, polarity, label, weight FROM walk;
$$;
