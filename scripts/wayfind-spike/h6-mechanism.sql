-- H6 — the mechanism behind D's slot shift, stated directly rather than inferred.
-- sal_norm does not depend on the query, so this needs no query vectors.
-- Principal: j-cole-taylor (shape 1). Arm: shipped default (p_lens NULL => sal_eff = r.salience).
-- Read-only.

WITH cand AS (
  SELECT r.id, r.home_anchor_table, r.home_anchor_id, r.salience AS sal_eff
  FROM visible_region_anchors('019d4add-f49d-7c43-a87d-dda470e5dd9c'::uuid) v
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = v.anchor_table AND r.home_anchor_id = v.anchor_id
  WHERE NOT r.is_folded
),
sn AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
         END AS sal_cur,
         CASE WHEN min(c.sal_eff) OVER anchw = max(c.sal_eff) OVER anchw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table, c.home_anchor_id
                                        ORDER BY c.sal_eff)
         END AS sal_alt
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.home_anchor_table),
         anchw AS (PARTITION BY c.home_anchor_table, c.home_anchor_id)
)
SELECT COALESCE(cm.name, cx.name) AS anchor,
       sn.home_anchor_table AS kind,
       count(*) AS live_regions,
       round(avg(sn.sal_eff)::numeric, 4)  AS mean_raw_salience,
       round(max(sn.sal_eff)::numeric, 4)  AS max_raw_salience,
       round(avg(sn.sal_cur)::numeric, 4)  AS mean_sal_norm_incumbent,
       round(max(sn.sal_cur)::numeric, 4)  AS max_sal_norm_incumbent,
       round(avg(sn.sal_alt)::numeric, 4)  AS mean_sal_norm_candidate,
       round(max(sn.sal_alt)::numeric, 4)  AS max_sal_norm_candidate
FROM sn
LEFT JOIN kb_cogmaps  cm ON sn.home_anchor_table = 'kb_cogmaps'  AND cm.id = sn.home_anchor_id
LEFT JOIN kb_contexts cx ON sn.home_anchor_table = 'kb_contexts' AND cx.id = sn.home_anchor_id
GROUP BY 1,2 ORDER BY 2, 3 DESC;
