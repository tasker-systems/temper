-- The strike substrate (ruled 2026-09-06 with Pete — the delete-act design; temper ruling
-- research 01a076e8-04ed-7bc0-84e5-e2ce6b3090a8; task 01a076e7-51a9-7f33-acd3-c287525e2291).
-- The shared emptying act behind BOTH forms — the ordinary delete and erasure. Full design:
-- temper-artifacts/specs/2026-09-06-delete-act-design.md.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * `content_type IS NULL` is the EMPTIED marker (D5.2: pathname/type/bytes nulled; hash,
--     home, owner kept) — a marker of emptiness, never of which act emptied. Any second
--     row-shape marker of the emptying act is a violation; the ledger alone tells the acts
--     apart, and attribution (owner_profile_id) breaks only at erasure, by the pseudonym
--     break — never by nulling.
--   * Uniqueness binds LIVE rows only: a struck row vacates its (home, hash) slot, so a
--     re-commit of identical bytes mints a fresh row and is never refused by, nor
--     deduplicated against, the struck one (delete-replay-reproduces-absence).
--   * The read floors carry the currency predicate — a struck blob renders the SAME absence
--     an unknown id gets, through EVERY read path (the N3 widening both
--     blob_readable_by_profile's own COMMENT and the readback rows' posture anticipated; the
--     pre-ruling "renders honestly rather than being hidden" stamp is superseded).
--   * The strike touches NO edge. Folding a relation reads as deliberately ended and would
--     falsify the ledger; a struck blob's edges persist and render absent BECAUSE THE BLOB
--     IS GONE.
--   * `blob_delete` takes the event type as a PARAMETER: the emptying shape and the byte
--     fate are shared, the vocabulary is per-act and one-shot at registration. The erasure
--     arm registers and fires its OWN type through this wrapper; `blob_deleted` is
--     registered HERE so the substrate's own tests can exercise its own path (AMEND of the
--     task body's registration-stays-with-the-acts; the vocabulary was ruled final).
--   * The refcount is SAME-TRANSACTION and LIVE-rows-only — never a pre-count; struck rows
--     never hold bytes hostage. The provider bytes are deleted by the CALLER after the
--     commit, at the returned pathname, when `released`.
--
-- Shape-breaking, per the anchor-envelope precedent (20260823000010): an old binary's
-- `blob_pathname!` / `content_bytes!` overrides hit a NULL they declared impossible on a
-- struck row. This release's widened floors keep the NULL from ever reaching a decode.

ALTER TABLE kb_blobs
    ALTER COLUMN blob_pathname DROP NOT NULL,
    ALTER COLUMN content_bytes  DROP NOT NULL;

-- The identity swap's second half: uniqueness binds LIVE rows only. The old table
-- constraint (20260903000060:56) held the slot for the row's whole life, struck or not —
-- the shape the delete act's re-commit clause needed vacated.
ALTER TABLE kb_blobs DROP CONSTRAINT kb_blobs_home_scope_key;
CREATE UNIQUE INDEX kb_blobs_home_scope_live_key
    ON kb_blobs (home_table, home_id, content_hash)
    WHERE content_type IS NOT NULL;

-- Get-or-create re-targeted at the partial index (AMEND of 20260903000060's projector): a
-- hash conflict within the caller's own LIVE rows returns the existing row; a hash whose
-- only same-home row is EMPTIED no longer conflicts — the insert mints the caller's fresh
-- row (their id, their event, their commit time), never a dedup hit against a struck row.
-- The fallback SELECT arm needs no currency predicate: the conflict target can only match
-- a live row, so reaching the fallback means a live row held the slot.
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
            (p_payload->>'owner_profile_id')::uuid,
            COALESCE((p_payload->>'originator_profile_id')::uuid,
                     (p_payload->>'owner_profile_id')::uuid),
            p_event, p_event, v_occurred)
    ON CONFLICT (home_table, home_id, content_hash) WHERE content_type IS NOT NULL
    DO NOTHING
    RETURNING id INTO v_inserted;

    IF v_inserted IS NULL THEN
        -- Get-or-create WITHIN the caller's own scope (live rows — the partial index
        -- cannot conflict on an emptied one, which is the N3 widening).
        SELECT id INTO v_inserted FROM kb_blobs
         WHERE home_table = v_home_table
           AND home_id    = v_home_id
           AND content_hash = v_hash
           AND content_type IS NOT NULL;
    END IF;

    RETURN ARRAY[v_inserted];
END;
$$;

COMMENT ON FUNCTION _project_blob_committed(uuid, jsonb) IS
'blob_committed projector (third definition — 20260903000020 minted the global-hash shape,
20260903000060 scoped it, 20260906000010 re-targeted it at the live-rows-only partial unique
index): get-or-create scoped to the payload''s own home over LIVE rows; a struck row''s slot
is vacated, so a re-commit of identical bytes mints a fresh row and is never deduplicated
against the struck one (delete-replay-reproduces-absence).';

