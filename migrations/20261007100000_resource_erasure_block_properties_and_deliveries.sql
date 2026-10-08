-- Resource erasure: what the act left for a resource's block-owned properties and its subscription
-- deliveries (task 01a0fedb-bfe6-7783-bf71-5e030bd6a806, items 1 and 2; spec 2026-09-28 D2 step 9, D8).
--
-- Section 1. _resource_erasure_apply_redaction gains step (9d′): every property owned by one of R's
--            blocks, live or folded, gets the (9b) treatment — key erased-key-<n> numbered per
--            block, value "erased", folded. The ledger copy of a block's `role` stays: F3 rules
--            `role` structural.
-- Section 2. resource_erasure_survey_plan counts those rows in the kb_properties target, and its
--            remainder gains arm 5: subscription deliveries whose rationale or scope_reason quotes
--            R's id, or whose event is in R's trail scope, named by delivery id (Q1: listed only).
-- Section 3. _resource_erasure_key_numbers' comment names its third caller.
-- Section 4. webhook_received leaves the element trails and the erasure trail scope. Intake stores
--            new bodies under `body` (payload_version 2), but a version-1 body that carried a
--            resource's id at a top-level key still matched that resource's trail, showing the
--            remote's text to its readers. The sensitivity sweep already excludes the type
--            (sensitivity.event_resource). The registry's schema_version follows to 2.
--
-- Arms 3 and 5 match R's id through _resource_erasure_quotes (Section 0): any case, with or
-- without hyphens. Both bodies are their live definitions (20261004130000, 20261004150000)
-- verbatim except for the added statements, those matches, and their comments. Signatures and
-- return types are unchanged.

-- ---------------------------------------------------------------------------
-- Section 0. Whether a text quotes a resource's id.
-- ---------------------------------------------------------------------------
CREATE FUNCTION _resource_erasure_quotes(p_text text, p_resource uuid)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT position(replace(p_resource::text, '-', '') IN replace(lower(p_text), '-', '')) > 0;
$$;
COMMENT ON FUNCTION _resource_erasure_quotes(text, uuid) IS
'Whether p_text carries p_resource''s id in any case, with or without hyphens (hyphens are dropped
from both sides, so a split in another place also matches; for a remainder over-listing is the
safe direction). The erasure survey''s cross-resource arms (3 and 5) read quotes through this.
NULL text quotes nothing.';

-- ---------------------------------------------------------------------------
-- Section 1. The act.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public._resource_erasure_apply_redaction(p_resource uuid, p_event uuid, p_blocks uuid[] DEFAULT NULL::uuid[])
 RETURNS void
 LANGUAGE plpgsql
