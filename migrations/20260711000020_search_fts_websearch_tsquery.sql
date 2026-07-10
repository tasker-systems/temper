-- Issue #356: FTS uses `plainto_tsquery` — exact phrases are unexpressible.
--
-- `plainto_tsquery` ANDs the query terms with no adjacency, so a distinctive multi-word
-- phrase (each term frequent corpus-wide, the phrase rare) loses all its phrase signal and
-- there is no syntax an agent can reach for to force phrase/proximity matching. Swap it for
-- `websearch_to_tsquery`, which gives `"quoted phrases"` (adjacency via `<->`), `OR`, and
-- `-negation` while parsing plain unquoted input IDENTICALLY to `plainto_tsquery` — so
-- unquoted queries are fully backward-compatible (the substrate parity test proves this).
--
-- Same signature/return as the shipped function (migration 20260626000002), so CREATE OR
-- REPLACE with NO DROP — additive-only-on-main and safe across the migrate-ahead-of-deploy
-- window (a DROP would break callers mid-window). `ts_rank(..., 32)` normalization is
-- unchanged; cover-density (`ts_rank_cd`) is a deliberate follow-up, out of scope here.
--
-- The peer FTS-only read path (`readback::fts_search`, backing `search_resources`) gains the
-- same swap in Rust in the same change so phrase syntax works on BOTH surfaces.

CREATE OR REPLACE FUNCTION search_fts_candidates(p_principal uuid, p_query text)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
  SELECT r.id,
         (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 32))::real
    FROM kb_resource_search_index si
    JOIN kb_resources r                       ON r.id = si.resource_id
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = r.id
   WHERE p_query IS NOT NULL AND p_query <> ''
     AND r.is_active
     AND si.search_vector @@ websearch_to_tsquery('english', p_query);
$$;
