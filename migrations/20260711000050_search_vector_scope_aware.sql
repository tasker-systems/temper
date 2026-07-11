-- Issue #358: the vector arm takes a GLOBAL top-100 ANN before the context/scope filter.
--
-- `unified_search`'s `vec` CTE called `search_vector_candidates(principal, emb, k=100)`, whose inner
-- `ann` CTE ordered ALL current chunks by distance and took the global top-100 — with no context or
-- scope predicate. The context/scope filter only ran later, in the `corpus` CTE. So a context whose
-- relevant chunks are not in the global top-100 for the query embedding contributed NOTHING from the
-- vector arm; a context-scoped search silently degraded to FTS(+graph)-only, and the larger the
-- instance the worse the starvation.
--
-- Fix — make `search_vector_candidates` scope-aware and branch on whether a scope is present:
--   • UNSCOPED (context_id IS NULL AND scope_ids IS NULL) → unchanged global HNSW over-fetch-then-
--     filter. The inner `ann` carries only the index's own predicate (is_current) + ORDER BY <=> LIMIT
--     so idx_kb_chunks_embedding engages; visibility/active filter AFTER (applying it inside the ANN
--     would force a seq-scan and defeat the index).
--   • SCOPED → restrict chunks to the scope's visible/active resources FIRST, then take an EXACT
--     best-per-resource distance over just that set. No HNSW, no global top-k, so a context's chunks
--     can no longer be starved out of the candidate set — perfect recall WITHIN scope by construction.
--     A scope is hundreds–low-thousands of chunks in practice, so the exact scan is sub-ms/low-ms; a
--     single pathologically huge context is the only latency edge (deferred to a perf pass — that
--     branch can later switch to a filtered HNSW + iterative scan).
--
-- plpgsql IF/ELSE (not a pure-SQL UNION) is deliberate: the two modes need structurally different
-- plans, and a UNION would execute BOTH branches — in the unscoped case the scoped branch's predicates
-- are all NULL-true, so it would exact-scan the ENTIRE corpus. IF/ELSE runs exactly one branch.
--
-- Sequencing: built ON TOP OF #357 (PR #368). `unified_search` keeps its exact 13-arg signature, so
-- this is a CREATE OR REPLACE (additive, no absence window under deploy skew) that carries BOTH #357
-- additions forward — `p_seed_only` AND the `seed_cand` CTE — changing ONLY the `vec` CTE to thread
-- context/scope into `search_vector_candidates`.

-- ── Scope-aware semantic candidates ──────────────────────────────────────────────────────────────
-- DROP+CREATE (arity change: +p_context_id, +p_scope_ids, both DEFAULT NULL). The defaults keep the
-- prior 3-arg call resolvable (the direct test caller and any skew binary bind through them → global
-- path = old behavior). No production binary calls this directly — only `unified_search`'s body, which
-- this migration replaces in the same transaction to pass the two new args.
DROP FUNCTION IF EXISTS search_vector_candidates(uuid, vector, int);

