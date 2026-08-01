#!/usr/bin/env python3
"""Would normalizing salience per ANCHOR instead of per anchor-KIND make it principal-agnostic?

`wayfind_region_scores` computes `sal_norm` as
`percent_rank() OVER (PARTITION BY home_anchor_table ORDER BY sal_eff)` over `cand`, and `cand` is
joined to `visible_region_anchors(p_principal)` (`20260731000050`, lines 98-116). So the
normalization *frame* is the asker's visible set: measured, 382 of 385 regions get a different
`sal_norm` depending on which anchors the asker can see.

The candidate alternative is one line — `PARTITION BY home_anchor_table, home_anchor_id`. It is
worth measuring for two independent reasons:

  * **It should be exactly principal-agnostic.** Visibility is granted per ANCHOR, not per region:
    `cand` admits every live region of every visible anchor. So an anchor's own live region set is
    the same set regardless of who is asking, and a percent_rank framed on it cannot vary by
    principal. Test A checks that rather than assuming it.
  * **It matches the unit selection already uses.** `ranked_rr` ranks
    `PARTITION BY r.home_anchor_id` — the round-robin picks per anchor — while `scored` normalizes
    per anchor KIND. Normalizing over a different unit than you select over is the mismatch this
    probe is aimed at.

Both arms are computed **differentially from one loaded candidate set**, never from two runs, and
the incumbent arm reproduces the shipped expression verbatim including the `min = max ⇒ 1.0`
cold-start guard.

Test A — principal invariance. Same regions under two visible-anchor sets ("all" = every anchor the
principal sees; "map" = scoped to one map, which is what a narrower principal's reach looks like).
The incumbent should differ; the alternative should not.

Test B — selection outcome. Regions ever selected and live nodes thereby reachable, over the probe
set at widths {1,3,5,10,20}, under each normalization. Round-robin is replicated as
`ORDER BY map_rank, score DESC, id LIMIT n`, which is the shipped `top_regions` ordering.

Read-only.

    python3 gen_salnorm_alt.py vectors-24.tsv 'Temper — self-cognition' > salnorm_alt.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
PRINCIPAL = "019d4add-f49d-7c43-a87d-dda470e5dd9c"
ALPHA, BETA, KAPPA = 0.4, 0.6, 0.05
PRIOR_COGMAP, PRIOR_CONTEXT = 1.0, 0.6

vectors_path, map_name = sys.argv[1], sys.argv[2]
esc_map = map_name.replace("'", "''")

rows = []
with open(vectors_path) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        _q, v = line.split("\t", 1)
        rows.append((i, v))

values = [f"({rows[0][0]}, '{rows[0][1]}'::vector)"] + [f"({i}, '{v}')" for i, v in rows[1:]]
values_sql = ",\n  ".join(values)
widths_sql = ", ".join(f"({w})" for w in WIDTHS)

print(f"""\\set ON_ERROR_STOP on
\\echo '=== sal_norm: per anchor-KIND (incumbent) vs per ANCHOR (candidate) ==='

WITH
q(qi, emb) AS (VALUES
  {values_sql}
),
w(n) AS (VALUES {widths_sql}),
ctx AS (
  SELECT (SELECT id FROM kb_cogmaps WHERE name='{esc_map}') AS map_id
),
lens AS (
  SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses
  WHERE name='telos-default' AND home_anchor_table IS NULL
),
scopings(s) AS (VALUES ('1-all'), ('2-map-only')),
-- `cand`, reproduced: every live region of every visible anchor, sal_eff recomputed under the lens.
cand AS (
  SELECT sc.s, r.id, r.centroid, r.home_anchor_table, r.home_anchor_id,
         l.s_telos * COALESCE(r.telos_alignment, 0)
       + l.s_ref   * COALESCE(r.reference_standing, 0)
       + l.s_central * COALESCE(r.centrality, 0) AS sal_eff
  FROM scopings sc
  CROSS JOIN lens l
  CROSS JOIN ctx c
  JOIN visible_region_anchors('{PRINCIPAL}'::uuid) v ON true
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = v.anchor_table AND r.home_anchor_id = v.anchor_id
   AND NOT r.is_folded
  WHERE sc.s = '1-all'
     OR (r.home_anchor_table = 'kb_cogmaps' AND r.home_anchor_id = c.map_id)
),
salnorm AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.s, c.home_anchor_table
                                        ORDER BY c.sal_eff) END AS sal_cur,
         CASE WHEN min(c.sal_eff) OVER anchw = max(c.sal_eff) OVER anchw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.s, c.home_anchor_table, c.home_anchor_id
                                        ORDER BY c.sal_eff) END AS sal_alt
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.s, c.home_anchor_table),
         anchw AS (PARTITION BY c.s, c.home_anchor_table, c.home_anchor_id)
),

