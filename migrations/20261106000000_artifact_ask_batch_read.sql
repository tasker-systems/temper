-- The batch arm of asking data artifacts: the matching artifacts themselves, whole, in bounded
-- pages ordered by (created, id). The counted set's body becomes one function that also takes a
-- keyset position, a row limit and matched-only; the counted set delegates to it, and a gated page
-- and a gated population read it.
-- Spec: temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md §2.3, §6.1,
-- §7.2–§7.4.
--
-- What a later edit must not break:
--   1. ONE BODY. `__temper_ungated_artifact_ask_rows` is the only place spec §6.1's conjuncts meet
--      governance (the rule of 20261104000000 point 2); the counted set, the page and the counts all
--      read it. Its guards — the shape row by id, the home re-check once per call, include_folded —
--      stay in it, never in a caller. Its EXECUTE text is constant and every value rides USING.
--   2. THE MATCHER AND THE CONTENT READ RUN ABOVE THE SORT. Rows are taken in (created, id) order
--      on cheap columns inside a subquery fenced by OFFSET 0; `matched` is computed in a LATERAL
--      subquery, also fenced, per row of that order. Under LIMIT a page stops reading content once
--      it has its rows. Unfenced, the planner may evaluate the matcher before sorting, over the
--      whole home: artifact_ask_batch_read.rs's plan witness fails that.
--   3. THE KEYSET IS A ROW COMPARISON, both halves or neither (20261102000000 point 3): half a
--      position answers nothing. A NULL limit is the whole set — the counted set's call only.
--   4. CONTENT OVER THE PAGE BUDGET NEVER LEAVES SQL. The page decides served, oversized and the
--      row that ends the page from content_bytes in one window over at most limit + 1 rows, and
--      reads content for served rows alone.
--   5. THE GATED FUNCTIONS BUILD THE VISIBLE SET ONCE, COALESCEd to ARRAY[]::uuid[] (the array_agg
--      fall-open scar, 20260820000030), and restate no conjunct.
--   6. NO DEFAULT on any parameter.
--
-- Additive: three CREATE FUNCTION; the counted set CREATE OR REPLACEd at its byte-identical
-- signature and return columns. No DROP.

-- ── The one body ────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION __temper_ungated_artifact_ask_rows(
    p_visible_ids   uuid[],
    p_reader        uuid,
    p_ask           jsonb,
    p_after_created timestamptz,
    p_after_id      uuid,
    p_limit         bigint,
    p_matched_only  boolean)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               shape_state text, matched boolean, created timestamptz, content_bytes bigint)
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
        SELECT o.artifact_id, o.resource_id, o.shape_id, o.shape_version, o.shape_state,
               m.matched, o.created, o.content_bytes
          FROM (SELECT a.id AS artifact_id, g.resource_id, g.shape_id, g.shape_version,
                       g.shape_state, a.created, a.content_bytes
                  FROM __temper_ungated_governed_artifact_states_in($1, $2, $3, $4, $5, $6) AS g
                  JOIN kb_resources_live lv ON lv.id = g.resource_id
                  JOIN kb_data_artifacts a ON a.id = g.artifact_id
                 WHERE jsonb_typeof($7->'intent') = 'array'
                   AND $7->'intent' @> jsonb_build_array(a.intent)
                   AND (($8 IS NULL AND $9 IS NULL)
                        OR ($8 IS NOT NULL AND $9 IS NOT NULL
                            AND (a.created, a.id) > ($8, $9)))
                 ORDER BY a.created, a.id
                OFFSET 0) AS o
         CROSS JOIN LATERAL (
                SELECT COALESCE(jsonb_typeof($7->'shape_states') = 'array'
                                AND $7->'shape_states' @> jsonb_build_array(o.shape_state)
                                AND _temper_artifact_ask_match(
                                      (SELECT c.content FROM kb_data_artifact_content c
                                        WHERE c.artifact_id = o.artifact_id),
                                      $7->'predicates'),
                                false) AS matched
                OFFSET 0) AS m
         WHERE NOT $11 OR m.matched
         ORDER BY o.created, o.artifact_id
         LIMIT $10
    $q$
    USING p_visible_ids, v_shape.home_anchor_table::text, v_shape.home_anchor_id,
          v_shape.kind_owner_table::text, v_shape.kind_owner_id, v_shape.artifact_kind, p_ask,
          p_after_created, p_after_id, p_limit, p_matched_only;
