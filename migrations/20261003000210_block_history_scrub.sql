-- Resource erasure build order 2e: the block history scrub (resource erasure spec 2026-09-28,
-- D11; D10 for the one computation, D13 for the lock, D5 for the refusals).
--
-- The scrub empties the history of named blocks of a LIVE resource and keeps their present: a
-- live block keeps its current revision and its current chunks, a folded block empties entirely.
-- It is the erasure's redaction body narrowed, never a second body.
--
-- Section 0. `_resource_erasure_apply_redaction` (same signature) narrows its steps (1)–(3) by
--            the scope parameter and stops after step (3) for a block set.
-- Section 1. `block_history_scrub_plan`: THE one computation (D10). The survey renders it and the
--            act consumes it.
-- Section 2. `block_history_scrub_survey`: the plan, rendered.
-- Section 3. `_block_history_scrub_apply`: the scrub's effect from its event, event-free. The act
--            and the replay arm both call it.
-- Section 4. `block_history_scrub_execute`: the act. Refusals RAISE; the service records them.
-- Section 5. `resource_erasure_refuse` gains the optional `act` and `blocks` (DROP + CREATE; the
--            six-argument call still resolves).
-- Section 6. The `block_history_scrubbed` and `resource_erasure_refused` payload_schemas are
--            re-registered with the optional properties Task 1 added. Each dollar-quoted literal
--            equals, as JSON, its committed fixture (`block_history_scrubbed.v1.schema.json`, then
--            `resource_erasure_refused.v1.schema.json`);
--            `payload_schema::the_migration_literal_matches_the_committed_fixture` pins both, in
--            this order.
-- Section 7. `resource_erasure_execute` (same signature) names the ingest it ended only when the
--            ingest was `in_progress`: a cancelled or abandoned ingest ended before the erasure.

-- ---------------------------------------------------------------------------
-- Section 0. The one redaction body (D2), given a scope. Event-free by design:
-- the erasure's and the scrub's replay arms run it beside the walk; only
-- `resource_erasure_execute` and `block_history_scrub_execute` append events
-- around it.
--
-- The scope is the third parameter, `p_blocks uuid[] DEFAULT NULL`:
--   * NULL is the whole resource: steps (1)–(9), every block, revision and
--     chunk of R, live, folded and superseded.
--   * A non-NULL array is the block history scrub (D11): steps (1)–(3) only,
--     narrowed to the named blocks of R and, on each, to what is not its
--     present. A live block keeps `current_revision_id` and its current chunks;
--     a folded block has no present and empties entirely.
-- There is exactly one function: an overload beside a DEFAULTed parameter would
-- make every two-argument call ambiguous. p_resource keys every join; p_event
-- supplies occurred_at for the replay-stable stamps.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION _resource_erasure_apply_redaction(p_resource uuid, p_event uuid, p_blocks uuid[] DEFAULT NULL)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_occurred      timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_orig_blocks   uuid[];
    v_orig_sources  uuid[];
    v_orig_ns       integer[];
    v_i             integer;
    v_sentinel      uuid;
    v_source        uuid;
    v_parked        jsonb;
