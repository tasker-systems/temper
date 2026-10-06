-- Resource erasure: the act's record names its derived-vector writes (0.6.0 line item 7a; task
-- 01a1092c-a11f-7ab0-a256-aa1bd9ffa2ee).
--
-- 20261004130000 added three writes to the act (_resource_erasure_apply_redaction): a live
-- region's centroid recomputed from the remaining members (7c), a folded region's centroid zeroed (7d),
-- and the home context's telos snapshot nulled (7b). It added no `targets` row for any of them,
-- so the survey's promise and the `resource_erased` record were both silent about three kinds of
-- stored vector the act changes.
--
-- `targets` has one author: resource_erasure_execute takes its record from this function's plan,
-- computed before the redaction runs (D10), so one body fixes the survey and the record together.
-- The two new rows follow the plan's own rule: each counts the row set its act step updates, by
-- that step's predicate, and a row whose count is zero is not claimed. Outcome text is counts and
-- verbs only (goal §8). Like every count here, they are the act's prediction taken under R's lock.
-- Region rows are written by the region drain, which that lock does not block, so a materialize
-- committing during the act can move the live/folded split the act then reaches (the same
-- exposure the formation-watermark cogmap count has).
--
--   * `kb_cogmap_regions.centroid` — live regions holding R (7c's predicate: every lens, every
--     anchor) and folded regions holding R (7d's predicate).
--   * `kb_contexts.telos_centroid` — the home context's snapshot when it is non-null (7b's
--     predicate).
--
-- `ErasureTargetOutcome` is open-textured (`target` and `outcome` are strings), so the
-- resource_erased payload schema is unchanged. The body is 20261004130000's live definition,
-- verbatim except for the added statements and their comment. Signature and return type are
-- unchanged. Additive: CREATE OR REPLACE only.

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
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_properties',
            'outcome', v_a || ' resource-owned and ' || v_b
                       || ' edge-owned rows: keys erased-key-<n>, values erased, folded');
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
           AND ev.payload::text LIKE '%' || p_resource::text || '%'
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

SELECT declare_migration(
    20261004150000,
    'additive',
    'CREATE OR REPLACE of resource_erasure_survey_plan with the same signature and return type; the body is its live definition (20261004130000) plus two targets rows, kb_cogmap_regions.centroid and kb_contexts.telos_centroid, each counting the rows the act''s derived-vector statements reach and claimed only when non-zero. resource_erasure_execute takes its record from this plan, so the resource_erased record gains the rows too. ErasureTargetOutcome is open-textured, so no payload schema changes. A binary that predates this migration reads targets as a list of target/outcome strings and is unaffected by two more. No table, column, constraint or grant changes.'
);
