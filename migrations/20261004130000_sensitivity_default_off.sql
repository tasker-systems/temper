-- Q52, Q53: the sweep is off until an operator turns it on, and so is every detector.
--
-- A deployment opts in with SENSITIVITY_SWEEP_ENABLED, which the door reads (Q53); nothing here
-- can start a scan. This migration turns every temper-provided detector off on every deployment.
-- Turning one on is an operator act, through the functions below or a plain UPDATE.
--
-- Every detector now records who provides it. Temper's migrations change only the rows temper
-- provides, this one included; a row an organization curates is never touched by one. What happens
-- when an organization edits a temper-provided row is still open (the detector administration plan).

ALTER TABLE sensitivity.detectors
    ADD COLUMN provided_by text NOT NULL DEFAULT 'organization'
        CONSTRAINT detectors_provided_by_known CHECK (provided_by IN ('temper', 'organization'));

-- The nine cut 1 seeded (20261002160000). A row under any other id can only be an organization's.
-- The update changes no versioned column, so detectors_versioned records nothing.
UPDATE sensitivity.detectors
   SET provided_by = 'temper'
 WHERE id IN ('private_key_block', 'cloud_saas_key', 'connection_string_password', 'jwt',
              'payment_card', 'aba_routing', 'us_ssn_delimited', 'us_ssn_contextual',
              'local_path_username');

-- A detector an operator inserts without naming the column is the organization's, so no later
-- temper migration rewrites it; a temper migration that seeds one names provided_by = 'temper'.
-- And it does not scan until someone enables it.
ALTER TABLE sensitivity.detectors ALTER COLUMN enabled SET DEFAULT false;

UPDATE sensitivity.detectors SET enabled = false WHERE provided_by = 'temper' AND enabled;

COMMENT ON COLUMN sensitivity.detectors.provided_by IS
$c$'temper' for a detector temper's migrations seed and may version (as Q42 and Q51 did);
'organization' for one a deployment curates, which no temper migration changes. Defaults to
'organization', so an operator's INSERT that leaves it out is never rewritten by a migration.$c$;

COMMENT ON COLUMN sensitivity.detectors.enabled IS
$c$Whether the sweep evaluates this detector. Off by default, and no migration turns one on (Q52).
Not versioned: flipping it needs no bump. On its next tick, a newly enabled version gets head and
backfill cursors at that tick's bound; a version enabled before keeps its cursors, so its head
resumes where it stopped and reads what arrived in between. Disabling stops evaluation and leaves
its findings, dispositions and cursors as they are.$c$;

-- ── Enabling and disabling (Q53) ────────────────────────────────────────────────────────────────
-- An operator enables a detector by name at the version they reviewed. The table holds only the
-- current version, so a version that is not current refuses: a bump since the review means the
-- pattern being enabled is not the one that was read. Disabling takes the name alone.

CREATE FUNCTION sensitivity.enable_detector(p_id text, p_version int) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_version int;
    v_enabled boolean;
BEGIN
    SELECT d.version, d.enabled INTO v_version, v_enabled
      FROM sensitivity.detectors d WHERE d.id = p_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no detector named %', p_id USING ERRCODE = 'no_data_found';
    END IF;
    IF v_version IS DISTINCT FROM p_version THEN
        RAISE EXCEPTION 'detector % is at version %, not %: review the current version before enabling it',
            p_id, v_version, p_version USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF v_enabled THEN
        RETURN false;
    END IF;
    UPDATE sensitivity.detectors SET enabled = true WHERE id = p_id;
    RETURN true;
END;
$$;

COMMENT ON FUNCTION sensitivity.enable_detector(text, int) IS
$c$Enable one detector at the version the operator reviewed; refuses when that is not the current
version. Returns whether it changed anything.$c$;

CREATE FUNCTION sensitivity.disable_detector(p_id text) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_enabled boolean;
BEGIN
    SELECT d.enabled INTO v_enabled FROM sensitivity.detectors d WHERE d.id = p_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no detector named %', p_id USING ERRCODE = 'no_data_found';
    END IF;
    IF NOT v_enabled THEN
        RETURN false;
    END IF;
    UPDATE sensitivity.detectors SET enabled = false WHERE id = p_id;
    RETURN true;
