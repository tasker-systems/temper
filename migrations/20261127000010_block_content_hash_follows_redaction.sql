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
--   * THIS IS THE STORED BLOCK-BYTES HASH ONLY. Chunk hashes (kb_chunks.content_hash, and the chunk
--     manifests on the ledger) are kept by design (resource erasure spec D2/D3: "hashes are kept
--     everywhere, so the merkles stay valid"), and for a block whose bytes are one already-trimmed
--     chunk a chunk hash equals the block-bytes hash. block_body_hash / body_hash are merkles over
--     those chunk hashes, not over this column, so they stay valid. No hash enters
--     kb_erased_content from a resource act (D12 rule 0), and this migration adds none. The
--     block-bytes hash itself never rides an event (BlockContent rides the sidecar only), and
--     replay re-supplies sidecars from the post-erasure rows, so replay stays byte-identical.
--   * AN EMPTIED ROW IS NOT A HASH TO ADMIT. principal_erasure_survey_plan gathers the charters'
--     block-bytes hashes into the act's redacted set and lists the subject's unstruck text hashes;
--     both now skip rows already emptied, so sha256('') never enters kb_erased_content (where it
--     would refuse every empty-block write) or a survey as "text". resource_erasure_survey_plan's
--     record of the act no longer says the block hashes were kept.
--   * THE BACKFILL IS EXACT: a row whose content is '' and whose hash is not sha256('') is a row a
--     redaction emptied. Every other writer hashes the bytes it stores (BlockContent::of).
--   * WRITERS DRAIN FIRST. The table is locked SHARE ROW EXCLUSIVE before anything else, so an act
--     already writing commits before the backfill reads, and none writes between the backfill and
--     the CHECK. An act blocked mid-UPDATE on that lock when this commits finishes with the old
--     body's statement and is refused by the CHECK (23514); it changed nothing and is re-run.
--
-- The four bodies are their live definitions (pg_get_functiondef: _resource_erasure_apply_redaction
-- after 20261015100020, _erasure_apply_redaction and principal_erasure_survey_plan after
-- 20261023100000, resource_erasure_survey_plan after 20261009100000), changed only as marked.
-- Witnesses: temper-substrate tests/block_content_hash_constraint.rs (the backfill and the CHECK),
-- resource_erasure_act.rs and block_history_scrub.rs (the redactions), and temper-services
-- erasure_service.rs (the person act).

LOCK TABLE kb_block_content IN SHARE ROW EXCLUSIVE MODE;

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
    v_redacted      jsonb := '[]'::jsonb;
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
            'outcome', v_a || ' block revision bodies emptied, their stored hashes recomputed over the empty bytes');
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
    -- D2 step (9f): the artifact families.
    SELECT count(*) INTO v_a FROM kb_data_artifacts da
     WHERE da.resource_id = p_resource
       AND da.artifact_kind <> 'erased:' || da.asserted_by_event_id::text;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifacts.artifact_kind',
            'outcome', v_a || ' artifact families set to erased:<asserted_by_event_id>');
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

    -- 6. Shapes in R's home for a family R's artifacts used (D4, ruled 2026-10-03, ruling 5;
    --    20261008100000). A shape's artifact_kind is the home's declaration of a family, not R's
    --    instance, so the act never touches it (Q1), and it may still name what R's artifacts
    --    were. Named by shape id, live shapes only. Matched on (home, kind owner, family), the
    --    family read from the artifact's row OR from the data_artifact_committed event that
    --    asserted it: step (9f) erases the row's family, so a plan computed on the husk (the
    --    survey after the act, a cut-1 husk the backfill below rewrote, cut 2's completion pass
    --    re-deriving) still finds the shapes through the ledger until cut 2 redacts that payload.
    --    Only R's current home: a shape in a home R was rehomed out of is not named.
    FOR v_row IN
        SELECT DISTINCT s.id
          FROM kb_data_artifacts da
          JOIN kb_events ev ON ev.id = da.asserted_by_event_id
          JOIN kb_resource_homes h ON h.resource_id = da.resource_id
          JOIN kb_data_artifact_shapes s
               ON NOT s.is_folded
              AND s.home_anchor_table = h.anchor_table AND s.home_anchor_id = h.anchor_id
              AND s.kind_owner_table = da.kind_owner_table AND s.kind_owner_id = da.kind_owner_id
              AND (s.artifact_kind = da.artifact_kind
                OR s.artifact_kind = ev.payload ->> 'artifact_kind')
         WHERE da.resource_id = p_resource
         ORDER BY s.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_data_artifact_shapes',
            'outcome', 'shape ' || v_row.id::text
                       || ' in the home declares a family the resource''s artifacts used; the home''s, never touched (Q1)');
    END LOOP;

    -- R's home context, whether R was ever a goal, and R's genesis — inputs to the telos arm
    -- below. Goal-ness is read from the LEDGER, not from kb_properties: step 9 sentinels and folds
    -- R's doc_type row, so a plan computed after the act (the survey on the husk, cut 2's
    -- completion pass re-deriving) would otherwise drop every telos copy without error. "Ever" —
    -- the created doc_type or any doc_type property event, normalized as the projection
    -- normalizes it — so a goal later re-typed still names the snapshots it contributed to.
    -- Cut 2 must read this before it redacts R's trail, or from the recorded resource_erased: both
    -- arms read paths this plan's own CASE names for redaction (`property_key` and `value`, and
    -- since 20261008100000 the created arm's `doc_type` too).
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
           AND _property_value_normalized('doc_type', ev.payload -> 'value') #>> '{}' = 'goal')
      -- After cut 2 the act redacts both arms above, so a plan on the husk reads goal-ness from
      -- the act's own record: it named telos copies only when R was a goal (20261009100000).
      OR EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         CROSS JOIN LATERAL jsonb_array_elements(ev.payload -> 'ledger_remainder') lr
         WHERE et.name = 'resource_erased' AND ev.payload ->> 'subject_id' = p_resource::text
           AND lr -> 'paths' ? 'telos_centroid');
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

    -- ── THE LEDGER REDACTION (D3, D4; 20261009100000): every path of R's own trail events the act
    --    rewrites to its class sentinel, in exactly the shape `RedactedEventFields` uses —
    --    {event, paths}. One definition, _resource_erasure_payload_redaction, which the act runs
    --    for the payloads it writes. A path already at its sentinel is not listed, so on a husk
    --    erased under cut 2 this is empty. On a husk erased under cut 1 it is NOT yet the
    --    completion pass's list: step (9e) has re-pointed that husk's provenance, so the remote-
    --    source numbering this reads finds no original and the derivation raises. The completion
    --    pass must number those sources from the ledger. Paths only, never values.
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redacted
      FROM _resource_erasure_payload_redaction(p_resource) d;

    -- ── THE LEDGER REMAINDER (D12): the ledger paths carrying R's content that the exception
    --    cannot reach, in the same {event, paths} shape. Since cut 2 the act reaches R's whole
    --    trail, so what remains is the telos copies below, anchored on the context (D3's
    --    exception never crosses the resource boundary).
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', s.event_id, 'paths', s.paths)
                               ORDER BY s.event_id), '[]'::jsonb)
      INTO v_ledger
      FROM (
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
        SELECT ev.id AS event_id, '["telos_centroid"]'::jsonb AS paths
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
        'redacted_fields', v_redacted,
        'ledger_remainder', v_ledger);
