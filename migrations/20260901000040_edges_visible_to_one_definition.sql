-- edges_visible_to routes through the one definition — again.
--
-- 20260901000020 (blob endpoint reads) extended this function with the readable_blobs set and
-- the kb_blobs endpoint arms, and in the same body restated two fragments whose single homes
-- predate it:
--
--   1. the reachable-teams expression (profile_effective_teams CROSS JOIN LATERAL
--      team_ancestors) — hand-copied instead of routing through profile_reachable_teams,
--      which 20260804000010 established as the one authoritative definition precisely so no
--      function would carry its own copy;
--
--   2. the context read-set as FOUR INLINE ARMS — and the copy was made from the
--      pre-20260712000010 shape, not from contexts_readable_by: the owned-by-team arm joined
--      on profile_effective_teams (flat) where contexts_readable_by joins on
--      reachable_teams (self-or-ancestor). That is not a verbatim relocation; it is DRIFT.
--      The functional face: an edge homed in a context OWNED by an ancestor team became
--      invisible to members of teams beneath it —
--      reachable_teams_one_definition_test::edges_visible_to_reaches_up_the_chain is the
--      witness, and it stayed red across S4/S5 because the test-db tier was not run.
--
-- This migration restores both routings (20260804000010 for the CTE, 20260712000010 for the
-- context read-set) and keeps every 20260901000020 blob addition — readable_blobs and the
-- kb_blobs endpoint arms unchanged, still composed from the same two sets this function
-- materializes, mirroring blob_readable_by_profile branch-for-branch as that migration
-- requires.
--
-- Declared 'additive' under the 2026-08-04 decision: body-only CREATE OR REPLACE, signature
-- unchanged, no caller's contract moves — semantic-equivalence risk is carried by
-- reachable_teams_one_definition_test.rs (the characterization + fold gates it kept red)
-- and the edges_visible_to equivalence oracle.

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
        SELECT context_id AS id FROM contexts_readable_by(p_profile)
    ),
    -- blob endpoints (D2): a blob is readable iff its OWN home is — composed here from the
    -- same two sets, exactly as blob_readable_by_profile composes anchor_readable_by_profile.
    readable_blobs AS (
        SELECT h.blob_id AS id
        FROM kb_blob_homes h
        WHERE (h.anchor_table = 'kb_contexts'
                 AND h.anchor_id IN (SELECT id FROM readable_contexts))
           OR (h.anchor_table = 'kb_cogmaps'
                 AND h.anchor_id IN (SELECT id FROM readable_cogmaps))
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
               AND e.source_id IN (SELECT id FROM readable_cogmaps))
         OR (e.source_table = 'kb_blobs'
               AND e.source_id IN (SELECT id FROM readable_blobs)) )
      AND ( (e.target_table = 'kb_resources'
               AND e.target_id IN (SELECT resource_id FROM vis))
         OR (e.target_table = 'kb_cogmaps'
               AND e.target_id IN (SELECT id FROM readable_cogmaps))
         OR (e.target_table = 'kb_blobs'
               AND e.target_id IN (SELECT id FROM readable_blobs)) );
$$;

SELECT declare_migration(
    20260901000040,
    'additive',
    'edges_visible_to routes the reachable-teams CTE through profile_reachable_teams (20260804000010''s one definition) and the context read-set through contexts_readable_by (20260712000010''s one read-set) again — 20260901000020 had restated both inline and its context copy was the pre-consolidation shape whose owned-by-team arm is flat, so an edge homed in an ancestor-team-OWNED context was invisible to members of teams beneath it (edges_visible_to_reaches_up_the_chain is the witness). Every 20260901000020 blob addition is kept unchanged.'
);
