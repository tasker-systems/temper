-- kb_blobs identity: PER-HOME (D2 as amended 2026-09-02 — temper-artifacts/specs/
-- 2026-09-01-binary-blobs-design.md, D2 amendment block; vault copy 01a05d01-648e-74d1-a6b4-345c9bde744b).
--
-- The original D2 made content_hash globally UNIQUE — one row per byte-sequence, first home
-- stands — so a second principal committing byte-identical bytes received the FIRST principal's
-- row id: a 200 carrying an identity they could not read, relate, or list, whose uuidv7
-- timestamp dated another principal's commit. A recourse dead-end, and a merged-provenance
-- record: the row a principal could "read" was asserted by another principal's event. Access
-- granted on upload must be access to a record of the uploader's own act.
--
-- D1 is UNCHANGED: the provider pathname stays content-addressed ({hash[0:2]}/{hash}), so
-- cross-principal STORAGE dedup is pathname-level and unaffected — identical bytes still land
-- as one object however many rows reference them. What is surrendered is cross-principal
-- LEDGER identity: the same bytes in another scope are invisible here and commit as the
-- caller's own fresh row, asserted by the caller's own event.
--
-- Mechanically: the home folds into the blob row (the kb_blob_homes table — one home per blob,
-- its own masked surrogate — carried nothing the row cannot), uniqueness moves to the home
-- scope, get-or-create in the projector becomes per-scope, and the two readability routings
-- (the scalar predicate and edges_visible_to's readable_blobs set) re-anchor from the homes
-- join to the row's own columns — branch-for-branch, so the equivalence oracle's mirroring
-- requirement (20260903000030/20260903000050) still holds verbatim.

ALTER TABLE kb_blobs
    ADD COLUMN home_table            VARCHAR(64),
    ADD COLUMN home_id               UUID,
    ADD COLUMN owner_profile_id      UUID REFERENCES kb_profiles(id),
    ADD COLUMN originator_profile_id UUID REFERENCES kb_profiles(id);

-- Backfill from the homes table before it goes: one home per blob (kb_blob_homes.blob_id was
-- UNIQUE), so this is a 1:1 move, not a merge.
UPDATE kb_blobs b
   SET home_table            = h.anchor_table,
       home_id               = h.anchor_id,
       owner_profile_id      = h.owner_profile_id,
       originator_profile_id = h.originator_profile_id
  FROM kb_blob_homes h
 WHERE h.blob_id = b.id;

ALTER TABLE kb_blobs
    ALTER COLUMN home_table            SET NOT NULL,
    ALTER COLUMN home_id               SET NOT NULL,
    ALTER COLUMN owner_profile_id      SET NOT NULL,
    ALTER COLUMN originator_profile_id SET NOT NULL;

ALTER TABLE kb_blobs
    ADD CONSTRAINT kb_blobs_home_table_check
        CHECK (home_table IN ('kb_contexts', 'kb_cogmaps'));

-- The identity swap: global hash-uniqueness OUT (it was the defect's source), home-scoped
-- uniqueness IN. The old constraint's story — "D1's dedup made a constraint" — was the
-- conflation this migration unwinds: D1's dedup lives at the pathname, which rows continue to
-- share for identical bytes.
ALTER TABLE kb_blobs DROP CONSTRAINT kb_blobs_content_hash_key;
ALTER TABLE kb_blobs
    ADD CONSTRAINT kb_blobs_home_scope_key UNIQUE (home_table, home_id, content_hash);
CREATE INDEX idx_kb_blobs_home ON kb_blobs(home_table, home_id);

DROP TABLE kb_blob_homes;

-- Get-or-create, scoped: a hash conflict WITHIN the payload's home returns the existing row
-- (the bytes are provably at the shared pathname); a hash known only to OTHER scopes inserts a
-- fresh row — the caller's id, the caller's event, the caller's commit time. Replaying N
-- commits of identical bytes from N homes reproduces N rows in event order (each row's
-- assert/created come from its own event), so full-dump equivalence is replay-stable.
CREATE OR REPLACE FUNCTION _project_blob_committed(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_id       uuid := (p_payload->>'blob_id')::uuid;
        v_hash     text := p_payload->>'content_hash';
        v_home_table text := p_payload#>>'{home,table}';
        v_home_id    uuid := (p_payload#>>'{home,id}')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_inserted uuid;
BEGIN
    INSERT INTO kb_blobs (id, content_hash, blob_pathname, content_type, content_bytes,
                          home_table, home_id, owner_profile_id, originator_profile_id,
                          asserted_by_event_id, last_event_id, created)
    VALUES (v_id, v_hash, p_payload->>'blob_pathname', p_payload->>'content_type',
            (p_payload->>'content_bytes')::bigint,
            v_home_table, v_home_id,
            -- owner/originator mapping (N4, 2026-09-03 review): owner ← the payload's owner,
            -- originator ← COALESCE(originator, owner) — the ResourceCreated precedent
            -- (20260624000002 _project_resource_created) and this migration's own backfill
            -- (:32-38) both copy straight across. The swapped shape was inert while every
            -- commit passed originator: None (both mappings degenerate to owner=caller) and
            -- replay-stable for the same reason, but would have minted swapped provenance the
            -- moment on-behalf-of is threaded, and the erasure joins key these columns.
            (p_payload->>'owner_profile_id')::uuid,
            COALESCE((p_payload->>'originator_profile_id')::uuid,
                     (p_payload->>'owner_profile_id')::uuid),
            p_event, p_event, v_occurred)
    ON CONFLICT (home_table, home_id, content_hash) DO NOTHING
    RETURNING id INTO v_inserted;

    IF v_inserted IS NULL THEN
        -- Get-or-create WITHIN the caller's own scope: the caller's home already holds these
        -- bytes, and their re-commit returns that row. A hash homed elsewhere never reaches
        -- this arm — the unique key cannot see other scopes, which is the point.
        SELECT id INTO v_inserted FROM kb_blobs
         WHERE home_table = v_home_table
           AND home_id    = v_home_id
           AND content_hash = v_hash;
    END IF;

    RETURN ARRAY[v_inserted];
END;
$$;

COMMENT ON FUNCTION _project_blob_committed(uuid, jsonb) IS
'blob_committed projector (final definition — 20260903000020 minted the global-hash shape,
this migration scopes it): get-or-create scoped to the payload''s OWN home — UNIQUE(home_table,
home_id, content_hash); a hash conflict within the caller''s scope returns the existing row,
a hash known only to other scopes inserts the caller''s fresh row. Replay reproduces N rows
for N home-scoped commits of identical bytes, in event order.';

-- The scalar read gate re-anchored: the row IS its home now. anchor_readable_by_profile stays
-- the one readability helper (the same post-consolidation routing the S6 verification pinned);
-- only the FROM/WHERE moves off the dropped table.
CREATE OR REPLACE FUNCTION blob_readable_by_profile(p_profile uuid, p_blob uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_blobs b
        WHERE b.id = p_blob
          AND anchor_readable_by_profile(p_profile, b.home_table, b.home_id)
    );
$$;

COMMENT ON FUNCTION blob_readable_by_profile(uuid, uuid) IS
'authorization predicate: a blob is readable iff its OWN home is — pure anchor equality
(anchor_readable_by_profile on the row''s home_table/home_id), never a currency predicate
(no is_folded/ingest_state here). Harness-enrolled as a DUALITY witness against the anchor
predicate: any edit must keep the anchor-equality answer. The erasure build (unbuilt) must
widen this and its dedup/read siblings (N3) — kb_blobs.content_type is nullable by design.';

-- edges_visible_to: only the readable_blobs set changes — FROM the row, home columns in place
-- of the homes join. Every other fragment is the 20260903000050 body verbatim (the one-
-- definition routings it restored are not touched here).
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
    -- blob endpoints (D2 as amended): a blob is readable iff its OWN home is — the row's home
    -- columns now, composed from the same two sets, mirroring blob_readable_by_profile
    -- branch-for-branch.
    readable_blobs AS (
        SELECT b.id AS id
        FROM kb_blobs b
        WHERE (b.home_table = 'kb_contexts'
                 AND b.home_id IN (SELECT id FROM readable_contexts))
           OR (b.home_table = 'kb_cogmaps'
                 AND b.home_id IN (SELECT id FROM readable_cogmaps))
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

COMMENT ON FUNCTION edges_visible_to(uuid) IS
'the ONE set-based edge-visibility function (routings restored by 20260804000010 +
20260712000010; 20260903000050 added the blob arms; this migration re-anchors readable_blobs
from the dropped homes join to the row''s home columns, branch-for-branch with
blob_readable_by_profile): an edge is visible iff it is live, its home anchor is readable,
and BOTH endpoints are readable — blob endpoints through the blob''s own home. Supersedes the
intermediate definitions in 20260903000030/20260903000050.';

SELECT declare_migration(
    20260903000060,
    'additive',
    'kb_blobs identity per home (D2 as amended 2026-09-02): the home folds into the row (home_table/home_id/owner_profile_id/originator_profile_id, backfilled 1:1 from kb_blob_homes before the drop), global content_hash UNIQUE swapped for UNIQUE(home_table, home_id, content_hash), the projector''s get-or-create scoped to the payload''s own home (a cross-scope hash match is the caller''s own fresh row, asserted by the caller''s event — never another principal''s identity), and the two readability routings (blob_readable_by_profile, edges_visible_to''s readable_blobs set) re-anchored from the homes join to the row''s columns branch-for-branch. D1 unchanged: storage dedup stays pathname-level. The original shape''s defect: a second principal committing byte-identical bytes received the first principal''s row id — unreadable to them, uuidv7-dated to their commit. Design: temper-artifacts specs/2026-09-01-binary-blobs-design.md, D2 amendment block.'
);
