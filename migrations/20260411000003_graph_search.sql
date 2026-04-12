-- =============================================================================
-- R7 Phase 3: Combined Vector + Graph Search
-- =============================================================================
-- Composes unified_search() with graph_traverse() to produce graph-enhanced
-- search results. Auto-seeds graph expansion from vector/FTS hits.

CREATE FUNCTION graph_search(
    p_profile_id      UUID,
    p_query           TEXT DEFAULT '',
    p_embedding       vector(768) DEFAULT NULL,
    p_search_config   VARCHAR DEFAULT 'english',
    p_context_name    VARCHAR DEFAULT NULL,
    p_doc_type        VARCHAR DEFAULT NULL,
    p_fts_weight      FLOAT DEFAULT 0.5,
    p_vec_weight      FLOAT DEFAULT 0.5,
    p_seed_ids        UUID[] DEFAULT '{}',
    p_edge_types      TEXT[] DEFAULT '{}',
    p_graph_depth     INT DEFAULT 2,
    p_graph_weight    FLOAT DEFAULT 0.3,
    p_limit           INT DEFAULT 10,
    p_offset          INT DEFAULT 0
) RETURNS TABLE (
    resource_id    UUID,
    title          TEXT,
    slug           VARCHAR(256),
    kb_uri         TEXT,
    origin_uri     TEXT,
    context        VARCHAR(128),
    doc_type       VARCHAR(64),
    fts_score      REAL,
    vector_score   REAL,
    combined_score REAL,
    origin         VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    -- Stage 1: Run unified_search to get FTS + vector results
    base_results AS (
        SELECT us.resource_id, us.title, us.slug, us.kb_uri, us.origin_uri,
               us.context, us.doc_type, us.fts_score, us.vector_score,
               us.combined_score, us.origin
          FROM unified_search(
            p_profile_id, p_query, p_embedding, p_search_config,
            p_context_name, p_doc_type, p_fts_weight, p_vec_weight,
            p_limit, p_offset
          ) us
    ),

    -- Stage 2: Collect seeds = base result IDs ∪ explicit seed_ids
    all_seeds AS (
        SELECT resource_id FROM base_results
        UNION
        SELECT unnest(p_seed_ids)
    ),

    -- Stage 3: Graph expand from seeds
    graph_hits AS (
        SELECT gt.resource_id, gt.depth, gt.path_weight
          FROM graph_traverse(
            p_profile_id,
            ARRAY(SELECT resource_id FROM all_seeds),
            p_graph_depth,
            p_edge_types
          ) gt
         WHERE gt.depth > 0
    ),

    -- Stage 4: Best graph proximity score per resource
    graph_scores AS (
        SELECT resource_id,
               MAX(path_weight / (depth + 1)::FLOAT)::REAL AS graph_proximity
          FROM graph_hits
         GROUP BY resource_id
    ),

    -- Stage 5: Combine base results with graph-expanded resources
    combined AS (
        SELECT
            COALESCE(br.resource_id, gs.resource_id) AS resource_id,
            COALESCE(br.fts_score, 0.0::REAL) AS fts_score,
            COALESCE(br.vector_score, 0.0::REAL) AS vector_score,
            COALESCE(gs.graph_proximity, 0.0::REAL) AS graph_score,
            ((1.0 - p_graph_weight) * GREATEST(COALESCE(br.fts_score, 0.0), COALESCE(br.vector_score, 0.0))
             + p_graph_weight * COALESCE(gs.graph_proximity, 0.0))::REAL AS combined_score,
            CASE
                WHEN br.resource_id IS NOT NULL AND gs.resource_id IS NOT NULL THEN 'both'
                WHEN br.resource_id IS NOT NULL THEN COALESCE(br.origin, 'fts')
                ELSE 'graph'
            END AS origin
        FROM base_results br
        FULL OUTER JOIN graph_scores gs ON gs.resource_id = br.resource_id
    )

    SELECT
        c.resource_id,
        r.title,
        r.slug,
        kb_resource_uri(r.id) AS kb_uri,
        r.origin_uri,
        ctx.name AS context,
        dt.name AS doc_type,
        c.fts_score,
        c.vector_score,
        c.combined_score,
        c.origin::VARCHAR(16)
    FROM combined c
    JOIN kb_resources r ON r.id = c.resource_id
    LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
    JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
    ORDER BY c.combined_score DESC
    LIMIT p_limit
    OFFSET p_offset
$$;
