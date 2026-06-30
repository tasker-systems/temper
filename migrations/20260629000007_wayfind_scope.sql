-- Surface B Half 2: the wayfind region-salience scope-resolution funnel (spec §4/§5/§7).
-- New scope-resolution SQL only; the unified_search blend is unchanged and consumes the
-- returned id set via p_scope_ids. All tuning constants live in the k CTE (single home,
-- mirroring unified_search's k CTE — calibrate on the corpus, see spec §8).

-- The set form of cogmap_readable_by_profile (membership-flat map admission): the maps a
-- principal can read, via direct kb_team_members ∩ kb_team_cogmaps (NOT ancestor-expanded —
-- so "a map you can read" and "the resources homed in it" agree by construction, spec §7).
-- DISTINCT collapses a principal joined to a map through several teams to one row.
CREATE FUNCTION cogmap_visible_maps(p_principal uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    SELECT DISTINCT tc.cogmap_id
    FROM kb_team_cogmaps tc
    JOIN profile_effective_teams(p_principal) e ON e.team_id = tc.team_id;
$$;

-- The bounding resource-id set for a wayfind pass: members of the top-N pooled regions across the
-- principal's visible maps, UNION the direct homed participants of region-less / thin maps
-- (cold-start §5), every stage visibility-gated. Returns a SETOF uuid consumed as unified_search's
-- p_scope_ids. NULL lens → memoized salience; non-null lens → recompute from stored components under
-- the override's s_*. NULL emb → β term zeroed (salience-only). NULL/over-ceiling N → clamped.
-- Deny → zero rows, never an error.
CREATE FUNCTION wayfind_scope_ids(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 0.4::float8 AS alpha,        -- salience weight
               0.6::float8 AS beta,         -- query-cosine weight (β≥α so relevance can buy a slot, §4.1)
               3   AS default_n,            -- default --regions
               20  AS max_n,                -- per-call ceiling
               0   AS thin_threshold,       -- region-count <= this ⇒ bypass to direct scope (region-less)
               false AS recall_floor),      -- always admit best-cosine region (default OFF, §4.2)
  -- Clamp N into [1, max_n]: COALESCE the default, cap at the ceiling, and floor at 1 so a
  -- negative / zero / overflow-wrapped N never reaches `LIMIT` (which Postgres rejects when
  -- negative) — deny degrades to zero rows, never an error (spec §5/§7/§8).
  n AS (SELECT GREATEST(LEAST(COALESCE(p_regions_n, (SELECT default_n FROM k)),
                              (SELECT max_n FROM k)), 1) AS regions_n),
  vmaps AS (SELECT t.cogmap_id FROM cogmap_visible_maps(p_principal) AS t(cogmap_id)),
  lens AS (SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses WHERE id = p_lens),
  -- candidate regions in visible maps; salience = memoized (default) or recomputed (override lens)
  cand AS (
    SELECT r.id, r.centroid,
           CASE WHEN p_lens IS NULL THEN r.salience
                ELSE (SELECT s_telos FROM lens)   * COALESCE(r.telos_alignment, 0)
                   + (SELECT s_ref FROM lens)     * COALESCE(r.reference_standing, 0)
                   + (SELECT s_central FROM lens) * COALESCE(r.centrality, 0)
           END AS sal_eff
    FROM kb_cogmap_regions r
    WHERE r.cogmap_id IN (SELECT cogmap_id FROM vmaps) AND NOT r.is_folded
  ),
  bounds AS (SELECT min(sal_eff) AS lo, max(sal_eff) AS hi FROM cand),
  scored AS (
    SELECT c.id,
           CASE WHEN (SELECT hi FROM bounds) = (SELECT lo FROM bounds) THEN 1.0
                ELSE (c.sal_eff - (SELECT lo FROM bounds))
                   / NULLIF((SELECT hi FROM bounds) - (SELECT lo FROM bounds), 0)
           END AS sal_norm,
           CASE WHEN p_emb IS NULL THEN 0.0 ELSE 1 - (c.centroid <=> p_emb) END AS query_cos
    FROM cand c
  ),
  ranked AS (
    SELECT id, query_cos,
           (SELECT alpha FROM k) * sal_norm + (SELECT beta FROM k) * query_cos AS region_score
    FROM scored
  ),
  top_regions AS (
    -- NULLS LAST defends the bogus-override-lens edge: a non-existent `p_lens` makes the recompute
    -- subqueries NULL → NULL region_score, which must sort BELOW real-scored regions, not above.
    (SELECT id FROM ranked ORDER BY region_score DESC NULLS LAST LIMIT (SELECT regions_n FROM n))
    UNION
    (SELECT id FROM ranked WHERE (SELECT recall_floor FROM k) ORDER BY query_cos DESC NULLS LAST LIMIT 1)
  ),
  region_ids AS (
    SELECT m.member_id AS resource_id
    FROM kb_cogmap_region_members m
    WHERE m.region_id IN (SELECT id FROM top_regions)
      AND m.member_table = 'kb_resources'
      AND m.member_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  ),
  -- cold-start (§5): region-less / thin maps contribute their direct homed participants.
  thin_maps AS (
    SELECT v.cogmap_id FROM vmaps v
    WHERE (SELECT count(*) FROM kb_cogmap_regions r
           WHERE r.cogmap_id = v.cogmap_id AND NOT r.is_folded) <= (SELECT thin_threshold FROM k)
  ),
  direct_ids AS (
    SELECT h.resource_id
    FROM kb_resource_homes h
    WHERE h.anchor_table = 'kb_cogmaps'
      AND h.anchor_id IN (SELECT cogmap_id FROM thin_maps)
      AND h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  )
  SELECT resource_id FROM region_ids
  UNION
  SELECT resource_id FROM direct_ids;
$$;
