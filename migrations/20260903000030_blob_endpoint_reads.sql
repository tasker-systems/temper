-- Blob endpoints are readable as edge endpoints through the blob's OWN home (spec: binary blobs,
-- 2026-09-01, D2 + D3 — blob-visibility-self-contained). This is the DELIBERATE admission, not an
-- accident of the CHECK: until now endpoint_readable_by_profile returned false for any table other
-- than resources/cogmaps, so admitting 'kb_blobs' on kb_edges without this arm would have made
-- every blob-related edge invisible to everyone — and the spec's negative face (graph walks never
-- traverse into blobs) does NOT live here. Walk surfaces stay node-typed: follow-from, the atlas,
-- and the composition acts resolve nodes against kb_resources/kb_cogmaps and never return a blob
-- as a node. An edge listing may render the edge (D3); a walk never materializes the endpoint.
--
-- The predicate is NAMED (blob_readable_by_profile), not inlined: the blob read surfaces (the
-- surfaces task) gate on the same question and must not restate it.

CREATE FUNCTION blob_readable_by_profile(p_profile uuid, p_blob uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_blob_homes h
        WHERE h.blob_id = p_blob
          AND anchor_readable_by_profile(p_profile, h.anchor_table, h.anchor_id)
    );
$$;

CREATE OR REPLACE FUNCTION endpoint_readable_by_profile(p_profile uuid, p_endpoint_table text, p_endpoint_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_endpoint_table
        WHEN 'kb_resources' THEN p_endpoint_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN 'kb_cogmaps'   THEN cogmap_readable_by_profile(p_profile, p_endpoint_id)
        WHEN 'kb_blobs'     THEN blob_readable_by_profile(p_profile, p_endpoint_id)
        ELSE false
    END;
$$;

COMMENT ON FUNCTION endpoint_readable_by_profile(uuid, text, uuid) IS
'scalar edge-endpoint gate: a kb_blobs endpoint is readable through the blob''s OWN home
(blob_readable_by_profile) — the deliberate 20260903000030 admission, not a CHECK accident.
Edge listings may render the edge; walk surfaces stay node-typed and never materialize a blob.';

-- The LIVE edges_visible_to is the set-based rewrite (20260708000009), whose body mirrors the
-- scalar helpers "branch-for-branch" and whose endpoint gates enumerate kb_resources/kb_cogmaps
-- explicitly — so a kb_blobs endpoint falls out of both OR arms and the edge would be invisible
-- to everyone, scalar-arm or not. This OR REPLACE extends it with a readable_blobs set that
-- mirrors blob_readable_by_profile branch-for-branch (a blob's home is a context or a cogmap, so
-- home readability composes from the SAME readable_contexts/readable_cogmaps sets this function
-- already materializes), plus one OR arm per endpoint. The equivalence oracle test
-- (edges_visible_to_equivalence_test) keeps function == scalar-gates honest for the new arm too.
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
        -- team-joined on a reachable (self-or-ancestor) team
        SELECT tc.cogmap_id AS id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        UNION
        -- explicit cogmap read-grant (profile direct, or team on a reachable team)
        SELECT g.subject_id
        FROM kb_access_grants g
        WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ),
    readable_contexts AS (
        -- personal context owned by the principal
        SELECT c.id
        FROM kb_contexts c
        WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
        UNION
        -- context shared to a reachable (self-or-ancestor) team
        SELECT tc.context_id
        FROM kb_team_contexts tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        UNION
        -- context OWNED by a team the principal is a member of (flat — direct membership
        -- via profile_effective_teams, deliberately NOT ancestor-expanded)
        SELECT c.id
        FROM kb_contexts c
        JOIN profile_effective_teams(p_profile) pet ON pet.team_id = c.owner_id
        WHERE c.owner_table = 'kb_teams'
        UNION
        -- explicit context read-grant (profile direct, or team on a reachable team)
        SELECT g.subject_id
        FROM kb_access_grants g
        WHERE g.subject_table = 'kb_contexts' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
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
    20260903000030,
    'additive',
    'blob_readable_by_profile (a blob endpoint is readable iff its own home is readable — blob-visibility-self-contained); the endpoint_readable_by_profile kb_blobs arm; and the set-based edges_visible_to extended with a readable_blobs set + per-endpoint OR arms, mirroring the scalar helpers branch-for-branch as its 20260708000009 rewrite requires — without it a kb_blobs endpoint falls out of both OR arms and every blob-related edge is invisible. This is the DELIBERATE D3 admission: edge listings may render a blob-related edge; graph walks stay node-typed (their node universe is resources_visible_to''s resource set) and never return a blob as a node — exclusion is a decision, not an omission. Design: temper-artifacts specs/2026-09-01-binary-blobs-design.md.'
);
