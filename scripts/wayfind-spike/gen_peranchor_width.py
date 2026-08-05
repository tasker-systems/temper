#!/usr/bin/env python3
"""Phase 1 step 0 (task 019fd25e): does a pooled competition survive the split — and if the global
width dissolves, what is left with a job?

THE STRUCTURAL READING THIS PROBE TESTS. In the deployed `wayfind_region_scores`
(`20260731000050`, reproduced verbatim below from the same `pg_get_functiondef` body `h3-body.sql`
prints), `p_regions_n` reaches EXACTLY ONE place:

    top_regions AS ((SELECT id FROM ranked_rr
                      ORDER BY map_rank ASC, region_score DESC NULLS LAST, id
                      LIMIT (SELECT regions_n FROM n)) UNION ...)

Everything upstream — `cand`, `scored` (sal_norm), `ranked` (region_score), `ranked_rr` (map_rank) —
is computed over the FULL candidate set, independent of the width. Three consequences, and this
probe is here to make each of them a measurement rather than an argument:

  1. `ranked_rr.map_rank` IS the per-anchor rank. A per-anchor width is `WHERE map_rank <= n`, not
     new machinery. So "per-anchor width" is modelled here by reusing the deployed row_number()
     verbatim, never by a re-derived expression.
  2. Round-robin has no job once the width is per-anchor: `map_rank` exists solely to interleave
     maps inside ONE global LIMIT.
  3. Per-kind `percent_rank` DOES still have a job — it is upstream of the width, still feeds
     `region_score`, and `region_score` still orders regions WITHIN an anchor. Both rules are
     monotone in `sal_eff` within an anchor, but `region_score` SUMS 0.4*sal_norm with
     0.6*query_cos, and a sum is not order-preserving under a monotone reparametrization of one
     term. Whether that changes real selections is test B.

TESTS

  A — scope cost. The number Phase 1 asked for. Per shape/arm/width: regions and distinct live
      members admitted under the deployed GLOBAL width vs a PER-ANCHOR width of the same n.
      This is the cost side: with k region-bearing anchors a width of n becomes up to n*k regions.

  B — does the sal_norm re-allocation survive the frame change? Under a PER-ANCHOR width, how many
      (query, region) selections do the incumbent and candidate sal_norm rules disagree on? The
      hinge measured 16.7% of slots at the shipped default; that figure is a property of the POOLED
      competition. This is the same disagreement re-measured under the per-anchor frame.

  C — the kappa bite. Asserted: kappa cannot change a WITHIN-anchor ordering, because it is a
      function of `home_anchor_table` alone and is therefore a constant added to every region of one
      anchor. A probe that only showed "0 differences under per-anchor width" would be satisfied by
      a kappa that is inert EVERYWHERE, so it is run in BOTH frames: under the global width it must
      DIFFER (the witness that the probe can see kappa at all), and under the per-anchor width it
      must not. One conjunct each, in one table.

  D — the shutout. `Storyteller System Design` takes 1 slot in 72 under the incumbent global frame
      (hinge research 019fd275). Per-anchor slot share under both frames, so the mechanism's fate
      under the split is read off rather than inferred.

WHAT THIS DOES NOT MODEL, stated so its absence is not read as a null result: the cold-start arm
(`direct_ids` in `wayfind_scope_reach` — region-less anchors' homed resources). It is independent of
the width and unchanged by every frame here, so it adds a constant to every scope figure in test A.
The members counted here are the REGION arm only.

Both arms (p_lens NULL / telos lens) and all eight reach shapes are computed DIFFERENTIALLY from ONE
loaded candidate set, as in gen_salnorm_reach.py. Read-only.

    python3 gen_peranchor_width.py vectors-24.tsv > peranchor_width.sql
    ./prod-readonly.sh peranchor_width.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
DEFAULT_WIDTH = 3

# Real principals, from h1-orient.sql section F / h2-reach-shapes.sql section A.
PETE = "019d4add-f49d-7c43-a87d-dda470e5dd9c"   # 3 region-bearing cogmaps + 2 contexts
AGENT = "019f25a6-230e-7e03-b38e-499e8f29fd81"  # 1 cogmap + 1 context

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
# tables AND views (both paid for once already -- see README), so there is nowhere to hoist it.
# Identical to gen_salnorm_reach.py's, deliberately: test B here is that probe's test B re-run
# under a different width frame, and a differing candidate set would confound the comparison.
# ---------------------------------------------------------------------------------------------
PREAMBLE = f"""WITH
ids AS (
  SELECT (SELECT id FROM kb_cogmaps  WHERE name = 'Temper — self-cognition')        AS map_temper,
         (SELECT id FROM kb_cogmaps  WHERE name = 'Storyteller System Design')      AS map_story,
         (SELECT id FROM kb_cogmaps  WHERE name = 'Cognitive Maps for Storyteller') AS map_cms,
         (SELECT id FROM kb_contexts WHERE slug = 'temper')                         AS ctx_temper,
         (SELECT id FROM kb_contexts WHERE slug = 'working-agreements')             AS ctx_wa
),
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

