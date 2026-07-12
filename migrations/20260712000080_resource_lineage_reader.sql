-- Ledger L2 — the lineage reader (read-only, additive).
--
-- Given a resource, walk the `derived_from` lineage graph in one direction and
-- return the reachable resources, access-gated per edge exactly as the R5
-- element trail is (`element_trail_edge`, 20260706000002): an edge is traversable
-- only when its HOME is readable AND both endpoints are independently readable
-- (`edges_visible_to` minus the folded filter — a folded lineage edge still shows,
-- flagged, because "you rest on a superseded ancestor" is the point of the read).
--
-- L1 (2026-07-12 decision) settled the graph: lineage lives on `derived_from`
-- EDGES, not `kb_events.references` (0 of 10,148 prod events carry references).
-- The one load-bearing subtlety: `derived_from` is projected under TWO edge_kinds
-- — `(express, forward)` and `(leads_to, inverse)` — so the walk keys on the
-- LABEL, never on edge_kind. Keying on edge_kind='leads_to' would silently drop
-- the ~2/3 of the graph that is `express`. Direction is uniform across both
-- shapes: for a `derived_from` edge, source = deriver/descendant, target =
-- ancestor/source-material (verified against prod).
--
-- Direction param:
--   'ancestors'   — "what does this derive from"  (follow source=seed → target)
--   'descendants' — "what derives from this"       (follow target=seed → source)
--
-- Cycle-safe (path array), depth-bounded, and returns the SHALLOWEST depth at
-- which each resource is reached. Additive: a new function, no schema change.
CREATE FUNCTION resource_lineage(
    p_profile uuid,
    p_resource uuid,
    p_direction text,
    p_max_depth int DEFAULT 16
) RETURNS TABLE(
    resource_id uuid,
    title text,
    is_active boolean,
    edge_id uuid,
    edge_is_folded boolean,
    depth int
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE walk AS (
        -- Seed level: the seed's direct `derived_from` neighbours in the requested
        -- direction. The seed itself is gated by the caller (service does the 404),
        -- but every edge here is independently gated, so an unreadable seed simply
        -- yields no rows.
        SELECT
            (CASE WHEN p_direction = 'ancestors' THEN e.target_id ELSE e.source_id END) AS resource_id,
            e.id AS edge_id,
            e.is_folded AS edge_is_folded,
            1 AS depth,
            ARRAY[p_resource,
                  (CASE WHEN p_direction = 'ancestors' THEN e.target_id ELSE e.source_id END)] AS path
        FROM kb_edges e
        WHERE e.label = 'derived_from'
          AND e.source_table = 'kb_resources'
          AND e.target_table = 'kb_resources'
          AND ( (p_direction = 'ancestors'   AND e.source_id = p_resource)
             OR (p_direction = 'descendants' AND e.target_id = p_resource) )
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND endpoint_readable_by_profile(p_profile, e.source_table, e.source_id)
          AND endpoint_readable_by_profile(p_profile, e.target_table, e.target_id)

        UNION ALL

        -- Recursive step: one more hop in the same direction, off each frontier node.
        SELECT
            (CASE WHEN p_direction = 'ancestors' THEN e.target_id ELSE e.source_id END),
            e.id,
            e.is_folded,
            w.depth + 1,
            w.path || (CASE WHEN p_direction = 'ancestors' THEN e.target_id ELSE e.source_id END)
        FROM walk w
        JOIN kb_edges e
          ON e.label = 'derived_from'
         AND e.source_table = 'kb_resources'
         AND e.target_table = 'kb_resources'
         AND ( (p_direction = 'ancestors'   AND e.source_id = w.resource_id)
            OR (p_direction = 'descendants' AND e.target_id = w.resource_id) )
        WHERE w.depth < p_max_depth
          -- cycle guard: never revisit a node already on this path
          AND (CASE WHEN p_direction = 'ancestors' THEN e.target_id ELSE e.source_id END) <> ALL(w.path)
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND endpoint_readable_by_profile(p_profile, e.source_table, e.source_id)
          AND endpoint_readable_by_profile(p_profile, e.target_table, e.target_id)
    )
    -- Collapse multiple reach-paths to one row per resource, keeping the shallowest
    -- depth (and, at that depth, one representative edge).
    SELECT DISTINCT ON (w.resource_id)
        w.resource_id,
        r.title,
        r.is_active,
        w.edge_id,
        w.edge_is_folded,
        w.depth
    FROM walk w
    JOIN kb_resources r ON r.id = w.resource_id
    ORDER BY w.resource_id, w.depth;
$$;
