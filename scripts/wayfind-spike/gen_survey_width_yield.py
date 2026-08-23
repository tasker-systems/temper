#!/usr/bin/env python3
"""What does widening `survey`'s funnel do COMPOSITIONALLY? (goal 019fbdb9-f287-79c0-aab6-efa0b1de12c8)

`survey` produces the member RESOURCES of the regions that cleared the cut, and those resources are
what a downstream stage is bounded by. So the funnel width is not a result count — it is the size
and shape of the seed set every following stage inherits.

THE WIDTH IS ALREADY A CALLER KNOB. `survey` admits `BoundTerm::Regions` with a published ceiling of
20 (`registry.rs`); the 3 is a DEFAULT, supplied by the fragment's COALESCE, not a cap. `survey`
DECLINES `limit`, because `limit` means rows and the funnel is a width — so `regions` is the only
lever, and the row count it yields is unbounded by anything else. That is why this is worth
measuring rather than guessing.

THE ROUND-ROBIN MAKES WIDTH STRUCTURAL, NOT JUST BIGGER. `top_regions` orders by
`map_rank ASC, region_score DESC` — every anchor's best region before any anchor's second. So while
`regions_n <= (visible region-bearing anchors)`, the cut is exactly one region per anchor and NO map
can contribute depth. Which anchors those are is decided by each map's single best `region_score`,
which is where the alpha finding (research 01a02e86-1ccd-7770-90f1-1f12487b54fd) lands: at the
default width, salience is choosing WHICH MAPS an asker sees, not which regions within a map.

Measured here, per width in {1,3,5,10,20}, over the 24-query probe set, for both real principals:

  0 — corpus shape: regions per anchor, members per region, and how many members are visible.
  A — what the seed set actually is: distinct maps, regions and VISIBLE member resources per query.
  B — marginal yield: new distinct resources each width step buys over the one below it, which is
      the question "is a wider funnel richer, or the same material re-covered".
  C — the round-robin's structural signature: regions per map at each width. Confirms (or refutes)
      that width <= anchor count is breadth-only.

Visibility is applied as `resources_visible_to(principal)`, so the counts are what an asker would
actually be handed, not what the region holds.

Read-only, one loaded candidate set per statement, statement_timeout set. No DDL, no writes, no temp
tables (a read-only transaction forbids them).

    python3 gen_survey_width_yield.py vectors-24.tsv > survey_width_yield.sql
    ./prod-readonly.sh survey_width_yield.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
PETE = "019d4add-f49d-7c43-a87d-dda470e5dd9c"
AGENT = "019f25a6-230e-7e03-b38e-499e8f29fd81"
ALPHA, BETA, KAPPA = 0.4, 0.6, 0.05
PRIOR_COGMAP, PRIOR_CONTEXT = 1.0, 0.6

rows = []
with open(sys.argv[1]) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        _q, v = line.split("\t", 1)
        rows.append((i, v))

values_sql = ",\n  ".join(
    [f"({rows[0][0]}, '{rows[0][1]}'::vector)"] + [f"({i}, '{v}')" for i, v in rows[1:]]
)
widths_sql = ", ".join(f"({w})" for w in WIDTHS)

# `vis` is the principal's visible resource set, materialized once per statement. The deployed
# survey path hoists exactly this (`__temper_vis`), so counting through it is what an asker gets.
PREAMBLE = f"""WITH
shape(shape, principal, anchor_table, anchor_id) AS (
            SELECT DISTINCT '1-pete (real)',  '{PETE}'::uuid,  a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{PETE}'::uuid) a
  UNION ALL SELECT DISTINCT '2-agent (real)', '{AGENT}'::uuid, a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{AGENT}'::uuid) a
),
vis(shape, member_id) AS (
            SELECT '1-pete (real)',  r FROM resources_visible_to('{PETE}'::uuid)  r
  UNION ALL SELECT '2-agent (real)', r FROM resources_visible_to('{AGENT}'::uuid) r
),
cand AS (
  SELECT sh.shape, r.id, r.centroid, r.home_anchor_table, r.home_anchor_id, r.salience AS sal_eff
  FROM shape sh
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = sh.anchor_table AND r.home_anchor_id = sh.anchor_id
  WHERE NOT r.is_folded
),
salnorm AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.shape, c.home_anchor_table
                                        ORDER BY c.sal_eff) END AS sal_norm
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.shape, c.home_anchor_table)
)"""

SELECTION = f""",
q(qi, emb) AS (VALUES
  {values_sql}
),
ranked AS (
  SELECT sn.shape, sn.id, sn.home_anchor_table, sn.home_anchor_id, q.qi,
         {ALPHA} * sn.sal_norm
       + {BETA}  * COALESCE(NULLIF(1 - (sn.centroid <=> q.emb), 'NaN'::float8), 0.0)
       + {KAPPA} * CASE sn.home_anchor_table WHEN 'kb_cogmaps'
                   THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS region_score
  FROM salnorm sn CROSS JOIN q
),
rr AS (
  SELECT r.*, row_number() OVER (PARTITION BY r.shape, r.qi, r.home_anchor_id
                                 ORDER BY r.region_score DESC, r.id) AS map_rank
  FROM ranked r
),
pos AS (
  SELECT rr.*, row_number() OVER (PARTITION BY rr.shape, rr.qi
                                  ORDER BY rr.map_rank, rr.region_score DESC, rr.id) AS pos
  FROM rr
),
-- The seed set a downstream stage inherits: visible member resources of the regions that cleared
-- the cut, per width.
seed AS (
  SELECT p.shape, p.qi, w.n AS width, p.home_anchor_id, p.id AS region_id, m.member_id
  FROM pos p
  CROSS JOIN (VALUES {widths_sql}) AS w(n)
  JOIN kb_cogmap_region_members m
    ON m.region_id = p.id AND m.member_table = 'kb_resources'
  JOIN vis v ON v.shape = p.shape AND v.member_id = m.member_id
  WHERE p.pos <= w.n
)"""

print(rf"""\set ON_ERROR_STOP on
\timing on
SET statement_timeout = '120s';
\echo '=== survey funnel width: what widening buys COMPOSITIONALLY ==='
\echo '=== probe set: queries-24.txt (24 queries). regions is a caller bound, ceiling 20, default 3. ==='

