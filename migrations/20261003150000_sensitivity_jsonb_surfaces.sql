-- The sensitivity sweep over its jsonb surfaces, per-path ledger remediability and derived closure
-- (spec D2, D3, Q26, Q34, Q37-Q39). Grounding: temper-artifacts
-- plans/2026-10-03-sensitivity-sweep-c2-payload-map.md and plans/2026-10-01-sensitivity-sweep-3a-core.md, PR C2.

-- Units each event type contributed to a run, so a type that dominates the budget is visible (Q37).
-- Event type names are registered vocabulary, never content. kb_event_types.name has no shape of its
-- own, so scan_lane counts a name off this one as `unrecognised_kind` rather than fail the tick.
CREATE FUNCTION sensitivity.is_kind_tally(p jsonb) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT jsonb_typeof(p) = 'object'
       AND NOT EXISTS (SELECT 1 FROM jsonb_each(p) e
                        WHERE e.key !~ '^[a-z0-9_]{1,63}$' OR jsonb_typeof(e.value) <> 'number'
                           OR (e.value #>> '{}')::numeric NOT BETWEEN 0 AND 100000);
$$;

-- The jsonb scan keeps no per-place observation: a jsonb surface on a mutable cursor would need one,
-- per path, and is refused until it is built.
ALTER TABLE sensitivity.surfaces
    ADD CONSTRAINT surfaces_jsonb_is_append_only
        CHECK (shape <> 'jsonb' OR cursor_kind IS DISTINCT FROM 'mutable_timestamp');

ALTER TABLE sensitivity.unscanned_places DROP CONSTRAINT unscanned_places_reason_check,
    ADD CONSTRAINT unscanned_places_reason_check CHECK (reason IN ('oversize', 'oversize_row'));

ALTER TABLE sensitivity.runs
    ADD COLUMN units_by_event_type jsonb NOT NULL DEFAULT '{}'
        CHECK (sensitivity.is_kind_tally(units_by_event_type));

-- Where a document's keys stop being schema and become a user's (payload map §1). Below a root,
-- every key is written `?`; '/' makes the whole document one. resource_reblocked's /dispositions
-- needs no root: its keys are block uuids, which Q26's shape already writes `?`. For a property
-- value, p_kind is the key of an EDGE-owned row only: `anchored-at` has a validated shape there
-- (db_backend validate_keyed_edge_write), while any open_meta key, `anchored-at` included, lands on
-- a resource as a user's document.
CREATE FUNCTION sensitivity.user_map_roots(p_surface text, p_kind text) RETURNS text[]
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN p_surface = 'kb_events.payload' THEN CASE p_kind
            WHEN 'invocation_closed' THEN '{/outcome}'
            WHEN 'property_asserted' THEN '{/value}'
            WHEN 'property_set'      THEN '{/value}'
            WHEN 'shape_declared'    THEN '{/schema}'
            WHEN 'webhook_received'  THEN '{/}'
            ELSE '{}' END
        WHEN p_surface = 'kb_properties.property_value' THEN
            CASE WHEN p_kind = 'anchored-at' THEN '{}' ELSE '{/}' END
        ELSE '{}'
    END::text[];
$$;

-- A document's string and number leaves, each at its Q26 path: schema keys as themselves, array
-- elements as `*`, and `?` for a key below a user-map root or off Q26's shape. A key written `?` is
-- also yielded as a unit at its own path, since what the path hides is still authored text. A
-- path stops growing at 16 segments, the 16th standing for that place and everything below it. A
-- scalar document is one place, `/?`.
CREATE FUNCTION sensitivity.jsonb_units(p_doc jsonb, p_roots text[]) RETURNS TABLE (path text, unit text)
LANGUAGE sql IMMUTABLE STRICT AS $$
    WITH RECURSIVE w (path, depth, raw, in_map, v, key_unit) AS (
        SELECT ''::text, 0, ''::text, '/' = ANY (p_roots), p_doc, NULL::text
        UNION ALL
        SELECT CASE WHEN w.depth >= 16 THEN w.path ELSE w.path || '/' || c.seg END,
               w.depth + 1,
               w.raw || '/' || coalesce(c.k, '*'),
               w.in_map OR (w.raw || '/' || coalesce(c.k, '*')) = ANY (p_roots),
               c.v,
               CASE WHEN c.seg = '?' THEN c.k END
          FROM w
         CROSS JOIN LATERAL (
              SELECT e.key, e.value,
                     CASE WHEN w.in_map OR e.key !~ '^[a-z_]{1,63}$' THEN '?' ELSE e.key END
                FROM jsonb_each(CASE WHEN jsonb_typeof(w.v) = 'object' THEN w.v END) e
              UNION ALL
              SELECT NULL, a.value, '*'
                FROM jsonb_array_elements(CASE WHEN jsonb_typeof(w.v) = 'array' THEN w.v END) a
         ) c (k, v, seg)
    )
    SELECT coalesce(nullif(w.path, ''), '/?'), w.v #>> '{}'
      FROM w WHERE jsonb_typeof(w.v) IN ('string', 'number')
    UNION ALL
    SELECT w.path, w.key_unit FROM w WHERE w.key_unit IS NOT NULL;
$$;

-- The jsonb sources: (target_id, order_at, doc, resource_id, roots, kind). kind is the event type,
-- tallied per run; a property key is authored text, so property_value rows carry none.

CREATE FUNCTION sensitivity.as_uuid(p text) RETURNS uuid
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN p ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN p::uuid END;
$$;

-- The resource whose erasure trail holds this event: the five arms of _resource_erasure_trail_scope
-- (20260929040730), read from the event's side, held equal to it by a test (Q41). An edge event with
-- a resource at both ends is in both trails; it is attributed to the source, as C1's edge view is.
-- webhook_received holds a provider's document, whose own `resource_id` key is not ours.
CREATE FUNCTION sensitivity.event_resource(p_type text, p_category text, p_payload jsonb) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN p_category <> 'domain' OR p_type = 'webhook_received' THEN NULL ELSE coalesce(
        sensitivity.as_uuid(p_payload ->> 'resource_id'),
        CASE WHEN p_payload #>> '{owner,table}' = 'kb_resources'
             THEN sensitivity.as_uuid(p_payload #>> '{owner,id}') END,
        (SELECT b.resource_id FROM kb_content_blocks b
          WHERE b.id = sensitivity.as_uuid(p_payload ->> 'block_id')),
        (SELECT CASE WHEN e.source_table = 'kb_resources' THEN e.source_id
                     WHEN e.target_table = 'kb_resources' THEN e.target_id END
           FROM kb_edges e
          WHERE e.id = coalesce(sensitivity.as_uuid(p_payload ->> 'edge_id'),
                                CASE WHEN p_payload #>> '{owner,table}' = 'kb_edges'
                                     THEN sensitivity.as_uuid(p_payload #>> '{owner,id}') END)))
    END;
$$;

CREATE VIEW sensitivity.src_kb_events__payload AS
    SELECT e.id AS target_id, NULL::timestamptz AS order_at, e.payload AS doc,
           sensitivity.event_resource(t.name, t.category, e.payload) AS resource_id,
           sensitivity.user_map_roots('kb_events.payload', t.name) AS roots, t.name AS kind
      FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id;

CREATE VIEW sensitivity.src_kb_events__metadata AS
    SELECT e.id AS target_id, NULL::timestamptz AS order_at, e.metadata AS doc,
           sensitivity.event_resource(t.name, t.category, e.payload) AS resource_id,
           sensitivity.user_map_roots('kb_events.metadata', t.name) AS roots, t.name AS kind
      FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id;

CREATE VIEW sensitivity.src_kb_properties__property_value AS
    SELECT p.id AS target_id, NULL::timestamptz AS order_at, p.property_value AS doc,
           CASE p.owner_table WHEN 'kb_resources' THEN p.owner_id ELSE b.resource_id END AS resource_id,
           sensitivity.user_map_roots('kb_properties.property_value',
                                    CASE WHEN p.owner_table = 'kb_edges' THEN p.property_key END) AS roots,
           NULL::text AS kind
      FROM kb_properties p
      LEFT JOIN kb_content_blocks b ON p.owner_table = 'kb_content_blocks' AND b.id = p.owner_id;

-- C1's body (20261002200000), with the place's path, and the memo only where p_memo (Q43): jsonb rows
-- are append-only and read once per detector version, and their ids and hashes are each unique, so
-- a memo row per unit would grow with the ledger and save almost nothing.
DROP FUNCTION sensitivity.scan_unit(text, uuid, uuid, text, text, text, int, bytea);

CREATE FUNCTION sensitivity.scan_unit(
    p_surface text, p_target uuid, p_path text, p_resource uuid, p_unit text, p_hash text,
    p_detector text, p_version int, p_salt bytea, p_memo boolean,
    OUT p_memo_hit boolean, OUT p_new_severity smallint
) LANGUAGE plpgsql AS $$
DECLARE
    v_dv     sensitivity.detector_versions;
    v_d      sensitivity.detectors;
    v_count  int;
    v_prints bytea[];
    v_id     uuid;
BEGIN
    p_memo_hit := p_memo AND EXISTS (SELECT 1 FROM sensitivity.memo m WHERE m.content_hash = p_hash
                             AND m.detector_id = p_detector AND m.detector_version = p_version);
    IF p_memo_hit THEN
        RETURN;
    END IF;
    SELECT * INTO v_dv FROM sensitivity.detector_versions WHERE detector_id = p_detector AND version = p_version;
    SELECT * INTO v_d FROM sensitivity.detectors WHERE id = p_detector;
    SELECT count(*)::int,
           array_agg(DISTINCT sha256(p_salt || convert_to(
               CASE WHEN v_dv.validator IS NULL THEN m ELSE regexp_replace(m, '[^0-9]', '', 'g') END,
               'UTF8'))) FILTER (WHERE v_d.fingerprinted)
      INTO v_count, v_prints
      FROM sensitivity.detector_matches(p_detector, p_version, p_unit) m;
    IF v_count = 0 THEN
        IF p_memo THEN
            INSERT INTO sensitivity.memo (content_hash, detector_id, detector_version)
            VALUES (p_hash, p_detector, p_version) ON CONFLICT DO NOTHING;
        END IF;
        RETURN;
    END IF;

    -- Q29: a bump's re-detection inherits its immediate predecessor's sightings, unless the bump
    -- changed the category, after which nothing carries (Q27).
    INSERT INTO sensitivity.findings (surface, target_table, target_id, path, resource_id, content_hash,
        detector_id, detector_version, category, severity, match_count, fingerprint_state,
        first_seen, last_seen)
    SELECT p_surface, split_part(p_surface, '.', 1), p_target, p_path, p_resource, p_hash, p_detector,
           p_version, v_dv.category, v_d.severity, v_count,
           CASE WHEN NOT v_d.fingerprinted THEN 'not_fingerprinted'
                WHEN cardinality(v_prints) > 64 THEN 'truncated' ELSE 'complete' END,
           CASE WHEN prior.category = v_dv.category THEN prior.first_seen ELSE now() END,
           CASE WHEN prior.category = v_dv.category THEN prior.last_seen ELSE now() END
      FROM (SELECT 1) one
      LEFT JOIN LATERAL (
          SELECT f.category, f.first_seen, f.last_seen FROM sensitivity.findings f
           WHERE f.surface = p_surface AND f.target_id = p_target AND f.path IS NOT DISTINCT FROM p_path
             AND f.content_hash = p_hash AND f.detector_id = p_detector
             AND f.detector_version < p_version
           ORDER BY f.detector_version DESC LIMIT 1) prior ON true
    ON CONFLICT DO NOTHING
    RETURNING id INTO v_id;
    IF v_id IS NULL THEN
        RETURN;
    END IF;
    p_new_severity := v_d.severity;
    INSERT INTO sensitivity.finding_fingerprints (finding_id, fingerprint)
    SELECT v_id, fp FROM unnest(v_prints) fp ORDER BY fp LIMIT 64;
END;
$$;

-- C1's body (20261002200000), now reading jsonb rows as well. A jsonb row is walked whole inside
-- the row loop: the deadline is checked between rows only, so no LIMIT or clock can leave a row
-- half-scanned beneath an advanced watermark. What bounds a row instead is its size (Q40): a row of
-- more than 10,000 units, or whose text exceeds 4 MiB, is passed whole and named in unscanned_places
-- as `oversize_row`, so one document can neither outlast the tick nor read as clean. The byte bound
-- keeps the count itself cheap; 10,000 units is about 1.5 s of detector work at measured rates.
CREATE OR REPLACE FUNCTION sensitivity.scan_lane(
    p_run uuid, p_surface sensitivity.surfaces, p_lane text, p_salt bytea,
    p_bound_at timestamptz, p_bound_id uuid, p_budget int, p_deadline timestamptz,
    OUT p_rows int
) LANGUAGE plpgsql AS $$
DECLARE
    v_mutable boolean := p_surface.cursor_kind = 'mutable_timestamp';
    v_jsonb   boolean := p_surface.shape = 'jsonb';
    v_head    boolean := p_lane = 'head';
    v_ids text[]; v_vers int[]; v_at timestamptz[]; v_id uuid[]; v_incl boolean[];
    v_lo_at timestamptz; v_lo_id uuid;
    v_last_at timestamptz; v_last_id uuid;
    v_sql text;
    r record;
    v_u record;
    v_hash text;
    v_prior text;
    v_hit boolean; v_sev smallint;
    v_units int;
    v_kind text;
    v_scanned int := 0; v_hits int := 0; v_found int := 0; v_oversize int := 0;
    v_bysev jsonb := '{}';
    v_bykind jsonb := '{}';
BEGIN
    p_rows := 0;
    -- A backfill cursor not yet started reads from its floor, inclusive.
    SELECT array_agg(c.detector_id ORDER BY c.detector_id), array_agg(c.detector_version ORDER BY c.detector_id),
           array_agg(CASE WHEN v_head OR c.watermark_id IS NOT NULL THEN c.watermark_at ELSE c.backfill_floor_at END ORDER BY c.detector_id),
           array_agg(coalesce(c.watermark_id, c.backfill_floor_id) ORDER BY c.detector_id),
           array_agg(NOT v_head AND c.watermark_id IS NULL ORDER BY c.detector_id)
      INTO v_ids, v_vers, v_at, v_id, v_incl
      FROM sensitivity.cursors c
      JOIN sensitivity.detectors d ON d.id = c.detector_id AND d.version = c.detector_version AND d.enabled
     WHERE c.surface = p_surface.surface AND c.lane = p_lane
       AND (v_head OR c.backfill_completed_at IS NULL);
    IF v_ids IS NULL OR p_budget <= 0 THEN
        RETURN;
    END IF;
    -- The head reads from its lowest watermark up; the backfill from its highest down (Q30).
    SELECT x.a, x.i INTO v_lo_at, v_lo_id FROM unnest(v_at, v_id) x(a, i)
     ORDER BY CASE WHEN v_head THEN x.a END, CASE WHEN v_head THEN x.i END,
              x.a DESC, x.i DESC LIMIT 1;
    v_sql := format(CASE WHEN v_jsonb
                         THEN 'SELECT target_id, order_at, NULL::text AS unit, resource_id, NULL::text AS content_hash, doc, roots, kind '
                         ELSE 'SELECT target_id, order_at, unit, resource_id, sensitivity.keyed_hash($6, coalesce(unit, '''')) AS content_hash, '
                              'NULL::jsonb AS doc, NULL::text[] AS roots, NULL::text AS kind ' END
                    || 'FROM sensitivity.%I WHERE ', 'src_' || replace(p_surface.surface, '.', '__'))
          || CASE WHEN v_mutable AND v_head THEN '(order_at, target_id) > ($1, $2) AND order_at < $3 ORDER BY order_at, target_id'
                  WHEN v_mutable THEN '(order_at, target_id) <= ($1, $2) ORDER BY order_at DESC, target_id DESC'
                  WHEN v_head THEN 'target_id > $2 AND target_id < $4 ORDER BY target_id'
                  ELSE 'target_id <= $2 ORDER BY target_id DESC' END
          || ' LIMIT $5';
    FOR r IN EXECUTE v_sql USING v_lo_at, v_lo_id, p_bound_at, p_bound_id, p_budget, p_salt LOOP
        EXIT WHEN clock_timestamp() > p_deadline;
        p_rows := p_rows + 1;
        v_last_at := r.order_at; v_last_id := r.target_id;
        IF v_mutable THEN
            -- Q29: content that changed since this place was last read and now matches an older
            -- finding is a recurrence; the same content read again is not.
            SELECT o.content_hash INTO v_prior FROM sensitivity.place_observations o
             WHERE o.surface = p_surface.surface AND o.target_id = r.target_id;
            IF FOUND AND v_prior <> r.content_hash THEN
                UPDATE sensitivity.findings SET last_seen = now()
                 WHERE surface = p_surface.surface AND target_id = r.target_id AND path IS NULL
                   AND content_hash = r.content_hash;
            END IF;
            INSERT INTO sensitivity.place_observations (surface, target_id, content_hash)
            VALUES (p_surface.surface, r.target_id, r.content_hash)
            ON CONFLICT (surface, target_id) DO UPDATE
               SET content_hash = EXCLUDED.content_hash, observed_at = now()
             WHERE sensitivity.place_observations.content_hash <> EXCLUDED.content_hash;
        END IF;
        v_units := 0;
        IF v_jsonb AND (octet_length(r.doc::text) > 4194304
                        OR (SELECT count(*) FROM (SELECT 1 FROM sensitivity.jsonb_units(r.doc, r.roots) LIMIT 10001) n) > 10000) THEN
            v_oversize := v_oversize + 1;
            INSERT INTO sensitivity.unscanned_places (surface, target_id, reason)
            VALUES (p_surface.surface, r.target_id, 'oversize_row') ON CONFLICT DO NOTHING;
            CONTINUE;
        END IF;
        FOR v_u IN SELECT NULL::text AS path, r.unit AS unit WHERE NOT v_jsonb
                 UNION ALL
                 SELECT j.path, j.unit FROM sensitivity.jsonb_units(r.doc, r.roots) j WHERE v_jsonb LOOP
            CONTINUE WHEN v_u.unit IS NULL OR v_u.unit = '';
            v_units := v_units + 1;
            IF octet_length(v_u.unit) > 1048576 THEN
                v_oversize := v_oversize + 1;
                INSERT INTO sensitivity.unscanned_places (surface, target_id, reason)
                VALUES (p_surface.surface, r.target_id, 'oversize') ON CONFLICT DO NOTHING;
                CONTINUE;
            END IF;
            v_hash := CASE WHEN v_jsonb THEN sensitivity.keyed_hash(p_salt, v_u.unit) ELSE r.content_hash END;
            FOR k IN 1 .. cardinality(v_ids) LOOP
                CONTINUE WHEN CASE WHEN v_head
                    THEN (coalesce(r.order_at, '-infinity'), r.target_id) <= (coalesce(v_at[k], '-infinity'), v_id[k])
                    ELSE (coalesce(r.order_at, '-infinity'), r.target_id) > (coalesce(v_at[k], '-infinity'), v_id[k])
                      OR ((coalesce(r.order_at, '-infinity'), r.target_id) = (coalesce(v_at[k], '-infinity'), v_id[k])
                          AND NOT v_incl[k]) END;
                SELECT s.p_memo_hit, s.p_new_severity INTO v_hit, v_sev FROM sensitivity.scan_unit(
                    p_surface.surface, r.target_id, v_u.path, r.resource_id, v_u.unit, v_hash,
                    v_ids[k], v_vers[k], p_salt, NOT v_jsonb) s;
                IF v_hit THEN v_hits := v_hits + 1; ELSE v_scanned := v_scanned + 1; END IF;
                IF v_sev IS NOT NULL THEN
                    v_found := v_found + 1;
                    v_bysev := jsonb_set(v_bysev, ARRAY[v_sev::text],
                                         to_jsonb(least(coalesce((v_bysev ->> v_sev::text)::int, 0) + 1, 100000)));
                END IF;
            END LOOP;
        END LOOP;
        IF r.kind IS NOT NULL AND v_units > 0 THEN
            v_kind := CASE WHEN r.kind ~ '^[a-z0-9_]{1,63}$' THEN r.kind ELSE 'unrecognised_kind' END;
            v_bykind := jsonb_set(v_bykind, ARRAY[v_kind],
                                  to_jsonb(least(coalesce((v_bykind ->> v_kind)::int, 0) + v_units, 100000)));
        END IF;
    END LOOP;

    IF v_last_id IS NOT NULL THEN
        UPDATE sensitivity.cursors c
           SET watermark_at = v_last_at, watermark_id = v_last_id, updated = now()
         WHERE c.surface = p_surface.surface AND c.lane = p_lane
           AND (c.detector_id, c.detector_version) IN (SELECT * FROM unnest(v_ids, v_vers))
           AND CASE WHEN v_head
                    THEN (coalesce(c.watermark_at, '-infinity'), c.watermark_id) < (coalesce(v_last_at, '-infinity'), v_last_id)
                    ELSE (coalesce(coalesce(c.watermark_at, c.backfill_floor_at), '-infinity'),
                          coalesce(c.watermark_id, c.backfill_floor_id)) >= (coalesce(v_last_at, '-infinity'), v_last_id) END;
    END IF;
    -- The source ran out before the budget or the clock did: this backfill has reached the tail.
    IF NOT v_head AND p_rows < p_budget AND clock_timestamp() <= p_deadline THEN
        UPDATE sensitivity.cursors c SET backfill_completed_at = now(), updated = now()
         WHERE c.surface = p_surface.surface AND c.lane = 'backfill'
           AND (c.detector_id, c.detector_version) IN (SELECT * FROM unnest(v_ids, v_vers));
    END IF;

    UPDATE sensitivity.runs u
       SET rows_examined   = least(u.rows_examined + p_rows, 100000),
           hashes_examined = least(u.hashes_examined + v_scanned, 100000),
           cache_hits      = least(u.cache_hits + v_hits, 100000),
           new_findings    = least(u.new_findings + v_found, 100000),
           new_findings_head     = least(u.new_findings_head + CASE WHEN v_head THEN v_found ELSE 0 END, 100000),
           new_findings_backfill = least(u.new_findings_backfill + CASE WHEN v_head THEN 0 ELSE v_found END, 100000),
           units_oversize  = least(u.units_oversize + v_oversize, 100000),
           cursor_advances = least(u.cursor_advances + CASE WHEN v_last_id IS NULL THEN 0 ELSE 1 END, 100000),
           by_severity     = (SELECT coalesce(jsonb_object_agg(k, least(coalesce((u.by_severity ->> k)::int, 0)
                                                                 + coalesce((v_bysev ->> k)::int, 0), 100000)), '{}')
                                FROM (SELECT jsonb_object_keys(u.by_severity || v_bysev) k) ks),
           units_by_event_type = (SELECT coalesce(jsonb_object_agg(k, least(coalesce((u.units_by_event_type ->> k)::int, 0)
                                                                     + coalesce((v_bykind ->> k)::int, 0), 100000)), '{}')
                                    FROM (SELECT jsonb_object_keys(u.units_by_event_type || v_bykind) k) ks)
     WHERE u.id = p_run;
END;
$$;

-- C1's body (20261002200000), picking from every enabled surface rather than the text ones only.
CREATE OR REPLACE FUNCTION sensitivity_sweep_claim(p_budget_rows int DEFAULT 2000, p_lease_seconds int DEFAULT 330)
RETURNS TABLE (run_id uuid, job_id uuid)
LANGUAGE plpgsql AS $$
DECLARE
    v_pick text;
    v_job  record;
BEGIN
    SELECT s.surface INTO v_pick FROM sensitivity.surfaces s
     WHERE s.enabled
     ORDER BY (SELECT r.id FROM sensitivity.runs r WHERE r.surface = s.surface ORDER BY r.id DESC LIMIT 1)
              NULLS FIRST, s.surface
     LIMIT 1;
    IF v_pick IS NOT NULL THEN
        PERFORM workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep',
            jsonb_build_object('surface', v_pick, 'budget', p_budget_rows));
    END IF;
    SELECT c.id, c.attempts, c.payload INTO v_job
      FROM workflow_job_claim_system('sensitivity', 'sensitivity-sweep', 1, p_lease_seconds) c;
    IF v_job.id IS NULL THEN
        RETURN;
    END IF;
    INSERT INTO sensitivity.runs (surface, job_id, attempt)
    VALUES (v_job.payload ->> 'surface', v_job.id, v_job.attempts) RETURNING id INTO run_id;
    job_id := v_job.id;
    RETURN NEXT;
END;
$$;

-- C1's body (20261002200000). A surface is scanned when it is enabled and has a source; one with no
-- source idles rather than failing, so enabling a surface ahead of its reader stalls nothing.
CREATE OR REPLACE FUNCTION sensitivity_sweep_tick(
    p_run uuid, p_job uuid, p_salt bytea, p_lag interval DEFAULT '5 minutes', p_budget_ms int DEFAULT 20000
) RETURNS TABLE (rows_examined int, hashes_examined int, cache_hits int, new_findings int,
                 cursor_advances int, units_oversize int, failed boolean)
LANGUAGE plpgsql AS $$
DECLARE
    v_job      record;
    v_surface  sensitivity.surfaces;
    v_deadline timestamptz := clock_timestamp() + make_interval(secs => p_budget_ms / 1000.0);
    v_bound_at timestamptz;
    v_bound_id uuid;
    v_budget   int;
    v_head     int;
    v_code     text;
BEGIN
    SELECT j.payload INTO v_job FROM kb_workflow_jobs j
      JOIN sensitivity.runs r ON r.id = p_run AND r.job_id = j.id AND r.attempt = j.attempts
                             AND r.outcome IS NULL AND r.surface = j.payload ->> 'surface'
     WHERE j.id = p_job AND j.persona = 'sensitivity' AND j.status = 'in_progress'
       AND j.lease_expires_at > now();
    IF NOT FOUND THEN
        RETURN;
    END IF;
    SELECT s.* INTO v_surface FROM sensitivity.surfaces s WHERE s.surface = v_job.payload ->> 'surface';
    IF p_salt IS NULL OR octet_length(p_salt) < 16 THEN
        v_code := 'salt_missing';
    ELSIF NOT v_surface.enabled
       OR to_regclass(format('sensitivity.%I', 'src_' || replace(v_surface.surface, '.', '__'))) IS NULL THEN
        UPDATE sensitivity.runs SET outcome = 'idle', finished_at = clock_timestamp() WHERE id = p_run;
    ELSE
        -- D4 rule 1: never past now() - lag, nor past the oldest transaction open here. Any open
        -- transaction may yet commit a row stamped before it wrote (Witness 5); the hold is
        -- recorded, so a stalled head never reads as quiet.
        SELECT least(now(), coalesce(min(a.xact_start), now())) - p_lag INTO v_bound_at
          FROM pg_stat_activity a
         WHERE a.datname = current_database() AND a.pid <> pg_backend_pid() AND a.xact_start IS NOT NULL;
        v_bound_id := sensitivity.v7_floor(v_bound_at);
        UPDATE sensitivity.runs
           SET head_holdback_seconds = least(ceil(extract(epoch FROM (now() - p_lag) - v_bound_at)), 100000)
         WHERE id = p_run;
        BEGIN
            -- A new detector version starts both lanes at the bound: its head goes forward, its
            -- backfill goes down from the same point (D4 rule 2).
            INSERT INTO sensitivity.cursors (surface, cursor_kind, detector_id, detector_version, lane,
                watermark_at, watermark_id, backfill_floor_at, backfill_floor_id)
            SELECT v_surface.surface, v_surface.cursor_kind, d.id, d.version, l.lane,
                   CASE WHEN l.lane = 'head' AND v_surface.cursor_kind = 'mutable_timestamp' THEN v_bound_at END,
                   CASE WHEN l.lane = 'head' THEN m.floor_id END,
                   CASE WHEN l.lane = 'backfill' AND v_surface.cursor_kind = 'mutable_timestamp' THEN v_bound_at END,
                   CASE WHEN l.lane = 'backfill' THEN m.floor_id END
              FROM sensitivity.detectors d
             CROSS JOIN (VALUES ('head'), ('backfill')) l(lane)
             CROSS JOIN (SELECT CASE WHEN v_surface.cursor_kind = 'mutable_timestamp'
                                     THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid ELSE v_bound_id END) m(floor_id)
             WHERE d.enabled
            ON CONFLICT DO NOTHING;
            -- The backfill keeps half the budget while it has history left (D4: its own budget).
            v_budget := least((v_job.payload ->> 'budget')::int, 100000);
            v_head := CASE WHEN EXISTS (
                          SELECT 1 FROM sensitivity.cursors c
                            JOIN sensitivity.detectors d ON d.id = c.detector_id AND d.version = c.detector_version
                           WHERE c.surface = v_surface.surface AND c.lane = 'backfill' AND d.enabled
                             AND c.backfill_completed_at IS NULL)
                      THEN v_budget / 2 ELSE v_budget END;
            v_head := sensitivity.scan_lane(p_run, v_surface, 'head', p_salt, v_bound_at, v_bound_id,
                                            v_head, v_deadline);
            PERFORM sensitivity.scan_lane(p_run, v_surface, 'backfill', p_salt, v_bound_at, v_bound_id,
                                          v_budget - v_head, v_deadline);
            UPDATE sensitivity.runs SET outcome = 'scanned', finished_at = clock_timestamp() WHERE id = p_run;
        EXCEPTION
            -- Never SQLERRM: a message can quote the row that raised it (D8 constraint 2). OTHERS
            -- does not match query_canceled, so the timeout is caught by name.
            WHEN query_canceled THEN
                v_code := 'statement_timeout';
            WHEN OTHERS THEN
                v_code := CASE WHEN SQLSTATE = '2201B' THEN 'detector_pattern_invalid'
                               WHEN SQLSTATE LIKE '23%' THEN 'store_constraint'
                               ELSE 'scan_failed' END;
        END;
    END IF;
    failed := v_code IS NOT NULL;
    IF failed THEN
        UPDATE sensitivity.runs SET outcome = 'failed', finished_at = clock_timestamp() WHERE id = p_run;
        PERFORM workflow_job_fail_system(p_job, 'sensitivity', 'sensitivity-sweep', v_code);
    ELSE
        PERFORM workflow_job_complete_system(p_job, 'sensitivity', 'sensitivity-sweep');
    END IF;
    SELECT r.rows_examined, r.hashes_examined, r.cache_hits, r.new_findings, r.cursor_advances, r.units_oversize
      INTO rows_examined, hashes_examined, cache_hits, new_findings, cursor_advances, units_oversize
      FROM sensitivity.runs r WHERE r.id = p_run;
    RETURN NEXT;
END;
$$;

-- Q42: payment_card v2. v1 passed Luhn on 0.43% of sha256 hex strings (431 of 100,000 measured),
-- so every chunk content_hash in the ledger and every commit SHA in a webhook could raise a
-- severity-4 finding. A run of digits with four hex characters beside it on either side is part of
-- a hash, not a card; prose such as `card4111…` still matches, since `card` is not all hex. v1's
-- findings close as superseded once v2's backfill passes their places (Q27, read in 3b).
UPDATE sensitivity.detectors
   SET version = version + 1,
       pattern = '(?<![0-9])(?<![0-9A-Fa-f]{4})([0-9]{13,19}|[0-9]{4}( [0-9]{4}){3}|[0-9]{4}(-[0-9]{4}){3}|[0-9]{4} [0-9]{6} [0-9]{4,5}|[0-9]{4}-[0-9]{6}-[0-9]{4,5})(?![0-9])(?![0-9A-Fa-f]{4})',
       note = 'contiguous, or in card groupings with one separator, behind Luhn; never inside a hex run'
 WHERE id = 'payment_card';

-- ── Remediability per (event_type, path) (D3, Witness 25) ────────────────────────────────────────
-- The interim source until the payload half of scripts/resource-erasure-surface.txt exists (erasure
-- D9): the arms of ledger_remainder in resource_erasure_survey_plan, verbatim, which a test holds
-- equal to the live function. event_type NULL is the metadata arm, which rides every type.
--
-- The switchover (Q39): erasure cut 2's migration replaces sensitivity.ledger_remediability, so a
-- listed path reads `remediable`, and drops this table for the manifest's `redact` lines. Until
-- then a listed path reads `blocked:cut-2`, and everything else `unremediable`. Either way a path
-- counts only on an event inside some resource's erasure trail (Q41): the act redacts nothing else.
CREATE TABLE sensitivity.ledger_redact_paths (
    event_type   text CHECK (event_type ~ '^[a-z_]{1,63}$'),
    erasure_path text NOT NULL CHECK (erasure_path ~ '^[a-z_.\[\]*]{1,200}$'),
    surface      text NOT NULL GENERATED ALWAYS AS (
                     CASE WHEN erasure_path LIKE 'metadata.%' THEN 'kb_events.metadata' ELSE 'kb_events.payload' END) STORED,
    path         text NOT NULL GENERATED ALWAYS AS (
                     '/' || replace(replace(regexp_replace(erasure_path, '^metadata\.', ''), '[*]', '/*'), '.', '/')) STORED,
    CONSTRAINT ledger_redact_paths_metadata_rides_every_type
        CHECK ((event_type IS NULL) = (erasure_path LIKE 'metadata.%')),
    UNIQUE NULLS NOT DISTINCT (event_type, erasure_path)
);

INSERT INTO sensitivity.ledger_redact_paths (event_type, erasure_path) VALUES
    ('resource_created',           'title'),
    ('resource_created',           'origin_uri'),
    ('resource_created',           'blocks[*].incorporated[*].source.value'),
    ('resource_updated',           'title'),
    ('resource_updated',           'origin_uri'),
    ('block_created',              'block.incorporated[*].source.value'),
    ('block_mutated',              'incorporated[*].source.value'),
    ('block_folded',               'reason'),
    ('citation_audited',           'reason'),
    ('relationship_asserted',      'label'),
    ('relationship_folded',        'reason'),
    ('relationship_corrected',     'scar'),
    ('block_provenance_corrected', 'scar'),
    ('block_provenance_corrected', 'source.value'),
    ('property_set',               'property_key'),
    ('property_set',               'value'),
    ('property_asserted',          'property_key'),
    ('property_asserted',          'value'),
    ('property_unset',             'property_key'),
    ('block_provenance_annotated', 'incorporated[*].source.value'),
    ('resource_reblocked',         'created[*].attribution[*].source.value'),
    ('resource_reblocked',         'kept[*].attribution[*].source.value'),
    (NULL,                         'metadata.reasoning'),
    (NULL,                         'metadata.rationale');

-- A finding falls under a listed path by prefix: property_*'s /value is a whole subtree. p_resource
-- is the resource whose trail holds the event (event_resource), NULL when no trail does.
CREATE FUNCTION sensitivity.ledger_remediability(p_surface text, p_event_type text, p_path text, p_resource uuid)
RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN p_resource IS NOT NULL AND EXISTS (
               SELECT 1 FROM sensitivity.ledger_redact_paths l
                WHERE l.surface = p_surface
                  AND (l.event_type = p_event_type OR l.event_type IS NULL)
                  AND (p_path = l.path OR left(p_path, length(l.path) + 1) = l.path || '/'))
           THEN 'blocked:cut-2' ELSE 'unremediable' END;
$$;

CREATE VIEW sensitivity.ledger_finding_remediability AS
    SELECT f.id AS finding_id, t.name AS event_type,
           sensitivity.ledger_remediability(f.surface, t.name, f.path, f.resource_id) AS remediability
      FROM sensitivity.findings f
      JOIN kb_events e ON e.id = f.target_id
      JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE f.surface IN ('kb_events.payload', 'kb_events.metadata');

-- ── Closure derives (D2, Q34, Q38; Witnesses 9 and 26) ───────────────────────────────────────────
-- From what the place holds now, and nothing else: a missing row, emptied content, an erasure
-- sentinel, or on a mutable surface the sweep's own later observation. Never from a hash comparison
-- against a source table (Q34): kb_erased_content holds unkeyed hashes, and principal erasure empties
-- the content it records, which `content_empty` reads. Never from `erased_at` alone: the act leaves
-- some places untouched (block-owned properties, task 01a0fedb), and on every place it does reach a
-- place signal already answers (D2 as amended). A ledger place never closes here (Q38): cut 1
-- redacts no payload. A surface this function does not know reads open, never closed.
CREATE FUNCTION sensitivity.place_closure(p_surface text, p_target uuid, p_hash text) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_surface NOT IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path',
                               'kb_resources.title', 'kb_resources.origin_uri', 'kb_properties.property_key',
                               'kb_edges.label', 'kb_citation_audits.reason', 'kb_remote_sources.uri',
                               'kb_properties.property_value') THEN NULL
        WHEN x.present IS NULL THEN 'row_missing'
        -- The resource erasure act's D4 sentinels (20260929040730, steps 9a, 9b, 9d).
        WHEN (p_surface = 'kb_resources.title'           AND x.unit = 'erased-' || p_target::text)
          OR (p_surface = 'kb_resources.origin_uri'      AND x.unit = 'erased:' || p_target::text)
          OR (p_surface = 'kb_properties.property_key'   AND x.unit ~ '^erased-key-[0-9]+$')
          OR (p_surface = 'kb_properties.property_value' AND x.unit = '"erased"') THEN 'sentinel'
        WHEN coalesce(x.unit, '') = ''
          OR (p_surface = 'kb_properties.property_value' AND x.unit IN ('{}', '[]', '""', 'null')) THEN 'content_empty'
        WHEN o.content_hash <> p_hash THEN 'changed'
    END
      FROM (SELECT 1) one
      LEFT JOIN LATERAL (
          SELECT true, bc.content FROM kb_block_content bc
           WHERE p_surface = 'kb_block_content.content' AND bc.block_revision_id = p_target
          UNION ALL
          SELECT true, cc.content FROM kb_chunk_content cc
           WHERE p_surface = 'kb_chunk_content.content' AND cc.chunk_id = p_target
          UNION ALL
          SELECT true, c.header_path FROM kb_chunks c
           WHERE p_surface = 'kb_chunks.header_path' AND c.id = p_target
          UNION ALL
          SELECT true, r.title FROM kb_resources r
           WHERE p_surface = 'kb_resources.title' AND r.id = p_target
          UNION ALL
          SELECT true, r.origin_uri FROM kb_resources r
           WHERE p_surface = 'kb_resources.origin_uri' AND r.id = p_target
          UNION ALL
          SELECT true, p.property_key FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_key' AND p.id = p_target
          UNION ALL
          SELECT true, p.property_value::text FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_value' AND p.id = p_target
          UNION ALL
          SELECT true, e.label FROM kb_edges e
           WHERE p_surface = 'kb_edges.label' AND e.id = p_target
          UNION ALL
          SELECT true, a.reason FROM kb_citation_audits a
           WHERE p_surface = 'kb_citation_audits.reason' AND a.id = p_target
          UNION ALL
          SELECT true, s.uri FROM kb_remote_sources s
           WHERE p_surface = 'kb_remote_sources.uri' AND s.id = p_target
      ) x (present, unit) ON true
      LEFT JOIN sensitivity.place_observations o ON o.surface = p_surface AND o.target_id = p_target;
$$;

-- A finding is closed when its place says so. An open finding has no row here.
CREATE VIEW sensitivity.finding_closure AS
    SELECT c.finding_id, c.closed_by
      FROM (SELECT f.id AS finding_id, sensitivity.place_closure(f.surface, f.target_id, f.content_hash) AS closed_by
              FROM sensitivity.findings f) c
     WHERE c.closed_by IS NOT NULL;

SELECT declare_migration(
    20261003150000,
    'additive',
    'The sensitivity sweep over its jsonb surfaces (kb_events.payload and metadata, kb_properties.property_value): a path walker writing user-map keys as ?, readers for the three surfaces, a per-row size bound (oversize_row), scan_unit taking the place''s path (dropped and recreated with a new signature; only scan_lane calls it), and the claim and tick accepting jsonb surfaces. Adds runs.units_by_event_type, a CHECK holding jsonb surfaces to append-only cursors, payment_card v2, the interim ledger_redact_paths table with ledger_remediability and its view, and the derived finding_closure view. Additive: no deployed binary names the sensitivity schema (the grep gate holds it); everything else is new or a body change inside that schema and its two tick functions.'
);
