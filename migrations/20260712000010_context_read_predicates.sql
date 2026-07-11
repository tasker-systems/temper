-- Context access predicates (spec §3.8) — one context read-set, and a mutation gate that is
-- finally distinct from it.
--
-- ---------------------------------------------------------------------------------------------
-- CORRECTING THE RECORD
-- ---------------------------------------------------------------------------------------------
--
-- The spec and plan assert that the `kb_contexts` arm of `anchor_readable_by_profile` "ignores
-- kb_access_grants entirely," citing the header of `20260630000002_access_grants_read_wiring.sql`:
-- *"the one place a context read-grant does not yet reach."*
--
-- That was true on 2026-06-30 and was fixed on 2026-07-01 by
-- `20260701000004_anchor_readable_context_grant.sql` — the migration that quotes it. The design
-- sweep read the stale header rather than the live function. `20260630000002` is checksum-locked
-- and cannot be amended, so the correction is recorded here.
--
-- Verifying that claim against the running database turned up two real defects instead.
--
-- ---------------------------------------------------------------------------------------------
-- THE MODEL (stated, because it was nowhere written down, which is how it rotted)
-- ---------------------------------------------------------------------------------------------
--
-- The team DAG is an org enclosure hierarchy:
--
--     EPD ─▸ engineering ─▸ payroll-product-group ─▸ squad-two
--
-- (plus cross-cutting affinity groups — same mechanism, no special case). Membership is
-- **transitive upward**: a direct member of `squad-two` is thereby a member of every enclosing
-- team. Two axes follow, and they are NOT the same axis:
--
--   READ  inherits UP the enclosure chain. A squad-two member reads what is at or above them:
--         engineering's contexts, EPD's contexts. Never sideways — `squad-one` and
--         `security-it-ops` are invisible. This is what `team_ancestors` expresses: it expands
--         upward FROM the principal's own team, so a thing attached to an ancestor reaches every
--         member beneath it.
--
--   WRITE requires DIRECT membership in the owning team, with an authoring role. Being
--         transitively in `engineering` lets you read engineering's context; it does not let you
--         author into it. `watcher` is read-only everywhere.
--
-- Team-management RBAC (creating/managing sub-teams as an owner/admin of an enclosing team) is a
-- third axis entirely and confers nothing on contexts or resources. It is not touched here.
--
-- ---------------------------------------------------------------------------------------------
-- DEFECT 1 (read, too NARROW): the team-owned arm was flat, in five places
-- ---------------------------------------------------------------------------------------------
--
-- The context-read rule was written out five times — `context_visible_to`, `resources_visible_to`
-- (branch 5), `edges_visible_to`, `graph_home_contexts`, `resources_in_team_scope` — and every copy
-- gated the team-OWNED arm on DIRECT membership only. So a squad-two member could read a context
-- *shared to* engineering but not the context engineering *owns*, which under the model above is
-- incoherent: engineering's own context is the most obviously-theirs thing there is.
--
-- The copies had already begun to drift from each other, which is the real lesson:
-- `graph_home_contexts` had gone flat on the SHARE arm too, and its `candidates` CTE is documented
-- as "a proven superset (same branches)" of `context_visible_to` — a claim that held only while
-- both were equally wrong. Widening the predicate alone would have silently made it a SUBSET and
-- dropped contexts out of the graph view.
--
-- So this migration does not widen five copies. It creates ONE — `contexts_readable_by(p_profile)`
-- — and routes all five through it. There is nothing left to drift.
--
-- ---------------------------------------------------------------------------------------------
-- DEFECT 2 (write, too WIDE): mutation inherited up the chain, and role gated nothing
-- ---------------------------------------------------------------------------------------------
--
-- `context_authorable_by_profile`'s team-owned arm ancestor-expanded. Combined with defect 1, that
-- produced a write-wider-than-read inversion on the same object: a squad-two member could AUTHOR
-- into engineering's context while being unable to READ it. And no access predicate anywhere
-- consulted `kb_team_members.role` (0 of 15) — a `watcher` could author.
--
-- The write arm is therefore NARROWED to direct membership in the owning team with an authoring
-- role (owner / maintainer / member; `watcher` is read-only). This REVOKES write that exists today
-- — the only non-additive change in this migration, and deliberate. It is safe now because the
-- deployment is a handful of alpha testers; it would not be later.
--
-- Explicit `kb_access_grants` WRITE grants are untouched and still reach through `team_ancestors`.
-- A grant is a deliberate act of delegation, not an accident of enclosure — granting write to an
-- umbrella team is a considered decision to let everyone under it author.

