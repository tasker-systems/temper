-- The follow-from walk admits an edge only if it is live, its home anchor is readable by the
-- caller, and both endpoints are in the caller's visible set. graph_traverse admits only edges
-- edges_visible_to admits.
--
-- ADDITIVELY (no DROP, no new arity of an existing name):
--   1. `__temper_ungated_follow_from_homed` is the 20260828000050 body plus an 11th parameter,
--      `p_anchor_reader`, and a home gate. The candidate edges (live, both endpoints admitted) are
--      materialized once, and a candidate enters the adjacency only if its home is readable by the
--      reader: a context home must be in contexts_readable_by(p_anchor_reader), materialized once
--      per call and only when some candidate is homed in a context; a cogmap home must pass
--      cogmap_readable_by_profile; any other home admits nothing. Those are exactly
--      anchor_readable_by_profile's arms, evaluated as a set for contexts. The walk lives here, and
--      only here. The endpoints stay gated by the visible set handed in, which is the caller's
--      verdict; the core computes no visible set of its own.
--   2. The 10-arity `__temper_ungated_follow_from` is a wrapper passing a NULL reader. It has no
--      principal to check a home against, so it admits no edge: fail-closed, never wider. The 8-
--      and 9-arity delegators reach it unchanged.
--   3. The 10-arity `query_follow_from` (the gated wrapper every gated arity and
--      `search_graph_expand` reach) passes its own `p_principal` as the reader.
--   4. The query compiler emits `__temper_ungated_follow_from_homed` with its `$1` principal
--      (query_plan.rs, same change).
--   5. `graph_traverse` (no Rust caller) joins `edges_visible_to(p_profile)` in both arms, on
--      20260709000001's body with its depth clamp of 10.
--
-- Cost, measured on 30k resources, 30k edges and 3000 distinct context homes (median ms, one /
-- four follow stages in one statement): this body 23 / 47; no home check 20 / 41; a per-stage
-- edges_visible_to join 42 / 123; one anchor_readable_by_profile call per distinct home 52 / 162.
-- Recorded in internal/decisions/2026-08-26-the-visibility-verdict-is-hoisted-once.md (amendment
-- of 2026-11-07).
--
-- Rationale lives in COMMENT ON FUNCTION, not in the bodies: the guard scans read
-- pg_get_functiondef.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

