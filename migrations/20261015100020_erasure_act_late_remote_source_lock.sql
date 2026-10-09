-- Resource erasure: the act locks R's remote sources at step (9e) only, never at its head.
--
-- kb_remote_sources is one table across every tenant, deduplicated by normalized URL. The act
-- locked R's captured original remote sources FOR UPDATE right after R's row and held them to
-- commit, so that its plan's shared/exclusive split and step (9e)'s delete decision read the same
-- citers. A write anywhere that cites one of those URLs (an upsert of the same row) waited out the
-- whole act. Since the write-side lock bound (20261015100000) such a write, cut off at 5 s,
-- answers 503 RESOURCE_BUSY: a tenant could learn that something citing a URL was being erased.
-- Witness: temper-substrate tests/resource_erasure_act.rs,
-- a_citer_of_the_same_url_in_another_home_does_not_wait_on_the_act (task
-- 01a0fd12-f4b7-7bd2-81d0-13c0814650d5; found by the security review of that branch).
--
-- The act now takes those rows only where step (9e) already took them: one at a time, at the
-- act's tail, lock-then-check. A citer waits only for the act's last steps. The cost is that a
-- source the plan read as exclusive (recorded "deleted") can gain a citer before (9e) keeps it.
-- The act detects exactly that after (9e) and raises 'remote source <id> gained a citer during
-- the act'; the service classifies it retryable, like a raced edge fold, and the retry's plan
-- names the source shared. The inverse is reachable too: two acts on resources that share a URL
-- each plan it shared, and the second's (9e), after the first commits, finds no citer and deletes
-- it. That raises 'remote source <id> lost its citers during the act' (witness:
-- two_acts_sharing_a_url_never_record_a_deleted_source_as_kept). Either way the record is never
-- committed calling a kept source deleted or a deleted one kept.
--
-- Step (9e) also takes those rows together, NOWAIT, retrying the set, so the act never holds
-- one while it waits for another: locked one at a time, a write citing an earlier URL waited on
-- an act stalled on a later one (witness:
-- a_citer_of_one_url_does_not_wait_while_the_act_waits_on_another).
--
-- Both bodies are their live definitions (pg_get_functiondef after 20261015100010), changed only
-- as marked. resource_erasure_execute keeps its SET clauses (search_path, lock_timeout = 0).

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
    --      are considered — the act never deletes a remote source it did not orphan. They are
    --      locked FOR UPDATE first, and "does anything still cite it?" is asked of each in a
    --      SEPARATE, later statement. This is the act's only lock on them
    --      (20261015100020: an earlier lock at the act's head made writes citing the same URL in
    --      any tenant wait out the whole act), so a source the plan read as exclusive can gain a
    --      citer before this point; the lock-then-check keeps it, and resource_erasure_execute
    --      raises the race for a retry. Under READ COMMITTED each statement of this VOLATILE
    --      function reads a fresh snapshot, and a concurrent citer's _upsert_remote_source holds
    --      the row's lock (ON CONFLICT DO UPDATE) until it commits. So the lock waits (retrying, below) for that
    --      citer, and the existence check that follows sees its committed provenance row and
    --      keeps the source. A single `DELETE … WHERE NOT EXISTS (…)` evaluates its subquery
    --      against the statement's own snapshot, taken before the lock wait, and would delete a
    --      row a citer committed during that wait. A citer that arrives after the lock waits on
    --      it, for the act's remaining steps; if the row is deleted, its upsert inserts the URL
    --      as a fresh row. NOWAIT never waits while holding, so two acts whose resources share
    --      originals cannot deadlock on them.
    --      The lock takes EVERY captured original at once, NOWAIT, in a subtransaction, and
    --      retries the whole set after a short sleep when any one is held (20261015100020). The
    --      rows are shared across tenants: locked one at a time, the act held each to commit
    --      while it waited for the next, so a write citing an earlier URL, in any tenant, waited
    --      on the act and, past the write-side lock bound, answered 503 (found by the security
    --      review of that branch). A failed NOWAIT rolls the subtransaction back, releasing what
    --      it had locked, so the act holds none of these rows while it waits. It still waits for
    --      as long as a citer holds one, as it always did.
    LOOP
        BEGIN
            PERFORM 1 FROM kb_remote_sources r
             WHERE r.id = ANY(v_orig_sources)
             ORDER BY r.id
               FOR UPDATE NOWAIT;
            EXIT;
        EXCEPTION WHEN lock_not_available THEN
            PERFORM pg_sleep(0.05);
        END;
    END LOOP;
    FOR v_source IN
        SELECT DISTINCT s.id FROM unnest(v_orig_sources) AS s(id) ORDER BY s.id
    LOOP
        IF NOT EXISTS (SELECT 1 FROM kb_block_provenance q
                        WHERE q.source_kind = 'remote' AND q.source_id = v_source) THEN
            DELETE FROM kb_remote_sources r WHERE r.id = v_source;
        END IF;
    END LOOP;

    -- (9f) Artifact families (D4, ruled 2026-10-03, ruling 5): every artifact of R, live, folded
    --      or superseded, takes 'erased:<asserted_by_event_id>', the sentinel cut 2 writes into
    --      data_artifact_committed.artifact_kind, so after cut 2 this step is an idempotent no-op.
    --      artifact_kind is free text (F3 corrected): no kb_artifact_kinds table, and the doors
    --      accept any family. The projector decides nothing on it, so no equality has to survive.
    --      A shape's artifact_kind is the home's declaration, not R's, and is left (the remainder
    --      names it by shape id).
    UPDATE kb_data_artifacts da
       SET artifact_kind = 'erased:' || da.asserted_by_event_id::text
     WHERE da.resource_id = p_resource
       AND da.artifact_kind <> 'erased:' || da.asserted_by_event_id::text;

    RETURN;
