-- Profile-owned-context anchor readability (closes the edge-home gap).
--
-- `edges_visible_to(profile)` gates an edge on `anchor_readable_by_profile` of
-- the edge's HOME anchor (an edge homes in its source resource's context —
-- `db_backend.rs` `assert_relationship`). The `kb_contexts` arm admitted a
-- context only when it was team-shared (`kb_team_contexts`) or team-owned-and-
-- I'm-a-member. It had NO clause for a context owned by the requesting profile
-- itself — so an owner could see their own resources (`resources_visible_to`
-- admits owned/originated) yet NOT the edges between them (`/api/resources/{id}/edges`
-- returned empty). That asymmetry was a bug: the prior migration
-- (`20260627000002`) deferred it as "the separate PROFILE-owned context anchor
-- gap," but it is incoherent for an owner to be denied the edges in their own
-- context, so it is closed here rather than deferred further.
--
-- The fix mirrors `context_visible_to` clause 1 (migration `20260627000001`):
--   (owner_table='kb_profiles' AND owner_id=principal).
-- After this, the three context-arm clauses match `context_visible_to` exactly —
-- profile-owned, team-owned-by-member, team-shared — so "a context you can
-- address," "the resources inside it," and "the edges among them" agree by
-- construction.
--
-- False-negative fix, NOT a leak fix: it only admits the legitimate owner to
-- their OWN context; non-owners still see nothing. STABLE + LANGUAGE sql so
-- `sqlx::query!` callers stay compile-checked.

CREATE OR REPLACE FUNCTION anchor_readable_by_profile(p_profile uuid, p_anchor_table text, p_anchor_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN (
            -- context owned by the principal themselves (mirrors
            -- `context_visible_to` clause 1 — personal context)
            EXISTS (
                SELECT 1
                FROM kb_contexts c
                WHERE c.id = p_anchor_id
                  AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
            )
            -- context shared to a reachable (self-or-ancestor) team
            OR EXISTS (
                SELECT 1
                FROM profile_effective_teams(p_profile) e
                CROSS JOIN LATERAL team_ancestors(e.team_id) a
                JOIN kb_team_contexts tc ON tc.team_id = a.team_id
                WHERE tc.context_id = p_anchor_id
            )
            -- context OWNED by a team the principal is a member of (mirrors
            -- `context_visible_to` clause 2 — direct membership)
            OR EXISTS (
                SELECT 1
                FROM kb_contexts c
                JOIN kb_team_members tm
                  ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
                WHERE c.id = p_anchor_id AND c.owner_table = 'kb_teams'
            )
        )
        ELSE false
    END;
$$;
