#!/usr/bin/env python3
"""The `sal_norm` hinge (task 019fd25e): does a region's normalized salience belong to the region,
or to the asker?

`wayfind_region_scores` computes

    CASE WHEN min(sal_eff) OVER kind = max(sal_eff) OVER kind THEN 1.0
         ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff) END

over `cand`, and `cand` joins `vanchors` = `visible_region_anchors(p_principal)`. Reproduced VERBATIM
below from `pg_get_functiondef` on prod (probe `h3-body.sql`), not from the migration file.

So the normalization DOMAIN is the asker's visible set. The candidate alternative is one line —
`PARTITION BY c.home_anchor_table, c.home_anchor_id` — normalizing over the anchor's own live region
set instead.

TWO ARMS, because the shipped default is not the lens path. `lens_id` is `None` unless the caller
passes `--lens` (`temper-cli/src/actions/search.rs:97`), and `p_lens IS NULL` makes `sal_eff` the
stored `r.salience` column, NOT the recomputed telos blend. Arm `a` is therefore what a caller
actually gets; arm `b` is the lens path, run so the conclusion cannot rest on that choice.

EIGHT REACH SHAPES, because the task's standing caveat is that the effect looks small only for a
principal who nearly is their own denominator. Two are REAL principals read straight from
`visible_region_anchors`; six are synthetic restrictions of Pete's anchor set, modelling principals
whose reach is a slice. All six are subsets of shape 1, so every comparison is over shared regions.

Test A — principal invariance. Per shape, against shape 1, on regions present in both: how many
regions' `sal_norm` moves under each rule, and by how much. The incumbent should move; the candidate
should not.

Test B — selection outcome. Over the 24-query probe set at widths {1,3,5,10,20}, replicating the
deployed round-robin (`ranked_rr.map_rank`, then `ORDER BY map_rank, region_score DESC, id LIMIT n`):
how many (query, region) selections the two rules disagree on, and how many queries change their
selected set at all.

Test C — reachable material. Distinct live region members each rule admits at the shipped default
width 3, unioned over the probe set.

Both arms and all shapes are computed DIFFERENTIALLY from ONE loaded candidate set. Never two runs.
Read-only.

    python3 gen_salnorm_reach.py vectors-24.tsv > salnorm_reach.sql
    ./prod-readonly.sh salnorm_reach.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
DEFAULT_WIDTH = 3

# Real principals, from h1-orient.sql §F / h2-reach-shapes.sql §A.
PETE = "019d4add-f49d-7c43-a87d-dda470e5dd9c"   # 3 region-bearing cogmaps + 2 contexts -> 444 / 472
AGENT = "019f25a6-230e-7e03-b38e-499e8f29fd81"  # 1 cogmap + 1 context                  -> 400 / 438

# Deployed constants, read from pg_get_functiondef (h3-body.sql).
ALPHA, BETA, KAPPA = 0.4, 0.6, 0.05
PRIOR_COGMAP, PRIOR_CONTEXT = 1.0, 0.6
TELOS_LENS = "019f04a9-82ac-712a-b298-8e9ac5438943"  # 'telos-default'

vectors_path = sys.argv[1]

rows = []
with open(vectors_path) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        _q, v = line.split("\t", 1)
        rows.append((i, v))

# Only the first literal needs the ::vector cast; the rest inherit the column type.
values_sql = ",\n  ".join(
    [f"({rows[0][0]}, '{rows[0][1]}'::vector)"] + [f"({i}, '{v}')" for i, v in rows[1:]]
)
widths_sql = ", ".join(f"({w})" for w in WIDTHS)

# ---------------------------------------------------------------------------------------------
# Shared preamble, emitted inline into every statement. A read-only transaction forbids temp
# tables AND views (both paid for once already — see README), so there is nowhere to hoist it.
# ---------------------------------------------------------------------------------------------
PREAMBLE = f"""WITH
ids AS (
  SELECT (SELECT id FROM kb_cogmaps  WHERE name = 'Temper — self-cognition')        AS map_temper,
         (SELECT id FROM kb_cogmaps  WHERE name = 'Storyteller System Design')      AS map_story,
         (SELECT id FROM kb_cogmaps  WHERE name = 'Cognitive Maps for Storyteller') AS map_cms,
         (SELECT id FROM kb_contexts WHERE slug = 'temper')                         AS ctx_temper,
         (SELECT id FROM kb_contexts WHERE slug = 'working-agreements')             AS ctx_wa
),
-- DISTINCT is defensive only: the deployed `vanchors` does not dedupe, and measured it has no
-- duplicates (h2 §C's region counts equal the per-anchor sums exactly). A duplicate anchor would
-- double-count regions and silently corrupt every percent_rank below.
shape(shape, anchor_table, anchor_id) AS (
            SELECT DISTINCT '1-pete-all (real)', a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{PETE}'::uuid) a
  UNION ALL SELECT DISTINCT '2-agent (real)',    a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{AGENT}'::uuid) a
  UNION ALL SELECT '3-map-only',    'kb_cogmaps'::varchar,  map_temper FROM ids
  UNION ALL SELECT '4-ctx-only',    'kb_contexts'::varchar, ctx_temper FROM ids
  UNION ALL SELECT '5-narrow-both', 'kb_cogmaps'::varchar,  map_story  FROM ids
  UNION ALL SELECT '5-narrow-both', 'kb_contexts'::varchar, ctx_wa     FROM ids
  UNION ALL SELECT '6-tiny',        'kb_cogmaps'::varchar,  map_cms    FROM ids
  UNION ALL SELECT '7-two-maps',    'kb_cogmaps'::varchar,  map_temper FROM ids
  UNION ALL SELECT '7-two-maps',    'kb_cogmaps'::varchar,  map_story  FROM ids
  UNION ALL SELECT '8-lopsided',    'kb_cogmaps'::varchar,  map_temper FROM ids
  UNION ALL SELECT '8-lopsided',    'kb_cogmaps'::varchar,  map_cms    FROM ids
),
lens AS (SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses WHERE id = '{TELOS_LENS}'::uuid),
arm(sal_source) AS (VALUES ('a-salience (shipped default, p_lens NULL)'),
                           ('b-telos-lens (--lens telos-default)')),
-- `cand`, reproduced verbatim from the deployed body, with both p_lens arms live.
cand AS (
  SELECT sh.shape, ar.sal_source, r.id, r.centroid, r.home_anchor_table, r.home_anchor_id,
         CASE WHEN ar.sal_source LIKE 'a-%' THEN r.salience
              ELSE l.s_telos   * COALESCE(r.telos_alignment, 0)
                 + l.s_ref     * COALESCE(r.reference_standing, 0)
                 + l.s_central * COALESCE(r.centrality, 0)
         END AS sal_eff
  FROM shape sh
  CROSS JOIN arm ar
  CROSS JOIN lens l
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = sh.anchor_table AND r.home_anchor_id = sh.anchor_id
  WHERE NOT r.is_folded
),
-- Both rules side by side off ONE candidate set. The min=max => 1.0 cold-start guard is carried
-- into BOTH arms verbatim; dropping it from the candidate would confound the comparison with a
-- second change.
salnorm AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.shape, c.sal_source, c.home_anchor_table
                                        ORDER BY c.sal_eff) END AS sal_cur,
         CASE WHEN min(c.sal_eff) OVER anchw = max(c.sal_eff) OVER anchw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.shape, c.sal_source, c.home_anchor_table,
                                                     c.home_anchor_id
                                        ORDER BY c.sal_eff) END AS sal_alt
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.shape, c.sal_source, c.home_anchor_table),
         anchw AS (PARTITION BY c.shape, c.sal_source, c.home_anchor_table, c.home_anchor_id)
)"""

# The selection pipeline — shared by tests B and C, which need identical positions.
SELECTION = f""",
q(qi, emb) AS (VALUES
  {values_sql}
),
scored AS (
  SELECT sn.shape, sn.sal_source, sn.id, sn.home_anchor_table, sn.home_anchor_id, q.qi,
         sn.sal_cur, sn.sal_alt,
         -- the deployed NaN guard, verbatim
         COALESCE(NULLIF(1 - (sn.centroid <=> q.emb), 'NaN'::float8), 0.0) AS query_cos
  FROM salnorm sn CROSS JOIN q
),
ranked AS (
  SELECT s.*,
         {ALPHA} * s.sal_cur + {BETA} * s.query_cos
           + {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_cur,
         {ALPHA} * s.sal_alt + {BETA} * s.query_cos
           + {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_alt
  FROM scored s
),
-- ranked_rr: row_number() OVER (PARTITION BY home_anchor_id ORDER BY region_score DESC, id)
rr AS (
  SELECT r.*,
         row_number() OVER (PARTITION BY r.shape, r.sal_source, r.qi, r.home_anchor_id
                            ORDER BY r.score_cur DESC, r.id) AS maprank_cur,
         row_number() OVER (PARTITION BY r.shape, r.sal_source, r.qi, r.home_anchor_id
                            ORDER BY r.score_alt DESC, r.id) AS maprank_alt
  FROM ranked r
),
-- top_regions: ORDER BY map_rank ASC, region_score DESC NULLS LAST, id LIMIT regions_n
pos AS (
  SELECT rr.*,
         row_number() OVER (PARTITION BY rr.shape, rr.sal_source, rr.qi
                            ORDER BY rr.maprank_cur, rr.score_cur DESC, rr.id) AS pos_cur,
         row_number() OVER (PARTITION BY rr.shape, rr.sal_source, rr.qi
                            ORDER BY rr.maprank_alt, rr.score_alt DESC, rr.id) AS pos_alt
  FROM rr
)"""

print(rf"""\set ON_ERROR_STOP on
\timing on
\echo '=== sal_norm hinge: per anchor-KIND (incumbent) vs per ANCHOR (candidate) ==='
\echo '=== probe set: queries-24.txt (24 queries). NOT comparable with queries.txt (20). ==='

\echo ''
\echo '=== 0. shape sizes + sal_eff nullity (sanity: the two real shapes must match h2 C) ==='
{PREAMBLE}
SELECT shape, sal_source,
       count(*) AS regions,
       count(*) FILTER (WHERE home_anchor_table = 'kb_cogmaps')  AS cogmap_partition,
       count(*) FILTER (WHERE home_anchor_table = 'kb_contexts') AS context_partition,
       count(DISTINCT home_anchor_id) AS anchors,
       count(*) FILTER (WHERE sal_eff IS NULL) AS sal_eff_null
FROM cand GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== A. principal invariance — each shape vs 1-pete-all, on regions present in BOTH ==='
{PREAMBLE}
SELECT b.shape, b.sal_source,
       count(*) AS regions_shared,
       count(*) FILTER (WHERE a.sal_cur IS DISTINCT FROM b.sal_cur) AS incumbent_differs,
       count(*) FILTER (WHERE a.sal_alt IS DISTINCT FROM b.sal_alt) AS candidate_differs,
       round(max(abs(a.sal_cur - b.sal_cur))::numeric, 4) AS incumbent_max_delta,
       round(avg(abs(a.sal_cur - b.sal_cur))::numeric, 4) AS incumbent_mean_delta,
       round(max(abs(a.sal_alt - b.sal_alt))::numeric, 4) AS candidate_max_delta
FROM salnorm b
JOIN salnorm a ON a.id = b.id AND a.sal_source = b.sal_source AND a.shape = '1-pete-all (real)'
WHERE b.shape <> '1-pete-all (real)'
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== B. selection outcome — incumbent vs candidate WITHIN each shape, 24 queries ==='
{PREAMBLE}{SELECTION},
w(n) AS (VALUES {widths_sql})
SELECT p.shape, p.sal_source, w.n AS width,
       count(*) FILTER (WHERE p.pos_cur <= w.n) AS slots,
       count(*) FILTER (WHERE p.pos_cur <= w.n AND p.pos_alt <= w.n) AS agree,
       count(*) FILTER (WHERE p.pos_cur <= w.n AND NOT (p.pos_alt <= w.n)) AS only_incumbent,
       count(DISTINCT p.qi) FILTER (WHERE (p.pos_cur <= w.n) IS DISTINCT FROM (p.pos_alt <= w.n))
         AS queries_changed
FROM pos p CROSS JOIN w
GROUP BY 1,2,3 ORDER BY 1,2,3;

\echo ''
\echo '=== C. reachable region members at the shipped default width {DEFAULT_WIDTH}, union over 24 queries ==='
{PREAMBLE}{SELECTION},
sel AS (
  SELECT DISTINCT shape, sal_source, id,
         bool_or(pos_cur <= {DEFAULT_WIDTH}) OVER (PARTITION BY shape, sal_source, id) AS won_cur,
         bool_or(pos_alt <= {DEFAULT_WIDTH}) OVER (PARTITION BY shape, sal_source, id) AS won_alt
  FROM pos
)
-- COUNT REGIONS FROM `sel`, MEMBERS FROM THE JOIN. A `count(*)` over the joined relation counts
-- region-MEMBER pairs, not regions — the two coincide only while every selected region holds
-- exactly one member, which is precisely the thing being measured and so cannot be assumed.
SELECT s.shape, s.sal_source,
       count(DISTINCT s.id) FILTER (WHERE s.won_cur) AS regions_cur,
       count(DISTINCT s.id) FILTER (WHERE s.won_alt) AS regions_alt,
       count(DISTINCT s.id) FILTER (WHERE s.won_cur AND s.won_alt) AS regions_both,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_cur) AS members_cur,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_alt) AS members_alt,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_cur AND s.won_alt) AS members_both
