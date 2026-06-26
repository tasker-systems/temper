-- Search Beat 2 — Surface A: blend FTS + vector + graph on /api/search.
-- Four additive SQL functions composed by unified_search. Builds on Beat 1's stored tsvector
-- (20260626000001). Additive-only-on-main: new functions, no schema change.

-- ── Lexical candidates: Beat 1's GIN-indexed stored tsvector, normalized to [0,1) ──────────────
-- ts_rank(..., 32) applies the rank/(rank+1) normalization flag — a FIXED, batch-independent
-- transform, so a doc's score does not depend on what else matched (stable across queries/corpus).
CREATE FUNCTION search_fts_candidates(p_principal uuid, p_query text)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
  SELECT r.id,
         (ts_rank(si.search_vector, plainto_tsquery('english', p_query), 32))::real
    FROM kb_resource_search_index si
    JOIN kb_resources r                       ON r.id = si.resource_id
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = r.id
   WHERE p_query IS NOT NULL AND p_query <> ''
     AND r.is_active
     AND si.search_vector @@ plainto_tsquery('english', p_query);
$$;

-- ── Semantic candidates: HNSW over-fetch-then-filter. The inner `ann` CTE carries ONLY the
-- index's own predicate (is_current) + ORDER BY <=> LIMIT, so idx_kb_chunks_embedding engages.
-- Visibility/active filtering happens AFTER (applying it inside the ANN would force a seq-scan and
-- defeat the index — the exact bug in the legacy GROUP BY/MIN-over-a-join shape). Over-fetch (p_k»limit)
-- absorbs the post-ANN attrition. Best chunk per resource decides rank; vec_norm = 1 - dist/2 ∈ [0,1].
CREATE FUNCTION search_vector_candidates(p_principal uuid, p_emb vector, p_k int)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE sql STABLE AS $$
  WITH ann AS (
    SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
      FROM kb_chunks c
     WHERE p_emb IS NOT NULL AND c.is_current
     ORDER BY c.embedding <=> p_emb
     LIMIT p_k
  )
  SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
    FROM ann a
    JOIN kb_resources r                       ON r.id = a.resource_id AND r.is_active
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = a.resource_id
   GROUP BY a.resource_id;
$$;

-- ── Structural candidates: scoped, weighted, bidirectional multi-hop expansion from seeds.
-- Mirrors graph_traverse's `visible` CTE scoping (canonical_functions.sql:1308) but is purpose-built:
-- BIDIRECTIONAL (follow an edge from either endpoint), WEIGHTED (γ^hop · Π edge_weight), SCORED with
-- MAX-over-paths (hub-robust: best path wins), and edge_kind-filtered. Surface A scope: kb_resources
-- endpoints only, NOT is_folded, every endpoint joined through resources_visible_to. Seeds = hop 0,
-- score 1.0. A path array gives the cycle guard (and bounds termination alongside p_depth).
CREATE FUNCTION search_graph_expand(
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
  SELECT node, MAX(score)::real
    FROM walk
   GROUP BY node;
$$;