-- =================================================================================================
-- THE ONE CONTEXT READ-SET. Everything that asks "which contexts can this profile read?" asks here.
-- =================================================================================================
CREATE OR REPLACE FUNCTION contexts_readable_by(p_profile uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        -- The principal's own teams, expanded UP to their enclosing teams. A thing attached to an
        -- ancestor is therefore reachable by every member beneath it — reads inherit DOWN the tree.
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile

    UNION

    -- 2. context OWNED by an enclosing team. FIXED: this arm was flat (direct members only) in all
    --    five copies. Transitive membership in engineering means engineering's own context reads.
    SELECT c.id
    FROM kb_contexts c
    JOIN reachable_teams rt ON rt.team_id = c.owner_id
    WHERE c.owner_table = 'kb_teams'

    UNION

    -- 3. context SHARED to an enclosing team
    SELECT tc.context_id
    FROM kb_team_contexts tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team)
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'
               AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

COMMENT ON FUNCTION contexts_readable_by(uuid) IS
    'THE context read-set for a profile — the single body behind context_readable_by_profile, '
    'context_visible_to, resources_visible_to, edges_visible_to, graph_home_contexts and '
    'resources_in_team_scope. Four arms: personal; owned by an enclosing team; shared to an '
    'enclosing team; explicit read-grant. Read inherits UP the enclosure chain and never sideways.';

-- The boolean grain. Delegates rather than restating the four arms — restating them is exactly how
-- the five copies drifted. Context counts are small; correctness over a micro-optimization we have
-- no evidence we need.
CREATE OR REPLACE FUNCTION context_readable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM contexts_readable_by(p_profile) c WHERE c.context_id = p_context
    );
$$;

COMMENT ON FUNCTION context_readable_by_profile(uuid, uuid) IS
    'Context read predicate — peer of cogmap_readable_by_profile. Delegates to contexts_readable_by.';

-- The addressability grain. This was the copy that had drifted furthest; it now cannot drift.
CREATE OR REPLACE FUNCTION context_visible_to(p_principal uuid, p_context_id uuid)
RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT context_readable_by_profile(p_principal, p_context_id);
$$;

COMMENT ON FUNCTION context_visible_to(uuid, uuid) IS
    'Delegates to context_readable_by_profile. Retained as the name existing callers use.';

-- The edge-home grain.
CREATE OR REPLACE FUNCTION anchor_readable_by_profile(
    p_profile uuid, p_anchor_table text, p_anchor_id uuid
) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN context_readable_by_profile(p_profile, p_anchor_id)
        ELSE false
    END;
$$;

-- =================================================================================================
-- THE MUTATION GATE — deliberately NOT the read-set.
-- =================================================================================================
CREATE OR REPLACE FUNCTION context_authorable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1
        FROM kb_contexts c
        WHERE c.id = p_context
          AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile

        UNION ALL

        -- team-owned: DIRECT membership in the OWNING team, with an authoring role.
        --
        -- NARROWED. This arm previously ancestor-expanded, so transitive membership in an enclosing
        -- team conferred write on that team's context. Read inherits up the enclosure chain;
        -- mutation does not. `watcher` is read-only.
        SELECT 1
        FROM kb_contexts c
        JOIN kb_team_members tm ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
        JOIN kb_teams t ON t.id = c.owner_id AND t.is_active
        WHERE c.id = p_context
          AND c.owner_table = 'kb_teams'
          AND tm.role IN ('owner', 'maintainer', 'member')
    )
    -- explicit write-grant (profile- or reachable-team-anchored). Untouched: a grant is a deliberate
    -- act of delegation, and granting write to an umbrella team is a considered decision.
    OR profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context);
$$;

