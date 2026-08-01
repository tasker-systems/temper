-- P1 — structural quantities. No query vector needed; these are corpus properties.
-- Principal: 019d4add-f49d-7c43-a87d-dda470e5dd9c
\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'

\echo '=== A. visible anchors: regions vs homed resources ==='
WITH v AS (SELECT * FROM visible_region_anchors(:'prin'::uuid))
SELECT v.anchor_table,
       COALESCE(cm.name, cx.name)                                   AS anchor,
       (SELECT count(*) FROM kb_cogmap_regions r
         WHERE r.home_anchor_table = v.anchor_table
           AND r.home_anchor_id = v.anchor_id AND NOT r.is_folded)   AS live_regions,
       (SELECT count(*) FROM kb_resource_homes h
          JOIN resources_visible_to(:'prin'::uuid) rv ON rv.resource_id = h.resource_id
         WHERE h.anchor_table = v.anchor_table AND h.anchor_id = v.anchor_id) AS visible_homed
FROM v
LEFT JOIN kb_cogmaps  cm ON v.anchor_table = 'kb_cogmaps'  AND cm.id = v.anchor_id
LEFT JOIN kb_contexts cx ON v.anchor_table = 'kb_contexts' AND cx.id = v.anchor_id
ORDER BY live_regions DESC, visible_homed DESC;

\echo ''
\echo '=== B. THE MATERIAL-PER-SLOT QUESTION: visible members per LIVE region, by kind ==='
WITH v AS (SELECT * FROM visible_region_anchors(:'prin'::uuid)),
     reg AS (
       SELECT r.id, r.home_anchor_table AS kind
       FROM kb_cogmap_regions r
       JOIN v ON v.anchor_table = r.home_anchor_table AND v.anchor_id = r.home_anchor_id
       WHERE NOT r.is_folded),
     mc AS (
       SELECT reg.id, reg.kind,
              (SELECT count(*) FROM kb_cogmap_region_members m
                WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
                  AND m.member_id IN (SELECT resource_id FROM resources_visible_to(:'prin'::uuid))
              ) AS visible_members
       FROM reg)
SELECT kind,
       count(*)                                                    AS live_regions,
       sum(visible_members)                                        AS total_visible_members,
       round(avg(visible_members)::numeric, 2)                     AS mean_members,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY visible_members) AS p50,
       percentile_disc(0.9) WITHIN GROUP (ORDER BY visible_members) AS p90,
       max(visible_members)                                        AS max_members,
       count(*) FILTER (WHERE visible_members <= 1)                AS singleton_or_empty,
       round(100.0 * count(*) FILTER (WHERE visible_members <= 1) / count(*), 1) AS pct_singleton
FROM mc GROUP BY kind ORDER BY kind;

\echo ''
\echo '=== C. members of each anchors CHAMPION-ELIGIBLE regions (top by salience, per anchor) ==='
-- Approximates "what a slot buys" independent of any query: the biggest-salience region per anchor.
WITH v AS (SELECT * FROM visible_region_anchors(:'prin'::uuid)),
     reg AS (
       SELECT r.id, r.home_anchor_table AS kind, r.home_anchor_id, r.salience,
              row_number() OVER (PARTITION BY r.home_anchor_id ORDER BY r.salience DESC) AS rn
       FROM kb_cogmap_regions r
       JOIN v ON v.anchor_table = r.home_anchor_table AND v.anchor_id = r.home_anchor_id
       WHERE NOT r.is_folded)
SELECT reg.kind,
       COALESCE(cm.name, cx.name) AS anchor,
       reg.rn                     AS salience_rank_in_anchor,
       round(reg.salience::numeric, 3) AS salience,
       (SELECT count(*) FROM kb_cogmap_region_members m
         WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
           AND m.member_id IN (SELECT resource_id FROM resources_visible_to(:'prin'::uuid))) AS visible_members
FROM reg
LEFT JOIN kb_cogmaps  cm ON reg.kind = 'kb_cogmaps'  AND cm.id = reg.home_anchor_id
LEFT JOIN kb_contexts cx ON reg.kind = 'kb_contexts' AND cx.id = reg.home_anchor_id
WHERE reg.rn <= 3
ORDER BY reg.kind, anchor, reg.rn;

\echo ''
\echo '=== D. chunk count + FTS text length per VISIBLE resource, by home-anchor kind ==='
-- vec_norm is best-of-N chunks; ts_rank rewards term density over document length.
WITH v AS (SELECT * FROM visible_region_anchors(:'prin'::uuid)),
     res AS (
       SELECT DISTINCT h.resource_id, h.anchor_table AS kind
       FROM kb_resource_homes h
       JOIN v ON v.anchor_table = h.anchor_table AND v.anchor_id = h.anchor_id
       JOIN resources_visible_to(:'prin'::uuid) rv ON rv.resource_id = h.resource_id
       JOIN kb_resources r ON r.id = h.resource_id AND r.is_active AND r.ingest_state = 'complete')
SELECT res.kind,
       count(*)                                                          AS resources,
       round(avg((SELECT count(*) FROM kb_chunks c
                   WHERE c.resource_id = res.resource_id AND c.is_current))::numeric, 2) AS mean_chunks,
       percentile_disc(0.5) WITHIN GROUP (
         ORDER BY (SELECT count(*) FROM kb_chunks c
                    WHERE c.resource_id = res.resource_id AND c.is_current))             AS p50_chunks,
       max((SELECT count(*) FROM kb_chunks c
             WHERE c.resource_id = res.resource_id AND c.is_current))                    AS max_chunks
FROM res GROUP BY res.kind ORDER BY res.kind;
