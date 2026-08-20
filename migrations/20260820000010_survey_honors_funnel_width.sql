-- survey honors its funnel width — `__temper_ungated_survey` filters `in_top_n`.
--
-- Task 01a01fe6-4b96-7bb0-ac98-18dc2a8f33be. Found from the graph successor surface (Beat B) by
-- POSTing the real composition builder's output at the deployed `/api/query` rather than at a
-- fixture — the first time any caller had asked survey for a specific funnel width and then looked
-- at what came back.
--
-- ── THE BUG ─────────────────────────────────────────────────────────────────────────────────────
--
-- `wayfind_region_scores` (20260731000050) does NOT limit. It returns **one row per CANDIDATE
-- region** carrying `in_top_n boolean` — *"did this region clear the Stage-1 top-N cut this query
-- would apply?"* — and leaves the cut to its consumers. `wayfind_scope_ids` filters it
-- (20260731000050:190, again at 20260731000060:83). `wayfind_region_diagnostics` deliberately does
-- not, because reporting every candidate is the whole point of diagnostics.
--
-- `__temper_ungated_survey` (20260816000020:94-103) also did not — and that one is not deliberate.
-- It joined EVERY candidate region to its members, so `p_regions_n` reached the scoring function,
-- set a flag, and changed nothing. Witnessed on prod (deployed commit b1e9b9cc), one survey bound to
-- a 406-region cogmap, three widths:
--
--     regions asked=1    terms_applied={regions: 1}    disclosed=406    rows=731
--     regions asked=3    terms_applied={regions: 3}    disclosed=406    rows=731
--     regions asked=20   terms_applied={regions: 20}   disclosed=406    rows=731
--
-- Identical. Every anchor disclosed exactly its full region count — 406/406, 32/32, 12/12 — which is
-- what made the defect visible at all.
--
-- ── WHY IT IS WORSE THAN AN UNBOUNDED READ ──────────────────────────────────────────────────────
--
-- The answer was never garbage: survey orders `region_score DESC, query_cos DESC`, so the
-- best-matching regions still lead. The damage is that the read **reported itself as bounded when it
-- was not**. `terms_applied` is the field whose own contract is *"the APPLIED value … the page this
-- stage actually RAN with"*, and whose doc forbids exactly this: *"reporting the request back would
-- make terms_applied an echo rather than a disclosure."* With the cut never applied it WAS the echo —
-- a false disclosure rather than a missing one, which no caller could detect from the response.
--
-- ── THE FIX ─────────────────────────────────────────────────────────────────────────────────────
--
-- One predicate. Same signature, same columns, same ordering, same visibility gates. Body is
-- 20260816000020's verbatim, plus `WHERE s.in_top_n` on the `regions` CTE.
--
-- `regions_n` is still passed down, because it is what `wayfind_region_scores` computes the cut FROM;
-- the SQL clamp (default 3, max 20) is unchanged and stays where it is. This migration does not touch
-- the funnel default, the ceiling, the blend, or the round-robin — only whether survey honors the cut
-- that was already computed for it.

CREATE OR REPLACE FUNCTION __temper_ungated_survey(
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
  --
  -- `WHERE s.in_top_n` is the cut. wayfind_region_scores scores every CANDIDATE and flags the
  -- winners rather than returning only them, so a consumer that does not filter reads the whole
  -- candidate pool — which is what this function did until 20260820000010, at every funnel width.
  regions AS (
    SELECT s.region_id, s.region_score
      FROM wayfind_region_scores(
             p_principal,
             NULL::uuid,           -- p_lens — definitional NULL (the baked salience)
             p_emb,
             (SELECT regions_n FROM n),
             p_anchor_table,
             p_anchor_id) s
     WHERE s.in_top_n
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
       WHERE ch.resource_id = rm.member_id
         AND ch.is_current
       ORDER BY ch.chunk_index
       LIMIT 1
    ) ch ON true
   WHERE rm.member_id = ANY(p_visible_ids)
  ORDER BY r.region_score DESC NULLS LAST,
           query_cos DESC NULLS LAST,
           rm.member_id;
$$;

COMMENT ON FUNCTION __temper_ungated_survey(uuid[], uuid, vector, int, varchar, uuid) IS
    'Ungated core for the survey act. Takes p_visible_ids (the hoisted resources_visible_to verdict, for the resource-member gate) and p_principal (for wayfind_region_scores'' internal region-visibility gate via visible_region_anchors). Calls wayfind_region_scores with p_lens = NULL (the lens is a clustering-time parameter; NULL reads the baked salience), keeps only the regions that cleared its per-map round-robin cut (in_top_n — the funnel width, honored since 20260820000010; before that every candidate region was joined and p_regions_n changed nothing), and joins them to their member resources via kb_cogmap_region_members. Returns resources with their region_id (disclosure), region_score (inherited from the region), query_cos (the resource''s own embedding similarity to the query, not the region''s centroid similarity), and affinity (how core the member is to its region — a disclosed quantity, not a ranking signal). Ordered by region_score DESC then query_cos DESC. The __temper_ungated_ prefix is source discipline enforced by audit-ungated-fragments.sh, NOT a database permission. p_emb is never NULL in production (the validator refuses a survey stage with no intention; the compiler refuses one whose embedding could not be obtained); the DEFAULT NULL is a testing convenience for direct psql calls only.';

-- Same signature, same columns, same ordering, same gates: a binary that does not carry this
-- migration calls the function identically and simply receives the bounded answer it always asked
-- for. Nothing dropped, no shape altered.
SELECT declare_migration(
    20260820000010,
    'additive',
    'survey honors its funnel width (task 01a01fe6-4b96-7bb0-ac98-18dc2a8f33be). __temper_ungated_survey now filters wayfind_region_scores on in_top_n, the flag that function sets for the winners of its per-map round-robin cut. Until now survey joined every CANDIDATE region to its members, so p_regions_n set a flag nobody read: witnessed on prod at widths 1, 3 and 20 returning identical 406-region, 731-row answers. terms_applied consequently reported a clamp that never applied — a false disclosure rather than a missing one. Same signature, columns, ordering and visibility gates; the funnel default (3), ceiling (20), blend and round-robin are untouched.'
);
