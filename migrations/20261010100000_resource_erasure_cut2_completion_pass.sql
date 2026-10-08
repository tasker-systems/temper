-- Resource erasure, cut 2 PR 3: the completion pass (build order 4).
--
-- Spec: temper-artifacts specs/2026-09-28-resource-erasure-design.md, D5, D12 and Witness 15, and
-- *Rulings from cut 2 PR 2*, item 14. Plan: plans/2026-10-08-erasure-cut2-pr3-completion-pass.md.
--
-- A resource erased under cut 1 kept its free text in the ledger, named in its record's
-- ledger_remainder (D12). Running the act on it now completes it: one more resource_erased for the
-- same subject, carrying only redacted_fields, and the ledger rewrite that record authorizes, in the
-- same transaction (20261009100000's verifier admits nothing else).
--
--   1. _resource_erasure_ledger_remote_elements / _numbers(resource): a husk's remote-source numbering, read
--      from the ledger in step (9e)'s order, because (9e) has already re-pointed the husk's
--      provenance and _resource_erasure_remote_originals finds no original there (ruling 14).
--   2. _resource_erasure_payload_redaction reads that numbering on an erased resource, after
--      checking it against the sentinel rows the husk's provenance cites.
--   3. resource_erasure_completion_fields(resource): the paths a completion pass would rewrite now,
--      for the survey door on a husk.
--   4. resource_erasure_execute: an erased resource with something left to redact is completed;
--      one with nothing left is refused `already erased`, as before.
--   5. The sweep's remediability reads the allowlist and answers `remediable`; its interim table
--      is dropped (sweep spec Q39, Q41; Witness 25).
--
-- resource_erasure_survey_plan is not replaced. Its comment on a husk, that the derivation raises
-- on a cut-1 husk's remote sources, is superseded by Section 2 below: the plan now lists a cut-1
-- husk's completion paths.

-- ---------------------------------------------------------------------------
-- Section 1. A husk's remote-source numbering, read from the ledger (ruling 14).
-- ---------------------------------------------------------------------------
-- Step (9e)'s numbering (_resource_erasure_remote_originals, 20260929040730 Section 0c) starts from
-- the provenance rows of R's blocks and finds each one's place in its contributing event's list. On
-- a husk those rows already cite the sentinel rows, because (9e) re-pointed them, so the capture
-- finds no original. A husk erased under cut 1 still carries the original URLs in its ledger. This
-- reads the same order from the ledger side. Per block of R, it numbers each distinct normalized
-- remote URL in R's trail events by its first appearance: the event's occurred_at, then the event
-- id, then the element's seq, then the element's position in that event's list for the block. The
-- position counts every element, remote or not, as (9e) counts it, and the per-type lists are
-- (9e)'s. A URL listed twice in one event takes its FIRST-listed element's seq and position, not
-- the lower seq: kb_block_provenance is unique per (block, kind, source, contributing event), and
-- _insert_block_provenance's ON CONFLICT DO NOTHING keeps the first element's accretion_seq, which
-- is the seq (9e) orders by. _resource_erasure_payload_redaction checks the result against the husk's provenance
-- before using it.
CREATE FUNCTION _resource_erasure_ledger_remote_elements(p_resource uuid)
RETURNS TABLE(block_id uuid, event_id uuid, occurred_at timestamptz, seq integer, pos bigint,
              uri_normalized text)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH ev AS (
        SELECT e.id, e.occurred_at, s.event_type, e.payload
          FROM _resource_erasure_trail_scope(p_resource) s
          JOIN kb_events e ON e.id = s.event_id
         WHERE s.event_type IN ('resource_created', 'block_created', 'block_mutated',
                                'block_provenance_annotated', 'resource_reblocked')
    ), lists AS (
        SELECT ev.id, ev.occurred_at, (ev.payload ->> 'block_id')::uuid AS block_id,
               ev.payload -> 'incorporated' AS list
          FROM ev
         WHERE ev.event_type IN ('block_mutated', 'block_provenance_annotated')
        UNION ALL
        SELECT ev.id, ev.occurred_at, (ev.payload #>> '{block,block_id}')::uuid,
               ev.payload #> '{block,incorporated}'
          FROM ev
         WHERE ev.event_type = 'block_created'
        UNION ALL
        SELECT ev.id, ev.occurred_at, (x.v ->> 'block_id')::uuid,
               jsonb_agg(inc.v ORDER BY x.i, inc.j)
          FROM ev
         CROSS JOIN LATERAL jsonb_array_elements(ev.payload -> 'blocks') WITH ORDINALITY AS x(v, i)
         CROSS JOIN LATERAL jsonb_array_elements(
                CASE WHEN jsonb_typeof(x.v -> 'incorporated') = 'array'
                     THEN x.v -> 'incorporated' ELSE '[]'::jsonb END) WITH ORDINALITY AS inc(v, j)
         WHERE ev.event_type = 'resource_created'
           AND jsonb_typeof(ev.payload -> 'blocks') = 'array'
         GROUP BY ev.id, ev.occurred_at, (x.v ->> 'block_id')::uuid
        UNION ALL
        SELECT ev.id, ev.occurred_at, (x.v ->> 'block_id')::uuid,
               jsonb_agg(att.v ORDER BY x.i, att.j)
          FROM ev
         CROSS JOIN LATERAL jsonb_array_elements(coalesce(ev.payload -> 'created', '[]'::jsonb)
                                                 || coalesce(ev.payload -> 'kept', '[]'::jsonb))
                    WITH ORDINALITY AS x(v, i)
         CROSS JOIN LATERAL jsonb_array_elements(
                CASE WHEN jsonb_typeof(x.v -> 'attribution') = 'array'
                     THEN x.v -> 'attribution' ELSE '[]'::jsonb END) WITH ORDINALITY AS att(v, j)
         WHERE ev.event_type = 'resource_reblocked'
         GROUP BY ev.id, ev.occurred_at, (x.v ->> 'block_id')::uuid
    ), elements AS (
        SELECT l.block_id, l.occurred_at, l.id AS event_id, (el.v ->> 'seq')::integer AS seq,
               el.ord AS pos, normalize_remote_uri(el.v #>> '{source,value}') AS uri_normalized
          FROM lists l
          JOIN kb_content_blocks b ON b.id = l.block_id AND b.resource_id = p_resource
         CROSS JOIN LATERAL jsonb_array_elements(
                CASE WHEN jsonb_typeof(l.list) = 'array' THEN l.list ELSE '[]'::jsonb END)
                    WITH ORDINALITY AS el(v, ord)
         WHERE el.v #>> '{source,kind}' = 'remote'
    )
    SELECT e.block_id, e.event_id, e.occurred_at, e.seq, e.pos, e.uri_normalized FROM elements e;
$$;

COMMENT ON FUNCTION _resource_erasure_ledger_remote_elements(uuid) IS
$c$Every remote-source element R's trail events list for a block of R (spec 2026-09-28 D4, ruling 14
of cut 2 PR 2): the event, its occurred_at, the element's seq, its position in that event's list for
the block (every element counted, as step (9e) counts it) and its normalized URL. The per-type lists
are (9e)'s. Read by _resource_erasure_ledger_remote_numbers, and by the derivation's check of a
husk's provenance against it.$c$;

CREATE FUNCTION _resource_erasure_ledger_remote_numbers(p_resource uuid)
RETURNS TABLE(block_id uuid, uri_normalized text, n integer)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH firsts AS (
        SELECT DISTINCT ON (e.block_id, e.uri_normalized)
               e.block_id, e.uri_normalized, e.occurred_at, e.event_id, e.seq, e.pos
          FROM _resource_erasure_ledger_remote_elements(p_resource) e
         ORDER BY e.block_id, e.uri_normalized, e.occurred_at, e.event_id, e.pos
    )
    SELECT f.block_id, f.uri_normalized,
           (row_number() OVER (PARTITION BY f.block_id
                                   ORDER BY f.occurred_at, f.event_id, f.seq, f.pos))::integer
      FROM firsts f;
$$;

COMMENT ON FUNCTION _resource_erasure_ledger_remote_numbers(uuid) IS
$c$A resource's remote-source sentinel numbering read from its ledger (spec 2026-09-28 D4, ruling 14
of cut 2 PR 2): one row per (block, normalized remote URL) R's trail events cite, with the n of
erased:<block_id>:<n>, in step (9e)'s order (first appearance by occurred_at, event id, element seq,
position in the event's list for the block). _resource_erasure_remote_originals reads the same order
from provenance; this is its form for a husk, whose provenance (9e) has already re-pointed.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The derivation numbers a husk's remote sources from the ledger.
-- ---------------------------------------------------------------------------
-- 20261009100000's function with one change: the remote-source map. On a live resource it is
-- still the capture (9e) re-points by. On an erased one it is Section 1's numbering, after a check
-- that, provenance row by provenance row, it gives each row's element the sentinel that row cites. On a husk
-- erased under cut 1 the ledger holds the originals and provenance the sentinels; on one erased
-- under cut 2, both hold the sentinels. A disagreement raises: the derivation never guesses.
CREATE OR REPLACE FUNCTION _resource_erasure_payload_redaction(p_resource uuid)
RETURNS TABLE(event_id uuid, new_payload jsonb, new_metadata jsonb, paths jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_remote   jsonb;
    -- Each numbering is computed once per owner or endpoint pair and kept here, keyed
    -- '<owner or pair>|<text>': the per-location lookups read the cache, so the cost is linear in
    -- the trail, not in the trail times its owners' families.
    v_keys     jsonb := '{}'::jsonb;
    v_facets   jsonb := '{}'::jsonb;
    v_labels   jsonb := '{}'::jsonb;
    v_done     jsonb := '{}'::jsonb;
    v_ev       record;
    v_path     text;
    v_loc      record;
    v_class    text;
    v_meta     boolean;
    v_payload  jsonb;
    v_metadata jsonb;
    v_paths    jsonb;
    v_new      jsonb;
    v_block    uuid;
    v_owner    text;
    v_n        integer;
BEGIN
    -- The remote-source numbering, captured once: (block, normalized URI) → n. On a live resource,
    -- from the ONE capture step (9e) re-points by (_resource_erasure_remote_originals). On a husk,
    -- (9e) has already re-pointed it, so the numbering is read from the ledger in (9e)'s order
    -- (ruling 14), and must name exactly the sentinel rows the husk's provenance cites.
    IF EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.erased_at IS NOT NULL) THEN
        -- The check is per provenance row, not per set: a set comparison would pass a
        -- permutation, the ledger giving one URL the n another URL's row cites, and the first
        -- rewrite of a path is final (kb_event_field_redactions' key), so a wrong n could never be
        -- corrected. Each remote row of the husk was inserted for the first element its
        -- contributing event lists for the block (ON CONFLICT DO NOTHING per event), at that
        -- element's seq, and (9e) re-pointed it to that element's n; so some element of that event,
        -- block and seq must carry a URL the ledger numbers to exactly the sentinel the row cites.
        -- Within one (event, seq), two URLs are ordered by their position in the list on both sides.
        -- And every number the ledger gives a block must be cited by some row of it, or the
        -- ledger names a source (9e) never re-pointed.
        IF EXISTS (
            SELECT 1
              FROM kb_block_provenance bp
              JOIN kb_content_blocks b ON b.id = bp.block_id
              JOIN kb_remote_sources rs ON rs.id = bp.source_id
             WHERE b.resource_id = p_resource AND bp.source_kind = 'remote'
               AND NOT EXISTS (
                   SELECT 1
                     FROM _resource_erasure_ledger_remote_elements(p_resource) el
                     JOIN _resource_erasure_ledger_remote_numbers(p_resource) ln
                       ON ln.block_id = el.block_id AND ln.uri_normalized = el.uri_normalized
                    WHERE el.block_id = bp.block_id
                      AND el.event_id = bp.contributed_by_event_id
                      AND el.seq = bp.accretion_seq
                      AND normalize_remote_uri('erased:' || ln.block_id::text || ':' || ln.n::text)
                          = rs.uri_normalized))
           OR EXISTS (
            SELECT 1
              FROM _resource_erasure_ledger_remote_numbers(p_resource) ln
             WHERE NOT EXISTS (
                   SELECT 1
                     FROM kb_block_provenance bp
                     JOIN kb_remote_sources rs ON rs.id = bp.source_id
                    WHERE bp.block_id = ln.block_id AND bp.source_kind = 'remote'
                      AND rs.uri_normalized
                          = normalize_remote_uri('erased:' || ln.block_id::text || ':' || ln.n::text))) THEN
            RAISE EXCEPTION '_resource_erasure_payload_redaction: the ledger''s remote-source numbering of erased resource % does not match the sentinels its provenance cites',
                p_resource;
        END IF;
        SELECT coalesce(jsonb_object_agg(l.block_id::text || '|' || l.uri_normalized, l.n), '{}'::jsonb)
          INTO v_remote
          FROM _resource_erasure_ledger_remote_numbers(p_resource) l;
    ELSE
        SELECT coalesce(jsonb_object_agg(o.block_id::text || '|' || rs.uri_normalized, o.n), '{}'::jsonb)
          INTO v_remote
          FROM _resource_erasure_remote_originals(p_resource) o
          JOIN kb_remote_sources rs ON rs.id = o.source_id;
    END IF;

    -- An erasure act's own events (its relationship_folded, correlated with its resource_erased)
    -- are that act's record, not R's text: a later erasure of an edge's other end leaves them.
    FOR v_ev IN
        SELECT s.event_id AS id, s.event_type AS type, ev.payload, ev.metadata
          FROM _resource_erasure_trail_scope(p_resource) s
          JOIN kb_events ev ON ev.id = s.event_id
         WHERE NOT EXISTS (
                   SELECT 1 FROM kb_events er JOIN kb_event_types ert ON ert.id = er.event_type_id
                    WHERE ert.name = 'resource_erased'
                      AND er.correlation_id = ev.correlation_id AND er.id <> ev.id)
         ORDER BY s.event_id
    LOOP
        v_payload  := v_ev.payload;
        v_metadata := v_ev.metadata;
        v_paths    := '[]'::jsonb;
        FOR v_path IN
            SELECT DISTINCT r.path FROM _erasure_redact_paths() r
             WHERE r.event_type = v_ev.type OR r.event_type IS NULL
             ORDER BY r.path
        LOOP
            v_class := _erasure_path_class(v_ev.type, v_path, v_ev.payload);
            CONTINUE WHEN v_class IS NULL OR v_class = 'keep';
            v_meta := v_path LIKE 'metadata.%';
            FOR v_loc IN
                SELECT x.at, x.val
                  FROM _erasure_expand(CASE WHEN v_meta THEN v_ev.metadata ELSE v_ev.payload END,
                                       CASE WHEN v_meta THEN substr(v_path, 10) ELSE v_path END) x
            LOOP
                -- A JSON null carries no text. Rewriting it would make replay project a value
                -- live never held (a doc_type row, and every later key number with it).
                CONTINUE WHEN jsonb_typeof(v_loc.val) = 'null';
                v_new := _erasure_sentinel_exact(v_class, v_ev.id, v_ev.payload);
                IF v_class = 'remote-source-url' THEN
                    IF coalesce(_erasure_location_is_remote(v_ev.payload, v_loc.at), false) THEN
                        v_block := _erasure_location_block(v_ev.payload, v_loc.at);
                        v_n := (v_remote ->> (v_block::text || '|'
                                              || normalize_remote_uri(v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no sentinel number for a remote source of block % in event %',
                                v_block, v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased:' || v_block::text || ':' || v_n::text);
                    ELSE
                        v_new := v_loc.val;
                    END IF;
                ELSIF v_class = 'edge-label' THEN
                    IF _erasure_label_is_kept(v_loc.val) THEN
                        v_new := v_loc.val;
                    ELSE
                        v_owner := concat_ws('|', v_ev.payload #>> '{source,table}', v_ev.payload #>> '{source,id}',
                                             v_ev.payload #>> '{target,table}', v_ev.payload #>> '{target,id}');
                        IF NOT v_done ? ('label|' || v_owner) THEN
                            SELECT v_labels || coalesce(jsonb_object_agg(v_owner || '|' || l.label, l.n), '{}'::jsonb)
                              INTO v_labels
                              FROM _resource_erasure_label_numbers(
                                       v_ev.payload #>> '{source,table}', (v_ev.payload #>> '{source,id}')::uuid,
                                       v_ev.payload #>> '{target,table}', (v_ev.payload #>> '{target,id}')::uuid) l;
                            v_done := v_done || jsonb_build_object('label|' || v_owner, true);
                        END IF;
                        v_n := (v_labels ->> (v_owner || '|' || (v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no label number for event %', v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased-label-' || v_n::text);
                    END IF;
                ELSIF v_class IN ('property-key', 'facet-value') THEN
                    v_owner := (v_ev.payload #>> '{owner,table}') || '|' || (v_ev.payload #>> '{owner,id}');
                    IF NOT v_done ? ('owner|' || v_owner) THEN
                        SELECT v_keys || coalesce(jsonb_object_agg(v_owner || '|' || k.property_key, k.n), '{}'::jsonb)
                          INTO v_keys
                          FROM _resource_erasure_ledger_key_numbers(v_ev.payload #>> '{owner,table}',
                                                                    (v_ev.payload #>> '{owner,id}')::uuid) k;
                        SELECT v_facets || coalesce(jsonb_object_agg(v_owner || '|' || f.inner_key, f.m), '{}'::jsonb)
                          INTO v_facets
                          FROM _resource_erasure_facet_numbers(v_ev.payload #>> '{owner,table}',
                                                               (v_ev.payload #>> '{owner,id}')::uuid) f;
                        v_done := v_done || jsonb_build_object('owner|' || v_owner, true);
                    END IF;
                    IF v_class = 'property-key' THEN
                        v_n := (v_keys ->> (v_owner || '|' || (v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no key number for event %', v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased-key-' || v_n::text);
                    ELSIF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm WHERE fm.inner_key IS NULL) THEN
                        v_new := to_jsonb('erased:' || v_ev.id::text);
                    ELSIF NOT EXISTS (SELECT 1 FROM _facet_marks(v_loc.val)) THEN
                        -- An empty facet value names no mark: nothing to redact (found by the
                        -- code review, 2026-10-08: it raised, and blocked the act).
                        v_new := v_loc.val;
                    ELSE
                        IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm
                                    WHERE NOT v_facets ? (v_owner || '|' || fm.inner_key)) THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no mark number for a facet of event %', v_ev.id;
                        END IF;
                        SELECT jsonb_object_agg('erased-facet-' || (v_facets ->> (v_owner || '|' || fm.inner_key)), 'erased')
                          INTO v_new
                          FROM _facet_marks(v_loc.val) fm;
                    END IF;
                END IF;
                IF v_new IS NULL THEN
                    -- jsonb_set with a NULL value returns NULL: never let a missing number empty
                    -- a whole payload.
                    RAISE EXCEPTION '_resource_erasure_payload_redaction: no sentinel for % of event %', v_path, v_ev.id;
                END IF;
                IF v_new IS DISTINCT FROM v_loc.val THEN
                    IF v_meta THEN
                        v_metadata := jsonb_set(v_metadata, v_loc.at, v_new, false);
                    ELSE
                        v_payload := jsonb_set(v_payload, v_loc.at, v_new, false);
                    END IF;
                    IF NOT v_paths @> to_jsonb(ARRAY[v_path]) THEN
                        v_paths := v_paths || to_jsonb(v_path);
                    END IF;
                END IF;
            END LOOP;
        END LOOP;
        IF jsonb_array_length(v_paths) > 0 THEN
            event_id     := v_ev.id;
            new_payload  := v_payload;
            new_metadata := v_metadata;
            paths        := v_paths;
            RETURN NEXT;
        END IF;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION _resource_erasure_payload_redaction(uuid) IS
$c$The cut-2 ledger redaction of one resource (spec 2026-09-28 D3, D4): for each event of its trail
(_resource_erasure_trail_scope), the payload and metadata with every allowlisted path
(_erasure_redact_paths) at its class sentinel, and the paths that changed. Sentinel numbers come
from the ONE numberings: remote sources from _resource_erasure_remote_originals (on a husk, from
_resource_erasure_ledger_remote_numbers, checked against the sentinels its provenance cites),
property keys from _resource_erasure_ledger_key_numbers (through _resource_erasure_key_numbers),
facet marks from _resource_erasure_facet_numbers, edge labels from _resource_erasure_label_numbers.
The survey plan reports its paths as redacted_fields; the act, and on a husk the completion pass,
writes its payloads.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. What a completion pass would rewrite now, for the survey door.
-- ---------------------------------------------------------------------------
-- The survey door answers a husk with no plan: the plan counts rows on the husk, which would
-- misstate what an act could reach. What the act can still reach on a husk is exactly this list,
-- read from the same derivation the act runs (D10), as paths only, never values. Empty when
-- nothing is left, and then the act refuses `already erased`.
CREATE FUNCTION resource_erasure_completion_fields(p_resource uuid)
RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'paths', d.paths)
                              ORDER BY d.event_id), '[]'::jsonb)
      FROM _resource_erasure_payload_redaction(p_resource) d;
$$;

COMMENT ON FUNCTION resource_erasure_completion_fields(uuid) IS
$c$The ledger paths a completion pass (spec 2026-09-28 D12) would rewrite on an erased resource now,
in the RedactedEventFields shape: the derivation the act runs, paths only. Read by the survey door on
a husk. Empty means the act would refuse already_erased.$c$;

-- ---------------------------------------------------------------------------
-- Section 4. The act completes a husk.
-- ---------------------------------------------------------------------------
-- 20261009100000's function with one change: the `already erased` verdict. An erased resource whose
-- derivation still names something is completed (D5, D12, ruled 2026-09-29); one with nothing left
-- is refused, as before.
CREATE OR REPLACE FUNCTION public.resource_erasure_execute(p_resource uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid, p_also_strike_blobs uuid[] DEFAULT '{}'::uuid[])
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
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
$function$
;

-- ---------------------------------------------------------------------------
-- Section 5. The sweep reads remediability from the allowlist (sweep spec Q39, Q41; Witness 25).
-- ---------------------------------------------------------------------------
-- Until now a listed path read `blocked:cut-2`: the act could not yet reach a cut-1 husk's ledger.
-- With the completion pass it can reach every resource's, so a listed path reads `remediable`, and
-- the list is the verifier's own allowlist, never a copy (sweep spec, *Carried to erasure cut 2*:
-- one relation both read). A `keep` line refines a redact line back to structural for one literal
-- key; the sweep's per-path read has no payload to qualify by, so the redact line it refines
-- answers. The trail gate is unchanged (Q41): a path counts only on an event inside some resource's
-- erasure trail, because the act redacts nothing else. And it counts only for a resource the act
-- will run on: a cogmap's charter resource is refused (D5; map-grain erasure is filed task
-- 01a0e960-0ca2-7f42-b33e-1ed19b024e6b), so its findings read `blocked:map-grain`, never a remedy
-- that does not exist (found by the security review, 2026-10-08). Closing a finding the pass redacted is not
-- this: place_closure reads no ledger surface yet (plan ruling 3, a separate PR).
CREATE OR REPLACE FUNCTION sensitivity.ledger_remediability(p_surface text, p_event_type text, p_path text, p_resource uuid)
RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE WHEN p_resource IS NULL OR NOT EXISTS (
               SELECT 1
                 FROM _erasure_redact_paths() l
                CROSS JOIN LATERAL (
                    SELECT CASE WHEN l.path LIKE 'metadata.%' THEN 'kb_events.metadata'
                                ELSE 'kb_events.payload' END AS surface,
                           '/' || replace(replace(regexp_replace(l.path, '^metadata\.', ''), '[*]', '/*'), '.', '/')
                               AS path) w
                WHERE l.class <> 'keep'
                  AND w.surface = p_surface
                  AND (l.event_type = p_event_type OR l.event_type IS NULL)
                  AND (p_path = w.path OR left(p_path, length(w.path) + 1) = w.path || '/'))
                THEN 'unremediable'
                WHEN EXISTS (SELECT 1 FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource)
                THEN 'blocked:map-grain'
                ELSE 'remediable' END;
$$;

DROP TABLE sensitivity.ledger_redact_paths;

SELECT declare_migration(
    20261010100000,
    'additive',
    'New: the functions _resource_erasure_ledger_remote_elements, _resource_erasure_ledger_remote_numbers and resource_erasure_completion_fields. CREATE OR REPLACE, with unchanged signatures and return types, of _resource_erasure_payload_redaction (on an erased resource, remote sources are numbered from the ledger, checked against the sentinels its provenance cites), resource_erasure_execute (an erased resource whose ledger still holds text is completed under a second resource_erased carrying only redacted_fields; one with nothing left still raises already erased) and sensitivity.ledger_remediability (reads _erasure_redact_paths and answers remediable instead of blocked:cut-2, or blocked:map-grain on a cogmap charter resource, which this act refuses). Drops sensitivity.ledger_redact_paths, which only ledger_remediability read. A binary that predates this reads the execute outcome it always read, and a completion record is a resource_erased whose other fields are omitted, which ResourceErased already defaults.'
);