END;
$function$;

CREATE OR REPLACE FUNCTION public.resource_erasure_execute(p_resource uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid, p_also_strike_blobs uuid[] DEFAULT '{}'::uuid[])
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path TO 'public', 'pg_temp'
 SET lock_timeout TO '0'
AS $function$
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
    v_redact  jsonb;
    v_fields  jsonb;
    v_payload jsonb;
    v_exclusive uuid[];
    v_shared    uuid[];
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
    -- The act queue, exclusive, before R's row lock (20261015100010): a floored writer arriving
    -- from here on queues behind this act instead of joining the row's KEY SHARE holders.
    PERFORM pg_advisory_xact_lock(_resource_act_queue_key(p_resource));
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR UPDATE;
    -- R's remote sources are NOT locked here (20261015100020). kb_remote_sources is shared across
    -- every tenant, deduplicated by URL, so a lock held from here to commit made any write that
    -- cites the same URL, anywhere, wait out the whole act, and past the write-side lock bound
    -- answer 503: a signal that something citing that URL was being erased. Step (9e) locks each
    -- one at the act's tail and decides there; a plan/decision mismatch raises below.
    SELECT c.telos_resource_id INTO v_charter FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.erased_at INTO v_erased_ts
      FROM kb_resources r WHERE r.id = p_resource;
    v_erased := v_erased_ts IS NOT NULL;

    IF v_charter IS NOT NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: charter resource (map-grain erasure is filed task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)';
    END IF;
    -- ── THE COMPLETION PASS (D12; 20261010100000). An erased resource is refused unless the
    --    derivation, re-run against the ledger as it stands now (never the first record's
    --    ledger_remainder, which a goal's telos copies keep non-empty for good), still names a
    --    path: a husk erased under cut 1. Then the act mints one more resource_erased for the same
    --    subject, carrying only redacted_fields, and rewrites those paths under it, in this
    --    transaction, which is the only one the verifier admits the rewrite in. The body runs
    --    AFTER the rewrite, the reverse of a first erasure: the subject is already erased, which is
    --    what a first erasure runs the body first to establish, and replay's arm runs the body at
    --    this record's position over the rewritten ledger, so live runs it over the same ledger.
    --    Over a husk erased under this body it changes only its stamps, the husk's `updated`
    --    and its property rows' `last_event_id` (witnessed); over one erased under an older body
    --    it also applies the steps that body lacked, as replay's arm does at this position, and
    --    on a husk whose home was materialized since, steps (7)-(7c) run again. Run before the
    --    rewrite instead, step (9e)'s capture would read the original URLs a cut-1 husk's ledger
    --    holds and find no position for the sentinel rows its provenance cites. No edge is
    --    folded, no blob struck, nothing named: the first record did all of that. ──
    IF v_erased THEN
        SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'payload', d.new_payload,
                                                     'metadata', d.new_metadata, 'paths', d.paths)
                                   ORDER BY d.event_id), '[]'::jsonb)
          INTO v_redact
          FROM _resource_erasure_payload_redaction(p_resource) d;
        IF jsonb_array_length(v_redact) = 0 THEN
            RAISE EXCEPTION 'resource_erasure_execute: already erased';
        END IF;
        -- The first record's plan named the related blobs, and its folds ended every relation a
        -- blob could reach the husk by, so the survey's remainder names none now.
        IF coalesce(array_length(p_also_strike_blobs, 1), 0) > 0 THEN
            RAISE EXCEPTION 'resource_erasure_execute: blob % is not in the survey''s related-blob remainder; strike refused',
                p_also_strike_blobs[1];
        END IF;
        SELECT coalesce(jsonb_agg(jsonb_build_object('event', r->'event', 'paths', r->'paths')
                                   ORDER BY r->>'event'), '[]'::jsonb)
          INTO v_fields
          FROM jsonb_array_elements(v_redact) r;
        v_payload := jsonb_build_object(
                'subject_table', 'kb_resources',
                'subject_id', p_resource,
                'actor', p_operator,
                'redacted_fields', v_fields);
        v_ev := _event_append(
            'resource_erased', p_emitter, NULL, NULL, v_payload,
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',
                    'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
                jsonb_build_object('rel','request',
                    'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
            p_correlation => p_request_ref);
        PERFORM _project_resource_erased_redactions(v_ev, v_payload);
        UPDATE kb_events e
           SET payload  = r->'payload',
               metadata = r->'metadata'
          FROM jsonb_array_elements(v_redact) r
         WHERE e.id = (r->>'event')::uuid;
        PERFORM _resource_erasure_apply_redaction(p_resource, v_ev);
        RETURN jsonb_build_object(
            'event_id',         v_ev,
            'edges',            '[]'::jsonb,
            'targets',          '[]'::jsonb,
            'remainder',        '[]'::jsonb,
            'redacted_fields',  v_fields,
            'ledger_remainder', '[]'::jsonb);
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

    -- The remote sources the plan records as deleted: R's captured originals (stable under R's
    -- row lock), less any that is itself a captured (block, n)'s sentinel (re-pointed onto, so
    -- kept), less those the plan's remainder names shared. Read from the PLAN, not re-derived,
    -- so the check after step (9e) compares the decision against the record that names it.
    SELECT coalesce(array_agg(DISTINCT o1.source_id), '{}')
      INTO v_exclusive
      FROM _resource_erasure_remote_originals(p_resource) o1
      JOIN kb_remote_sources rs ON rs.id = o1.source_id
     WHERE NOT EXISTS (
               SELECT 1 FROM _resource_erasure_remote_originals(p_resource) o2
                WHERE normalize_remote_uri('erased:' || o2.block_id::text || ':' || o2.n::text)
                      = rs.uri_normalized)
       AND NOT EXISTS (
               SELECT 1 FROM jsonb_array_elements(v_plan->'remainder') rem
                WHERE rem->>'target' = 'kb_remote_sources.id'
                  AND rem->>'outcome' LIKE 'shared remote source ' || o1.source_id::text || ';%');
    -- And the ones the plan's remainder names shared, which the record says are kept.
    SELECT coalesce(array_agg(DISTINCT o1.source_id), '{}')
      INTO v_shared
      FROM _resource_erasure_remote_originals(p_resource) o1
     WHERE EXISTS (
               SELECT 1 FROM jsonb_array_elements(v_plan->'remainder') rem
                WHERE rem->>'target' = 'kb_remote_sources.id'
                  AND rem->>'outcome' LIKE 'shared remote source ' || o1.source_id::text || ';%');

    -- ── The ledger redaction (D3, D4; 20261009100000), derived NOW, before the fold events
    --    below exist and before the body's step 9 rewrites the projection: the same function the
    --    plan lists redacted_fields from, under R's lock, with the payloads it will write. ──
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'payload', d.new_payload,
                                                 'metadata', d.new_metadata, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redact
      FROM _resource_erasure_payload_redaction(p_resource) d;
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', r->'event', 'paths', r->'paths')
                               ORDER BY r->>'event'), '[]'::jsonb)
      INTO v_fields
      FROM jsonb_array_elements(v_redact) r;

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
    v_payload := jsonb_build_object(
            'subject_table', 'kb_resources',
            'subject_id', p_resource,
            'actor', p_operator,
            'redacted_fields', v_fields,
            'folded_edges', v_edges,
            'targets', v_targets,
            'remainder', v_plan->'remainder',
            'ledger_remainder', v_plan->'ledger_remainder');
    v_ev := _event_append(
        'resource_erased', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    -- ── The ledger exception, in D3's order: the event is appended and its projector records
    --    each (event, path) it names. The redaction body runs next, over the ledger as it was:
    --    step (9e)'s capture reads each remote source's position in its event's list, which the
    --    rewrite would erase, and it marks the subject erased, which the verifier requires. Only
    --    then are the events rewritten, in one statement; kb_events_append_only admits each row
    --    against those rows, this transaction's erasure and the class sentinels, and
    --    kb_events_redaction_in_trail checks the trail once for the statement. ─────────────
    PERFORM _project_resource_erased_redactions(v_ev, v_payload);

    PERFORM _resource_erasure_apply_redaction(p_resource, v_ev);

    -- Step (9e) locked each captured original and kept any that something still cites. One the
    -- plan recorded as deleted but (9e) kept gained a citer after the plan read it (it was not
    -- locked then, above): the record would call a kept source deleted. Raise; the service
    -- retries the act, whose next plan sees the citer and names the source shared.
    SELECT rs.id INTO v_id FROM kb_remote_sources rs WHERE rs.id = ANY(v_exclusive) ORDER BY rs.id LIMIT 1;
    IF v_id IS NOT NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: remote source % gained a citer during the act', v_id;
    END IF;
    -- The inverse: one the plan named shared that (9e) deleted. Its other citer was a resource
    -- whose own erasure committed after this plan read it (two acts on resources sharing a URL),
    -- so the record would call a deleted source kept. Raise; the retry's plan sees it exclusive.
    SELECT s.id INTO v_id
      FROM unnest(v_shared) AS s(id)
     WHERE NOT EXISTS (SELECT 1 FROM kb_remote_sources rs WHERE rs.id = s.id)
     ORDER BY s.id LIMIT 1;
    IF v_id IS NOT NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: remote source % lost its citers during the act', v_id;
    END IF;

    UPDATE kb_events e
       SET payload  = r->'payload',
           metadata = r->'metadata'
      FROM jsonb_array_elements(v_redact) r
     WHERE e.id = (r->>'event')::uuid;

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'edges',           v_edges,
        'targets',         v_targets,
        'remainder',       v_plan->'remainder',
        'redacted_fields', v_fields,
        'ledger_remainder', v_plan->'ledger_remainder');
END;
$function$;

SELECT declare_migration(
    20261015100020,
    'additive',
    'CREATE OR REPLACE of resource_erasure_execute and _resource_erasure_apply_redaction with the same signatures, return types and SET clauses. The act no longer locks R''s remote sources before its plan; after step (9e) it raises a retryable race when a source its plan recorded as deleted was kept, or one it recorded as kept was deleted. The redaction body takes step (9e)''s remote-source locks together, NOWAIT, retrying the set, instead of one at a time. No table, column, constraint, grant or COMMENT changes.'
);
