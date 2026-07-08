-- ─────────────────────────────────────────────────────────────────────────────
-- Atlas Beat D polish — add `slug` to graph_home_contexts (the build lens).
--   The Home build circles route to a context view (`/vault/[owner]/[context]`,
--   which resolves `context_ref = owner/slug`). The Beat-B function returned only
--   `name` (the display label, which may collide across owners) — no slug — so the
--   UI had no way to build the URL and fell back to `/vault/<owner_ref>`, which
--   404s. Add `c.slug` (the per-owner addressable handle) so the build circle can
--   route to the working context resource list.
--   RETURNS TABLE gains one OUT column → DROP+CREATE (a plain CREATE OR REPLACE
--   cannot alter the return type). The shipped birth migration
--   (20260707140000) stays immutable.
-- Additive on main (one added OUT column; no destructive change). Skew-safe: the
-- only caller (atlas_home) selects columns BY NAME, so pre-deploy code selecting
-- the old 5 columns keeps working against the new 6-column function.
-- ─────────────────────────────────────────────────────────────────────────────

DROP FUNCTION IF EXISTS graph_home_contexts(uuid);
CREATE FUNCTION graph_home_contexts(p_profile uuid)
RETURNS TABLE(context_id uuid, name text, slug text, owner_ref text, resource_count int, last_active_at timestamptz)
LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    ),
    -- Membership-join-first candidate set: the UNION of the branches
    -- `context_visible_to` admits, so we never scan all of kb_contexts. The
    -- canonical gate is still applied below as defense-in-depth — the union is a
    -- proven superset (same branches), so the gate can only confirm, never over-show.
    candidates AS (
        SELECT c.id FROM kb_contexts c
         WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
        UNION
        SELECT c.id FROM kb_contexts c
         JOIN profile_effective_teams(p_profile) pet ON pet.team_id = c.owner_id
         WHERE c.owner_table = 'kb_teams'
        UNION
        SELECT tc.context_id FROM kb_team_contexts tc
         JOIN profile_effective_teams(p_profile) pet ON pet.team_id = tc.team_id
        UNION
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_contexts' AND g.can_read
           AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
              OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    )
    SELECT c.id, c.name, c.slug,
           CASE
               WHEN c.owner_table = 'kb_profiles' AND c.owner_id = p_profile THEN '@me'
               WHEN c.owner_table = 'kb_teams' AND owner_team.slug IS NOT NULL THEN '+' || owner_team.slug
               -- Owned by another profile but visible (team-share → that team; otherwise
               -- an explicit read-grant) — label it 'shared', never mis-claim it as '@me'.
               ELSE COALESCE('+' || shared.slug, 'shared')
           END AS owner_ref,
           (SELECT count(*)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id)::int AS resource_count,
           -- Same counted set as resource_count above (identical FROM/JOIN/WHERE), so
           -- recency can never reflect a resource the caller can't see or one that's
           -- been soft-deleted — the two always agree on what's "in" the context.
           (SELECT max(rr.updated)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id) AS last_active_at
    FROM candidates cand
    JOIN kb_contexts c ON c.id = cand.id
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
