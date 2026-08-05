#!/usr/bin/env python3
"""Equivalence guard for gen_peranchor_width.py (task 019fd25e, Phase 1 step 0).

Every figure that probe reports rests on ONE claim: that `pos_cur <= n` reproduces the deployed
`wayfind_region_scores(...).in_top_n`. Reproducing the body verbatim is evidence for that; it is not
the check. This IS the check — it calls the deployed function on prod for the real principal and
compares its `in_top_n` against the modelled cut, per query, over the same 24-query probe set.

Reads shape 1 only ('1-pete-all (real)' = `visible_region_anchors(PETE)` with no anchor filter),
because that is the shape whose modelled inputs the deployed six-arg call can be made to match
exactly: `wayfind_region_scores(PETE, NULL, emb, n, NULL, NULL)`.

`p_lens` is NULL, matching arm `a` (the shipped default -- `sal_eff = r.salience`).

A nonzero `differs` on any row invalidates every selection figure in the sibling probe.

    python3 gen_peranchor_equiv.py vectors-24.tsv > peranchor_equiv.sql
    ./prod-readonly.sh peranchor_equiv.sql
"""
import sys

WIDTHS = [1, 3, 5]
PETE = "019d4add-f49d-7c43-a87d-dda470e5dd9c"
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

# One statement per width: the deployed function takes the width as a scalar argument, so it cannot
# be cross-joined against a width VALUES list without a LATERAL per width anyway. Explicit is clearer.
stmts = []
for n in WIDTHS:
    stmts.append(rf"""
\echo ''
\echo '--- width {n} ---'
WITH q(qi, emb) AS (VALUES
  {values_sql}
),
-- THE INCUMBENT, called not modelled.
deployed AS (
  SELECT q.qi, s.region_id, s.in_top_n
  FROM q
  CROSS JOIN LATERAL wayfind_region_scores(
    '{PETE}'::uuid, NULL::uuid, q.emb, {n}, NULL::varchar, NULL::uuid) s
),
-- THE MODEL, reproduced as in gen_peranchor_width.py: cand -> scored -> ranked -> ranked_rr -> pos.
cand AS (
  SELECT r.id, r.centroid, r.home_anchor_table, r.home_anchor_id, r.salience AS sal_eff
  FROM kb_cogmap_regions r
  JOIN (SELECT DISTINCT a.anchor_table, a.anchor_id
        FROM visible_region_anchors('{PETE}'::uuid) a) v
    ON v.anchor_table = r.home_anchor_table AND v.anchor_id = r.home_anchor_id
  WHERE NOT r.is_folded
),
salnorm AS (
  SELECT c.*,
         CASE WHEN min(c.sal_eff) OVER kindw = max(c.sal_eff) OVER kindw THEN 1.0
              ELSE percent_rank() OVER (PARTITION BY c.home_anchor_table ORDER BY c.sal_eff)
         END AS sal_cur
  FROM cand c
  WINDOW kindw AS (PARTITION BY c.home_anchor_table)
),
scored AS (
  SELECT sn.id, sn.home_anchor_table, sn.home_anchor_id, q.qi, sn.sal_cur,
         COALESCE(NULLIF(1 - (sn.centroid <=> q.emb), 'NaN'::float8), 0.0) AS query_cos
  FROM salnorm sn CROSS JOIN q
),
ranked AS (
  SELECT s.*, {ALPHA} * s.sal_cur + {BETA} * s.query_cos
         + {KAPPA} * CASE s.home_anchor_table WHEN 'kb_cogmaps'
                     THEN {PRIOR_COGMAP} ELSE {PRIOR_CONTEXT} END AS score_cur
  FROM scored s
),
rr AS (
  SELECT r.*, row_number() OVER (PARTITION BY r.qi, r.home_anchor_id
                                 ORDER BY r.score_cur DESC, r.id) AS maprank_cur
  FROM ranked r
),
pos AS (
  SELECT rr.*, row_number() OVER (PARTITION BY rr.qi
                                  ORDER BY rr.maprank_cur, rr.score_cur DESC, rr.id) AS pos_cur
  FROM rr
)
SELECT {n} AS width,
       count(*) AS region_query_pairs,
       count(*) FILTER (WHERE d.in_top_n) AS deployed_selected,
       count(*) FILTER (WHERE p.pos_cur <= {n}) AS modelled_selected,
       count(*) FILTER (WHERE d.in_top_n IS DISTINCT FROM (p.pos_cur <= {n})) AS differs
FROM pos p
JOIN deployed d ON d.qi = p.qi AND d.region_id = p.id;
""")

print(r"""\set ON_ERROR_STOP on
\timing on
\echo '=== EQUIVALENCE GUARD: modelled cut vs deployed wayfind_region_scores.in_top_n ==='
\echo '=== shape 1 (real principal), p_lens NULL, queries-24.txt. `differs` must be 0. ==='
""" + "".join(stmts))
