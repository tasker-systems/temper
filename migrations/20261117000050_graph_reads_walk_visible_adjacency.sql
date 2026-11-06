-- The graph reads still built on per-edge readability, or on an OR-join walk, now read the visible
-- edge set once per call and walk it as a hashed adjacency. Cost only: every function keeps its
-- signature and returns exactly what it returned before.
--
-- Two costs, both growing with the instance's live edges rather than with what is drawn:
--   1. `anchor_readable_by_profile` per edge (not inlinable; see 20261116000000's header).
--   2. The recursive arm's `e.source_id = r.node_id OR e.target_id = r.node_id`, which the planner
--      can only run as a nested loop -- frontier x every candidate edge, per iteration.
-- The fix for (2) is `adj`: each visible edge listed once per direction, so the walk is a hash join
-- on `adj.a = r.node_id`. `adj.b` is the CASE's "other endpoint", a self-loop included.
-- Measured on a synthetic 26k-resource / 38k-edge corpus, outputs identical (EXCEPT both ways):
-- graph_induced_edges depth 2 13 s -> 0.38 s, depth 3 35 s -> 0.44 s; region composition depth 0
-- 13.2 s -> 0.28 s; cogmap traverse depth 2 2.35 s -> 0.31 s, depth 10 33.7 s -> 0.46 s.
-- Review: temper-artifacts reviews/2026-11-03-v0.6.4-graph-entry-induced-edges-cost.md.
--
-- SCOPE STAYS A JOIN, NOT A REPLACEMENT: resources_in_cogmap_scope is itself confined to
-- resources_visible_to, so `edges_visible_to` ∩ scope-both-ends is exactly the old conjunction.
-- Dropping the scope join would widen the cogmap traverse to every visible edge.
--
-- graph_region_composition_edges was graph_induced_edges' body with different seeds; it now
-- delegates, so the walk has one definition. The context-panorama readers keep their shape and
-- only swap the per-edge predicate for a semi-join on `ve`; their walk is confined to one context.
-- graph_traverse_scoped is left as it is: nothing in the product calls it.

CREATE OR REPLACE FUNCTION graph_induced_edges(p_profile uuid, p_node_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    -- unfolded, both endpoints visible, home readable: edges_visible_to is the one definition.
    ve AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
    ),
    seeds AS (
        SELECT DISTINCT s.id
          FROM unnest(p_node_ids) s(id)
          JOIN resources_visible_to(p_profile) rv ON rv.resource_id = s.id
    ),
    adj AS MATERIALIZED (
        SELECT source_id AS a, target_id AS b FROM ve
        UNION ALL
        SELECT target_id, source_id FROM ve
    ),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT adj.b, r.depth + 1
          FROM reached r
          JOIN adj ON adj.a = r.node_id
           -- At p_depth = 0 this arm is dead (`0 < 0`), so `reached` stays the seed array and the
           -- final SELECT is the INDUCED subgraph. The entry read depends on that; raising the
           -- floor to 1 would silently put undrawn endpoints back on the canvas.
         WHERE r.depth < LEAST(p_depth, 3)
    ),
    nodes AS MATERIALIZED (SELECT DISTINCT node_id FROM reached)
    SELECT DISTINCT ve.id, ve.source_id, ve.target_id, ve.edge_kind, ve.polarity, ve.label, ve.weight
      FROM ve
      JOIN nodes ns ON ns.node_id = ve.source_id
      JOIN nodes nt ON nt.node_id = ve.target_id;
$$;

COMMENT ON FUNCTION graph_induced_edges(uuid, uuid[], integer) IS
$c$Edges in AtlasEdge shape over an arbitrary node-id array, serving the entry read (depth 0), the
traversal read (depth > 0) and region composition. An edge is returned only if
edges_visible_to(p_profile) admits it (unfolded, both endpoints visible, home anchor readable) and it
is kb_resources at both ends.

p_depth = 0 returns the INDUCED subgraph over p_node_ids -- every returned edge has both endpoints in
the array the caller passed. Above 0 it expands outward first (clamped to 3), then returns the edges
among everything reached.

Visibility is read once per call as a set and walked as a hashed adjacency (20261116000000,
20261117000050).$c$;

