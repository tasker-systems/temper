-- Instrument the wayfind region-selection stage (issue #585, goal
-- 019fb559-7191-75a3-99d4-879090c60e94).
--
-- `wayfind_scope_ids` (20260712000090) is a TWO-STAGE funnel: Stage 1 picks the global top-N regions
-- by `region_score`, Stage 2 (`unified_search`) ranks resources WITHIN that scope. The bug in #585 —
-- one sibling cogmap sweeping every scope slot — lives entirely in Stage 1, yet Stage 1 discards every
-- signal it computes: `wayfind_scope_ids` returns `SETOF uuid` (member resource ids), so the per-region
-- `sal_norm` / `query_cos` / `region_score` and each region's HOME MAP are unobservable. The per-hit
-- scores an agent CAN see (`UnifiedSearchResultRow.{fts,vector,graph}`) come from Stage 2 and are blind
-- to the stage where the competition happens: a map whose regions never clear the Stage-1 cut has no
-- members in the scope and therefore no per-hit scores at all.
--
-- This function exposes exactly that discarded competition, keyed by map, WITHOUT touching the hot
-- path. It is READ-ONLY and witnesses nothing itself — it is the before/after measurement surface for
-- the per-map-fairness fix (Task 2), in the spirit of T7's own on-prod sweep (20260712000090 header).
--
-- ============================================================================================
-- IT IS A FAITHFUL MIRROR OF `wayfind_scope_ids`' STAGE-1 SCORING. The k/n/vanchors/lens/cand/scored/
-- ranked CTEs below are copied from 20260712000090 verbatim except for threading the extra columns the
-- diagnostics need (home_anchor_id, and sal_norm/query_cos/home_anchor_table all the way to the final
-- projection). The tuning constants (α/β/κ, the priors, default/max N, thin threshold, recall floor)
-- and the blend MUST stay in lockstep with `wayfind_scope_ids`, or this stops measuring the real
-- selection. `wayfind_region_diagnostics_matches_scope_ids` (tests/cogmap_wayfind_scope.rs) pins that
-- equivalence — the diagnostics' `in_top_n` region set, expanded to visible members, must equal
-- `wayfind_scope_ids`' output — so drift fails CI. When Task 2 changes the ranking, change BOTH
-- functions together (or, better, collapse `wayfind_scope_ids` onto this one so the scoring has a single
-- home; that refactor is Task 2's call, not this instrumentation's).
-- ============================================================================================
--
-- What it does NOT reproduce: the member-dereference (`region_ids`) and the cold-start direct-scope
-- arm (`thin_anchors`/`direct_ids`). Those are Stage-1's SCOPE-ASSEMBLY, not its region SCORING; a
-- region-less anchor forms no candidate region and so is correctly absent here (it contributes to the
-- scope through cold-start, but there is no region competition to report for it). The equivalence test
-- runs a no-cold-start fixture precisely so the two functions' outputs coincide exactly.

