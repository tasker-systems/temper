-- `reachable_teams` gets one authoritative home — the read path.
--
-- =================================================================================================
-- WHAT THIS IS
-- =================================================================================================
--
-- One expression decides, everywhere in this schema, which teams a profile's reads and writes
-- reach:
--
--     SELECT DISTINCT a.team_id
--     FROM profile_effective_teams(p_profile) e
--     CROSS JOIN LATERAL team_ancestors(e.team_id) a
--
-- It expands the profile's DIRECT team memberships UP the enclosure chain, so a thing attached to
-- an ancestor reaches every member beneath it. `team_ancestors` is itself a WITH RECURSIVE walk,
-- run once per direct team.
--
-- Measured 2026-08-04 against the live schema, that expression was hand-copied verbatim into NINE
-- functions -- byte-identical once normalized for whitespace and parameter name, two of them merely
-- naming the CTE `member_teams` instead of `reachable_teams`. Not drifted. Nine copies in lockstep
-- by coincidence.
--
-- This migration does not widen nine copies. It creates ONE -- `profile_reachable_teams(p_profile)`
-- -- and routes the seven READ-path copies through it. The two remaining copies are gates on
-- MUTATION and are routed by `20260804000020`, deliberately separate; see WHY THE SPLIT below.
--
-- Exercises goal 019fb881 ("Principal-agnostic substrate vs. principal constraint in SQL"),
-- clause 1: "A principal-agnostic fragment shared by more than one function has exactly one
-- authoritative definition." The goal was `Never run` before this.
--
-- The move has precedent in this tree: `20260712000010_context_read_predicates.sql` did exactly
-- this for the context read-set -- "This migration does not widen five copies. It creates ONE --
-- contexts_readable_by(p_profile) -- and routes all five through it. There is nothing left to
-- drift." That migration also recorded that its five copies had ALREADY begun drifting from each
-- other. These nine had not yet. Doing it before the drift is the point; after it, the same work
-- also has to adjudicate which copy was right.
--
-- =================================================================================================
-- WHAT IS **NOT** BEING CLAIMED
-- =================================================================================================
--
-- **No performance claim.** Goal 019fb881's own framing note is binding here: "a regular Postgres
-- view is not itself an optimization -- the planner inlines it exactly like a CTE, and there is no
-- separate plan or place to hang an index." This buys clause 1 (drift-safety) and clause 4
-- (a fragment separably plannable from any single consumer). It does NOT reduce the number of
-- computations, and nothing here was measured for cost.
--
-- Specifically NOT addressed, and named so this does not read as covering it: the spine nests on
-- itself. `resources_visible_to(p)` computes reachable teams AND calls `contexts_readable_by(p)`,
-- which computes them again -- and after this migration both of those go through the same one
-- relation, which changes the drift story and not the call count. Collapsing that needs
-- EXPLAIN ANALYZE first: the planner may already fold it, in which case "collapsing" buys risk
-- alone.
--
-- =================================================================================================
-- WHY THE SPLIT (this migration is the read path only)
-- =================================================================================================
--
-- Nine copies, two risk classes. Seven are read predicates. Two gate mutation:
--
--   * `can_modify_resource`   -- the write gate itself.
--   * `profile_explicit_grant` -- action-parameterized (read/write/delete/grant), and called by
--                                 `can`, `cogmap_authorable_by_profile`, `context_authorable_by_profile`
--                                 as well as `cogmap_readable_by_profile`.
--
-- `profile_explicit_grant` serves read callers too, and is still routed with the write gates rather
-- than here. Applying the risk rule to a function's WORST caller is the point of having the rule.
--
-- An equivalence defect in a read predicate shows up as material that should have been visible and
-- was not (or the reverse). In a write gate it shows up as a mutation that should have been refused
-- and was not. Those need to be revertible independently.
--
-- CONSEQUENCE, stated rather than left to be discovered: **clause 1 is NOT satisfied when this
-- migration alone has been applied.** Two hand-copies remain live until `20260804000020` lands, and
-- the clause-1 witness (`reachable_teams_one_definition_test.rs`) stays red until then. That is the
-- intended intermediate state, not a defect.
--
-- =================================================================================================
-- CLASSIFICATION
-- =================================================================================================
--
-- Every statement below is a body-only CREATE OR REPLACE with an UNCHANGED signature.
--
-- **Declared `additive`** `[decided -- 2026-08-04, Pete]`, and this decides a question goal 019fb881
-- deliberately left open, so it is recorded rather than assumed.
--
-- That goal's refusal face reads: "The deploy should decline to auto-apply -- as additive -- a
-- refactor that redefines a function body while leaving its signature unchanged, because that class
-- can silently shift result semantics (the value-level shape of the 2026-07-30 outage)." It then
-- names its own uncertainty: "Whether the #589 classifier can express 'body changed, signature did
-- not => not-additive' is a declared hole."
--
-- The call, and the reasoning behind it: the signatures are unchanged, so nothing here bears on
-- outage risk or downtime readiness -- which is what the additive/shape-breaking axis is actually
-- for. `shape-breaking` gates a migration on an OPERATOR being present because the schema and the
-- binary that speaks it could disagree across the apply. That cannot happen here: no caller's
-- contract moves, and a binary deployed before or after reads these functions identically.
-- Semantic-equivalence risk is real but is a DIFFERENT axis, and it is what the witness is for --
-- `reachable_teams_one_definition_test.rs`, nine characterizations across all nine routed functions
-- plus a bite probe establishing that the tier discriminates.
--
-- The consequence to be honest about: on this reading, the refusal face as literally written is not
-- exercised by this migration and, applied consistently, would not be exercised by any extraction
-- under this goal. That is a live disagreement with the goal's text, not a gap in it, and it belongs
-- back in the goal rather than only here.

