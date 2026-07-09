-- Beat E: the context-graph reads. Builder-axis sibling of graph_region_composition
-- (20260708000002), from which the edge-visibility predicate is reproduced
-- conjunct-for-conjunct: both endpoints in resources_visible_to, NOT is_folded, and the
-- edge's home anchor readable.
--
-- INVARIANT: the container walk filters on NO edge label and NO direction. Goal->task
-- membership is recorded as `parent_of` (contains, goal->task) historically and as
-- `advances` (leads_to, task->goal) currently; migration 20260709000005 converts the
-- former to the latter. A label- or direction-filtered walk empties every territory the
-- day that backfill lands. Undirected + label-blind makes member counts invariant.

-- Container territories: resources of `p_container_types` homed in the context, sized by
-- how many distinct resources they reach within p_depth over VISIBLE internal edges.
CREATE FUNCTION graph_context_containers(
    p_profile         uuid,
    p_context_id      uuid,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(id uuid, label text, member_count int) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
           AND NOT p.is_folded
    ),
    ctx AS (  -- context-homed, active, visible. deny-as-absence via the vis join.
        SELECT r.id, r.title, d.dt
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
                                  AND h.anchor_table = 'kb_contexts'
                                  AND h.anchor_id = p_context_id
          JOIN vis v ON v.id = r.id
          LEFT JOIN doc d ON d.rid = r.id
         WHERE r.is_active
    ),
    ie AS (  -- internal edges, both endpoints visible + in-context, edge home readable
        SELECT e.source_id, e.target_id
          FROM kb_edges e
          JOIN ctx s ON s.id = e.source_id
          JOIN ctx t ON t.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
    ),
    containers AS (SELECT c.id, c.title FROM ctx c WHERE c.dt = ANY(p_container_types)),
    reached AS (
        SELECT c.id AS root, c.id AS node_id, 0 AS depth FROM containers c
        UNION
        SELECT r.root,
               CASE WHEN ie.source_id = r.node_id THEN ie.target_id ELSE ie.source_id END,
               r.depth + 1
          FROM reached r
          JOIN ie ON (ie.source_id = r.node_id OR ie.target_id = r.node_id)
         WHERE r.depth < LEAST(p_depth, 3)
    )
    SELECT c.id,
           c.title AS label,
           (SELECT count(DISTINCT rr.node_id)::int - 1 FROM reached rr WHERE rr.root = c.id)
      FROM containers c;
$$;

-- Residual = context-homed + visible, reaching NO container. Grouped by an arbitrary
-- kb_properties key, so the bucket set is derived from data, never enumerated.
CREATE FUNCTION graph_context_residual_counts(
    p_profile         uuid,
    p_context_id      uuid,
    p_group_key       text,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(group_value text, member_count int) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
           AND NOT p.is_folded
    ),
    ctx AS (
        SELECT r.id, d.dt
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
                                  AND h.anchor_table = 'kb_contexts'
                                  AND h.anchor_id = p_context_id
          JOIN vis v ON v.id = r.id
          LEFT JOIN doc d ON d.rid = r.id
         WHERE r.is_active
    ),
    ie AS (
        SELECT e.source_id, e.target_id
          FROM kb_edges e
          JOIN ctx s ON s.id = e.source_id
          JOIN ctx t ON t.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
    ),
    reached AS (
        SELECT c.id AS node_id, 0 AS depth FROM ctx c WHERE c.dt = ANY(p_container_types)
        UNION
        SELECT CASE WHEN ie.source_id = r.node_id THEN ie.target_id ELSE ie.source_id END,
               r.depth + 1
          FROM reached r
          JOIN ie ON (ie.source_id = r.node_id OR ie.target_id = r.node_id)
         WHERE r.depth < LEAST(p_depth, 3)
    ),
    grp AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS gv
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = p_group_key
           AND NOT p.is_folded
    )
    SELECT COALESCE(g.gv, '(none)') AS group_value, count(*)::int
      FROM ctx c
      LEFT JOIN grp g ON g.rid = c.id
     WHERE c.id NOT IN (SELECT node_id FROM reached)
     GROUP BY 1
     ORDER BY 2 DESC;
$$;

-- The ids behind one residual bucket — the seeds for its drill.
CREATE FUNCTION graph_context_residual_members(
    p_profile         uuid,
    p_context_id      uuid,
    p_group_key       text,
    p_group_value     text,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(id uuid) LANGUAGE sql STABLE AS $$
    -- The member ids behind ONE bucket of graph_context_residual_counts. The two functions are
    -- two spellings of one set, so the exclusion below must be the identical depth-bounded walk
    -- the counts function performs -- NOT a 1-hop neighbor check. A resource two hops from a
    -- container is contained at p_depth >= 2; excluding only direct neighbors would hand the
    -- drill a resource the tray had already classified as contained.
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
           AND NOT p.is_folded
    ),
    ctx AS (
        SELECT r.id, d.dt
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
                                  AND h.anchor_table = 'kb_contexts'
                                  AND h.anchor_id = p_context_id
          JOIN vis v ON v.id = r.id
          LEFT JOIN doc d ON d.rid = r.id
         WHERE r.is_active
    ),
    ie AS (
        SELECT e.source_id, e.target_id
          FROM kb_edges e
          JOIN ctx s ON s.id = e.source_id
          JOIN ctx t ON t.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
    ),
    reached AS (
        SELECT c.id AS node_id, 0 AS depth FROM ctx c WHERE c.dt = ANY(p_container_types)
        UNION
        SELECT CASE WHEN ie.source_id = r.node_id THEN ie.target_id ELSE ie.source_id END,
               r.depth + 1
          FROM reached r
          JOIN ie ON (ie.source_id = r.node_id OR ie.target_id = r.node_id)
         WHERE r.depth < LEAST(p_depth, 3)
    ),
    grp AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS gv
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = p_group_key
           AND NOT p.is_folded
    )
    SELECT c.id
      FROM ctx c
      LEFT JOIN grp g ON g.rid = c.id
     WHERE COALESCE(g.gv, '(none)') = p_group_value
       AND c.id NOT IN (SELECT node_id FROM reached);
$$;

-- Composition edges from an arbitrary visible seed set. NOT fenced to the context: the
-- walk follows visible edges out to cogmap-homed resources, which is what makes "the work
-- + the ideas distilled from it" one graph. Mirrors graph_region_composition_edges.
CREATE FUNCTION graph_context_composition_edges(
    p_profile  uuid,
    p_seed_ids uuid[],
    p_depth    int
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    seeds AS (SELECT DISTINCT s.id FROM unnest(p_seed_ids) s(id) JOIN vis v ON v.id = s.id),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END,
               r.depth + 1
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
