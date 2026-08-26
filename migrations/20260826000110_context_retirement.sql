-- Context retirement: a context can be made invisible and unwriteable without losing a row.
--
-- Supersedes the hard delete of PR #777. kb_contexts is a replay INPUT table restored verbatim
-- (crates/temper-substrate/src/replay.rs:101-125) and both context projectors RAISE on a missing
-- row (20260731000040:48, 20260715000010:28), so a hard delete breaks replay for any context that
-- was ever renamed or reassigned. A flag rides in with the verbatim restore and breaks nothing.
--
-- ADDITIVE. One new column with a default, and CREATE OR REPLACE on two STABLE read functions
-- whose signatures and return types are unchanged. UNIQUE (owner_table, owner_id, slug) is NOT
-- touched: retire mangles the slug instead, which frees the address without a shape-breaking
-- constraint swap. See internal/superpowers/specs/2026-08-25-context-retirement-design.md.

ALTER TABLE kb_contexts ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;

COMMENT ON COLUMN kb_contexts.is_active IS
'Retirement flag, mirroring kb_teams.is_active. false = retired: every row it homes is preserved,
and the context is addressed on the ADMIN axis, never the read axis.

SCOPED DELIBERATELY, and the scope is the whole design decision. Two chokepoints carry the floor:
contexts_readable_by_teams (which context_visible_to, context_readable_by_profile,
contexts_readable_by and resources_visible_to all delegate to) and context_authorable_by_profile.
Both are the PROFILE axis. Retirement closes reach for people; it is not a containment boundary.

WHAT IS DELIBERATELY NOT FLOORED. The cogmap principal axis still reaches a retired context''s
resources: vis_team joins kb_team_contexts to kb_resource_homes without touching kb_contexts, and
resources_accessible_to_cogmap builds on it, as does resources_readable_by(''context'', ...). That is
intent, not an oversight. Cogmap-principal reach exists so a steward agent can curate its map
against its charter, and a retired context does not thereby become irrelevant to that charter -- it
simply stops contributing new material. The write floor is what makes that true: no new content
event can be produced under a retired context, so it stops advancing the steward watermark even
though it stays in steward_team_contexts. (The retire/restore/rename events themselves do land in
that window, but steward_ingest_delta gates real work on new_resources, which counts only
resource_created, so they cause at most a no-op tick.)

Two consequences worth stating rather than discovering: a resource OWNER keeps read, write, delete
and grant over their own resources in a retired context -- resources_visible_to and
can_modify_resource both have owner-first arms with no context floor, which is what lets an owner
move work out. And reassign is floored (see reassign_service) precisely because it can CHANGE who
the owner is, which would otherwise hand that owner-arm reach to someone retirement had just cut
off.';

-- ============================================================================
-- Chokepoint 1 -- the read axis.
--
-- ARMS 3 AND 4 USE `NOT EXISTS (... AND NOT c.is_active)`, NOT `EXISTS (... AND c.is_active)`.
-- The double negative is deliberate and load-bearing. Those two arms read kb_team_contexts and
-- kb_access_grants, neither of which has a foreign key to kb_contexts, so a share or a grant can
-- name an id with NO row behind it -- and before this migration both arms admitted that id,
-- because neither joined kb_contexts at all. The positive spelling would have SILENTLY REVOKED
-- those dangling grants, which is a behaviour change nobody asked for and which
-- `access_grants_read_wiring::explicit_context_read_grant_confers_context_and_resources` (a
-- pre-existing test using a synthetic anchor) correctly caught. The requirement here is narrow:
-- do not admit a context that is RETIRED. A context that does not exist is somebody else's
-- question, and this migration must not answer it.
--
--   row active  -> no retired row  -> NOT EXISTS = true  -> admit   (unchanged)
--   row retired -> retired row     -> NOT EXISTS = false -> deny    (the floor)
--   no row      -> no retired row  -> NOT EXISTS = true  -> admit   (unchanged, deliberately) Arms 1 and 2 select from kb_contexts and take the floor
-- directly; arms 3 and 4 read kb_team_contexts and kb_access_grants and never join the
-- context row, so they need an EXISTS. Missing either of those two is the silent hole.
-- ============================================================================
CREATE OR REPLACE FUNCTION contexts_readable_by_teams(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE AS $$
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
      AND c.is_active

    UNION

    -- 2. context OWNED by an enclosing team.
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_teams' AND c.owner_id = ANY(p_teams)
      AND c.is_active

    UNION

    -- 3. context SHARED to an enclosing team
    SELECT tc.context_id
    FROM kb_team_contexts tc
    WHERE tc.team_id = ANY(p_teams)
      AND NOT EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = tc.context_id AND NOT c.is_active)

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team)
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams' AND g.principal_id = ANY(p_teams)) )
      AND NOT EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = g.subject_id AND NOT c.is_active);
$$;

-- ============================================================================
-- Chokepoint 2 -- the write axis. Its team arm already floors on kb_teams.is_active; this
-- adds the same shape for the context's own flag. The grant arm delegates to
-- profile_explicit_grant, which knows nothing about contexts, so its floor is added here.
-- ============================================================================
CREATE OR REPLACE FUNCTION context_authorable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1
        FROM kb_contexts c
        WHERE c.id = p_context
          AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
          AND c.is_active

        UNION ALL

        -- team-owned: DIRECT membership in the OWNING team, with an authoring role.
        --
        -- NARROWED. This arm previously ancestor-expanded, so transitive membership in an enclosing
        -- team conferred write on that team's context. Read inherits up the enclosure chain;
        -- mutation does not. `watcher` is read-only. Carried forward verbatim from the definition
        -- this CREATE OR REPLACE supersedes: it states a live invariant, and dropping it invites
        -- the next reader to re-widen the arm.
        SELECT 1
        FROM kb_contexts c
        JOIN kb_team_members tm ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
        JOIN kb_teams t ON t.id = c.owner_id AND t.is_active
        WHERE c.id = p_context
          AND c.owner_table = 'kb_teams'
          AND tm.role IN ('owner', 'maintainer', 'member')
          AND c.is_active
    )
    -- explicit write-grant, floored here because profile_explicit_grant is subject-polymorphic
    -- and cannot know a context is retired.
    OR ( profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context)
         AND EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = p_context AND c.is_active) );
$$;

SELECT declare_migration(
    20260826000110,
    'additive',
    'Context retirement: one defaulted column on kb_contexts plus CREATE OR REPLACE on two STABLE read functions whose signatures and return types are unchanged. A binary predating this migration keeps working -- it reads kb_contexts without the column, every existing row is born is_active = true, and both functions answer identically for an active context. Nothing is dropped: UNIQUE (owner_table, owner_id, slug) stays, and retire mangles the slug instead of relaxing the constraint, which is what keeps this class additive rather than shape-breaking (DEPLOYING.md:68-72). Supersedes the hard delete of PR #777, which could not ship: kb_contexts is a replay input table restored verbatim and both context projectors RAISE on a missing row. Design: internal/superpowers/specs/2026-08-25-context-retirement-design.md.'
);