BEGIN
    -- ── The scope (D11). Each of steps (1)–(3) carries one predicate,
    --    `p_blocks IS NULL OR (<the row's block> = ANY(p_blocks) AND (<block is folded> OR <row
    --    is not current>))`, so the whole-resource form reaches what it always reached and the
    --    block-set form reaches only the named blocks' history. Keep-current lives in the WHERE
    --    clause, never in a SET value: every assignment stays an erasure constant, which is what
    --    the 2d fence (resource_erasure_surface_test) binds. One statement per column. ──────────

    -- ── (1) Chunk prose emptied BY ROW JOIN, hash kept (D2 step 1, and the joint-read fix:
    --     header_path rides the same chunks — authored heading prose, scanned by the sweep).
    --     The whole resource: current AND superseded. A block set: a live block's non-current
    --     chunks, and every chunk of a folded block. The CAS retention rule never fires:
    --     fold/supersede affect visibility, not existence, and emptied rows stay rows. ─────────
    UPDATE kb_chunk_content cc
       SET content = ''
      FROM kb_chunks c
      JOIN kb_content_blocks b ON b.id = c.block_id
     WHERE cc.chunk_id = c.id
       AND c.resource_id = p_resource
       AND cc.content <> ''
       AND (p_blocks IS NULL
            OR (c.block_id = ANY(p_blocks) AND (b.is_folded OR NOT c.is_current)));

    UPDATE kb_chunks c
       SET header_path = NULL
      FROM kb_content_blocks b
     WHERE b.id = c.block_id
       AND c.resource_id = p_resource
       AND c.header_path IS NOT NULL
       AND (p_blocks IS NULL
            OR (c.block_id = ANY(p_blocks) AND (b.is_folded OR NOT c.is_current)));

    -- ── (2) Verbatim block bytes (D2 step 2, F1: kb_block_revisions → kb_content_blocks.resource_id
    --     reaches them all with NO hash in the join). The whole resource: every revision of every
    --     block, live and folded. A block set: every revision of a live block except its
    --     current_revision_id, and every revision of a folded block. ──────────────────────────
    UPDATE kb_block_content bc
       SET content = ''
     WHERE bc.block_revision_id IN (
           SELECT br.id FROM kb_block_revisions br
             JOIN kb_content_blocks b ON b.id = br.block_id
            WHERE b.resource_id = p_resource
              AND (p_blocks IS NULL
                   OR (b.id = ANY(p_blocks)
                       AND (b.is_folded OR br.id IS DISTINCT FROM b.current_revision_id))))
       AND bc.content <> '';

    -- ── (3) Embeddings and provenance nulled TOGETHER — `embedding IS NULL` and `embedded_with IS
    --     NULL` can never disagree (20260713000040:84-87), the coherence rule the principal act's
    --     step (4) carries (D2 step 3). Nothing re-embeds afterwards. For the whole resource: the
    --     embed drain's write-backs carry the D13 write guard (Section W of 20260929040730), and
    --     the act holds R's row lock, so a drain write either landed before the act and is nulled
    --     here, or is refused after it. For a block set: the drain reads its candidates (current
    --     chunks of live blocks) long before it writes their vectors back, and a chunk it read can
    --     be superseded, or its block folded, in between; this statement then empties it. The
    --     drain's write-backs (temper-substrate `CHUNK_EMBEDDING_WRITE_BACK` and
    --     `stamp_blank_chunks`) re-check at write time that the target chunk is current
    --     and its block not folded, so a write-back that arrives after the scrub writes nothing,
    --     and one that landed before it is nulled here. ────────────────────────────────────────
    UPDATE kb_chunks c
       SET embedding = NULL,
           embedded_with = NULL
      FROM kb_content_blocks b
     WHERE b.id = c.block_id
       AND c.resource_id = p_resource
       AND c.embedding IS NOT NULL
       AND (p_blocks IS NULL
            OR (c.block_id = ANY(p_blocks) AND (b.is_folded OR NOT c.is_current)));

    -- ── A block set stops here. Steps (4)–(9) (the search vector, artifacts, citation audits,
    --    watermarks, workflow jobs, the ingestion record and verdicts, and the husk's sentinels)
    --    are the erasure's alone: D11 is "D2 steps 1–3, narrowed to the block", and the scrubbed
    --    resource stays live with its present untouched. ─────────────────────────────────────
    IF p_blocks IS NOT NULL THEN
        RETURN;
    END IF;

    -- ── (4) Search vector emptied: the vector folds title+body+meta, so a partial redaction would
    --     leave redacted terms searchable (D2 step 4). ────────────────────────────────────────
    UPDATE kb_resource_search_index si
       SET search_vector = ''
      WHERE si.resource_id = p_resource
        AND si.search_vector <> '';

    -- ── (5) Data artifacts: EVERY artifact of the resource — every kind owner (kb_profiles,
    --     kb_teams), every intent (current, member, pinned), folded/superseded included. Ruled
    --     2026-09-28: artifacts are bound to their resource and do not exist independently of it;
    --     that is the cost of security. Content '{}'::jsonb — the column is JSONB NOT NULL
    --     (20260820000020:48); the principal act's own value and the sweep's closure signal. Hashes
    --     and ids kept; artifact SHAPES are homed in a context or cogmap, not in the resource, and
    --     are untouched. NO hash enters kb_erased_content from here, ever (D12 rule 0). ────────
    UPDATE kb_data_artifact_content dac
       SET content = '{}'::jsonb
     WHERE dac.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND dac.content <> '{}'::jsonb;

    -- ── (6) The citation audits PROJECTED from R's blocks' events: the projected column the sweep
    --     scans (20260724000110:29), reached SEPARATELY from the ledger half, which cut 1 does NOT
    --     touch (D2 step 8's joint-read fix; D3 redacts the event's reason at cut 2). Audits by
    --     other findings that CITE R's blocks are listed-only (Q1) and unreached — they are
    --     anchored on other findings' blocks, outside the resource join. ──────────────────────
    UPDATE kb_citation_audits ca
       SET reason = NULL
     WHERE ca.block_id IN (
           SELECT b.id FROM kb_content_blocks b WHERE b.resource_id = p_resource)
       AND ca.reason IS NOT NULL;

    -- ── (7) Formation watermarks nulled (D2 step 6, the principal act's rule UNCHANGED): the
    --     resource's home context and any cogmap holding it as a region member, so the next
    --     materialize recomputes centroids from survivors. Nulling in LEDGER ORDER (the walk arm
    --     re-applies at the end of the act's correlated span within its transaction (D14); the later of the two events' stamps is what replay
    --     leaves — an idempotent no-op either way, since replay also runs this body). ──────────
    UPDATE kb_contexts c
       SET shape_materialized_event_id = NULL
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');
    UPDATE kb_cogmaps m
       SET shape_materialized_event_id = NULL
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource);

    -- ── (8) Workflow jobs scoped to the resource (D2 step 7). Every row, in EVERY status, loses
    --     its payload and last_error: the excerpt is the carrier, not the job's state, so a
    --     `done` or `dead` row keeps its status but not its excerpts. Rows not yet finished
    --     (`pending`, `waiting_for_retry`, `in_progress`) are also cancelled to `dead`;
    --     `in_progress` is reached deliberately: a leased job would otherwise run on against the
    --     husk until lease expiry. Not replay inputs; excerpt carriers on the personal-data
    --     surface. ────────────────────────────────────────────────────────────────────────────
    UPDATE kb_workflow_jobs j
       SET payload    = '{}'::jsonb,
           last_error = NULL
     WHERE j.resource_id = p_resource
       AND (j.payload <> '{}'::jsonb OR j.last_error IS NOT NULL);

    UPDATE kb_workflow_jobs j
       SET status = 'dead'
     WHERE j.resource_id = p_resource
       AND j.status IN ('pending', 'waiting_for_retry', 'in_progress');

    -- ── (8a) The ingestion record and the artifact verdicts (D2 step 7a), both reachable by
    --     foreign key from the resource. `kb_ingestion_records.source_uri` takes the origin_uri
    --     class sentinel 'erased:<resource_id>' (the column is NOT NULL, and it carries the same
    --     shape of text origin_uri does); `source_hash` is kept, like every hash. A verdict's
    --     `detail` carries validator messages that quote instance values and keys, so it is
    --     nulled on every verdict of every artifact of the resource. ─────────────────────────
    UPDATE kb_ingestion_records ir
       SET source_uri = 'erased:' || p_resource::text
     WHERE ir.resource_id = p_resource
       AND ir.source_uri <> 'erased:' || p_resource::text;

    UPDATE kb_data_artifact_verdicts v
       SET detail = NULL
     WHERE v.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND v.detail IS NOT NULL;

    -- ── (9) PROJECTION-SIDE SENTINELS, applied inside the same body (D2 step 9, D4's projection
    --     side). Before cut 2 ships, step (9a) is what keeps replay byte-identical: the walk
    --     projects the original title from resource_created, then the resource_erased arm runs
    --     this body at the end of the act's correlated span within its transaction (D14) and
    --     overwrites it there. After cut 2 ships, the redacted payloads already project these values from genesis and
    --     this step becomes an idempotent no-op. ──────────────────────────────────────────────

    -- (9a) The husk: is_active cleared and erased_at set in the SAME UPDATE — the CHECK
    --      kb_resources_erased_is_inactive (20260929000010) raises on any in-between state, so the
    --      order is a constraint, not policy. erased_at = the event's occurred_at (never now(),
    --      the replay-stable rule, D6). Title and origin_uri: D4 sentinels, projector-reproducible
    --      constants, never operator input. `updated` rides occurred_at like every projector.
    UPDATE kb_resources r
       SET is_active      = false,
           erased_at      = COALESCE(r.erased_at, v_occurred),
           title          = 'erased-' || r.id::text,
           origin_uri     = 'erased:' || r.id::text,
           updated        = v_occurred
     WHERE r.id = p_resource;

    -- (9b) The resource's properties: keys AND values sentineled (Q3, ruled 2026-09-28 — an
    --      operator does not tell a resource apart from its metadata; the husk keeps no metadata
    --      at all; doc_type, tags and facets are inside the property surface and go with it).
    --      Keys map erased-key-<n> by LEDGER ORDER of first appearance (D4): the mapping derives
    --      from ledger order, never from key text, so it reveals nothing; property_unset stays
    --      consistent because the same original key maps to the same n across property_set,
    --      property_asserted and property_unset events, which replay reproduces (the mapping is
    --      a pure function of (owner, key) under the total order of each key's first-asserting
    --      event: its occurred_at, then its id — keys asserted in one transaction share
    --      occurred_at and the event id decides; the key text never does). The numbering is
    --      _resource_erasure_key_numbers (Section 0d), the ONE definition this step and step
    --      (9d) share. EVERY family row — live and folded — is folded by this pass: the husk
    --      keeps NO metadata (Q3), and folding avoids a UNIQUE-index collision the sentinel
    --      values would otherwise raise — uq_kb_properties_active is partial on NOT is_folded
    --      over (owner, key, value); two live rows of ONE key in the facet shape (several live
    --      rows per key is what facet_set IS) would both map to (erased-key-n, "erased") and
    --      violate it. Folding is also what replay reproduces: the act's later, folded rows came
    --      from events that are themselves behind the erasure event in ledger order, so the
    --      arm, run at the end of the act's correlated span within its transaction (D14), sees
    --      the same family state the live act sees.
    --
    --      A key set → unset → re-set maps to one n: the numbering is per original key over the
    --      whole family, live and folded rows alike. last_event_id points at the erasure event
    --      (the property fold rides the act — the trail records which event retired the
    --      property, the 20260727000030 shape).
    WITH ranked AS (
        SELECT k.property_key, k.n
          FROM _resource_erasure_key_numbers('kb_resources', p_resource) k
    )
    UPDATE kb_properties p
       SET property_key   = 'erased-key-' || ranked.n::text,
           property_value = '"erased"'::jsonb,
           is_folded      = true,
           last_event_id  = COALESCE(p_event, p.last_event_id)
      FROM ranked
     WHERE ranked.property_key = p.property_key
       AND p.owner_table = 'kb_resources' AND p.owner_id = p_resource;

    -- (9c) Edge labels: NULL on every edge at either end (D4 — kind and polarity survive; they
    --      are the structure; the "system vocabulary" alternative was rejected in the spec — no
    --      registry exists, and the edge is folded anyway at the act level).
    --      Goal §8: an edge touching R is R's surface whoever authored it — its structure and its
    --      fold event stay, all of its content goes.
    UPDATE kb_edges e
       SET label = NULL
     WHERE (e.source_table = 'kb_resources' AND e.source_id = p_resource)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_resource);

    -- (9d) Edge-owned properties: keys AND values sentineled, the (9b) pass applied per edge
    --      (D2 step 9; D4 "numbered per edge … whoever authored them", goal §8). Every edge with R
    --      at either end — live or already folded — and every kb_edges-owned row of it, whoever
    --      asserted it: keys map erased-key-<n> numbered WITHIN EACH EDGE by the same
    --      ledger-identity order (_resource_erasure_key_numbers), values '"erased"'::jsonb, every
    --      row folded (the same uq_kb_properties_active collision reason as (9b)), last_event_id
    --      the erasure event. It runs after the act's relationship_folded events, whose projector
    --      has already folded the live edges' rows; the fold only folds, so the key and value
    --      text are this pass's to replace.
    UPDATE kb_properties p
       SET property_key   = 'erased-key-' || k.n::text,
           property_value = '"erased"'::jsonb,
           is_folded      = true,
           last_event_id  = COALESCE(p_event, p.last_event_id)
      FROM kb_edges e
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_edges', e.id) k
     WHERE ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
         OR (e.target_table = 'kb_resources' AND e.target_id = p_resource))
       AND p.owner_table = 'kb_edges' AND p.owner_id = e.id
       AND p.property_key = k.property_key;

    -- (9e) The remote-source re-pointing (D4). Every remote provenance row of R's blocks
    --      re-points to the sentinel row replay's redacted incorporated[*].source.value upserts:
    --      'erased:<block_id>:<n>', where n numbers the distinct original remote sources on
    --      that block in ledger order of first appearance. The key is unique per (block,
    --      original source), so two sources cited in one event at one accretion seq never share
    --      a sentinel and the provenance unique key (block_id, source_kind, source_id,
    --      contributed_by_event_id) cannot collide. It carries nothing of the URL.
    --
    --      Capture: R's original remote sources, numbered, read ONCE through
    --      _resource_erasure_remote_originals (Section 0c, the same capture the survey reads)
    --      before anything below changes a provenance row. The captured ids are the whole of
    --      what the delete at the end may consider.
    SELECT coalesce(array_agg(o.block_id  ORDER BY o.block_id, o.n), '{}'),
           coalesce(array_agg(o.source_id ORDER BY o.block_id, o.n), '{}'),
           coalesce(array_agg(o.n         ORDER BY o.block_id, o.n), '{}')
      INTO v_orig_blocks, v_orig_sources, v_orig_ns
      FROM _resource_erasure_remote_originals(p_resource) o;

    --      Upsert, then re-point BY THE ID the upsert returns. _upsert_remote_source deduplicates
    --      on uri_normalized and keeps the first writer's spelling, so the row it returns may be
    --      a look-alike someone minted first (' erased:<block>:1', leading space); matching on
    --      `uri` text would miss it and leave the provenance on the original URL.
    --
    --      The re-point runs in two passes, park then place. The returned sentinel row can
    --      itself be one of the block's originals: a block that cites the literal
    --      'erased:<block>:1' beside a URL numbered 1 gets that literal's row back as the URL's
    --      sentinel, while the literal, numbered 2, moves on to 'erased:<block>:2'. Moving the
    --      URL's row first would duplicate the literal's row on the provenance unique key
    --      (block_id, source_kind, source_id, contributed_by_event_id) before the literal's row
    --      moves away, and a non-deferrable unique check raises on that intermediate state. So
    --      the park pass sets each captured row's source_id to the row's own id, which no other
    --      row holds, and records row id → sentinel id. The place pass then sets every parked row
    --      to its sentinel in one statement. The final state cannot collide: each original on a
    --      block has its own n, distinct n give distinct uri_normalized and so distinct sentinel
    --      rows, and an event contributes at most one row per original per block. It is the
    --      state replay of the redacted payloads lands on, one row per original per event,
    --      each on its own sentinel.
    v_parked := '{}'::jsonb;
    FOR v_i IN 1 .. coalesce(array_length(v_orig_blocks, 1), 0) LOOP
        v_sentinel := _upsert_remote_source(
            'erased:' || v_orig_blocks[v_i]::text || ':' || v_orig_ns[v_i]::text);
        WITH parked AS (
            UPDATE kb_block_provenance bp
               SET source_id = bp.id
             WHERE bp.block_id = v_orig_blocks[v_i]
               AND bp.source_kind = 'remote'
               AND bp.source_id = v_orig_sources[v_i]
            RETURNING bp.id)
        SELECT v_parked || coalesce(jsonb_object_agg(parked.id::text, v_sentinel), '{}'::jsonb)
          INTO v_parked
          FROM parked;
    END LOOP;

    UPDATE kb_block_provenance bp
       SET source_id = (v_parked ->> bp.id::text)::uuid
     WHERE bp.block_id = ANY(v_orig_blocks)
       AND v_parked ? bp.id::text;

    --      Delete, scoped and locked. A captured original that nothing cites any more is deleted:
    --      replay of the redacted payloads never mints it. One that another resource's block
    --      still cites stays, and the survey names it by id (D8). Only the captured originals
    --      are considered — the act never deletes a remote source it did not orphan. Each one
    --      is locked FOR UPDATE in its own statement, and "does anything still cite it?" is
    --      asked in a SEPARATE, later statement. resource_erasure_execute has already locked
    --      every captured original before the plan ran, so inside the act this lock is a
    --      re-lock the transaction already holds; the lock-then-check shape is what keeps the
    --      body correct on its own. Under READ COMMITTED each statement of this VOLATILE
    --      function reads a fresh snapshot, and a concurrent citer's _upsert_remote_source holds
    --      the row's lock (ON CONFLICT DO UPDATE) until it commits. So the lock waits for that
    --      citer, and the existence check that follows sees its committed provenance row and
    --      keeps the source. A single `DELETE … WHERE NOT EXISTS (…)` evaluates its subquery
    --      against the statement's own snapshot, taken before the lock wait, and would delete a
    --      row a citer committed during that wait. A citer that arrives after the lock waits on
    --      it; if the row is deleted, its upsert inserts the URL as a fresh row. Ids are locked
    --      in uuid order, so two acts whose resources share originals take those locks in one
    --      order and cannot deadlock on them.
    FOR v_source IN
        SELECT DISTINCT s.id FROM unnest(v_orig_sources) AS s(id) ORDER BY s.id
    LOOP
        PERFORM 1 FROM kb_remote_sources r WHERE r.id = v_source FOR UPDATE;
        IF NOT EXISTS (SELECT 1 FROM kb_block_provenance q
                        WHERE q.source_kind = 'remote' AND q.source_id = v_source) THEN
            DELETE FROM kb_remote_sources r WHERE r.id = v_source;
        END IF;
    END LOOP;

    RETURN;