CREATE FUNCTION search_vector_candidates(
  p_principal   uuid,
  p_emb         vector,
  p_k           int,
  p_context_id  uuid   DEFAULT NULL,
  p_scope_ids   uuid[] DEFAULT NULL)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_context_id IS NULL AND p_scope_ids IS NULL THEN
    -- Unscoped: global HNSW top-k, then visibility/active filter. Best chunk per resource decides
    -- rank; vec_norm = 1 - dist/2 ∈ [0,1]. Unchanged from 20260626000002.
    RETURN QUERY
      WITH ann AS (
        SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
          FROM kb_chunks c
         WHERE p_emb IS NOT NULL AND c.is_current
         ORDER BY c.embedding <=> p_emb
         LIMIT p_k
      )
      SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
        FROM ann a
        JOIN kb_resources r                      ON r.id = a.resource_id AND r.is_active
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
       GROUP BY a.resource_id;
  ELSE
    -- Scoped (issue #358): restrict to the scope's visible/active resources FIRST, then exact
    -- best-per-resource distance over just that set — no HNSW, no global top-k. p_k is intentionally
    -- unused: every scoped resource with a current chunk contributes, and `unified_search` applies
    -- the final LIMIT after blending. Scope predicates mirror `unified_search`'s `corpus` CTE exactly
    -- (context = home-anchored to kb_contexts; scope = resource-id allowlist).
    RETURN QUERY
      WITH scoped_res AS (
        SELECT v.resource_id AS id
          FROM resources_visible_to(p_principal) v
          JOIN kb_resources r ON r.id = v.resource_id AND r.is_active
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
      SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
        FROM ann a
       GROUP BY a.resource_id;
  END IF;
END;
$$;

-- ── The aggregate: thread context/scope into the vector arm ───────────────────────────────────────
-- CREATE OR REPLACE — identical 13-arg signature (no DROP, no deploy-skew absence window). Body is the
-- 20260711000030 (#357) variant verbatim EXCEPT the `vec` CTE, which now passes p_context_id and
-- p_scope_ids so the ANN itself is scope-bounded. `p_seed_only` and the `seed_cand` CTE are preserved.
CREATE OR REPLACE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int, p_scope_ids uuid[], p_seed_only boolean DEFAULT false)
RETURNS TABLE (resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 1.0::float8 AS w_fts, 1.0::float8 AS w_vec, 0.5::float8 AS w_graph,
               0.5::float8 AS gamma, 100 AS vector_k, 20 AS auto_seed_n),
  fts AS (SELECT * FROM search_fts_candidates(p_principal, p_query)),
  -- #358: scope-bounded ANN. Passing p_context_id/p_scope_ids routes the vector arm through the
  -- scoped exact branch so a context's chunks can't be starved by the global top-k.
  vec AS (SELECT * FROM search_vector_candidates(
            p_principal, p_emb, (SELECT vector_k FROM k), p_context_id, p_scope_ids)),
  blend0 AS (
    SELECT COALESCE(f.resource_id, v.resource_id) AS id,
           (SELECT w_fts FROM k) * COALESCE(f.fts_norm, 0)
         + (SELECT w_vec FROM k) * COALESCE(v.vec_norm, 0) AS s0
      FROM fts f FULL OUTER JOIN vec v ON f.resource_id = v.resource_id
  ),
  seeds AS (
    SELECT unnest(COALESCE(p_seed_ids, ARRAY[]::uuid[])) AS id
    UNION
    -- Auto-seed union — the blend's top-N. Suppressed under --seed-only WHEN explicit seeds exist,
    -- so the caller's seeds alone define the neighborhood. seed_only with no explicit seeds keeps
    -- the auto-seeds (falling back to default behavior rather than returning nothing).
    SELECT id FROM (SELECT id, s0 FROM blend0 ORDER BY s0 DESC LIMIT (SELECT auto_seed_n FROM k)) t
     WHERE NOT (COALESCE(p_seed_only, false) AND COALESCE(array_length(p_seed_ids, 1), 0) > 0)
  ),
  graph AS (
    SELECT * FROM search_graph_expand(
      p_principal,
      CASE WHEN p_graph_expand THEN ARRAY(SELECT id FROM seeds) ELSE ARRAY[]::uuid[] END,
      p_depth, p_edge_types, (SELECT gamma FROM k))
  ),
  -- Explicit seeds stay CANDIDATES even though hop-0 no longer self-scores (issue #357): a resource
  -- you searched *from* must still surface under its own context/scope. It earns graph_score 0 (no
  -- +0.5 self-bonus) and ranks on its own FTS/vector signal — de-bias without vanishing. Visibility-
  -- gated (mirrors search_graph_expand's `visible` seed filter) so a seed the principal cannot see
  -- never leaks into the candidate set. Gated on p_graph_expand: seeds anchor graph expansion.
  seed_cand AS (
    SELECT s.id
      FROM unnest(COALESCE(p_seed_ids, ARRAY[]::uuid[])) AS s(id)
      JOIN resources_visible_to(p_principal) v ON v.resource_id = s.id
     WHERE p_graph_expand
  ),
  cand AS (
    SELECT id FROM blend0
    UNION SELECT resource_id FROM graph
    UNION SELECT id FROM seed_cand
  ),
  corpus AS (   -- context/doc_type/scope candidate-corpus filter
    SELECT c.id FROM cand c
     WHERE (p_context_id IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_homes h
              WHERE h.resource_id = c.id AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
       AND (p_scope_ids IS NULL OR c.id = ANY(p_scope_ids))
       AND (p_doc_type IS NULL OR EXISTS (
             SELECT 1 FROM kb_properties p
              WHERE p.owner_table = 'kb_resources' AND p.owner_id = c.id
                AND p.property_key = 'doc_type' AND NOT p.is_folded
                AND p.property_value #>> '{}' = p_doc_type))
  ),
  scored AS (
    SELECT co.id,
           COALESCE(f.fts_norm, 0)::real    AS fts_score,
           COALESCE(v.vec_norm, 0)::real    AS vector_score,
           COALESCE(g.graph_score, 0)::real AS graph_score,
           ((SELECT w_fts FROM k)   * COALESCE(f.fts_norm, 0)
          + (SELECT w_vec FROM k)   * COALESCE(v.vec_norm, 0)
          + (SELECT w_graph FROM k) * COALESCE(g.graph_score, 0))::real AS combined_score
      FROM corpus co
      LEFT JOIN fts f   ON f.resource_id = co.id
      LEFT JOIN vec v   ON v.resource_id = co.id
      LEFT JOIN graph g ON g.resource_id = co.id
  )
  SELECT id, fts_score, vector_score, graph_score, combined_score
    FROM scored
   ORDER BY combined_score DESC, id
   LIMIT p_limit OFFSET p_offset;
$$;