END;
$$;

COMMENT ON FUNCTION sensitivity.disable_detector(text) IS
$c$Disable one detector. Its findings, dispositions and cursors stay. Returns whether it changed
anything.$c$;

-- By severity, as a threshold: enable everything at least this serious, disable everything at most
-- this serious. Either may be narrowed to one provider. Each returns the rows it changed.

CREATE FUNCTION sensitivity.check_severity_selection(p_severity int, p_provided_by text) RETURNS void
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF p_severity IS NULL OR p_severity NOT BETWEEN 1 AND 4 THEN
        RAISE EXCEPTION 'severity must be 1 to 4, not %', p_severity USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_provided_by IS NOT NULL AND p_provided_by NOT IN ('temper', 'organization') THEN
        RAISE EXCEPTION 'provided_by must be temper or organization, not %', p_provided_by
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
END;
$$;

CREATE FUNCTION sensitivity.enable_detectors(p_min_severity int, p_provided_by text DEFAULT NULL)
RETURNS TABLE (detector_id text, detector_version int)
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM sensitivity.check_severity_selection(p_min_severity, p_provided_by);
    RETURN QUERY
    UPDATE sensitivity.detectors d SET enabled = true
     WHERE NOT d.enabled AND d.severity >= p_min_severity
       AND (p_provided_by IS NULL OR d.provided_by = p_provided_by)
    RETURNING d.id, d.version;
END;
$$;

COMMENT ON FUNCTION sensitivity.enable_detectors(int, text) IS
$c$Enable every current detector at p_min_severity or above, optionally only those one provider
gives ('temper' or 'organization'). Enables the current version of each, whatever it is: the
operator who wants a reviewed version enables it by name. Returns each detector it enabled.$c$;

CREATE FUNCTION sensitivity.disable_detectors(p_max_severity int, p_provided_by text DEFAULT NULL)
RETURNS TABLE (detector_id text, detector_version int)
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM sensitivity.check_severity_selection(p_max_severity, p_provided_by);
    RETURN QUERY
    UPDATE sensitivity.detectors d SET enabled = false
     WHERE d.enabled AND d.severity <= p_max_severity
       AND (p_provided_by IS NULL OR d.provided_by = p_provided_by)
    RETURNING d.id, d.version;
END;
$$;

COMMENT ON FUNCTION sensitivity.disable_detectors(int, text) IS
$c$Disable every detector at p_max_severity or below, optionally only those one provider gives.
Returns each detector it disabled.$c$;

-- ── Dry runs (Q54) ──────────────────────────────────────────────────────────────────────────────
-- Measure one detector version over a bounded slice of content, writing nothing. The version may be
-- any recorded one, enabled or not. A candidate pattern is tried by inserting it as a detector and
-- rolling back after the dry run: a new row is disabled by default, and the dry run writes nothing
-- that the rollback would need to undo.
--
-- A bound is structural, never a principal's visibility: the admin reads as admin.
--   latest   the newest units of each surface
--   context  units of resources homed in that context        (kb_resource_homes)
--   cogmap   units of resources homed in that cognitive map  (kb_resource_homes)
--   profile  units of resources that profile owns            (kb_resource_homes.owner_profile_id)
--   team     units of resources homed in contexts the team owns
-- A unit with no resource (a remote source's uri, an edge between non-resources, an event about
-- none) is reached only by `latest`.
--
-- The answer is one row per surface: counts, and the places with the most matches as pointers.
-- Never a matched value (§8, D1). STABLE, so Postgres itself refuses any write the body attempts.
-- Oversized units are skipped and counted as the sweep skips them, so the two agree on what is read.

