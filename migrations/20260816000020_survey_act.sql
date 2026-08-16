-- ── WHAT ───────────────────────────────────────────────────────────────────────────────────────
--
-- Gives the `survey` act a door. The mechanic (`wayfind_region_scores`) has been live since
-- 20260731000050; the act has been `DoorReach::Absent` at all three doors because its fragment
-- takes a `p_lens` no slot supplies. The design (01a00c0b-200c) settled that `p_lens = NULL` is
-- correct, not just a default: the lens is a clustering-time parameter, and `NULL` at query time
-- reads the baked salience. So survey ships with `p_lens = NULL` and a declared hole for the
-- future re-lensing capability.
--
-- The ratified ⟨3⟩ redesign (01a0003f): survey produces the member RESOURCES of matched regions,
-- not the regions themselves. Regions become trace disclosure. Within-region ranking by
-- `query_cos` (the resource's own embedding similarity to the query), which is an existing declared
-- quantity reused at a finer grain — anchor-agnostic, and it does not interact with the sal_norm
-- open ruling.
--
-- Survey REQUIRES an intention (a query text + embedding), same as the find acts. Without a query,
-- survey collapses into `cogmap_read(shape)` / `context_read(shape)`, which already serve pure
-- orientation. Survey's distinct value is query-relevance within a scope's region structure: "what
-- does this scope know *about X*?" The shape-pass validator refuses a survey stage with no
-- intention as `MissingIntention`; the compiler refuses a survey stage whose embedding could not
-- be obtained as `EmbeddingUnavailable`. So `p_emb` is never NULL in production — the DEFAULT NULL
-- is a testing convenience for direct `psql` calls only.
--
-- ── THE TWO VISIBILITY GATES ────────────────────────────────────────────────────────────────────
--
-- Survey has TWO visibility gates, and they are different:
--
--   1. REGION visibility — `wayfind_region_scores(p_principal, ...)` calls
--      `visible_region_anchors(p_principal)` internally. This gates which regions the principal
--      may see. It takes `p_principal`, not a visible-ids set.
--
--   2. RESOURCE visibility — the member join (`kb_cogmap_region_members`) must be filtered by
--      `resources_visible_to(p_principal)` so a visible region does not leak an invisible member
--      resource. This is the gate the composition's hoisted CTE (`__temper_vis`) exists to share.
--
-- The ungated core takes BOTH: `p_visible_ids` for the resource gate (same as every other ungated
-- core — the hoisted CTE), and `p_principal` for the region gate (which `wayfind_region_scores`
-- applies internally). This is a different shape from the other ungated cores, which take only
-- `p_visible_ids` — but survey is the only act that delegates to a function with its own internal
-- visibility. The `p_principal` is the compiler's `$1` (always bound first), and the
-- `audit-ungated-fragments.sh` guard's invariant — "every ungated fragment is handed the RBAC
-- verdict as `p_visible_ids`" — holds for the resource gate. The region gate is inside
-- `wayfind_region_scores`, which is not an ungated function.
--
-- ── CLASS ───────────────────────────────────────────────────────────────────────────────────────
--
-- `additive`. Two new CREATE FUNCTION, no DROP, no signature change to any existing function.
-- `wayfind_region_scores` is called unchanged; this wraps it.
--
-- ── WHY RESOURCES OUT, NOT REGIONS ──────────────────────────────────────────────────────────────
--
-- The declaration said `produces: Region` and the funnel was built around that: `find-exact`
-- declines `bounds: [region]`, so survey was "the only act that PRODUCES a kind no reachable
-- downstream act accepts" (validate/mod.rs:853). That was a deliberate funnel — and it was also
-- a dead end: survey→find-exact could not compose because find-exact does not accept regions.
--
-- The redesign produces resources, so survey→find-exact becomes a resource→resource pipe —
-- "survey a cogmap, then find-exact within the resources it surfaced" — which is the natural
-- agent flow. Regions move into `StageTrace` as disclosure (which regions matched, their scores).
--
-- ── THE WITHIN-REGION RANKING ───────────────────────────────────────────────────────────────────
--
-- Resources within a matched region are ranked by `query_cos` — the RESOURCE's own embedding
-- similarity to the query (`1 - (embedding <=> p_emb)`), not the region's centroid similarity.
-- That is a different `query_cos` from the region-scoring one (which uses the centroid), at a
-- finer grain, and it is the signal that makes the output answer "what does this scope know about
-- X" rather than "what is this region about."
--
-- `query_cos` is already a declared quantity in the `region_score` blend; the within-region reuse
-- is the same signal at a finer grain, not a new scoring quantity. The `orders_by` declaration
-- carries both clauses: regions ranked by `region_score`, resources within a region ranked by
-- `query_cos`.

CREATE FUNCTION __temper_ungated_survey(
    p_visible_ids    uuid[],
    p_principal      uuid,
    p_emb            vector DEFAULT NULL,
    p_regions_n      int    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL)
RETURNS TABLE (
    resource_id      uuid,
    region_id        uuid,
    region_score     double precision,
    query_cos        double precision,
    affinity         double precision)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 3 AS default_n, 20 AS max_n),
  n AS (SELECT GREATEST(LEAST(COALESCE(p_regions_n, (SELECT default_n FROM k)),
                              (SELECT max_n FROM k)), 1) AS regions_n),
  -- The matched regions: wayfind_region_scores with p_lens = NULL (baked salience).
  -- p_principal drives region visibility INSIDE wayfind (via visible_region_anchors).
  regions AS (
    SELECT s.region_id, s.region_score
      FROM wayfind_region_scores(
             p_principal,
             NULL::uuid,           -- p_lens — definitional NULL (the baked salience)
             p_emb,
             (SELECT regions_n FROM n),
             p_anchor_table,
             p_anchor_id) s
  )
  SELECT rm.member_id            AS resource_id,
         r.region_id,
         r.region_score,
         CASE WHEN p_emb IS NULL THEN 0.0
              ELSE COALESCE(NULLIF(1 - (ch.embedding <=> p_emb), 'NaN'::float8), 0.0)
         END                     AS query_cos,
         rm.affinity
    FROM regions r
    JOIN kb_cogmap_region_members rm
      ON rm.region_id = r.region_id
     AND rm.member_table = 'kb_resources'
    JOIN LATERAL (
      SELECT ch.embedding
        FROM kb_chunks ch
       WHERE ch.home_resource_id = rm.member_id
       ORDER BY ch.chunk_ordinal
       LIMIT 1
    ) ch ON true
   WHERE rm.member_id = ANY(p_visible_ids)
  ORDER BY r.region_score DESC NULLS LAST,
           query_cos DESC NULLS LAST,
           rm.member_id;