AS $function$
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

    -- ── (7b) The home context's telos SNAPSHOT nulled beside its watermark (D2 step 6, ruled
    --     2026-10-04). The live telos (`anchor_telos_embedding`) already drops R: step 3 nulled its
    --     embeddings and the husk is inactive. The stored snapshot does not: it holds R's share
    --     until a materialize re-arms it, and when R was the context's only goal the live telos is
    --     NULL, the drift gate declines on NULL, and nothing ever re-arms it. NULL is "not
    --     materialized" to the drift gate, matching the watermark nulled above; the region job
    --     the service enqueues after commit re-arms it from the live telos. Whatever R's doc type:
    --     re-arming is free, and a goal-only predicate would guard nothing. ──────────────────────
    UPDATE kb_contexts c
       SET telos_centroid = NULL
     WHERE c.telos_centroid IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');

    -- ── (7c) LIVE regions holding R as a member: centroid recomputed over current members' current
    --     embeddings (ruled 2026-10-04). Step 3 has nulled R's, and `avg` skips NULL, so this is the
    --     survivors' mean, or the zero vector when R was the only member. It must happen HERE, not
    --     in the drain: the drain's materialize re-forms these regions without the husk, so their
    --     member sets change, they are not reused, and fold_live_regions folds them with the
    --     centroid they hold at that moment — after (7d) has run. Recomputing now means the row the
    --     drain freezes is already clean. The statement is populate_readouts' centroid statement
    --     (temper-substrate write.rs), predicate and all. The readouts (telos_alignment, salience,
    --     cohesion) are left to the drain, which re-derives them within a minute. A helper, not
    --     an inline UPDATE: a recompute is not an erasure constant, and the 2d fence requires every
    --     assignment to a handled column in THIS body to be one (resource-erasure-surface.txt). ─
    PERFORM _resource_erasure_recompute_live_centroids(p_resource);

    -- ── (7d) FOLDED regions holding R as a member: centroid zeroed (ruled 2026-10-04). A folded
    --     region is never recomputed, so its centroid keeps R's share indefinitely, and R's
    --     vector is recoverable from it: the centroid itself when R was the only member, and
    --     `n·centroid − Σ survivors` from the member rows and the survivors' current chunks while
    --     they have not moved. The column is NOT NULL; the zero vector is the memberless
    --     convention (write.rs `zero_centroid`). Member rows stay: pointers, not content. ─────
    UPDATE kb_cogmap_regions r
       SET centroid = array_fill(0, ARRAY[768])::vector
     WHERE r.is_folded
       AND EXISTS (SELECT 1 FROM kb_cogmap_region_members mem
                    WHERE mem.region_id = r.id
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

    -- (9d′) Block-owned properties: the (9b) pass applied per block of R, live and folded
    --      (ruled 2026-10-07). kb_properties_owner_table_check admits kb_content_blocks, and the
    --      only writer today is _project_blocks' block_role, which only charter blocks carry
    --      and the act refuses charters (D5). The pass reaches every block-owned ROW, whatever
    --      its key. It does not reach a ledger copy: _resource_erasure_trail_scope has no arm for
    --      property events owned by a block, so a future block-owned property event needs one.
    --      Keys numbered WITHIN EACH BLOCK by _resource_erasure_key_numbers, which orders keys by
    --      their asserting event and is total only while one event asserts one key per owner
    --      (true of _project_blocks, which writes block_role alone). Values '"erased"'::jsonb,
    --      every row folded, last_event_id the erasure event.
    UPDATE kb_properties p
       SET property_key   = 'erased-key-' || k.n::text,
           property_value = '"erased"'::jsonb,
           is_folded      = true,
           last_event_id  = COALESCE(p_event, p.last_event_id)
      FROM kb_content_blocks b
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_content_blocks', b.id) k
     WHERE b.resource_id = p_resource
       AND p.owner_table = 'kb_content_blocks' AND p.owner_id = b.id
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
$function$
;

COMMENT ON FUNCTION _resource_erasure_apply_redaction(uuid, uuid, uuid[]) IS
'THE ONE row-anchored redaction body, for resource erasure (spec 2026-09-28 D2, D4) and for the
block history scrub (D11). Its scope parameter p_blocks has two forms. NULL is the whole resource:
chunk prose, header_path, block revision bytes, embeddings+embedded_with, search vector, data
artifact content ({}::jsonb, EVERY artifact of the resource whatever its kind owner, intent or
supersession — ruled 2026-09-28), citation-audit projected reasons, formation watermark nulls,
workflow-job payloads and last_errors in every status (unfinished rows cancelled to dead), the
ingestion record''s source_uri (erased:<id>, hash kept) and artifact verdict details (NULL), and
the projection-side sentinels (husk title/origin_uri, property keys erased-key-<n> by ledger order
of first appearance — the resource''s, each touching edge''s numbered per edge whoever authored
them, and each of its blocks'' numbered per block (20261007100000) — values ''erased''::jsonb, edge labels NULL, remote-source re-pointing to the sentinel rows
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
-- Section 2. The survey plan.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.resource_erasure_survey_plan(p_resource uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_exists        uuid;
    v_n_blocks      integer;
    v_n_revisions   integer;
    v_n_chunks      integer;
    v_n_artifacts   integer;
    v_n_edges       integer;
    v_edges         jsonb := '[]'::jsonb;
    v_charter_of    uuid;
    v_ingest        text;
    v_ledger        jsonb := '[]'::jsonb;
    v_genesis       uuid;
    v_erased        uuid;
    v_home_ctx      uuid;
    v_is_goal       boolean;
    v_remainder     jsonb := '[]'::jsonb;
    v_row           record;
    v_targets       jsonb := '[]'::jsonb;
    v_a             bigint;
    v_b             bigint;
    v_c             bigint;
BEGIN
    SELECT id INTO v_exists FROM kb_resources WHERE id = p_resource;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_survey_plan: resource % not found', p_resource;
    END IF;

    -- ── Scope counts, PRE-redaction state (the same discipline the principal plan's per-target
    --    arms hold: the reads only report what the act will find). ────────────────────────────
    SELECT count(*) INTO v_n_blocks FROM kb_content_blocks WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_revisions FROM kb_block_revisions br
      JOIN kb_content_blocks b ON b.id = br.block_id
     WHERE b.resource_id = p_resource;
    SELECT count(*) INTO v_n_chunks FROM kb_chunks WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_artifacts FROM kb_data_artifacts WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_edges FROM kb_edges e
      WHERE NOT e.is_folded
        AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
          OR (e.target_table = 'kb_resources' AND e.target_id = p_resource));

    -- ── The edges the act folds, each listed so the record's `folded_edges` and the per-edge
    --     events agree. LIVE edges only: an edge already folded by history is the fold's
    --     business, not this act's — enumerating it would abort execute against a lawful
    --     state. `folded_edges` lists only what THIS act folds; a pre-existing fold already
    --     carries its own event. ────────────────────────────────────────────────────────
    FOR v_row IN
        SELECT e.id
          FROM kb_edges e
          WHERE NOT e.is_folded
            AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
              OR (e.target_table = 'kb_resources' AND e.target_id = p_resource))
          ORDER BY e.id
    LOOP
        v_edges := v_edges || to_jsonb(v_row.id);
    END LOOP;

    -- ── THE TARGETS (D8): what the act will reach, one {target, outcome} per D2 step that
    --    reaches something, counted PRE-act here because the resource_erased payload is
    --    appended before the redaction body runs and the act never computes twice (D10). Each
    --    count reads the row set its step in _resource_erasure_apply_redaction updates, by the
    --    same predicate. The outcome text is counts and verbs only: never a title, URL, key,
    --    value or any other content (goal §8 — the record never repeats what it erased). A
    --    table the act reaches nothing in is not claimed; the husk is always reached. execute
    --    appends the blob strikes and the ended ingest to this list. ─────────────────────────
    -- D2 step 1: chunk prose and heading trails.
    SELECT count(*) INTO v_a FROM kb_chunk_content cc
      JOIN kb_chunks c ON c.id = cc.chunk_id
     WHERE c.resource_id = p_resource AND cc.content <> '';
    SELECT count(*) INTO v_b FROM kb_chunks
     WHERE resource_id = p_resource AND header_path IS NOT NULL;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_chunk_content',
            'outcome', v_a || ' chunk bodies emptied, hashes kept; '
                       || v_b || ' heading paths nulled');
    END IF;
    -- D2 step 2: every revision's bytes.
    SELECT count(*) INTO v_a FROM kb_block_content bc
     WHERE bc.block_revision_id IN (
           SELECT br.id FROM kb_block_revisions br
             JOIN kb_content_blocks b ON b.id = br.block_id
            WHERE b.resource_id = p_resource)
       AND bc.content <> '';
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_block_content',
            'outcome', v_a || ' block revision bodies emptied, hashes kept');
    END IF;
    -- D2 step 3: embeddings with their provenance.
    SELECT count(*) INTO v_a FROM kb_chunks
     WHERE resource_id = p_resource AND embedding IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_chunks.embedding',
            'outcome', v_a || ' embeddings nulled with embedded_with');
    END IF;
    -- D2 step 4: the search vector.
    SELECT count(*) INTO v_a FROM kb_resource_search_index
     WHERE resource_id = p_resource AND search_vector <> '';
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_resource_search_index',
            'outcome', v_a || ' search vector emptied');
    END IF;
    -- D2 step 5: every artifact's content.
    SELECT count(*) INTO v_a FROM kb_data_artifact_content dac
     WHERE dac.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND dac.content <> '{}'::jsonb;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifact_content',
            'outcome', v_a || ' artifact bodies emptied to {}, hashes kept');
    END IF;
    -- D2 step 6: formation watermarks.
    SELECT count(*) INTO v_a FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');
    SELECT count(*) INTO v_b FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource);
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'formation watermarks',
            'outcome', v_a || ' context and ' || v_b
                       || ' cogmap shape_materialized_event_id nulled');
    END IF;
    -- D2 step 6 (7c, 7d): region centroids holding R — live ones recomputed from the remaining
    --    members (the zero vector when R was alone), folded ones zeroed. Each count is its act
    --    statement's predicate, read before the act (see the migration header on concurrent
    --    materializes). Neither statement skips an already-cleared row, so on an erased resource
    --    this row still claims the folded regions its member rows point at.
    SELECT count(*) FILTER (WHERE NOT r.is_folded), count(*) FILTER (WHERE r.is_folded)
      INTO v_a, v_b
      FROM kb_cogmap_regions r
     WHERE EXISTS (SELECT 1 FROM kb_cogmap_region_members mem
                    WHERE mem.region_id = r.id
                      AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource);
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_cogmap_regions.centroid',
            'outcome', v_a || ' live region centroids recomputed from the remaining members; '
                       || v_b || ' folded region centroids zeroed');
    END IF;
    -- D2 step 6 (7b): the home context's telos snapshot.
    SELECT count(*) INTO v_a FROM kb_contexts c
     WHERE c.telos_centroid IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_contexts.telos_centroid',
            'outcome', v_a || ' context telos snapshot nulled');
    END IF;
    -- D2 step 7: workflow jobs, every status.
    SELECT count(*) FILTER (WHERE j.payload <> '{}'::jsonb OR j.last_error IS NOT NULL),
           count(*) FILTER (WHERE j.status IN ('pending', 'waiting_for_retry', 'in_progress'))
      INTO v_a, v_b
      FROM kb_workflow_jobs j WHERE j.resource_id = p_resource;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_workflow_jobs',
            'outcome', v_a || ' jobs emptied of payload and last_error; '
                       || v_b || ' unfinished jobs cancelled to dead');
    END IF;
    -- D2 step 7a: the ingestion record and the verdicts.
    SELECT count(*) INTO v_a FROM kb_ingestion_records ir
     WHERE ir.resource_id = p_resource AND ir.source_uri <> 'erased:' || p_resource::text;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_ingestion_records.source_uri',
            'outcome', v_a || ' ingestion record source_uri sentineled, source_hash kept');
    END IF;
    SELECT count(*) INTO v_a FROM kb_data_artifact_verdicts v
     WHERE v.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND v.detail IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifact_verdicts.detail',
            'outcome', v_a || ' verdict details nulled');
    END IF;
    -- D2 step 8: projected citation-audit reasons.
    SELECT count(*) INTO v_a FROM kb_citation_audits ca
     WHERE ca.block_id IN (
           SELECT b.id FROM kb_content_blocks b WHERE b.resource_id = p_resource)
       AND ca.reason IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_citation_audits.reason',
            'outcome', v_a || ' audit reasons nulled');
    END IF;
    -- D2 step 9: the husk (always), the property sentinels, the edges, the remote sources.
    v_targets := v_targets || jsonb_build_object('target', 'kb_resources',
        'outcome', '1 husk: title and origin_uri sentineled, is_active cleared, erased_at set');
    SELECT count(*) INTO v_a FROM kb_properties p
      JOIN _resource_erasure_key_numbers('kb_resources', p_resource) k
        ON k.property_key = p.property_key
     WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource;
    SELECT count(*) INTO v_b
      FROM kb_edges e
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_edges', e.id) k
      JOIN kb_properties p
        ON p.owner_table = 'kb_edges' AND p.owner_id = e.id AND p.property_key = k.property_key
     WHERE (e.source_table = 'kb_resources' AND e.source_id = p_resource)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_resource);
    SELECT count(*) INTO v_c
      FROM kb_content_blocks b
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_content_blocks', b.id) k
      JOIN kb_properties p
        ON p.owner_table = 'kb_content_blocks' AND p.owner_id = b.id
       AND p.property_key = k.property_key
     WHERE b.resource_id = p_resource;
    IF v_a + v_b + v_c > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_properties',
            'outcome', v_a || ' resource-owned, ' || v_b || ' edge-owned and ' || v_c
                       || ' block-owned rows: keys erased-key-<n>, values erased, folded');
    END IF;
    SELECT count(*) INTO v_b FROM kb_edges e
     WHERE e.label IS NOT NULL
       AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
         OR (e.target_table = 'kb_resources' AND e.target_id = p_resource));
    IF v_n_edges + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_edges',
            'outcome', v_n_edges || ' edges folded by this act; ' || v_b || ' labels nulled');
    END IF;
    --    The remote sources, from the ONE capture step (9e) reads
    --    (_resource_erasure_remote_originals): the provenance rows it re-points, the exclusive
    --    originals it deletes, the shared ones it keeps. An exclusive original that is itself
    --    some captured (block, n)'s sentinel row is re-pointed onto, so it stays cited and is
    --    not deleted; the count leaves it out for that reason.
    WITH o AS MATERIALIZED (
        SELECT * FROM _resource_erasure_remote_originals(p_resource)
    )
    SELECT (SELECT count(*) FROM kb_block_provenance bp
              JOIN o ON o.block_id = bp.block_id AND o.source_id = bp.source_id
             WHERE bp.source_kind = 'remote'),
           (SELECT count(DISTINCT o1.source_id) FROM o o1
              JOIN kb_remote_sources rs ON rs.id = o1.source_id
             WHERE NOT o1.shared
               AND NOT EXISTS (
                   SELECT 1 FROM o o2
                    WHERE normalize_remote_uri('erased:' || o2.block_id::text || ':' || o2.n::text)
                          = rs.uri_normalized)),
           (SELECT count(DISTINCT o1.source_id) FROM o o1 WHERE o1.shared)
      INTO v_c, v_a, v_b;
    IF v_c > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_block_provenance',
            'outcome', v_c || ' remote provenance rows re-pointed to erased:<block_id>:<n> sentinels');
    END IF;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_remote_sources',
            'outcome', v_a || ' exclusive remote sources deleted; '
                       || v_b || ' shared remote sources kept, named in the remainder');
    END IF;

    -- ── THE REFUSAL FACE (D5), computed here so execute consumes the verdict rather than
    -- re-deriving it: a charter resource (Q2 — the map-grain act is another task, named), an
    -- already-erased resource. The ingest state is reported, not refused: an in-flight ingest
    -- ends with the erasure and execute names it in `targets`. ─────────────────────────────
    SELECT c.telos_resource_id INTO v_charter_of FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.ingest_state INTO v_ingest FROM kb_resources r WHERE r.id = p_resource;

    -- ── THE REMAINDER (D8). Each entry is the named, never-struck part. The four shapes: ─────

    -- 1. Blobs related to the resource, live or struck, through a relation edge. The act folds
    --    their edges and names each blob; it strikes ONLY what the operator listed
    --    (also_strike_blobs), per row through blob_delete('blob_erased', …) and the byte-delete
    --    fence. It never infers a strike from a relation.
    FOR v_row IN
        SELECT DISTINCT b.id, b.content_hash, b.blob_pathname,
               EXISTS (SELECT 1 FROM kb_blobs live
                        WHERE live.id = b.id AND live.content_type IS NOT NULL) AS is_live
          FROM kb_blobs b
          JOIN kb_edges e ON NOT e.is_folded
               AND ((e.source_table = 'kb_blobs' AND e.source_id = b.id
                      AND e.target_table = 'kb_resources' AND e.target_id = p_resource)
                 OR (e.target_table = 'kb_blobs' AND e.target_id = b.id
                      AND e.source_table = 'kb_resources' AND e.source_id = p_resource))
         ORDER BY b.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_blobs',
            'outcome', 'related blob ' || v_row.id::text || '; hash ' || v_row.content_hash
                       || '; ' || CASE WHEN v_row.is_live THEN 'live; struck only when the operator lists it'
                                            ELSE 'already struck' END);
    END LOOP;

    -- 2. Derivers: resources with a live `derived_from` edge INTO R, or block provenance citing R.
    --    Named by id, never touched; their citing provenance rows are ids only (no text).
    FOR v_row IN
        SELECT DISTINCT e.source_id AS deriver_id
          FROM kb_edges e
         WHERE e.is_folded = false AND e.edge_kind = 'leads_to'
           AND e.label = 'derived_from'
           AND e.target_table = 'kb_resources' AND e.target_id = p_resource
           AND e.source_table = 'kb_resources'
        UNION
        SELECT DISTINCT b.resource_id
          FROM kb_block_provenance q
          JOIN kb_content_blocks b ON b.id = q.block_id
         WHERE q.source_kind = 'resource' AND q.source_id = p_resource
     LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'deriver',
            'outcome', 'resource ' || v_row.deriver_id::text
                       || ' holds a structural lead (' ||
                       CASE WHEN EXISTS (SELECT 1 FROM kb_edges e
                                          WHERE e.is_folded = false AND e.edge_kind = 'leads_to'
                                            AND e.label = 'derived_from'
                                            AND e.target_table = 'kb_resources' AND e.target_id = p_resource
                                            AND e.source_table = 'kb_resources' AND e.source_id = v_row.deriver_id)
                            THEN 'derived_from edge'
                            ELSE 'provenance citation' END
                       || '); never touched; discovery-bound');
    END LOOP;

    -- 3. Cross-resource ledger text that quotes R (Q1, listed only): events of a quoting type
    --    whose payload text names R's id AND whose subject arm is NOT R's own. A
    --    citation_audited event ON one of R's blocks is R's own trail (step (6) reached its
    --    projected column; its ledger reason rides R's ledger remainder below) — this arm is
    --    only the events anchored elsewhere. Structurally: the payload text contains R's id,
    --    and no block of R carries the payload's block_id. Named by event id, never redacted —
    --    the exception never crosses the resource boundary (Q1).
    FOR v_row IN
        SELECT DISTINCT ev.id, et.name
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name IN ('citation_audited', 'subscription_delivery_disposed',
                           'invocation_closed')
           AND _resource_erasure_quotes(ev.payload::text, p_resource)
           AND NOT EXISTS (
               SELECT 1 FROM kb_content_blocks b
                WHERE b.resource_id = p_resource
                  AND b.id = (ev.payload->>'block_id')::uuid)
           AND NOT EXISTS (
               SELECT 1
                 FROM _resource_erasure_trail_scope(p_resource) t
                WHERE t.event_id = ev.id)
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'cross-resource ledger text',
            'outcome', 'event ' || v_row.id::text || ' (' || v_row.name
                       || ') may quote the resource; listed only (Q1), never redacted');
    END LOOP;

    -- 4. Shared remote sources: kb_remote_sources rows R's blocks cite that ANOTHER
    --    resource's block also cites (live or folded — the row is shared either way), read from
    --    the ONE capture the act's step (9e) re-points from (_resource_erasure_remote_originals).
    --    A source R exclusively cites is NOT here: step (9e) deletes it, and replay of the
    --    redacted payloads never mints it, so the projection agrees by construction. A shared
    --    one stays and is named by its kb_remote_sources id, never by its URL (D4, D8): the
    --    record is an admin event outside the trail scope, so nothing could ever redact a URL
    --    written into it.
    FOR v_row IN
        SELECT DISTINCT o.source_id
          FROM _resource_erasure_remote_originals(p_resource) o
         WHERE o.shared
         ORDER BY o.source_id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_remote_sources.id',
            'outcome', 'shared remote source ' || v_row.source_id::text
                       || '; another resource''s block still cites it; named, kept');
    END LOOP;

    -- 5. Subscription deliveries that may quote R (Q1, listed only; ruled 2026-10-07). A delivery
    --    is projected only by webhook intake, so it carries three texts: the remote's body (its
    --    event's payload, bare at version 1, under `body` at 2), `rationale` (copied onto
    --    subscription_delivery_disposed, which arm 3 lists by event) and `scope_reason` (a
    --    caller's text, never on the ledger, so no event id can name it). A delivery is named,
    --    by its id, when any of the three quotes R's id. Never redacted.
    FOR v_row IN
        SELECT d.id
          FROM kb_subscription_deliveries d
          JOIN kb_events ev ON ev.id = d.event_id
         WHERE _resource_erasure_quotes(d.rationale, p_resource)
            OR _resource_erasure_quotes(d.scope_reason, p_resource)
            OR _resource_erasure_quotes(ev.payload::text, p_resource)
         ORDER BY d.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_subscription_deliveries',
            'outcome', 'delivery ' || v_row.id::text
                       || ' may quote the resource in its webhook body, rationale or scope_reason; listed only (Q1), never redacted');
    END LOOP;

    -- R's home context, whether R was ever a goal, and R's genesis — inputs to the telos arm
    -- below. Goal-ness is read from the LEDGER, not from kb_properties: step 9 sentinels and folds
    -- R's doc_type row, so a plan computed after the act (the survey on the husk, cut 2's
    -- completion pass re-deriving) would otherwise drop every telos copy without error. "Ever" —
    -- the created doc_type or any doc_type property event, normalized as the projection
    -- normalizes it — so a goal later re-typed still names the snapshots it contributed to.
    -- Cut 2 must read this before it redacts R's trail: the property arm reads `property_key` and
    -- `value`, which this plan's own CASE names for redaction (the created arm's `doc_type` is not).
    SELECT h.anchor_id INTO v_home_ctx FROM kb_resource_homes h
     WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts';
    v_is_goal := EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name = 'resource_created'
           AND (ev.payload ->> 'resource_id')::uuid = p_resource
           AND ev.payload ->> 'doc_type' = 'goal')
      OR EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name IN ('property_set', 'property_asserted')
           AND (ev.payload -> 'owner') ->> 'table' = 'kb_resources'
           AND ((ev.payload -> 'owner') ->> 'id')::uuid = p_resource
           AND ev.payload ->> 'property_key' = 'doc_type'
           AND _property_value_normalized('doc_type', ev.payload -> 'value') #>> '{}' = 'goal');
    SELECT ev.id INTO v_genesis
      FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.name = 'resource_created' AND (ev.payload ->> 'resource_id')::uuid = p_resource
     ORDER BY ev.id
     LIMIT 1;
    SELECT ev.id INTO v_erased
      FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.name = 'resource_erased' AND ev.payload ->> 'subject_id' = p_resource::text
     ORDER BY ev.id
     LIMIT 1;

    -- ── THE LEDGER REMAINDER (D12): the resource's OWN ledger free-text paths the act has not
    --    reached, in exactly the shape `RedactedEventFields` uses — {event, paths} — so the
    --    cut-2 completion pass reads this list directly, no translation. The scope is the ONE
    --    trail-scope definition (see _resource_erasure_trail_scope, F2); every free-text path
    --    per F3's catalog, spelled explicitly per event type so a new payload field is caught
    --    by the fence (D9, build order 2d) rather than drifting silent here. Ids of EXISTING
    --    events only — no id is minted; paths only, never values (a redaction record must not
    --    carry what it would redact). `metadata` authorship prose (F3's last row: reasoning /
    --    rationale) rides each event's own list when the metadata carries those keys.
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', s.event_id, 'paths', s.paths)
                               ORDER BY s.event_id), '[]'::jsonb)
      INTO v_ledger
      FROM (
        SELECT t.event_id,
               CASE t.event_type
                   WHEN 'resource_created'           THEN '["title","origin_uri","blocks[*].incorporated[*].source.value"]'::jsonb
                   WHEN 'resource_updated'           THEN '["title","origin_uri"]'::jsonb
                   WHEN 'block_created'              THEN '["block.incorporated[*].source.value"]'::jsonb
                   WHEN 'block_mutated'              THEN '["incorporated[*].source.value"]'::jsonb
                   WHEN 'block_folded'               THEN '["reason"]'::jsonb
                   WHEN 'citation_audited'           THEN '["reason"]'::jsonb
                   WHEN 'relationship_asserted'      THEN '["label"]'::jsonb
                   WHEN 'relationship_folded'        THEN '["reason"]'::jsonb
                   WHEN 'relationship_corrected'     THEN '["scar"]'::jsonb
                   WHEN 'block_provenance_corrected' THEN '["scar","source.value"]'::jsonb
                   WHEN 'property_set'               THEN '["property_key","value"]'::jsonb
                   WHEN 'property_asserted'          THEN '["property_key","value"]'::jsonb
                   WHEN 'property_unset'             THEN '["property_key"]'::jsonb
                   WHEN 'block_provenance_annotated' THEN '["incorporated[*].source.value"]'::jsonb
                   WHEN 'resource_reblocked'         THEN '["created[*].attribution[*].source.value","kept[*].attribution[*].source.value"]'::jsonb
                   ELSE '[]'::jsonb
               END
             || (CASE WHEN EXISTS (
                       SELECT 1 FROM jsonb_object_keys(t.metadata) k
                        WHERE k = 'reasoning' OR k = 'rationale')
                      THEN '["metadata.reasoning","metadata.rationale"]'::jsonb
                      ELSE '[]'::jsonb END) AS paths
          FROM (
            SELECT s.event_id, s.event_type, ev.metadata
              FROM _resource_erasure_trail_scope(p_resource) s
              JOIN kb_events ev ON ev.id = s.event_id
          ) t
         UNION ALL
        -- ── The telos copies (ruled 2026-10-04): `ledger_remainder` names the ledger paths
        --    carrying R's content, not only R's own trail. When R was ever a goal, every telos
        --    snapshot its current home context's materializations and salience refreshes recorded
        --    may average R's embedding in. Bounded by R's genesis below and, once R is erased,
        --    by its `resource_erased` above (ruled 2026-10-04): later snapshots are the context's
        --    own history, and a re-derive must name the same set the act recorded. Not named: a
        --    snapshot that a materialize already in flight at the act minted after it (loaded
        --    before the act, so its telos may still average R in; the region drain repairs the
        --    stored snapshot, not that ledger copy), and a former home's snapshots. Named, never reached here: the ledger edit is
        --    cut 2's. Bounded below by R's genesis — R contributed nothing before it existed — and
        --    otherwise over-inclusive by design: a snapshot minted while R was done or unembedded
        --    is named too. Not in the trail scope (anchored on the context, not on R), so no
        --    event appears twice. ────────────────────────────────────────────────────────────
        SELECT ev.id, '["telos_centroid"]'::jsonb
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE v_is_goal
           AND et.name IN ('region_materialized', 'salience_refreshed')
           AND ev.producing_anchor_table = 'kb_contexts' AND ev.producing_anchor_id = v_home_ctx
           AND (v_genesis IS NULL OR ev.id > v_genesis)
           AND (v_erased IS NULL OR ev.id < v_erased)
           AND jsonb_typeof(ev.payload -> 'telos_centroid') = 'string'
       ) s;

    RETURN jsonb_build_object(
        'resource',        p_resource,
        'n_blocks',        v_n_blocks,
        'n_revisions',     v_n_revisions,
        'n_chunks',        v_n_chunks,
        'n_artifacts',     v_n_artifacts,
        'n_edges',         v_n_edges,
        'edges',           v_edges,
        'targets',         v_targets,
        'already_erased',  (SELECT r.erased_at IS NOT NULL FROM kb_resources r WHERE r.id = p_resource),
        'charter_of',      v_charter_of,
        'ingest_state',    v_ingest,
        'fingerprint_available',
            to_regproc('sensitivity.deriver_fingerprint_matches') IS NOT NULL,
        'remainder',       v_remainder,
        'ledger_remainder', v_ledger);
