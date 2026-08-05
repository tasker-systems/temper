#!/usr/bin/env python3
"""Per-query candidate admission: HNSW (what production runs) vs exact scan.

Reproduces the UNSCOPED branch of the deployed `search_vector_candidates` verbatim
(migrations/20260801000010_stage2_length_subsidy_kind_blind.sql:145-160):

    ann AS (SELECT c.resource_id, (c.embedding <=> p_emb) FROM kb_chunks c
             WHERE p_emb IS NOT NULL AND c.is_current
             ORDER BY c.embedding <=> p_emb LIMIT p_k)
    admitted AS (SELECT DISTINCT a.resource_id FROM ann a
                   JOIN kb_resources r ON r.id=a.resource_id AND r.is_active
                                      AND r.ingest_state='complete'
                   JOIN resources_visible_to(p_principal) v ON v.resource_id=a.resource_id)

The ORDER BY sits inside a LATERAL so the query vector is a correlated parameter and the
HNSW index engages -- the arrangement the task brief requires ("select candidates the way
production does"). Emits one row per (query, chunk) with a visibility flag; every statistic
is computed in Python from that, so the prod read happens once per arm.

    python3 gen_recall.py vectors-20.tsv hnsw  > recall_hnsw.sql
    python3 gen_recall.py vectors-20.tsv exact > recall_exact.sql
"""
import sys

PRINCIPAL = "019d4add-f49d-7c43-a87d-dda470e5dd9c"
K = 100

vectors_path, arm = sys.argv[1], sys.argv[2]
assert arm in ("hnsw", "exact"), arm

rows = []
with open(vectors_path) as fh:
    for i, line in enumerate(fh, start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        query, vec = line.split("\t", 1)
        rows.append((i, vec))

values = ",\n    ".join(f"({i}, '{v}'::vector)" for i, v in rows)

# The exact arm must defeat the index; the hnsw arm must use it.
prelude = ("SET enable_indexscan = off;\nSET enable_bitmapscan = off;"
           if arm == "exact" else "-- hnsw arm: index left enabled, ef_search at its default")

print(f"""\\set ON_ERROR_STOP on
SELECT '[1,2]'::vector <=> '[1,3]'::vector AS warmup \\gset ignore
{prelude}

\\echo '=== ARM={arm} k={K} :: qi|chunk_id|resource_id|visible ==='
WITH q(qi, emb) AS (VALUES
    {values}
)
SELECT t.qi,
       t.chunk_id,
       t.resource_id,
       (r.id IS NOT NULL AND v.resource_id IS NOT NULL) AS visible
  FROM q
  CROSS JOIN LATERAL (
        SELECT q.qi, c.id AS chunk_id, c.resource_id
          FROM kb_chunks c
         WHERE c.is_current
         ORDER BY c.embedding <=> q.emb
         LIMIT {K}
  ) t
  LEFT JOIN kb_resources r
         ON r.id = t.resource_id AND r.is_active AND r.ingest_state = 'complete'
  LEFT JOIN resources_visible_to('{PRINCIPAL}'::uuid) v
         ON v.resource_id = t.resource_id
 ORDER BY t.qi;
""")
