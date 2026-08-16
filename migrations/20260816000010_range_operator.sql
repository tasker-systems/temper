-- The ordering operator: `PropertyOp::Compare`, a third arm of the closed operator set
-- declared in §12 of `2026-08-05-query-builder-compositional-design.md` ("open keys, closed
-- operators"). `Compare` carries one bound and a closed `OrdOp` direction (`gt`/`gte`/`lt`/`lte`),
-- compiled to jsonb's native ordering (`<`, `>`, `<=`, `>=`) under a `jsonb_typeof` type guard.
--
-- `[ruled — 2026-08-16, Pete]` Three contract decisions, recorded in the spec
-- (`01a00a79-1dd7-7ae2-9d34-b801eb509904`) and NOT re-litigated here:
--   1. ONE `Compare` variant with four directions. `Between` is NOT added — it composes from
--      `gte` AND `lte` through the existing AND-across-the-list, and adding it would cost a
--      second value slot and SQL branch for one saved probe.
--   2. jsonb NATIVE ordering, type-guarded by `jsonb_typeof(property_value) = jsonb_typeof($bound)`.
--      Cross-type rows fall to `ELSE false` — an honest empty, not a type-confusion match. jsonb
--      defines a total type ordering (`null < boolean < number < string < array < object`), so
--      without the guard a numeric bound against a string-valued key would match EVERY string row
--      (`string > number` is true in jsonb's ordering) — a type-confusion artifact, not an answer.
--   3. Type is INFERRED from the caller's bound value, never declared on the operator. No `as`
--      field: per-VALUE inference (the trap rules out per-key — `temper-pr` is 68 string / 7
--      numeric on ONE key, so no per-key answer exists; each caller sends one bound with one
--      JSON type, and the guard makes the other-type rows honest empties).
--
-- `probe_count` for `Compare` is 1 (one bound, one comparison per row that carries the key), like
-- `HasKey` — not like `Contains { values }` whose cost is `Σ|values|`. The 256 probe ceiling and
-- the cap arithmetic are unchanged; the refusal text generalized from "containment probes" to
-- "probes" since the set now includes comparisons.
--
-- This migration is `additive`: it adds a `WHEN 'compare'` arm to the `CASE q.op` in BOTH
-- predicate bodies and narrows the `ELSE false` catch (which already existed for unknown
-- operators). No existing arm's semantics change. No DROP, no signature change, no default
-- change. The gated wrappers (`query_find_resources_with`, `query_follow_from`) delegate to the
-- ungated bodies and pass `p_properties` / `p_edge_properties` through unchanged.
--
-- The two bodies CANNOT be unified into a shared function, and that is measured rather than
-- stylistic (carried — `20260808000020:308`): a `LANGUAGE sql STABLE` predicate whose body
-- contains a sublink does not inline — the `EXISTS` loses its Index Only Scan on
-- `uq_kb_properties_active` and becomes a per-row call. They are held to a differential witness
-- instead: `crates/temper-substrate/tests/property_predicate_parity.rs`, which is
-- `artifact-tests`-gated and runs only under `cargo make test-artifacts` locally (CI's Substrate
-- Artifact Tests job runs it). A symmetric edit — both bodies wrong identically — passes the
-- differential; the companion test `the_shared_predicate_admits_and_denies_the_cases_it_is_supposed_to`
-- pins the expected verdicts and catches that.

-- ── `__temper_ungated_find_resources_with`: add the `compare` arm ──────────────────────────────
CREATE OR REPLACE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
  -- The wire shape parsed once into columns, exactly as the edge sibling parses it. `PropertyOp`
  -- is internally tagged inside a field named `op`, hence `q->'op'->>'op'`. `Compare` adds two
  -- columns: `direction` (the `OrdOp` discriminant) and `bound` (the single value). `Contains`
  -- still carries its list in `vals`; `HasKey` carries neither.
  WITH predicates AS (
    SELECT q->>'key'      AS property_key,
           q->'op'->>'op' AS op,
           CASE jsonb_typeof(q->'op'->'values')
             WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END AS vals,
           q->'op'->>'direction' AS direction,
           q->'op'->'value'       AS bound
      FROM jsonb_array_elements(
             CASE jsonb_typeof(p_properties)
               WHEN 'array' THEN p_properties ELSE '[]'::jsonb END) AS q
  )
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resources_live lv                    ON lv.id = r.id
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
      JOIN kb_resource_homes h                     ON h.resource_id = r.id
     WHERE (p_doc_types IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_doc_type dt
              WHERE dt.resource_id = r.id AND dt.doc_type = ANY(p_doc_types)))
       AND (p_stage IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.stage = p_stage))
       AND (p_status IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.status = p_status))
       AND (p_owner_profile IS NULL OR h.owner_profile_id = p_owner_profile)
       AND (p_owner_handle IS NULL OR EXISTS (
             SELECT 1 FROM kb_profiles p
              WHERE p.id = h.owner_profile_id AND p.handle = p_owner_handle))
       AND (p_title_contains IS NULL OR r.title ILIKE '%' || p_title_contains || '%')
       -- AND-containment, both sides folded. A resource with no `tags` is excluded because the
       -- `coalesce` makes it the EMPTY ARRAY — NOT because it is NULL. Drop the `coalesce` and
       -- every tag filter becomes NULL and matches nothing.
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{}')), '{}')
               FROM kb_property_elements pe
              WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                AND pe.property_key = 'tags'
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
       -- Three guards, distinct jobs: `jsonb_typeof` fails closed, the `CASE` stops
       -- `jsonb_array_elements` raising, the null-key test stops `jsonb_build_object` raising.
       --
       -- Reading `pe.element` rather than `fp.property_value` WIDENS this: an ARRAY-shaped facet
       -- value explodes, and a one-element array wrapping an object does not contain that object
       -- while the element does (jsonb's array-containment exception is primitives only).
       AND (p_facets IS NULL OR (
             jsonb_typeof(p_facets) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM jsonb_array_elements(
                 CASE jsonb_typeof(p_facets) WHEN 'array' THEN p_facets ELSE '[]'::jsonb END) AS f
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_property_elements pe
                   WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                     AND pe.property_key = 'facet'
                     AND f->>'key' IS NOT NULL
                     AND pe.element @> jsonb_build_object(f->>'key', f->>'value')))))
       -- Open key space, closed operator set. AND across the list, OR within one predicate's
       -- values. NULL narrows nothing; a malformed argument narrows to zero, and the fail-closed
       -- test must stay HERE rather than in `predicates`, which is empty for an absent argument and
       -- a malformed one alike — the distinction does not survive the parse.
       --
       -- `has_key` needs no value test: the row's existence in the view IS the answer, which is why
       -- it can be asked of a `[]`-valued key that the element relation cannot see at all.
       --
       -- `compare` is type-guarded: `jsonb_typeof(property_value) = jsonb_typeof($bound)` segments
       -- by JSON type before the comparison, so a row whose JSON type differs from the bound's is
       -- an honest empty rather than a type-confusion match. jsonb's total type ordering
       -- (`null < boolean < number < string < array < object`) means without the guard a numeric
       -- bound against a string-valued key matches EVERY string row — a type-confusion artifact.
       -- A missing `value` makes `bound` NULL; `jsonb_typeof(NULL)` is NULL, so the guard is
       -- falsy and the whole arm denies. A missing `direction` falls to the inner `ELSE false`.
       -- Both match the established fail-closed rule: a malformed argument narrows to zero.
       AND (p_properties IS NULL OR (
             jsonb_typeof(p_properties) = 'array'
             AND NOT EXISTS (                       -- no listed predicate FAILS to match
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_resource_properties rp
                   WHERE rp.resource_id = r.id
                     AND rp.property_key = q.property_key
                     AND CASE q.op
                           WHEN 'has_key'  THEN true
                           WHEN 'contains' THEN EXISTS (   -- OR within one predicate's values
                             SELECT 1 FROM jsonb_array_elements(q.vals) AS v
                              WHERE rp.property_value @> v)
                           WHEN 'compare' THEN
                                 jsonb_typeof(rp.property_value) = jsonb_typeof(q.bound)
                             AND CASE q.direction
                                   WHEN 'gt'  THEN rp.property_value >  q.bound
                                   WHEN 'gte' THEN rp.property_value >= q.bound
                                   WHEN 'lt'  THEN rp.property_value <  q.bound
                                   WHEN 'lte' THEN rp.property_value <= q.bound
                                   ELSE false
                                 END
                           ELSE false                      -- an operator the closed set lacks
                         END))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

-- The gated wrapper delegates and is byte-identical in signature; no second `CREATE OR REPLACE`
-- is needed because the wrapper passes `p_properties` through and its body did not change. The
-- incumbent `CREATE OR REPLACE FUNCTION __temper_ungated_find_resources_with` with the DEFAULT
-- arity above already re-points the 12-arity incumbent's delegate (`20260815000040:134-154`), and
-- that delegate passes `NULL::jsonb` for `p_properties`, so it is unaffected by the new arm.

-- ── `__temper_ungated_follow_from`: add the `compare` arm to the edge predicate ─────────────────
CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids      uuid[],
    p_seed_ids         uuid[],
    p_depth            int,
    p_gamma            double precision,
    p_edge_kinds       text[],
    p_labels           text[],
    p_bound_ids        uuid[],
    p_limit            int,
    p_edge_properties  jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE
  -- `PropertyOp` is internally tagged inside a field named `op`, hence `q->'op'->>'op'`. The
  -- `CASE`s are raise-guards: `jsonb_array_elements` errors on a non-array, and a `WHERE` beside a
  -- lateral SRF runs after the expansion that already raised. `Compare` adds `direction` and
  -- `bound` columns; `Contains` still carries its list in `vals`; `HasKey` carries neither.
  predicates AS (
    SELECT q->>'key'      AS property_key,
           q->'op'->>'op' AS op,
           CASE jsonb_typeof(q->'op'->'values')
             WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END AS vals,
           q->'op'->>'direction' AS direction,
           q->'op'->'value'       AS bound
      FROM jsonb_array_elements(
             CASE jsonb_typeof(p_edge_properties)
               WHEN 'array' THEN p_edge_properties ELSE '[]'::jsonb END) AS q
  ),
  admitted AS (
    SELECT v.id
      FROM unnest(p_visible_ids) AS v(id)
     WHERE p_bound_ids IS NULL
        OR EXISTS (SELECT 1 FROM unnest(p_bound_ids) AS b(id) WHERE b.id = v.id)
  ),
  adj AS (
    SELECT e.source_id, e.target_id, e.weight,
           e.edge_kind::text AS edge_kind, e.label, e.polarity::text AS polarity
      FROM kb_edges e
      JOIN admitted sa ON sa.id = e.source_id
      JOIN admitted ta ON ta.id = e.target_id
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
            OR e.edge_kind::text = ANY(p_edge_kinds))
       AND (p_labels IS NULL OR array_length(p_labels, 1) IS NULL
            OR e.label = ANY(p_labels))
       -- Fail-closed lives here and cannot move into `predicates`: that CTE is empty for an absent
       -- argument and a malformed one alike, so the distinction does not survive the parse.
       -- `compare` carries the same `jsonb_typeof` type guard as the resource body — see the
       -- sibling migration's `WHERE` for the full rationale. Edge-owned properties are zero on
       -- this deployment but the shape is identical for both subjects by design (`§12`).
       AND (p_edge_properties IS NULL OR (
             jsonb_typeof(p_edge_properties) = 'array'
             AND NOT EXISTS (                       -- no listed predicate FAILS to match
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_edge_properties ep
                   WHERE ep.edge_id = e.id
                     AND ep.property_key = q.property_key
                     AND CASE q.op
                           WHEN 'has_key'  THEN true
                           WHEN 'contains' THEN EXISTS (   -- OR within one predicate's values
                             SELECT 1 FROM jsonb_array_elements(q.vals) AS v
                              WHERE ep.property_value @> v)
                           WHEN 'compare' THEN
                                 jsonb_typeof(ep.property_value) = jsonb_typeof(q.bound)
                             AND CASE q.direction
                                   WHEN 'gt'  THEN ep.property_value >  q.bound
                                   WHEN 'gte' THEN ep.property_value >= q.bound
                                   WHEN 'lt'  THEN ep.property_value <  q.bound
                                   WHEN 'lte' THEN ep.property_value <= q.bound
                                   ELSE false
                                 END
                           ELSE false                      -- an operator the closed set lacks
                         END))))
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path,
           s.id AS seed_id,
           NULL::uuid AS e_source, NULL::uuid AS e_target,
           NULL::text AS e_kind, NULL::text AS e_label, NULL::text AS e_polarity
      FROM unnest(p_seed_ids) AS s(id)
     WHERE EXISTS (SELECT 1 FROM admitted a WHERE a.id = s.id)
    UNION ALL
    SELECT nb.node, w.score * p_gamma * nb.weight, w.hop + 1, w.path || nb.node,
           w.seed_id, nb.source_id, nb.target_id, nb.edge_kind, nb.label, nb.polarity
      FROM walk w
      JOIN LATERAL (
        SELECT adj.target_id AS node, adj.weight, adj.source_id, adj.target_id,
               adj.edge_kind, adj.label, adj.polarity
          FROM adj WHERE adj.source_id = w.node
        UNION ALL
        SELECT adj.source_id AS node, adj.weight, adj.source_id, adj.target_id,
               adj.edge_kind, adj.label, adj.polarity
          FROM adj WHERE adj.target_id = w.node
      ) nb ON true
     WHERE w.hop < p_depth
       AND NOT nb.node = ANY(w.path)
  ),
  ranked AS (
    SELECT w.node, MAX(w.score)::real AS graph_score
      FROM walk w
     WHERE w.hop > 0
     GROUP BY w.node
     ORDER BY MAX(w.score) DESC, w.node
     LIMIT p_limit
  )
  SELECT r.node, r.graph_score,
         (SELECT jsonb_agg(DISTINCT jsonb_build_object(
                   'seed_id',   w.seed_id,
                   'source_id', w.e_source,
                   'target_id', w.e_target,
                   'edge_kind', w.e_kind,
                   'label',     w.e_label,
                   'polarity',  w.e_polarity))
            FROM walk w
           WHERE w.node = r.node AND w.hop > 0)
    FROM ranked r
   ORDER BY r.graph_score DESC, r.node;
