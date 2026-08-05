-- H1 — orientation for the sal_norm hinge (task 019fd25e).
-- Verify the DEPLOYED objects before reasoning about them, and map the reach landscape
-- so "measure across reach shapes" can be grounded in what actually exists.
-- Read-only.

\echo '=== A. deployed signatures ==='
SELECT p.proname || '(' || pg_get_function_arguments(p.oid) || ')' AS sig
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = 'public'
  AND p.proname IN ('wayfind_region_scores','visible_region_anchors','wayfind_scope_reach',
                    'wayfind_scope_ids','resources_visible_to')
ORDER BY 1;

\echo ''
\echo '=== B. wayfind_region_scores: is the shipped expression still what the probe reproduces? ==='
SELECT
  (prosrc ~ 'percent_rank\(\) OVER \(PARTITION BY c\.home_anchor_table')      AS per_kind_percent_rank,
  (prosrc ~ 'visible_region_anchors')                                        AS cand_joins_visible_anchors,
  (prosrc ~ 'PARTITION BY r\.home_anchor_id')                                AS rr_partitions_per_anchor,
  (prosrc ~ 'NOT r\.is_folded' OR prosrc ~ 'NOT is_folded')                   AS live_regions_only,
  (prosrc ~ '0\.4::float8')                                                  AS alpha_0_4,
  (prosrc ~ '0\.6::float8')                                                  AS beta_0_6,
  (prosrc ~ '0\.05::float8')                                                 AS kappa_0_05,
  md5(prosrc)                                                                AS body_md5
FROM pg_proc WHERE proname = 'wayfind_region_scores';

\echo ''
\echo '=== C. the sal_eff / sal_norm block, verbatim from the deployed body ==='
SELECT regexp_replace(
         substring(prosrc from 'cand AS \(.*?\)\s*,\s*scored AS \(.*?\n\s*\)'),
         E'\n', ' | ', 'g') AS block
FROM pg_proc WHERE proname = 'wayfind_region_scores';

\echo ''
\echo '=== D. visible_region_anchors, verbatim ==='
SELECT pg_get_functiondef(p.oid) AS def
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname='public' AND p.proname='visible_region_anchors';

\echo ''
\echo '=== E. reach landscape — live regions per region-bearing anchor ==='
SELECT r.home_anchor_table AS kind,
       COALESCE(cm.name, cx.name, r.home_anchor_id::text) AS anchor,
       count(*) AS live_regions
FROM kb_cogmap_regions r
LEFT JOIN kb_cogmaps  cm ON r.home_anchor_table='kb_cogmaps'  AND cm.id = r.home_anchor_id
LEFT JOIN kb_contexts cx ON r.home_anchor_table='kb_contexts' AND cx.id = r.home_anchor_id
WHERE NOT r.is_folded
GROUP BY 1,2
ORDER BY 1, 3 DESC;

\echo ''
\echo '=== F. every principal, and how much of that landscape each one reaches ==='
SELECT p.id::text AS principal,
       COALESCE(p.handle, p.display_name, '(none)') AS who,
       count(*) FILTER (WHERE v.anchor_table='kb_cogmaps')  AS cogmap_anchors,
       count(*) FILTER (WHERE v.anchor_table='kb_contexts') AS context_anchors,
       (SELECT count(*) FROM kb_cogmap_regions r
         WHERE NOT r.is_folded
           AND (r.home_anchor_table, r.home_anchor_id) IN
               (SELECT anchor_table, anchor_id FROM visible_region_anchors(p.id))
       ) AS live_regions_reached
FROM kb_profiles p
LEFT JOIN LATERAL visible_region_anchors(p.id) v ON true
GROUP BY p.id, p.handle, p.display_name
ORDER BY 5 DESC NULLS LAST, 1;
