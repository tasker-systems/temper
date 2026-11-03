-- The entry read's two hot functions test edge visibility ONCE PER CALL, as a set, instead of once
-- per edge (graph_induced_edges) or once per node (graph_atlas_nodes_visible). Cost only: both
-- return exactly what they returned before, on the same signatures.
--
-- Why: on a large corpus `graph_induced_edges(…, 0)` spent ~98% of its time calling
-- `anchor_readable_by_profile` on every live edge, before the join to the ids discarded almost all of
-- them -- the planner attaches that predicate to the kb_edges scan because it names only `e`. And the
-- hydration's `degree` LATERAL re-ran `edges_visible_to` once per id. Measured on a synthetic 20k
-- resource / 29k edge corpus: induced depth 0 3.6 s -> 0.4 s, depth 1 14 s -> 0.6 s, hydration of
-- 130 ids 13.2 s -> 0.46 s, outputs identical. Review: temper-artifacts
-- reviews/2026-11-03-v0.6.4-graph-entry-induced-edges-cost.md (asks A and C).
--
-- WHY `edges_visible_to` AND NOT ITS CTEs INLINED: it is the one definition of "an edge this caller
-- may see" (20260903000050). For an edge with kb_resources at both ends it is exactly the conjunction
-- the old bodies spelled out -- NOT is_folded, both endpoints in resources_visible_to, and the home
-- anchor readable -- and its anchor arm reads the same rules `anchor_readable_by_profile` does
-- (contexts_readable_by for contexts; team cogmaps over profile_reachable_teams plus can_read grants
-- for cogmaps). Copying its CTEs here to save the one pass would make a third copy of the read rules.
--
-- `ve` IS MATERIALIZED ON PURPOSE: the recursive arm and the final SELECT both read it, and an
-- inlined copy inside the recursive term would be re-planned per iteration. It is built once.
--
-- The degree keeps 20260821000010's semantics: an edge counts once per node it touches, so a
-- self-loop counts once -- hence UNION, never UNION ALL, over (edge, end) pairs, as the ranking does.

CREATE OR REPLACE FUNCTION graph_induced_edges(p_profile uuid, p_node_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    ve AS MATERIALIZED (SELECT edge_id AS id FROM edges_visible_to(p_profile)),
    seeds AS (
        SELECT DISTINCT s.id
          FROM unnest(p_node_ids) s(id)
          JOIN resources_visible_to(p_profile) rv ON rv.resource_id = s.id
    ),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END,
               r.depth + 1
          FROM reached r
          JOIN kb_edges e ON (e.source_id = r.node_id OR e.target_id = r.node_id)
          JOIN ve ON ve.id = e.id   -- unfolded, both ends visible, home readable
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           -- At p_depth = 0 this arm is dead (`0 < 0`), so `reached` stays the seed array and the
           -- final SELECT is the INDUCED subgraph. The entry read depends on that; raising the
           -- floor to 1 would silently put undrawn endpoints back on the canvas.
           AND r.depth < LEAST(p_depth, 3)
    ),
    nodes AS MATERIALIZED (SELECT DISTINCT node_id FROM reached)
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
      FROM kb_edges e
      JOIN nodes ns ON ns.node_id = e.source_id
      JOIN nodes nt ON nt.node_id = e.target_id
      JOIN ve ON ve.id = e.id
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources';
$$;

COMMENT ON FUNCTION graph_induced_edges(uuid, uuid[], integer) IS
$c$Edges in AtlasEdge shape over an arbitrary node-id array, serving the entry read (depth 0) and the
traversal read (depth > 0). An edge is returned only if edges_visible_to(p_profile) admits it
(unfolded, both endpoints visible, home anchor readable) and it is kb_resources at both ends.

p_depth = 0 returns the INDUCED subgraph over p_node_ids -- every returned edge has both endpoints in
the array the caller passed. Above 0 it expands outward first (clamped to 3), then returns the edges
among everything reached.

Visibility is read as a set once per call (20261116000000); before that it was tested per edge, at a
cost that grew with every live edge in the instance.$c$;

CREATE OR REPLACE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE (id uuid, title text, doc_type text, home text, degree integer,
               first_chunk text, stage text, home_id uuid, updated timestamptz)
LANGUAGE sql
STABLE
AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    -- Every visible resource-to-resource edge touching any requested id, found in ONE pass.
    e AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND (e.source_id = ANY(p_ids) OR e.target_id = ANY(p_ids))
    ),
    -- UNION, never UNION ALL: a self-loop yields (edge, node) twice and must count once.
    inc AS (
        SELECT id AS eid, source_id AS n FROM e
        UNION
        SELECT id, target_id FROM e
    ),
    deg AS (SELECT n AS rid, count(*)::int AS degree FROM inc GROUP BY n),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    ),
    stg AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS st
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'temper-stage' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
           s.st AS stage,
           h.home_id,
           r.updated
    FROM ids
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN stg s ON s.rid = r.id
    LEFT JOIN LATERAL (
        -- `bool_or` is carried over verbatim rather than simplified to a LIMIT 1: with one home
        -- they agree, with more than one they do not. Postgres has no `min(uuid)`, hence array_agg.
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home,
               (array_agg(h2.anchor_id))[1] AS home_id
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN deg ON deg.rid = r.id;
$$;

COMMENT ON FUNCTION graph_atlas_nodes_visible(uuid, uuid[]) IS
$c$Hydrates a resource-id set into AtlasNode rows: title, doc_type, home kind and id, first chunk,
stage, updated, and degree. Ids the caller cannot see are absent, not errors. No ORDER BY: the caller
orders.

degree is the number of edges_visible_to edges (kb_resources both ends) touching the node, a
self-loop counted once. It is computed for the whole id set in one pass (20261116000000); before
that it re-ran edges_visible_to once per id.$c$;

SELECT declare_migration(
    20261116000000,
    'additive',
    'CREATE OR REPLACE of graph_induced_edges(uuid, uuid[], integer) and graph_atlas_nodes_visible(uuid, uuid[]) with unchanged argument lists, unchanged RETURNS TABLE columns, and unchanged results -- only how visibility is evaluated moves (one set-based edges_visible_to pass per call instead of a per-edge anchor_readable_by_profile or a per-node edges_visible_to). Two COMMENTs. Nothing created, dropped or renamed, so a binary on either side of this migration calls the same names and reads the same rows. graph_context_composition_edges delegates to graph_induced_edges and picks the change up unchanged.'
);
