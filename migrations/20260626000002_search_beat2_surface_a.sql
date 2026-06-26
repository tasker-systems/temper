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
