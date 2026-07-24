-- cogmap show surface: single-map orientation (identity + charter + foundational resources).
--
-- Two reads. The first GENERALIZES cogmap_list_rows (20260724000010) with an optional single-map
-- filter so `cogmap list` and `cogmap show` share ONE identity query rather than maintaining two
-- copies of the team-aggregation + charter-statement LATERAL — the DRY-SQL discipline the parent
-- goal (019f5c66) is about. `list` passes NULL (every visible map); `show` passes the map id (that
-- one map, or zero rows when it is not visible → the surface renders 404). Replacing the one-arg form
-- added one commit earlier is safe: it ships in the same unmerged PR, so no deployed code depends on
-- the one-arg signature (the additive-only-on-main concern does not arise).
DROP FUNCTION IF EXISTS cogmap_list_rows(uuid);

CREATE FUNCTION cogmap_list_rows(p_profile uuid, p_cogmap uuid DEFAULT NULL)
RETURNS TABLE(
    cogmap_id uuid,
    name text,
    owner_ref text,
    team_ids uuid[],
    region_count int,
    resource_count int,
    telos_resource_id uuid,
    charter_statement text
)
LANGUAGE sql STABLE AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    member_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT c.id, c.name,
           COALESCE('+' || min(mt.slug), 'temper') AS owner_ref,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ) AS team_ids,
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int AS region_count,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int AS resource_count,
           c.telos_resource_id,
           (SELECT rb.body_text
              FROM resource_blocks(c.telos_resource_id, 'profile', p_profile, 'statement') rb
             ORDER BY rb.seq
             LIMIT 1) AS charter_statement
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    LEFT JOIN kb_teams mt ON mt.id = tc.team_id AND tc.team_id IN (SELECT team_id FROM member_teams)
    WHERE (p_cogmap IS NULL OR c.id = p_cogmap)
    GROUP BY c.id, c.name, c.telos_resource_id
    ORDER BY owner_ref, c.name;
$$;

-- cogmap_foundations: the resources a map is BUILT ON — its homed resources
-- (anchor_table='kb_cogmaps'), visibility-intersected, with the telos/charter resource flagged.
-- This is the body of cogmap_scope_ids (the single-map search corpus) minus the search plumbing: the
-- structural, always-available answer to "what shaped this map" (region-anchor surveys are the
-- region tier's job — 019f5c66 — and are a non-goal here). Ordered telos-first, then by title.
--
-- Access: resources_visible_to gates membership; an interrupted segmented ingest
-- (ingest_state <> 'complete') is not yet a document, excluded exactly as list/search exclude it.
-- Deny → empty, never an error.
CREATE FUNCTION cogmap_foundations(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(resource_id uuid, title text, doc_type text, is_telos boolean)
LANGUAGE sql STABLE AS $$
    SELECT r.id,
           r.title,
           dt.property_value #>> '{}' AS doc_type,
           (r.id = (SELECT telos_resource_id FROM kb_cogmaps WHERE id = p_cogmap)) AS is_telos
    FROM kb_resource_homes h
    JOIN kb_resources r ON r.id = h.resource_id
    JOIN resources_visible_to(p_profile) v ON v.resource_id = r.id
    JOIN kb_properties dt
      ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
     AND dt.property_key = 'doc_type' AND NOT dt.is_folded
    WHERE h.anchor_table = 'kb_cogmaps'
      AND h.anchor_id = p_cogmap
      AND r.is_active
      AND r.ingest_state = 'complete'
    ORDER BY is_telos DESC, r.title;
$$;