END;
$$;

COMMENT ON FUNCTION _resource_erasure_apply_redaction(uuid, uuid, uuid[]) IS
'THE ONE row-anchored redaction body, for resource erasure (spec 2026-09-28 D2, D4) and for the
block history scrub (D11). Its scope parameter p_blocks has two forms. NULL is the whole resource:
chunk prose, header_path, block revision bytes, embeddings+embedded_with, search vector, data
artifact content ({}::jsonb, EVERY artifact of the resource whatever its kind owner, intent or
supersession — ruled 2026-09-28), citation-audit projected reasons, formation watermark nulls,
workflow-job payloads and last_errors in every status (unfinished rows cancelled to dead), the
ingestion record''s source_uri (erased:<id>, hash kept) and artifact verdict details (NULL), and
the projection-side sentinels (husk title/origin_uri, property keys erased-key-<n> by ledger order
of first appearance — the resource''s, and each touching edge''s numbered per edge whoever authored
them — values ''erased''::jsonb, edge labels NULL, remote-source re-pointing to the sentinel rows
replay mints). A non-NULL block set runs the first three steps only (chunk prose and header_path,
block revision bytes, embeddings+embedded_with), narrowed to the named blocks of the resource with
keep-current: a live block keeps its current_revision_id and its current chunks, a folded block
empties entirely; the resource stays live. Keep-current is a WHERE predicate, never a SET value.
ROW-ANCHORED: every join is on resource id, never a content hash — another resource''s
byte-identical content is NEVER reached (the custody-never-bytes ruling), and no hash enters
kb_erased_content from this body. Event-free: the erasure''s replay arm (replay.rs
ResourceErased) calls it at the end of the act''s correlated span within its transaction (D14),
and _block_history_scrub_apply calls it for the scrub; only resource_erasure_execute and
block_history_scrub_execute append events around it.';