END;
$function$;

CREATE OR REPLACE FUNCTION public.principal_erasure_survey_plan(p_subject uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_exists     uuid;
    v_governed   uuid[] := '{}';
    v_resources  uuid[] := '{}';
    v_hashes     text[] := '{}';
    v_blob_hash  text[] := '{}';
    v_targets    jsonb  := '[]'::jsonb;
    v_strikes    jsonb  := '[]'::jsonb;
    v_row        record;
    v_rel        boolean;
    v_struck     text[] := '{}';
    v_team  record;
    v_n     integer;
    v_text_rem text;
    v_prof  kb_profiles%ROWTYPE;
    v_charters   uuid[] := '{}';
    v_resrows    jsonb  := '[]'::jsonb;
    v_disp       text;
    v_counts     jsonb  := jsonb_build_object('erase', 0, 'complete', 0, 'skip', 0, 'charter', 0);
    v_ctx_erase  uuid[] := '{}';
    v_ctx_fields jsonb  := '[]'::jsonb;
    v_trail      uuid[] := '{}';
    v_retained   jsonb  := _erasure_retention_snapshot();
    v_kept       text;
BEGIN
    SELECT id INTO v_exists FROM kb_profiles WHERE id = p_subject;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: subject % not found', p_subject;
    END IF;

    -- The estate contexts (R1, ruled 2026-10-10): the subject's @me contexts and their personal
    -- team's contexts, from _erasure_estate_contexts, the ONE definition. Other team contexts and
    -- map homes are NOT the estate (disposition iii). The act records this set in its event and
    -- the redaction reads it from there, so the arms no longer re-derive it.
    v_governed := ARRAY(SELECT _erasure_estate_contexts(p_subject));

    -- ── Scope: the estate, HOME-PURE (ruled 2026-09-11 with Pete — the "scope of
    -- engagement" ruling) ─────────────────────────────────────────────────────────────
    -- Everything homed in a governed context wipes with the estate, whoever authored,
    -- owns or emitted it: writing into someone's PRIVATE context under a grant declares
    -- the content's scope of engagement — it lives and dies with that estate (the
    -- terms-of-use documentation states this). The 20260909000025 actor halves are
    -- dissolved: the owner/originator OR kept reassigned-in-place authorship in reach
    -- (20260703140000 moves owner_profile_id in place), and the emitted half was already
    -- subsumed by the governed-home EXISTS it carried. Erasure keeps the resource rows
    -- live (D3), so a grantee whose prose dies with the estate holds their remaining
    -- access only against a retired context (arm 13), and their attribution dies by the
    -- pseudonym break, never by edits.
    SELECT coalesce(array_agg(DISTINCT h.resource_id), '{}') INTO v_resources
      FROM kb_resource_homes h
     WHERE h.anchor_table = 'kb_contexts'
       AND h.anchor_id = ANY(v_governed);

    -- ── Each estate resource's disposition, in resource-id order: the order the act erases them
    --    in (2c), which is also the order every person act takes their locks in.
    --      charter  — a cogmap's telos. Resource erasure refuses it; the act empties its text
    --                 by hash below and holds it until map-grain erasure (2a).
    --      erase    — live or soft-deleted: resource erasure runs on it (2b).
    --      complete — erased, and the ledger still holds paths to rewrite (an erasure from
    --                 before cut 2): resource erasure's completion pass runs (2f).
    --      skip     — erased and complete (2f). ──────────────────────────────────────────────
    FOR v_row IN
        SELECT r.id,
               r.erased_at IS NOT NULL AS erased,
               EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id) AS charter
          FROM kb_resources r
         WHERE r.id = ANY(v_resources)
         ORDER BY r.id
    LOOP
        v_disp := CASE
            WHEN v_row.charter THEN 'charter'
            WHEN NOT v_row.erased THEN 'erase'
            WHEN resource_erasure_completion_fields(v_row.id) <> '[]'::jsonb THEN 'complete'
            ELSE 'skip'
        END;
        IF v_disp = 'charter' THEN
            v_charters := v_charters || v_row.id;
        END IF;
        v_resrows := v_resrows || jsonb_build_array(jsonb_build_object(
            'resource_id', v_row.id, 'disposition', v_disp));
        v_counts := jsonb_set(v_counts, ARRAY[v_disp], to_jsonb((v_counts->>v_disp)::integer + 1));
    END LOOP;

    -- The text hashes the act empties BY HASH (D5, ruled 2026-10-10): the charters' chunk and
    -- verbatim block content hashes. Every other estate resource is emptied row by row by
    -- resource erasure, which admits no hash to kb_erased_content.
    SELECT coalesce(array_agg(DISTINCT h), '{}') INTO v_hashes FROM (
        SELECT c.content_hash AS h FROM kb_chunks c
         WHERE c.resource_id = ANY(v_charters)
        UNION
        SELECT bc.content_hash AS h
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
         WHERE b.resource_id = ANY(v_charters)
           AND bc.content <> ''
    ) s;

    -- ── The blob pass: HOME-PURE (the 2026-09-12 ruling), WOULD-STRIKE structured ───────
    -- Scope = every blob row homed in a governed context, WHOEVER committed it, plus the
    -- subject's own rows (owner/originator/emitted) wherever they home — the subject's
    -- team- and map-homed rows stay the named remainder (disposition iii). Governed-home
    -- LIVE rows report would-strike entries; governed-home struck rows report
    -- already-erased; EVERY other home is the named remainder. released_would_be SIMULATES
    -- the act's own sequential refcount: the ONE blob_delete live-row predicate (live rows
    -- with the hash, this one included, <= 1) minus the same-hash rows EARLIER IN THE
    -- PLAN'S OWN STRIKE SET — execute consumes the entries in order, so those siblings are
    -- already emptied when the act reaches this row. Run at survey time WITHOUT the hash's
    -- advisory lock: a PREDICTION, honest about the moment it ran; the wrapper's
    -- strike-time verdict is authoritative in the act. The enumeration is ORDERED BY id —
    -- the act strikes in plan order and the survey must walk the SAME sequence when it
    -- simulates it, so the order cannot be left to the scan.
    FOR v_row IN
        SELECT b.id, b.content_hash, b.blob_pathname, b.content_type,
               (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed)) AS governed_home
          FROM kb_blobs b
         WHERE (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed))
            OR b.owner_profile_id = p_subject
            OR b.originator_profile_id = p_subject
            OR b.asserted_by_event_id IN (
               SELECT e.id FROM kb_events e
                 JOIN kb_entities en ON en.id = e.emitter_entity_id
                WHERE en.profile_id = p_subject)
          ORDER BY b.id
    LOOP
        IF v_row.governed_home AND v_row.content_type IS NOT NULL THEN
            -- The act's own SEQUENTIAL refcount, simulated: the strike loop consumes the
            -- plan's rows by id IN ORDER and each blob_delete counts live rows at ITS
            -- moment, so every same-hash row already in this plan's strike set is emptied
            -- when the act reaches this row — subtract it (accumulated below, in plan
            -- order) from the ONE live-row predicate. Two same-hash rows in the estate
            -- therefore predict released=false then released=true, exactly as the act
            -- strikes them.
            v_rel := ((SELECT count(*) FROM kb_blobs live
                        WHERE live.content_hash = v_row.content_hash
                          AND live.content_type IS NOT NULL)
                      - (SELECT count(*) FROM unnest(v_struck) prior
                          WHERE prior = v_row.content_hash)) <= 1;
            v_struck := v_struck || ARRAY[v_row.content_hash];
            v_blob_hash := v_blob_hash || ARRAY[v_row.content_hash];
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'would_strike', jsonb_build_object(
                    'blob_id',           v_row.id,
                    'content_hash',      v_row.content_hash,
                    'pathname',          v_row.blob_pathname,
                    'released_would_be', v_rel)));
            v_strikes := v_strikes || jsonb_build_array(jsonb_build_object(
                'blob_id',           v_row.id,
                'released_would_be', v_rel));
        ELSIF v_row.governed_home THEN
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'already-erased'));
        ELSE
            -- The named remainder: independent_obligation-shaped, hash named (D2 vocabulary),
            -- never admitted to the erased-content set.
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'independent_obligation: home governed by a team or map; '
                           || 'hash ' || v_row.content_hash || ' not struck'));
        END IF;
    END LOOP;

    v_hashes := v_hashes || v_blob_hash;

    -- ── Per-target outcomes, read against the PRE-redaction state (the mutations live in
    -- _erasure_apply_redaction alone; these reads only report what it will find).
    -- 20260911000000: every read below scopes to GOVERNED homes with the same predicate
    -- the redaction spells — the record reports "erased" only for rows the act actually
    -- redacts; a same-hash row in an ungoverned home is nobody's outcome (it is not the
    -- subject's data and was never a target). ────────────────────────────────────────────
    SELECT * INTO v_prof FROM kb_profiles WHERE id = p_subject;
    IF v_prof.tombstoned_at IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','already-erased'));
    ELSE
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','tombstoned'));
        IF v_prof.email IS NOT NULL THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.email','outcome','erased'));
        END IF;
        IF v_prof.preferences <> '{}'::jsonb THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.preferences','outcome','erased'));
        END IF;
    END IF;

    SELECT * INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;
    IF FOUND THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_teams.slug',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END),
            jsonb_build_object('target','kb_teams.name',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END));
    END IF;

    -- The Slack stores, matched through a Slack auth link or the vault row's own profile_id (R3):
    -- a vault row can outlive its link.
    SELECT count(*) INTO v_n FROM kb_slack_grant_vault v
     WHERE v.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_grant_vault.slack_principal_id',
                'outcome','deleted'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_slack_link_intents i
     WHERE i.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_link_intents.slack_principal_id',
                'outcome','deleted'));
    END IF;

    -- The auth-link identifiers, the entity names, and the subject's own artifact content —
    -- outcomes read against the PRE-redaction state exactly like every arm above (the
    -- mutations live in _erasure_apply_redaction alone; these reads only report what it
    -- will find). The artifact remainder subset is named with the disposition-iii
    -- vocabulary the blob arm set: another principal's kind namespace on a governed
    -- resource is an independent obligation, re-named on every run because it stands.
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject AND l.email IS NOT NULL;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.email',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject
       AND l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.auth_provider_user_id',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject
       AND en.name IS DISTINCT FROM 'erased-' || en.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.name',
                'outcome','sentinel-scrubbed'));
    END IF;

    -- ── The operational identity arms (R3), read against the pre-redaction state through the
    --    same selectors the redaction uses. Each is claimed only when the act will change a row, so
    --    a re-run omits it. ──────────────────────────────────────────────────────────────────────
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject AND en.metadata <> '{}'::jsonb;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.metadata', 'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_team_invitations i
     WHERE i.id IN (SELECT _erasure_subject_invitations(p_subject))
       AND i.invited_email IS DISTINCT FROM 'erased-' || i.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_team_invitations.invited_email',
                'outcome', v_n || ' invitation(s) sent by or to the subject; sentinel-scrubbed, '
                           || 'token replaced, pending ones revoked'));
    END IF;
    -- Invitations to an address the subject holds only unverified: perhaps theirs, perhaps not, so
    -- never touched, and named by count.
    SELECT count(*) INTO v_n FROM kb_team_invitations i
     WHERE i.invited_email IS DISTINCT FROM 'erased-' || i.id::text
       AND i.id NOT IN (SELECT _erasure_subject_invitations(p_subject))
       AND lower(i.invited_email COLLATE "C") IN (
           SELECT lower(l.email COLLATE "C") FROM kb_profile_auth_links l
            WHERE l.profile_id = p_subject AND l.email IS NOT NULL AND NOT l.email_verified);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_team_invitations.invited_email (unverified address)',
                'outcome', 'independent_obligation: ' || v_n || ' invitation(s) to an address the '
                           || 'subject never verified; may be another person''s; not reached'));
    END IF;
    -- Unbound refresh tokens are reached by the login provider the doors pass. Without it they
    -- cannot be attributed, so say so rather than imply none exist.
    IF _erasure_auth_provider() IS NULL
       AND EXISTS (SELECT 1 FROM kb_oauth_refresh_tokens t
                    WHERE t.profile_id IS NULL AND t.revoked_at IS NULL) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_tokens (unbound)',
                'outcome', 'not checked: the act was not told the login provider, so tokens never '
                           || 'bound to a profile cannot be attributed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_oauth_flow f
     WHERE f.id IN (SELECT _erasure_subject_oauth_flows(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_flow', 'outcome', v_n || ' flow(s) deleted'));
    END IF;
    v_kept := _erasure_retained_by(v_retained, 'credential-replay-evidence');
    SELECT count(*) INTO v_n FROM kb_oauth_refresh_tokens t
     WHERE t.id IN (SELECT _erasure_subject_refresh_tokens(p_subject))
       AND (v_kept IS NULL OR t.claims <> '{}'::jsonb);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_tokens',
                'outcome', CASE WHEN v_kept IS NULL
                                THEN v_n || ' refresh token(s) revoked and deleted'
                                ELSE v_n || ' refresh token(s) revoked; claims and hash erased' END));
    END IF;
    SELECT count(*) INTO v_n FROM kb_oauth_refresh_replays r
     WHERE r.profile_id = p_subject
        OR r.token_id IN (SELECT _erasure_subject_refresh_tokens(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_replays',
                'outcome', CASE WHEN v_kept IS NULL THEN v_n || ' replay record(s) deleted'
                                ELSE 'retained under policy ' || v_kept END));
    END IF;
    SELECT count(*) INTO v_n FROM kb_connections c
     WHERE c.id IN (SELECT _erasure_subject_connections(p_subject, v_governed))
       AND (c.revoked_at IS NULL OR c.credential IS NOT NULL OR c.observed_reach IS NOT NULL);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_connections.credential',
                'outcome', v_n || ' connection(s) revoked in temper; credential and observed reach '
                           || 'erased; tokens already minted at the provider are not revoked there'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_blob_uploads u
     WHERE u.id IN (SELECT _erasure_subject_uploads(p_subject, v_governed));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_blob_upload_segments.bytes',
                'outcome', v_n || ' staged upload(s) deleted'));
    END IF;
    -- The free text about the subject: erased unless a policy keeps its category (R4).
    SELECT count(*) INTO v_n FROM kb_principal_standing_events s
     WHERE s.profile_id = p_subject AND s.reason IS NOT NULL;
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'standing-reasons');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_principal_standing_events.reason',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    SELECT count(*) INTO v_n FROM kb_join_requests j
     WHERE j.requesting_profile_id = p_subject
       AND (j.message IS NOT NULL OR j.decision_note IS NOT NULL);
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'join-request-notes');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_join_requests.message',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    SELECT count(*) INTO v_n FROM kb_principal_review_requests r
     WHERE r.profile_id = p_subject
       AND (r.message IS NOT NULL OR r.decision_note IS NOT NULL);
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'review-request-notes');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_principal_review_requests.message',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    -- The ledger copies a policy keeps: named, never rewritten.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_events.payload (' || k.cls || ')',
                   'outcome', 'retained under policy ' || k.policy) ORDER BY k.cls)
          FROM (SELECT DISTINCT
                       CASE t.name WHEN 'principal_standing_changed'   THEN 'standing reasons'
                                   WHEN 'principal_governance_changed' THEN 'governance reasons'
                                   ELSE 'erasure-refusal details' END AS cls,
                       _erasure_retained_by(v_retained,
                           _principal_erasure_path_category(t.name, p.path)) AS policy
                  FROM kb_events e
                  JOIN kb_event_types t ON t.id = e.event_type_id
                  JOIN _principal_erasure_redact_paths() p ON p.event_type = t.name
                 WHERE t.name IN ('principal_standing_changed', 'principal_governance_changed',
                                  'principal_erasure_refused')
                   AND e.payload ->> 'subject_table' = 'kb_profiles'
                   AND e.payload ->> 'subject_id' = p_subject::text
                   AND jsonb_typeof(e.payload -> p.path) = 'string') k
         WHERE k.policy IS NOT NULL), '[]'::jsonb);
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE da.kind_owner_table = 'kb_profiles'
       AND da.kind_owner_id = p_subject
       AND dac.content <> '{}'::jsonb
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE dac.content <> '{}'::jsonb
       AND (da.kind_owner_table <> 'kb_profiles' OR da.kind_owner_id <> p_subject)
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content (other-kind)',
                'outcome','independent_obligation: the artifact''s kind is owned by '
                           || 'another principal; content on a governed resource not '
                           || 'struck'));
    END IF;

    IF EXISTS (SELECT 1 FROM kb_chunk_content cc
                 JOIN kb_chunks c ON c.id = cc.chunk_id
                WHERE c.content_hash = ANY(v_hashes)
                  AND c.resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunk_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               JOIN kb_chunks c ON c.id = cc.chunk_id
                                              WHERE c.content_hash = ANY(v_hashes)
                                                AND cc.content <> ''
                                                AND c.resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_block_content
                WHERE content_hash = ANY(v_hashes)
                  AND block_revision_id IN (
                      SELECT br.id FROM kb_block_revisions br
                        JOIN kb_content_blocks b ON b.id = br.block_id
                       WHERE b.resource_id IN (
                             SELECT h.resource_id FROM kb_resource_homes h
                              WHERE h.anchor_table = 'kb_contexts'
                                AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_block_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_block_content
                                              WHERE content_hash = ANY(v_hashes)
                                                AND content <> ''
                                                AND block_revision_id IN (
                                                    SELECT br.id FROM kb_block_revisions br
                                                      JOIN kb_content_blocks b ON b.id = br.block_id
                                                     WHERE b.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed))))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    -- The text remainder (the attribution ruling, 2026-09-13, on the 2026-09-06 clause):
    -- prose ATTRIBUTED to the subject by authorship — content blocks whose genesis event
    -- the subject's entity emitted — in homes outside the governed estate (team and map
    -- alike). Named for audit with count + hashes (capped at 8, "and N more" beyond),
    -- never struck, never admitted to v_hashes: nothing in a shared home is the act's to
    -- strike, and the redaction's reads stay governed-scoped. Attribution is the emitting
    -- entity, the blob arm's own emitted-half shape; direct authorship only — carried
    -- attribution is provenance the pseudonym break already covers.
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT bc.content_hash) || ' text hash(s) not struck'
               || CASE WHEN count(DISTINCT bc.content_hash) <= 8
                       THEN ': ' || array_to_string(array_agg(DISTINCT bc.content_hash), ', ')
                       ELSE ': ' || array_to_string(ARRAY(
                                   SELECT DISTINCT bc2.content_hash
                                     FROM kb_block_content bc2
                                     JOIN kb_block_revisions br2 ON br2.id = bc2.block_revision_id
                                     JOIN kb_content_blocks b2 ON b2.id = br2.block_id
                                     JOIN kb_events ge2 ON ge2.id = b2.genesis_event_id
                                     JOIN kb_entities ge2_en ON ge2_en.id = ge2.emitter_entity_id
                                    WHERE ge2_en.profile_id = p_subject
                                      AND bc2.content <> ''
                                      AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg2
                                                       WHERE hg2.resource_id = b2.resource_id
                                                         AND hg2.anchor_table = 'kb_contexts'
                                                         AND hg2.anchor_id = ANY(v_governed))
                                    ORDER BY 1 LIMIT 8), ', ')
                            || '; and ' || (count(DISTINCT bc.content_hash) - 8) || ' more'
                  END
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
          JOIN kb_events ge ON ge.id = b.genesis_event_id
          JOIN kb_entities ge_en ON ge_en.id = ge.emitter_entity_id
         WHERE ge_en.profile_id = p_subject
           AND bc.content <> ''
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = b.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
        HAVING count(DISTINCT bc.content_hash) > 0);
    IF v_text_rem IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_block_content.content',
            'outcome', v_text_rem));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.embedding',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END),
            jsonb_build_object('target','kb_chunks.embedded_with',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    -- The heading trail, arm (4a) of the redaction (20261020100000). Claimed only when the act
    -- will null one: a chunk with no heading trail never held one, so "already-erased" would
    -- claim an act that never happened. A re-run of the act therefore omits the target.
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND header_path IS NOT NULL
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.header_path', 'outcome','erased'));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_resource_search_index si
                WHERE si.resource_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                          WHERE c.content_hash = ANY(v_hashes)
                                            AND c.resource_id IN (
                                                SELECT h.resource_id FROM kb_resource_homes h
                                                 WHERE h.anchor_table = 'kb_contexts'
                                                   AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_resource_search_index.search_vector',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_resource_search_index si
                                              WHERE si.resource_id IN (
                                                    SELECT DISTINCT c.resource_id FROM kb_chunks c
                                                     WHERE c.content_hash = ANY(v_hashes)
                                                       AND c.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed)))
                                              AND si.search_vector <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources'
              AND mem.member_id = ANY(v_resources));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    -- The estate contexts the act erases (R7, ruled 2026-10-10): every one without a
    -- context_erased event yet. Each gets one, which retires it and replaces its name and slug with
    -- the sentinels; a re-run finds none left. The context rows' earlier names and slugs on the
    -- ledger (context_renamed, context_retired, context_restored) are rewritten to the same
    -- sentinels under the act's own authority: redacted_fields, paths only.
    v_ctx_erase := _principal_erasure_contexts_to_erase(v_governed);
    IF cardinality(v_ctx_erase) > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.is_active','outcome','retired'),
            jsonb_build_object('target','kb_contexts.name',     'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_contexts.slug',     'outcome','sentinel-scrubbed'));
    END IF;
    v_ctx_fields := _principal_erasure_redacted_fields(p_subject, v_governed, v_retained);
    SELECT count(*) INTO v_n
      FROM jsonb_array_elements(v_ctx_fields) f
      JOIN kb_events e ON e.id = (f ->> 'event')::uuid
     WHERE e.producing_anchor_table = 'kb_contexts';
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_events.payload',
            'outcome', v_n
                       || ' context event(s) naming an estate context; names and slugs sentinel-scrubbed'));
    END IF;
    -- The subject's own admin events (R3): standing and governance reasons, and Slack ids.
    v_n := jsonb_array_length(v_ctx_fields) - v_n;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_events.payload',
            'outcome', v_n
                       || ' admin event(s) about the subject; reasons and refusal details erased and Slack ids sentinel-scrubbed'));
    END IF;

    -- What the subject wrote outside the estate (R8, disposition iii): it stays, and the record
    -- names it by count per carrier class, never by content. A carrier is one of the subject's own
    -- ledger events (its emitting entity is theirs) still holding the text: outside the trail of
    -- every estate resource, which resource erasure rewrites, and with no redaction row at the
    -- path, which an earlier erasure or scrub already rewrote.
    v_trail := ARRAY(SELECT DISTINCT ts.event_id
                       FROM unnest(v_resources) AS er(id),
                            LATERAL _resource_erasure_trail_scope(er.id) ts);
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_events.payload (' || c.cls || ')',
                   'outcome', 'independent_obligation: authored by the subject outside the estate; '
                              || c.n || ' event(s) kept')
                   ORDER BY c.cls)
          FROM (SELECT s.cls, count(*) AS n
                  FROM (SELECT CASE
                                 WHEN t.name IN ('resource_created', 'resource_updated')
                                      AND jsonb_typeof(e.payload -> 'title') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'title')
                                   THEN 'titles'
                                 -- A doc_type value is a type name, not authored text.
                                 WHEN t.name IN ('property_set', 'property_asserted')
                                      AND e.payload ? 'value'
                                      AND e.payload ->> 'property_key' IS DISTINCT FROM 'doc_type'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'value')
                                   THEN 'property values'
                                 WHEN t.name = 'relationship_asserted'
                                      AND NOT _erasure_label_is_kept(e.payload -> 'label')
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'label')
                                   THEN 'edge labels'
                                 WHEN t.name = 'citation_audited'
                                      AND jsonb_typeof(e.payload -> 'reason') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'reason')
                                   THEN 'citation-audit reasons'
                                 -- A rename the subject made of a context outside the estate: the
                                 -- estate's own are rewritten by this act.
                                 WHEN t.name = 'context_renamed'
                                      AND NOT (e.producing_anchor_id = ANY(v_governed))
                                      AND jsonb_typeof(e.payload -> 'to_name') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'to_name')
                                   THEN 'context names'
                               END AS cls
                          FROM kb_events e
                          JOIN kb_event_types t ON t.id = e.event_type_id
                          JOIN kb_entities en ON en.id = e.emitter_entity_id
                         WHERE en.profile_id = p_subject
                           AND t.name IN ('resource_created', 'resource_updated', 'property_set',
                                          'property_asserted', 'relationship_asserted',
                                          'citation_audited', 'context_renamed')
                           AND NOT (e.id = ANY(v_trail))) s
                 WHERE s.cls IS NOT NULL
                 GROUP BY s.cls) c), '[]'::jsonb);
    SELECT count(*) INTO v_n FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND ( c.id IN (
               SELECT r.home_anchor_id FROM kb_cogmap_regions r
                 JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                WHERE r.home_anchor_table = 'kb_contexts' AND NOT r.is_folded
                  AND mem.member_table = 'kb_resources'
                  AND mem.member_id = ANY(v_resources))
          -- Every estate context (R1).
          OR c.id = ANY(v_governed) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    -- The charters held (2a), and the resources already complete (2f), named in the record.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_resources',
                   'outcome', 'charter ' || ch || '; held until map-grain erasure (task '
                              || '01a0e960-0ca2-7f42-b33e-1ed19b024e6b); text emptied; title, '
                              || 'properties, edges and ledger kept') ORDER BY ch)
          FROM unnest(v_charters) AS ch), '[]'::jsonb);
    IF (v_counts->>'skip')::integer > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', (v_counts->>'skip') || ' estate resource(s) already erased and complete; skipped'));
    END IF;

    RETURN jsonb_build_object(
        'estate_contexts', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_governed) AS g),
        'estate',          v_counts || jsonb_build_object('total', jsonb_array_length(v_resrows)),
        'resources',       v_resrows,
        'charters',        (SELECT coalesce(jsonb_agg(ch ORDER BY ch), '[]'::jsonb) FROM unnest(v_charters) AS ch),
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL),
        'blob_strikes',    v_strikes,
        'contexts_to_erase', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_ctx_erase) AS g),
        'redacted_fields',   v_ctx_fields,
        'retained',          v_retained);