# The selection pipeline. THREE scores, not two: incumbent sal_norm, candidate sal_norm, and
# incumbent-sal_norm-with-kappa-zeroed (test C's arm).
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
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_alt,
         -- kappa zeroed, incumbent sal_norm otherwise identical (test C)
         {ALPHA} * s.sal_cur + {BETA} * s.query_cos AS score_nok
  FROM scored s
),
-- `maprank_*` is the deployed `ranked_rr.map_rank`, verbatim:
--   row_number() OVER (PARTITION BY home_anchor_id ORDER BY region_score DESC, id)
-- It is ALSO exactly the per-anchor width position, which is the point: `maprank <= n` needs no
-- new expression, it reads the rank the deployed function already computes and then discards.
rr AS (
  SELECT r.*,
         row_number() OVER (PARTITION BY r.shape, r.sal_source, r.qi, r.home_anchor_id
                            ORDER BY r.score_cur DESC, r.id) AS maprank_cur,
         row_number() OVER (PARTITION BY r.shape, r.sal_source, r.qi, r.home_anchor_id
                            ORDER BY r.score_alt DESC, r.id) AS maprank_alt,
         row_number() OVER (PARTITION BY r.shape, r.sal_source, r.qi, r.home_anchor_id
                            ORDER BY r.score_nok DESC, r.id) AS maprank_nok
  FROM ranked r
),
-- `pos_*` is the deployed global cut: ORDER BY map_rank ASC, region_score DESC NULLS LAST, id
-- then LIMIT regions_n. `pos <= n` is the GLOBAL width; `maprank <= n` is the PER-ANCHOR width.
pos AS (
  SELECT rr.*,
         row_number() OVER (PARTITION BY rr.shape, rr.sal_source, rr.qi
                            ORDER BY rr.maprank_cur, rr.score_cur DESC, rr.id) AS pos_cur,
         row_number() OVER (PARTITION BY rr.shape, rr.sal_source, rr.qi
                            ORDER BY rr.maprank_alt, rr.score_alt DESC, rr.id) AS pos_alt,
         row_number() OVER (PARTITION BY rr.shape, rr.sal_source, rr.qi
                            ORDER BY rr.maprank_nok, rr.score_nok DESC, rr.id) AS pos_nok
  FROM rr
)"""

print(rf"""\set ON_ERROR_STOP on
\timing on
\echo '=== per-anchor width vs global width: what still has a job after the split ==='
\echo '=== probe set: queries-24.txt (24 queries). NOT comparable with queries.txt (20). ==='
\echo '=== region arm ONLY -- the cold-start arm (direct_ids) is width-independent, see header ==='

\echo ''
\echo '=== 0. anchors per shape (the multiplier: per-anchor width n admits up to n*anchors) ==='
{PREAMBLE}
SELECT shape, sal_source,
       count(*) AS regions,
       count(DISTINCT home_anchor_id) AS anchors,
       count(DISTINCT home_anchor_id) FILTER (WHERE home_anchor_table = 'kb_cogmaps')  AS cogmaps,
       count(DISTINCT home_anchor_id) FILTER (WHERE home_anchor_table = 'kb_contexts') AS contexts
FROM cand GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== A. SCOPE COST -- global width vs per-anchor width, same n. Regions and live members. ==='
{PREAMBLE}{SELECTION},
w(n) AS (VALUES {widths_sql}),
sel AS (
  SELECT DISTINCT p.shape, p.sal_source, w.n, p.id,
         bool_or(p.pos_cur     <= w.n) OVER (PARTITION BY p.shape, p.sal_source, w.n, p.id) AS won_global,
         bool_or(p.maprank_cur <= w.n) OVER (PARTITION BY p.shape, p.sal_source, w.n, p.id) AS won_anchor
  FROM pos p CROSS JOIN w
)
-- Count REGIONS from `sel`, MEMBERS from the join: a count(*) over the joined relation counts
-- region-MEMBER pairs, not regions (README gotcha).
SELECT s.shape, s.sal_source, s.n AS width,
       count(DISTINCT s.id) FILTER (WHERE s.won_global) AS regions_global,
       count(DISTINCT s.id) FILTER (WHERE s.won_anchor) AS regions_anchor,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_global) AS members_global,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_anchor) AS members_anchor,
       count(DISTINCT m.member_id) FILTER (WHERE s.won_anchor)
         - count(DISTINCT m.member_id) FILTER (WHERE s.won_global) AS members_delta
