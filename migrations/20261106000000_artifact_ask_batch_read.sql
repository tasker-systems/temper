-- The batch arm of asking data artifacts: the matching artifacts themselves, whole, in bounded
-- pages ordered by (created, id). The counted set's body becomes one function that also takes a
-- keyset position, a row limit, matched-only and a read budget; the counted set delegates to it,
-- and a gated page and a gated population read it.
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
--   3. THE READ BUDGET IS CHARGED ON THE CHEAP COLUMNS, BEHIND THE SAME FENCE. Each candidate after
--      the position is charged its content_bytes plus a fixed per-candidate amount, as a running sum
--      in (created, id) order inside the fenced subquery; a candidate is read — matched, its content
--      fetched — only while the charge before it is under the budget, so the first candidate is
--      always read and a page always moves forward. When the budget ends the scan, the last
--      candidate read is returned marked `frontier`, matched or not, so a page can say where to
--      continue. A NULL budget reads every candidate and marks none: the counted set's call, which
--      therefore answers exactly as before. A matched-only call with a NULL budget or charge raises.
--   4. THE KEYSET IS A ROW COMPARISON, both halves or neither (20261102000000 point 3): half a
--      position answers nothing. A NULL limit is the whole set — the counted set's call only.
--   5. THE BUDGET BOUNDS THE WHOLE PAGE, AND CONTENT OVER IT NEVER LEAVES SQL. Every matched row the
--      page examines is charged against it: a served row its content_bytes plus its metadata (each
--      text column it carries, measured as the JSON string the page writes, plus p_row_bytes for the
--      rest), an oversized row p_row_bytes. The family name has no length cap and repeats on every
--      row, so a content-only charge cannot hold the response cap. One window over the body's rows
--      decides served, oversized and the row that ends the page; content, and the text columns, are
--      projected for served rows alone. The charge estimates what a served row writes: storage can
--      print a number longer than it was committed, so the caller (temper-services'
--      artifact_ask_read) holds the page to the budget again on the bytes it writes.
--   6. THE GATED FUNCTIONS BUILD THE VISIBLE SET ONCE, COALESCEd to ARRAY[]::uuid[] (the array_agg
--      fall-open scar, 20260820000030), and restate no conjunct.
--   7. NO DEFAULT on any parameter.
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
    p_matched_only  boolean,
    p_read_budget   bigint,
    p_read_charge   bigint)
RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int,
               shape_state text, matched boolean, created timestamptz, content_bytes bigint,
               frontier boolean)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_shape kb_data_artifact_shapes%ROWTYPE;
BEGIN
    IF p_matched_only IS NOT false AND (p_read_budget IS NULL OR p_read_charge IS NULL) THEN
        RAISE EXCEPTION 'a matched-only artifact ask read needs a read budget and charge';
    END IF;
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
               m.matched, o.created, o.content_bytes, o.frontier
          FROM (SELECT a.id AS artifact_id, g.resource_id, g.shape_id, g.shape_version,
                       g.shape_state, a.created, a.content_bytes,
                       sum(a.content_bytes + $13) OVER read_order - (a.content_bytes + $13)
                           AS read_before,
                       COALESCE(lead(a.id) OVER read_order IS NOT NULL
                                AND sum(a.content_bytes + $13) OVER read_order >= $12,
                                false) AS frontier
                  FROM __temper_ungated_governed_artifact_states_in($1, $2, $3, $4, $5, $6) AS g
                  JOIN kb_resources_live lv ON lv.id = g.resource_id
                  JOIN kb_data_artifacts a ON a.id = g.artifact_id
                 WHERE jsonb_typeof($7->'intent') = 'array'
                   AND $7->'intent' @> jsonb_build_array(a.intent)
                   AND (($8 IS NULL AND $9 IS NULL)
                        OR ($8 IS NOT NULL AND $9 IS NOT NULL
                            AND (a.created, a.id) > ($8, $9)))
                WINDOW read_order AS (ORDER BY a.created, a.id ROWS UNBOUNDED PRECEDING)
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
         WHERE ($12 IS NULL OR o.read_before < $12)
           AND (NOT $11 OR m.matched OR o.frontier)
         ORDER BY o.created, o.artifact_id
         LIMIT $10
    $q$
    USING p_visible_ids, v_shape.home_anchor_table::text, v_shape.home_anchor_id,
          v_shape.kind_owner_table::text, v_shape.kind_owner_id, v_shape.artifact_kind, p_ask,
          p_after_created, p_after_id, p_limit, p_matched_only, p_read_budget, p_read_charge;
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
                                              false, NULL::bigint, NULL::bigint) AS r
$$;