CREATE FUNCTION __temper_ungated_follow_from_homed(
    p_visible_ids     uuid[],
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb,
    p_offset          int,
    p_anchor_reader   uuid)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE
  predicates AS (
    SELECT * FROM _temper_property_predicates_parse(p_edge_properties)
  ),
  admitted AS (
    SELECT v.id
      FROM unnest(p_visible_ids) AS v(id)
     WHERE p_bound_ids IS NULL
        OR EXISTS (SELECT 1 FROM unnest(p_bound_ids) AS b(id) WHERE b.id = v.id)
  ),
  cand AS MATERIALIZED (
    SELECT e.id AS edge_id, e.source_id, e.target_id, e.weight,
           e.home_anchor_table, e.home_anchor_id
      FROM kb_edges e
      JOIN admitted sa ON sa.id = e.source_id
      JOIN admitted ta ON ta.id = e.target_id
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
            OR e.edge_kind::text = ANY(p_edge_kinds))
       AND (p_labels IS NULL OR array_length(p_labels, 1) IS NULL
            OR e.label = ANY(p_labels))
       AND (p_edge_properties IS NULL OR (
             jsonb_typeof(p_edge_properties) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_edge_properties ep
                   WHERE ep.edge_id = e.id
                     AND ep.property_key = q.property_key
                     AND _temper_property_op_match(q.op, q.vals, q.direction, q.bound, ep.property_value)))))
  ),
  homes AS MATERIALIZED (
    SELECT DISTINCT c.home_anchor_table, c.home_anchor_id FROM cand c
  ),
  readable_contexts AS MATERIALIZED (
    SELECT rc.context_id FROM contexts_readable_by(p_anchor_reader) rc
     WHERE p_anchor_reader IS NOT NULL
       AND EXISTS (SELECT 1 FROM homes h WHERE h.home_anchor_table = 'kb_contexts')
  ),
  readable_homes AS MATERIALIZED (
    SELECT h.home_anchor_table, h.home_anchor_id
      FROM homes h
     WHERE p_anchor_reader IS NOT NULL
       AND ( (h.home_anchor_table = 'kb_contexts'
                AND h.home_anchor_id IN (SELECT context_id FROM readable_contexts))
          OR (h.home_anchor_table = 'kb_cogmaps'
                AND cogmap_readable_by_profile(p_anchor_reader, h.home_anchor_id)) )
  ),
  adj AS (
    SELECT c.edge_id, c.source_id, c.target_id, c.weight
      FROM cand c
      JOIN readable_homes rh
        ON rh.home_anchor_table = c.home_anchor_table
       AND rh.home_anchor_id = c.home_anchor_id
  ),
  -- Undirected adjacency: both orientations once, so `walk` joins plainly and the duplicated LATERAL
  -- disappears. The edge's own source/target are recovered at the end by joining `kb_edges` on the
  -- edge ID carried through the recursion — one column instead of five.
  undirected AS (
    SELECT adj.source_id AS from_node, adj.target_id AS to_node,
           adj.weight, adj.edge_id
      FROM adj
    UNION ALL
    SELECT adj.target_id AS from_node, adj.source_id AS to_node,
           adj.weight, adj.edge_id
      FROM adj
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path,
           s.id AS seed_id,
           NULL::uuid AS e_id
      FROM unnest(p_seed_ids) AS s(id)
     WHERE EXISTS (SELECT 1 FROM admitted a WHERE a.id = s.id)
    UNION ALL
    SELECT u.to_node, w.score * p_gamma * u.weight, w.hop + 1, w.path || u.to_node,
           w.seed_id, u.edge_id
      FROM walk w
      JOIN undirected u ON u.from_node = w.node
     -- The clamp, written as TWO CONJUNCTS rather than as the siblings' `LEAST(p_depth, 3)`, and
     -- the divergence is the whole reason this comment is long.
     --
     -- **`LEAST` IGNORES NULL.** `LEAST(NULL::int, 3)` is `3`, not NULL — so `w.hop < LEAST(p_depth,
     -- 3)` turns a NULL depth from "no recursion at all" (`w.hop < NULL` is NULL, hence false) into
     -- "walk the full three hops". Measured on the live container: a 6-node chain seeded at n0
     -- answers 0 rows at NULL depth through the incumbent and 3 rows through `LEAST`. Empty to
     -- maximal, in a change whose entire purpose is to bound.
     --
     -- Written this way the NULL propagates out of the first conjunct exactly as it did before, so
     -- the only answers that move are the ones above 3 — which is what "defence in depth" has to
     -- mean if it is to be worth applying to a walk nobody can currently reach.
     --
     -- The ceiling is still 3 and still the siblings'; it is the SPELLING that differs, and it
     -- differs because every sibling was BORN with `LEAST` and so never flipped a NULL from 0 to N.
     -- Adopting their spelling here would have imported a widening they do not have. (Their own
     -- NULL behaviour is a separate question about their own callers, and is not touched here.)
     WHERE w.hop < p_depth
       AND w.hop < 3
       AND NOT u.to_node = ANY(w.path)
  ),
  -- The ORDER BY is the total order the OFFSET needs; it predates paging and is untouched. Pages
  -- tile exactly within one snapshot — never across two, since each page is its own statement.
  ranked AS (
    SELECT w.node, MAX(w.score)::real AS graph_score
      FROM walk w
     WHERE w.hop > 0
     GROUP BY w.node
     ORDER BY MAX(w.score) DESC, w.node
     LIMIT p_limit
    OFFSET p_offset
  )
  SELECT r.node, r.graph_score,
         -- Every parent, deduplicated at the EDGE grain: the edge ID is a 1:1 handle for the
         -- (source, target, kind, label, polarity) tuple, so DISTINCT on the jsonb object and
         -- DISTINCT on e_id are the same set. Joining `kb_edges` here is a PK lookup.
         (SELECT jsonb_agg(DISTINCT jsonb_build_object(
                   'seed_id',   w.seed_id,
                   'source_id', e.source_id,
                   'target_id', e.target_id,
                   'edge_kind', e.edge_kind::text,
                   'label',     e.label,
                   'polarity',  e.polarity::text))
            FROM walk w
            JOIN kb_edges e ON e.id = w.e_id
           WHERE w.node = r.node AND w.hop > 0)
    FROM ranked r
   ORDER BY r.graph_score DESC, r.node;
$$;
COMMENT ON FUNCTION __temper_ungated_follow_from_homed(
    uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int, uuid) IS
