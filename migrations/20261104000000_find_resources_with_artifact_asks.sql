-- `find-resources-with` narrows by what a resource's data artifacts hold: one counted set every arm
-- that asks artifacts reads, and the selection core and its gated wrapper widened by one argument.
-- Spec: temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md §2.3, §4.4,
-- §6.1. Conforms to the widening precedent 20260815000040.
--
-- What a later edit must not break:
--   1. NO `DEFAULT` on the widened signatures, and `p_artifact_asks` beside neither jsonb narrowing
--      (`p_facets`, `p_properties`) — 20260815000040, points 1 and 3.
--   2. ONE COUNTED SET. `__temper_ungated_artifact_ask_population` is the only place spec §6.1's
--      conjuncts meet governance; every arm that counts, pages or narrows by an ask reads it.
--   3. ONE BODY OF GOVERNANCE. The `_in` relations are 20261103000000's bodies taking the home and
--      family as values; its by-id forms read the row and delegate to them, so
--      artifact_ask_parity.rs still holds the one definition.
--   4. THE COUNTED SET IS PLPGSQL ON PURPOSE (its COMMENT says why). Its EXECUTE text is constant
--      and every value rides USING; never interpolate one.
--   5. The core computes each ask's set once (an uncorrelated `IN`), never per resource; asks AND
--      (`HAVING count(DISTINCT k.n)`), and an ask's predicates hold of one artifact.
--   6. THE SHAPE'S HOME IS RE-CHECKED READABLE once per ask, before anything is read; an unreadable
--      home answers nothing.
--
-- Additive: five CREATE FUNCTION, four CREATE OR REPLACE at byte-identical signatures and return
-- columns. No DROP.

-- ── Governance, taking the home and family as values ───────────────────────────────────────────
-- 20261103000000's two relations, with the home and family as arguments instead of read from the
-- named row. The by-id forms below read the row and delegate here, so there is one body, and the
-- differential in artifact_ask_parity.rs still holds it to _data_artifact_shape_in_force and
-- _data_artifact_shape_state.
CREATE FUNCTION __temper_ungated_governed_artifacts_in(
    p_visible_ids       uuid[],
    p_home_anchor_table text,
    p_home_anchor_id    uuid,
    p_kind_owner_table  text,
    p_kind_owner_id     uuid,
    p_artifact_kind     text)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text)
LANGUAGE sql STABLE AS $$
    SELECT a.id, a.resource_id, live.id, live.shape_version, a.content_hash
      FROM kb_data_artifact_shapes live
      JOIN kb_resource_homes h
        ON h.anchor_table = p_home_anchor_table
       AND h.anchor_id    = p_home_anchor_id
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id
      JOIN kb_data_artifacts a
        ON a.resource_id      = h.resource_id
       AND a.kind_owner_table = p_kind_owner_table
       AND a.kind_owner_id    = p_kind_owner_id
       AND a.artifact_kind    = p_artifact_kind
       AND NOT a.is_folded
     WHERE live.home_anchor_table = p_home_anchor_table
       AND live.home_anchor_id    = p_home_anchor_id
       AND live.kind_owner_table  = p_kind_owner_table
       AND live.kind_owner_id     = p_kind_owner_id
       AND live.artifact_kind     = p_artifact_kind
       AND NOT live.is_folded
$$;

CREATE FUNCTION __temper_ungated_governed_artifact_states_in(
    p_visible_ids       uuid[],
    p_home_anchor_table text,
    p_home_anchor_id    uuid,
    p_kind_owner_table  text,
    p_kind_owner_id     uuid,
    p_artifact_kind     text)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text, shape_state text)
LANGUAGE sql STABLE AS $$
    SELECT g.artifact_id, g.resource_id, g.shape_id, g.shape_version, g.content_hash,
           CASE WHEN v.artifact_id IS NULL THEN 'declared_not_yet_checked'
                WHEN v.satisfied           THEN 'declared_satisfied'
                ELSE                            'declared_not_satisfied'
           END
      FROM __temper_ungated_governed_artifacts_in(
             p_visible_ids, p_home_anchor_table, p_home_anchor_id,
             p_kind_owner_table, p_kind_owner_id, p_artifact_kind) AS g
      LEFT JOIN kb_data_artifact_verdicts v
             ON v.artifact_id   = g.artifact_id
            AND v.shape_id      = g.shape_id
            AND v.shape_version = g.shape_version
            AND v.content_hash  = g.content_hash
