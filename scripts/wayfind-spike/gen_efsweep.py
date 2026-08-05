#!/usr/bin/env python3
"""Sweep hnsw.ef_search over the 20-query set: admission and latency at each setting.

Same LATERAL/parameterized form as gen_recall.py, so the index engages exactly as it does
for the deployed `search_vector_candidates`. Reports, per ef: chunks drawn, distinct
resources admitted after the production visibility/active/complete predicates, and wall time.
"""
import sys
PRINCIPAL = "019d4add-f49d-7c43-a87d-dda470e5dd9c"
K = 100
EFS = [40, 64, 100, 200, 400, 800]

rows = []
for line in open(sys.argv[1]):
    line = line.rstrip("\n")
    if line:
        q, v = line.split("\t", 1)
        rows.append(v)
values = ",\n    ".join(f"({i}, '{v}'::vector)" for i, v in enumerate(rows, 1))

print("\\set ON_ERROR_STOP on")
print("\\timing on")
print("SELECT '[1,2]'::vector <=> '[1,3]'::vector AS warmup \\gset w")
for ef in EFS:
    print(f"\n\\echo '=== ef_search = {ef} ==='")
    print(f"SET hnsw.ef_search = {ef};")
    print(f"""WITH q(qi, emb) AS (VALUES
    {values}
),
drawn AS (
  SELECT t.qi, t.chunk_id, t.resource_id
    FROM q CROSS JOIN LATERAL (
      SELECT q.qi, c.id AS chunk_id, c.resource_id
        FROM kb_chunks c WHERE c.is_current
       ORDER BY c.embedding <=> q.emb LIMIT {K}) t
),
adm AS (
  SELECT d.qi, d.resource_id
    FROM drawn d
    JOIN kb_resources r ON r.id = d.resource_id AND r.is_active AND r.ingest_state = 'complete'
    JOIN resources_visible_to('{PRINCIPAL}'::uuid) v ON v.resource_id = d.resource_id
)
SELECT {ef} AS ef_search,
       count(*) AS chunks_total,
       round(count(*)::numeric / 20, 1) AS chunks_per_query,
       (SELECT round(count(*)::numeric / 20, 1) FROM (SELECT DISTINCT qi, resource_id FROM adm) x)
         AS admitted_per_query
  FROM drawn;""")
print("\nRESET hnsw.ef_search;")
