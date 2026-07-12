-- T7 — Anchor-agnostic wayfind (spec §3.7).
--
-- Wayfind stops being a cogmap-only funnel. It pools regions from EVERY anchor the principal can
-- read — cogmaps and contexts alike — so one wayfind can surface a distilled idea AND the raw work
-- it came from (the "composition read" §3.7 exists to deliver).
--
-- Three things change inside wayfind_scope_ids, and two of them are corrections to the spec's own
-- design, made because the numbers said so. Measured on prod (@me/temper: 273 context regions,
-- 217 cogmap regions) before writing a line of this.
--
--   1. THE POOL is keyed on (home_anchor_table, home_anchor_id) via visible_region_anchors, not on
--      cogmap_id via cogmap_visible_maps.
--
--   2. SALIENCE IS NORMALIZED PER ANCHOR KIND, BY percent_rank — NOT pooled min-max.
--      Raw salience is not comparable across kinds. A context's salience is driven by `centrality`,
--      an UNBOUNDED degree count: max 276 in a context vs 21.5 in a cogmap, giving max salience
--      69.55 vs 9.53. Under the old pooled min-max, dropping contexts into the pool collapsed every
--      cogmap region's sal_norm to <= 0.137 — the alpha term shrank from a [0, 0.4] range to
--      [0, 0.055], annihilating the distilled salience signal. That is the very drowning §3.7 says
--      the prior exists to prevent, and an ADDITIVE prior cannot repair a MULTIPLICATIVE range crush.
--      min-max is also outlier-dominated WITHIN a kind (90% of context regions sat in the bottom 5%
--      of their own range; cogmaps the bottom 23%), so alpha was already near-inert. percent_rank is
--      outlier-immune and spreads each kind uniformly over [0,1], which makes alpha a real signal for
--      both kinds for the first time and leaves kappa as the ONLY cross-kind lever — which is exactly
--      what §3.7 wants the prior to be.
--
--   3. THE ANCHOR PRIOR (kappa) is keyed on home_anchor_table here in the k CTE, alongside alpha/beta
--      (the established "wayfind tuning constants stay SQL-resident" precedent). It is therefore
--      correct by construction — keyed on the ACTUAL anchor kind, not on a lens that merely proxies
--      for one. See the COMMENT on kb_cogmap_lenses.kappa_anchor_prior below.
--
-- Also fixed here, because turning contexts on is what makes it fire: THE NaN TRAP. See query_cos.
--
-- Deploy-skew safety: wayfind_scope_ids gains two trailing params WITH DEFAULTS, so a 4-arg call
-- from already-deployed code still resolves after this migration runs and before that code updates
-- (migrations are operator-run ahead of deploy). DROP + CREATE rather than CREATE OR REPLACE because
-- adding a parameter changes a function's identity — CREATE OR REPLACE would leave a second,
-- ambiguous overload callable. Same mechanics as 20260709000050_act_correlation_passthrough.sql.

-- ---------------------------------------------------------------------------
-- 1. visible_region_anchors — the anchor-generic peer of cogmap_visible_maps.
--
-- Both halves already exist and already encode the read-up access model; this only unions them, so
-- there is exactly one place where "which anchors can this principal pool regions from" is decided.
-- ---------------------------------------------------------------------------
CREATE FUNCTION visible_region_anchors(p_principal uuid)
RETURNS TABLE(anchor_table varchar(64), anchor_id uuid) LANGUAGE sql STABLE AS $$
    SELECT 'kb_cogmaps'::varchar(64), t.cogmap_id
    FROM cogmap_visible_maps(p_principal) AS t(cogmap_id)
    UNION ALL
    SELECT 'kb_contexts'::varchar(64), c.context_id
    FROM contexts_readable_by(p_principal) c;
$$;

COMMENT ON FUNCTION visible_region_anchors(uuid) IS
    'The anchors a principal may pool regions from, over BOTH kinds (spec §3.7). Replaces '
    'cogmap_visible_maps as wayfind''s admission gate. UNION ALL is safe: the two arms are disjoint '
    'by anchor_table, and each source already de-duplicates within its own kind.';

