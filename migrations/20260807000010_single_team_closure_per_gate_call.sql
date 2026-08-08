-- The team closure is expanded ONCE per visibility call, not twice.
--
-- =================================================================================================
-- WHAT THIS IS
-- =================================================================================================
--
-- `20260804000010` gave `reachable_teams` one authoritative home — `profile_reachable_teams` — and
-- routed seven read-path copies through it. It did exactly what it claimed. What it left behind is
-- the residue of that shape: with nine hand-copied CTEs collapsed into one function, the CALL SITES
-- are now nested, and two of them sit on the same path.
--
-- `resources_visible_to` opens with
--
--     WITH reachable_teams AS (SELECT team_id FROM profile_reachable_teams(p_profile))
--
-- and then, in its fourth arm, calls `contexts_readable_by(p_profile)` — whose body opens with the
-- byte-identical CTE and recomputes the whole closure. `profile_reachable_teams` is itself
--
--     SELECT DISTINCT a.team_id
--     FROM profile_effective_teams(p_profile) e
--     CROSS JOIN LATERAL team_ancestors(e.team_id) a
--
-- where `team_ancestors` is a WITH RECURSIVE walk run once per direct membership. So a single
-- `resources_visible_to` call performs TWO full recursive expansions of the profile's team
-- hierarchy.
--
-- Observed in a live `EXPLAIN (ANALYZE)` plan as two independent `Recursive Union` subtrees under
-- one gate call. This is not drift and not a defect in `20260804000010`: both copies are the same
-- function and cannot disagree. It is pure repeated work.
--
-- =================================================================================================
-- WHY IT MATTERS MORE THAN THE MEASUREMENT SUGGESTS
-- =================================================================================================
--
-- On the community instance the two expansions cost 0.085 ms and 0.061 ms against THREE teams at
-- depth ~1, which is why nothing noticed. That instance is the smallest in the install base and its
-- team-anchored grant arm returns ZERO rows — the arms this touches are the ones it does not
-- exercise. Deployments with dozens of nested teams pay two full hierarchy closures per gate call,
-- and a default `list` with the open-meta section makes THREE gate calls, so six closures per
-- request.
--
-- The magnitude is therefore unmeasured here and deliberately not claimed. What is claimed is
-- structural and readable from the two function bodies: the closure was computed twice, and after
-- this migration it is computed once. That claim does not depend on a corpus.
--
-- =================================================================================================
-- HOW: ONE PREDICATE BODY, TWO ENTRY POINTS
-- =================================================================================================
--
-- The obvious fix — inline `contexts_readable_by`'s four arms into `resources_visible_to` so they
-- share the CTE — is refused. It would put the context read-set in two places, on an AUTHORIZATION
-- surface, which is precisely the drift `20260712000010` and `20260804000010` each spent a
-- migration eliminating. Trading a redundant computation for a redundant definition is a bad trade
-- at any speed.
--
-- Instead the body moves to `contexts_readable_by_teams(p_profile, p_teams)`, which takes the
-- already-computed closure, and `contexts_readable_by(p_profile)` becomes a delegating wrapper that
-- computes it. Six other callers of `contexts_readable_by` — `context_readable_by_profile`,
-- `resources_readable_by`, the wayfind anchor set, and others — keep their signature and their
-- behaviour, and get the wrapper. Only `resources_visible_to`, which already holds the set, calls
-- the two-argument form.
--
-- Inside `contexts_readable_by_teams`, `JOIN reachable_teams rt ON rt.team_id = <col>` becomes
-- `<col> = ANY(p_teams)` — set-equal, because `profile_reachable_teams` returns DISTINCT rows so the
-- join could not multiply, and every arm is UNION'd regardless.
--
-- `resources_visible_to`'s own five arms are carried over VERBATIM, joins and all. Only its context
-- arm changes, and only in which function it calls. That is deliberate: on an authorization
-- predicate, the smallest diff that achieves the result is the one whose correctness can be checked
-- by reading it.
--
-- NULL SAFETY, stated because it is the direction that matters on an authz predicate: `x = ANY(NULL)`
-- evaluates to NULL, and a NULL predicate excludes the row. A profile in no teams therefore reaches
-- nothing through the team arms — falls CLOSED, never open. The wrapper additionally coalesces an
-- empty aggregate to `'{}'` so the argument is total rather than relying on that.
--
-- =================================================================================================
-- WHAT IS **NOT** BEING CLAIMED
-- =================================================================================================
--
-- Not a change to who can see what. Every arm, every predicate and their UNION structure are carried
-- over unchanged; the only edits are where the closure is computed and how the team set is spelled.
-- `contexts_readable_by_teams_agrees_with_the_profile_form` and the existing reach-equivalence suite
-- are what hold that.
--
-- Not a caching change. The closure is still computed per call, from base tables, with no
-- memoization anywhere. Caching an authorization predicate is deferred with its precondition named
-- (task 019fdd33) — staleness on this surface is a leak, so invalidation would have to be
-- synchronous with membership and grant writes.
--
-- Not a fix for the gate's larger cost. `resources_visible_to` still materializes the principal's
-- entire visible set for callers that cannot push a predicate into it; PR #659 addressed the two
-- read paths that can, and `filtered_visible_page` still cannot. That work is task 019fdd33.