-- ---------------------------------------------------------------------------
-- Section 1. THE scrub plan (D10): the one computation the survey renders and
-- the act consumes. Read-only.
--
-- Per named block of R, in the order p_blocks names them: `block`, `folded`,
-- `revisions_to_empty` and `chunks_to_empty`. At the top level: `resource`,
-- `ingest_state`, and `cancels_ingest` (the act cancels an in-flight ingest,
-- ruling 3 of 2e). A named id that is not a block of R is not listed; the act
-- refuses it before it reads the plan.
--
-- The counts restate the redaction body's step (1)–(3) predicates for a block
-- set: a row is counted exactly when one of those statements would change it
-- (non-empty revision bytes; a chunk with non-empty prose, a header_path or an
-- embedding). That is a second spelling of the same predicates, and it can
-- drift from the body. The survey-equals-act witness
-- (block_history_scrub.rs) pins the two together against the rows the act
-- actually empties.
-- ---------------------------------------------------------------------------
CREATE FUNCTION block_history_scrub_plan(p_resource uuid, p_blocks uuid[])
RETURNS jsonb LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object(
        'resource',       r.id,
        'ingest_state',   r.ingest_state,
        'cancels_ingest', r.ingest_state = 'in_progress',
        'blocks', coalesce((
            SELECT jsonb_agg(jsonb_build_object(
                       'block',  b.id,
                       'folded', b.is_folded,
                       'revisions_to_empty', (
                           SELECT count(*)
                             FROM kb_block_revisions br
                             JOIN kb_block_content bc ON bc.block_revision_id = br.id
                            WHERE br.block_id = b.id
                              AND bc.content <> ''
                              AND (b.is_folded OR br.id IS DISTINCT FROM b.current_revision_id)),
                       'chunks_to_empty', (
                           SELECT count(*)
                             FROM kb_chunks c
                            WHERE c.block_id = b.id
                              AND c.resource_id = p_resource
                              AND (b.is_folded OR NOT c.is_current)
                              AND (c.header_path IS NOT NULL
                                   OR c.embedding IS NOT NULL
                                   OR EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               WHERE cc.chunk_id = c.id AND cc.content <> ''))))
                   ORDER BY u.n)
              FROM (SELECT DISTINCT ON (x.id) x.id, x.n
                      FROM unnest(p_blocks) WITH ORDINALITY AS x(id, n)
                     ORDER BY x.id, x.n) u
              JOIN kb_content_blocks b ON b.id = u.id AND b.resource_id = p_resource),
            '[]'::jsonb))
      FROM kb_resources r
     WHERE r.id = p_resource;
$$;

COMMENT ON FUNCTION block_history_scrub_plan(uuid, uuid[]) IS
'THE block history scrub plan (resource erasure spec D10, D11): per named block of the resource,
in p_blocks order, whether it is folded and how many revisions and chunks the scrub would empty;
the resource''s ingest_state and whether the scrub cancels it (in_progress only). Read-only. The
counts restate the redaction body''s narrowed step (1)–(3) predicates; the survey-equals-act
witness pins them to the rows the act empties. NULL when the resource does not exist.';

-- ---------------------------------------------------------------------------
-- Section 2. The survey door's render: the plan and nothing else. A survey
-- records nothing and is not a scrub request.
-- ---------------------------------------------------------------------------
CREATE FUNCTION block_history_scrub_survey(p_resource uuid, p_blocks uuid[])
RETURNS jsonb LANGUAGE plpgsql AS $$
BEGIN
    RETURN block_history_scrub_plan(p_resource, p_blocks);
END;
$$;

COMMENT ON FUNCTION block_history_scrub_survey(uuid, uuid[]) IS
'the block history scrub survey''s answer: the ONE plan (block_history_scrub_plan), rendered.
Read-only; appends nothing. It reports counts only: the warning that a current revision still
carries a finding''s hash comes from the sensitivity sweep''s stored findings (build order 3c).';

