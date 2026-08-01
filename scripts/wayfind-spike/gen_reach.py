#!/usr/bin/env python3
"""Emit the region-reachability probe for one database, from a vectors TSV.

Answers, for the named map, over every query at every width 1..MAXN:
  1. how many distinct regions are EVER selected
  2. how many live member resources those regions dereference to (the reachable set)
  3. the same bucketed by region size, so "small regions cannot win" is visible rather than asserted
  4. the sal_norm ladder, i.e. what eligibility costs a small region

Two scopings are emitted and they mean different things:
  * MAP-SCOPED  — `wayfind_region_scores(..., 'kb_cogmaps', <map>)`. The map's own router, with no
    sibling anchor competing. This is the primary measure for goal 019fbb77: a region that cannot
    win even here is unreachable for reasons internal to formation, not because a sibling map beat
    it. Cross-map competition is goal 019fb559's subject and must not be conflated with this one.
  * UNSCOPED    — every anchor the principal can see, which is what a default caller receives.

**One statement, materialized CTEs.** `default_transaction_read_only = on` forbids temp tables
(measured, not assumed — `CREATE TABLE in a read-only transaction`), and the probe set is 24 queries
x 20 widths x 2 scopings, so re-deriving it per report block would triple the work. Every block
therefore hangs off one MATERIALIZED CTE and the output is a labelled union with text columns.

Read-only: `wayfind_region_scores` is STABLE and nothing here writes.

    python3 gen_reach.py vectors-24.tsv 'Temper — self-cognition' > reach.sql
"""
import sys

MAXN = 20
PRINCIPAL = "019d4add-f49d-7c43-a87d-dda470e5dd9c"

vectors_path, map_name = sys.argv[1], sys.argv[2]
esc_map = map_name.replace("'", "''")

rows = []
with open(vectors_path) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        query, vec = line.split("\t", 1)
        rows.append((i, query.replace("'", "''"), vec))

first = rows[0]
values = [f"({first[0]}, '{first[1]}'::text, '{first[2]}'::vector)"]
values += [f"({i}, '{q}', '{v}')" for i, q, v in rows[1:]]
values_sql = ",\n    ".join(values)

BUCKET = """CASE WHEN {c} <= 1 THEN '1 singleton'
                 WHEN {c} <= 3 THEN '2 two-three'
                 WHEN {c} <= 9 THEN '3 four-nine'
                 ELSE '4 ten-plus' END"""

print(f"""\\set ON_ERROR_STOP on
\\echo '=== reachability: {len(rows)} queries x widths 1..{MAXN}, map {map_name!r} ==='

WITH
q(qi, query, emb) AS (VALUES
    {values_sql}
),
w(n) AS (SELECT generate_series(1, {MAXN})),
ctx AS (
  SELECT (SELECT id FROM kb_cogmaps WHERE name = '{esc_map}') AS map_id,
         (SELECT id FROM kb_cogmap_lenses
           WHERE name='telos-default' AND home_anchor_table IS NULL) AS lens_id
),
sel_scoped AS MATERIALIZED (
  SELECT q.qi, w.n, s.region_id, s.in_top_n, s.sal_norm, s.query_cos, s.region_score
  FROM q CROSS JOIN w CROSS JOIN ctx c
  CROSS JOIN LATERAL wayfind_region_scores(
      '{PRINCIPAL}'::uuid, c.lens_id, q.emb, w.n, 'kb_cogmaps', c.map_id) s
),
sel_open AS MATERIALIZED (
  SELECT s.region_id, s.in_top_n
  FROM q CROSS JOIN w CROSS JOIN ctx c
  CROSS JOIN LATERAL wayfind_region_scores(
      '{PRINCIPAL}'::uuid, c.lens_id, q.emb, w.n) s
  WHERE s.home_anchor_table='kb_cogmaps' AND s.home_anchor_id = c.map_id
),
live AS (
  SELECT r.id, r.member_count
  FROM kb_cogmap_regions r, ctx c
  WHERE r.home_anchor_table='kb_cogmaps' AND r.home_anchor_id=c.map_id AND NOT r.is_folded
),
vis AS (SELECT resource_id FROM resources_visible_to('{PRINCIPAL}')),
ever_scoped AS (SELECT DISTINCT region_id FROM sel_scoped WHERE in_top_n),
ever_open   AS (SELECT DISTINCT region_id FROM sel_open   WHERE in_top_n),
deref AS (
  SELECT 'map-scoped' AS scoping, count(DISTINCT m.member_id) AS nodes
  FROM kb_cogmap_region_members m
  WHERE m.region_id IN (SELECT region_id FROM ever_scoped)
    AND m.member_table='kb_resources' AND m.member_id IN (SELECT resource_id FROM vis)
  UNION ALL
  SELECT 'unscoped', count(DISTINCT m.member_id)
  FROM kb_cogmap_region_members m
  WHERE m.region_id IN (SELECT region_id FROM ever_open)
    AND m.member_table='kb_resources' AND m.member_id IN (SELECT resource_id FROM vis)
),
headline AS (
  SELECT '1 headline' AS section, d.scoping AS k1,
         (SELECT count(*) FROM live)::text AS k2,
         CASE d.scoping WHEN 'map-scoped' THEN (SELECT count(*) FROM ever_scoped)
                        ELSE (SELECT count(*) FROM ever_open) END::text AS k3,
         (SELECT count(DISTINCT m.member_id) FROM kb_cogmap_region_members m
           WHERE m.region_id IN (SELECT id FROM live) AND m.member_table='kb_resources'
             AND m.member_id IN (SELECT resource_id FROM vis))::text AS k4,
         d.nodes::text AS k5
  FROM deref d
),
by_size AS (
  SELECT '2 by-size' AS section,
         {BUCKET.format(c='l.member_count')} AS k1,
         count(*)::text AS k2,
         count(*) FILTER (WHERE l.id IN (SELECT region_id FROM ever_scoped))::text AS k3,
         coalesce(round(max(b.best)::numeric, 4)::text, '-') AS k4,
         ''::text AS k5
  FROM live l
  LEFT JOIN (SELECT region_id, max(region_score) AS best FROM sel_scoped GROUP BY 1) b
         ON b.region_id = l.id
  GROUP BY 1, 2
),
ladder AS (
  SELECT '3 ladder' AS section,
         {BUCKET.format(c='l.member_count')} AS k1,
         round(avg(s.sal_norm)::numeric, 4)::text AS k2,
         round(avg(s.query_cos)::numeric, 4)::text AS k3,
         round(max(s.query_cos)::numeric, 4)::text AS k4,
         round(max(s.region_score)::numeric, 4)::text AS k5
  FROM live l JOIN sel_scoped s ON s.region_id = l.id AND s.n = {MAXN}
  GROUP BY 1, 2
)
SELECT * FROM (
  SELECT * FROM headline
  UNION ALL SELECT * FROM by_size
  UNION ALL SELECT * FROM ladder
) t
ORDER BY section, k1;

\\echo ''
\\echo 'headline : scoping | live_regions | regions_ever_selected | live_nodes | nodes_reachable'
\\echo 'by-size  : bucket  | regions | ever_selected | max_region_score'
\\echo 'ladder   : bucket  | mean_sal_norm | mean_query_cos | max_query_cos | max_region_score'
""")