-- -------------------------------------------------------------------------------------------------
-- 1/3 -- contexts_readable_by_teams: the body, taking the closure rather than computing it.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION contexts_readable_by_teams(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile

    UNION

    -- 2. context OWNED by an enclosing team.
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_teams' AND c.owner_id = ANY(p_teams)

    UNION

    -- 3. context SHARED to an enclosing team
    SELECT tc.context_id
    FROM kb_team_contexts tc
    WHERE tc.team_id = ANY(p_teams)

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team)
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams' AND g.principal_id = ANY(p_teams)) );
$$;

COMMENT ON FUNCTION contexts_readable_by_teams(uuid, uuid[]) IS
    'The context read-set, taking an already-expanded team closure. THE authoritative body; '
    'contexts_readable_by(uuid) is the wrapper that computes the closure for callers that do not '
    'already hold it. Exists so resources_visible_to, which computes the closure for its own arms, '
    'does not pay for a second identical expansion. Team arms use = ANY(p_teams), which is set-equal '
    'to the previous JOIN (profile_reachable_teams returns DISTINCT; every arm is UNIONed). A NULL '
    'or empty p_teams reaches nothing through the team arms — falls closed.';

-- -------------------------------------------------------------------------------------------------
-- 2/3 -- contexts_readable_by: unchanged signature, unchanged result, now a delegating wrapper.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION contexts_readable_by(p_profile uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT t.context_id
    FROM contexts_readable_by_teams(
             p_profile,
             coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams(p_profile)),
                      '{}'::uuid[])
         ) t;
$$;

COMMENT ON FUNCTION contexts_readable_by(uuid) IS
    'Context read predicate — the profile-shaped entry point. Delegates to '
    'contexts_readable_by_teams after expanding the team closure once. Same signature, same rows, '
    'same callers as before; the four arms now live in exactly one place.';

-- -------------------------------------------------------------------------------------------------
-- 3/3 -- resources_visible_to: expands the closure ONCE and threads it through every arm.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- MATERIALIZED is load-bearing and cheap: a handful of team ids. It makes "computed once" a
    -- property of the plan rather than a hope — this CTE now has four references (three joins plus
    -- the aggregate below), and an inlined CTE could be expanded at each, which is the very defect
    -- this migration removes, multiplied.
    WITH reachable_teams AS MATERIALIZED (
        SELECT team_id FROM profile_reachable_teams(p_profile)
    )
    SELECT v.resource_id
    FROM (
        -- owned (the home confers access to its OWNER; originator is provenance only, not access)
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.owner_profile_id = p_profile
        UNION
        -- direct profile-anchored grant (consumer-axis ONLY -- never enters a vis(T))
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
           AND g.principal_id = p_profile AND g.can_read
        UNION
        -- team-anchored grant on a reachable (self-or-ancestor) team
        SELECT g.subject_id FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context the profile can READ.
        --
        -- THE ONLY ARM THIS MIGRATION CHANGES. It called contexts_readable_by(p_profile), which
        -- recomputed the closure this function already has; it now passes it. The aggregate is an
        -- InitPlan over the materialized CTE — one evaluation, no second walk. Every other arm
        -- above and below is carried over verbatim, which is what makes the row set trivially
        -- unchanged rather than merely believed to be.
        SELECT h.resource_id
        FROM contexts_readable_by_teams(
                 p_profile,
                 coalesce((SELECT array_agg(team_id) FROM reachable_teams), '{}'::uuid[])
             ) rc
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
        -- explicit read-grant on a COGMAP home
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        WHERE h.anchor_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ) v
    -- soft-delete READ floor: a deleted resource is invisible on every axis.
    JOIN kb_resources r ON r.id = v.resource_id AND r.is_active;
$$;

COMMENT ON FUNCTION resources_visible_to(uuid) IS
    'THE consumer-axis read predicate: every resource a profile may see, on any axis. Expands the '
    'team closure ONCE into an array and threads it through all six arms including the context arm, '
    'which previously reached it via contexts_readable_by and paid for a second full recursive '
    'expansion. Row set unchanged. Still computed per call from base tables — there is no cache.';

SELECT declare_migration(
    20260807000010,
    'additive',
    'Expands the profile team closure once per resources_visible_to call instead of twice. resources_visible_to computed reachable_teams in its own CTE and then called contexts_readable_by, whose body recomputed the identical closure; since profile_reachable_teams is a per-membership WITH RECURSIVE ancestor walk, one gate call performed two full hierarchy expansions, observed as two independent Recursive Union subtrees in a live plan. The context read-set body moves to a new contexts_readable_by_teams(uuid, uuid[]) taking the already-expanded set, with contexts_readable_by(uuid) kept as a delegating wrapper so its six other callers are untouched — one predicate body, two entry points, rather than inlining the arms into resources_visible_to and putting an authorization predicate in two places. Additive: one new function plus CREATE OR REPLACE on two existing ones at unchanged signatures, no DROP. Row sets are unchanged; team JOINs became = ANY(array), which is set-equal because profile_reachable_teams returns DISTINCT and every arm is UNIONed, and a NULL or empty team set falls closed.'
);