-- ---------------------------------------------------------------------------
-- Section 3. The scrub's effect, from its event. Event-free: the act calls it
-- right after appending `block_history_scrubbed`, and the replay arm calls it at
-- the event's ledger position. Everything it does is read from the payload:
-- `subject_ids` names the blocks (and through them the one resource), and
-- `cancelled_ingest` decides the ingest cancel (ruling 6 of 2e: the field, not
-- the `targets` line, drives replay).
-- ---------------------------------------------------------------------------
CREATE FUNCTION _block_history_scrub_apply(p_event uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_type      text;
    v_payload   jsonb;
    v_blocks    uuid[];
    v_found     bigint;
    v_resources bigint;
    v_resource  uuid;
BEGIN
    SELECT t.name, e.payload INTO v_type, v_payload
      FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE e.id = p_event;
    IF v_type IS DISTINCT FROM 'block_history_scrubbed' THEN
        RAISE EXCEPTION '_block_history_scrub_apply: event % is not a block_history_scrubbed event',
                        p_event;
    END IF;
    IF v_payload->>'subject_table' IS DISTINCT FROM 'kb_content_blocks' THEN
        RAISE EXCEPTION '_block_history_scrub_apply: event % names subject_table %, not kb_content_blocks',
                        p_event, v_payload->>'subject_table';
    END IF;

    SELECT coalesce(array_agg(x.id::uuid ORDER BY x.n), '{}')
      INTO v_blocks
      FROM jsonb_array_elements_text(v_payload->'subject_ids') WITH ORDINALITY AS x(id, n);

    SELECT count(*), count(DISTINCT b.resource_id), (array_agg(DISTINCT b.resource_id))[1]
      INTO v_found, v_resources, v_resource
      FROM kb_content_blocks b
     WHERE b.id = ANY(v_blocks);
    IF cardinality(v_blocks) = 0 OR v_found <> cardinality(v_blocks) OR v_resources <> 1 THEN
        RAISE EXCEPTION '_block_history_scrub_apply: event % does not name blocks of exactly one resource',
                        p_event;
    END IF;

    -- The cancel is set only from `in_progress`: a resource whose ingest already ended keeps
    -- the state it has.
    IF (v_payload->>'cancelled_ingest')::boolean IS TRUE THEN
        UPDATE kb_resources
           SET ingest_state = 'cancelled'
         WHERE id = v_resource
           AND ingest_state = 'in_progress';
    END IF;

    PERFORM _resource_erasure_apply_redaction(v_resource, p_event, v_blocks);
END;
$$;

COMMENT ON FUNCTION _block_history_scrub_apply(uuid) IS
'the block history scrub''s effect (resource erasure spec D11), read from its block_history_scrubbed
event alone: when cancelled_ingest is true, ingest_state in_progress becomes cancelled; then the
one redaction body runs with the event''s subject_ids as its block set. Event-free; the act and the
replay arm both call it, so the live act and replay apply one definition.';

-- ---------------------------------------------------------------------------
-- Section 4. THE ACT (D11): refuses or completes, all one transaction. Emits ONE
-- `block_history_scrubbed` admin event (NULL-anchored, the request reference
-- on `references` and as the correlation id) and applies it through
-- `_block_history_scrub_apply`.
--
-- Legality does NOT live here: is_system_admin is the Rust caller's gate. A
-- refusal RAISES with a typed message; the caller records it through
-- `resource_erasure_refuse(…, p_act => 'block_history_scrub', p_blocks => …)`.
--
-- Two states are not refusals. An in-flight ingest is cancelled by the scrub
-- and recorded (rulings 3 and 6 of 2e). A tombstone is scrubbable: in D11
-- "live" means not erased (ruling 8 of 2e).
-- ---------------------------------------------------------------------------
CREATE FUNCTION block_history_scrub_execute(
    p_resource    uuid,
    p_blocks      uuid[],
    p_operator    uuid,
    p_emitter     uuid,
    p_request_ref uuid
) RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_found     boolean;
    v_charter   uuid;
    v_erased_ts timestamptz;
    v_foreign   uuid;
    v_repeated  uuid;
    v_plan      jsonb;
    v_targets   jsonb;
    v_payload   jsonb;
    v_ev        uuid;
BEGIN
    IF p_resource IS NULL THEN
        RAISE EXCEPTION 'block_history_scrub_execute: p_resource is required';
    END IF;
    -- The request reference is the act's correlation id; without one, _event_append correlates
    -- the event to itself and the scrub pairs with nothing.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'block_history_scrub_execute: p_request_ref is required';
    END IF;

    -- ── The verdicts, read under R's row lock (D13). FOR UPDATE waits out every writer already
    --    holding the row (their FK KEY SHARE, the write guard's KEY SHARE, the segmented-ingest
    --    refusals' KEY SHARE), so the plan and the body see one settled state, and an append or
    --    finalize arriving after it waits, then reads the cancelled state and refuses (TF004,
    --    20261003000110). The same lock serializes a concurrent erasure: whichever commits
    --    second sees the first. ──────────────────────────────────────────────────────────────
    SELECT count(*) > 0 INTO v_found FROM kb_resources r WHERE r.id = p_resource;
    IF NOT v_found THEN
        RAISE EXCEPTION 'block_history_scrub_execute: resource % not found', p_resource;
    END IF;
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR UPDATE;
    SELECT c.telos_resource_id INTO v_charter FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.erased_at INTO v_erased_ts FROM kb_resources r WHERE r.id = p_resource;

    IF v_charter IS NOT NULL THEN
        RAISE EXCEPTION 'block_history_scrub_execute: charter resource (map-grain erasure is filed task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)';
    END IF;
    IF v_erased_ts IS NOT NULL THEN
        RAISE EXCEPTION 'block_history_scrub_execute: already erased';
    END IF;

    -- ── The named blocks. An empty list scrubs nothing and is refused. Every id must be a block
    --    of R; the first one that is not is named. A repeated id is refused rather than folded
    --    into one: the service refuses that list, and the SQL never completes on a list the
    --    service would refuse. ───────────────────────────────────────────────────────────────
    IF p_blocks IS NULL OR cardinality(p_blocks) = 0 THEN
        RAISE EXCEPTION 'block_history_scrub_execute: p_blocks is empty';
    END IF;
    SELECT x.id INTO v_foreign
      FROM unnest(p_blocks) WITH ORDINALITY AS x(id, n)
     WHERE NOT EXISTS (SELECT 1 FROM kb_content_blocks b
                        WHERE b.id = x.id AND b.resource_id = p_resource)
     ORDER BY x.n
     LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'block_history_scrub_execute: block % is not a block of resource %',
                        v_foreign, p_resource;
    END IF;
    SELECT x.id INTO v_repeated
      FROM unnest(p_blocks) WITH ORDINALITY AS x(id, n)
     GROUP BY x.id
    HAVING count(*) > 1
     ORDER BY min(x.n)
     LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'block_history_scrub_execute: block % is named more than once', v_repeated;
    END IF;

    -- ── The ONE computation (D10). The record's per-block lines are the plan's counts. ───────
    v_plan := block_history_scrub_plan(p_resource, p_blocks);

    SELECT coalesce(jsonb_agg(jsonb_build_object(
               'target',  'kb_content_blocks',
               'outcome', 'block ' || (blk->>'block')
                          || CASE WHEN (blk->>'folded')::boolean THEN ' folded: ' ELSE ' live: ' END
                          || (blk->>'revisions_to_empty') || ' revisions emptied, '
                          || (blk->>'chunks_to_empty') || ' chunks emptied')
               ORDER BY t.ord), '[]'::jsonb)
      INTO v_targets
      FROM jsonb_array_elements(v_plan->'blocks') WITH ORDINALITY AS t(blk, ord);

    -- ── An in-flight ingest is cancelled, not refused (rulings 3 and 6 of 2e). It is recorded
    --    twice: `cancelled_ingest` is what replay reads, and the `targets` line is the record in
    --    the erasure act's shape. The key is written only when true. ────────────────────────
    v_payload := jsonb_build_object(
        'subject_table', 'kb_content_blocks',
        'subject_ids',   to_jsonb(p_blocks),
        'actor',         p_operator,
        'targets',       v_targets);
    IF (v_plan->>'cancels_ingest')::boolean THEN
        v_payload := v_payload
            || jsonb_build_object(
                   'targets', v_targets || jsonb_build_array(jsonb_build_object(
                       'target',  'kb_resources.ingest_state',
                       'outcome', 'ingest in_progress; cancelled by block history scrub')),
                   'cancelled_ingest', true);
    END IF;

    v_ev := _event_append(
        'block_history_scrubbed', p_emitter, NULL, NULL,
        v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    PERFORM _block_history_scrub_apply(v_ev);

    RETURN jsonb_build_object(
        'event_id', v_ev,
        'targets',  v_payload->'targets');
END;
$$;

COMMENT ON FUNCTION block_history_scrub_execute(uuid, uuid[], uuid, uuid, uuid) IS
'the block history scrub (resource erasure spec D11): empties every revision but the current one
and every non-current chunk of each named live block, and every revision and chunk of each named
folded block, on a resource that is not erased. Under R''s FOR UPDATE it refuses, by RAISE, a
missing resource, a charter resource, an erased resource, an empty block list, a block of another
resource and a repeated block; the caller records the refusal through resource_erasure_refuse. An
in-flight ingest is cancelled (ingest_state cancelled), recorded as cancelled_ingest and as one
kb_resources.ingest_state target line. Appends ONE block_history_scrubbed event (NULL-anchored;
references: the resource as subject, the request reference as request; correlation: the request
reference) and applies it through _block_history_scrub_apply. Returns {event_id, targets}.';

-- ---------------------------------------------------------------------------
-- Section 5. THE REFUSAL (D5, D11): one `resource_erasure_refused` event,
-- closed reason vocabulary, nothing else mutated, for either act. `p_act` names
-- the refused act (absent means the erasure, ruling 5 of 2e); `p_blocks` names
-- the blocks a refused scrub named (ruling 7 of 2e). Both are DEFAULTed, so the
-- six-argument call a deployed binary makes still resolves; DROP + CREATE leaves
-- exactly one function, never an overload beside it.
-- ---------------------------------------------------------------------------
DROP FUNCTION resource_erasure_refuse(uuid, uuid, uuid, uuid, text, text);

CREATE FUNCTION resource_erasure_refuse(
    p_resource     uuid,
    p_attempted_by uuid,
    p_emitter      uuid,
    p_request_ref  uuid,
    p_reason       text,
    p_detail       text DEFAULT NULL,
    p_act          text DEFAULT NULL,
    p_blocks       uuid[] DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    -- The request reference is the refusal's correlation id; without one, _event_append
    -- correlates the event to itself and the refusal pairs with nothing.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_refuse: p_request_ref is required';
    END IF;
    IF p_reason NOT IN ('unauthorized','charter_resource','ingest_in_flight','already_erased') THEN
        RAISE EXCEPTION 'resource_erasure_refuse: % is not a resource-erasure refusal reason',
                        p_reason;
    END IF;
    IF p_act IS NOT NULL AND p_act NOT IN ('erasure','block_history_scrub') THEN
        RAISE EXCEPTION 'resource_erasure_refuse: % is not a refusable act', p_act;
    END IF;
    IF cardinality(p_blocks) > 0 AND p_act IS DISTINCT FROM 'block_history_scrub' THEN
        RAISE EXCEPTION 'resource_erasure_refuse: p_blocks is carried only by a block_history_scrub refusal';
    END IF;

    v_ev := _event_append(
        'resource_erasure_refused', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'subject_table', 'kb_resources',
            'subject_id',    p_resource,
            'actor',         p_attempted_by,
            'reason',        p_reason,
            'detail',        p_detail,
            'act',           p_act,
            'blocks',        CASE WHEN cardinality(p_blocks) > 0 THEN to_jsonb(p_blocks) END)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    RETURN v_ev;
END;
$$;

COMMENT ON FUNCTION resource_erasure_refuse(uuid, uuid, uuid, uuid, text, text, text, uuid[]) IS
'the negative face of resource erasure and of the block history scrub (spec D5, D11; the closed
refusal vocabulary ruled 2026-09-29): unauthorized | charter_resource | ingest_in_flight |
already_erased — one recorded event, nothing else mutated. p_act names the refused act, erasure
or block_history_scrub; absent means erasure. p_blocks names the blocks a refused scrub named and
is accepted only with p_act block_history_scrub; NULL or empty, it is omitted from the payload.
unauthorized and ingest_in_flight are RETIRED: no path raises them (a non-admin is answered at the
wire with no event; ingest state refuses neither act), and they stay accepted because removing a
value from a closed vocabulary is not additive. A repeat erasure, and a scrub of an erased
resource, is a recorded refusal (already_erased), not a silent no-op: nothing in the projection
changes, but the attempt is part of the record, the same as every other refusal.';

-- ---------------------------------------------------------------------------
-- Section 6. The payload_schemas, re-registered with the optional properties
-- build order 2e adds: `cancelled_ingest` on block_history_scrubbed, `act` and
-- `blocks` on resource_erasure_refused. Neither is required and neither schema
-- sets additionalProperties, so every payload valid before stays valid, and the
-- version stays 1. The precedent is 20261002000020.
-- ---------------------------------------------------------------------------
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BlockHistoryScrubbed",
  "description": "`block_history_scrubbed` — the remedy lighter than erasure (resource erasure spec D11): every\nrevision of each block but its current one, and every non-current chunk, emptied, on a LIVE\nresource. CAS-only; it touches no ledger payload.\n\nKeyed `subject_table` (`kb_content_blocks`) / `subject_ids`, never `block_id`:\n`element_trail_node` joins on `payload->>'block_id'` (the D1 join-key rule).",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting system admin.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "cancelled_ingest": {
      "description": "True when the scrub also cancelled an in-flight ingest. Replay reads this to reproduce\n`ingest_state = 'cancelled'`; absent means the ingest state was untouched.",
      "type": "boolean"
    },
    "subject_ids": {
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "subject_table": {
      "description": "Always `kb_content_blocks`.",
      "$ref": "#/$defs/AnchorTable"
    },
    "targets": {
      "description": "Per-block outcomes (revisions and chunks emptied).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureTargetOutcome"
      }
    }
  },
  "required": [
    "subject_table",
    "subject_ids"
  ],
  "$defs": {
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles",
        "kb_connections",
        "kb_machine_clients",
        "kb_blobs",
        "kb_events"
      ]
    },
    "ErasureTargetOutcome": {
      "description": "One target of a completed erasure and what happened to it (erasure spec, \"per-target\noutcomes\"). The target names itself the way the personal-data manifest does — `table` or\n`table.column`; the outcome is the act's own record of what redaction applied. Deliberately\nopen-textured in v1: ceilings are DATA, not types (D1), and the per-target vocabulary is the\nexecution build's to pin. `unhonourable_scope` outcomes land here, never silent.",
      "type": "object",
      "properties": {
        "outcome": {
          "description": "What the act did to it (erased / sentinel-scrubbed / accepted-in-part / …).",
          "type": "string"
        },
        "target": {
          "description": "Manifest identity of the target (`kb_profiles.display_name`, `kb_teams.slug`, …).",
          "type": "string"
        }
      },
      "required": [
        "target",
        "outcome"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb
 WHERE name = 'block_history_scrubbed';

UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ResourceErasureRefused",
  "description": "`resource_erasure_refused` — the negative face of resource erasure and of the block history\nscrub (resource erasure spec D5, D11). Keyed on the resource, spelled as [`ResourceErased`]\nspells it: every reason is a fact about the resource, whichever act was refused.",
  "type": "object",
  "properties": {
    "act": {
      "description": "Which act was refused. Absent means [`ErasureAct::Erasure`].",
      "anyOf": [
        {
          "$ref": "#/$defs/ErasureAct"
        },
        {
          "type": "null"
        }
      ]
    },
    "actor": {
      "description": "Who attempted the act.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "blocks": {
      "description": "The blocks a refused block history scrub named. Carried here, never in `subject_ids` or\n`detail`; empty for an erasure refusal.",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "detail": {
      "description": "The reason's evidence, e.g. the task that owns map-grain charter erasure.",
      "type": [
        "string",
        "null"
      ]
    },
    "reason": {
      "$ref": "#/$defs/ResourceErasureRefusalReason"
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "description": "Always `kb_resources`.",
      "$ref": "#/$defs/AnchorTable"
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "reason"
  ],
  "$defs": {
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles",
        "kb_connections",
        "kb_machine_clients",
        "kb_blobs",
        "kb_events"
      ]
    },
    "ErasureAct": {
      "description": "Which act a `resource_erasure_refused` event refuses. Absent on the payload reads as\n[`ErasureAct::Erasure`].",
      "oneOf": [
        {
          "description": "The resource erasure act.",
          "type": "string",
          "const": "erasure"
        },
        {
          "description": "The block history scrub.",
          "type": "string",
          "const": "block_history_scrub"
        }
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "ResourceErasureRefusalReason": {
      "description": "The closed refusal vocabulary for `resource_erasure_refused` (resource erasure spec D5, D11).",
      "oneOf": [
        {
          "description": "Retired: no path raises it. A non-admin is refused at the wire with no event. The value\nstays registered because removing one from a closed vocabulary is not additive.",
          "type": "string",
          "const": "unauthorized"
        },
        {
          "description": "A cogmap's telos/charter resource: map-grain erasure is its own act, named in `detail`.",
          "type": "string",
          "const": "charter_resource"
        },
        {
          "description": "Retired: no path raises it. Ingest state does not refuse an erasure; an in-flight ingest\nends with it (spec D5). The value stays registered because removing one from a closed\nvocabulary is not additive.",
          "type": "string",
          "const": "ingest_in_flight"
        },
        {
          "description": "The resource is already erased. Recorded by the erasure act on a repeat request and by\nthe block history scrub, which has nothing to scrub on an erased resource. Nothing in the\nprojection changes and no second `resource_erased` is minted.",
          "type": "string",
          "const": "already_erased"
        }
      ]
    }
  }
}
$JS$::jsonb
 WHERE name = 'resource_erasure_refused';

-- ---------------------------------------------------------------------------
-- Section 7. `resource_erasure_execute` (same signature; body: 20261002000020,
-- copied verbatim) records the ingest it ended only when the ingest was still
-- `in_progress`. The ingest target line's condition is the one change. A
-- `cancelled` ingest (20261003000110; the scrub sets it) was ended before the
-- erasure, and the erasure did not end it; `abandoned` is reserved for an
-- abandoned-ingest reaper, nothing sets it yet, and it is treated the same.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION resource_erasure_execute(
    p_resource    uuid,
    p_operator    uuid,
    p_emitter     uuid,
    p_request_ref uuid,
    p_also_strike_blobs uuid[] DEFAULT '{}'::uuid[]
) RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan    jsonb;
    v_charter uuid;
    v_ingest  text;
    v_erased  boolean;
    v_erased_ts timestamptz;
    v_found   boolean;
    v_id      uuid;
    v_ev      uuid;
    v_targets jsonb;
    v_remainder jsonb;
    v_edges   jsonb;
    v_i       integer;
    v_eid     uuid;
    v_bid     uuid; v_rel boolean; v_path text;
    v_ledger  jsonb := '[]'::jsonb;
BEGIN
    IF p_resource IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: p_resource is required';
    END IF;
    -- The request reference is the act's correlation id: every event the act appends carries
    -- it, and replay finds the act's span by it (D14). Without one, _event_append correlates
    -- each event to itself and the span is lost.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: p_request_ref is required';
    END IF;

    -- ── THE REFUSAL VERDICTS, read PRE-act (D5). A refusal here RAISES — the Rust caller
    --    catches the typed message and records it through resource_erasure_refuse, so the SQL
    --    never silently widens the negative face to a partial act (and never appends a refusal
    --    event itself: _event_append's emitter is the OPERATOR and a refused attempt at SQL
    --    grain would attribute wrongly). The verdict reads happen before anything mutates, and
    --    under R's row lock (D13), taken right after the existence check: FOR UPDATE waits out
    --    every writer already holding the row (their FK KEY SHARE, the write guard's KEY SHARE,
    --    the body-hash recompute's NO KEY UPDATE) so the plan, the folds and the body all see one
    --    settled state, and a writer arriving after it waits on the lock, then refuses at the
    --    write guard (Section W). The same lock serializes a second execute: its verdict read
    --    runs after the first commits, sees `erased_at`, and raises `already erased` rather than
    --    both passing the reads and double-completing.
    --    PR 2's service parses these strings into refusal vocabulary. Two states are NOT
    --    refusals (D5): an in-flight ingest (below, where `targets` names it) and a tombstone
    --    (the paragraph after the verdicts). ──
    SELECT count(*) > 0 INTO v_found FROM kb_resources r WHERE r.id = p_resource;
    IF NOT v_found THEN
        RAISE EXCEPTION 'resource_erasure_execute: resource % not found', p_resource;
    END IF;
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR UPDATE;
    -- R's captured original remote sources, locked BEFORE the plan reads them, in uuid order (the
    -- order step (9e) locks them in, so two acts sharing originals cannot deadlock). A citer whose
    -- _upsert_remote_source already holds one of these rows makes the act wait for its commit, so
    -- the plan's shared/exclusive split and step (9e)'s delete decision read the same citers; a
    -- citer arriving later waits on the act, and if the act deleted the row, its upsert inserts
    -- the URL as a fresh row.
    PERFORM 1 FROM kb_remote_sources rs
     WHERE rs.id IN (SELECT o.source_id FROM _resource_erasure_remote_originals(p_resource) o)
     ORDER BY rs.id
       FOR UPDATE;
    SELECT c.telos_resource_id INTO v_charter FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.erased_at INTO v_erased_ts
      FROM kb_resources r WHERE r.id = p_resource;
    v_erased := v_erased_ts IS NOT NULL;

    IF v_charter IS NOT NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: charter resource (map-grain erasure is filed task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)';
    END IF;
    IF v_erased THEN
        RAISE EXCEPTION 'resource_erasure_execute: already erased';
    END IF;
    -- A TOMBSTONE IS ERASABLE: a soft-deleted resource is not a refusal. It is arguably the
    -- flow's most common shape — the content was soft-deleted, and the compliance need then
    -- arrives that demands it not exist at all. The principal act has no tombstone refusal
    -- (it tombstones VIA the act, 20260909000025), F4's write floor makes is_active=false
    -- already permanent, and no restore verb exists (the spec's F4 — "un-modifiable on every
    -- axis"), so erasure is the only way out of a tombstone. The act completes over one
    -- mechanically: is_active is already false, the CHECK is satisfied, and COALESCE keeps
    -- erased_at stable. D6's rule ("a soft-deleted resource must never be mistaken for an
    -- erased one") is a PROJECTION honesty rule — erased_at is NULL until this act sets it. ──

    -- ── The ONE computation (D10). No re-enumeration of the remainder, block counts, artifact
    --    counts or edges happens below — the plan computed them once. The would_strike entries
    --    the principal plan's shape used do not exist here: the survey names related blobs via
    --    the remainder, and the operator's `also_strike_blobs` arrives AT THE ACT (D8), where
    --    the strike loop below consumes it. ──────────────────────────────────────────────────
    v_plan := resource_erasure_survey_plan(p_resource);
    v_targets := v_plan->'targets';
    v_ingest := v_plan->>'ingest_state';

    -- ── The operator-listed blob strikes, through the wrapper, PER ROW (D8: a blob is struck
    --    ONLY when the operator listed it; the act never infers a strike from a relation). Each
    --    strike carries the wrapper's verdict at ITS OWN moment — the byte-delete fence runs
    --    inside — and its prose template is the ONE the fence parses by exact prefix. The plan
    --    itself predicts nothing here, because the operator's list arrives at the act, not at
    --    the survey (the survey names related blobs; the operator answers with the subset to
    --    strike).
    --
    --    A listed blob the plan did NOT name is refused, not silently struck: the survey is the
    --    reviewed record of what the act may reach, and an operator widening it mid-act is a
    --    drift the fence exists to catch. ─────────────────────────────────────────────────────
    FOR v_i IN 0 .. coalesce(array_upper(p_also_strike_blobs, 1), 0) - 1 LOOP
        v_bid := p_also_strike_blobs[v_i + 1];
        IF NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(v_plan->'remainder') rem
             WHERE rem->>'target' = 'kb_blobs'
               AND rem->>'outcome' LIKE '%blob ' || v_bid::text || ';%') THEN
            RAISE EXCEPTION 'resource_erasure_execute: blob % is not in the survey''s related-blob remainder; strike refused', v_bid;
        END IF;
        SELECT blob_id, released, pathname
          INTO v_bid, v_rel, v_path
          FROM blob_delete('blob_erased',
                           jsonb_build_object('blob_id', v_bid),
                           p_emitter,
                           p_correlation => p_request_ref);
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_blobs',
            'outcome', blob_strike_outcome_text(v_rel, v_path)));
    END LOOP;

    -- ── Ingest state is not a refusal (D5): a partial or in-flight ingest ends with the
    --    erasure. The husk keeps its ingest_state; erased_at is authoritative, and the write
    --    floor (Section W) refuses any later attempt to continue the ingest. The record names
    --    the ingest the act ended. ─────────────────────────────────────────────────────────
    IF v_ingest = 'in_progress' THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources.ingest_state',
            'outcome', 'ingest ' || v_ingest || '; ended by erasure; erased_at is authoritative'));
    END IF;

    -- ── Per-edge folds: ONE relationship_folded per edge touching R (D1 — the incumbent verb,
    --    its OWN trail shows who ended it and why, another principal's view reads as
    --    deliberately ended; replay folds through the existing projector). reason is a FIXED
    --    literal 'resource_erased', never operator prose. Each event carries the act's
    --    correlation id (the request reference), so the act's pairing is a fact, not a
    --    convention. Edges are folded FIRST (the projected is_folded) so the plan's edge arm and
    --    the fold events agree in the same transaction.
    v_edges := v_plan->'edges';
    FOR v_i IN 0 .. jsonb_array_length(v_edges) - 1 LOOP
        v_eid := (v_edges->>v_i)::uuid;
        SELECT id INTO v_id FROM kb_edges WHERE id = v_eid AND NOT is_folded FOR UPDATE;
        IF v_id IS NULL THEN
            RAISE EXCEPTION 'resource_erasure_execute: edge % missing or already folded', v_eid;
        END IF;
        v_ev := _event_append('relationship_folded', p_emitter,
                              (SELECT home_anchor_table FROM kb_edges WHERE id = v_eid),
                              (SELECT home_anchor_id FROM kb_edges WHERE id = v_eid),
                              jsonb_build_object(
                                  'edge_id', v_eid,
                                  'reason', 'resource_erased'),
                              p_correlation => p_request_ref);
        PERFORM _project_relationship_folded(v_ev, jsonb_build_object(
            'edge_id', v_eid,
            'reason', 'resource_erased'));
    END LOOP;

    -- ── Projection-side sentinels + the content sweep — THE one body, at the act's event
    --    position. No events inside; the appended event below is the record. ─────────────────
    v_ev := _event_append(
        'resource_erased', p_emitter, NULL, NULL,
        jsonb_build_object(
            'subject_table', 'kb_resources',
            'subject_id', p_resource,
            'actor', p_operator,
            'redacted_fields', '[]'::jsonb,
            'folded_edges', v_edges,
            'targets', v_targets,
            'remainder', v_plan->'remainder',
            'ledger_remainder', v_plan->'ledger_remainder'),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    PERFORM _resource_erasure_apply_redaction(p_resource, v_ev);

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'edges',           v_edges,
        'targets',         v_targets,
        'remainder',       v_plan->'remainder',
        'ledger_remainder', v_plan->'ledger_remainder');
END;
$$;

SELECT declare_migration(
    20261003000210,
    'additive',
    'The block history scrub (resource erasure spec D11, build order 2e). CREATE OR REPLACEs _resource_erasure_apply_redaction with the same signature: the whole-resource call (p_blocks NULL) every deployed binary and the replay arm make reaches exactly what it reached before, and a non-NULL block set, which raised before, now runs steps 1-3 narrowed with keep-current. DROP + CREATE of resource_erasure_refuse adds two DEFAULTed parameters (p_act, p_blocks), so the six-argument call a deployed binary makes still resolves to the one function and writes the same payload. Re-registers the block_history_scrubbed and resource_erasure_refused payload_schemas with optional properties only (cancelled_ingest; act, blocks): nothing becomes required and neither schema sets additionalProperties, so every payload valid before stays valid. CREATE OR REPLACEs resource_erasure_execute with the same signature and return shape: its body is 20261002000020''s verbatim except that the kb_resources.ingest_state target line is written only for an in_progress ingest, so an erasure of a resource whose ingest was already cancelled or abandoned no longer claims to have ended it. Every other object is new: block_history_scrub_plan, block_history_scrub_survey, _block_history_scrub_apply and block_history_scrub_execute. No table, column, constraint or grant changes.'
);