-- ── The gated page ──────────────────────────────────────────────────────────────────────────────
-- One window over the body's rows, in order: at most p_limit + 1 matched rows, and the read
-- frontier when the read budget ended the body's scan first. A matched row's `charge` is what it
-- costs the page: content_bytes plus metadata_bytes when it fits the budget alone (content and
-- metadata), else p_row_bytes, the size of the oversized entry that names it. An unmatched frontier
-- costs nothing and counts toward no limit: it is a position, never a row of the page. `n` counts
-- matched rows and `spent` totals the charges. A row is examined while `n` is within the limit and
-- `spent` within the budget, so a matched row that does not fit alone is examined (oversized) and
-- the page goes on, and the first that fits alone but not what is left ends the page with every row
-- after it. A row is served when it is examined, matched and fits alone, so no served row is past
-- the budget as charged. A budget below p_row_bytes reads nothing, so the first matching row is
-- always examined.
CREATE FUNCTION artifacts_matching_ask(
    p_principal     uuid,
    p_ask           jsonb,
    p_after_created timestamptz,
    p_after_id      uuid,
    p_limit         bigint,
    p_page_budget   bigint,
    p_row_bytes     bigint,
    p_read_budget   bigint,
    p_read_charge   bigint)
RETURNS TABLE (artifact_id uuid, resource_id uuid, kind_owner_table varchar, kind_owner_id uuid,
               artifact_kind text, intent text, precedence double precision, content_hash text,
               content_bytes bigint, shape_state text, is_folded boolean, created timestamptz,
               matched boolean, frontier boolean, examined boolean, served boolean, content jsonb)
LANGUAGE sql STABLE AS $$
    SELECT w.artifact_id, w.resource_id,
           CASE WHEN w.served THEN w.kind_owner_table END,
           w.kind_owner_id,
           CASE WHEN w.served THEN w.artifact_kind END,
           CASE WHEN w.served THEN w.intent END,
           w.precedence,
           CASE WHEN w.served THEN w.content_hash END,
           w.content_bytes,
           CASE WHEN w.served THEN w.shape_state END,
           w.is_folded, w.created, w.matched, w.frontier, w.examined, w.served,
           CASE WHEN w.served
                THEN (SELECT c.content FROM kb_data_artifact_content c
                       WHERE c.artifact_id = w.artifact_id)
           END
      FROM (SELECT e.*, e.examined AND e.matched AND e.fits AS served
              FROM (SELECT b.*, b.n <= p_limit AND b.spent <= p_page_budget AS examined
                      FROM (SELECT m.*,
                                   count(*) FILTER (WHERE m.matched) OVER ordered AS n,
                                   sum(m.charge) OVER ordered AS spent
                              FROM (SELECT k.*,
                                           k.content_bytes + k.metadata_bytes <= p_page_budget
                                               AS fits,
                                           CASE WHEN NOT k.matched THEN 0
                                                WHEN k.content_bytes + k.metadata_bytes
                                                     <= p_page_budget
                                                THEN k.content_bytes + k.metadata_bytes
                                                ELSE p_row_bytes END AS charge
                                      FROM (SELECT r.artifact_id, r.resource_id,
                                                   d.kind_owner_table, d.kind_owner_id,
                                                   d.artifact_kind, d.intent, d.precedence,
                                                   d.content_hash, r.content_bytes, r.shape_state,
                                                   d.is_folded, r.created, r.matched, r.frontier,
                                                   (p_row_bytes
                                                    + octet_length(to_json(d.kind_owner_table)::text)
                                                    + octet_length(to_json(d.artifact_kind)::text)
                                                    + octet_length(to_json(d.intent)::text)
                                                    + octet_length(to_json(d.content_hash)::text)
                                                    + octet_length(to_json(r.shape_state)::text))
                                                       AS metadata_bytes
                                              FROM __temper_ungated_artifact_ask_rows(
                                                     COALESCE((SELECT array_agg(v.resource_id)
                                                                 FROM resources_visible_to(p_principal) v),
                                                              ARRAY[]::uuid[]),
                                                     p_principal, p_ask, p_after_created, p_after_id,
                                                     p_limit + 1, true, p_read_budget,
                                                     p_read_charge) AS r
                                              JOIN kb_data_artifacts d ON d.id = r.artifact_id
                                             WHERE p_limit >= 1 AND p_row_bytes >= 0
                                               AND p_page_budget >= p_row_bytes
                                               AND p_read_budget >= 1
                                               AND p_read_charge >= 1) AS k) AS m
                            WINDOW ordered AS (ORDER BY m.created, m.artifact_id
                                               ROWS UNBOUNDED PRECEDING)) AS b) AS e) AS w
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