-- -------------------------------------------------------------------------------------------------
-- The one definition.
-- -------------------------------------------------------------------------------------------------
--
-- Parameterized by the principal, never embedding one -- goal 019fb881 negative face 2: "a shared
-- view/SRF must never embed a specific principal: it is principal-agnostic, or it is not shared."
--
-- STABLE, LANGUAGE sql, and deliberately inlineable: this is a shared *definition*, not a
-- materialization. Negative face 1 governs the read-set it feeds -- "must never be precomputed into
-- a stored or materialized per-principal artifact" -- and nothing here stores anything.
CREATE OR REPLACE FUNCTION profile_reachable_teams(p_profile uuid)
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- The principal's own teams, expanded UP to their enclosing teams. A thing attached to an
    -- ancestor is therefore reachable by every member beneath it -- reads inherit DOWN the tree.
    -- `team_ancestors` already filters soft-deleted teams, so this confers nothing through one.
    SELECT DISTINCT a.team_id
    FROM profile_effective_teams(p_profile) e
    CROSS JOIN LATERAL team_ancestors(e.team_id) a;
$$;

COMMENT ON FUNCTION profile_reachable_teams(uuid) IS
    'The one definition of which teams a profile reaches: direct memberships expanded up the '
    'enclosure chain. Join this; never restate it. Nine hand-copies were folded into it on '
    '2026-08-04 (migrations 20260804000010 and 20260804000020). Guarded by '
    'reachable_teams_one_definition_test.rs, which scans pg_proc for new copies.';

-- -------------------------------------------------------------------------------------------------
-- 1/7 -- contexts_readable_by. Routed first because resources_visible_to calls it.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION contexts_readable_by(p_profile uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql
STABLE
AS $$
    WITH reachable_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_profile)
    )
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile

    UNION

    -- 2. context OWNED by an enclosing team.
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

