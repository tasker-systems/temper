-- `follow-from` gains an OFFSET, so a walk wider than the act's published ceiling of 50 can be
-- paged instead of truncated at the first page.
--
-- WHY AN OFFSET RATHER THAN A LARGER CEILING. The ceiling is published per act and is the same 50
-- the three find acts publish, so raising it here alone would make one reported number mean
-- different things per act — and however large it grew, only the first page would ever be
-- reachable. The find acts pair that ceiling with an offset; the walk could not, because the
-- fragment had no slot. This adds the slot. `registry.rs` gains the matching `BoundTerm::Offset`
-- in the same change: a declaration describes the DEPLOYED system, so neither half is true alone.
--
-- WHY THE PAGE IS SOUND. `ranked`'s `ORDER BY MAX(w.score) DESC, w.node` is untouched. `w.node` is
-- the GROUP BY key and a uuid, so the order was already TOTAL and no tiebreak was added for paging
-- — paging became expressible because one was already there. Cutting inside `ranked` rather than
-- on the final SELECT is also what keeps `via` describing only the rows that ship.
--
-- WHY NO `DEFAULT` ON ANY PARAMETER OF THE NEW ARITIES. A default makes the incumbent arity
-- ambiguous and every existing call to it an error at run time — measured on pg18 one arity down,
-- and recorded in `20260815000010`'s register note. Reaching the new form means naming all ten
-- arguments.
--
-- WHY THE INCUMBENT ARITIES BECOME DELEGATIONS. One body per arm: two copies of this walk would
-- drift silently, both still returning plausible rows. The widest arity holds the body and 8 -> 9
-- -> 10 delegates down to it.
--
-- Additive: two CREATE FUNCTION, two CREATE OR REPLACE at unchanged signatures, no DROP. A DROP is
-- non-additive and halts `temper-migrate --additive-only` at deploy.
--
-- Everything else — the measurements, the paging/consistency semantics, the alternatives weighed —
-- is in task 01a0112c-4155-7b62-9a63-85e79c970125 and the PR, deliberately not here: a migration is
-- immutable once applied, so anything in it that dates cannot be corrected.
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
-- Unchanged signature, so this REPLACES the 9-arity rather than overloading it. Its body stops
-- being the walk and becomes a delegation, leaving the walk in one place. The 8-arity delegator
-- reaches this one and needs no edit.
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
-- Unchanged signature. Delegates sideways to the widened wrapper rather than down to the core, so
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
    'The walk, widened with p_offset so it can be PAGED past the act''s published ceiling. NULL is no offset, as NULL p_limit is unbounded; both are applied in `ranked` before provenance is described, so a page bounds what is DESCRIBED rather than what is walked. Sound because `ranked` was already ordered by MAX(score) DESC, w.node — a total order predating this change, to which no second tiebreak was added. Carries NO DEFAULT on any parameter: a default makes the 9-arity incumbent ambiguous and every existing call to it an error, measured one arity down by 20260815000010. This arity holds the only copy of the walk; 8 and 9 delegate to it. Rationale: task 01a0112c-4155-7b62-9a63-85e79c970125.';

COMMENT ON FUNCTION query_follow_from(
        uuid, uuid[], int, double precision, text[], text[], uuid[], int, jsonb, int) IS
    'Gated wrapper over the offset-widened __temper_ungated_follow_from: computes resources_visible_to(p_principal) once and passes it down. Carries p_offset so a direct caller can page a walk the way the find acts already page their arms (accepts_bound_terms Limit + Offset). No defaults, for the reason recorded on the core.';

SELECT declare_migration(
    20260817000020,
    'additive',
    'Gives the follow-from walk an offset so a neighbourhood larger than the act''s published ceiling of 50 can be paged rather than truncated at page one. Adds p_offset as a tenth parameter of __temper_ungated_follow_from and query_follow_from, applied as OFFSET p_offset beneath the existing LIMIT inside `ranked`. TWO CREATE FUNCTION (widened core and gated wrapper) and TWO CREATE OR REPLACE re-pointing the 9-arity core and wrapper to delegate; the 8-arity delegators are untouched, so the walk exists exactly once and the chain is 8 -> 9 -> 10. NO DROP, and NO DEFAULT on any parameter of the new arities — a default makes the incumbent arity ambiguous at run time, measured on pg18 by 20260815000010. registry.rs admits BoundTerm::Offset in the same change. Design and measurements: task 01a0112c-4155-7b62-9a63-85e79c970125.'
);
