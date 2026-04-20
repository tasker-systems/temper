-- =============================================================================
-- C3: Performance indexes for graph + edge hot paths
-- =============================================================================
-- Audit: docs/code-reviews/2026-04-20-graph-performance-audit.md §Indexes.
--
-- Each index targets a specific query pattern that currently either re-scans
-- a broader index + filters in Postgres, or requires a sort it could skip.

-- Aggregator seed selection in graph_subgraph_nodes() filters on
-- (kb_context_id, kb_doc_type_id, is_active). The existing
-- idx_kb_resources_context is (kb_context_id) only, so the planner has to
-- re-check doc_type_id and is_active in a heap filter. Partial index on
-- active rows since the query always gates on is_active = true.
CREATE INDEX IF NOT EXISTS idx_kb_resources_ctx_doctype_active
    ON kb_resources (kb_context_id, kb_doc_type_id)
    WHERE is_active = true;

-- reconcile_edges (crates/temper-api/src/services/edge_service.rs) filters
-- frontmatter-provenance edges on every UPDATE. Expression index on the
-- JSONB lookup avoids scanning every outgoing/incoming edge for this source.
CREATE INDEX IF NOT EXISTS idx_kb_resource_edges_provenance_src
    ON kb_resource_edges (source_resource_id)
    WHERE (metadata->>'provenance') = 'frontmatter';

-- Deferred-edge resolution currently matches target_ref exactly. Once
-- deferred resolution batches across contexts (future work — follow-on to
-- the edge_service batching in §F2), (target_context_id, target_ref) becomes
-- the join key. Tiny write cost, protects us when that lands.
CREATE INDEX IF NOT EXISTS idx_deferred_edges_ctx_ref
    ON kb_deferred_edges (target_context_id, target_ref);

-- The first_chunks CTE in graph_subgraph_nodes() does:
--   DISTINCT ON (resource_id) … ORDER BY resource_id, chunk_index
-- which needs (resource_id, chunk_index) in order. idx_chunks_resource is
-- (resource_id) only, forcing an in-memory sort per resource. Partial on
-- is_current so it stays small (one version of each chunk).
CREATE INDEX IF NOT EXISTS idx_kb_chunks_resource_index
    ON kb_chunks (resource_id, chunk_index)
    WHERE is_current = true;
