-- Beat E (spec D8): widen graph_atlas_nodes_visible with `stage`. The legacy subgraph
-- returned `stage_raw`; AtlasNode did not carry it, so retiring that surface would have
-- dropped the task-stage signal from a builder view. RETURNS TABLE changed => DROP+CREATE
-- in a NEW migration (20260708000002 is applied and immutable).
--
-- The stage lookup is a LEFT JOIN over kb_properties: stage lives under property_key
-- `temper-stage` (NOT `stage`) as a jsonb scalar, extracted with `#>> '{}'` exactly like
-- the neighbouring `doc` CTE and the legacy `stage_raw` (20260708000004). Most doc-types
-- carry no stage, so those nodes still return with stage = NULL. Every other conjunct —
-- the `vis` deny-as-absence join, `r.is_active`, the `home` LATERAL, the `degree` LATERAL
-- over edges_visible_to, and the `first_chunk` subquery — is reproduced verbatim from the
-- 20260708000002 body.

DROP FUNCTION IF EXISTS graph_atlas_nodes_visible(uuid, uuid[]);

CREATE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int,
              first_chunk text, stage text)
LANGUAGE sql STABLE AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
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
           s.st AS stage
    FROM ids
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN stg s ON s.rid = r.id
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
