-- ─────────────────────────────────────────────────────────────────────────────
-- Atlas Home reads — the you→teams→cogmaps membership graph + cogmap panorama.
-- (Renumber to sort after the latest main migration at execution time.)
-- ─────────────────────────────────────────────────────────────────────────────

-- Home teams: the profile's member teams, with per-team resource + cogmap counts.
-- Mirrors list_teams' membership set (kb_team_members), adds the two counts.
CREATE FUNCTION graph_home_teams(p_profile uuid)
RETURNS TABLE(team_id uuid, slug text, name text, resource_count int, cogmap_count int)
LANGUAGE sql STABLE AS $$
    SELECT t.id, t.slug, t.name,
           (SELECT count(*) FROM resources_in_team_scope(p_profile, t.id))::int,
           (SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.team_id = t.id)::int
    FROM kb_teams t
    JOIN kb_team_members tm ON tm.team_id = t.id
    WHERE tm.profile_id = p_profile AND t.is_active
    ORDER BY t.name;
$$;

-- Home cogmaps: the profile's visible cogmaps, each with the visible team ids it
-- joins (the bipartite edges) and region/facet counts. Gated by cogmap_visible_maps.
CREATE FUNCTION graph_home_cogmaps(p_profile uuid)
RETURNS TABLE(cogmap_id uuid, name text, team_ids uuid[], region_count int, facet_count int)
LANGUAGE sql STABLE AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    member_teams AS (
        SELECT tm.team_id FROM kb_team_members tm WHERE tm.profile_id = p_profile
    )
    SELECT c.id, c.name,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ),
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    GROUP BY c.id, c.name;
$$;