$c$The follow-from walk (the ONLY copy of its recursion; 20260828000050's body plus a home gate,
20261119000010). Ungated in the compiler's sense: the
visible-resource set is handed IN as p_visible_ids, built once by the caller, and the core computes
no visible set of its own. An edge enters the adjacency only if it is live, both endpoints are in
p_visible_ids (and in p_bound_ids when given), and its home anchor is readable by p_anchor_reader:
a context home must be in contexts_readable_by(p_anchor_reader), computed once per call and only
when a candidate edge is homed in a context; a cogmap home must pass cogmap_readable_by_profile;
any other home admits nothing. This is anchor_readable_by_profile's verdict, evaluated as a set
for contexts. A NULL p_anchor_reader admits no edge. Callers: query_follow_from (10-arity) and the
query compiler's EMIT_FOLLOW_FROM, which passes its $1 principal.$c$;

CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids     uuid[],
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb,
    p_offset          int)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from_homed(
             p_visible_ids, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties, p_offset,
             NULL::uuid) c;
$$;

COMMENT ON FUNCTION __temper_ungated_follow_from(
    uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int) IS
$c$Retired entry, kept so nothing is dropped (20261119000010). Delegates to
__temper_ungated_follow_from_homed with a NULL anchor reader: with no principal there is no home
to check an edge against, so it admits NO edge and walks nowhere. Fail-closed, never wider. The 8-
and 9-arity delegators reach this one. New callers use __temper_ungated_follow_from_homed.$c$;

CREATE OR REPLACE FUNCTION query_follow_from(
    p_principal       uuid,
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb,
    p_offset          int)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from_homed(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties, p_offset,
             p_principal) c;
$$;

COMMENT ON FUNCTION query_follow_from(
    uuid, uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int) IS
$c$Gated wrapper over __temper_ungated_follow_from_homed: computes resources_visible_to(p_principal)
once and passes it down, with p_principal as the anchor reader, so every walked edge is live, has a
readable home and two readable endpoints (20261119000010). Carries p_offset so a direct caller can
page a walk. No defaults, for the reason recorded on the core.$c$;

CREATE OR REPLACE FUNCTION graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth int)
RETURNS TABLE (resource_id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, depth int)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE visible AS (SELECT rv.resource_id AS id FROM resources_visible_to(p_profile) rv),
  ve AS MATERIALIZED (SELECT ev.edge_id FROM edges_visible_to(p_profile) ev),
  walk AS (
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, 1 AS depth
      FROM kb_edges e
      JOIN ve ON ve.edge_id = e.id
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND e.source_id = ANY(p_seed_ids) AND NOT e.is_folded
       AND e.source_id IN (SELECT id FROM visible) AND e.target_id IN (SELECT id FROM visible)
    UNION
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, w.depth+1
      FROM kb_edges e JOIN walk w ON e.source_id = w.target_id
      JOIN ve ON ve.edge_id = e.id
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND NOT e.is_folded AND w.depth < LEAST(p_depth, 10)
       AND e.target_id IN (SELECT id FROM visible)
  )
  SELECT w.target_id, w.source_id, w.target_id, w.edge_kind, w.polarity, w.label, w.depth FROM walk w;
$$;

COMMENT ON FUNCTION graph_traverse(uuid, uuid[], int) IS
$c$The directed resource walk, retired from the product: no Rust caller since graph_service moved
to graph_induced_edges. Kept and gated rather than dropped. 20261119000010 has both arms join
edges_visible_to(p_profile), so it admits an edge only if it is live, its home is readable and
both endpoints are readable, and keeps 20260709000001's depth clamp (10).$c$;

SELECT declare_migration(
    20261119000010,
    'additive',
    'The follow-from walk admits an edge only if it is live, both endpoints are in the visible set '
    'handed in, and its home anchor is readable by the caller. Adds __temper_ungated_follow_from_homed '
    '(the 20260828000050 body plus p_anchor_reader; context homes checked against '
    'contexts_readable_by once per call, cogmap homes with cogmap_readable_by_profile). The 10-arity '
    '__temper_ungated_follow_from becomes a wrapper passing a NULL reader, which admits no edge; the '
    '10-arity query_follow_from passes its principal; the compiler emits the new body. graph_traverse '
    'admits only edges edges_visible_to admits, depth clamped at 10. No DROP and no new arity of an '
    'existing name. Skew, in DEPLOYING.md terms: this narrows, never widens. An older binary still '
    'emits the 10-arity core, so its follow stages return no rows until the new binary is live.'
);