END;
$function$
;

-- ---------------------------------------------------------------------------
-- Section 3. The key numbering's callers.
-- ---------------------------------------------------------------------------
COMMENT ON FUNCTION _resource_erasure_key_numbers(text, uuid) IS
'The property-key sentinel numbering (spec 2026-09-28 D4): one row per distinct original key the
owner''s kb_properties rows carry, live and folded, with n for erased-key-<n>, in ledger order of
first appearance (the first-asserting event''s occurred_at, then its id; never the key text). The
ONE definition: the redaction body''s step (9b) numbers the resource with it, step (9d) each edge
touching the resource, and step (9d′) each block of the resource (20261007100000).';

-- ---------------------------------------------------------------------------
-- Section 4. A received webhook is in no resource's or edge's trail.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.element_trail_node(p_profile uuid, p_resource uuid)
 RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamp with time zone, metadata jsonb, payload jsonb, actor_name text, correlation_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    WITH ev_ids AS (
        -- `producing_anchor_table IS NOT NULL` on every arm, not once at the end: it prunes inside
        -- the index scans rather than after the UNION.
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
          AND ev.producing_anchor_table IS NOT NULL
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE et.category = 'domain'
      AND et.name <> 'webhook_received'
      AND EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$function$
;

CREATE OR REPLACE FUNCTION public.element_trail_edge(p_profile uuid, p_edge uuid)
 RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamp with time zone, metadata jsonb, payload jsonb, actor_name text, correlation_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'edge_id')::uuid = p_edge
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_edges'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_edge
           AND ev.producing_anchor_table IS NOT NULL
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM kb_edges edg
    JOIN ev_ids ON TRUE
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND et.category = 'domain'
      AND et.name <> 'webhook_received'
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$function$
;