$$;

CREATE OR REPLACE FUNCTION __temper_ungated_governed_artifacts(p_visible_ids uuid[], p_shape_id uuid)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text)
LANGUAGE sql STABLE AS $$
    SELECT g.artifact_id, g.resource_id, g.shape_id, g.shape_version, g.content_hash
      FROM kb_data_artifact_shapes s
     CROSS JOIN LATERAL __temper_ungated_governed_artifacts_in(
             p_visible_ids, s.home_anchor_table, s.home_anchor_id,
             s.kind_owner_table, s.kind_owner_id, s.artifact_kind) AS g
     WHERE s.id = p_shape_id
$$;

CREATE OR REPLACE FUNCTION __temper_ungated_governed_artifact_states(p_visible_ids uuid[], p_shape_id uuid)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text, shape_state text)
LANGUAGE sql STABLE AS $$
    SELECT g.artifact_id, g.resource_id, g.shape_id, g.shape_version, g.content_hash,
           g.shape_state
      FROM kb_data_artifact_shapes s
     CROSS JOIN LATERAL __temper_ungated_governed_artifact_states_in(
             p_visible_ids, s.home_anchor_table, s.home_anchor_id,
             s.kind_owner_table, s.kind_owner_id, s.artifact_kind) AS g
     WHERE s.id = p_shape_id
$$;

-- ── The counted set ─────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION __temper_ungated_artifact_ask_population(
    p_visible_ids uuid[],
    p_reader      uuid,
    p_ask         jsonb)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               shape_state text, matched boolean)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_shape kb_data_artifact_shapes%ROWTYPE;
BEGIN
    SELECT * INTO v_shape FROM kb_data_artifact_shapes s WHERE s.id = (p_ask->>'shape')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF NOT COALESCE(anchor_readable_by_profile(p_reader, v_shape.home_anchor_table,
                                               v_shape.home_anchor_id), false) THEN
        RETURN;
    END IF;
    IF NOT COALESCE(p_ask->'include_folded' = 'false'::jsonb, false) THEN
        RETURN;
    END IF;
    RETURN QUERY EXECUTE $q$
        SELECT g.artifact_id, g.resource_id, g.shape_id, g.shape_version, g.shape_state,
               COALESCE(jsonb_typeof($7->'shape_states') = 'array'
                        AND $7->'shape_states' @> jsonb_build_array(g.shape_state)
                        AND _temper_artifact_ask_match(c.content, $7->'predicates'),
                        false)
          FROM __temper_ungated_governed_artifact_states_in($1, $2, $3, $4, $5, $6) AS g
          JOIN kb_resources_live lv ON lv.id = g.resource_id
          JOIN kb_data_artifacts a ON a.id = g.artifact_id
          LEFT JOIN kb_data_artifact_content c ON c.artifact_id = g.artifact_id
         WHERE jsonb_typeof($7->'intent') = 'array'
           AND $7->'intent' @> jsonb_build_array(a.intent)
    $q$
    USING p_visible_ids, v_shape.home_anchor_table::text, v_shape.home_anchor_id,
          v_shape.kind_owner_table::text, v_shape.kind_owner_id, v_shape.artifact_kind, p_ask;
END;
$$;