COMMENT ON FUNCTION context_authorable_by_profile(uuid, uuid) IS
    'Context mutation gate. Team-owned contexts require DIRECT membership in the owning team with '
    'an authoring role (owner/maintainer/member; watcher is read-only) — mutation does NOT inherit '
    'up the enclosure chain, unlike read. Explicit write-grants still reach through team_ancestors.';

-- =================================================================================================
-- ROUTE THE FIVE COPIES THROUGH THE ONE READ-SET.
-- =================================================================================================

-- resources_visible_to: the three context arms (share / team-owned / explicit-context-grant) collapse
-- into "resources homed in a context I can read." Every other branch is reproduced VERBATIM.
CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT v.resource_id
    FROM (
        WITH reachable_teams AS (
            SELECT DISTINCT a.team_id
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
        )
        -- owned / originated (the home confers access to its principals)
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile
        UNION
        -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T))
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
           AND g.principal_id = p_profile AND g.can_read
        UNION
        -- team-anchored grant on a reachable (self-or-ancestor) team
        SELECT g.subject_id FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context the profile can READ. Replaces the three former arms
        -- (context-share, team-owned-context, explicit context read-grant) with the one read-set —
        -- and thereby picks up the team-owned fix. Also newly admits resources in the profile's OWN
        -- personal context that they neither own nor originated, which was an oversight.
        SELECT h.resource_id
        FROM contexts_readable_by(p_profile) rc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = rc.context_id
        UNION
        -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit read-grant on a COGMAP home (the kb_contexts half of this branch is now covered
        -- by contexts_readable_by above)
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        WHERE h.anchor_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ) v
    -- soft-delete READ floor: a deleted resource is invisible on every axis.
    JOIN kb_resources r ON r.id = v.resource_id AND r.is_active;
$$;

-- edges_visible_to: its inline readable_contexts CTE becomes the one read-set. The cogmap side and
-- the edge-gating logic are reproduced VERBATIM.
CREATE OR REPLACE FUNCTION edges_visible_to(p_profile uuid)
RETURNS TABLE(edge_id uuid)
LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    ),
    vis AS (
        SELECT resource_id FROM resources_visible_to(p_profile)
    ),
    readable_cogmaps AS (
        SELECT tc.cogmap_id AS id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        UNION
        SELECT g.subject_id
        FROM kb_access_grants g
        WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ),
    readable_contexts AS (
        -- was four inline arms, one of them flat; now the one read-set
        SELECT context_id AS id FROM contexts_readable_by(p_profile)
    )
    SELECT e.id
    FROM kb_edges e
    WHERE NOT e.is_folded
      AND ( (e.home_anchor_table = 'kb_cogmaps'
               AND e.home_anchor_id IN (SELECT id FROM readable_cogmaps))
         OR (e.home_anchor_table = 'kb_contexts'
               AND e.home_anchor_id IN (SELECT id FROM readable_contexts)) )
      AND ( (e.source_table = 'kb_resources'
               AND e.source_id IN (SELECT resource_id FROM vis))
         OR (e.source_table = 'kb_cogmaps'
               AND e.source_id IN (SELECT id FROM readable_cogmaps)) )
      AND ( (e.target_table = 'kb_resources'
               AND e.target_id IN (SELECT resource_id FROM vis))
         OR (e.target_table = 'kb_cogmaps'
               AND e.target_id IN (SELECT id FROM readable_cogmaps)) );
$$;

