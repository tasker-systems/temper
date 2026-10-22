-- Asking data artifacts by what they are: the relations both arms compose — a path helper, an ask
-- matcher, the set-based governance and shape-state relations, and an index leading with the family.
-- Spec: temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md §2.3, §3.1,
-- §3.2, §5. Nothing calls them yet.
--
-- Supersedes the reason 20260820000020 gives for NO GIN index on content ("artifacts are reached
-- only through what points at them"): an artifact is now also selected by what it is, under its
-- family's shape (spec §1). The no-GIN decision stands: jsonb_path_ops indexes whole-document @>,
-- not a value at a path (spec §7.4).
--
-- What a later edit must not break:
--   1. PATHS STEP OBJECT KEYS ONLY. `#>` reads a numeric segment as an array index, so
--      ["rows","0","x"] would match rows[0].x on content whose `rows` is an array — a type-confusion
--      match the reference evaluator (temper_core::types::artifact_ask::reference) never makes.
--   2. PRESENCE IS CONJOINED BY THE MATCHER. `_temper_property_op_match`'s has_key arm is `true`
--      (20260816000010), because a property's presence is its row. An artifact's is a non-NULL value
--      at the path, and an absent path matches no operator.
--   3. TWO DEFINITIONS OF GOVERNANCE, HELD TO ONE. The governance and shape-state relations restate
--      `_data_artifact_shape_in_force` (20260822000010) and `_data_artifact_shape_state`
--      (20260822000020) set-based, because a per-row plpgsql call is opaque to the planner. The
--      differential in artifact_ask_parity.rs is what keeps them one definition.
--   4. THE POPULATION IS A HOME AND A FAMILY, read from the named shape row (folded rows included —
--      a row never changes), never "resolves to this id": an amendment folds the row and mints a new
--      id, and a population keyed on the id would empty on amendment and read as absence.
--   5. NO GATE OF THEIR OWN. Like every `__temper_ungated_` fragment they take the caller's verdict
--      as `p_visible_ids`, and a NULL set admits nothing.

-- The governance relations reach artifacts from the family, through this index, or from the home,
-- through idx_kb_resource_homes_anchor (20260624000001), whichever the planner estimates smaller.
CREATE INDEX idx_kb_data_artifacts_family
    ON kb_data_artifacts (kind_owner_table, kind_owner_id, artifact_kind)
    WHERE NOT is_folded;

-- The value at a path of object keys, or NULL when the path is absent. A JSON null at the path is
-- present (`'null'::jsonb`, not SQL NULL). An empty path is the content itself; the matcher below
-- never asks for one.
CREATE FUNCTION _temper_artifact_get(p_content jsonb, p_path text[])
RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE v_at jsonb := p_content;
        v_segment text;
BEGIN
    FOREACH v_segment IN ARRAY p_path LOOP
        IF jsonb_typeof(v_at) IS DISTINCT FROM 'object' THEN
            RETURN NULL;
        END IF;
        v_at := v_at -> v_segment;
    END LOOP;
    RETURN v_at;
END;
$$;

-- Does this content satisfy every predicate of one ask? `p_predicates` is the ask's predicate list
-- as `ArtifactPredicate` serializes it: [{"path": [...], "op": {"op": ..., ...}}, ...].
--
-- Fails closed, as 20260816000010's guard does: anything but an array — NULL included — matches
-- nothing, and so does a predicate whose path is not a non-empty array of strings. The empty array
-- is the one spelling of "no predicates", and matches every artifact, content or none.
CREATE FUNCTION _temper_artifact_ask_match(p_content jsonb, p_predicates jsonb)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(jsonb_typeof(p_predicates) = 'array', false)
       AND NOT EXISTS (
             SELECT 1
               FROM jsonb_array_elements(
                      CASE jsonb_typeof(p_predicates)
                        WHEN 'array' THEN p_predicates ELSE '[]'::jsonb END) AS q
              CROSS JOIN LATERAL (
                SELECT CASE WHEN jsonb_typeof(q->'path') = 'array' THEN
                         CASE WHEN jsonb_array_length(q->'path') > 0
                               AND NOT EXISTS (
                                     SELECT 1 FROM jsonb_array_elements(q->'path') AS e
                                      WHERE jsonb_typeof(e) <> 'string')
                              THEN ARRAY(SELECT t.segment
                                           FROM jsonb_array_elements_text(q->'path')
                                                WITH ORDINALITY AS t(segment, n)
                                          ORDER BY t.n)
                         END
                       END AS segments) AS p
              CROSS JOIN LATERAL (
                SELECT _temper_artifact_get(p_content, p.segments) AS value) AS at
              WHERE NOT COALESCE(
                      at.value IS NOT NULL
                      AND _temper_property_op_match(
                            q->'op'->>'op',
                            CASE jsonb_typeof(q->'op'->'values')
                              WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END,
                            q->'op'->>'direction',
                            q->'op'->'value',
                            at.value),
                      false))
$$;

-- The governed population of a shape's lineage, within the caller's visible set: every live artifact
-- of the shape's family on a resource homed in the shape's home, with the shape in force for it (the
-- lineage's live row) and its content hash. `_data_artifact_shape_in_force`, set-based.
CREATE FUNCTION __temper_ungated_governed_artifacts(p_visible_ids uuid[], p_shape_id uuid)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text)
LANGUAGE sql STABLE AS $$
    SELECT a.id, a.resource_id, live.id, live.shape_version, a.content_hash
      FROM kb_data_artifact_shapes s
      JOIN kb_data_artifact_shapes live
        ON live.home_anchor_table = s.home_anchor_table
       AND live.home_anchor_id    = s.home_anchor_id
       AND live.kind_owner_table  = s.kind_owner_table
       AND live.kind_owner_id     = s.kind_owner_id
       AND live.artifact_kind     = s.artifact_kind
       AND NOT live.is_folded
      JOIN kb_resource_homes h
        ON h.anchor_table = s.home_anchor_table
       AND h.anchor_id    = s.home_anchor_id
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id
      JOIN kb_data_artifacts a
        ON a.resource_id      = h.resource_id
       AND a.kind_owner_table = s.kind_owner_table
       AND a.kind_owner_id    = s.kind_owner_id
       AND a.artifact_kind    = s.artifact_kind
       AND NOT a.is_folded
     WHERE s.id = p_shape_id
$$;

-- The governed population with each artifact's shape state, decided by the staleness triple
-- (shape_id, shape_version, content_hash) against the shape in force. `_data_artifact_shape_state`,
-- set-based. `never_declared` cannot occur: a governed artifact has a shape in force.
CREATE FUNCTION __temper_ungated_governed_artifact_states(p_visible_ids uuid[], p_shape_id uuid)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               content_hash text, shape_state text)
LANGUAGE sql STABLE AS $$
    SELECT g.artifact_id, g.resource_id, g.shape_id, g.shape_version, g.content_hash,
           CASE WHEN v.artifact_id IS NULL THEN 'declared_not_yet_checked'
                WHEN v.satisfied           THEN 'declared_satisfied'
                ELSE                            'declared_not_satisfied'
           END
      FROM __temper_ungated_governed_artifacts(p_visible_ids, p_shape_id) AS g
      LEFT JOIN kb_data_artifact_verdicts v
             ON v.artifact_id   = g.artifact_id
            AND v.shape_id      = g.shape_id
            AND v.shape_version = g.shape_version
            AND v.content_hash  = g.content_hash