FROM sel s
LEFT JOIN kb_cogmap_region_members m
       ON m.region_id = s.id AND m.member_table = 'kb_resources'
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== D. where the swaps land — per-ANCHOR slot share at width {DEFAULT_WIDTH}, shape 1 (real) ==='
{PREAMBLE}{SELECTION}
SELECT COALESCE(cm.name, cx.name, p.home_anchor_id::text) AS anchor,
       p.home_anchor_table AS kind,
       count(*) FILTER (WHERE p.pos_cur <= {DEFAULT_WIDTH}) AS slots_incumbent,
       count(*) FILTER (WHERE p.pos_alt <= {DEFAULT_WIDTH}) AS slots_candidate,
       count(*) FILTER (WHERE p.pos_alt <= {DEFAULT_WIDTH})
         - count(*) FILTER (WHERE p.pos_cur <= {DEFAULT_WIDTH}) AS delta
FROM pos p
LEFT JOIN kb_cogmaps  cm ON p.home_anchor_table = 'kb_cogmaps'  AND cm.id = p.home_anchor_id
LEFT JOIN kb_contexts cx ON p.home_anchor_table = 'kb_contexts' AND cx.id = p.home_anchor_id
WHERE p.shape = '1-pete-all (real)'
  AND p.sal_source LIKE 'a-%'
  AND (p.pos_cur <= {DEFAULT_WIDTH} OR p.pos_alt <= {DEFAULT_WIDTH})