END;
$$;

-- ── The counted set, delegating ─────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION __temper_ungated_artifact_ask_population(
    p_visible_ids uuid[],
    p_reader      uuid,
    p_ask         jsonb)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               shape_state text, matched boolean)
LANGUAGE sql STABLE AS $$
    SELECT r.artifact_id, r.resource_id, r.shape_id, r.shape_version, r.shape_state, r.matched
      FROM __temper_ungated_artifact_ask_rows(p_visible_ids, p_reader, p_ask,
                                              NULL::timestamptz, NULL::uuid, NULL::bigint,
                                              false) AS r
$$;

-- ── The gated page ──────────────────────────────────────────────────────────────────────────────
-- One window over at most p_limit + 1 matched rows, in order. `fitted` is the running total of the
-- content of every row that fits the budget alone, so at a row that does not it is the total before
-- it. A row is examined while it is within the limit and `fitted` is within the budget: a row over
-- the budget alone is examined (oversized) and the page goes on; the first row that fits alone but
-- not the remainder, and every row after it, is not examined. A row is served when it is examined,
-- fits alone, and the total through it is within the budget — so no served content is past it.
CREATE FUNCTION artifacts_matching_ask(
    p_principal      uuid,
    p_ask            jsonb,
    p_after_created  timestamptz,
    p_after_id       uuid,
    p_limit          bigint,
    p_content_budget bigint)
RETURNS TABLE (artifact_id uuid, resource_id uuid, kind_owner_table varchar, kind_owner_id uuid,
               artifact_kind text, intent text, precedence double precision, content_hash text,
               content_bytes bigint, shape_state text, is_folded boolean, created timestamptz,
               examined boolean, served boolean, content jsonb)
LANGUAGE sql STABLE AS $$
    SELECT w.artifact_id, a.resource_id, a.kind_owner_table, a.kind_owner_id, a.artifact_kind,
           a.intent, a.precedence, a.content_hash, w.content_bytes, w.shape_state, a.is_folded,
           w.created, w.examined, w.served,
           CASE WHEN w.served
                THEN (SELECT c.content FROM kb_data_artifact_content c
                       WHERE c.artifact_id = w.artifact_id)
           END
      FROM (SELECT e.artifact_id, e.shape_state, e.created, e.content_bytes, e.examined,
                   e.examined AND e.fits AND e.fitted <= p_content_budget AS served
              FROM (SELECT b.artifact_id, b.shape_state, b.created, b.content_bytes, b.fits,
                           b.fitted,
                           b.n <= p_limit AND b.fitted <= p_content_budget AS examined
                      FROM (SELECT r.artifact_id, r.shape_state, r.created, r.content_bytes,
                                   r.content_bytes <= p_content_budget AS fits,
                                   row_number() OVER ordered AS n,
                                   sum(CASE WHEN r.content_bytes <= p_content_budget
                                            THEN r.content_bytes ELSE 0 END) OVER ordered AS fitted
                              FROM __temper_ungated_artifact_ask_rows(
                                     COALESCE((SELECT array_agg(v.resource_id)
                                                 FROM resources_visible_to(p_principal) v),
                                              ARRAY[]::uuid[]),
                                     p_principal, p_ask, p_after_created, p_after_id,
                                     p_limit + 1, true) AS r
                             WHERE p_limit >= 1 AND p_content_budget >= 0
                            WINDOW ordered AS (ORDER BY r.created, r.artifact_id
                                               ROWS UNBOUNDED PRECEDING)) AS b) AS e) AS w
      JOIN kb_data_artifacts a ON a.id = w.artifact_id
     ORDER BY w.created, w.artifact_id
$$;

