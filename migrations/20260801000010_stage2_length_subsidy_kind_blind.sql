-- Stage-2 pays a subsidy for document length — remove it KIND-BLIND (goal
-- 019fb559-7191-75a3-99d4-879090c60e94, task 019fbb8d-99b7-7ff3-8efb-0d2192b29b04; the
-- `distilled-content-survives-raw` and `distilled-content-never-wholly-displaced` clauses).
--
-- THE DEFECT. `unified_search`'s blend pays for document LENGTH through two independent channels,
-- neither of which is about relevance. Distillation is length reduction by definition, so the ranker
-- taxes distilled material for being distilled. Both channels are measured on the production corpus
-- (research 019fbb58-a3ee-7961-86e6-3e8922c42819 and this task's re-measurement):
--
--   1. `fts_norm` = ts_rank(..., 32). Flag 32 is rank/(rank+1) and applies NO length normalization
--      whatsoever, so a document's lexical score is free of any length correction while the `@@`
--      match itself rises with length. Raw enters the FTS arm 5.6x more often than distilled.
--   2. `vec_norm` = 1 - MIN(dist)/2 over a resource's chunks. This is an ORDER STATISTIC: N chunks is
--      N draws at the minimum, so expected best-similarity rises with N for reasons that have nothing
--      to do with content. Raw carries 10.88 chunks (p50 7, max 157); distilled carries 1.89
--      (p50 1, max 12).
--
-- At equal length the two classes are statistically indistinguishable — established twice, by two
-- methods, at two layers (per-resource controlling chunk count: 0.8313 vs 0.8308 in the 4-7 bucket;
-- per-anchor controlling corpus size: material gap -0.0566 -> -0.0205). The deficit is length and
-- volume, not semantics.
--
-- WHY KIND-BLIND, AND NOT PER-KIND. The obvious fix — normalize Stage-2's arms per anchor kind, by
-- analogy with Stage-1's per-kind percent_rank (T7, 20260712000090) — is REJECTED, and the analogy is
-- what makes it tempting. Stage-1 normalizes SALIENCE, an intrinsic property of a region that is
-- independent of the query, and deliberately leaves the relevance term cross-kind comparable; that is
-- what lets the query decide. `fts_norm` and `vec_norm` ARE the relevance terms. Rank-equalizing them
-- per kind would make the best distilled document score ~1.0 whatever its absolute relevance, so the
-- returned mix would become kind-balanced BY CONSTRUCTION regardless of what was asked — a standing
-- kind preference at the ranking layer, and a stronger one than the anchor-kind prior it would sit
-- beside. That is what `cross-kind-outcome-is-earned` forbids.
--
-- Both corrections below are kind-blind: neither formula asks, or can ask, what kind of anchor homes a
-- document. Distilled material benefits BECAUSE IT IS SHORT, not because it is distilled — earned in
-- exactly the sense that clause requires.
--
-- ── Channel 1: ts_rank flag 32 -> 33 (add |1, log-length normalization) ────────────────────────────
--
-- Flag 1 divides the rank by 1 + log(document length); 32 (rank/(rank+1)) is retained, so the output
-- stays in [0,1) and `search_surface_a`'s range assertion is unaffected.
--
-- WHY 1 AND NOT 2. Flag 2 (divide by raw document length) was the inherited recommendation, and it is
-- REJECTED on measurement. `fts_norm` enters the blend RAW and unweighted (COALESCE(f.fts_norm, 0) at
-- w_fts = 1.0) against `vec_norm` ~= 0.81. Under flag 2 the entire FTS arm collapses to a mean of
-- 0.0038 (distilled) / 0.0007 (raw) — roughly 1% of its current magnitude — which does not normalize
-- the arm so much as DELETE it, reproducing a `w_fts = 0` blend by accident rather than by intent.
-- Measured end to end on 20 real queries, flags 33, 48 and 34 land within 0.5 points of each other on
-- distilled share despite spanning 20x in magnitude, which is what identifies the effect as arm
-- attenuation rather than length correction. Flag 33 buys the same outcome while leaving the lexical
-- signal large enough to still participate in the blend (mean 0.076 / 0.052).
--
-- ── Channel 2: shrink the order statistic toward the mean, in proportion to the draws ──────────────
--
--     vec_norm = 1 - (MIN + (AVG - MIN) * (1 - 1/sqrt(N))) / 2
--
-- The correction is the (1 - 1/sqrt(N)) factor, and its shape is the point:
--
--   * At N = 1 the factor is 0 and the expression reduces EXACTLY to the shipped `1 - MIN/2`. A
--     single-chunk resource is bit-for-bit unchanged by this migration. The change is a no-op at the
--     short end and grows with the number of draws that produced the minimum — which is precisely the
--     quantity the order statistic was being paid for.
--   * It preserves passage orientation. A long document with one genuinely excellent chunk still
--     scores well above its own mean; it simply no longer scores at its unpenalized minimum. This is
--     why the flat length-controlled alternatives were not taken: replacing MIN with AVG outright
--     penalizes a long document for containing anything besides the answer.
--   * `count(*)` counts the draws that actually competed for the minimum — every current chunk in the
--     scoped branch, and in the unscoped branch only those that reached the global top-k pool. That is
--     the correct denominator in both cases, and it is why the same expression is right in each branch
--     despite the branches differing in what they admit.
--
-- Measured on 20 real queries with REAL query vectors (via crates/temper-ingest/examples/
-- query_vectors.rs, PR #604 — the affordance that made an end-to-end vector counterfactual possible
-- at all; the source research could only reach document vectors and said so), recomputing the whole
-- blend from the same rows, differentially, never two runs. Distilled share of top-10 / queries
-- returning zero distilled rows:
--
--   wayfind (--regions 3)      shipped 10.5% (13/20 zero)  ->  35.0% (3/20 zero)
--   plain search (no scope)    shipped 12.5% ( 7/20 zero)  ->  22.0% (4/20 zero)
--
-- The two channels dominate in OPPOSITE modes, which is why both are changed together and neither is
-- sufficient alone: in wayfind the vector arm is unbounded over the scope, so Channel 2 carries the
-- larger effect; in plain search resources compete for slots in a global top-k CHUNK pool, so a
-- one-chunk resource must win a slot outright and Channel 2 measured a 0.0-point change there. A
-- Channel-2-only fix would have looked like a large win and done nothing at all for plain search,
-- which is the majority of traffic.
--
-- WHAT THIS DOES NOT DO. It cannot reach material that never enters the scope. 15 of 385 regions in
-- the self-cognition map are ever selected, and 239 live distilled nodes are unreachable through the
-- region arm by any query (research 019fbb76-1a15-7e70-8ca9-74b23a0772a9, goal
-- 019fbb77-72a3-72e1-bbbd-13eb6aa64982). This improves the ranking of what was ADMITTED; region
-- formation is a separate question with a separate home.
--
-- SCORES REMAIN COMPARABLE ACROSS QUERIES. Both corrections are per-row functions of a single
-- document; neither is a window over the candidate set. Absolute magnitudes do change (the FTS arm
-- drops roughly 5x), so anything comparing a raw `combined_score` against a fixed threshold would be
-- affected — nothing in the tree does; the only assertions are `> 0.0`, which both corrections
-- preserve for any real match.

-- ── search_fts_candidates ─────────────────────────────────────────────────────────────────────────
-- Body copied VERBATIM from its LIVE deployed definition (pg_get_functiondef on temper-cloud/main,
-- 2026-08-01), which matches 20260714000001_ingest_state.sql. Re-deriving this by hand from an older
-- migration silently reverts `websearch_to_tsquery` back to `plainto_tsquery` and drops the
-- `ingest_state` gate. The ONLY edit is the rank flag, marked below.
CREATE OR REPLACE FUNCTION search_fts_candidates(p_principal uuid, p_query text)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
  SELECT r.id,
         -- 32 -> 33: adds flag 1 (divide by 1 + log(document length)).            -- ← CHANGED
         (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 33))::real
    FROM kb_resource_search_index si
    JOIN kb_resources r                       ON r.id = si.resource_id
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = r.id
   WHERE p_query IS NOT NULL AND p_query <> ''
     AND r.is_active
     AND r.ingest_state = 'complete'
     AND si.search_vector @@ websearch_to_tsquery('english', p_query);