$$;

COMMENT ON INDEX idx_kb_data_artifacts_family IS
    'Live artifacts by owner-qualified family: the governance relations'' way in from the family, beside idx_kb_resource_homes_anchor from the home (20261103000000).';
COMMENT ON FUNCTION _temper_artifact_get(jsonb, text[]) IS
    'The value at a path of object keys, stepping with -> and requiring an object before each step; NULL when absent. Never #>, which reads a numeric segment as an array index. An artifact ask''s path (20261103000000).';
COMMENT ON FUNCTION _temper_artifact_ask_match(jsonb, jsonb) IS
    'Whether content satisfies every predicate of one artifact ask: presence at each path conjoined with _temper_property_op_match. Fails closed: a non-array predicate list, or a path that is not a non-empty array of strings, matches nothing; [] matches everything. Differential witness: artifact_ask_parity.rs (20261103000000).';
COMMENT ON FUNCTION __temper_ungated_governed_artifacts(uuid[], uuid) IS
    'UNGATED. The live artifacts governed by a shape''s lineage — its home and family, read from the named row, folded or not — on resources in p_visible_ids, with the shape in force and the content hash. Set-based _data_artifact_shape_in_force, held to it by artifact_ask_parity.rs (20261103000000).';
COMMENT ON FUNCTION __temper_ungated_governed_artifact_states(uuid[], uuid) IS
    'UNGATED. __temper_ungated_governed_artifacts with each row''s shape state by the staleness triple. Set-based _data_artifact_shape_state, held to it by artifact_ask_parity.rs (20261103000000).';

SELECT declare_migration(
    20261103000000,
    'additive',
    'New objects only: the index idx_kb_data_artifacts_family and the functions _temper_artifact_get, _temper_artifact_ask_match, __temper_ungated_governed_artifacts and __temper_ungated_governed_artifact_states. No existing object, signature or default changes, and no deployed binary calls any of them.'
);