-- ── The gated population ────────────────────────────────────────────────────────────────────────
CREATE FUNCTION artifact_ask_counts(p_principal uuid, p_ask jsonb)
RETURNS TABLE (in_force_shape_id uuid, in_force_shape_version int,
               declared_satisfied bigint, declared_not_satisfied bigint,
               declared_not_yet_checked bigint, matched bigint)
LANGUAGE sql STABLE AS $$
    SELECT c.in_force_shape_id, c.in_force_shape_version, c.declared_satisfied,
           c.declared_not_satisfied, c.declared_not_yet_checked, c.matched
      FROM __temper_ungated_artifact_ask_counts(
             COALESCE((SELECT array_agg(v.resource_id) FROM resources_visible_to(p_principal) v),
                      ARRAY[]::uuid[]),
             p_principal, p_ask) AS c
$$;

COMMENT ON FUNCTION __temper_ungated_artifact_ask_rows(uuid[], uuid, jsonb, timestamptz, uuid, bigint, boolean) IS
    'UNGATED. The one body of an artifact ask''s counted set: every live artifact the asked shape''s lineage governs (its home and family, read from the named row by id, through __temper_ungated_governed_artifact_states_in), on a resource in p_visible_ids (NULL admits none) that is live and fully ingested (kb_resources_live), while p_reader can read the shape''s home (anchor_readable_by_profile, checked once per call before anything is read) and the artifact''s intent is among the ask''s; include_folded true admits nothing. One row per artifact with its shape in force, shape_state, matched (its state is among the ask''s shape_states and its content satisfies every predicate, _temper_artifact_ask_match), created and content_bytes, in (created, id) order strictly after (p_after_created, p_after_id) — from the start when both are NULL, nothing when one is — at most p_limit rows (NULL: all), only matched rows when p_matched_only. plpgsql because the artifact query runs by EXECUTE … USING with the home and family as values, so it is planned with their real sizes. The order is taken on cheap columns behind an OFFSET 0 fence and the matcher and the content read run above it, so a limited call reads content only for the rows it examines. p_ask is one element of ValidatedAsk::bind. The caller owes the verdict and the reader; this applies every other conjunct of spec 2026-10-19-asking-artifacts-by-what-they-are §6.1 (20261106000000).';

COMMENT ON FUNCTION __temper_ungated_artifact_ask_population(uuid[], uuid, jsonb) IS
    'UNGATED. The counted set of one artifact ask: every governed row with its shape in force, shape_state and matched, as __temper_ungated_artifact_ask_rows defines them, which this delegates to with no position, no limit and every row, matched or not. The selection core''s asks conjunct and __temper_ungated_artifact_ask_counts read it. The caller owes the verdict (p_visible_ids, NULL admits none) and the reader (20261104000000, 20261106000000).';

COMMENT ON FUNCTION artifacts_matching_ask(uuid, jsonb, timestamptz, uuid, bigint, bigint) IS
    'One page of the artifacts matching one ask (one element of ValidatedAsk::bind), for p_principal: its own resources_visible_to, built once, handed to __temper_ungated_artifact_ask_rows with p_principal as reader, matched rows only, at most p_limit + 1 in (created, id) order after the position. examined is within p_limit and the page''s content budget; served is examined and within the budget alone (content carried), an examined row over it is oversized (no content). The first unexamined row, if any, is where the next page starts. Nothing for a NULL or non-positive p_limit or a NULL or negative budget (20261106000000).';

COMMENT ON FUNCTION artifact_ask_counts(uuid, jsonb) IS
    'The population of one ask for p_principal: __temper_ungated_artifact_ask_counts over its own resources_visible_to, built once (an empty set admits nothing), with p_principal as reader (20261106000000).';

SELECT declare_migration(
    20261106000000,
    'additive',
    'New functions __temper_ungated_artifact_ask_rows (the counted set''s body, widened with a keyset position, a row limit and matched-only), artifacts_matching_ask (a gated page of matching artifacts) and artifact_ask_counts (a gated population), with no DEFAULT on any parameter. __temper_ungated_artifact_ask_population is CREATE OR REPLACEd at its byte-identical signature and return columns to delegate to the new body with no position, no limit and every row, so it answers as before and a deployed binary reading it through the selection core or the counts keeps its answer. No DROP.'
);
