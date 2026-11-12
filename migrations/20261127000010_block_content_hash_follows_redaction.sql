-- Redaction empties a block's stored hash with its bytes.
--
-- kb_block_content.content_hash is the bare sha256 of the row's bytes. Both redactions that empty
-- those bytes — resource erasure and its block-history scrub (_resource_erasure_apply_redaction)
-- and the principal act's hash-keyed arm (_erasure_apply_redaction, arm 3) — set content to ''
-- and left content_hash naming the text that was there. Nothing served it, but reads are about to
-- (`bytes_sha256` on GET /api/resources/{id}/blocks, and the `bytes_merkle` over those hashes on
-- resource reads), and a stored sha256 of short erased text is a dictionary-confirmation surface.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * THE HASH FOLLOWS THE BYTES IN THE SAME STATEMENT. Each redaction's UPDATE sets
--     content = '' and content_hash = sha256('') together, so no reader, in any snapshot, sees
--     emptied bytes beside the old hash. ck_kb_block_content_emptied_hash makes it structural: a
--     future path that empties content and forgets the hash raises.
--   * THE HASH IS RECOMPUTED, NOT BLANKED: the column stays NOT NULL and keeps meaning "sha256 of
--     these bytes", so a served bytes_sha256 is honest about an emptied row.
--   * NOTHING ELSE MOVES. block_body_hash / body_hash are merkles over CHUNK hashes, not over this
--     column, so they stay valid (resource erasure spec: "hashes are kept everywhere, so the
--     merkles stay valid"). No hash enters kb_erased_content from a resource act (D12 rule 0):
--     this migration adds none, and the principal act's record of what it redacted stays its
--     event's `redacted_hashes` and kb_erased_content, never the emptied row. The ledger carries
--     no block-bytes hash at all (BlockContent rides the sidecar only), and replay re-supplies
--     sidecars from the post-erasure rows, so replay stays byte-identical.
--   * THE BACKFILL IS EXACT: a row whose content is '' and whose hash is not sha256('') is a row a
--     redaction emptied. Nothing else writes '' with another hash.
--
-- Both bodies are their live definitions (pg_get_functiondef: _resource_erasure_apply_redaction
-- after 20261015100020, _erasure_apply_redaction after 20261023100000), changed only as marked.
-- Witness: temper-services tests/block_content_hash_redaction_test.rs.

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
    --     current_revision_id, and every revision of a folded block. The stored hash follows the
    --     bytes in the same statement (20261127000010): an emptied row carries sha256(''), never a
    --     hash of the text it held. ──────────────────────────────────────────────────────────
    UPDATE kb_block_content bc
       SET content = '',
           content_hash = encode(sha256(''::bytea), 'hex')
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

CREATE OR REPLACE FUNCTION _erasure_apply_redaction(p_subject uuid, p_hashes text[], p_event uuid)
 RETURNS void
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_sentinel text := 'erased-' || p_subject::text;
    v_team     uuid;
    -- The estate the act recorded (D1 of the 2026-10-10 person-erasure design): the subject's
    -- @me contexts and their personal team's contexts, read from THIS event's payload, never
    -- re-derived. Live and replay therefore reach the same set whatever has happened to
    -- personal_of since. Required: no principal was erased before the key existed (Q5, ruled
    -- 2026-10-10, guarded by 20261021100000), so an event without it is a defect, not history.
    v_estate   uuid[] := (SELECT ARRAY(SELECT jsonb_array_elements_text(e.payload->'estate_contexts')::uuid)
                            FROM kb_events e
                           WHERE e.id = p_event AND e.payload ? 'estate_contexts');
    -- The categories the deployment's retention policy kept, read from THIS event's payload like
    -- the estate (20261023100000, R4): live and replay keep the same set whatever the policy says
    -- since. Required for the same reason the estate is: no principal_erased predates the key.
    v_chains   uuid[];
    v_pass     integer;
    v_retained jsonb := (SELECT e.payload -> 'retained' FROM kb_events e
                          WHERE e.id = p_event AND e.payload ? 'retained');
BEGIN
    IF v_estate IS NULL THEN
        RAISE EXCEPTION '_erasure_apply_redaction: event % records no estate_contexts', p_event;
    END IF;
    IF v_retained IS NULL THEN
        RAISE EXCEPTION '_erasure_apply_redaction: event % records no retained categories', p_event;
    END IF;

    -- (1) The personal-team denormalization (D5 blind-spot row 1 — the only denormalization
    --     carrier Derivation C found). Found by kb_teams.personal_of, never by recomputing the
    --     slug: a personal team whose slug was held at genesis carries a `-N` suffix, and the
    --     team at the bare slug is someone else's (20261013100000). Scrubbed to the same
    --     derivation applied to the sentinel identity. The trigger is AFTER INSERT only, so
    --     this never re-fires.
    SELECT t.id INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;

    UPDATE kb_teams
       SET slug = 'personal-' || v_sentinel,
           name = v_sentinel || ' (personal)'
      WHERE id = v_team;

    -- Arms (2)-(5), (11) and (12)'s member arm reach by hash. Since 20261021100000 every estate
    -- resource but a charter has already been erased by resource erasure in this transaction,
    -- row by row, before this body runs, and the act's hashes are the charters' and the struck
    -- blobs' alone (D5). So for a husk these arms find nothing left to empty, and what they still
    -- do is empty the charters the act holds (D3).
    --
    -- (2) Chunk prose emptied BY HASH, hash kept (D3: emptied, not deleted — the CAS
    --     retention rule never fires) — GOVERNED HOMES ONLY (ruled 2026-09-10, decision
    --     01a08dc2): the hash is the record key, never the reach. A same-hash chunk in a
    --     home the subject does not govern keeps its prose; erasure never reaches beyond
    --     the estate its subject governs. The predicate is HOME-CUSTODY shaped, not
    --     authorship shaped: a resource homed in the subject's governed context but owned
    --     by a grantee is IN the estate (the ruling — the context wipes with the estate)
    --     and its content redacts; a same-hash row in a home the subject does not govern
    --     is never the subject's to erase.
    UPDATE kb_chunk_content cc
       SET content = ''
      FROM kb_chunks c
     WHERE cc.chunk_id = c.id
       AND c.content_hash = ANY(p_hashes)
       AND cc.content <> ''
       AND c.resource_id IN (
           SELECT h.resource_id
             FROM kb_resource_homes h
            WHERE h.anchor_table = 'kb_contexts'
              AND h.anchor_id = ANY(v_estate));

    -- (3) Verbatim block bytes, same shape, same governed reach (the content row resolves
    --     to its resource through the revision chain). The stored hash follows the bytes in the
    --     same statement (20261127000010): the act's record of the hash is the event's
    --     `redacted_hashes` and kb_erased_content, never the emptied row.
    UPDATE kb_block_content
       SET content = '',
           content_hash = encode(sha256(''::bytea), 'hex')
      WHERE content_hash = ANY(p_hashes)
        AND content <> ''
        AND block_revision_id IN (
            SELECT br.id
              FROM kb_block_revisions br
              JOIN kb_content_blocks b ON b.id = br.block_id
             WHERE b.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)));

    -- (4) Embeddings + provenance nulled TOGETHER — `embedding IS NULL` and
    --     `embedded_with IS NULL` can never disagree (20260713000040:84-87). Governed
    --     homes only: a same-hash chunk elsewhere keeps its vector and stays searchable.
    UPDATE kb_chunks
       SET embedding = NULL,
           embedded_with = NULL
      WHERE content_hash = ANY(p_hashes)
        AND embedding IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (4a) The heading trail nulled (20261020100000): header_path is authored heading prose,
    --      so a heading naming the subject is the subject's text like the chunk body (2). The
    --      resource act's D2.1 spelling (NULL, not ''). Same hash, same governed reach: a
    --      same-hash chunk elsewhere keeps its own trail.
    UPDATE kb_chunks
       SET header_path = NULL
      WHERE content_hash = ANY(p_hashes)
        AND header_path IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (5) Search vectors emptied wholesale for every hash-affected GOVERNED resource (the
    --     _rebuild_resource_search_vector site, 20260711000060:74-83): the vector folds
    --     title+body+meta, so a partial redaction would leave redacted terms searchable.
    --     Resources outside the governed homes keep their vectors — their content was
    --     never touched.
    UPDATE kb_resource_search_index si
       SET search_vector = ''
      WHERE si.resource_id IN (
            SELECT DISTINCT c.resource_id FROM kb_chunks c
             WHERE c.content_hash = ANY(p_hashes)
               AND c.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)))
        AND si.search_vector <> '';

    -- (6) The erased-content set (D4): first admit keeps attribution — ON CONFLICT DO
    --     NOTHING is what makes a ledger-order rebuild byte-identical. The set remains the
    --     RECORD of what the act redacted (hash + admitting event); its former role as an
    --     instance-wide write-path oracle is retired (see block_mutate below).
    INSERT INTO kb_erased_content (content_hash, erased_by_event_id)
    SELECT h, p_event FROM unnest(p_hashes) AS h
    ON CONFLICT (content_hash) DO NOTHING;

    -- (6a) The operational identity arms (R3, 20261023100000). Each reaches its rows through the
    --      _erasure_subject_* selectors the plan reads. The invitation selector matches through the
    --      auth links' verified emails, which (9) destroys, so they run here, first. In a replay
    --      namespace these tables are empty, so the arms are no-ops there.
    --
    --      Invitations sent by the subject or addressed to them: the email and the token become
    --      the row's own sentinel (both are UNIQUE-scoped, so the sentinel is per row), and a
    --      pending one is revoked at the act's moment. The row stays: the team keeps its history.
    UPDATE kb_team_invitations i
       SET invited_email = 'erased-' || i.id::text,
           -- Random, not derived from the id: a token is the invitation's bearer secret, and one
           -- anyone could compute from the id would answer the preview door. Nothing replays it.
           token         = 'erased-' || gen_random_uuid()::text,
           revoked_at    = CASE WHEN i.status = 'pending' AND i.revoked_at IS NULL
                                THEN v_occurred ELSE i.revoked_at END
     WHERE i.id IN (SELECT _erasure_subject_invitations(p_subject))
       AND i.invited_email IS DISTINCT FROM 'erased-' || i.id::text;

    --      Login flows: transient, deleted. The refresh tokens are swept after the tombstone (7a).
    DELETE FROM kb_oauth_flow f
     WHERE f.id IN (SELECT _erasure_subject_oauth_flows(p_subject));

    --      The subject's connections (ruled 2026-10-10: homed in the estate, or the subject's own
    --      machine principal). Revoked in temper and the external binding emptied. The broker is not
    --      reached: tokens it already minted are not revoked at the provider (task
    --      01a128a4-5a16-7724-8e57-3dc2f2327f0f), and the record says so.
    UPDATE kb_connections c
       SET revoked_at     = COALESCE(c.revoked_at, v_occurred),
           credential     = NULL,
           observed_reach = NULL
     WHERE c.id IN (SELECT _erasure_subject_connections(p_subject, v_estate))
       AND (c.revoked_at IS NULL OR c.credential IS NOT NULL OR c.observed_reach IS NOT NULL);

    --      The subject's entities' launch metadata (model, platform, persona). The names are (10).
    UPDATE kb_entities en
       SET metadata = '{}'::jsonb
     WHERE en.profile_id = p_subject
       AND en.metadata <> '{}'::jsonb;

    --      Staged upload sessions (R2: staged bytes are content). Segments cascade.
    DELETE FROM kb_blob_uploads u
     WHERE u.id IN (SELECT _erasure_subject_uploads(p_subject, v_estate));

    -- (7) The tombstone (D5): identifiers nulled/sentinel'd, the UUID KEPT — it is the
    --     pseudonym. tombstoned_at is the admitting event's occurred_at, never now().
    UPDATE kb_profiles pr
       SET handle        = v_sentinel,
           display_name  = v_sentinel,
           email         = NULL,
           preferences   = '{}'::jsonb,
           tombstoned_at = COALESCE(pr.tombstoned_at, v_occurred)
      WHERE pr.id = p_subject;

    -- (7a) The subject's refresh tokens: live credentials, revoked and deleted with their replay
    --      evidence, which RESTRICTs a token's delete. When a policy keeps that evidence, the tokens
    --      stay as its referents: revoked, claims emptied, the hash replaced so no presented token
    --      can match one. Before (9), which unclaims the provider ids unbound tokens are matched by.
    --
    --      A rotation racing the act must not leave a successor (2026-10-11). The AS rotates in two
    --      statements with no transaction across them: it retires the presented token, then inserts
    --      the successor. So, in this order:
    --        * After the tombstone (7), whose row lock the successor's insert meets: an insert that
    --          locked the profile first commits before (7) proceeds and is swept below; one that
    --          comes after waits for this act and is refused by kb_oauth_refresh_tokens_refuse_erased
    --          once it sees the tombstone.
    --        * Every chain of the subject is ENDED (kb_oauth_refresh_chain_ends, the replay
    --          responder's marker), so storeRefreshToken and that trigger refuse any successor.
    --        * The sweep runs twice. The trigger locks a successor's predecessor for share, so the
    --          first sweep can wait on an insert in flight; the second statement takes a fresh
    --          snapshot and reaches the successor that committed meanwhile.
    v_chains := ARRAY(SELECT DISTINCT t.chain_id FROM kb_oauth_refresh_tokens t
                       WHERE t.id IN (SELECT _erasure_subject_refresh_tokens(p_subject)));
    INSERT INTO kb_oauth_refresh_chain_ends (chain_id, ended_at)
    SELECT c, v_occurred FROM unnest(v_chains) AS c
    ON CONFLICT (chain_id) DO NOTHING;
    FOR v_pass IN 1 .. 2 LOOP
        IF _erasure_retained_by(v_retained, 'credential-replay-evidence') IS NULL THEN
            DELETE FROM kb_oauth_refresh_replays r
             WHERE r.profile_id = p_subject
                OR r.chain_id = ANY(v_chains)
                OR r.token_id IN (SELECT _erasure_subject_refresh_tokens(p_subject));
            DELETE FROM kb_oauth_refresh_tokens t
             WHERE t.chain_id = ANY(v_chains)
                OR t.id IN (SELECT _erasure_subject_refresh_tokens(p_subject));
        ELSE
            UPDATE kb_oauth_refresh_tokens t
               SET revoked_at = COALESCE(t.revoked_at, v_occurred),
                   claims     = '{}'::jsonb,
                   token_hash = 'erased-' || t.id::text
             WHERE (t.chain_id = ANY(v_chains)
                    OR t.id IN (SELECT _erasure_subject_refresh_tokens(p_subject)))
               AND (t.claims <> '{}'::jsonb OR t.token_hash <> 'erased-' || t.id::text
                    OR t.revoked_at IS NULL);
        END IF;
    END LOOP;

    -- (8) The no-FK external identifiers (D5 blind-spot row 3): they identify the person
    --     through an EXTERNAL system, so the pseudonym break does not cover them — deleted,
    --     the slack_disconnect_service.rs:168,224 precedent. The principals resolve through
    --     the auth-link rows (auth_provider 'slack' — slack_link_service::SLACK_AUTH_PROVIDER).
    --     Since 20261023100000 also through the vault row's own profile_id (R3): a grant can
    --     outlive its link. Intents first, because the principals resolve through the vault.
    DELETE FROM kb_slack_link_intents i
      WHERE i.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));

    DELETE FROM kb_slack_grant_vault v
      WHERE v.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));

    -- (9) The auth-link identifiers (the manifest's identifier | full columns on
    --     kb_profile_auth_links): the email and the IdP's subject id identify the person
    --     DIRECTLY — the pseudonym break does not cover them, the same reasoning as the
    --     Slack stores above. They are unclaimed only HERE, after (8): the Slack-store
    --     deletes resolve their principals THROUGH auth_provider_user_id, so nulling first
    --     would leave the external stores standing over a destroyed lookup. email is
    --     nullable and NULLed; auth_provider_user_id is NOT NULL and sentineled — the D5
    --     nulled/sentinel'd split. THE SENTINEL IS THE ROW'S OWN ID ('erased-' || id): the
    --     kb_profiles derivation generalized, and unique by construction under
    --     kb_profile_auth_links_auth_provider_auth_provider_user_id_key, which two rows of
    --     one provider sharing a subject-keyed sentinel would violate. Every provider's
    --     rows, not just Slack's: the manifest declares the columns, not a provider.
    UPDATE kb_profile_auth_links l
       SET email                 = NULL,
           auth_provider_user_id = 'erased-' || l.id::text
      WHERE l.profile_id = p_subject
        AND (l.email IS NOT NULL OR l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text);

    -- (9a) Free text about the subject (R3), unless a policy keeps the category (R4). After (7):
    --      the standing log admits this update only for a tombstoned subject.
    IF _erasure_retained_by(v_retained, 'standing-reasons') IS NULL THEN
        UPDATE kb_principal_standing_events s
           SET reason = NULL
         WHERE s.profile_id = p_subject AND s.reason IS NOT NULL;
    END IF;
    IF _erasure_retained_by(v_retained, 'join-request-notes') IS NULL THEN
        UPDATE kb_join_requests j
           SET message = NULL, decision_note = NULL
         WHERE j.requesting_profile_id = p_subject
           AND (j.message IS NOT NULL OR j.decision_note IS NOT NULL);
    END IF;
    IF _erasure_retained_by(v_retained, 'review-request-notes') IS NULL THEN
        UPDATE kb_principal_review_requests r
           SET message = NULL, decision_note = NULL
         WHERE r.profile_id = p_subject
           AND (r.message IS NOT NULL OR r.decision_note IS NOT NULL);
    END IF;

    -- (10) The subject's entities' names (manifest identifier | full: an agent-instance
    --      name may embed the person's name). Sentineled to 'erased-' || the entity's own
    --      id — the display_name sentinel derivation (the row's kept UUID is the
    --      pseudonym), and the ONLY per-row spelling that satisfies the
    --      (profile_id, name) UNIQUE grain: two of the subject's entities cannot share
    --      one subject-keyed sentinel. The entity UUIDs stay.
    UPDATE kb_entities en
       SET name = 'erased-' || en.id::text
      WHERE en.profile_id = p_subject
        AND en.name IS DISTINCT FROM 'erased-' || en.id::text;

    -- (11) The subject's own data-artifact content (manifest content | full). Artifacts are
    --      reached only through what points at them, so the reach is the governed-home join:
    --      the resource is homed in one of the subject's OWN personal contexts (the same
    --      owner arm (12) spells) AND the kind is owned by the subject's profile. Emptied,
    --      never deleted — the artifact id, kb_data_artifacts.content_hash and
    --      kb_data_artifact_content.content_hash all stay, keeping the metadata/bytes split's
    --      hash-retention shape (D3). A team-kind or other-profile-kind artifact on the
    --      subject's governed resource is NOT the subject's data — the disposition-iii
    --      remainder the act's targets name, never struck here. NOT admitted to
    --      kb_erased_content: the set is the record of the act's redacted hashes, and no
    --      artifact consult site exists — the tombstone is what keeps the subject from
    --      re-committing.
    UPDATE kb_data_artifact_content dac
       SET content = '{}'::jsonb
      FROM kb_data_artifacts da
      WHERE dac.artifact_id = da.id
        AND da.kind_owner_table = 'kb_profiles'
        AND da.kind_owner_id = p_subject
        AND da.resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate))
        AND dac.content <> '{}'::jsonb;

    -- (12) The centroid sites MARKED for recompute (D5): the formation watermark nulled on
    --     every anchor whose live regions hold an ESTATE member, plus every estate context
    --     (their telos aggregates their goals). Keyed by home since 20261021100000, not by the
    --     act's hashes, which are now the charters' alone: every estate resource is erased,
    --     and resource erasure's own step (7) marks only a resource's home context and the maps
    --     holding it, never another principal's context whose region holds it.
    UPDATE kb_cogmaps m
       SET shape_materialized_event_id = NULL
      WHERE m.shape_materialized_event_id IS NOT NULL
        AND m.id IN (
            SELECT r.home_anchor_id
              FROM kb_cogmap_regions r
              JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
             WHERE r.home_anchor_table = 'kb_cogmaps'
               AND NOT r.is_folded
               AND mem.member_table = 'kb_resources'
               AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)));

    UPDATE kb_contexts c
       SET shape_materialized_event_id = NULL
      WHERE c.shape_materialized_event_id IS NOT NULL
        AND ( c.id IN (
                SELECT r.home_anchor_id
                  FROM kb_cogmap_regions r
                  JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                 WHERE r.home_anchor_table = 'kb_contexts'
                   AND NOT r.is_folded
                   AND mem.member_table = 'kb_resources'
                   AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)))
            -- Every estate context, the personal team's included (R1).
            OR c.id = ANY(v_estate) );

    -- (13) The custody closure is no longer here. Since 20261022100000 the act appends one
    --      context_erased per estate context after this body, which retires the context and
    --      replaces its name and slug with sentinels, and replay re-applies it at its position.
