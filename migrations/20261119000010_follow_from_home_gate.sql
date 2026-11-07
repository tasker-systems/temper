-- The follow-from walk admits an edge only as edges_visible_to defines it (PW-6).
--
-- WHAT WAS WRONG. `__temper_ungated_follow_from`'s `adj` (latest body 20260828000050) admitted any
-- live edge whose two endpoints were in the caller's visible set. It never asked whether the edge's
-- HOME anchor was readable. So an edge homed in a context or cogmap the caller cannot read, between
-- two resources the caller can, was walked, returned in `via`, and moved `search_graph_expand`'s
-- ranking. `edges_visible_to` is the one definition: live, home readable, both endpoints readable.
--
-- THE FIX, ADDITIVELY (no DROP, no new arity of an existing name):
--   1. `__temper_ungated_follow_from_homed` is the 20260828000050 body VERBATIM plus an 11th
--      parameter `p_anchor_reader` and one join: `adj` joins `edges_visible_to(p_anchor_reader)`,
--      hoisted once into a MATERIALIZED `ve`. The walk now lives here, and only here.
--   2. The 10-arity `__temper_ungated_follow_from` becomes a wrapper passing a NULL reader. It has
--      no principal to check a home against, so it admits no edge: fail-closed, never wider. The
--      8- and 9-arity delegators reach it unchanged.
--   3. The 10-arity `query_follow_from` (the gated wrapper every gated arity and
--      `search_graph_expand` reach) passes its own `p_principal` as the reader.
--   4. The query compiler emits `__temper_ungated_follow_from_homed` with its `$1` principal
--      (query_plan.rs, same change).
--   5. `graph_traverse` (no Rust caller) gets the same gate in place: both arms join
--      `edges_visible_to(p_profile)`. Same signature, same columns.
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
  ve AS MATERIALIZED (
    SELECT ev.edge_id FROM edges_visible_to(p_anchor_reader) ev WHERE p_anchor_reader IS NOT NULL
  ),
  adj AS (
    SELECT e.id AS edge_id, e.source_id, e.target_id, e.weight
      FROM kb_edges e
      JOIN ve ON ve.edge_id = e.id
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
$c$The follow-from walk (the ONLY copy of its recursion; 20260828000050's body plus the PW-6 home
gate of 20261119000010). Ungated in the compiler's sense: the visible-resource set is handed IN
as p_visible_ids, built once by the caller. The edge gate is decided here against p_anchor_reader:
`adj` admits an edge only if it is in edges_visible_to(p_anchor_reader) -- live, home anchor
readable, both endpoints readable -- AND both endpoints are in p_visible_ids (and in p_bound_ids
when given). A NULL p_anchor_reader admits no edge. Callers: query_follow_from (10-arity) and the
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
once and passes it down, with p_principal as the anchor reader, so every walked edge is one
edges_visible_to admits (PW-6, 20261119000010). Carries p_offset so a direct caller can page a
walk. No defaults, for the reason recorded on the core.$c$;

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
       AND NOT e.is_folded AND w.depth < p_depth
       AND e.target_id IN (SELECT id FROM visible)
  )
  SELECT w.target_id, w.source_id, w.target_id, w.edge_kind, w.polarity, w.label, w.depth FROM walk w;
$$;

COMMENT ON FUNCTION graph_traverse(uuid, uuid[], int) IS
$c$The directed resource walk (20260624000002), retired from the product: no Rust caller since
graph_service moved to graph_induced_edges. Kept and gated rather than dropped: 20261119000010 makes
both arms join edges_visible_to(p_profile), so it admits an edge only if it is live, its home is
readable and both endpoints are readable (spec P12). Otherwise unchanged.$c$;

SELECT declare_migration(
    20261119000010,
    'additive',
    'PW-6: the follow-from walk admitted any live edge whose two endpoints were visible, without '
    'checking that the edge''s home anchor was readable, so an edge homed in an unreadable context '
    'or cogmap was walked and moved search_graph_expand ranking. Adds '
    '__temper_ungated_follow_from_homed (the 20260828000050 body verbatim, plus p_anchor_reader '
    'and a join on edges_visible_to). The 10-arity __temper_ungated_follow_from becomes a wrapper '
    'passing a NULL reader, which admits no edge (fail-closed); the 10-arity query_follow_from '
    'passes its principal; the compiler emits the new body. NO DROP and no new arity of an existing '
    'name. graph_traverse has no Rust caller and is fixed in place the same way rather than left '
    'open, so the edge-conjunction harness can classify it as delegating. Skew: an older binary '
    'still emits the 10-arity core, whose follow stages then return no rows until the new binary '
    'is live. That is narrower, never wider. Plan: temper-artifacts '
    'plans/2026-11-06-public-within.md Phase 0.1.'
);