$$;

-- The gated wrapper `query_follow_from` delegates to `__temper_ungated_follow_from` and passes
-- `p_edge_properties` through; its signature did not change, so no second `CREATE OR REPLACE` is
-- needed.

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, uuid, jsonb) IS
    'The selection core, with the closed operator set widened to include `compare`: property_value <direction> $bound over jsonb native ordering, type-guarded by jsonb_typeof(property_value) = jsonb_typeof($bound) so cross-type rows are honest empties rather than type-confusion matches. Two `CREATE OR REPLACE FUNCTION` at byte-identical signatures: the bodies are replaced to add the `WHEN ''compare''` arm to the `CASE q.op`; no DROP, no signature change, no default change. The two predicate bodies CANNOT be unified into a shared function (measured — 20260808000020:308: a LANGUAGE sql STABLE body with a sublink does not inline and loses its Index Only Scan); they are held to a differential witness in property_predicate_parity.rs. `compare` is one probe (one bound, one comparison per row), like has_key; the 256 probe ceiling is unchanged. See migration 20260815000040 for the container it rides on and 2026-08-05-query-builder-compositional-design.md §12 for the closed-operator contract.';

COMMENT ON FUNCTION __temper_ungated_follow_from(
        uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb) IS
    'The walk, with the closed operator set widened to include `compare`: same type-guarded jsonb native ordering as the resource body, applied inside `adj` because it constrains which edge may be TRAVERSED. Two `CREATE OR REPLACE FUNCTION` at byte-identical signatures; no DROP, no signature change, no default change. Edge-owned properties are zero on this deployment but the shape is identical for both subjects by design (§12). The two predicate bodies are held to a differential witness rather than unified (measured; see the sibling migration 20260816000010). See 2026-08-05-query-builder-compositional-design.md §12.';

SELECT declare_migration(
    20260816000010,
    'additive',
    'Widens the closed PropertyOp set with a third arm, `Compare { direction, value }`, carrying a closed OrdOp sub-enum (gt/gte/lt/lte). Three contract rulings (task 01a001af-02a9-7c02-9977-195aeecadefb, spec 01a00a79-1dd7-7ae2-9d34-b801eb509904): ONE variant with four directions, no Between (composes from gte AND lte); jsonb NATIVE ordering type-guarded by jsonb_typeof(property_value) = jsonb_typeof($bound) so cross-type rows are honest empties, not type-confusion matches; type INFERRED from the caller''s bound value, no declared `as` field. probe_count for compare is 1, like has_key; the 256 probe ceiling is unchanged. Two CREATE OR REPLACE FUNCTION at byte-identical signatures (the two ungated bodies cannot be unified — measured, 20260808000020:308); no DROP, no signature change, no default change. The gated wrappers delegate and are unaffected. Differential witness in property_predicate_parity.rs gains compare cases in both directions and both type-guard directions; the companion test pins expected verdicts so a symmetric mistake on both bodies cannot pass. Rulings, measurements and the type-instability corpus live in the spec — deliberately not here, because an applied migration cannot have its prose corrected.'
);