-- ── The selection core, widened ─────────────────────────────────────────────────────────────────
-- 20260816000010's body, with the property parse read in place rather than through a CTE (the same
-- rows; a body with no WITH can be inlined into a caller that hands it plain arguments) and the
-- asks conjunct added. The population is computed once per ask, not once per candidate resource.
CREATE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resources_live lv                    ON lv.id = r.id
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
      JOIN kb_resource_homes h                     ON h.resource_id = r.id
     WHERE (p_doc_types IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_doc_type dt
              WHERE dt.resource_id = r.id AND dt.doc_type = ANY(p_doc_types)))
       AND (p_stage IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.stage = p_stage))
       AND (p_status IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.status = p_status))
       AND (p_owner_profile IS NULL OR h.owner_profile_id = p_owner_profile)
       AND (p_owner_handle IS NULL OR EXISTS (
             SELECT 1 FROM kb_profiles p
              WHERE p.id = h.owner_profile_id AND p.handle = p_owner_handle))
       AND (p_title_contains IS NULL OR r.title ILIKE '%' || p_title_contains || '%')
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{}')), '{}')
               FROM kb_property_elements pe
              WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                AND pe.property_key = 'tags'
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
       AND (p_facets IS NULL OR (
             jsonb_typeof(p_facets) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM jsonb_array_elements(
                 CASE jsonb_typeof(p_facets) WHEN 'array' THEN p_facets ELSE '[]'::jsonb END) AS f
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_property_elements pe
                   WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                     AND pe.property_key = 'facet'
                     AND f->>'key' IS NOT NULL
                     AND pe.element @> jsonb_build_object(f->>'key', f->>'value')))))
       AND (p_properties IS NULL OR (
             jsonb_typeof(p_properties) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM _temper_property_predicates_parse(p_properties) AS q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_resource_properties rp
                   WHERE rp.resource_id = r.id
                     AND rp.property_key = q.property_key
                     AND _temper_property_op_match(q.op, q.vals, q.direction, q.bound, rp.property_value)))))
       -- Fail-closed as p_properties is: NULL narrows nothing, `[]` is the vacuous conjunction, and
       -- anything but an array narrows to zero. An element that is not an ask names no shape, so
       -- its population is empty and no resource satisfies the list.
       AND (p_artifact_asks IS NULL OR (
             jsonb_typeof(p_artifact_asks) = 'array'
             AND (jsonb_array_length(CASE jsonb_typeof(p_artifact_asks)
                                       WHEN 'array' THEN p_artifact_asks ELSE '[]'::jsonb END) = 0
                  OR r.id IN (
                    SELECT m.resource_id
                      FROM jsonb_array_elements(
                             CASE jsonb_typeof(p_artifact_asks)
                               WHEN 'array' THEN p_artifact_asks ELSE '[]'::jsonb END)
                             WITH ORDINALITY AS k(ask, n)
                     CROSS JOIN LATERAL __temper_ungated_artifact_ask_population(
                             p_visible_ids, p_anchor_reader, k.ask) AS m
                     WHERE m.matched
                     GROUP BY m.resource_id
                    HAVING count(DISTINCT k.n) = jsonb_array_length(
                             CASE jsonb_typeof(p_artifact_asks)
                               WHEN 'array' THEN p_artifact_asks ELSE '[]'::jsonb END)))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

CREATE OR REPLACE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             p_visible_ids, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, NULL::jsonb,
             p_anchor_table, p_anchor_id, p_anchor_reader, p_properties) c;
$$;

-- ── The gated wrapper, widened to match ─────────────────────────────────────────────────────────
CREATE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_artifact_asks  jsonb,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, p_artifact_asks,
             p_anchor_table, p_anchor_id, p_principal, p_properties) c;
$$;

CREATE OR REPLACE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM query_find_resources_with(
             p_principal, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains, NULL::jsonb,
             p_anchor_table, p_anchor_id, p_properties) c;
$$;

COMMENT ON FUNCTION __temper_ungated_artifact_ask_population(uuid[], uuid, jsonb) IS
    'UNGATED. The counted set of one artifact ask: every live artifact the asked shape''s lineage governs (its home and family, read from the named row by id, through __temper_ungated_governed_artifact_states_in), on a resource in p_visible_ids (NULL admits none) that is live and fully ingested (kb_resources_live), while p_reader can read the shape''s home (anchor_readable_by_profile, checked once per call before anything is read) and the artifact''s intent is among the ask''s. plpgsql because the query reading artifacts runs by EXECUTE … USING with the home and family as values, so it is planned with their real sizes; a family read from the row in-statement is sized from n_distinct. include_folded true admits nothing: the set is live artifacts only. One row per artifact with its shape in force, its shape_state, and matched — its state is among the ask''s shape_states and its content satisfies every predicate (_temper_artifact_ask_match). p_ask is one element of ValidatedAsk::bind, every list written out. The caller owes the verdict (its own resources_visible_to) and the reader; this function applies every other conjunct of spec 2026-10-19-asking-artifacts-by-what-they-are §6.1 (20261104000000).';