GROUP BY 1,2 ORDER BY 5;

\echo ''
\echo '=== E. the width-1 flips, named — shape 1 (real), shipped-default arm ==='
{PREAMBLE}{SELECTION}
SELECT c.qi,
       COALESCE(cmc.name, cxc.name) AS incumbent_top_anchor,
       COALESCE(cma.name, cxa.name) AS candidate_top_anchor,
       round(c.sal_cur::numeric, 4) AS incumbent_top_sal_cur,
       round(a.sal_alt::numeric, 4) AS candidate_top_sal_alt
FROM pos c
JOIN pos a ON a.shape = c.shape AND a.sal_source = c.sal_source AND a.qi = c.qi AND a.pos_alt = 1
LEFT JOIN kb_cogmaps  cmc ON c.home_anchor_table = 'kb_cogmaps'  AND cmc.id = c.home_anchor_id
LEFT JOIN kb_contexts cxc ON c.home_anchor_table = 'kb_contexts' AND cxc.id = c.home_anchor_id
LEFT JOIN kb_cogmaps  cma ON a.home_anchor_table = 'kb_cogmaps'  AND cma.id = a.home_anchor_id
LEFT JOIN kb_contexts cxa ON a.home_anchor_table = 'kb_contexts' AND cxa.id = a.home_anchor_id
WHERE c.shape = '1-pete-all (real)' AND c.sal_source LIKE 'a-%'
  AND c.pos_cur = 1 AND a.id <> c.id
ORDER BY c.qi;
""")
