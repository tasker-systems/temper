-- SQL-audit chunk 8: graph hardening pair (SQLA-1, align-with-siblings).
--
-- 1. graph_traverse: clamp p_depth at 10 inside the function. Every scoped
--    sibling (graph_traverse_scoped, graph_traverse_cogmap_scoped,
--    graph_region_composition_edges) clamps internally with LEAST(p_depth, 10);
--    the unscoped walk trusted the caller. The live caller already clamps at
--    MAX_DEPTH=10, so this is defensive — no behavior change.
--
-- 2. graph_cogmap_orphan_nodes: the degree LATERAL omitted the
--    source_table/target_table = 'kb_resources' predicate, diverging from
--    graph_atlas_nodes_visible, and both laterals omit NOT is_folded — which
--    the partial indexes idx_kb_edges_source/idx_kb_edges_target require to
--    be eligible at all (verified via EXPLAIN: table predicates alone still
--    seq-scan; adding NOT is_folded probes the index). The is_folded predicate
--    is behavior-neutral: the edges_visible_to join already excludes folded
--    edges, so the count is unchanged either way.
--
-- 3. graph_atlas_nodes_visible: same NOT is_folded enabling predicate added
--    to its degree LATERAL (surfaced by the same EXPLAIN check; identical
--    behavior-neutrality argument).

CREATE OR REPLACE FUNCTION public.graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth integer)
 RETURNS TABLE(resource_id uuid, source_id uuid, target_id uuid, edge_kind edge_kind, polarity edge_polarity, label text, depth integer)
 LANGUAGE sql
 STABLE
AS $function$
  WITH RECURSIVE visible AS (SELECT rv.resource_id AS id FROM resources_visible_to(p_profile) rv),
  walk AS (
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, 1 AS depth
      FROM kb_edges e
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND e.source_id = ANY(p_seed_ids) AND NOT e.is_folded
       AND e.source_id IN (SELECT id FROM visible) AND e.target_id IN (SELECT id FROM visible)
    UNION
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, w.depth+1
      FROM kb_edges e JOIN walk w ON e.source_id = w.target_id
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND NOT e.is_folded AND w.depth < LEAST(p_depth, 10)
       AND e.target_id IN (SELECT id FROM visible)
  )
  SELECT w.target_id, w.source_id, w.target_id, w.edge_kind, w.polarity, w.label, w.depth FROM walk w;
$function$;

CREATE OR REPLACE FUNCTION public.graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid)
 RETURNS TABLE(id uuid, title text, doc_type text, degree integer, anchor_id uuid, anchor_label text)
 LANGUAGE sql
 STABLE
AS $function$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    homed AS (
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap
    ),
    region_members AS (
        SELECT DISTINCT rm.member_id AS resource_id
        FROM kb_cogmap_region_members rm
        JOIN kb_cogmap_regions reg ON reg.id = rm.region_id
        WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded
          AND rm.member_table = 'kb_resources'
    )
    SELECT r.id, r.title, d.dt AS doc_type, deg.degree, p_cogmap AS anchor_id,
           (SELECT name FROM kb_cogmaps WHERE id = p_cogmap) AS anchor_label
    FROM homed
    JOIN kb_resources r ON r.id = homed.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE r.id NOT IN (SELECT resource_id FROM region_members)
    ORDER BY deg.degree DESC;
$function$;

CREATE OR REPLACE FUNCTION public.graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
 RETURNS TABLE(id uuid, title text, doc_type text, home text, degree integer, first_chunk text)
 LANGUAGE sql
 STABLE
AS $function$
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
          AND NOT e.is_folded
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$function$;