END;
$function$;

-- The rows redactions already emptied: their hash becomes the hash of what they now hold.
UPDATE kb_block_content
   SET content_hash = encode(sha256(''::bytea), 'hex')
 WHERE content = ''
   AND content_hash <> encode(sha256(''::bytea), 'hex');

ALTER TABLE kb_block_content
    ADD CONSTRAINT ck_kb_block_content_emptied_hash
        CHECK (content <> '' OR content_hash = encode(sha256(''::bytea), 'hex')) NOT VALID;
ALTER TABLE kb_block_content VALIDATE CONSTRAINT ck_kb_block_content_emptied_hash;

COMMENT ON CONSTRAINT ck_kb_block_content_emptied_hash ON kb_block_content IS
    'An emptied row (content = '''') carries sha256(''''), never the hash of the bytes it held. Every '
    'redaction sets both in one UPDATE (20261127000010); this refuses a path that forgets the hash.';

COMMENT ON COLUMN kb_block_content.content_hash IS
    'Bare sha256 hex of content''s bytes (the Rust sha256_hex twin). Served as bytes_sha256 on the '
    '/blocks read. A redaction empties content and recomputes this in the same statement, so it never '
    'names erased text.';

SELECT declare_migration(
    20261127000010,
    'additive',
    'Replaces _resource_erasure_apply_redaction and _erasure_apply_redaction (signatures unchanged) so '
    'the UPDATE that empties kb_block_content.content also sets content_hash to sha256(''''); backfills '
    'rows already emptied; adds a CHECK that an emptied row carries that hash; comments the column. No '
    'column, type or result shape moves. A binary without this change never writes an empty block '
    'with another hash (only these two functions empty a row), so it is unaffected; the only observable '
    'change is that an erased row''s stored hash no longer names its old text.'
);