-- The scalar read floor widens (AMEND — the N3 widening the function's own COMMENT declared:
-- "The erasure build (unbuilt) must widen this and its dedup/read siblings"): the currency
-- predicate rides the D5.2 emptied marker. Authorization stays pure home-anchor equality;
-- emptiness is a CONTENT predicate — a struck blob is not unreadable-to-someone, it is
-- ABSENT, the same absence an unknown id gets, for everyone.
CREATE OR REPLACE FUNCTION blob_readable_by_profile(p_profile uuid, p_blob uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_blobs b
        WHERE b.id = p_blob
          AND b.content_type IS NOT NULL
          AND anchor_readable_by_profile(p_profile, b.home_table, b.home_id)
    );
$$;

COMMENT ON FUNCTION blob_readable_by_profile(uuid, uuid) IS
'authorization predicate: a blob is readable iff its OWN home is AND the row is live — the
currency half is the D5.2 emptied marker (20260906000010, the N3 widening): a struck blob
reads as absent through every read path, as if never committed, for every caller. Never a
marker of which act emptied the row — delete and erasure produce the identical shape, and
the ledger alone tells them apart. Harness-enrolled as a DUALITY witness against the anchor
predicate: any edit must keep the anchor-equality answer for live rows.';

-- The set-based mirror widens branch-for-branch (the equivalence-oracle requirement the
-- 20260903000060 header carries verbatim): readable_blobs gains the same currency
-- predicate, so an edge whose blob endpoint is struck falls out of the endpoint arms and
-- the edge renders absent — because the blob is gone, NOT because the relation ended
-- (the strike folds no edges; the edge rows persist untouched).
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
    readable_blobs AS (
        SELECT b.id AS id
        FROM kb_blobs b
        WHERE b.content_type IS NOT NULL
          AND ( (b.home_table = 'kb_contexts'
                   AND b.home_id IN (SELECT id FROM readable_contexts))
             OR (b.home_table = 'kb_cogmaps'
                   AND b.home_id IN (SELECT id FROM readable_cogmaps)) )
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
20260712000010; 20260903000050 added the blob arms; 20260903000060 re-anchored
readable_blobs on the row''s home columns; 20260906000010 widens it with the D5.2 currency
predicate, branch-for-branch with blob_readable_by_profile): an edge is visible iff it is
live, its home anchor is readable, and BOTH endpoints are readable — a struck blob''s edges
persist in the table and render absent because the blob is gone, never folded.';

