-- R4 Atlas neighborhood slice: team-scoped, edge-kind-filtered traversal + node projection.
-- Composes resources_in_team_scope (team clamp) with a graph_traverse-shaped recursive walk.
--
-- Design: docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md (read model R4).
-- Reference shape: graph_traverse (migrations/20260624000002_canonical_functions.sql) — this keeps
-- the recursive-CTE shape but swaps the profile-visibility CTE for a team-scope CTE
-- (resources_in_team_scope), adds an edge-kind filter to both the seed and recursive arms,
-- returns weight, and joins via the `scope` CTE rather than `IN (SELECT …)`.

-- Scoped, edge-kind-filtered directed walk. p_edge_kinds empty/NULL => all kinds.
CREATE FUNCTION graph_traverse_scoped(
    p_profile     uuid,
    p_team        uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision, depth int
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    walk AS (
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, 1 AS depth
        FROM kb_edges e
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND e.source_id = ANY(p_seed_ids)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
        UNION
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, w.depth + 1
        FROM kb_edges e
        JOIN walk w ON e.source_id = w.target_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND w.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    SELECT source_id, target_id, edge_kind, polarity, label, weight, depth FROM walk;
$$;

-- Project Atlas node attributes for a set of ids, clamped to team scope.
-- doc_type is LEFT-joined (nullable). home = cogmap if any cogmap home exists, else context.
CREATE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int)
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
           COALESCE(deg.degree, 0) AS degree
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
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
