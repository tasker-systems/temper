-- Per-map fairness in wayfind Stage-1 region selection (issue #585, goal
-- 019fb559-7191-75a3-99d4-879090c60e94; Task 2 of the cross-map-wayfind goal).
--
-- THE BUG (#585, confirmed on prod by the Task-1 diagnostics). `wayfind_scope_ids`
-- (20260712000090) picks the Stage-1 scope with a plain global
-- `ORDER BY region_score DESC LIMIT regions_n` and NO per-map floor. A map with many regions
-- clustered near the top of the pooled rank hoards every slot: the reporter's richest cogmap took
-- 12 / 12 slots with best score 0.45, while a sibling map's best region scored 0.4053 and got ZERO —
-- competitive, but volume-crowded out. Its members never reach Stage-2 (`unified_search`), so no
-- amount of Stage-2 relevance can surface them: they are not in the candidate scope at all.
--
-- T7 (20260712000090, header note 2) fixed exactly this drowning between anchor *kinds*
-- (cogmap vs context) with per-KIND `percent_rank` + an anchor-kind prior κ. The identical drowning
-- recurs one level down, on the map-vs-map axis WITHIN the cogmap kind, where nothing normalizes or
-- reserves representation. This migration extends the medicine to the map axis.
--
-- THE FIX — per-map round-robin (rank-major) selection. Instead of taking the global top-N by score,
-- rank each map's candidate regions within the map (`map_rank` = row_number by score, per home anchor),
-- then fill the N slots in ROUNDS: round 1 admits every map's #1 region (ordered by score), round 2
-- admits every map's #2 region, and so on until the N slots fill. A single map can therefore take at
-- most one slot until every other visible map has taken one — no map can monopolize while a sibling's
-- competitive champion sits below the cut. This is textbook result-diversification and adds NO new
-- tuning constant (the register's stated silence forbids committing to one); κ still orders regions
-- WITHIN each round, so distilled cogmap content still leads raw context content — the cross-kind tilt
-- T7 established is preserved, not re-broken (`cross-kind-fairness-preserved`).
--
-- What round-robin trades: when the scope is narrower than the visible-map count (regions_n < #maps),
-- a single squarely-matching map yields depth in the scope to breadth across maps — the register's
-- headline priority ("every relevant map is reachable ... not confined to one map"), and Stage-2
-- re-ranking is the relevance backstop for the members that do enter scope. A map authored so its
-- content genuinely does not answer a query is not *owed* a slot, but including its champion when a
-- slot is free costs only scope width (Stage-2 filters it), never a shut-out of a relevant sibling.
--
-- ============================================================================================
-- SINGLE SCORING HOME. Task 1 (20260731000030) shipped `wayfind_region_diagnostics` as a *mirror* of
-- `wayfind_scope_ids`' Stage-1 scoring, pinned together by an equivalence test because duplicated
-- scoring is a drift hazard — and it left the collapse to Task 2. This migration performs it: the
-- scoring CTEs and the top-N selection now live ONCE, in `wayfind_region_scores`, and BOTH consumers
-- read from it. `wayfind_scope_ids` becomes scope ASSEMBLY only (member-deref + cold-start);
-- `wayfind_region_diagnostics` becomes the same rows plus resolved map names. The equivalence the
-- Task-1 test asserts is now true BY CONSTRUCTION — there is no second copy of the blend to drift.
-- ============================================================================================
--
-- Deploy safety: purely additive. One new function (`wayfind_region_scores`) and two CREATE OR REPLACE
-- of existing functions with UNCHANGED signatures — no shape break, no drop, and the round-robin only
-- reorders which regions a read selects. Safe for a binary that does not carry it (the readback
-- signatures are unchanged, so an old binary calls the new functions identically). `additive` per the
-- schema/binary pairing design.

-- ---------------------------------------------------------------------------
-- 1. wayfind_region_scores — the ONE home for Stage-1 region scoring + top-N selection.
--
-- Returns EVERY candidate region across the principal's visible anchors, each with its per-kind
-- sal_norm, NaN-guarded query_cos, the composite region_score, and `in_top_n` — whether it cleared the
-- per-map round-robin cut for this query. The k/n/vanchors/lens/cand/scored/ranked CTEs are T7's
-- verbatim (the tuning constants and the per-KIND percent_rank are unchanged); the NEW step is
-- `ranked_rr`'s per-MAP rank and the round-robin `top_regions` ordering.
--
-- Same six-arg signature as its two consumers, so they thread their inputs through unchanged. No name
-- resolution here (the hot scope path must not pay for display strings) — the diagnostics wrapper adds
-- those. Visibility-gated via `visible_region_anchors` inside `vanchors`; deny ⇒ zero rows.
-- ---------------------------------------------------------------------------
CREATE FUNCTION wayfind_region_scores(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS TABLE(
    region_id         uuid,
    home_anchor_table varchar,
    home_anchor_id    uuid,
    sal_norm          float8,
    query_cos         float8,
    region_score      float8,
    in_top_n          boolean)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 0.4::float8  AS alpha,          -- salience weight
               0.6::float8  AS beta,           -- query-cosine weight (β≥α so relevance can buy a slot)
               0.05::float8 AS kappa,          -- anchor-kind prior (MEASURED — see 20260712000090)
               1.0::float8  AS prior_cogmap,   -- distilled
               0.6::float8  AS prior_context,  -- raw
               3   AS default_n,               -- default --regions
               20  AS max_n,                   -- per-call ceiling
               false AS recall_floor),         -- always admit best-cosine region (default OFF, §4.2)
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
           -- PER-KIND percent_rank (T7 header note 2). The min=max arm keeps a single-region / all-equal
           -- kind at 1.0 rather than percent_rank's lone-row 0.0 (the cold-start guard). UNCHANGED.
           CASE WHEN min(c.sal_eff) OVER kind = max(c.sal_eff) OVER kind THEN 1.0
                ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
           END AS sal_norm,
           -- THE NaN GUARD (T7). A zero-vector centroid makes `<=>` NaN, which sorts ABOVE every real
           -- value on DESC and would deterministically hoard the cut; coerce to 0.0. UNCHANGED.
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
  -- THE PER-MAP FAIRNESS STEP (#585). Rank each region within its home MAP by score, so the round-robin
  -- below can interleave maps. The `, id` tiebreak makes the rank deterministic across calls.
  ranked_rr AS (
    SELECT r.*,
           row_number() OVER (PARTITION BY r.home_anchor_id ORDER BY r.region_score DESC, r.id) AS map_rank
    FROM ranked r
  ),
  -- ROUND-ROBIN top-N. `ORDER BY map_rank ASC, region_score DESC` fills round 1 (every map's #1, by
  -- score) before round 2 (every map's #2), so no map takes a second slot until every map has a first —
  -- the volume-monopoly (#585) cannot form. `id` tiebreaks a score tie deterministically. The recall
  -- UNION is T7's (default OFF); when enabled it still force-admits the single best-cosine region.
  top_regions AS (
    (SELECT id FROM ranked_rr
      ORDER BY map_rank ASC, region_score DESC NULLS LAST, id
      LIMIT (SELECT regions_n FROM n))
    UNION
    (SELECT id FROM ranked_rr WHERE (SELECT recall_floor FROM k)
      ORDER BY query_cos DESC NULLS LAST LIMIT 1)
  )
  SELECT r.id AS region_id,
         r.home_anchor_table,
         r.home_anchor_id,
         r.sal_norm,
         r.query_cos,
         r.region_score,
         (r.id IN (SELECT id FROM top_regions)) AS in_top_n
  FROM ranked_rr r
  -- Winners first, deterministic tiebreak, so both consumers and any sweep read top-down identically.
  ORDER BY r.region_score DESC NULLS LAST, r.id;
$$;

COMMENT ON FUNCTION wayfind_region_scores(uuid, uuid, vector, int, varchar, uuid) IS
    'The ONE home for wayfind Stage-1 region scoring (issue #585 Task 2). Returns one row per candidate '
    'region across the principal''s visible anchors — per-kind sal_norm, NaN-guarded query_cos, the '
    'composite region_score, and in_top_n (cleared the per-MAP round-robin cut). Scoring is T7''s blend '
    '(20260712000090) verbatim; the added step is per-map round-robin selection so no single map can '
    'monopolize the top-N while a sibling''s competitive champion sits below the cut. Both '
    'wayfind_scope_ids (scope assembly) and wayfind_region_diagnostics (adds map names) read from here, '
    'so their selection cannot drift. Visibility-gated; deny ⇒ zero rows. No cold-start / member-deref '
    'here — those are scope assembly, and live in wayfind_scope_ids.';

-- ---------------------------------------------------------------------------
-- 2. wayfind_scope_ids — now scope ASSEMBLY over the shared scoring. Selection moved to
--    wayfind_region_scores; this dereferences the winning regions to visible members and UNIONs the
--    cold-start direct scope of region-less anchors. Signature UNCHANGED (CREATE OR REPLACE).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION wayfind_scope_ids(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
  WITH
  -- Stage-1 winners come from the single scoring home; this function no longer scores anything.
  top_regions AS (
    SELECT s.region_id
    FROM wayfind_region_scores(p_principal, p_lens, p_emb, p_regions_n, p_anchor_table, p_anchor_id) s
    WHERE s.in_top_n
  ),
  region_ids AS (
    SELECT m.member_id AS resource_id
    FROM kb_cogmap_region_members m
    WHERE m.region_id IN (SELECT region_id FROM top_regions)
      AND m.member_table = 'kb_resources'
      AND m.member_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  ),
  -- Cold-start (§5), over both kinds and UNCHANGED from T7: the visible anchors, then those with zero
  -- non-folded regions (region-less — too small to have formed one) contribute their direct homed
  -- resources, so a fresh anchor is reachable by wayfind before it has ever formed a region. This is
  -- SCOPE ASSEMBLY, not region scoring, which is why it stays here and not in wayfind_region_scores.
  vanchors AS (
    SELECT a.anchor_table, a.anchor_id
    FROM visible_region_anchors(p_principal) a
    WHERE (p_anchor_table IS NULL OR a.anchor_table = p_anchor_table)
      AND (p_anchor_id    IS NULL OR a.anchor_id    = p_anchor_id)
  ),
  thin_anchors AS (
    SELECT v.anchor_table, v.anchor_id
    FROM vanchors v
    WHERE (SELECT count(*) FROM kb_cogmap_regions r
           WHERE r.home_anchor_table = v.anchor_table
             AND r.home_anchor_id    = v.anchor_id
             AND NOT r.is_folded) = 0   -- region-less: no non-folded region formed (T7 thin_threshold 0)
  ),
  direct_ids AS (
    SELECT h.resource_id
    FROM kb_resource_homes h
    JOIN thin_anchors t
      ON t.anchor_table = h.anchor_table AND t.anchor_id = h.anchor_id
    WHERE h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  )
  SELECT resource_id FROM region_ids
  UNION
  SELECT resource_id FROM direct_ids;
$$;

COMMENT ON FUNCTION wayfind_scope_ids(uuid, uuid, vector, int, varchar, uuid) IS
    'The bounding resource-id set for a wayfind pass, over BOTH anchor kinds (spec §3.7). Stage-1 '
    'region SCORING + per-map round-robin selection now live in wayfind_region_scores (issue #585 '
    'Task 2); this function ASSEMBLES scope from the winners — dereferences their visible members — '
    'UNION the direct homed resources of region-less anchors (cold-start §5). Every stage '
    'visibility-gated; deny ⇒ zero rows, never an error. p_anchor_table/p_anchor_id scope the pool to '
    'one anchor ("wayfind within this context"); NULL ⇒ every visible anchor.';

-- ---------------------------------------------------------------------------
-- 3. wayfind_region_diagnostics — now a thin wrapper over the shared scoring plus resolved map names.
--    No longer a *copy* of the blend: the equivalence the Task-1 test pins is true BY CONSTRUCTION,
--    because there is exactly one place the score and in_top_n are computed. Signature UNCHANGED.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION wayfind_region_diagnostics(
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
    in_top_n          boolean)
LANGUAGE sql STABLE AS $$
  SELECT s.region_id,
         s.home_anchor_table,
         s.home_anchor_id,
         CASE s.home_anchor_table
           WHEN 'kb_cogmaps'  THEN (SELECT name FROM kb_cogmaps  WHERE id = s.home_anchor_id)
           WHEN 'kb_contexts' THEN (SELECT name FROM kb_contexts WHERE id = s.home_anchor_id)
         END AS home_anchor_name,
         s.sal_norm,
         s.query_cos,
         s.region_score,
         s.in_top_n
  FROM wayfind_region_scores(p_principal, p_lens, p_emb, p_regions_n, p_anchor_table, p_anchor_id) s
  -- wayfind_region_scores already orders winners-first with a deterministic tiebreak; re-state it so the
  -- diagnostics' own contract ("rows come back highest-score first") does not depend on the callee's.
  ORDER BY s.region_score DESC NULLS LAST, s.region_id;
$$;

COMMENT ON FUNCTION wayfind_region_diagnostics(uuid, uuid, vector, int, varchar, uuid) IS
    'READ-ONLY instrumentation of wayfind Stage-1 region selection (issue #585). One row per CANDIDATE '
    'region across the principal''s visible anchors — home map (resolved name), per-kind sal_norm, '
    'query_cos, region_score, and whether it cleared the per-map round-robin top-N cut. Since Task 2 a '
    'thin wrapper over wayfind_region_scores (the single scoring home) plus name resolution, so it can '
    'no longer drift from wayfind_scope_ids'' selection — the equivalence is by construction. Same '
    'visibility gate and signature as wayfind_scope_ids; deny ⇒ zero rows. Does not report cold-start '
    'region-less anchors — they form no candidate region to score.';

-- Purely additive: one new read-only scoring function plus two same-signature CREATE OR REPLACE that
-- only reorder which regions a read selects (per-map round-robin). No pre-existing shape altered or
-- dropped; the readback signatures are unchanged, so a binary that does not carry this calls the new
-- functions identically. `additive` per the schema/binary pairing design.
SELECT declare_migration(
    20260731000040,
    'additive',
    'Per-map round-robin fairness in wayfind Stage-1 (issue #585). New wayfind_region_scores becomes the single scoring home; wayfind_scope_ids and wayfind_region_diagnostics are rewired onto it (same signatures). Reorders region selection only; nothing dropped or shape-changed.'
);