CREATE FUNCTION wayfind_region_diagnostics(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS TABLE(
    region_id         uuid,
    home_anchor_table varchar,
    home_anchor_id    uuid,
    home_anchor_name  text,      -- resolved cogmap/context display name, so a sweep reads by map not UUID
    sal_norm          float8,
    query_cos         float8,
    region_score      float8,
    in_top_n          boolean)   -- did this region clear the Stage-1 top-N cut this query would apply?
LANGUAGE sql STABLE AS $$
  WITH
  -- --- MIRROR START: k/n/vanchors/lens/cand/scored/ranked are `wayfind_scope_ids`' CTEs verbatim,
  -- --- plus the threaded home_anchor_id/home_anchor_table/sal_norm/query_cos columns. See header.
  k AS (SELECT 0.4::float8  AS alpha,          -- salience weight
               0.6::float8  AS beta,           -- query-cosine weight (β≥α so relevance can buy a slot)
               0.05::float8 AS kappa,          -- anchor-kind prior (MEASURED — see 20260712000090)
               1.0::float8  AS prior_cogmap,   -- distilled
               0.6::float8  AS prior_context,  -- raw
               3   AS default_n,               -- default --regions
               20  AS max_n,                   -- per-call ceiling
               0   AS thin_threshold,          -- region-count <= this ⇒ region-less (unused here; see header)
               false AS recall_floor),         -- always admit best-cosine region (default OFF)
  n AS (SELECT GREATEST(LEAST(COALESCE(p_regions_n, (SELECT default_n FROM k)),
                              (SELECT max_n FROM k)), 1) AS regions_n),
  vanchors AS (
    SELECT a.anchor_table, a.anchor_id
    FROM visible_region_anchors(p_principal) a
    WHERE (p_anchor_table IS NULL OR a.anchor_table = p_anchor_table)
      AND (p_anchor_id    IS NULL OR a.anchor_id    = p_anchor_id)
  ),
  lens AS (SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses WHERE id = p_lens),
  cand AS (
    SELECT r.id, r.centroid, r.home_anchor_table, r.home_anchor_id,
           CASE WHEN p_lens IS NULL THEN r.salience
                ELSE (SELECT s_telos FROM lens)   * COALESCE(r.telos_alignment, 0)
                   + (SELECT s_ref FROM lens)     * COALESCE(r.reference_standing, 0)
                   + (SELECT s_central FROM lens) * COALESCE(r.centrality, 0)
           END AS sal_eff
    FROM kb_cogmap_regions r
    JOIN vanchors v
      ON v.anchor_table = r.home_anchor_table AND v.anchor_id = r.home_anchor_id
    WHERE NOT r.is_folded
  ),
  scored AS (
    SELECT c.id, c.home_anchor_table, c.home_anchor_id,
           CASE WHEN min(c.sal_eff) OVER kind = max(c.sal_eff) OVER kind THEN 1.0
                ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
           END AS sal_norm,
           CASE WHEN p_emb IS NULL THEN 0.0
                ELSE COALESCE(NULLIF(1 - (c.centroid <=> p_emb), 'NaN'::float8), 0.0)
           END AS query_cos
    FROM cand c
    WINDOW kind AS (PARTITION BY c.home_anchor_table)
  ),
  ranked AS (
    SELECT id, home_anchor_table, home_anchor_id, sal_norm, query_cos,
           (SELECT alpha FROM k) * sal_norm
         + (SELECT beta  FROM k) * query_cos
         + (SELECT kappa FROM k) * CASE home_anchor_table
                                     WHEN 'kb_cogmaps' THEN (SELECT prior_cogmap  FROM k)
                                     ELSE                   (SELECT prior_context FROM k)
                                   END AS region_score
    FROM scored
  ),
  -- --- MIRROR END. `top_regions` is `wayfind_scope_ids`' selection verbatim; the difference is that
  -- --- here we KEEP every ranked region and merely FLAG membership, rather than filtering to it.
  top_regions AS (
    (SELECT id FROM ranked ORDER BY region_score DESC NULLS LAST LIMIT (SELECT regions_n FROM n))
    UNION
    (SELECT id FROM ranked WHERE (SELECT recall_floor FROM k) ORDER BY query_cos DESC NULLS LAST LIMIT 1)
  )
  SELECT r.id AS region_id,
         r.home_anchor_table,
         r.home_anchor_id,
         CASE r.home_anchor_table
           WHEN 'kb_cogmaps'  THEN (SELECT name FROM kb_cogmaps  WHERE id = r.home_anchor_id)
           WHEN 'kb_contexts' THEN (SELECT name FROM kb_contexts WHERE id = r.home_anchor_id)
         END AS home_anchor_name,
         r.sal_norm,
         r.query_cos,
         r.region_score,
         (r.id IN (SELECT id FROM top_regions)) AS in_top_n
  FROM ranked r
  -- Winners first, so a sweep reads top-down; the tiebreak keeps output deterministic across calls.
  ORDER BY r.region_score DESC NULLS LAST, r.id;
$$;

COMMENT ON FUNCTION wayfind_region_diagnostics(uuid, uuid, vector, int, varchar, uuid) IS
    'READ-ONLY instrumentation of wayfind Stage-1 region selection (issue #585). Returns one row per '
    'CANDIDATE region across the principal''s visible anchors — home map, per-kind sal_norm, query_cos, '
    'region_score, and whether it cleared the top-N cut — so the cross-map competition wayfind_scope_ids '
    'discards is observable, keyed by map. A faithful mirror of wayfind_scope_ids'' scoring: the tuning '
    'constants and blend must stay in lockstep (equivalence pinned by tests/cogmap_wayfind_scope.rs). '
    'Same visibility gate (visible_region_anchors) and same signature as wayfind_scope_ids; deny ⇒ zero '
    'rows, never an error. Does not report cold-start region-less anchors — they form no candidate '
    'region to score.';
