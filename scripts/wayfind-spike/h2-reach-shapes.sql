-- H2 — exactly which anchors each principal reaches, and how much the reach sets overlap.
-- Decides whether a cross-PRINCIPAL differential is possible on real principals, or whether the
-- narrow-reach case has to be modelled by restricting one principal's anchor set.
-- Read-only.

\echo '=== A. anchors per principal, named, with live-region counts ==='
WITH p AS (
  SELECT id, COALESCE(handle, display_name, '(none)') AS who FROM kb_profiles
),
va AS (
  SELECT p.who, v.anchor_table, v.anchor_id
  FROM p CROSS JOIN LATERAL visible_region_anchors(p.id) v
),
lr AS (
  SELECT home_anchor_table AS at, home_anchor_id AS ai, count(*) AS n
  FROM kb_cogmap_regions WHERE NOT is_folded GROUP BY 1,2
)
SELECT va.who,
       va.anchor_table AS kind,
       COALESCE(cm.name, cx.name, va.anchor_id::text) AS anchor,
       COALESCE(lr.n, 0) AS live_regions
FROM va
LEFT JOIN lr ON lr.at = va.anchor_table AND lr.ai = va.anchor_id
LEFT JOIN kb_cogmaps  cm ON va.anchor_table='kb_cogmaps'  AND cm.id = va.anchor_id
LEFT JOIN kb_contexts cx ON va.anchor_table='kb_contexts' AND cx.id = va.anchor_id
ORDER BY va.who, va.anchor_table, 4 DESC;

\echo ''
\echo '=== B. pairwise overlap of REGION sets between principals (shared / only-a / only-b) ==='
WITH p AS (
  SELECT id, COALESCE(handle, display_name, '(none)') AS who FROM kb_profiles
),
reg AS (
  SELECT p.who, r.id AS region_id
  FROM p
  CROSS JOIN LATERAL visible_region_anchors(p.id) v
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = v.anchor_table AND r.home_anchor_id = v.anchor_id
   AND NOT r.is_folded
)
SELECT a.who AS principal_a, b.who AS principal_b,
       count(*) FILTER (WHERE a.region_id IS NOT NULL AND b.region_id IS NOT NULL) AS shared
FROM (SELECT DISTINCT who, region_id FROM reg) a
FULL JOIN (SELECT DISTINCT who, region_id FROM reg) b ON b.region_id = a.region_id
WHERE a.who < b.who
GROUP BY 1,2
ORDER BY 3 DESC;

\echo ''
\echo '=== C. per-KIND partition size per principal — the incumbent sal_norm denominator ==='
WITH p AS (
  SELECT id, COALESCE(handle, display_name, '(none)') AS who FROM kb_profiles
)
SELECT p.who,
       count(*) FILTER (WHERE r.home_anchor_table='kb_cogmaps')  AS cogmap_partition,
       count(*) FILTER (WHERE r.home_anchor_table='kb_contexts') AS context_partition
FROM p
CROSS JOIN LATERAL visible_region_anchors(p.id) v
JOIN kb_cogmap_regions r
  ON r.home_anchor_table = v.anchor_table AND r.home_anchor_id = v.anchor_id
 AND NOT r.is_folded
GROUP BY 1 ORDER BY 2 DESC, 3 DESC;
