#!/usr/bin/env python3
"""What does `count(*)` actually count in `search_vector_candidates`' UNSCOPED branch?

The Stage-2 length-subsidy correction (migration `20260801000010`, research
`019fbbef-f38d-7a61-a0b1-5c64ea4564b6`) shrinks a resource's `vec_norm` toward its mean by
`1 - 1/sqrt(count(*))`, to stop a long document being paid for the order statistic of having many
chunks. In the **scoped** branch `count(*)` is every current chunk of the resource — the true number
of draws, and the right denominator.

In the **unscoped** branch the count is taken over `ann`, i.e. **after** `LIMIT p_k`. That is a
post-selection denominator: every one of a resource's chunks competed for a slot in the global
top-k, and this counts the ones that *won*. Conditioning on winners cannot correct a selection
effect, because the bias being corrected IS the selection — more chunks means more chances the best
one lands.

This probe prints the distribution, so the consequence is measured rather than argued. Measured
2026-08-01 on prod over `queries-24.txt`: **75.5% of resource-query pairs admit exactly one chunk**,
where the shrinkage factor is exactly 0.0000 — so for three resources in four the correction is a
no-op, and it is a no-op precisely on its target case (the long document whose one lucky chunk
placed).

Read-only. `p_k` mirrors `unified_search`'s `vector_k` = 100.

    python3 gen_denominator.py vectors-24.tsv > denominator.sql
    ./prod-readonly.sh denominator.sql
"""
import sys

VECTOR_K = 100

rows = []
with open(sys.argv[1]) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        _query, vec = line.split("\t", 1)
        rows.append((i, vec))

values = [f"({rows[0][0]}, '{rows[0][1]}'::vector)"]
values += [f"({i}, '{v}')" for i, v in rows[1:]]
values_sql = ",\n  ".join(values)

print(f"""\\set ON_ERROR_STOP on
\\echo '=== UNSCOPED branch: how many of a resource''s chunks survive the global top-{VECTOR_K}? ==='
\\echo '(this is the count(*) the shrinkage factor divides by in plain search)'

WITH q(qi, emb) AS (VALUES
  {values_sql}
),
ann AS (
  SELECT q.qi, c.resource_id,
         row_number() OVER (PARTITION BY q.qi ORDER BY c.embedding <=> q.emb) AS rn
  FROM q JOIN kb_chunks c ON c.is_current AND c.embedding IS NOT NULL
),
top AS (SELECT qi, resource_id FROM ann WHERE rn <= {VECTOR_K}),
per_res AS (SELECT qi, resource_id, count(*) AS n_admitted FROM top GROUP BY 1, 2)
SELECT n_admitted,
       count(*) AS resource_query_pairs,
       round(100.0 * count(*) / sum(count(*)) OVER (), 1) AS pct,
       round((1.0 - 1.0 / sqrt(n_admitted::float8))::numeric, 4) AS shrinkage_factor
FROM per_res GROUP BY n_admitted ORDER BY n_admitted;
""")
