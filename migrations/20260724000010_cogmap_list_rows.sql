-- cogmap list surface: the profile's visible cognitive maps with identity + charter statement.
--
-- A charter-bearing SIBLING of graph_home_cogmaps (20260707140000), NOT an edit to it: that function
-- is consumed by the Atlas (graph_service::atlas_home) and must keep its shape. This one adds the two
-- columns a `cogmap list` / `cogmap show` surface needs to orient — telos_resource_id and the charter
-- STATEMENT (block-0 of the telos) — on top of the same visible-maps base.
--
-- No new access predicate. cogmap_visible_maps(p_profile) IS the "maps I can reach" gate
-- (up-expanded team membership ∪ explicit read grant); the statement rides the same member-gated
-- resource_blocks projection cogmap_charter_select uses, so a caller never sees a statement drawn
-- from a block they could not read. Because map-read = resource-read agree by construction, a listed
-- map's telos statement is readable whenever the map is — but the projection stays the gate, not an
-- assumption. A map whose charter has no authored `statement` block lists with a NULL statement, not
-- a hidden row.
--
-- Additive-only (a new CREATE FUNCTION), safe on main per the deploy invariant.
CREATE FUNCTION cogmap_list_rows(p_profile uuid)
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
    -- reachable teams = self + ancestors (is_active), mirroring cogmap_visible_maps' admit basis, so
    -- the derived held-by owner_ref / team_ids reflect the team that actually made the map visible.
    member_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT c.id, c.name,
           -- held-by scope: the alphabetically-first member team's slug, else the universal marker
           -- (a public/system kernel joins no member team).
           COALESCE('+' || min(mt.slug), 'temper') AS owner_ref,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ) AS team_ids,
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int AS region_count,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int AS resource_count,
           c.telos_resource_id,
           -- charter statement: block-0 (role='statement') of the telos, through the member-gated
           -- resource_blocks projection. NULL when unauthored or unreadable.
           (SELECT rb.body_text
              FROM resource_blocks(c.telos_resource_id, 'profile', p_profile, 'statement') rb
             ORDER BY rb.seq
             LIMIT 1) AS charter_statement
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    LEFT JOIN kb_teams mt ON mt.id = tc.team_id AND tc.team_id IN (SELECT team_id FROM member_teams)
    GROUP BY c.id, c.name, c.telos_resource_id
    ORDER BY owner_ref, c.name;
$$;
