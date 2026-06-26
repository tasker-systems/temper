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
