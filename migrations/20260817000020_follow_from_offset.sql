-- `follow-from` gains an OFFSET: the walk can be PAGED past its published ceiling.
--
-- ── WHY AN OFFSET AND NOT A LARGER CEILING ──────────────────────────────────────────────────────
--
-- The ceiling is 50, published on the act and identical to the find family's
-- (`registry.rs:161,207,233,363` — `bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)])` on
-- `FindExact`, `FindAboutAnywhere`, `FindAboutWithin` and `FollowFrom` alike). Raising it for this
-- one act would make the number mean something different per act while still being reported through
-- the same `applied_terms` channel, and it would not fix the actual gap: however large the ceiling,
-- a caller can still only ever see the FIRST page of it.
--
-- The gap is narrower than "the ceiling is too small". The three find acts already declare
-- `accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset]` (`registry.rs:159,205,231`) and
-- their fragments already end `LIMIT p_limit OFFSET p_offset` (`20260808000030:174,233,289`).
-- `FollowFrom` declares `accepts_bound_terms: vec![BoundTerm::Limit]` (`registry.rs:361`) — and
-- could not have declared more, because THE FRAGMENT HAS NO SLOT. So the walk is the one scoring
-- act in the family whose second page is unreachable, and the fix is the parameter the others
-- already have rather than a number nobody else's ceiling agrees with.
--
-- `Offset` deliberately carries no ceiling anywhere (`registry.rs:653`: "`Offset` has no ceiling on
-- any act, so there is nothing for it to default to"). This file adds none.
--
-- ── THE TIEBREAK ALREADY EXISTED. IT WAS NOT INVENTED HERE ──────────────────────────────────────
--
-- Paging is sound only over a TOTAL order — a `LIMIT/OFFSET` pair over ties returns an arbitrary and
-- unstable slice. `ranked` has ordered by `MAX(w.score) DESC, w.node` since `20260814000030:207`,
-- unchanged through `20260816000010:224` and `20260817000010:85`. `w.node` is a uuid and unique
-- within the GROUP BY, so the order is already total.
--
-- That is a CONSTRAINT this file honors, not a property it establishes: the `ORDER BY` is untouched,
-- and a second tiebreak is NOT added. One exists; adding another would silently reorder pages
-- against the incumbent for no benefit. The same rule is written down for the find arms at
-- `20260806000020:275` — "The tiebreak is part of the contract".
--
-- ── NO `DEFAULT` ON THE WIDENED SIGNATURES, AND THAT IS LOAD-BEARING ────────────────────────────
--
-- `20260815000010` measured the consequence and recorded it twice, on the function comment and in
-- its own register note: "measured on pg18, a default on the added parameter makes the incumbent
-- 8-arity call ambiguous and therefore breaks `search_graph_expand`, the compiler's emitted call and
-- four test call sites at run time."
--
-- The same rule applies one arity up and for the same reason: a `DEFAULT` on `p_offset` would make
-- every existing 9-argument call resolvable against two candidates. So the new 10-arity carries NO
-- default on ANY parameter, and reaching it requires naming all ten arguments. The two incumbent
-- arities keep their exact signatures and pass `NULL`.
--
-- `OFFSET NULL` is "no offset", the same way `LIMIT NULL` is "unbounded" — which this walk has
-- relied on since it was written: `search_graph_expand` passes `NULL` into `p_limit`
-- (`20260814000030:275`) and the 8-arity core declares `p_limit int DEFAULT NULL`
-- (`20260815000010:142`), both reaching a body whose `ranked` ends in a bare `LIMIT p_limit`. The
-- delegators below rely on the OFFSET half of that same clause, so it is stated as measured rather
-- than as read: `[verified — 2026-08-17, pg18 local]`
--
--     WITH t(n) AS (SELECT * FROM generate_series(1,5))
--     SELECT count(*) FROM (SELECT n FROM t ORDER BY n LIMIT NULL OFFSET NULL) s;  -- 5
--     WITH t(n) AS (SELECT * FROM generate_series(1,5))
--     SELECT count(*) FROM (SELECT n FROM t ORDER BY n LIMIT NULL OFFSET 2) s;     -- 3
--
-- Five of five under `OFFSET NULL` is "no offset"; the second row proves the clause was live rather
-- than ignored, which one query alone would not have distinguished.
--
-- Note the find family spells its idle offset `p_offset int DEFAULT 0` (`20260808000030:145,190`)
-- rather than NULL. That is a different position in the call graph — a DEFAULT is exactly what this
-- file may not have — and `0` and `NULL` are the same slice.
--
-- ── WHAT IS STILL NOT A CALLER INPUT ────────────────────────────────────────────────────────────
--
-- `p_depth` and `p_gamma` remain DEFINITIONAL `[ruled — 2026-08-14, Pete]` and are untouched here.
-- The ruling is recorded in full at `20260814000030:43-56`: gamma because `orders_by.means` is a
-- fixed sentence describing what `graph_score` IS, depth because "depth 3 is too large for a
-- neighborhood traversal of this kind" is a claim about what `follow-from` MEANS. `limit` was
-- always in the other category, and `offset` joins it — a page boundary says nothing about what the
-- act means, only about which part of its answer you are looking at.
--
-- ── ONE BODY PER ARM ────────────────────────────────────────────────────────────────────────────
--
-- Same move as `20260815000010`: the widest arity carries the walk, and the incumbent arity is
-- re-pointed at it by `CREATE OR REPLACE` at a byte-identical signature. The 8-arity delegators
-- (`20260815000010:134,170`) are untouched and reach the body through the 9-arity, so the chain is
-- 8 -> 9 -> 10 and the walk exists exactly once. The body below is `20260817000010`'s, carried
-- across unchanged apart from the added `OFFSET`.
--
-- Additive: two CREATE FUNCTION and two CREATE OR REPLACE at byte-identical signatures. No DROP —
-- a DROP is non-additive, halts `temper-migrate --additive-only` at deploy, and blocks every
-- subsequent deploy on that target until an operator takes a cutover (`20260814000030:14-17`).
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

-- ── 1. The ungated core, widened ────────────────────────────────────────────────────────────────
CREATE FUNCTION __temper_ungated_follow_from(
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
     WHERE w.hop < p_depth
       AND NOT u.to_node = ANY(w.path)
  ),
  -- `ORDER BY MAX(w.score) DESC, w.node` is unchanged from 20260814000030:207 and is what makes the
  -- OFFSET sound: `w.node` is a uuid, unique within the GROUP BY, so the order is already TOTAL and
  -- page N+1 cannot repeat or skip a row of page N. The tiebreak was not added for paging; paging
  -- became expressible because it was already there.
  --
  -- The whole ranking is still computed before the page is cut — an offset reduces what is
  -- DESCRIBED (the `via` subquery runs per surviving row), never what is walked. That is the same
  -- bound the limit gives and the reason both are applied HERE rather than on the final SELECT.
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

-- ── 2. The gated wrapper, widened ───────────────────────────────────────────────────────────────
CREATE FUNCTION query_follow_from(
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
      FROM __temper_ungated_follow_from(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties, p_offset) c;
$$;

-- ── 3. The 9-arity core, re-pointed ─────────────────────────────────────────────────────────────
--
-- Signature byte-identical to 20260817000010:11-21 (which is itself byte-identical to
-- 20260815000010:28-37) — same ten identifiers, same types, still no defaults. Its body stops being
-- the walk and becomes a delegation, so the walk exists in one place. The 8-arity delegator at
-- 20260815000010:134 reaches this one and needs no edit.
CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids     uuid[],
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from(
             p_visible_ids, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties, NULL::int) c;
$$;

-- ── 4. The 9-arity gated wrapper, re-pointed ────────────────────────────────────────────────────
--
-- Signature byte-identical to 20260815000010:151-160. It delegates sideways to the widened wrapper
-- rather than down to the core, exactly as its 8-arity sibling delegates to it — so
-- `resources_visible_to` is still computed exactly once per call.
CREATE OR REPLACE FUNCTION query_follow_from(
    p_principal       uuid,
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM query_follow_from(
             p_principal, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties, NULL::int) c;
$$;

COMMENT ON FUNCTION __temper_ungated_follow_from(
        uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int) IS
    'The walk, widened with p_offset so it can be PAGED. NULL is no offset, the same way NULL p_limit is unbounded; the two are applied together in `ranked`, before provenance is described, so a page bounds what is DESCRIBED rather than what is walked. Paging is sound only because `ranked` was already ordered by MAX(score) DESC, w.node — a total order, since w.node is a uuid unique within the GROUP BY — and that tiebreak dates to 20260814000030 rather than to this widening: no second tiebreak was added, because adding one would silently reorder pages against the incumbent for nothing. A larger ceiling was refused instead: the ceiling of 50 is published per act and is the find family''s number too, so raising it here alone would make the same reported value mean different things per act while still leaving only the first page reachable. Offset carries no ceiling on any act and gains none here. p_depth and p_gamma remain definitional rather than caller inputs (ruled 2026-08-14) and are untouched. Carries NO DEFAULT on any parameter, and that is load-bearing for the same measured reason 20260815000010 recorded one arity down: a default makes the 9-arity incumbent ambiguous and every existing call to it an error. This arity holds the only copy of the walk; the 9- and 8-arity signatures delegate to it.';

COMMENT ON FUNCTION query_follow_from(
        uuid, uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int) IS
    'Gated wrapper over the offset-widened __temper_ungated_follow_from: computes resources_visible_to(p_principal) once and passes it down. Carries p_offset so a direct caller can page a walk the way the find acts already page their arms (accepts_bound_terms Limit + Offset). No defaults, for the reason recorded on the core.';

SELECT declare_migration(
    20260817000020,
    'additive',
    'Gives the follow-from walk an offset so a result set larger than the act''s published ceiling can be PAGED. Adds p_offset to __temper_ungated_follow_from and query_follow_from as a new 10-arity of each, applied as OFFSET p_offset beneath the existing LIMIT p_limit inside `ranked` — before the via subquery, so a page bounds what is described rather than what is walked. A larger ceiling was refused as the alternative: 50 is published on the act and is the same number the three find acts publish (registry.rs:161,207,233,363), so raising it for this act alone would make one reported value mean different things per act and would still leave only the first page reachable; the actual gap is that FindExact, FindAboutAnywhere and FindAboutWithin declare accepts_bound_terms Limit + Offset and end their fragments LIMIT p_limit OFFSET p_offset, while FollowFrom declares Limit alone because the fragment had no slot. The ORDER BY MAX(score) DESC, w.node in `ranked` is UNCHANGED and is what makes paging sound — w.node is a uuid unique within the GROUP BY, so the order was already total; the tiebreak dates to 20260814000030 and no second one was added here. Offset carries no ceiling on any act (registry.rs:653) and gains none. p_depth and p_gamma remain definitional rather than caller inputs (ruled 2026-08-14, 20260814000030:43-56) and are untouched. TWO CREATE FUNCTION (widened core and widened gated wrapper) and TWO CREATE OR REPLACE at byte-identical signatures re-pointing the 9-arity core and the 9-arity wrapper to delegate with NULL::int; the 8-arity delegators from 20260815000010 are untouched and reach the body through the 9-arity, so the walk exists exactly once and the chain is 8 -> 9 -> 10. NO DROP anywhere, which is the point rather than an accident: a DROP is non-additive, halts temper-migrate --additive-only at deploy and blocks every subsequent deploy on that target until an operator takes a cutover. The widened signatures carry NO DEFAULT on any parameter, honoring the rule 20260815000010 measured on pg18 one arity down — a default on the added parameter makes the incumbent arity ambiguous and every existing call to it an error at run time, which would break search_graph_expand, the compiler''s emitted call and the substrate test call sites.'
);