-- graph_home_contexts: the `candidates` CTE becomes the one read-set. Its header claimed to be "a
-- proven superset (same branches)" of context_visible_to — a claim that held only while both were
-- equally wrong (it had ALSO gone flat on the share arm). Now it is the same set by construction,
-- so the trailing context_visible_to gate is exact rather than merely confirming.
CREATE OR REPLACE FUNCTION graph_home_contexts(p_profile uuid)
RETURNS TABLE(context_id uuid, name text, slug text, owner_ref text, resource_count integer, last_active_at timestamptz)
LANGUAGE sql STABLE AS $$
    SELECT c.id, c.name, c.slug,
           CASE
               WHEN c.owner_table = 'kb_profiles' AND c.owner_id = p_profile THEN '@me'
               WHEN c.owner_table = 'kb_teams' AND owner_team.slug IS NOT NULL THEN '+' || owner_team.slug
               -- Owned by another profile but visible (team-share → that team; otherwise an
               -- explicit read-grant) — label it 'shared', never mis-claim it as '@me'.
               ELSE COALESCE('+' || shared.slug, 'shared')
           END AS owner_ref,
           (SELECT count(*)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id)::int AS resource_count,
           -- Same counted set as resource_count above, so recency can never reflect a resource the
           -- caller can't see or one that's been soft-deleted.
           (SELECT max(rr.updated)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id) AS last_active_at
    FROM contexts_readable_by(p_profile) cand
    JOIN kb_contexts c ON c.id = cand.context_id
    LEFT JOIN kb_teams owner_team ON c.owner_table = 'kb_teams' AND owner_team.id = c.owner_id
    LEFT JOIN LATERAL (
        -- team this context is shared INTO that the profile effectively belongs to
        SELECT tt.slug
        FROM kb_team_contexts tc
        JOIN profile_effective_teams(p_profile) pet ON pet.team_id = tc.team_id
        JOIN kb_teams tt ON tt.id = tc.team_id
        WHERE tc.context_id = c.id
        ORDER BY tt.slug
        LIMIT 1
    ) shared ON true
    ORDER BY owner_ref, c.name;
$$;

-- resources_in_team_scope: the team-owned-context arm self-gated on DIRECT membership, on the
-- explicit rationale that "team-owned context is FLAT in the visibility model." That premise is now
-- false. The arm stays scope-bounded (owner ∈ scope_teams) and the trailing intersection with
-- resources_visible_to does the gating, exactly as the other arms already rely on. Every other
-- branch reproduced VERBATIM.
CREATE OR REPLACE FUNCTION resources_in_team_scope(p_profile uuid, p_team uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql STABLE AS $$
    WITH scope_teams AS (
        SELECT a.team_id FROM team_ancestors(p_team) a
    ),
    scoped AS (
        -- team-anchored resource read-grant on a scope team
        SELECT g.subject_id AS resource_id
        FROM kb_access_grants g
        JOIN scope_teams st ON g.principal_id = st.team_id
        WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context SHARED to a scope team
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        UNION
        -- resources homed in a context OWNED by a scope team
        SELECT h.resource_id
        FROM kb_contexts c
        JOIN scope_teams st ON c.owner_table = 'kb_teams' AND c.owner_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        UNION
        -- resources homed in a cogmap JOINED to a scope team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit container-grant: resources homed in a context/cogmap the scope team holds a
        -- read-grant on
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        JOIN scope_teams st ON g.principal_id = st.team_id
        WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts')
          AND g.principal_table = 'kb_teams' AND g.can_read
    )
    SELECT s.resource_id
    FROM scoped s
    JOIN resources_visible_to(p_profile) v ON v.resource_id = s.resource_id;
$$;

-- =================================================================================================
-- resources_readable_by gains the 'context' principal kind (spec §3.8 item 2).
--
-- Note the shape: this is LANGUAGE sql — a UNION whose arms are guarded by
-- `WHERE p_principal_kind = …`, NOT a plpgsql IF/ELSIF. An unhandled kind returns ZERO ROWS rather
-- than raising. That fail-closed behavior is pre-existing and deliberately left alone.
-- =================================================================================================
CREATE OR REPLACE FUNCTION resources_readable_by(p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT resource_id FROM resources_visible_to(p_principal_id)           WHERE p_principal_kind = 'profile'
    UNION
    SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id) WHERE p_principal_kind = 'cogmap'
    UNION
    -- the context's own interior, under the same soft-delete read floor
    SELECT h.resource_id
      FROM kb_resource_homes h
      JOIN kb_resources r ON r.id = h.resource_id AND r.is_active
     WHERE p_principal_kind = 'context'
       AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_principal_id;
$$;

COMMENT ON FUNCTION resources_readable_by(text, uuid) IS
    'Resource read-set for a principal. Kinds: profile, cogmap, context. An unknown kind returns '
    'zero rows (fail-closed), not an error — this is LANGUAGE sql with guarded UNION arms.';
