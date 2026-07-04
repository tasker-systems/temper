-- C3 SearchAccelerator: team-scoped name-locate over the Atlas graph.
-- Reuses the scope-agnostic unified_search blend (weights/visibility unchanged),
-- bounding it to resources_in_team_scope(profile, team) and projecting each hit
-- to Atlas display attrs (doc_type, home) + an optional best-affinity region.
-- v1: NULL embedding (FTS + graph-off name-locate); graph_expand = false.
CREATE FUNCTION atlas_search(
    p_profile uuid,
    p_team    uuid,
    p_query   text,
    p_limit   int
) RETURNS TABLE(
    node_id uuid,
    title text,
    doc_type text,
    home text,
    region_id uuid,
    combined_score real,
    fts_score real,
    vector_score real,
    graph_score real
)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT array_agg(resource_id) AS ids
        FROM resources_in_team_scope(p_profile, p_team)
    ),
    hits AS (
        SELECT u.resource_id, u.combined_score, u.fts_score, u.vector_score, u.graph_score
        FROM unified_search(
            p_profile,               -- $1 principal
            p_query,                 -- $2 query text
            NULL::vector,            -- $3 embedding (NULL → vector term zeroed)
            ARRAY[]::uuid[],         -- $4 seed_ids
            0,                       -- $5 depth
            ARRAY[]::text[],         -- $6 edge_types
            NULL,                    -- $7 context_id
            NULL,                    -- $8 doc_type
            false,                   -- $9 graph_expand
            p_limit,                 -- $10 limit
            0,                       -- $11 offset
            (SELECT ids FROM scope)  -- $12 scope_ids
        ) u
    ),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT
        r.id AS node_id,
        r.title,
        d.dt AS doc_type,
        h.home,
        reg.region_id,
        hits.combined_score::real,
        hits.fts_score::real,
        hits.vector_score::real,
        hits.graph_score::real
    FROM hits
    JOIN kb_resources r ON r.id = hits.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT m.region_id
        FROM kb_cogmap_region_members m
        WHERE m.member_table = 'kb_resources' AND m.member_id = r.id
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) reg ON true
    ORDER BY hits.combined_score DESC, r.id;
$$;
