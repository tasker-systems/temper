-- Migration: replace graph_subgraph_nodes to accept p_context_id (uuid) instead of p_context_name
-- (varchar). The old name-based variant resolved the context via a WHERE name = ... CTE, which is
-- ambiguous (two contexts under different owners can share a name). Callers now resolve the context
-- ref server-side (parse_context_ref → resolve_context_ref → ContextId) and pass the resolved UUID.
--
-- The old overload (uuid, varchar, text[], int) is dropped first because CREATE OR REPLACE FUNCTION
-- with different parameter types creates a new overload rather than replacing the original.

DROP FUNCTION IF EXISTS graph_subgraph_nodes(uuid, varchar, text[], int);

CREATE OR REPLACE FUNCTION graph_subgraph_nodes(
  p_profile uuid, p_context_id uuid, p_aggregator_types text[], p_depth int)
RETURNS TABLE (resource_id uuid, slug varchar, title text, doc_type varchar,
               edge_count int, session_count int, first_chunk text, stage_raw text)
LANGUAGE sql STABLE AS $$
  WITH doc AS (  -- doc_type property per resource
    SELECT p.owner_id AS rid, p.property_value #>> '{}' AS dt
      FROM kb_properties p
     WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded),
  seeds AS (
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resource_homes h ON h.resource_id=r.id
                               AND h.anchor_table='kb_contexts'
                               AND h.anchor_id = p_context_id
      JOIN doc ON doc.rid = r.id
     WHERE r.is_active AND doc.dt = ANY(p_aggregator_types)),
  walked AS (
    SELECT DISTINCT t.resource_id AS id
      FROM graph_traverse(p_profile, ARRAY(SELECT id FROM seeds), p_depth) t
    UNION SELECT id FROM seeds),
  nodes AS (
    SELECT r.id, doc.dt AS doc_type, r.title FROM kb_resources r
      JOIN walked w ON w.id=r.id JOIN doc ON doc.rid=r.id
     WHERE r.is_active AND doc.dt <> 'session')  -- sessions are not nodes
  SELECT
    n.id,
    -- slug retired in substrate (§7-dissolved); derive from title to match Rust text::slugify:
    -- lowercase, non-alphanumeric runs → single dash, trim leading/trailing dashes. Presentational.
    lower(regexp_replace(regexp_replace(n.title, '[^a-zA-Z0-9]+', '-', 'g'), '(^-+|-+$)', '', 'g'))::varchar AS slug,
    n.title,
    n.doc_type::varchar,
    (SELECT count(*)::int FROM kb_edges e
       WHERE NOT e.is_folded AND e.source_table='kb_resources' AND e.target_table='kb_resources'
         AND (e.source_id=n.id OR e.target_id=n.id)) AS edge_count,
    -- session adjacency: 0 until re-modelled (see original function comment).
    0::int AS session_count,
    (SELECT cc.content FROM kb_chunks ch
       JOIN kb_content_blocks b ON b.id=ch.block_id
       JOIN kb_chunk_content cc ON cc.chunk_id=ch.id
      WHERE ch.resource_id=n.id AND ch.is_current AND NOT b.is_folded
      ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
    (SELECT sp.property_value #>> '{}' FROM kb_properties sp
      WHERE sp.owner_table='kb_resources' AND sp.owner_id=n.id
        AND sp.property_key='temper-stage' AND NOT sp.is_folded LIMIT 1) AS stage_raw
  FROM nodes n;
$$;