$$;

-- The gated wrapper. Computes resources_visible_to(p_principal) once and hands it down.
-- wayfind_region_scores applies its own region visibility internally; the visible-ids set filters
-- the member resources.
CREATE FUNCTION query_survey(
    p_principal      uuid,
    p_emb            vector DEFAULT NULL,
    p_regions_n      int    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL)
RETURNS TABLE (
    resource_id      uuid,
    region_id        uuid,
    region_score     double precision,
    query_cos        double precision,
    affinity         double precision)
LANGUAGE sql STABLE AS $$
  SELECT s.resource_id, s.region_id, s.region_score, s.query_cos, s.affinity
    FROM __temper_ungated_survey(
           ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
           p_principal,
           p_emb, p_regions_n, p_anchor_table, p_anchor_id) s;
$$;

COMMENT ON FUNCTION __temper_ungated_survey(uuid[], uuid, vector, int, varchar, uuid) IS
    'Ungated core for the survey act. Takes p_visible_ids (the hoisted resources_visible_to verdict, for the resource-member gate) and p_principal (for wayfind_region_scores'' internal region-visibility gate via visible_region_anchors). Calls wayfind_region_scores with p_lens = NULL (the lens is a clustering-time parameter; NULL reads the baked salience) and joins matched regions to their member resources via kb_cogmap_region_members. Returns resources with their region_id (disclosure), region_score (inherited from the region), query_cos (the resource''s own embedding similarity to the query, not the region''s centroid similarity), and affinity (how core the member is to its region — a disclosed quantity, not a ranking signal). Ordered by region_score DESC then query_cos DESC. The __temper_ungated_ prefix is source discipline enforced by audit-ungated-fragments.sh, NOT a database permission. p_emb is never NULL in production (the validator refuses a survey stage with no intention; the compiler refuses one whose embedding could not be obtained); the DEFAULT NULL is a testing convenience for direct psql calls only.';

COMMENT ON FUNCTION query_survey(uuid, vector, int, varchar, uuid) IS
    'Gated wrapper over __temper_ungated_survey: computes resources_visible_to(p_principal) once and hands it down as the visible resource set. Serves the survey act, which produces resources ordered by region_score (the region''s blended score) then within-region query_cos, and discloses the region each resource came from. p_lens is definitional NULL — passed by the ungated core, not a caller slot. The lens selector at query time is a declared hole: re-lensing regions under a different telos at read time is a future capability with no use case today. Survey requires an intention (query + embedding); without one it collapses into cogmap_read(shape)/context_read(shape), which already serve pure orientation.';

SELECT declare_migration(
    20260816000020,
    'additive',
    'Gives the survey act a door (task 01a00c0b-9a02, design 01a00c0b-200c). Two new functions: __temper_ungated_survey (the ungated core) and query_survey (the gated wrapper). Calls wayfind_region_scores with p_lens = NULL (the lens is a clustering-time parameter; NULL reads the baked salience) and joins matched regions to member resources via kb_cogmap_region_members. The ratified redesign: survey produces RESOURCES, not regions; regions become trace disclosure. Within-region ranking by the resource''s own embedding similarity to the query (query_cos). Survey requires an intention — without a query it collapses into cogmap_read(shape), which already exists. Survey has two visibility gates: region visibility (inside wayfind_region_scores, by principal) and resource visibility (the member join, by the hoisted visible-ids set). The ungated core takes both p_visible_ids and p_principal — a different shape from the other ungated cores, which take only p_visible_ids, because survey is the only act that delegates to a function with its own internal visibility. No DROP, no signature change to wayfind_region_scores or any existing function.'
);