-- H5 — the one REAL narrow principal (lohjishan: 1 context anchor, 10 live regions, 0 cogmap
-- regions). It shares no region with any other principal, so it admits no cross-principal delta.
-- What it DOES show is the granularity a narrow reach gives sal_norm, and that the min=max guard
-- is not what is producing its values. Read-only.

\echo '=== A. lohjishan sal_norm (incumbent == candidate here: one anchor per kind) ==='
WITH cand AS (
  SELECT r.id, r.home_anchor_table, r.home_anchor_id, r.salience AS sal_eff
  FROM visible_region_anchors('019e4d87-af1d-78a3-addd-dbe0f0e77079'::uuid) v
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = v.anchor_table AND r.home_anchor_id = v.anchor_id
  WHERE NOT r.is_folded
),
sn AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER w = max(c.sal_eff) OVER w THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
         END AS sal_norm
  FROM cand c
  WINDOW w AS (PARTITION BY c.home_anchor_table)
)
SELECT count(*) AS partition_rows,
       count(DISTINCT sal_eff) AS distinct_sal_eff,
       (min(sal_eff) = max(sal_eff)) AS min_eq_max_guard_fires,
       count(DISTINCT sal_norm) AS distinct_sal_norm_values,
       round(min(sal_norm)::numeric, 4) AS min_sal_norm,
       round((1.0 / NULLIF(count(*) - 1, 0))::numeric, 4) AS percent_rank_step
FROM sn;

\echo ''
\echo '=== B. step size by partition width — what reach costs in resolution ==='
SELECT who, partition_rows,
       round((1.0 / NULLIF(partition_rows - 1, 0))::numeric, 5) AS percent_rank_step
FROM (VALUES
  ('j-cole-taylor / contexts', 472),
  ('j-cole-taylor / cogmaps',  444),
  ('agent          / contexts', 438),
  ('agent          / cogmaps',  400),
  ('lohjishan      / contexts',  10),
  ('lohjishan      / cogmaps',    0)
) AS t(who, partition_rows)
ORDER BY partition_rows DESC;