FROM sel s
LEFT JOIN kb_cogmap_region_members m
       ON m.region_id = s.id AND m.member_table = 'kb_resources'
GROUP BY 1,2,3 ORDER BY 1,2,3;

\echo ''
\echo '=== B. does the sal_norm re-allocation survive? incumbent vs candidate, PER-ANCHOR width ==='
\echo '===    (global-width columns are the hinge probe re-run, so the two frames sit side by side) ==='
{PREAMBLE}{SELECTION},
w(n) AS (VALUES {widths_sql})
SELECT p.shape, p.sal_source, w.n AS width,
       count(*) FILTER (WHERE p.maprank_cur <= w.n) AS anchor_slots,
       count(*) FILTER (WHERE p.maprank_cur <= w.n AND NOT (p.maprank_alt <= w.n))
         AS anchor_only_incumbent,
       count(DISTINCT p.qi) FILTER (WHERE (p.maprank_cur <= w.n) IS DISTINCT FROM (p.maprank_alt <= w.n))
         AS anchor_queries_changed,
       count(*) FILTER (WHERE p.pos_cur <= w.n) AS global_slots,
       count(*) FILTER (WHERE p.pos_cur <= w.n AND NOT (p.pos_alt <= w.n))
         AS global_only_incumbent,
       count(DISTINCT p.qi) FILTER (WHERE (p.pos_cur <= w.n) IS DISTINCT FROM (p.pos_alt <= w.n))
         AS global_queries_changed
FROM pos p CROSS JOIN w
GROUP BY 1,2,3 ORDER BY 1,2,3;

\echo ''
\echo '=== C. THE KAPPA BITE. Asserted: kappa cannot change a WITHIN-anchor ordering. ==='
\echo '===    global_kappa_differs MUST be > 0 somewhere (else the probe cannot see kappa at all) ==='
\echo '===    anchor_kappa_differs MUST be 0 everywhere (the assertion) ==='
{PREAMBLE}{SELECTION},
w(n) AS (VALUES {widths_sql})
SELECT p.shape, p.sal_source, w.n AS width,
       count(*) FILTER (WHERE (p.maprank_cur <= w.n) IS DISTINCT FROM (p.maprank_nok <= w.n))
         AS anchor_kappa_differs,
       count(*) FILTER (WHERE (p.pos_cur <= w.n) IS DISTINCT FROM (p.pos_nok <= w.n))
         AS global_kappa_differs
FROM pos p CROSS JOIN w
GROUP BY 1,2,3 ORDER BY 1,2,3;

\echo ''
\echo '=== D. THE SHUTOUT -- per-anchor slot share at width {DEFAULT_WIDTH}, shape 1, shipped-default arm ==='
{PREAMBLE}{SELECTION}
SELECT COALESCE(cm.name, cx.name, p.home_anchor_id::text) AS anchor,
       p.home_anchor_table AS kind,
       count(*) FILTER (WHERE p.pos_cur     <= {DEFAULT_WIDTH}) AS slots_global,
       count(*) FILTER (WHERE p.maprank_cur <= {DEFAULT_WIDTH}) AS slots_per_anchor,
       -- the top-of-scale question: best sal_norm this anchor achieves under each rule
       round(max(p.sal_cur)::numeric, 4) AS best_sal_incumbent,
       round(max(p.sal_alt)::numeric, 4) AS best_sal_candidate
FROM pos p
LEFT JOIN kb_cogmaps  cm ON p.home_anchor_table = 'kb_cogmaps'  AND cm.id = p.home_anchor_id
LEFT JOIN kb_contexts cx ON p.home_anchor_table = 'kb_contexts' AND cx.id = p.home_anchor_id
WHERE p.shape = '1-pete-all (real)'
  AND p.sal_source LIKE 'a-%'
GROUP BY 1,2 ORDER BY 3;
""")
