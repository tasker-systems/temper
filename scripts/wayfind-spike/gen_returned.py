#!/usr/bin/env python3
"""Push the probe set through the FULL wayfind funnel and report what a reader RECEIVES.

`gen_reach.py` stops at scope — the resources the winning regions dereference to. That is the
right level for "is this node routable at all", but it is NOT what a caller gets, and this area has
already been burned by exactly that inference once (98% of scope vs 33% of returned rows). So this
probe runs the second stage too:

    wayfind_scope_ids(principal, lens, emb, n)  ->  unified_search(..., p_scope_ids => that)

**Production caller's arguments, grounded rather than guessed** — `clamp_search_params`
(`temper-services/src/backend/substrate_read.rs:468-471`) defaults `depth` 2 and `limit` 10, and
`default_graph_expand` (`temper-core/src/types/api.rs:39`) is `true`. Everything else is the unset
default: no seed ids, no edge-type filter, no doc-type, no context, offset 0, `seed_only` false.

**Widths sampled, not exhaustive.** {1, 3, 5, 10, 20} — narrow, default, mid, and the `max_n`
ceiling — rather than all 20, because each width costs 24 full FTS+vector+graph searches. Stated
because a "for some query, at some width" claim is weaker over a sample than over the full range.

Read-only: both functions are STABLE.

    python3 gen_returned.py vectors-24.tsv 'Temper — self-cognition' > returned.sql
"""
import sys

WIDTHS = [1, 3, 5, 10, 20]
DEPTH, LIMIT = 2, 10
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
widths_sql = ", ".join(f"({w})" for w in WIDTHS)

print(f"""\\set ON_ERROR_STOP on
\\echo '=== returned rows: {len(rows)} queries x widths {WIDTHS}, depth {DEPTH}, limit {LIMIT} ==='

WITH
q(qi, query, emb) AS (VALUES
    {values_sql}
),
w(n) AS (VALUES {widths_sql}),
ctx AS (
  SELECT (SELECT id FROM kb_cogmaps WHERE name = '{esc_map}') AS map_id,
         (SELECT id FROM kb_cogmap_lenses
           WHERE name='telos-default' AND home_anchor_table IS NULL) AS lens_id
),
-- Stage-1 + scope assembly, then Stage-2 with the production caller's arguments.
hits AS MATERIALIZED (
  SELECT q.qi, w.n, u.resource_id, u.combined_score
  FROM q CROSS JOIN w CROSS JOIN ctx c
  CROSS JOIN LATERAL (
    SELECT array_agg(sid) AS ids
    FROM wayfind_scope_ids('{PRINCIPAL}'::uuid, c.lens_id, q.emb, w.n) sid
  ) sc
  CROSS JOIN LATERAL unified_search(
      '{PRINCIPAL}'::uuid, q.query, q.emb, ARRAY[]::uuid[], {DEPTH}, ARRAY[]::text[],
      NULL, NULL, true, {LIMIT}, 0, sc.ids, false) u
),
-- Which returned resources are homed in THIS map (distilled), vs anywhere else (raw).
homed AS (
  SELECT DISTINCT h.resource_id
  FROM kb_resource_homes h, ctx c
  WHERE h.anchor_table='kb_cogmaps' AND h.anchor_id = c.map_id
),
live_map AS (
  SELECT count(DISTINCT m.member_id) AS n
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  , ctx c
  WHERE r.home_anchor_table='kb_cogmaps' AND r.home_anchor_id=c.map_id
    AND m.member_table='kb_resources'
    AND m.member_id IN (SELECT resource_id FROM resources_visible_to('{PRINCIPAL}'))
),
per_width AS (
  SELECT h.n::text AS k1,
         count(*)::text AS k2,
         count(*) FILTER (WHERE h.resource_id IN (SELECT resource_id FROM homed))::text AS k3,
         round(100.0 * count(*) FILTER (WHERE h.resource_id IN (SELECT resource_id FROM homed))
               / nullif(count(*), 0), 1)::text AS k4,
         count(DISTINCT h.resource_id) FILTER (
              WHERE h.resource_id IN (SELECT resource_id FROM homed))::text AS k5
  FROM hits h GROUP BY h.n
),
overall AS (
  SELECT 'ALL'::text AS k1,
         count(*)::text AS k2,
         count(*) FILTER (WHERE h.resource_id IN (SELECT resource_id FROM homed))::text AS k3,
         round(100.0 * count(*) FILTER (WHERE h.resource_id IN (SELECT resource_id FROM homed))
               / nullif(count(*), 0), 1)::text AS k4,
         count(DISTINCT h.resource_id) FILTER (
              WHERE h.resource_id IN (SELECT resource_id FROM homed))::text AS k5
  FROM hits h
)
SELECT * FROM (
  SELECT '1 per-width' AS section, * FROM per_width
  UNION ALL SELECT '2 overall', * FROM overall
  UNION ALL SELECT '3 denominator', 'live map nodes', (SELECT n FROM live_map)::text, '', '', ''
) t ORDER BY section, k1;

\\echo ''
\\echo 'per-width/overall : width | rows_returned | map_homed_rows | map_share_pct | DISTINCT map nodes ever returned'
""")