$$;

-- ── search_vector_candidates ──────────────────────────────────────────────────────────────────────
-- Body copied VERBATIM from its LIVE deployed definition (pg_get_functiondef on temper-cloud/main,
-- 2026-08-01). Both branches are preserved exactly, including the deliberate asymmetry that the
-- unscoped branch applies `LIMIT p_k` inside `ann` and lands the visibility/active/complete predicates
-- AFTER it (applying them inside would force a seq-scan and defeat idx_kb_chunks_embedding), while the
-- scoped branch pre-filters into `scoped_res` and carries NO top-k. The ONLY edit in each branch is
-- the aggregate expression, marked below.
CREATE OR REPLACE FUNCTION search_vector_candidates(
  p_principal   uuid,
  p_emb         vector,
  p_k           int,
  p_context_id  uuid   DEFAULT NULL,
  p_scope_ids   uuid[] DEFAULT NULL)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_context_id IS NULL AND p_scope_ids IS NULL THEN
    RETURN QUERY
      WITH ann AS (
        SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
          FROM kb_chunks c
         WHERE p_emb IS NOT NULL AND c.is_current
         ORDER BY c.embedding <=> p_emb
         LIMIT p_k
      )
      -- Order statistic shrunk toward the mean by the draws that produced it.     -- ← CHANGED
      SELECT a.resource_id,
             (1.0 - (MIN(a.dist)
                     + (AVG(a.dist) - MIN(a.dist)) * (1.0 - 1.0 / sqrt(count(*)::float8))
                    ) / 2.0)::real
        FROM ann a
        JOIN kb_resources r                      ON r.id = a.resource_id AND r.is_active
                                                AND r.ingest_state = 'complete'
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
       GROUP BY a.resource_id;
  ELSE
    RETURN QUERY
      WITH scoped_res AS (
        SELECT v.resource_id AS id
          FROM resources_visible_to(p_principal) v
          JOIN kb_resources r ON r.id = v.resource_id AND r.is_active
                             AND r.ingest_state = 'complete'
         WHERE (p_context_id IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_homes h
                  WHERE h.resource_id = v.resource_id
                    AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
           AND (p_scope_ids IS NULL OR v.resource_id = ANY(p_scope_ids))
      ),
      ann AS (
        SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
          FROM kb_chunks c
          JOIN scoped_res s ON s.id = c.resource_id
         WHERE p_emb IS NOT NULL AND c.is_current
      )
      -- Order statistic shrunk toward the mean by the draws that produced it.     -- ← CHANGED
      SELECT a.resource_id,
             (1.0 - (MIN(a.dist)
                     + (AVG(a.dist) - MIN(a.dist)) * (1.0 - 1.0 / sqrt(count(*)::float8))
                    ) / 2.0)::real
        FROM ann a
       GROUP BY a.resource_id;
  END IF;
END;
$$;

-- Purely additive: two same-signature CREATE OR REPLACE bodies. No table, column, type or return
-- shape is touched, nothing is dropped, and a binary that does not carry this calls both functions
-- identically. Only the scores they return move.
SELECT declare_migration(
    20260801000010,
    'additive',
    'Stage-2 length subsidy removed kind-blind (goal 019fb559). search_fts_candidates ts_rank flag 32 -> 33 (log-length normalization); search_vector_candidates best-of-N shrunk toward the chunk mean by (1 - 1/sqrt(N)), an identity at N=1. Same signatures, same return shapes; ranking scores change.'
);