END;
$function$;

-- The rows redactions already emptied: their hash becomes the hash of what they now hold.
-- BACKFILL BEGIN
UPDATE kb_block_content
   SET content_hash = encode(sha256(''::bytea), 'hex')
 WHERE content = ''
   AND content_hash <> encode(sha256(''::bytea), 'hex');
-- BACKFILL END

-- One validating ADD under the lock above: a NOT VALID / VALIDATE split buys nothing inside one
-- transaction. The scan compares each row's content with '', which needs no detoast.
-- CHECK BEGIN
ALTER TABLE kb_block_content
    ADD CONSTRAINT ck_kb_block_content_emptied_hash
        CHECK (content <> '' OR content_hash = encode(sha256(''::bytea), 'hex'));
-- CHECK END

COMMENT ON CONSTRAINT ck_kb_block_content_emptied_hash ON kb_block_content IS
    'An emptied row (content = '''') carries sha256(''''), never the hash of the bytes it held. Every '
    'redaction sets both in one UPDATE (20261127000010); this refuses a path that forgets the hash.';

COMMENT ON COLUMN kb_block_content.content_hash IS
    'Bare sha256 hex of content''s bytes (the Rust sha256_hex twin). Served as bytes_sha256 on reads. '
    'A redaction empties content and recomputes this in the same statement, so it never names '
    'erased text. Chunk hashes are a different column and are kept by design.';

SELECT declare_migration(
    20261127000010,
    'additive',
    'Replaces _resource_erasure_apply_redaction, _erasure_apply_redaction, '
    'principal_erasure_survey_plan and resource_erasure_survey_plan (signatures unchanged): the '
    'UPDATE that empties kb_block_content.content also sets content_hash to sha256(''''); the plans '
    'skip already-emptied rows when gathering text hashes, and the resource plan''s record no longer '
    'says block hashes are kept. Backfills rows already emptied and adds a CHECK that an emptied row '
    'carries that hash, under a SHARE ROW EXCLUSIVE lock. No column, type or result shape moves. A '
    'binary without this change is unaffected: only these database functions empty a row. An erasure '
    'blocked mid-statement on the lock when this commits is refused by the CHECK and is re-run.'
);
