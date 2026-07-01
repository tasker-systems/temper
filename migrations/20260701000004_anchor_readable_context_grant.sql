-- Generalized access-capability arc — fast-follow to Deliverable 3a: the CONTEXT-homed-edge arm of
-- `anchor_readable_by_profile`.
--
-- D3a (`20260630000002`) wired explicit `kb_access_grants` READ rows into `context_visible_to` (a
-- context you can address) and `resources_visible_to` (the resources homed in it), but its own scope
-- note flagged one place a context read-grant did not yet reach: `anchor_readable_by_profile`'s
-- `kb_contexts` arm. `edges_visible_to(profile)` gates each edge on `anchor_readable_by_profile` of
-- its HOME anchor (`canonical_functions.sql:309`; an edge homes in its source resource's context),
-- so a profile granted read on a context could see the context's resources yet NOT the edges among
-- them — the `/api/resources/{id}/edges` view came back empty. A false-negative, never a leak.
--
-- The cogmap side already followed for free: the `kb_cogmaps` arm delegates to
-- `cogmap_readable_by_profile`, which gained its explicit read-grant branch in D3a. Only the context
-- arm was left short. This closes it by adding the same clause `context_visible_to` already carries
-- (`20260630000002` clause 4): `profile_explicit_grant(p_profile, 'read', 'kb_contexts', …)`.
--
-- After this, the context arm has FOUR clauses — profile-owned, team-shared, team-owned-by-member,
-- explicit-read-grant — matching `context_visible_to` exactly, so "a context you can read", "the
-- resources inside it", and "the edges among them" agree by construction, grant included.
--
-- ADDITIVE, widens-never-narrows: a profile with no matching `kb_access_grants` row reads exactly as
-- before. The three pre-existing clauses are re-emitted VERBATIM from `20260627000003`. Leak-safety
-- (Q-B) is preserved — `profile_explicit_grant` is the union-up Profile axis only; the Cogmap
-- producer intersection is untouched.
--
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).
-- STABLE + LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

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
            -- explicit context read-grant (NEW, additive — mirrors `context_visible_to` clause 4):
            -- a profile granted read on the context sees the edges homed in it, not just its resources.
            OR profile_explicit_grant(p_profile, 'read', 'kb_contexts', p_anchor_id)
        )
        ELSE false
    END;
$$;
