-- ─────────────────────────────────────────────────────────────────────────────
-- Atlas Beat B — Home reframe as the build / research verb-lens footprint.
--   * graph_home_contexts (NEW): the build lens — every context the profile can
--     build in (personal + team), visibility-gated by the canonical
--     `context_visible_to`, each sized by its `resources_visible_to`-scoped
--     resource count, with a decorated owner-scope `owner_ref` (@me / +slug).
--   * graph_home_cogmaps (DROP+CREATE): the research lens gains a derived "held-by"
--     `owner_ref` (+first-member-team slug, or `temper` for the universal/system
--     kernel) that the UI tints by. RETURNS TABLE changes → DROP+CREATE (a plain
--     CREATE OR REPLACE cannot alter the return type). Skew-safe: the only caller
--     (atlas_home) selects columns BY NAME, so pre-deploy code selecting the old 5
--     columns keeps working against the new 6-column function.
-- Additive on main (new function + one added OUT column; no destructive change to a
-- shipped object's existing callers). graph_home_teams is left in place (unused by
-- the new service; dropped in a later beat once no caller remains).
-- ─────────────────────────────────────────────────────────────────────────────

-- Build lens: the profile's contexts — personal + team — each with its visible
-- resource count and decorated owner-scope. The SET is gated by the FULL canonical
-- predicate `context_visible_to` (self-owned OR effective-team-owned OR team-shared
-- OR explicit read grant), so it never leaks and never under-shows. Per-context
-- count is scoped through `resources_visible_to` so a private resource the caller
-- can't see never inflates a count.
CREATE FUNCTION graph_home_contexts(p_profile uuid)
RETURNS TABLE(context_id uuid, name text, owner_ref text, resource_count int)
LANGUAGE sql STABLE AS $$
    SELECT c.id, c.name,
           CASE
               WHEN c.owner_table = 'kb_profiles' AND c.owner_id = p_profile THEN '@me'
               WHEN c.owner_table = 'kb_teams' AND owner_team.slug IS NOT NULL THEN '+' || owner_team.slug
               ELSE COALESCE('+' || shared.slug, '@me')
           END AS owner_ref,
           (SELECT count(*)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id)::int AS resource_count
    FROM kb_contexts c
    LEFT JOIN kb_teams owner_team ON c.owner_table = 'kb_teams' AND owner_team.id = c.owner_id
    LEFT JOIN LATERAL (
        -- team this context is shared INTO that the profile effectively belongs to
        -- (covers a personal-owned context shared to a team the caller is in)
        SELECT tt.slug
        FROM kb_team_contexts tc
        JOIN profile_effective_teams(p_profile) pet ON pet.team_id = tc.team_id
        JOIN kb_teams tt ON tt.id = tc.team_id
        WHERE tc.context_id = c.id
        ORDER BY tt.slug
        LIMIT 1
    ) shared ON true
    WHERE context_visible_to(p_profile, c.id)
    ORDER BY owner_ref, c.name;
$$;

-- Research lens: the profile's visible cogmaps, now with a derived held-by scope.
DROP FUNCTION IF EXISTS graph_home_cogmaps(uuid);
CREATE FUNCTION graph_home_cogmaps(p_profile uuid)
RETURNS TABLE(cogmap_id uuid, name text, owner_ref text, team_ids uuid[], region_count int, facet_count int)
LANGUAGE sql STABLE AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    member_teams AS (
        SELECT tm.team_id FROM kb_team_members tm WHERE tm.profile_id = p_profile
    )
    SELECT c.id, c.name,
           -- held-by scope: the alphabetically-first member team's slug, else the
           -- universal/system marker (a public/system kernel joins no member team).
           COALESCE('+' || min(mt.slug), 'temper') AS owner_ref,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ) AS team_ids,
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int AS region_count,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int AS facet_count
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    LEFT JOIN kb_teams mt ON mt.id = tc.team_id AND tc.team_id IN (SELECT team_id FROM member_teams)
    GROUP BY c.id, c.name;
$$;