-- ---------------------------------------------------------------------------
-- 2. wayfind_scope_ids — anchor-agnostic pool, per-kind normalization, anchor prior, NaN guard.
-- ---------------------------------------------------------------------------
DROP FUNCTION wayfind_scope_ids(uuid, uuid, vector, int);

CREATE FUNCTION wayfind_scope_ids(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 0.4::float8  AS alpha,          -- salience weight
               0.6::float8  AS beta,           -- query-cosine weight (β≥α so relevance can buy a slot, §4.1)
               -- The anchor-kind prior. MEASURED, not guessed: swept against 40 real query vectors on
               -- prod. The spec's stated priors (1.0/0.6) imply κ≈0.25 — which is a TOTAL SHUTOUT
               -- (cogmaps take 3/3 of top-3 and 10/10 of top-10; context regions never appear), i.e. a
               -- structural exclusion, which §3.7 explicitly says the prior must NOT be. The curve:
               --
               --   κ      cogmap share of top-3   of top-10   queries still surfacing a context in top-3
               --   0.00        0.85 / 3            2.85 / 10        39 / 40
               --   0.05       ~2.1  / 3           ~5.8  / 10       ~25 / 40      <-- a tilt
               --   0.10        2.85 / 3            8.65 / 10         6 / 40
               --   0.25        3.00 / 3           10.00 / 10         0 / 40      <-- an exclusion
               --
               -- κ=0.05 is the tilt: distilled content leads, raw work stays genuinely reachable, and
               -- a single wayfind surfaces BOTH — which is the composition read. Re-tune by additive
               -- migration as the corpus grows (α/β precedent); re-run the sweep, don't guess.
               0.05::float8 AS kappa,
               1.0::float8  AS prior_cogmap,    -- distilled
               0.6::float8  AS prior_context,   -- raw
               3   AS default_n,                -- default --regions
               20  AS max_n,                    -- per-call ceiling
               0   AS thin_threshold,           -- region-count <= this ⇒ bypass to direct scope (region-less)
               false AS recall_floor),          -- always admit best-cosine region (default OFF, §4.2)
  -- Clamp N into [1, max_n]: COALESCE the default, cap at the ceiling, and floor at 1 so a
  -- negative / zero / overflow-wrapped N never reaches `LIMIT` (which Postgres rejects when
  -- negative) — deny degrades to zero rows, never an error (spec §5/§7/§8).
  n AS (SELECT GREATEST(LEAST(COALESCE(p_regions_n, (SELECT default_n FROM k)),
                              (SELECT max_n FROM k)), 1) AS regions_n),
  -- The anchors in play. Unscoped ⇒ every visible anchor over both kinds. Scoped (`--context X
  -- --wayfind` / `--cogmap Y --wayfind`) ⇒ that anchor only — but STILL filtered through
  -- visible_region_anchors, so naming an anchor you cannot read yields zero rows, never a leak.
  vanchors AS (
    SELECT a.anchor_table, a.anchor_id
    FROM visible_region_anchors(p_principal) a
    WHERE (p_anchor_table IS NULL OR a.anchor_table = p_anchor_table)
      AND (p_anchor_id    IS NULL OR a.anchor_id    = p_anchor_id)
  ),
  lens AS (SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses WHERE id = p_lens),
  -- Candidate regions across the visible anchors; salience = memoized (default) or recomputed
  -- (override lens). Keyed on the anchor pair — cogmap_id is vestigial and unread here (spec §3.6 M2).
  cand AS (
    SELECT r.id, r.centroid, r.home_anchor_table,
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
    SELECT c.id, c.home_anchor_table,
           -- PER-KIND percent_rank (see header note 2). The min=max arm preserves the shipped
           -- semantics of the old min-max normalizer's `WHEN hi = lo THEN 1.0` case: an anchor kind
           -- with a single region, or with all-equal salience, normalizes to 1.0 rather than being
           -- silently zeroed. (percent_rank() of a lone row is 0, NOT 1 — this arm is load-bearing,
           -- it is what keeps the cold-start / single-region case from regressing.)
           CASE WHEN min(c.sal_eff) OVER kind = max(c.sal_eff) OVER kind THEN 1.0
                ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
           END AS sal_norm,
           -- THE NaN GUARD. A region whose members carry no embedding (a bodyless resource ⇒ zero
           -- chunks) has a ZERO-VECTOR centroid, and pgvector's `<=>` against a zero vector is NaN.
           -- Postgres sorts NaN ABOVE every real value on ORDER BY … DESC, and `NULLS LAST` does not
           -- guard it — so un-guarded, the `top_regions` LIMIT below would return those contentless
           -- regions for EVERY query, deterministically. (Measured: 10 such regions, 3.7% of prod's
           -- context regions.) This was latent in the shipped function; it only stayed dormant
           -- because no COGMAP region has a zero centroid. Turning contexts on is what fires it.
           --
           -- Coerce to 0.0, don't exclude: a zero vector has no direction, so it has no similarity to
           -- anything — the honest score is zero, and the region still competes on salience alone.
           -- NULLIF traps NaN because Postgres defines NaN = NaN as TRUE. Dimension-agnostic, so it
           -- also covers a zero-vector QUERY embedding.
           CASE WHEN p_emb IS NULL THEN 0.0
                ELSE COALESCE(NULLIF(1 - (c.centroid <=> p_emb), 'NaN'::float8), 0.0)
           END AS query_cos
    FROM cand c
    WINDOW kind AS (PARTITION BY c.home_anchor_table)
  ),
  ranked AS (
    SELECT id, query_cos,
           (SELECT alpha FROM k) * sal_norm
         + (SELECT beta  FROM k) * query_cos
         + (SELECT kappa FROM k) * CASE home_anchor_table
                                     WHEN 'kb_cogmaps' THEN (SELECT prior_cogmap  FROM k)
                                     ELSE                   (SELECT prior_context FROM k)
                                   END AS region_score
    FROM scored
  ),
  top_regions AS (
    -- NULLS LAST defends the bogus-override-lens edge: a non-existent `p_lens` makes the recompute
    -- subqueries NULL. (It cannot admit a NaN: query_cos is guarded above, so region_score is NaN-free.)
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
  -- cold-start (§5), now over both kinds: a region-less anchor contributes its direct homed
  -- resources, so a fresh context is reachable by wayfind before it has ever formed a region —
  -- the same courtesy cogmaps already got. Bounded by thin_threshold = 0, so it fires ONLY for an
  -- anchor with zero regions, which is by definition an anchor too small to have formed any.
  thin_anchors AS (
    SELECT v.anchor_table, v.anchor_id
    FROM vanchors v
    WHERE (SELECT count(*) FROM kb_cogmap_regions r
           WHERE r.home_anchor_table = v.anchor_table
             AND r.home_anchor_id    = v.anchor_id
             AND NOT r.is_folded) <= (SELECT thin_threshold FROM k)
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
    'The bounding resource-id set for a wayfind pass, over BOTH anchor kinds (spec §3.7). Pools the '
    'top-N regions across the principal''s visible anchors — salience normalized PER KIND by '
    'percent_rank, tilted by the anchor-kind prior κ — UNION the direct homed resources of '
    'region-less anchors (cold-start §5). Every stage visibility-gated; deny ⇒ zero rows, never an '
    'error. p_anchor_table/p_anchor_id scope the pool to one anchor ("wayfind within this context"); '
    'NULL ⇒ every visible anchor. Defaulted so a pre-T7 4-arg call still resolves across deploy skew.';

-- ---------------------------------------------------------------------------
-- 3. Say plainly that the lens column is not consumed, so the next reader does not go looking for
--    the code that reads it, and does not assume its 0.0 default is silently zeroing the prior.
-- ---------------------------------------------------------------------------
COMMENT ON COLUMN kb_cogmap_lenses.kappa_anchor_prior IS
    'NOT CONSUMED. Added in T2 "consumed in T7"; T7 instead keys the anchor prior on home_anchor_table '
    'directly in wayfind_scope_ids'' k CTE, alongside α/β (the SQL-resident tuning-constant precedent), '
    'because that is correct BY CONSTRUCTION — keyed on the actual anchor kind rather than on a lens '
    'that merely proxies for one (context regions happen to form under workflow-default and cogmap '
    'regions under telos-default, but nothing enforces that at read time). This column is the seam to '
    'reach for if a lens-tunable prior is ever wanted; it is not dead by accident.';
