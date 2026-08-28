-- The follow-from walk gains the internal depth clamp every sibling walk already carries.
--
-- WHAT CHANGES. One predicate inside the recursive `walk` CTE: `w.hop < p_depth` becomes
-- `w.hop < p_depth AND w.hop < 3`. Nothing else in the body moves, and the signature is unchanged.
--
-- WHY THE PREDICATE IS TWO CONJUNCTS AND NOT THE SIBLINGS' `LEAST`. `LEAST` ignores NULL, so
-- `LEAST(p_depth, 3)` reads a NULL depth as 3 and would turn "walk nothing" into "walk the full
-- neighbourhood" -- the one direction a bounding change must never move. Found in review, after the
-- first draft of this file shipped a comment asserting the opposite. See the predicate itself.
--
-- WHY 3, AND WHY IT CHANGES NO ANSWER TODAY. Every walk in this schema clamps p_depth internally
-- rather than trusting its caller: the resource- and context-grain walks at 3
-- (`20260709000010:56`, `20260708000002:36`, `20260821000010:134`) and the cogmap/atlas traversals
-- at 10 (`20260706120200:40`, `20260709000001:38`, `20260704000009:40`). This walk is the
-- resource-grain one, so it takes 3. Its only live depth is the compiler's fixed `WALK_DEPTH = 2`
-- (`query_plan.rs:140`), which is below the clamp — so no answer this schema can produce today is
-- different afterwards.
--
-- WHY IT IS WORTH ADDING ANYWAY, STATED PRECISELY. This is DEFENCE IN DEPTH and not the closing of
-- a reachable hole, and the difference matters enough to write down. `/api/query` fixes the depth
-- at a compile-time constant, so no caller of that door chooses it. What the walk has instead is a
-- delegation chain — `search_graph_expand` -> `query_follow_from` -> this body — whose outermost
-- arm passes a caller's depth straight through, and there is no `SECURITY DEFINER` anywhere to make
-- a wrapper the only reachable entry. So the property "a walk's depth is bounded by the walk" is
-- true here by the compiler's choice rather than by the walk, and it is the only walk in the schema
-- of which that is so. Clamping is what makes the invariant belong to the function.
--
-- Additive: one CREATE OR REPLACE at the unchanged 10-arity signature, no DROP. The 8- and 9-arity
-- delegators reach this body and need no edit, which is the ONE BODY PER ARM property
-- `20260817000020` established holding.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

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
  adj AS (
    SELECT e.id AS edge_id, e.source_id, e.target_id, e.weight
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

SELECT declare_migration(
    20260828000040,
    'additive',
    'Clamps the follow-from walk''s recursion depth inside the walk: `w.hop < p_depth` becomes `w.hop < p_depth AND w.hop < 3` in the 10-arity __temper_ungated_follow_from, which since 20260817000020 holds the only copy of the body (8 and 9 delegate to it and are untouched). TWO CONJUNCTS AND NOT THE SIBLINGS'' `LEAST(p_depth, 3)`, because LEAST IGNORES NULL: LEAST(NULL::int, 3) is 3, so that spelling would read a NULL depth as 3 and turn "walk nothing" into "walk the full three-hop neighbourhood" -- measured at 0 rows vs 3 rows on a 6-node chain, and witnessed by the_walk_clamps_its_own_depth_and_still_honours_a_smaller_request. The ceiling is the siblings''; only the spelling differs, and it differs because each sibling was born with LEAST and so never flipped a NULL, where this one would have. 3 matches every resource- and context-grain sibling — 20260709000010:56, 20260708000002:36, 20260821000010:134 — against 10 for the cogmap/atlas traversals. NO ANSWER CHANGES TODAY: the only live depth is the compiler''s fixed WALK_DEPTH = 2 (query_plan.rs:140), which is under the clamp, and search_graph_expand''s only Rust callers are temper-substrate''s own tests. This is DEFENCE IN DEPTH rather than the closing of a reachable hole, and is stated that way so a later reader does not infer an exposure that was never measured: what it buys is that the bound belongs to the walk instead of to one caller''s constant, in the one walk in this schema where it did not — search_graph_expand passes a caller''s depth straight through and there is no SECURITY DEFINER anywhere to make a wrapper the only reachable entry. Additive: one CREATE OR REPLACE at the unchanged signature, no DROP. Draft: triage-inbound/upstream-issues/30-query-engine-resource-bounds.md item 8; task 01a035f1-0614-7483-9043-6d96aa181158.'
);