CREATE FUNCTION sensitivity.dry_run(
    p_detector text, p_version int, p_bound text, p_bound_id uuid DEFAULT NULL,
    p_surface text DEFAULT NULL, p_max_units int DEFAULT 5000, p_budget_ms int DEFAULT 20000,
    p_top int DEFAULT 10
) RETURNS TABLE (surface text, shape text, places_examined int, places_matched int, matches bigint,
                 places_oversize int, complete boolean, hotspots jsonb)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_deadline timestamptz := clock_timestamp() + make_interval(secs => p_budget_ms / 1000.0);
    v_max      int := least(greatest(coalesce(p_max_units, 0), 0), 100000);
    v_s        sensitivity.surfaces;
    v_where    text;
    v_sql      text;
    r          record;
    v_u        record;
    v_n        int;
    v_place    int;
    v_places   jsonb;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM sensitivity.detector_versions dv
                    WHERE dv.detector_id = p_detector AND dv.version = p_version) THEN
        RAISE EXCEPTION 'no detector % at version %', p_detector, p_version USING ERRCODE = 'no_data_found';
    END IF;
    v_where := CASE p_bound
        WHEN 'latest'  THEN 'true'
        WHEN 'context' THEN 'resource_id IN (SELECT h.resource_id FROM kb_resource_homes h
                                              WHERE h.anchor_table = ''kb_contexts'' AND h.anchor_id = $1)'
        WHEN 'cogmap'  THEN 'resource_id IN (SELECT h.resource_id FROM kb_resource_homes h
                                              WHERE h.anchor_table = ''kb_cogmaps'' AND h.anchor_id = $1)'
        WHEN 'profile' THEN 'resource_id IN (SELECT h.resource_id FROM kb_resource_homes h
                                              WHERE h.owner_profile_id = $1)'
        WHEN 'team'    THEN 'resource_id IN (SELECT h.resource_id FROM kb_resource_homes h
                                               JOIN kb_contexts c ON c.id = h.anchor_id
                                              WHERE h.anchor_table = ''kb_contexts''
                                                AND c.owner_table = ''kb_teams'' AND c.owner_id = $1)'
    END;
    IF v_where IS NULL THEN
        RAISE EXCEPTION 'bound must be latest, context, cogmap, profile or team, not %', p_bound
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    -- A bound naming nothing would read as "no matches"; it refuses instead.
    -- Parenthesised: plpgsql ends an IF condition at the first THEN it sees outside parentheses.
    IF p_bound <> 'latest' AND NOT (CASE p_bound
            WHEN 'context' THEN EXISTS (SELECT 1 FROM kb_contexts WHERE id = p_bound_id)
            WHEN 'cogmap'  THEN EXISTS (SELECT 1 FROM kb_cogmaps WHERE id = p_bound_id)
            WHEN 'profile' THEN EXISTS (SELECT 1 FROM kb_profiles WHERE id = p_bound_id)
            WHEN 'team'    THEN EXISTS (SELECT 1 FROM kb_teams WHERE id = p_bound_id)
        END) THEN
        RAISE EXCEPTION 'no % with id %', p_bound, p_bound_id USING ERRCODE = 'no_data_found';
    END IF;
    IF p_surface IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sensitivity.surfaces s WHERE s.surface = p_surface) THEN
        RAISE EXCEPTION 'no surface %', p_surface USING ERRCODE = 'no_data_found';
    END IF;

    FOR v_s IN
        SELECT s.* FROM sensitivity.surfaces s
         WHERE (p_surface IS NULL OR s.surface = p_surface)
           AND to_regclass(format('sensitivity.%I', 'src_' || replace(s.surface, '.', '__'))) IS NOT NULL
         ORDER BY s.surface
    LOOP
        surface := v_s.surface; shape := v_s.shape;
        places_examined := 0; places_matched := 0; matches := 0; places_oversize := 0;
        complete := true;
        v_places := '[]';
        -- The sources' own columns, as scan_lane reads them (20261003150000); newest first.
        v_sql := format(CASE WHEN v_s.shape = 'jsonb'
                             THEN 'SELECT target_id, resource_id, NULL::text AS unit, doc, roots '
                             ELSE 'SELECT target_id, resource_id, unit, NULL::jsonb AS doc, NULL::text[] AS roots ' END
                        || 'FROM sensitivity.%I WHERE ' || v_where
                        || CASE WHEN v_s.cursor_kind = 'mutable_timestamp'
                                THEN ' ORDER BY order_at DESC, target_id DESC'
                                ELSE ' ORDER BY target_id DESC' END
                        || ' LIMIT $2',
                        'src_' || replace(v_s.surface, '.', '__'));
        IF clock_timestamp() > v_deadline THEN
            complete := false;
            hotspots := v_places;
            RETURN NEXT;
            CONTINUE;
        END IF;
        FOR r IN EXECUTE v_sql USING p_bound_id, v_max + 1 LOOP
            IF places_examined >= v_max OR clock_timestamp() > v_deadline THEN
                complete := false;
                EXIT;
            END IF;
            places_examined := places_examined + 1;
            IF v_s.shape = 'jsonb' AND (octet_length(r.doc::text) > 4194304
                    OR (SELECT count(*) FROM (SELECT 1 FROM sensitivity.jsonb_units(r.doc, r.roots) LIMIT 10001) n) > 10000) THEN
                places_oversize := places_oversize + 1;
                CONTINUE;
            END IF;
            FOR v_u IN SELECT NULL::text AS path, r.unit AS unit WHERE v_s.shape <> 'jsonb'
                     UNION ALL
                     SELECT j.path, j.unit FROM sensitivity.jsonb_units(r.doc, r.roots) j WHERE v_s.shape = 'jsonb' LOOP
                CONTINUE WHEN v_u.unit IS NULL OR v_u.unit = '';
                IF octet_length(v_u.unit) > 1048576 THEN
                    places_oversize := places_oversize + 1;
                    CONTINUE;
                END IF;
                SELECT count(*)::int INTO v_n FROM sensitivity.detector_matches(p_detector, p_version, v_u.unit);
                IF v_n > 0 THEN
                    places_matched := places_matched + 1;
                    matches := matches + v_n;
                    v_places := v_places || jsonb_build_object(
                        'target_id', r.target_id, 'path', v_u.path, 'resource_id', r.resource_id,
                        'matches', v_n);
                END IF;
            END LOOP;
        END LOOP;
        SELECT coalesce(jsonb_agg(x.p ORDER BY (x.p ->> 'matches')::int DESC, x.p ->> 'target_id'), '[]')
          INTO hotspots
          FROM (SELECT p FROM jsonb_array_elements(v_places) p
                 ORDER BY (p ->> 'matches')::int DESC, p ->> 'target_id'
                 LIMIT greatest(coalesce(p_top, 0), 0)) x;
        RETURN NEXT;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION sensitivity.dry_run(text, int, text, uuid, text, int, int, int) IS
$c$Measure detector p_detector at p_version over a bounded slice, writing nothing. p_bound is latest,
context, cogmap, profile or team (p_bound_id names the last four); p_surface narrows to one surface.
Each surface reads at most p_max_units of its newest units within the bound (capped at 100000), and
the whole call stops at p_budget_ms; complete says whether the bound was read to its end. hotspots
holds the p_top places with the most matches as {target_id, path, resource_id, matches}: pointers
and counts, never a matched value.$c$;

SELECT declare_migration(
    20261004130000,
    'additive',
    'The sensitivity sweep defaults to off (sweep Q52, Q53, Q54). Adds sensitivity.detectors.provided_by (temper or organization, defaulting to organization; the nine seeded ids backfill as temper), sets the enabled default to false, and turns every temper-provided detector off. Adds enable_detector, disable_detector, enable_detectors, disable_detectors, check_severity_selection and dry_run, all new. Additive: a NOT NULL column with a default, two defaults, one data update and new functions; no signature the deployed door calls changes. A deployed binary that predates the opt-in keeps ticking, and with every detector off its ticks scan nothing and leave no rows (Q49).'
);
