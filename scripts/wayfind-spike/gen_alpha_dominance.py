#!/usr/bin/env python3
"""Does salience DOMINATE the question in `wayfind_region_scores`? (goal 019fbdb9-f287-79c0-aab6-efa0b1de12c8)

The clause `the-question-decides-within-an-act` says a quantity intrinsic to the corpus "may inform
but never dominate an act whose served-by is about the question". `wayfind_region_scores` computes

    region_score = 0.4 * sal_norm + 0.6 * query_cos + 0.05 * prior(kind)

and `survey` takes its region cut on that score (`WHERE s.in_top_n`), so salience decides which
regions are ADMITTED, not merely how they are ordered. Since 2026-08-16 `survey` serves every door,
so this reaches askers.

WHAT HAS AND HAS NOT BEEN MEASURED. Research 019fd275 measured sal_norm's normalization DOMAIN —
per-kind against per-anchor — and found 16.7% re-allocation. That is a different variable. Nothing
has varied ALPHA. This probe does, and only that.

    isolate alpha, hold everything else:  live  = 0.4*sal + 0.6*cos + 0.05*prior
                                          qonly = 0.0*sal + 0.6*cos + 0.05*prior

KAPPA IS HELD, DELIBERATELY. The anchor-kind prior is also corpus-intrinsic, but it is governed by
`no-constant-decides-between-acts` and carries its own recorded caveat. Varying both would confound
which term produced a displacement. So `qonly` is not "the question alone" — it is "the question
plus every corpus term EXCEPT salience", and every figure below is a LOWER bound on corpus
influence, never an upper one.

REAL PRINCIPALS ONLY. 019fd275 used six synthetic reach shapes because its variable was the
normalization domain, for which a restricted anchor set is the treatment. Alpha is not that: a
synthetic principal would report a displacement rate for a corpus no door serves. Both real
region-bearing principals are here and nothing else.

SHIPPED DEFAULT ARM ONLY. `p_lens` is NULL unless the caller passes `--lens`
(crates/temper-cli/src/actions/search.rs), which makes `sal_eff` the stored `r.salience` column.
019fd275 ran both arms and they agreed on every figure.

Tests:
  0 — candidate-set sanity. Must reproduce h2 SS C / 019fd275: pete 444 cogmap + 472 context,
      agent 400 + 438. If it does not, the corpus moved and every figure below is as-of this run.
  A — selection displacement, per width: how many queries change their admitted region set when
      alpha goes to zero, and how many slots differ.
  B — THE SHARP CASE. At the shipped default width, pairs where salience ADMITTED a region whose
      query_cos is strictly LOWER than one it EXCLUDED. This is the clause's failure shape stated
      as a fact rather than as a threshold: a definition of "dominate" is a ruling, but no
      definition wants this to be common.
  C — term headroom. Per query, the spread each term commands across the candidate set. Bounds any
      definition of "dominate" from the mechanism side rather than the outcome side.
  D — where displaced slots land, per anchor.

Everything is computed DIFFERENTIALLY from ONE loaded candidate set, in one statement per test.
Never two runs. Read-only: no DDL, no writes, no temp tables (a read-only transaction forbids them).

    python3 gen_alpha_dominance.py vectors-24.tsv > alpha_dominance.sql
    ./prod-readonly.sh alpha_dominance.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
DEFAULT_WIDTH = 3

# Real principals, from h1-orient.sql SS F / h2-reach-shapes.sql SS A.
PETE = "019d4add-f49d-7c43-a87d-dda470e5dd9c"   # 3 region-bearing cogmaps + 2 contexts
AGENT = "019f25a6-230e-7e03-b38e-499e8f29fd81"  # 1 cogmap + 1 context

# Deployed constants, read from pg_get_functiondef (h3-body.sql) and unchanged in
# migrations/20260731000050_wayfind_per_map_fairness.sql.
ALPHA, BETA, KAPPA = 0.4, 0.6, 0.05
ALPHA_QONLY = 0.0
PRIOR_COGMAP, PRIOR_CONTEXT = 1.0, 0.6

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
# tables AND views, so there is nowhere to hoist it (see README).
# ---------------------------------------------------------------------------------------------
PREAMBLE = f"""WITH
shape(shape, anchor_table, anchor_id) AS (
            SELECT DISTINCT '1-pete (real)',  a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{PETE}'::uuid) a
  UNION ALL SELECT DISTINCT '2-agent (real)', a.anchor_table, a.anchor_id
              FROM visible_region_anchors('{AGENT}'::uuid) a
),
-- `cand`, reproduced verbatim from the deployed body, shipped-default arm (p_lens IS NULL).
cand AS (
  SELECT sh.shape, r.id, r.centroid, r.home_anchor_table, r.home_anchor_id,
         r.salience AS sal_eff
  FROM shape sh
  JOIN kb_cogmap_regions r
    ON r.home_anchor_table = sh.anchor_table AND r.home_anchor_id = sh.anchor_id
  WHERE NOT r.is_folded
),
-- The deployed per-KIND percent_rank, with the min=max => 1.0 cold-start guard carried verbatim.
-- ONE sal_norm: alpha is the variable, not the normalization.
salnorm AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.shape, c.home_anchor_table
                                        ORDER BY c.sal_eff) END AS sal_norm
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.shape, c.home_anchor_table)
)"""

# The selection pipeline, replicating ranked_rr + top_regions for both alpha arms.
SELECTION = f""",
q(qi, emb) AS (VALUES
  {values_sql}
),
scored AS (
  SELECT sn.shape, sn.id, sn.home_anchor_table, sn.home_anchor_id, q.qi, sn.sal_norm,
         -- the deployed NaN guard, verbatim
         COALESCE(NULLIF(1 - (sn.centroid <=> q.emb), 'NaN'::float8), 0.0) AS query_cos
  FROM salnorm sn CROSS JOIN q
),
ranked AS (
  SELECT s.*,
         {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                   THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS prior_term,
         {ALPHA} * s.sal_norm + {BETA} * s.query_cos
           + {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_live,
         {ALPHA_QONLY} * s.sal_norm + {BETA} * s.query_cos
           + {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                       THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_qonly
  FROM scored s
),
-- ranked_rr: row_number() OVER (PARTITION BY home_anchor_id ORDER BY region_score DESC, id)
rr AS (
  SELECT r.*,
         row_number() OVER (PARTITION BY r.shape, r.qi, r.home_anchor_id
                            ORDER BY r.score_live DESC, r.id)  AS maprank_live,
         row_number() OVER (PARTITION BY r.shape, r.qi, r.home_anchor_id
                            ORDER BY r.score_qonly DESC, r.id) AS maprank_qonly
  FROM ranked r
),
-- top_regions: ORDER BY map_rank ASC, region_score DESC NULLS LAST, id LIMIT regions_n
pos AS (
  SELECT rr.*,
         row_number() OVER (PARTITION BY rr.shape, rr.qi
                            ORDER BY rr.maprank_live,  rr.score_live  DESC, rr.id) AS pos_live,
         row_number() OVER (PARTITION BY rr.shape, rr.qi
                            ORDER BY rr.maprank_qonly, rr.score_qonly DESC, rr.id) AS pos_qonly
  FROM rr
)"""

print(rf"""\set ON_ERROR_STOP on
\timing on
\echo '=== alpha dominance: 0.4*sal_norm live vs 0.0 (question + kappa only) ==='
\echo '=== probe set: queries-24.txt (24 queries). NOT comparable with queries.txt (20). ==='
\echo '=== kappa is HELD in both arms, so every figure is a LOWER bound on corpus influence. ==='

\echo ''
\echo '=== 0. candidate-set sanity — must reproduce h2 C / 019fd275 or the corpus moved ==='
{PREAMBLE}
SELECT shape,
       count(*) AS regions,
       count(*) FILTER (WHERE home_anchor_table = 'kb_cogmaps')  AS cogmap_partition,
       count(*) FILTER (WHERE home_anchor_table = 'kb_contexts') AS context_partition,
       count(DISTINCT home_anchor_id) AS anchors,
       count(*) FILTER (WHERE sal_eff IS NULL) AS sal_eff_null
FROM cand GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== A. selection displacement — does dropping alpha change what is ADMITTED ==='
{PREAMBLE}{SELECTION},
w(n) AS (VALUES {widths_sql})
SELECT p.shape, w.n AS width,
       count(*) FILTER (WHERE p.pos_live <= w.n) AS slots,
       count(*) FILTER (WHERE p.pos_live <= w.n AND p.pos_qonly <= w.n) AS agree,
       count(*) FILTER (WHERE p.pos_live <= w.n AND NOT (p.pos_qonly <= w.n)) AS admitted_by_salience,
       count(DISTINCT p.qi) FILTER (WHERE (p.pos_live <= w.n) IS DISTINCT FROM (p.pos_qonly <= w.n))
         AS queries_changed,
       count(DISTINCT p.qi) AS queries_total
FROM pos p CROSS JOIN w
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== B. THE SHARP CASE — salience admitted a region LESS relevant than one it excluded ==='
\echo '===    (width {DEFAULT_WIDTH}; gained = in live, not in qonly; lost = in qonly, not in live) ==='
{PREAMBLE}{SELECTION},
gained AS (SELECT shape, qi, id, query_cos FROM pos
            WHERE pos_live <= {DEFAULT_WIDTH} AND pos_qonly > {DEFAULT_WIDTH}),
lost   AS (SELECT shape, qi, id, query_cos FROM pos
            WHERE pos_qonly <= {DEFAULT_WIDTH} AND pos_live > {DEFAULT_WIDTH})
SELECT g.shape,
       count(*) AS inverted_pairs,
       count(DISTINCT g.qi) AS queries_with_an_inversion,
       round(max(l.query_cos - g.query_cos)::numeric, 4) AS max_cos_gap,
       round(avg(l.query_cos - g.query_cos)::numeric, 4) AS mean_cos_gap
FROM gained g
JOIN lost l ON l.shape = g.shape AND l.qi = g.qi AND l.query_cos > g.query_cos
GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== B2. the same inversions, one row each, worst first ==='
{PREAMBLE}{SELECTION},
gained AS (SELECT shape, qi, id, query_cos, sal_norm FROM pos
            WHERE pos_live <= {DEFAULT_WIDTH} AND pos_qonly > {DEFAULT_WIDTH}),
lost   AS (SELECT shape, qi, id, query_cos, sal_norm FROM pos
            WHERE pos_qonly <= {DEFAULT_WIDTH} AND pos_live > {DEFAULT_WIDTH})
SELECT g.shape, g.qi,
       round(g.query_cos::numeric, 4) AS admitted_cos, round(g.sal_norm::numeric, 4) AS admitted_sal,
       round(l.query_cos::numeric, 4) AS excluded_cos, round(l.sal_norm::numeric, 4) AS excluded_sal,
       round((l.query_cos - g.query_cos)::numeric, 4) AS cos_gap
FROM gained g
JOIN lost l ON l.shape = g.shape AND l.qi = g.qi AND l.query_cos > g.query_cos
ORDER BY cos_gap DESC LIMIT 40;

\echo ''
\echo '=== C. term headroom — the spread each term commands over the candidate set, per query ==='
{PREAMBLE}{SELECTION}
SELECT shape,
       round(avg(sal_span)::numeric, 4)  AS mean_alpha_span,
       round(avg(cos_span)::numeric, 4)  AS mean_beta_span,
       round(max(sal_span)::numeric, 4)  AS max_alpha_span,
       round(min(cos_span)::numeric, 4)  AS min_beta_span,
       count(*) FILTER (WHERE sal_span > cos_span) AS queries_alpha_span_exceeds_beta,
       count(*) AS queries
FROM (
  SELECT shape, qi,
         {ALPHA} * (max(sal_norm)  - min(sal_norm))  AS sal_span,
         {BETA}  * (max(query_cos) - min(query_cos)) AS cos_span
  FROM pos GROUP BY shape, qi
) s
GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== D. where the displaced slots land — per anchor, width {DEFAULT_WIDTH} ==='
{PREAMBLE}{SELECTION}
SELECT p.shape,
       COALESCE(cm.name, cx.name, p.home_anchor_id::text) AS anchor,
       p.home_anchor_table AS kind,
       count(*) FILTER (WHERE p.pos_live  <= {DEFAULT_WIDTH}) AS slots_live,
       count(*) FILTER (WHERE p.pos_qonly <= {DEFAULT_WIDTH}) AS slots_qonly,
       count(*) FILTER (WHERE p.pos_live  <= {DEFAULT_WIDTH})
         - count(*) FILTER (WHERE p.pos_qonly <= {DEFAULT_WIDTH}) AS delta_from_salience
FROM pos p
LEFT JOIN kb_cogmaps  cm ON p.home_anchor_table = 'kb_cogmaps'  AND cm.id = p.home_anchor_id
LEFT JOIN kb_contexts cx ON p.home_anchor_table = 'kb_contexts' AND cx.id = p.home_anchor_id
GROUP BY 1,2,3 ORDER BY 1, abs(count(*) FILTER (WHERE p.pos_live <= {DEFAULT_WIDTH})
                              - count(*) FILTER (WHERE p.pos_qonly <= {DEFAULT_WIDTH})) DESC;
""")
