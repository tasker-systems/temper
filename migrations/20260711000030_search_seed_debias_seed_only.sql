-- Issue #357: auto-seed top-20 union gives a flat +0.5 rich-get-richer graph bonus.
--
-- Two composed behaviors entrenched incumbents and made explicit `--seed` nearly useless:
--   1. Every graph seed self-scored 1.0 at hop 0, so with w_graph·γ⁰ = 0.5 each of the
--      auto-seeded top-20 blend incumbents pocketed a flat +0.5 on combined_score.
--   2. Seeds were the caller's explicit ids UNIONed with the blend's top-20, so an explicit
--      seed's neighborhood competed against 20 self-boosted incumbents it could not outrank.
--
-- Two composable fixes:
--   • Option 1 (de-bias): `search_graph_expand` no longer emits the hop-0 self-score — only
--     genuine ≥1-hop proximity to a seed contributes. Seeds get no bonus for being seeds.
--     Same signature, so CREATE OR REPLACE with no DROP (additive, deploy-skew-safe).
--   • Option 2 (--seed-only): `unified_search` gains `p_seed_only boolean DEFAULT false`. When
--     true AND explicit seeds are present, the auto-seed union is suppressed so the caller's
--     seeds alone define the graph neighborhood. seed_only with no explicit seeds falls back to
--     normal behavior (never returns an empty result). The DEFAULT keeps the old 12-arg call
--     resolvable during the migrate-ahead-of-deploy window: an old binary's positional 12-arg
--     call binds through the default, so DROP+CREATE here does not break skew.

-- ── Option 1: exclude the hop-0 self-score from the graph contribution ───────────────────────────
CREATE OR REPLACE FUNCTION search_graph_expand(
  p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision)
RETURNS TABLE (resource_id uuid, graph_score real)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE visible AS (
    SELECT rv.resource_id AS id FROM resources_visible_to(p_principal) rv
  ),
  adj AS (   -- undirected adjacency over visible, unfolded, kb_resources edges (optional kind filter)
    SELECT e.source_id AS a, e.target_id AS b, e.weight
      FROM kb_edges e
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND (p_edge_types IS NULL OR array_length(p_edge_types, 1) IS NULL
            OR e.edge_kind::text = ANY(p_edge_types))
       AND e.source_id IN (SELECT id FROM visible)
       AND e.target_id IN (SELECT id FROM visible)
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path
      FROM unnest(p_seed_ids) AS s(id)
     WHERE s.id IN (SELECT id FROM visible)
    UNION ALL
    SELECT nb.node, w.score * p_gamma * nb.weight, w.hop + 1, w.path || nb.node
      FROM walk w
      JOIN LATERAL (
        SELECT adj.b AS node, adj.weight FROM adj WHERE adj.a = w.node
        UNION ALL
        SELECT adj.a AS node, adj.weight FROM adj WHERE adj.b = w.node
      ) nb ON true
     WHERE w.hop < p_depth
       AND NOT nb.node = ANY(w.path)
  )
  -- hop > 0: a seed no longer scores itself 1.0; only genuine ≥1-hop neighborhood proximity is
  -- returned (a seed reached back via a ≥1-hop cycle keeps that path's score — real proximity).
  SELECT node, MAX(score)::real
    FROM walk
   WHERE hop > 0
   GROUP BY node;
$$;

-- ── Option 2: `p_seed_only` suppresses the auto-seed union when explicit seeds are given ──────────
-- DROP+CREATE (arity change) rather than CREATE OR REPLACE; the new param's DEFAULT keeps the prior
-- 12-arg signature callable during deploy skew (see header). Body is the 20260629000004 variant with
-- only the `seeds` CTE changed.
DROP FUNCTION IF EXISTS unified_search(uuid, text, vector, uuid[], int, text[], uuid, text, boolean, int, int, uuid[]);

CREATE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int, p_scope_ids uuid[], p_seed_only boolean DEFAULT false)
RETURNS TABLE (resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 1.0::float8 AS w_fts, 1.0::float8 AS w_vec, 0.5::float8 AS w_graph,
               0.5::float8 AS gamma, 100 AS vector_k, 20 AS auto_seed_n),
  fts AS (SELECT * FROM search_fts_candidates(p_principal, p_query)),
  vec AS (SELECT * FROM search_vector_candidates(p_principal, p_emb, (SELECT vector_k FROM k))),
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
  cand AS (SELECT id FROM blend0 UNION SELECT resource_id FROM graph),
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