-- The shared emptying projector: fire and replay both land here. The guard
-- (`content_type IS NOT NULL`) makes re-application a no-op — idempotent under replay, and
-- a second strike event of one row would empty nothing. The strike performs NO edit: no
-- kb_edges row is touched, `owner_profile_id` survives (attribution breaks only at
-- erasure, by the pseudonym break — never by nulling), and the hash stays (the erasure
-- arm's join key).
CREATE FUNCTION _project_blob_deleted(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_id uuid := (p_payload->>'blob_id')::uuid;
BEGIN
    UPDATE kb_blobs
       SET blob_pathname = NULL,
           content_type  = NULL,
           content_bytes = NULL,
           last_event_id = p_event
     WHERE id = v_id
       AND content_type IS NOT NULL;
    RETURN ARRAY[v_id];
END;
$$;

COMMENT ON FUNCTION _project_blob_deleted(uuid, jsonb) IS
'blob_deleted / strike projector (the SHARED emptying shape — both acts land here): the row
empties into the D5.2 shape (pathname/type/bytes nulled; hash, home, owner kept) and the
guard makes re-application a no-op, so replay is idempotent and a double strike empties
nothing. No edge is touched: relations render absent because the blob is gone.';

-- The strike wrapper: act-parameterized (p_event_type) so the delete act and the erasure
-- arm append THEIR OWN event types through this one path — the emptying shape and the byte
-- fate are shared, the vocabulary never is. One transaction appends the event, empties the
-- row, and decides the byte fate; the provider bytes are deleted by the CALLER after the
-- commit (a provider call cannot join the transaction), at the returned pathname, when
-- `released`.
CREATE FUNCTION blob_delete(p_event_type text, p_payload jsonb, p_emitter uuid,
                            p_metadata jsonb DEFAULT '{}'::jsonb,
                            p_invocation uuid DEFAULT NULL::uuid,
                            p_correlation uuid DEFAULT NULL::uuid)
RETURNS TABLE(blob_id uuid, released boolean, pathname text)
LANGUAGE plpgsql AS $$
DECLARE
    v_id   uuid := (p_payload->>'blob_id')::uuid;
    v_hash text;
    v_path text;
    v_home_table text;
    v_home_id    uuid;
    v_live  integer;
    v_ev    uuid;
BEGIN
    IF v_id IS NULL THEN
        RAISE EXCEPTION 'blob_delete: the payload carries no blob_id — a strike addresses '
                        'the row it empties';
    END IF;

    -- Lock the row: concurrent strikes serialize here, the second seeing the first's
    -- emptied shape and refusing — the ledger never carries two emptying events for one
    -- row, and the byte fate is decided on a row that cannot move under it.
    SELECT content_hash, blob_pathname, home_table, home_id
      INTO v_hash, v_path, v_home_table, v_home_id
      FROM kb_blobs WHERE id = v_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'blob_delete: blob % not found', v_id;
    END IF;
    IF v_path IS NULL THEN
        RAISE EXCEPTION 'blob_delete: blob % is already struck — the ledger carries its '
                        'emptying', v_id;
    END IF;

    -- The byte fate, from the SAME transaction's live-row refcount (never a pre-count;
    -- LIVE rows only — the D5.2 marker excludes struck rows, which never hold bytes
    -- hostage). This row is live and locked, so released ⟺ it is the only live row
    -- carrying the hash: N homes over one provider object means N-1 live neighbors keep
    -- the bytes when any one of them strikes.
    SELECT count(*) INTO v_live
      FROM kb_blobs
     WHERE content_hash = v_hash
       AND content_type IS NOT NULL;

    v_ev := _event_append(p_event_type, p_emitter, v_home_table, v_home_id, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    PERFORM _project_blob_deleted(v_ev, p_payload);

    RETURN QUERY SELECT v_id, (v_live <= 1), v_path;
END;
$$;

COMMENT ON FUNCTION blob_delete(text, jsonb, uuid, jsonb, uuid, uuid) IS
'strike wrapper for blobs: the shared emptying act, parameterized by the calling act''s
event type (the vocabulary is per-act and one-shot at registration; the emptying shape and
the byte fate are shared). Refuses in its own voice: an absent blob, an already-struck
blob. The refcount is same-transaction, live-rows-only; released ⟺ the struck row was the
last live row carrying its hash. Provider bytes are deleted by the caller AFTER the commit.
The strike performs no edits: it folds no edges.';

-- TYPED registration, identity-only — the resource_deleted shape. The literal is the
-- committed schemars snapshot (blob_deleted.v1.schema.json), byte for byte: repo ==
-- registry == Rust types. See the header note for why registration lands here rather than
-- with the delete-act build.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('blob_deleted', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BlobDeleted",
  "description": "Strike one blob — the shared emptying act behind BOTH forms, the ordinary delete and\nerasure (ruled 2026-09-06: one byte-fate, one emptied-row shape; the ledger alone tells\nthem apart). Identity-only, the `ResourceDeleted` shape: the event's envelope carries the\nhome (producing anchor), the actor (`emitter_entity_id`), and the time (`occurred_at`);\ncustody is derivable by replay from the persisted edges and the peers' homes, so it is\nnever stamped. `kb_events.\"references\"` is erasure's apparatus and is never populated here.\n\nThe projection empties the row into the D5.2 shape — pathname/type/bytes nulled, the hash\nand home and owner kept — so the row shape carries no marker of WHICH act emptied it, and\na re-commit of identical bytes into the same home mints a fresh row (the per-home UNIQUE\nis partial on live rows). Attribution never breaks at a delete: `owner_profile_id`\nsurvives; it dies only at erasure, by the pseudonym break, never by nulling.",
  "type": "object",
  "properties": {
    "blob_id": {
      "$ref": "#/$defs/BlobId"
    }
  },
  "required": [
    "blob_id"
  ],
  "$defs": {
    "BlobId": {
      "description": "A `kb_blobs.id` value — one immutable, content-addressed binary blob, homed like a\nresource and related to resources by edges (spec: binary blobs, 2026-09-01).",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

SELECT declare_migration(
    20260906000010,
    'shape-breaking',
    'The strike substrate (the delete-act design, ruled 2026-09-06): the shared emptying act behind the ordinary delete and erasure. D5.2 nullability completed (blob_pathname/content_bytes join content_type as nullable — the emptied marker is content_type IS NULL, a marker of emptiness, never of which act); the per-home UNIQUE made PARTIAL on live rows (a struck row vacates its slot, so a re-commit of identical bytes mints a fresh row — never refused by, nor deduplicated against, the struck one); the read floors widened with the N3 currency predicate (blob_readable_by_profile, edges_visible_to''s readable_blobs — a struck blob renders the same absence an unknown id gets, through every read path; the pre-ruling renders-honestly stamp superseded by the ruled renders-absent-everywhere posture); the shared _project_blob_deleted projector (idempotent under replay; touches no edge) and the act-parameterized blob_delete wrapper (row lock, own-voice refusals, the SAME-TRANSACTION live-row refcount deciding the byte fate — never a pre-count; released ⟺ last live row carrying the hash); the typed blob_deleted event registered here (AMEND of the task body''s registration-stays-with-the-acts: the substrate''s own tests must be able to exercise its own path, and the vocabulary was ruled final — act-neutrality is preserved in the load-bearing place, the wrapper''s p_event_type parameter, so the erasure arm registers and fires its own type). Shape-breaking per the anchor-envelope precedent: an old binary''s blob_pathname!/content_bytes! overrides hit a NULL they declared impossible on a struck row; this release''s widened floors keep the NULL from ever reaching a decode. Design: temper-artifacts/specs/2026-09-06-delete-act-design.md.'
);
