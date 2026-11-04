-- A resource that leaves a home says so in that home: the departure.
--
-- Ruled by the owner 2026-11-03 for task 01a1a1a5-773a-7599-a157-6128705ef71e: a context's change
-- feed (20261116000010) has an entry for every change to what lives there, arrivals and departures
-- included, and never for what a thing contains. Two acts left a context silently:
--   - a move: resource_rehome anchors its one event on the DESTINATION, and its payload names no
--     prior home, so the home it left recorded nothing;
--   - an erasure: its record, resource_erased, is an admin event anchored on no context.
-- A deletion already says so: resource_deleted is anchored on the home.
--
-- resource_departed is one event anchored on the home being left, carrying only the resource id.
-- It names no destination, so a reader of the old home learns only that the resource left. It
-- projects nothing: the act it belongs to does the work, and replay walks it as a no-op.
-- _resource_departure_append holds the rule in one place. It writes a departure only for a live
-- resource leaving a home it still has, so a resource deleted earlier does not depart twice, and a
-- move to the home it is already in is no departure.
--
-- resource_rehome and resource_erasure_execute are replaced body-only, signatures unchanged.
-- resource_erasure_execute is copied from the live definition (20261015100010's), with one call
-- added before its record.

INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('resource_departed', NULL, 1, 'domain');

-- The departure of p_resource from its current home, unless it is not live, has no home, or the
-- destination is that same home. p_to_table/p_to_id name the destination of a move, NULL for an
-- act that ends the resource. Returns the event id, or NULL when there is no departure.
CREATE FUNCTION _resource_departure_append(
    p_resource uuid, p_emitter uuid,
    p_to_table text, p_to_id uuid,
    p_correlation uuid,
    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_table text; v_id uuid;
BEGIN
    SELECT h.anchor_table, h.anchor_id INTO v_table, v_id
      FROM kb_resource_homes h JOIN kb_resources r ON r.id = h.resource_id
     WHERE h.resource_id = p_resource AND r.is_active;
    IF v_id IS NULL OR (v_table = p_to_table AND v_id = p_to_id) THEN
        RETURN NULL;
    END IF;
    RETURN _event_append('resource_departed', p_emitter, v_table, v_id,
                         jsonb_build_object('resource_id', p_resource),
                         p_metadata => p_metadata, p_invocation => p_invocation,
                         p_correlation => p_correlation);
END;
$$;

-- The move: its record on the destination as before, then the departure on the home it leaves,
-- under the same correlation id, read from the record so a move fired without one still pairs.
-- Both before the projection re-points the home.
CREATE OR REPLACE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid,
                                           p_metadata jsonb DEFAULT '{}'::jsonb,
                                           p_invocation uuid DEFAULT NULL,
                                           p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    PERFORM _resource_departure_append(
        (p_payload->>'resource_id')::uuid, p_emitter,
        p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
        (SELECT correlation_id FROM kb_events WHERE id = v_ev),
        p_metadata, p_invocation);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;

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
    -- ── The departure (20261116000020): the erased resource leaves the home it is read in, as
    --    a pointer anchored there, so a reader following that home's changes is told it went.
    --    Before the record below, while the resource is still live; the act's correlation id,
    --    so it is the act's own event. ───────────────────────────────────────────────────────
    PERFORM _resource_departure_append(p_resource, p_emitter, NULL, NULL, p_request_ref);

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
    20261116000020,
    'additive',
    'Registers the domain event type resource_departed (permissive, NULL payload_schema), adds _resource_departure_append, and replaces resource_rehome and resource_erasure_execute body-only with signatures unchanged: each now also appends a resource_departed anchored on the home the resource leaves. Nothing is dropped, renamed or reshaped. A binary from before this migration calls both functions unchanged and gets the departure written for it; the one thing it cannot do is replay a ledger holding resource_departed rows, because its EventKind has no arm for the type. Replay is an operator and test tool, not a serving path.'
);
