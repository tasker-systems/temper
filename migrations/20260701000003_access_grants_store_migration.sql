-- Generalized access-capability arc — Deliverable 5 (D5): kb_resource_access → kb_access_grants store swap.
-- Design: docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md §4 steps 4-5.
--
-- The last beat before `kb_resource_access` retires. Its three live readers — `resources_visible_to`'s two
-- direct-resource-grant branches, `can_modify_resource`'s two write-grant branches, and `vis_team`'s
-- team-grant branch — move onto the dual-polymorphic `kb_access_grants` (20260630000001), filtering
-- `subject_table='kb_resources'`. Every other branch of those functions is re-emitted VERBATIM (carrying D4's
-- UP+union cogmap-membership branch and the D3a explicit-subject-grant branch), so the swap is purely
-- mechanical: identical behavior, one store.
--
-- Column map (kb_resource_access → kb_access_grants):
--   resource_id → subject_id (with subject_table='kb_resources'); anchor_table → principal_table;
--   anchor_id → principal_id; the four rwx booleans + granted_by/granted_at carry across 1:1.
--
-- Q-B leak-safety (load-bearing, §3.7 / §4 step 4): `vis_team`'s grant branch reads ONLY
-- (subject_table='kb_resources', principal_table='kb_teams', can_read). Profile-principal grants and the new
-- context/cogmap subjects NEVER enter the producer intersection — the generalization of `vis_team`'s as-built
-- profile-grant exclusion. `resources_accessible_to_cogmap` (which iterates `vis_team`) is untouched.
--
-- `derived_access_profile` / `can()` need no edit: the profile derived floor now transitively reads the unified
-- store because `resources_visible_to` / `can_modify_resource` do (that IS the "inline" of §4 step 4). The
-- overlap with `profile_explicit_grant` on 'kb_resources' subjects is a harmless UNION/OR — same rows, same result.
--
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public everywhere —
-- prod/dev/e2e and the ephemeral artifact-test DBs).

-- ============================================================================
-- Step 4a — Backfill: migrate every existing kb_resource_access row into kb_access_grants BEFORE the reader
-- swap, or team/profile resource-grant reads would regress. Idempotent via ON CONFLICT (the unified store's
-- UNIQUE (subject_table, subject_id, principal_table, principal_id)); no kb_resources-subject grant is written
-- anywhere else today, so there is nothing to collide with — the guard is defensive.
-- ============================================================================
INSERT INTO kb_access_grants
    (subject_table, subject_id, principal_table, principal_id,
     can_read, can_write, can_delete, can_grant, granted_by_profile_id, granted_at)
SELECT 'kb_resources', ra.resource_id, ra.anchor_table, ra.anchor_id,
       ra.can_read, ra.can_write, ra.can_delete, ra.can_grant, ra.granted_by_profile_id, ra.granted_at
FROM kb_resource_access ra
ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING;

-- ============================================================================
-- Step 4b — resources_visible_to: D4 body (20260701000002) VERBATIM, except the two direct-resource-grant
-- branches now read kb_access_grants (subject_table='kb_resources'). The D3a explicit-subject-grant branch
-- (cogmap/context homes) already reads kb_access_grants and is carried forward unchanged.
-- ============================================================================
CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    -- owned / originated (the home confers access to its principals)
    SELECT h.resource_id FROM kb_resource_homes h
     WHERE h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile
    UNION
    -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T)). kb_access_grants store.
    SELECT g.subject_id FROM kb_access_grants g
     WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
       AND g.principal_id = p_profile AND g.can_read
    UNION
    -- team-anchored grant on a reachable (self-or-ancestor) team. kb_access_grants store.
    SELECT g.subject_id FROM kb_access_grants g
     JOIN reachable_teams rt ON g.principal_id = rt.team_id
     WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
    UNION
    -- context-share: resources homed in a context shared to a reachable team (WS6 §2)
    SELECT h.resource_id
    FROM kb_team_contexts tc
    JOIN reachable_teams rt ON tc.team_id = rt.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
    UNION
    -- team-owned context: resources homed in a context OWNED by a team the principal is a member of.
    -- FLAT by design (§3 item C: context-ownership is home-like + paired with flat addressability) —
    -- do NOT ancestor-expand this branch.
    SELECT h.resource_id
    FROM kb_contexts c
    JOIN kb_team_members tm
      ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE (self-or-ancestor)
    -- team — UP+union (D4) to match the team-grant/context-share reach above, so map-read and resource-read
    -- agree by construction at the up-expanded level.
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
    UNION
    -- explicit subject-grant (D3a): resources homed in a cogmap or context the profile holds an explicit
    -- read-grant on (direct profile grant OR team grant on a reachable team). kb_access_grants only.
    SELECT h.resource_id
    FROM kb_resource_homes h
    JOIN kb_access_grants g
      ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
    WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts') AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- ============================================================================
-- Step 4c — can_modify_resource: canonical body (20260624000002) VERBATIM, except the two direct-resource
-- WRITE-grant branches now read kb_access_grants (subject_table='kb_resources', can_write).
-- ============================================================================
CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        -- owned / originated (the home confers modify to its principals)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND (h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile)
        UNION ALL
        -- direct profile-anchored WRITE grant. kb_access_grants store.
        SELECT 1 FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_profiles' AND g.principal_id = p_profile AND g.can_write
        UNION ALL
        -- team-anchored WRITE grant on a reachable (self-or-ancestor) team. kb_access_grants store.
        SELECT 1 FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_teams' AND g.can_write
    );
$$;

-- ============================================================================
-- Step 4d — vis_team: canonical body (20260624000002) VERBATIM, except the team-grant branch now reads
-- kb_access_grants. Load-bearing Q-B filter: ONLY (subject_table='kb_resources', principal_table='kb_teams',
-- can_read) — profile-principal grants and context/cogmap subjects never enter the producer intersection.
-- The context-share UNION is unchanged.
-- ============================================================================
CREATE OR REPLACE FUNCTION vis_team(p_team uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT g.subject_id
    FROM team_ancestors(p_team) a
    JOIN kb_access_grants g
      ON g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams'
         AND g.principal_id = a.team_id AND g.can_read
    UNION
    SELECT h.resource_id
    FROM team_ancestors(p_team) a
    JOIN kb_team_contexts tc ON tc.team_id = a.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id;
$$;

-- ============================================================================
-- Step 5 — retire the legacy store. No reader remains (the three functions above were its only readers), so
-- the table drops. Additive-only-on-main holds: this is a forward migration; DROP of a now-orphaned table is
-- the intended endgame of the swap, backfilled above.
-- ============================================================================
DROP TABLE kb_resource_access;
