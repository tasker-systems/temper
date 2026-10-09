-- The erasure act and the block history scrub take a fair queue ahead of R's row lock.
--
-- Both acts take FOR UPDATE on R and wait out every FOR KEY SHARE holder. A floored writer's
-- FOR KEY SHARE conflicts with no other holder, so Postgres grants it without queueing, even
-- while an act waits: a writer arriving mid-wait jumps ahead of the act. Overlapping writers can
-- therefore hold an act off for as long as they keep overlapping, which is a denial of erasure by
-- anyone who may write R or link to it. Witnessed: temper-services
-- tests/write_lock_bound_test.rs, overlapping_writers_cannot_hold_the_act_off (task
-- 01a0fd12-f4b7-7bd2-81d0-13c0814650d5), where the act finished only after a 2.25 s chain of
-- writers that all arrived after it.
--
-- The fix is a heavyweight lock with a queue. The acts take a transaction advisory lock on R's
-- key EXCLUSIVE, just before FOR UPDATE; every floored writer takes it SHARED before its first
-- row lock on R (temper-services write_floor::queue_behind_acts, from lock_resource_row, and the
-- delete door ahead of its FOR UPDATE). A shared request that conflicts with a WAITING exclusive
-- one queues behind it, so a writer arriving after the act waits for the act, and the act waits
-- only for writers that were already in. Writers never conflict with each other (shared vs
-- shared), so ordinary writes are unchanged. The writer's wait falls under the write-side lock
-- bound (503 RESOURCE_BUSY at 5 s); the acts pin lock_timeout = 0 (20261015100000, restated here).
--
-- The order matters. A writer must request the queue before any row lock on R, or it could hold
-- a KEY SHARE the act waits on while itself waiting on the act's queue entry: a deadlock. Every
-- floored writer's first lock on R is the floor or the delete door's FOR UPDATE, and both take
-- the queue first. Writers outside a floor (substrate writes the surfaces do not reach, segmented
-- appends' own KEY SHARE, the FK KEY SHARE of an insert citing R) never take the queue, so they
-- can still pass an act, but they never wait on it either, so they cannot deadlock with it.
--
-- One key, one definition: _resource_act_queue_key. The acts call it here; Rust calls it from
-- write_floor. Distinct from every other advisory key (hash-keyed blob locks, goal_patch:<id>) by
-- its prefix.
--
-- Both bodies are their live definitions (pg_get_functiondef, after 20261015100000), verbatim
-- except for the queue line and its comment, so each restates SET lock_timeout = 0.

CREATE FUNCTION public._resource_act_queue_key(p_resource uuid)
 RETURNS bigint
 LANGUAGE sql
 IMMUTABLE
AS $$ SELECT hashtextextended('resource_act_queue:' || p_resource::text, 0) $$;

COMMENT ON FUNCTION public._resource_act_queue_key(uuid) IS
'The advisory key of resource p_resource''s act queue: the erasure act and the block history
scrub take it exclusive before FOR UPDATE on the kb_resources row; floored writers take it shared
before their first row lock (20261015100010).';

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

CREATE OR REPLACE FUNCTION public.block_history_scrub_execute(p_resource uuid, p_blocks uuid[], p_operator uuid, p_emitter uuid, p_request_ref uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
 SET lock_timeout TO '0'
AS $function$
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
    -- The act queue, exclusive, before R's row lock (20261015100010): a floored writer arriving
    -- from here on queues behind this act instead of joining the row's KEY SHARE holders.
    PERFORM pg_advisory_xact_lock(_resource_act_queue_key(p_resource));
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
$function$;

SELECT declare_migration(
    20261015100010,
    'additive',
    'New IMMUTABLE function _resource_act_queue_key(uuid). CREATE OR REPLACE of resource_erasure_execute and block_history_scrub_execute with the same signatures, return types and SET clauses; each body is its live definition plus one pg_advisory_xact_lock on the key before FOR UPDATE on the resource row. No table, column, constraint, grant or COMMENT changes to existing objects.'
);
