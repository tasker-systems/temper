-- `EdgeFilter`'s third axis: property predicates that constrain a HOP.
--
-- Design: docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md,
-- §6.5's edge half. Task 01a000c2-033c-7451-8b13-b7aa7469d217.
--
-- ── WHY THIS IS A HOP PREDICATE AND NOT AN ACT ──────────────────────────────────────────────────
--
-- `[decided — 2026-08-14, Pete]` *A narrowing that can be expressed as a set must be an act. A
-- narrowing that cannot be a set belongs to the act whose semantics it constrains.*
--
-- The set-shaped substitute — "nodes that participate in an edge matching P", piped in as a bound —
-- is not this, and looks identical. It admits a node because it has a matching edge SOMEWHERE and
-- then walks it through a different, non-matching one, returning plausible rows while appearing to
-- have narrowed. So the predicate goes where the traversal is: inside `adj`, beside `p_edge_kinds`
-- and `p_labels`, which is the same place and for the same reason.
--
-- ── ADDING A PARAMETER TO A SHIPPED FUNCTION: A DEFAULT MAKES THE INCUMBENT ARITY UNCALLABLE ────
--
-- `20260814000010` already recorded that *"a new parameter makes a new function, so --additive-only
-- would have halted the deploy"*. **`[measured — 2026-08-15]` refines it, and the refinement is the
-- whole shape of this file**: the new function is not the problem, the DEFAULT on the new parameter
-- is. Against Postgres 18:
--
--   CREATE FUNCTION f(a int, b int DEFAULT NULL);
--   CREATE FUNCTION f(a int, b int DEFAULT NULL, c jsonb DEFAULT NULL);
--   SELECT f(1, 2);
--   ERROR:  function f(integer, integer) is not unique
--
-- The incumbent arity does not merely keep its old behaviour — **every existing call to it becomes
-- an error**, because the wider function is a candidate for it through its default. Applied here
-- that would have broken `search_graph_expand`, `query_follow_from`, the compiler's emitted call and
-- four test call sites, at run time, on a migration that declares itself additive.
--
-- With NO default on any parameter of the wider function, an N-argument call has exactly one
-- candidate and resolution is unambiguous in both directions — measured the same way, and that is
-- why the widened signatures below carry no `DEFAULT` at all:
--
--   arity 8 -> the incumbent          arity 9 -> the widened one
--
-- ── ONE BODY PER ARM, WHICH IS WHY THE INCUMBENT ARITY IS RE-POINTED RATHER THAN LEFT ───────────
--
-- `20260808000030`'s rule. Left alone, the 8-arity function would be a second walk that must agree
-- with this one and is linked to it by nothing — "two bodies drift, and the drift is silent because
-- both keep returning plausible rows". So it is `CREATE OR REPLACE`d at a byte-identical signature
-- into a two-line delegation, and there is exactly one body of the walk in the schema afterwards.
--
-- It survives at all only because DROP is not additive. Nothing in this repo calls it after this
-- migration except `search_graph_expand`, whose signature is a contract this file does not touch.
--
-- ── WHAT THE PREDICATE DOES, AND WHERE IT IS BORROWED FROM ──────────────────────────────────────
--
-- AND across the list, OR within one predicate's `values` — the idiom settled by
-- `20260814000010`'s facets block, spelled the same way ("no listed predicate FAILS to match",
-- which is universal quantification rendered directly and short-circuits on the first miss).
-- `NULL` narrows nothing, the opposite polarity from `p_visible_ids` beside it.
--
-- **The argument IS the serialization of `Vec<PropertyPredicate>`, verbatim** — which is why the
-- operator is read at `q->'op'->>'op'` and not at `q->>'op'`. `PropertyOp` is an internally-tagged
-- enum in a field called `op`, so `{"key":k,"op":{"op":"contains","values":[…]}}` is what the type
-- already produces. The compiler binds `serde_json::to_value` of the typed vector and builds no JSON
-- of its own: a hand-assembled object here (which `20260814000010`'s facets slot does have) is a
-- second spelling of the shape, free to drift from the type it claims to carry.
--
-- ── FAILING CLOSED TAKES TWO GUARDS, AND NORMALIZATION ALONE IS THE WRONG ONE ──────────────────
--
-- `jsonb_array_elements` RAISES on a non-array, so both levels are normalized before the
-- set-returning function sees them. **`[found — 2026-08-15, by the witness below]` normalizing a
-- malformed argument to `'[]'` does not fail closed — it fails OPEN**, and the two read alike in a
-- comment. `NOT EXISTS` over zero elements is TRUE, so every edge passes and a caller who asked to
-- narrow receives an UNNARROWED walk: the silent substitution this whole contract exists to
-- prevent, arriving through the error path.
--
-- So the argument carries two guards with two distinct jobs, and neither substitutes for the other:
--
--   * `jsonb_typeof(...) = 'array'` — a plain scalar test, and the one that makes a malformed
--     argument narrow to NOTHING;
--   * the `CASE` — what keeps `jsonb_array_elements` from raising, whatever order the planner
--     evaluates the conjuncts in. A `WHERE` guard beside a lateral SRF is evaluated after the
--     expansion that already raised, which is why the normalization stays even though the typeof
--     test now precedes it.
--
-- `values` needs only the `CASE`: a non-array there yields no element, the `EXISTS` is false, the
-- predicate is unmatched and the edge is excluded — already closed. Same for an unrecognized `op`.
--
-- **`20260814000010`'s facets slot has the first shape and describes itself as the second**
-- (*"Fail closed: a non-array narrows to nothing"*), and it is fail-OPEN:
-- `[measured — 2026-08-15]` over three visible ids, a malformed `p_facets` returns the same 1 row
-- an absent filter does, while a well-formed non-matching one returns 0. That is a different act's
-- fragment, a different door, and is deliberately NOT fixed here — task
-- `01a00510-e583-78b1-bdc5-f08f4ca483a8`.
--
-- All of this is unreachable through the compiler, whose input is the typed vector; these functions
-- are directly callable and a slot that turns a malformed filter into a server fault — or into a
-- confident full answer — is the wrong default to leave for the next caller.
--
-- An unrecognized `op` matches nothing, by the same rule. `HasKey` is a row-existence check on the
-- `property_key` btree and deliberately not a jsonb operator: `jsonb_path_ops` does not index
-- key-existence.
--
-- **ZERO EDGE-OWNED PROPERTIES EXIST IN PROD** `[measured — 2026-08-14]`, over a write path that
-- shipped in `20260727000030` against a schema whose DDL comment has said *"§4a edges carry facets"*
-- since `20260624000001:656`. So this predicate narrows nothing today **by data, not by design** —
-- which is exactly why its witness CREATES an edge-owned property rather than asserting over the
-- corpus. A green suite over zero rows would prove the code compiles against a case it never runs.
--
-- Additive: two CREATE FUNCTION and two CREATE OR REPLACE at byte-identical signatures. No DROP.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

-- ── 1. The widened core ─────────────────────────────────────────────────────────────────────────
--
-- NO DEFAULTS. See the header — this is what keeps the incumbent arity callable.
CREATE FUNCTION __temper_ungated_follow_from(
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
  WITH RECURSIVE
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
       -- The third axis. It constrains WHICH EDGE MAY BE TRAVERSED, which is why it sits here with
       -- the other two and not over the returned set — see the header's first section.
       -- **TWO guards, with two different jobs — see the header.** The `jsonb_typeof` test is what
       -- makes a malformed argument fail CLOSED; the `CASE` is what keeps it from RAISING whatever
       -- order the planner evaluates the conjuncts in. Neither substitutes for the other.
       AND (p_edge_properties IS NULL OR (
             jsonb_typeof(p_edge_properties) = 'array'
             AND NOT EXISTS (
             SELECT 1 FROM jsonb_array_elements(
               CASE jsonb_typeof(p_edge_properties)
                 WHEN 'array' THEN p_edge_properties ELSE '[]'::jsonb END) AS q
              WHERE NOT EXISTS (
                SELECT 1 FROM kb_properties ep
                 WHERE ep.owner_table = 'kb_edges' AND ep.owner_id = e.id
                   AND ep.property_key = q->>'key' AND NOT ep.is_folded
                   AND (q->'op'->>'op' = 'has_key'
                        OR (q->'op'->>'op' = 'contains' AND EXISTS (
                              SELECT 1 FROM jsonb_array_elements(
                                CASE jsonb_typeof(q->'op'->'values')
                                  WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END) AS val
                               WHERE ep.property_value @> val)))))))
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

-- ── 2. The incumbent core arity, re-pointed ─────────────────────────────────────────────────────
--
-- Signature byte-identical to 20260814000030:137-146, defaults included — CREATE OR REPLACE cannot
-- remove a default, and changing one would be a shape change wearing an additive statement.
CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids uuid[],
    p_seed_ids    uuid[],
    p_depth       int,
    p_gamma       double precision,
    p_edge_kinds  text[] DEFAULT NULL,
    p_labels      text[] DEFAULT NULL,
    p_bound_ids   uuid[] DEFAULT NULL,
    p_limit       int    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from(
             p_visible_ids, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, NULL::jsonb) c;
$$;

-- ── 3. The widened gated wrapper ────────────────────────────────────────────────────────────────
--
-- The compiler calls the CORE, so this exists for the same reason `query_follow_from` existed
-- before it: a direct caller (a test, a future door) that must not be handed the ungated body. It
-- gains the axis rather than being left one short — a gated wrapper that cannot express what its
-- core can is how a slot ends up reachable only through the ungated path.
CREATE FUNCTION query_follow_from(
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
      FROM __temper_ungated_follow_from(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties) c;
$$;

-- ── 4. The incumbent gated arity, re-pointed ────────────────────────────────────────────────────
--
-- Signature byte-identical to 20260814000030:232-241. One body per arm here too: it delegates to
-- the widened wrapper rather than re-computing the visible set, so `resources_visible_to` is named
-- in one place across both arities.
CREATE OR REPLACE FUNCTION query_follow_from(
    p_principal   uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_gamma       double precision,
    p_edge_kinds  text[] DEFAULT NULL,
    p_labels      text[] DEFAULT NULL,
    p_bound_ids   uuid[] DEFAULT NULL,
    p_limit       int    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM query_follow_from(
             p_principal, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, NULL::jsonb) c;
$$;

COMMENT ON FUNCTION __temper_ungated_follow_from(
        uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb) IS
    'The walk, widened with p_edge_properties: EdgeFilter''s third axis, applied inside `adj` beside p_edge_kinds and p_labels because it constrains WHICH EDGE MAY BE TRAVERSED. The set-shaped substitute — bind the walk to nodes that participate in a matching edge — is a different question that returns plausible rows: it admits a node for an edge somewhere and then walks it through a non-matching one. Shape is the verbatim serialization of Rust''s Vec<PropertyPredicate> — [{"key":k,"op":{"op":"has_key"}}, ...] — so the operator is read at q->''op''->>''op'' rather than at the top level, and the compiler binds the typed value instead of assembling a second spelling of it. AND across the list, OR within one predicate''s values, NULL narrows nothing (the opposite polarity from p_visible_ids). Both jsonb levels are normalized before jsonb_array_elements sees them and an unrecognized op matches nothing, so a malformed filter narrows to zero rather than raising. has_key is a row-existence check on the property_key btree, not a jsonb operator, because jsonb_path_ops does not index key-existence. Carries NO DEFAULT on any parameter, and that is load-bearing rather than stylistic: a default on the added parameter makes the 8-arity incumbent ambiguous and every existing call to it an error (measured, Postgres 18). Zero edge-owned properties exist in this deployment, so the axis narrows nothing here by data rather than by design.';

COMMENT ON FUNCTION query_follow_from(
        uuid, uuid[], int, double precision, text[], text[], uuid[], int, jsonb) IS
    'Gated wrapper over the widened __temper_ungated_follow_from: computes resources_visible_to(p_principal) once and passes it down as p_visible_ids. Carries p_edge_properties so a direct caller can express every axis the core can — a gated wrapper one axis short of its core is how a slot ends up reachable only through the ungated path. No defaults, for the incumbent-arity reason recorded on the core.';

SELECT declare_migration(
    20260815000010,
    'additive',
    'Gives EdgeFilter a third axis — property predicates over the edge''s own kb_properties rows (task 01a000c2-033c-7451-8b13-b7aa7469d217, design docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md §6.5). p_edge_properties is applied inside `adj` beside p_edge_kinds and p_labels because an edge predicate constrains a HOP and has no set-shaped substitute: binding the walk to "nodes that participate in a matching edge" admits a node for an edge somewhere and then walks it through a different, non-matching one, answering a different question while looking like it narrowed. AND across the list, OR within one predicate''s values, NULL narrows nothing; both jsonb levels are normalized before jsonb_array_elements sees them and an unrecognized op matches nothing, so a malformed filter narrows to zero rather than raising — the idiom 20260814000010 settled for facets, spelled the same way. TWO CREATE FUNCTION (the widened core and the widened gated wrapper) and TWO CREATE OR REPLACE at byte-identical signatures, no DROP. THE WIDENED SIGNATURES CARRY NO DEFAULT ON ANY PARAMETER, and that is the whole shape of the file rather than a style choice: measured against Postgres 18, a DEFAULT on the added parameter makes the incumbent 8-arity call AMBIGUOUS ("function is not unique") and therefore breaks search_graph_expand, the compiler''s emitted call and four test call sites at run time, on a migration declaring itself additive. 20260814000010 recorded that a new parameter makes a new function; this refines it — the new function is fine, the default is not. Both incumbent arities are re-pointed to delegate rather than left alone, for 20260808000030''s ONE BODY PER ARM: two walks that must agree and are linked by nothing drift silently because both keep returning plausible rows. Zero edge-owned properties exist in prod over a write path that shipped in 20260727000030, so the axis is correct-by-construction and witnessed by nothing unless its test CREATES one — which is why the witness writes an edge property and narrows on it rather than asserting over the corpus.'
);
