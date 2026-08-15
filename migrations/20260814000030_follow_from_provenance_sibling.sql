-- `follow-from`'s provenance-carrying sibling: the walk that says which seed each neighbour came
-- from, and via which edge.
--
-- Design: docs/superpowers/specs/2026-08-14-follow-from-mechanic-design.md. Nothing here is a design
-- question — §§2-6 are grounded file:line and their numbers were measured against prod. The four
-- questions that document left to the builder were ruled 2026-08-14 (Pete) and are recorded at their
-- own sections; the two that shape this file are named below.
--
-- ── THE TRAP THIS FILE EXISTS TO AVOID ──────────────────────────────────────────────────────────
--
-- `search_graph_expand` already carries provenance internally — `walk` builds `path uuid[]` and uses
-- it for cycle exclusion (20260711000030:41,45,53), so the parent of every node is already in hand.
-- The obvious move is to widen its RETURNS TABLE to project it. **That is a DROP and re-CREATE, it
-- passes every local test, and it fails at deploy on `main`**: a fresh test database applies the
-- whole chain and never evaluates additivity, while `vercel.json`'s buildCommand runs
-- `temper-migrate --additive-only`, which halts at the first shape-breaking migration and fails the
-- build — blocking every subsequent deploy on that target until an operator takes a cutover.
--
-- So the projection lands in a NEW function and the incumbent is re-pointed at it, unchanged in
-- shape. `[verified — 2026-08-14]` against the deployed schema: the CREATE OR REPLACE below applies
-- with `pg_get_function_identity_arguments` and `pg_get_function_result` byte-identical to
-- 20260711000030:23-25 afterwards. Proven before this file was written, not assumed by it.
--
-- ── THREE LEVELS, AND THE TOP ONE IS HERE FOR A DIFFERENT REASON THAN THE FIND FAMILY'S ─────────
--
--   search_graph_expand(p_principal, …)                      -- unchanged signature
--     -> query_follow_from(p_principal, …)                   -- composable, GATED
--       -> __temper_ungated_follow_from(p_visible_ids, …)    -- the real body. Applies NO gate.
--
-- In 20260808000030 the top level exists because `/api/search` must keep its shape. **Nothing
-- outside temper-substrate's tests calls `search_graph_expand`** (registry.rs:356,
-- 20260806000010's declaration), so there is no door to preserve here. The top level earns its place
-- on that migration's *other* rule: ONE BODY PER ARM. Left alone, the incumbent is a second walk
-- that must agree with this one and is linked to it by nothing — "two bodies drift, and the drift is
-- silent because both keep returning plausible rows". Re-pointed, its existing tests become
-- coverage of the real body.
--
-- The bottom level exists for 20260808000030's reason unchanged: a composition must compute
-- visibility ONCE. A CTE cannot be passed to a function, so the verdict is handed over as a VALUE.
-- The `__temper_ungated_` prefix is source discipline (audit-ungated-fragments.sh plus a single
-- compiler emitter), NOT a database permission — the app connects as the owning role.
--
-- ── TWO PARAMETERS THE ACT NEVER EXPOSES, AND THAT IS THE DESIGN ────────────────────────────────
--
-- `p_depth` and `p_gamma` are DEFINITIONAL, not caller inputs `[ruled — 2026-08-14, Pete]`.
--
--   * gamma, because `orders_by.means` is a fixed sentence describing what `graph_score` IS, and a
--     caller-set rate makes it "decayed at whatever rate you asked for" — still true, no longer
--     interpretable. Nothing would stop `gamma > 1`, which inverts the meaning (spec §2.2).
--   * depth, because "depth 3 is too large for a neighborhood traversal of this kind" is a claim
--     about what `follow-from` MEANS, which is gamma's category rather than `limit`'s. Fixed at 2.
--     `BoundTerm` does not grow a variant (spec §2.1, §5).
--
-- They are parameters HERE because the incumbent's signature has both slots and the re-point passes
-- them through. A caller-settable depth would return additively; nothing about this file forecloses
-- it.
--
-- ── p_bound_ids CONSTRAINS THE WHOLE WALK, INTERMEDIATES INCLUDED ───────────────────────────────
--
-- `[ruled — 2026-08-14, Pete]`, spec §9. This is the semantic half of the parameter and answering it
-- silently was the failure the surface was designed against, so it is written down beside the code.
--
-- Three things decide it:
--   1. The declaration had already said so — registry.rs:328-330 reads "walk from these seeds but
--      STAY INSIDE THIS SET".
--   2. The walk already has one set-shaped constraint behaving exactly this way: `adj` admits an
--      edge only when BOTH endpoints are visible (20260711000030:37-38). A bound that filtered only
--      the output would be the odd one out among the walk's own constraints.
--   3. An output-only bound IS `CombineOp::Intersect`, and a slot for it would be a second spelling
--      of a combinator — which is exactly why `find-resources-with` was given none (20260814000010).
--      Only the interior reading cannot be composed from outside the walk, and that is what earns it
--      a parameter.
--
-- **The observable difference, stated once:** seed -> B (not in bound) -> C (in bound) returns C
-- under the output-only reading and does NOT return it under this one.
--
-- Both set-shaped parameters are therefore applied in `admitted`, upstream of `adj`. There is
-- deliberately no post-filter on the final SELECT; adding one later would silently restore the
-- reading this ruling refused.
--
-- ── POLARITY OF THE THREE ID SETS, WHICH IS NOT UNIFORM AND MUST NOT BE MADE SO ─────────────────
--
--   p_visible_ids  NULL admits NOTHING      -- fail-closed. The verdict, handed in.
--   p_bound_ids    NULL is UNBOUNDED, '{}' returns zero rows
--   p_seed_ids     NULL/'{}' returns zero rows -- a walk from nowhere reaches nowhere
--
-- `p_bound_ids`' empty case is the one that matters: an upstream stage that produced nothing must
-- not silently widen into a global walk. Same rule as 20260808000030's.
--
-- **`unnest`, NEVER `= ANY`** for the id sets — `= ANY(uuid[])` establishes no equivalence class and
-- so cannot propagate a join condition (20260808000030's header, measured). The bound is an EXISTS
-- over `unnest` rather than a JOIN only because its NULL branch means "no restriction", which a JOIN
-- cannot express; it is still a semi-join over a function scan, never an array filter.
--
-- ── WHAT A `via` ENTRY IS ───────────────────────────────────────────────────────────────────────
--
-- One row per node, `via` an array, always on, CARRYING NO NUMBERS (spec §3). An entry names the
-- edge AS ASSERTED — `seed_id`, `source_id`, `target_id`, `edge_kind`, `label`, `polarity` — rather
-- than a `from_id` plus a direction flag, because two fields that must agree are two fields that can
-- drift, and the edge as stored cannot contradict itself (spec §4).
--
-- Source and target ride because the walk is UNDIRECTED over a DIRECTED graph: `adj` unions both
-- orientations, so a traversal runs with or against the stored arrow. Polarity rides because
-- `[measured on prod — 2026-08-14]` 1,099 of 4,454 resource-resource edges are `inverse`, and
-- `contains` — the most direction-sensitive kind — is inverse in 87% of cases. A `via` entry
-- omitting it would report the majority of containment relationships backwards (spec §4.1, §4.2).
--
-- **EVERY parent, not the winning path's.** `graph_score` is MAX over paths; the parent set is a
-- union over ALL paths. They disagree by construction and the non-correspondence is declared on the
-- field rather than repaired — a per-parent score is nearly free to compute and is refused, because
-- provenance is structure rather than quantity and a scored entry would be a second ranking axis
-- inside the row (spec §3.2).
--
-- **No cap, and the reason is structural rather than "small corpus"** `[measured on prod]`: worst
-- case (25 highest-degree seeds, depth 3, ungated) the whole walk holds 9,434 via tuples with 124 on
-- one node, while the page that ships carries 125 in total and at most 9 on any row. Nodes with huge
-- parent sets are reached by many long weak paths, so MAX(gamma^hop * weights) ranks them out and
-- the limit sheds them. The score and the limit bound the payload in the right direction — and
-- capping would silently truncate provenance, which is the one thing this field exists to be
-- trusted about (spec §5).
--
-- ── THE LABEL AXIS IS NEW, AND ITS NULL HANDLING IS REQUIRED THOUGH NO CALLER SEES ONE ──────────
--
-- The incumbent carries one of `EdgeFilter`'s two axes: `p_edge_types`, compared as
-- `e.edge_kind::text = ANY(...)` (20260711000030:35-36). **The label axis has never existed in the
-- walk** (spec §6). `kb_edges.label` is `TEXT` with no NOT NULL (20260624000001:636) and the repo's
-- convention for it is `COALESCE(label,'')` in the uniqueness index (:646);
-- `[measured on prod — 2026-08-14]` all 4,454 edges carry one, across 68 distinct values on forward
-- `leads_to` alone. So no caller sees a null today and the DDL still admits one: **`p_labels`
-- silently excludes unlabelled edges**, which is correct and is stated here rather than discovered
-- (spec §4.3).
--
-- Additive: two CREATE FUNCTION and one CREATE OR REPLACE at a byte-identical signature. No DROP.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

-- ── 1. The ungated core ─────────────────────────────────────────────────────────────────────────
CREATE FUNCTION __temper_ungated_follow_from(
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
  WITH RECURSIVE
  -- Both set-shaped constraints, in ONE place and upstream of the traversal. This is what makes the
  -- bound constrain intermediates rather than the returned set — see the header.
  admitted AS (
    SELECT v.id
      FROM unnest(p_visible_ids) AS v(id)
     WHERE p_bound_ids IS NULL
        OR EXISTS (SELECT 1 FROM unnest(p_bound_ids) AS b(id) WHERE b.id = v.id)
  ),
  -- Undirected adjacency over admitted, unfolded, kb_resources edges. The edge's OWN source/target
  -- are carried through unchanged so a `via` entry can name it as asserted; the traversal direction
  -- is a property of the walk and is deliberately not recorded as a second field.
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
       -- Excludes unlabelled edges by construction when supplied. See the header.
       AND (p_labels IS NULL OR array_length(p_labels, 1) IS NULL
            OR e.label = ANY(p_labels))
  ),
  -- `seed_id` rides the whole walk so a node three hops out still knows which seed it descends from
  -- — the act's own `means` says "from any seed", and the act takes a seed SET.
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
  -- `hop > 0` and the MAX are the act's declared quantity and do not move: a seed does not score
  -- itself 1.0 (issue #357), and a seed reached back via a >=1-hop cycle keeps that path's score.
  -- The limit is applied HERE, before provenance is described — which is what bounds the payload.
  ranked AS (
    SELECT w.node, MAX(w.score)::real AS graph_score
      FROM walk w
     WHERE w.hop > 0
     GROUP BY w.node
     ORDER BY MAX(w.score) DESC, w.node
     LIMIT p_limit
  )
  SELECT r.node, r.graph_score,
         -- Every parent, deduplicated at the EDGE grain, never at the path grain: a node reached by
         -- one edge along five different paths names that edge once.
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

-- ── 2. The gated wrapper ────────────────────────────────────────────────────────────────────────
--
-- No `CASE` guard, for `query_find_resources_with`'s reason (20260814000010): that guard exists on
-- `query_find_exact` because a text-only `/api/search` calls the wide arm with a guaranteed-empty
-- input, and this act has no such input — a walk with no seeds is a caller error, not a routine
-- call shape, and it costs nothing to answer.
CREATE FUNCTION query_follow_from(
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
      FROM __temper_ungated_follow_from(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit) c;
$$;

-- ── 3. The incumbent, re-pointed ────────────────────────────────────────────────────────────────
--
-- Signature and return type byte-identical to 20260711000030:23-25 — verified against the deployed
-- schema before this was written. It passes its `p_edge_types` into the new `p_edge_kinds` slot and
-- supplies NULL for the two axes it never had (labels, bound) and for the limit it never applied.
-- It projects two of the core's three columns; a caller of this signature asked for a walk without
-- provenance and gets exactly that.
--
-- **IT PAYS FOR `via` AND THROWS IT AWAY, and that is accepted rather than unnoticed.**
-- `[measured — 2026-08-14]` the planner does not prune the discarded column through the recursive
-- CTE: 314.8 ms projecting two columns against 321.5 ms projecting three, where the same walk
-- without the `via` subquery at all runs in 262.9 ms. So this signature carries the full +20%
-- surcharge for a column its callers cannot see.
--
-- Free today because its only callers are temper-substrate's own tests — which is a fact about
-- today's callers, not about the function. The first real caller inherits the surcharge. The
-- alternative was letting the incumbent keep its own body, which is the drift ONE BODY PER ARM
-- exists to prevent; this is the cheaper side of that trade, stated so it is not later mistaken for
-- an oversight.
CREATE OR REPLACE FUNCTION search_graph_expand(
  p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision)
RETURNS TABLE (resource_id uuid, graph_score real)
LANGUAGE sql STABLE AS $$
  SELECT c.resource_id, c.graph_score
    FROM query_follow_from(p_principal, p_seed_ids, p_depth, p_gamma,
                           p_edge_types, NULL, NULL, NULL) c;
$$;

COMMENT ON FUNCTION __temper_ungated_follow_from(
        uuid[], uuid[], int, double precision, text[], text[], uuid[], int) IS
    'Provenance-carrying core for the follow-from act: an undirected walk over unfolded kb_resources edges returning one row per reached node with MAX(gamma^hop * product of edge weights) and a `via` array naming every edge it was reached by. A via entry names the edge AS ASSERTED — seed_id, source_id, target_id, edge_kind, label, polarity — never a from_id plus a direction flag, because two fields that must agree can drift and the walk is undirected over a directed graph. via carries NO numbers: provenance is structure rather than quantity, and a per-parent score would be a second ranking axis inside the row. It names EVERY parent while graph_score is the MAX over one path; the two disagree by construction and the non-correspondence is declared rather than repaired. There is no cap: nodes with huge parent sets are reached by many long weak paths, so the score ranks them out and the limit sheds them before they are described — capping would silently truncate the one field that exists to be trusted. Takes its visibility verdict as p_visible_ids (NULL admits NOTHING) rather than computing it, so a multi-stage /api/query composition pays resources_visible_to once; the __temper_ungated_ prefix is source discipline enforced by audit-ungated-fragments.sh and a single compiler emitter, NOT a database permission. p_bound_ids has the OPPOSITE polarity — NULL is unbounded, {} returns zero rows — and constrains THE WHOLE WALK including intermediate nodes, applied in `admitted` upstream of `adj` exactly where visibility is applied: an output-only bound would be CombineOp::Intersect and a slot for it would be a second spelling of a combinator, so only the interior reading earns a parameter. p_depth and p_gamma are definitional rather than caller inputs (the act fixes depth at 2 and gamma at the rate its orders_by sentence describes); they are parameters here because the incumbent search_graph_expand signature has both slots and delegates through them. p_labels is a new axis the walk never had and silently excludes unlabelled edges, which kb_edges.label admits and prod does not currently contain.';

COMMENT ON FUNCTION query_follow_from(
        uuid, uuid[], int, double precision, text[], text[], uuid[], int) IS
    'Gated wrapper over __temper_ungated_follow_from: computes resources_visible_to(p_principal) once and hands it down. Carries no CASE guard because this act has no guaranteed-empty input, unlike query_find_exact''s text-only call into the wide arm. Serves the follow-from act, which produces resources ordered by graph_score and discloses input contribution via the `via` column — the disclosure Disclosure::InputContribution was retired against and returns for.';

SELECT declare_migration(
    20260814000030,
    'additive',
    'Gives the follow-from act a provenance-carrying mechanic (task 01a001b0-10b9-7752-8d7a-770df8dcdb8c), design docs/superpowers/specs/2026-08-14-follow-from-mechanic-design.md: __temper_ungated_follow_from plus the gated wrapper query_follow_from, both new, and search_graph_expand re-pointed at them by CREATE OR REPLACE at a byte-identical signature and return type. TWO CREATE FUNCTION and ONE CREATE OR REPLACE, no DROP and no signature change — which is the whole point rather than an accident: search_graph_expand already carries the parent of every node in its `path` array, so widening its RETURNS TABLE to project provenance is the obvious move, and it is a DROP and re-CREATE that passes every local test (a fresh test database applies the chain and never evaluates additivity) and then halts --additive-only at deploy, blocking every subsequent deploy on that target until an operator takes a cutover. The incumbent is re-pointed rather than left alone because 20260808000030''s ONE BODY PER ARM rule applies even though no door depends on this one: two walks that must agree and are linked by nothing will drift silently, both still returning plausible rows. via is one row per node carrying an array of edges AS ASSERTED (seed_id, source_id, target_id, edge_kind, label, polarity) with no numbers, no cap, and every parent rather than the winning path''s — measured on prod, the page that ships carries 125 via tuples in total against 9,434 in the whole depth-3 walk, because the score and the limit shed exactly the rows that would be expensive to describe. p_bound_ids constrains the whole walk INCLUDING INTERMEDIATE NODES, applied in `admitted` upstream of `adj` where visibility is already applied; the output-only reading was refused because it is CombineOp::Intersect and would be a second spelling of a combinator, and because registry.rs already said "walk from these seeds but stay inside this set". p_depth and p_gamma are definitional rather than caller inputs and are parameters only because the incumbent signature has both slots. p_labels is EdgeFilter''s second axis, which the walk has never had, and it excludes unlabelled edges by construction.'
);
