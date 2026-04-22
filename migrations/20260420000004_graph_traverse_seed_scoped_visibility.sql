-- =============================================================================
-- C5: graph_traverse — scope base-step visibility check to seeds
-- =============================================================================
-- Audit: docs/code-reviews/2026-04-20-graph-performance-audit.md §F4
--
-- The previous `visible` CTE materialized the full resources_visible_to set
-- before the recursion began. In a vault with 10k resources that's 10k rows
-- persisted for what might resolve to a 50-row traversal. `resources_visible_to`
-- already accepts a resource_ids[] filter; passing p_seed_ids at the base
-- step pre-filters that CTE to just the seeds.
--
-- The recursive step still uses the unrestricted `'{}'` form because the
-- discovered targets aren't known ahead of time.
--
-- Semantics are identical — the seed filter is only tightened, and the
-- original base-step `WHERE v.resource_id = ANY(p_seed_ids)` is preserved
-- so nothing invisible slips through.
--
-- CREATE OR REPLACE because the function signature is unchanged.

CREATE OR REPLACE FUNCTION graph_traverse(
    p_profile_id  UUID,
    p_seed_ids    UUID[],
    p_max_depth   INT DEFAULT 3,
    p_edge_types  TEXT[] DEFAULT '{}'
) RETURNS TABLE (
    resource_id       UUID,
    depth             INT,
    path              UUID[],
    edge_type         edge_type,
    from_resource_id  UUID,
    path_weight       FLOAT
)
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE
      -- Base step: only the seeds need visibility resolution. Pass
      -- p_seed_ids so resources_visible_to short-circuits to the relevant
      -- rows instead of materializing every visible resource.
      seed_visible AS (
        SELECT v.resource_id
          FROM resources_visible_to(p_profile_id, NULL, p_seed_ids) v
         WHERE v.resource_id = ANY(p_seed_ids)
      ),
      -- Recursive step still needs the unrestricted form because the
      -- target set is discovered during traversal.
      visible AS (
        SELECT v.resource_id
          FROM resources_visible_to(p_profile_id, NULL, '{}') v
      ),
      traversal AS (
        -- Base case: seed resources (must be visible)
        SELECT
          sv.resource_id,
          0 AS depth,
          ARRAY[sv.resource_id] AS path,
          NULL::edge_type AS edge_type,
          NULL::UUID AS from_resource_id,
          1.0::FLOAT AS path_weight
        FROM seed_visible sv

        UNION ALL

        -- Recursive case: expand one hop forward
        SELECT
          e.target_resource_id,
          t.depth + 1,
          t.path || e.target_resource_id,
          e.edge_type,
          t.resource_id,
          t.path_weight * e.weight
        FROM traversal t
        JOIN kb_resource_edges e ON e.source_resource_id = t.resource_id
        JOIN visible v ON v.resource_id = e.target_resource_id
        WHERE t.depth < p_max_depth
          AND NOT e.target_resource_id = ANY(t.path)
          AND (p_edge_types = '{}' OR e.edge_type::TEXT = ANY(p_edge_types))
      )
    SELECT DISTINCT ON (t.resource_id)
      t.resource_id,
      t.depth,
      t.path,
      t.edge_type,
      t.from_resource_id,
      t.path_weight
    FROM traversal t
    WHERE t.depth > 0   -- exclude seeds from result set
    ORDER BY t.resource_id, t.depth ASC, t.path_weight DESC
$$;