CREATE OR REPLACE FUNCTION graph_region_composition_edges(p_profile uuid, p_region_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    -- Region members are the seeds; graph_induced_edges drops the ones the caller cannot see.
    SELECT * FROM graph_induced_edges(
        p_profile,
        ARRAY(SELECT DISTINCT m.member_id
                FROM kb_cogmap_region_members m
               WHERE m.region_id = ANY(p_region_ids)),
        p_depth);
$$;

COMMENT ON FUNCTION graph_region_composition_edges(uuid, uuid[], integer) IS
$c$graph_induced_edges seeded with the members of p_region_ids (20261117000050). Same visibility,
clamp and depth-0 induced semantics.$c$;

CREATE OR REPLACE FUNCTION graph_traverse_cogmap_scoped(
    p_profile uuid, p_cogmap uuid, p_seed_ids uuid[], p_depth integer, p_edge_kinds edge_kind[])
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    scope AS MATERIALIZED (
        SELECT resource_id AS id FROM resources_in_cogmap_scope(p_profile, p_cogmap)
    ),
    -- Visible, in scope at both ends, of the requested kinds: the only edges the walk may use.
    se AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
          JOIN scope ss ON ss.id = e.source_id
          JOIN scope st ON st.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
                OR e.edge_kind = ANY(p_edge_kinds))
    ),
    adj AS MATERIALIZED (
        SELECT source_id AS a, target_id AS b FROM se
        UNION ALL
        SELECT target_id, source_id FROM se
    ),
    reached AS (
        SELECT unnest(p_seed_ids) AS node_id, 0 AS depth
        UNION
        SELECT adj.b, r.depth + 1
          FROM reached r
          JOIN adj ON adj.a = r.node_id
         WHERE r.depth < LEAST(p_depth, 10)
    ),
    nodes AS MATERIALIZED (SELECT DISTINCT node_id FROM reached)
    SELECT DISTINCT se.id, se.source_id, se.target_id, se.edge_kind, se.polarity, se.label, se.weight
      FROM se
      JOIN nodes ns ON ns.node_id = se.source_id
      JOIN nodes nt ON nt.node_id = se.target_id;
$$;

COMMENT ON FUNCTION graph_traverse_cogmap_scoped(uuid, uuid, uuid[], integer, edge_kind[]) IS
$c$Undirected walk from p_seed_ids over edges visible to p_profile with both endpoints in
resources_in_cogmap_scope(p_profile, p_cogmap), optionally of p_edge_kinds, clamped to depth 10;
returns the in-scope edges among everything reached. Walked as a hashed adjacency over one
visible-edge pass (20261117000050).$c$;

CREATE OR REPLACE FUNCTION graph_context_containers(p_profile uuid, p_context_id uuid, p_container_types text[], p_depth integer)
RETURNS TABLE(id uuid, label text, member_count integer)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    ve AS MATERIALIZED (SELECT edge_id AS id FROM edges_visible_to(p_profile)),
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
           AND e.id IN (SELECT id FROM ve)
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

CREATE OR REPLACE FUNCTION graph_context_residual_counts(p_profile uuid, p_context_id uuid, p_group_key text, p_container_types text[], p_depth integer)
RETURNS TABLE(group_value text, member_count integer)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    ve AS MATERIALIZED (SELECT edge_id AS id FROM edges_visible_to(p_profile)),
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
           AND e.id IN (SELECT id FROM ve)
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

CREATE OR REPLACE FUNCTION graph_context_residual_members(p_profile uuid, p_context_id uuid, p_group_key text, p_group_value text, p_container_types text[], p_depth integer)
RETURNS TABLE(id uuid)
LANGUAGE sql
STABLE
AS $$
    -- The member ids behind ONE bucket of graph_context_residual_counts. The two functions are
    -- two spellings of one set, so the exclusion below must be the identical depth-bounded walk
    -- the counts function performs -- NOT a 1-hop neighbor check. A resource two hops from a
    -- container is contained at p_depth >= 2; excluding only direct neighbors would hand the
    -- drill a resource the tray had already classified as contained.
    WITH RECURSIVE
    ve AS MATERIALIZED (SELECT edge_id AS id FROM edges_visible_to(p_profile)),
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
           AND e.id IN (SELECT id FROM ve)
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

SELECT declare_migration(
    20261117000050,
    'additive',
    'CREATE OR REPLACE of graph_induced_edges(uuid, uuid[], integer), graph_region_composition_edges(uuid, uuid[], integer), graph_traverse_cogmap_scoped(uuid, uuid, uuid[], integer, edge_kind[]), graph_context_containers, graph_context_residual_counts and graph_context_residual_members with unchanged argument lists, unchanged RETURNS TABLE columns and unchanged results -- only how visibility is evaluated (one edges_visible_to pass per call) and, in the first three, how the walk joins (a hashed adjacency in place of an OR join) move. Three COMMENTs. Nothing created, dropped or renamed, so a binary on either side of this migration calls the same names and reads the same rows.'
);