COMMENT ON FUNCTION __temper_ungated_governed_artifacts_in(uuid[], text, uuid, text, uuid, text) IS
    'UNGATED. The live artifacts of one family (kind owner table, kind owner id, artifact kind) on resources homed in one home and in p_visible_ids (NULL admits none), with the live shape of that home and family and the content hash. The body of __temper_ungated_governed_artifacts, which reads a shape row by id and delegates here; taking the values lets a caller plan with them (20261104000000).';

COMMENT ON FUNCTION __temper_ungated_governed_artifact_states_in(uuid[], text, uuid, text, uuid, text) IS
    'UNGATED. __temper_ungated_governed_artifacts_in with each row''s shape state by the staleness triple. The body of __temper_ungated_governed_artifact_states, which reads a shape row by id and delegates here (20261104000000).';

COMMENT ON FUNCTION __temper_ungated_governed_artifacts(uuid[], uuid) IS
    'UNGATED. The live artifacts governed by a shape''s lineage — its home and family, read from the named row, folded or not — on resources in p_visible_ids (NULL admits none), with the shape in force and the content hash: reads the row by id and delegates to __temper_ungated_governed_artifacts_in. Set-based _data_artifact_shape_in_force, held to it by artifact_ask_parity.rs. Applies neither the enumeration floor (kb_resources_live) nor the re-check that the shape''s home is readable; __temper_ungated_artifact_ask_population applies both (20261103000000, 20261104000000).';

COMMENT ON FUNCTION __temper_ungated_governed_artifact_states(uuid[], uuid) IS
    'UNGATED. __temper_ungated_governed_artifacts with each row''s shape state by the staleness triple (shape_id, shape_version, content_hash) against the shape in force: reads the row by id and delegates to __temper_ungated_governed_artifact_states_in. Set-based _data_artifact_shape_state, held to it by artifact_ask_parity.rs. Applies neither the enumeration floor nor the home re-check; __temper_ungated_artifact_ask_population applies both (20261103000000, 20261104000000).';

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, uuid,
        jsonb) IS
    'The selection core, widened with p_artifact_asks: a jsonb array of asks (ValidatedAsk::bind), AND-ed, each satisfied by a resource owning an artifact its counted set (__temper_ungated_artifact_ask_population) marks matched; the predicates within an ask hold of one artifact. NULL narrows nothing, [] is the vacuous conjunction, anything but an array narrows to zero. p_anchor_reader reads the shape''s home as it reads the stage anchor. No DEFAULT on any parameter, and p_artifact_asks beside neither jsonb narrowing, for the reasons recorded on 20260815000040 (20261104000000).';

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, uuid, jsonb) IS
    'Delegates to the 14-argument selection core with no artifact asks (20261104000000).';

COMMENT ON FUNCTION query_find_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, jsonb, varchar, uuid, jsonb) IS
    'Gated wrapper over the 14-argument __temper_ungated_find_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as the anchor reader, which also reads each asked shape''s home. No defaults, for the reason recorded on the core (20261104000000).';

COMMENT ON FUNCTION query_find_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, jsonb) IS
    'Delegates to the 13-argument gated wrapper with no artifact asks (20261104000000).';

SELECT declare_migration(
    20261104000000,
    'additive',
    'New functions __temper_ungated_governed_artifacts_in and __temper_ungated_governed_artifact_states_in (the 20261103000000 relations taking the home and family as values; the by-id forms are CREATE OR REPLACEd at byte-identical signatures and return columns to delegate to them, answering as before), __temper_ungated_artifact_ask_population (the counted set of one artifact ask), a 14-argument __temper_ungated_find_resources_with and a 13-argument query_find_resources_with, each carrying p_artifact_asks jsonb with no DEFAULT on any parameter. The 13-argument core and the 12-argument wrapper are CREATE OR REPLACEd at byte-identical signatures and return columns to delegate with NULL asks, so a deployed binary emitting either arity keeps the answer it had. No DROP; no existing default changes.'
);
