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

-- Cogmap-scoped territories: this cogmap's live regions for the given lens.
-- Keyed on cogmap (not team), gated by cogmap_readable_by_profile.
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

-- Cogmap-scoped orphan facets: this cogmap's homed resources with NO live region,
-- gated per-row by resources_visible_to, ranked by visible edge-degree.
CREATE FUNCTION graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)
LANGUAGE sql STABLE AS $$
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
        -- kb_cogmap_region_members is polymorphic: (region_id, member_table, member_id).
        SELECT DISTINCT rm.member_id AS resource_id
        FROM kb_cogmap_region_members rm
        JOIN kb_cogmap_regions reg ON reg.id = rm.region_id
        WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded
          AND rm.member_table = 'kb_resources'
    )
    SELECT r.id, r.title, d.dt AS doc_type, deg.degree, p_cogmap AS anchor_id
    FROM homed
    JOIN kb_resources r ON r.id = homed.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE r.id NOT IN (SELECT resource_id FROM region_members)
    ORDER BY deg.degree DESC;
$$;