-- ============ TEST A — is each rule principal-invariant? ============
inv AS (
  SELECT count(*) AS regions_compared,
         count(*) FILTER (WHERE a.sal_cur IS DISTINCT FROM b.sal_cur) AS incumbent_differs,
         count(*) FILTER (WHERE a.sal_alt IS DISTINCT FROM b.sal_alt) AS candidate_differs,
         round(max(abs(a.sal_cur - b.sal_cur))::numeric, 4) AS incumbent_max_delta,
         round(max(abs(a.sal_alt - b.sal_alt))::numeric, 4) AS candidate_max_delta
  FROM salnorm a JOIN salnorm b ON b.id = a.id AND a.s = '1-all' AND b.s = '2-map-only'
),

-- ============ TEST B — what does it do to selection? ============
scored AS (
  SELECT sn.id, sn.home_anchor_table, sn.home_anchor_id, q.qi, sn.sal_cur, sn.sal_alt,
         COALESCE(NULLIF(1 - (sn.centroid <=> q.emb), 'NaN'::float8), 0.0) AS query_cos
  FROM salnorm sn CROSS JOIN q
  WHERE sn.s = '1-all'
),
ranked AS (
  SELECT sc.*,
         {ALPHA} * sc.sal_cur + {BETA} * sc.query_cos
           + {KAPPA} * CASE sc.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_cur,
         {ALPHA} * sc.sal_alt + {BETA} * sc.query_cos
           + {KAPPA} * CASE sc.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_alt
  FROM scored sc
),
rr AS (
  SELECT r.*,
         row_number() OVER (PARTITION BY r.qi, r.home_anchor_id
                            ORDER BY r.score_cur DESC, r.id) AS maprank_cur,
         row_number() OVER (PARTITION BY r.qi, r.home_anchor_id
                            ORDER BY r.score_alt DESC, r.id) AS maprank_alt
  FROM ranked r
),
pos AS (
  SELECT rr.*,
         row_number() OVER (PARTITION BY qi ORDER BY maprank_cur, score_cur DESC, id) AS pos_cur,
         row_number() OVER (PARTITION BY qi ORDER BY maprank_alt, score_alt DESC, id) AS pos_alt
  FROM rr
),
sel AS (
  SELECT p.id, p.home_anchor_table, p.home_anchor_id,
         bool_or(p.pos_cur <= w.n) AS won_cur,
         bool_or(p.pos_alt <= w.n) AS won_alt
  FROM pos p CROSS JOIN w GROUP BY 1,2,3
),
vis AS (SELECT resource_id FROM resources_visible_to('{PRINCIPAL}'))
SELECT 'A invariance' AS section,
       'regions='||regions_compared AS k1,
       'INCUMBENT differs='||incumbent_differs AS k2,
       'CANDIDATE differs='||candidate_differs AS k3,
       'max_delta cur='||incumbent_max_delta||' alt='||candidate_max_delta AS k4
FROM inv
UNION ALL
SELECT 'B selection',
       COALESCE(cm.name, cx.name, sel.home_anchor_id::text),
       'live='||count(*)||' cur='||count(*) FILTER (WHERE won_cur)
                        ||' alt='||count(*) FILTER (WHERE won_alt),
       'nodes cur='||(SELECT count(DISTINCT m.member_id) FROM kb_cogmap_region_members m
                       WHERE m.region_id IN (SELECT id FROM sel s2
                                             WHERE s2.won_cur AND s2.home_anchor_id = sel.home_anchor_id)
                         AND m.member_table='kb_resources'
                         AND m.member_id IN (SELECT resource_id FROM vis)),
       'nodes alt='||(SELECT count(DISTINCT m.member_id) FROM kb_cogmap_region_members m
                       WHERE m.region_id IN (SELECT id FROM sel s2
                                             WHERE s2.won_alt AND s2.home_anchor_id = sel.home_anchor_id)
                         AND m.member_table='kb_resources'
                         AND m.member_id IN (SELECT resource_id FROM vis))
FROM sel
LEFT JOIN kb_cogmaps  cm ON sel.home_anchor_table='kb_cogmaps'  AND cm.id = sel.home_anchor_id
LEFT JOIN kb_contexts cx ON sel.home_anchor_table='kb_contexts' AND cx.id = sel.home_anchor_id
GROUP BY sel.home_anchor_id, sel.home_anchor_table, cm.name, cx.name
ORDER BY 1, 2;
""")
