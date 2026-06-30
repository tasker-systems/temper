-- Beat 3 / Surface B: generalized scope-id corpus filter (spec §6).
-- Additive to the dormant p_context_id filter: restrict the corpus to an explicit id set.
-- When p_scope_ids IS NULL the function behaves identically to the 11-arg Beat-2 variant.
DROP FUNCTION IF EXISTS unified_search(uuid, text, vector, uuid[], int, text[], uuid, text, boolean, int, int);

CREATE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int, p_scope_ids uuid[])
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
    SELECT id FROM (SELECT id, s0 FROM blend0 ORDER BY s0 DESC LIMIT (SELECT auto_seed_n FROM k)) t
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