CREATE OR REPLACE FUNCTION public._resource_erasure_trail_scope(p_resource uuid)
 RETURNS TABLE(event_id uuid, event_type text)
 LANGUAGE sql
 STABLE
AS $function$
    SELECT ev.id, et.name
      FROM kb_events ev
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.category = 'domain'
       AND et.name <> 'webhook_received'
       AND (
            (ev.payload ->> 'resource_id')::uuid = p_resource
         OR ((ev.payload #>> '{owner,table}') = 'kb_resources'
             AND (ev.payload #>> '{owner,id}')::uuid = p_resource)
         OR EXISTS (SELECT 1 FROM kb_content_blocks b
                     WHERE b.id = (ev.payload ->> 'block_id')::uuid
                       AND b.resource_id = p_resource)
         OR EXISTS (SELECT 1 FROM kb_edges ee
                     WHERE ee.id = (ev.payload ->> 'edge_id')::uuid
                       AND ((ee.source_table = 'kb_resources' AND ee.source_id = p_resource)
                         OR (ee.target_table = 'kb_resources' AND ee.target_id = p_resource)))
         OR ((ev.payload #>> '{owner,table}') = 'kb_edges'
             AND (ev.payload #>> '{owner,id}')::uuid IN (
                 SELECT ee2.id FROM kb_edges ee2
                  WHERE (ee2.source_table = 'kb_resources' AND ee2.source_id = p_resource)
                     OR (ee2.target_table = 'kb_resources' AND ee2.target_id = p_resource)))
       );
$function$
;

COMMENT ON FUNCTION _resource_erasure_trail_scope(uuid) IS
'the ONE scope predicate for "a resource''s own ledger events" (spec 2026-09-28 F2): the
element-trail read''s own predicate (payload->>''resource_id''; property events owner-keyed to the
resource; block events through the block join; events carrying a touched edge''s edge_id) PLUS
property events whose owner IS an edge touching the resource (edge-owned properties ride the
20260727000030 edge-facet shape). element_trail_edge reads that same owner arm for one edge since
20260930000010; element_trail_node has no edge arms, so over a resource this predicate is still the
only union of them. A received webhook is never one of them, whatever its body carries
(20261007100000). Every consumer — the survey''s ledger remainder, cut 2''s completion pass, any
operator audit — walks THIS predicate, never a second derivation.';

UPDATE kb_event_types SET schema_version = 2 WHERE name = 'webhook_received';

SELECT declare_migration(
    20261007100000,
    'additive',
    'CREATE OR REPLACE of _resource_erasure_apply_redaction and resource_erasure_survey_plan with unchanged signatures and return types, plus a COMMENT. The act also sentinels and folds properties owned by the erased resource''s blocks (today only charter block_role rows exist, and the act refuses charters, so no live row changes). The plan''s kb_properties target text gains a block-owned count, and its remainder gains kb_subscription_deliveries entries; arm 3 matches ids in any case. element_trail_node, element_trail_edge and _resource_erasure_trail_scope (unchanged signatures) stop returning webhook_received events, which no reader is meant to see there; the webhook_received registry row''s schema_version becomes 2, read by nothing. A new helper function _resource_erasure_quotes is added. ErasureTargetOutcome is open-textured, so no payload schema changes, and a binary that predates this reads both lists unchanged. No table, column, constraint or grant changes.'
);