\echo ''
\echo '=== 0. corpus shape — regions per anchor, and visible members per region ==='
{PREAMBLE}
SELECT c.shape,
       count(DISTINCT c.home_anchor_id) AS anchors,
       count(*) AS regions,
       round(avg(mc.n_visible)::numeric, 1)  AS mean_visible_members_per_region,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY mc.n_visible) AS median_visible_members,
       percentile_disc(0.9) WITHIN GROUP (ORDER BY mc.n_visible) AS p90_visible_members,
       max(mc.n_visible) AS max_visible_members,
       count(*) FILTER (WHERE mc.n_visible = 0) AS regions_with_no_visible_member
FROM cand c
JOIN LATERAL (
  SELECT count(*) AS n_visible
  FROM kb_cogmap_region_members m
  JOIN vis v ON v.shape = c.shape AND v.member_id = m.member_id
  WHERE m.region_id = c.id AND m.member_table = 'kb_resources'
) mc ON TRUE
GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== A. the seed set a downstream stage inherits, per width (mean over 24 queries) ==='
{PREAMBLE}{SELECTION}
SELECT shape, width,
       round(avg(maps)::numeric, 2)      AS mean_maps,
       round(avg(regions)::numeric, 2)   AS mean_regions,
       round(avg(resources)::numeric, 1) AS mean_seed_resources,
       min(resources) AS min_seed, max(resources) AS max_seed
FROM (
  SELECT shape, width, qi,
         count(DISTINCT home_anchor_id) AS maps,
         count(DISTINCT region_id)      AS regions,
         count(DISTINCT member_id)      AS resources
  FROM seed GROUP BY 1,2,3
) s
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== B. marginal yield — new distinct resources each step buys over the width below ==='
{PREAMBLE}{SELECTION},
per AS (
  SELECT shape, width, qi, count(DISTINCT member_id) AS resources
  FROM seed GROUP BY 1,2,3
),
-- resources present at width w but NOT at the previous width, per query
newly AS (
  SELECT a.shape, a.width, a.qi, count(DISTINCT a.member_id) AS new_resources
  FROM seed a
  WHERE NOT EXISTS (
    SELECT 1 FROM seed b
    WHERE b.shape = a.shape AND b.qi = a.qi AND b.member_id = a.member_id
      AND b.width = (SELECT max(w2.n) FROM (VALUES {widths_sql}) AS w2(n) WHERE w2.n < a.width)
  )
  GROUP BY 1,2,3
)
SELECT p.shape, p.width,
       round(avg(p.resources)::numeric, 1) AS mean_total_resources,
       round(avg(COALESCE(n.new_resources, 0))::numeric, 1) AS mean_new_vs_prev_width,
       round((100.0 * avg(COALESCE(n.new_resources, 0)) / NULLIF(avg(p.resources), 0))::numeric, 1)
         AS pct_of_seed_that_is_new
FROM per p LEFT JOIN newly n ON n.shape = p.shape AND n.width = p.width AND n.qi = p.qi
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== C. round-robin signature — regions per map at each width (is width <= anchors breadth-only?) ==='
{PREAMBLE}{SELECTION}
SELECT shape, width,
       round(avg(regions_in_map)::numeric, 2) AS mean_regions_per_map,
       max(regions_in_map) AS max_regions_in_one_map,
       count(*) FILTER (WHERE regions_in_map > 1) AS map_query_pairs_with_depth,
       count(*) AS map_query_pairs
FROM (
  SELECT shape, width, qi, home_anchor_id, count(DISTINCT region_id) AS regions_in_map
  FROM seed GROUP BY 1,2,3,4
) s
GROUP BY 1,2 ORDER BY 1,2;
""")