-- -------------------------------------------------------------------------------------------------
-- 2/7 -- resources_visible_to.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT v.resource_id
    FROM (
        WITH reachable_teams AS (
            SELECT team_id FROM profile_reachable_teams(p_profile)
        )
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
        -- resources homed in a context the profile can READ
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
        -- explicit read-grant on a COGMAP home
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

-- -------------------------------------------------------------------------------------------------
-- 3/7 -- cogmap_visible_maps.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cogmap_visible_maps(p_principal uuid)
RETURNS SETOF uuid
LANGUAGE sql
STABLE
AS $$
    WITH reachable_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_principal)
    )
    SELECT DISTINCT tc.cogmap_id
    FROM kb_team_cogmaps tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    UNION
    -- explicit cogmap read-grant (direct profile, or team grant on a reachable team) admits the map
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_principal)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- -------------------------------------------------------------------------------------------------
-- 4/7 -- cogmap_readable_by_profile. This copy was an inline subquery, not a named CTE.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cogmap_readable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps tc
        JOIN profile_reachable_teams(p_profile) rt ON rt.team_id = tc.team_id
        WHERE tc.cogmap_id = p_cogmap
    )
    OR profile_explicit_grant(p_profile, 'read', 'kb_cogmaps', p_cogmap);
$$;

-- -------------------------------------------------------------------------------------------------
-- 5/7 -- edges_visible_to.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION edges_visible_to(p_profile uuid)
RETURNS TABLE(edge_id uuid)
LANGUAGE sql
STABLE
AS $$
    WITH reachable_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_profile)
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

-- -------------------------------------------------------------------------------------------------
-- 6/7 -- cogmap_list_rows. This copy is named `member_teams`; the name is kept so the rest of the
-- body is untouched.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cogmap_list_rows(p_profile uuid, p_cogmap uuid DEFAULT NULL::uuid)
RETURNS TABLE(cogmap_id uuid, name text, owner_ref text, team_ids uuid[], region_count integer, resource_count integer, telos_resource_id uuid, charter_statement text)
LANGUAGE sql
STABLE
AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    member_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_profile)
    )
    SELECT c.id, c.name,
           COALESCE('+' || min(mt.slug), 'temper') AS owner_ref,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ) AS team_ids,
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int AS region_count,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int AS resource_count,
           c.telos_resource_id,
           (SELECT rb.body_text
              FROM resource_blocks(c.telos_resource_id, 'profile', p_profile, 'statement') rb
             ORDER BY rb.seq
             LIMIT 1) AS charter_statement
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    LEFT JOIN kb_teams mt ON mt.id = tc.team_id AND tc.team_id IN (SELECT team_id FROM member_teams)
    WHERE (p_cogmap IS NULL OR c.id = p_cogmap)
    GROUP BY c.id, c.name, c.telos_resource_id
    ORDER BY owner_ref, c.name;
$$;

-- -------------------------------------------------------------------------------------------------
-- 7/7 -- graph_home_cogmaps. Also `member_teams`.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION graph_home_cogmaps(p_profile uuid)
RETURNS TABLE(cogmap_id uuid, name text, owner_ref text, team_ids uuid[], region_count integer, facet_count integer)
LANGUAGE sql
STABLE
AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    -- reachable teams = self + ancestors (is_active), mirroring cogmap_visible_maps'
    -- admit basis, so the derived held-by owner_ref / team_ids reflect the team that
    -- actually made the map visible -- an ancestor-held map reads as that team, not the
    -- universal 'temper' marker, and soft-deleted teams confer nothing.
    member_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_profile)
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

SELECT declare_migration(
    20260804000010,
    'additive',
    'reachable_teams gets one authoritative home (goal 019fb881 clause 1, first exercise): new profile_reachable_teams(p_profile), with the seven READ-path hand-copies routed through it. Additive because every signature is unchanged, so no deployed binary can disagree with this schema across the apply -- the axis the additive/shape-breaking split actually governs. Semantic equivalence is a separate axis, carried by reachable_teams_one_definition_test.rs (nine characterizations plus a bite probe). This decides against a literal reading of that goal''s refusal face; see the file header. The two mutation-gating copies are routed separately by 20260804000020.'
);