COMMENT ON FUNCTION __temper_ungated_artifact_ask_rows(uuid[], uuid, jsonb, timestamptz, uuid, bigint, boolean, bigint, bigint) IS
    'UNGATED. The one body of an artifact ask''s counted set: every live artifact the asked shape''s lineage governs (its home and family, read from the named row by id, through __temper_ungated_governed_artifact_states_in), on a resource in p_visible_ids (NULL admits none) that is live and fully ingested (kb_resources_live), while p_reader can read the shape''s home (anchor_readable_by_profile, checked once per call before anything is read) and the artifact''s intent is among the ask''s; include_folded true admits nothing. One row per artifact with its shape in force, shape_state, matched (its state is among the ask''s shape_states and its content satisfies every predicate, _temper_artifact_ask_match), created and content_bytes, in (created, id) order strictly after (p_after_created, p_after_id) — from the start when both are NULL, nothing when one is — at most p_limit rows (NULL: all), only matched rows when p_matched_only. p_read_budget bounds what one call reads: each candidate after the position is charged its content_bytes plus p_read_charge, and a candidate is read (matched, its content fetched) only while the charge before it is under the budget, so the first is always read; when the budget ends the scan, the last candidate read is returned with frontier true, matched or not, and is the one unmatched row a matched-only call can return. NULL reads every candidate and marks no frontier, which is the counted set''s call; a matched-only call with a NULL p_read_budget or p_read_charge raises rather than reading without bound. plpgsql because the artifact query runs by EXECUTE … USING with the home and family as values, so it is planned with their real sizes. The order and the read charge are taken on cheap columns behind an OFFSET 0 fence and the matcher and the content read run above it, so a limited or budgeted call reads content only for the candidates it reads. p_ask is one element of ValidatedAsk::bind. The caller owes the verdict and the reader; this applies every other conjunct of spec 2026-10-19-asking-artifacts-by-what-they-are §6.1 (20261106000000).';

COMMENT ON FUNCTION __temper_ungated_artifact_ask_population(uuid[], uuid, jsonb) IS
    'UNGATED. The counted set of one artifact ask: every governed row with its shape in force, shape_state and matched, as __temper_ungated_artifact_ask_rows defines them, which this delegates to with no position, no limit, every row matched or not, and no read budget. The selection core''s asks conjunct and __temper_ungated_artifact_ask_counts read it. The caller owes the verdict (p_visible_ids, NULL admits none) and the reader (20261104000000, 20261106000000).';

COMMENT ON FUNCTION artifacts_matching_ask(uuid, jsonb, timestamptz, uuid, bigint, bigint, bigint, bigint, bigint) IS
    'One page of the artifacts matching one ask (one element of ValidatedAsk::bind), for p_principal: its own resources_visible_to, built once, handed to __temper_ungated_artifact_ask_rows with p_principal as reader, matched rows only, at most p_limit + 1 in (created, id) order after the position, under the read budget p_read_budget at p_read_charge per candidate. p_page_budget bounds the whole page as charged, an estimate of what it writes: a matched row that fits it alone is charged its content_bytes and its metadata (each text column as the JSON string the page writes, plus p_row_bytes), any other matched row p_row_bytes. examined is within p_limit matched rows and the running charge within the budget; served is examined, matched and fitting alone (content and text columns carried); an examined matched row that does not fit alone is oversized (ids, created and size only). The read frontier — the last candidate the body read when its read budget ended the scan — is returned with frontier true; unmatched, it is charged nothing, counts toward no limit and is neither served nor oversized. The earliest of three ends the page: the row limit, the page budget, and the read frontier. The last examined row is where the next page starts; when every row is examined and none is the frontier, the set is exhausted. Nothing for a NULL or non-positive p_limit, a NULL or negative p_row_bytes, a budget that is NULL or below p_row_bytes, or a non-positive p_read_budget or p_read_charge; a NULL p_read_budget or p_read_charge is never read without bound, since the body raises if it is reached with one (20261106000000).';

COMMENT ON FUNCTION artifact_ask_counts(uuid, jsonb) IS
    'The population of one ask for p_principal: __temper_ungated_artifact_ask_counts over its own resources_visible_to, built once (an empty set admits nothing), with p_principal as reader. Exact counts over the whole counted set: no read budget bounds them (20261106000000).';

SELECT declare_migration(
    20261106000000,
    'additive',
    'New functions __temper_ungated_artifact_ask_rows (the counted set''s body, widened with a keyset position, a row limit, matched-only and a read budget that marks the last candidate it read), artifacts_matching_ask (a gated page of matching artifacts) and artifact_ask_counts (a gated population), with no DEFAULT on any parameter. __temper_ungated_artifact_ask_population is CREATE OR REPLACEd at its byte-identical signature and return columns to delegate to the new body with no position, no limit, every row and no read budget, so it answers as before and a deployed binary reading it through the selection core or the counts keeps its answer. No DROP.'
);
