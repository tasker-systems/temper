-- Task 6's decision gate: does routing /api/search through an ungated core REGRESS it?
--
-- Spec §5 splits each arm into a gated wrapper and an `__temper_ungated_` core taking the visible
-- set, so a composition computes visibility ONCE instead of once per stage. A CTE cannot be passed
-- to a function, so the core must receive that set as `uuid[]`. The full chain becomes
-- `search_exact` -> `query_find_exact` -> `__temper_ungated_find_exact`, which puts /api/search on
-- an ARRAY path where it has a JOIN today — and PR #659 established that `= ANY(...)` forms no
-- equivalence class, which is that hazard run in reverse.
--
-- So the question is narrow and answerable: over the same corpus, is joining `unnest($1::uuid[])`
-- within noise of joining `resources_visible_to(p)`?
--
-- `unnest`, NOT `= ANY` — spec §5 is explicit, because `= ANY` is the form that forms no
-- equivalence class and cannot propagate.
--
-- Usage: psql "$DATABASE_URL" -X -f scripts/measure/gate-shape-comparison.sql
-- Expects `cargo make seed-corpus 20`.

\set ON_ERROR_STOP on
\pset pager off
\timing off

SELECT id AS principal FROM kb_profiles WHERE handle = 'ana' \gset

-- Both shapes narrow to the same rows by construction: the array IS the gate's output. Confirmed
-- rather than assumed, because a comparison between two queries returning different row sets would
-- be meaningless.
\echo '=== row-set equality (must be t, or the two shapes are not comparable) ==='
SELECT (SELECT count(*) FROM resources_visible_to(:'principal')) AS via_join,
       (SELECT count(*) FROM unnest(ARRAY(SELECT resource_id FROM resources_visible_to(:'principal')))) AS via_array,
       (SELECT count(*) FROM resources_visible_to(:'principal'))
       = (SELECT count(*) FROM unnest(ARRAY(SELECT resource_id FROM resources_visible_to(:'principal')))) AS equal;

\echo ''
\echo '=== SHAPE A — the incumbent: JOIN resources_visible_to(p) ==='
EXPLAIN (ANALYZE, BUFFERS, TIMING ON, COSTS OFF)
SELECT r.id
  FROM kb_resource_search_index si
  JOIN kb_resources_live r                  ON r.id = si.resource_id
  JOIN resources_visible_to(:'principal') v ON v.resource_id = r.id
 WHERE si.search_vector @@ websearch_to_tsquery('english', 'postgres');

\echo ''
\echo '=== SHAPE B — the ungated core: JOIN unnest($1::uuid[]) ==='
-- The array is materialized OUTSIDE the timed statement in the real design too: the gated wrapper
-- computes it once and passes it in. Building it inline here would time the gate twice and measure
-- the wrong thing.
SELECT ARRAY(SELECT resource_id FROM resources_visible_to(:'principal'))::text AS vis \gset

EXPLAIN (ANALYZE, BUFFERS, TIMING ON, COSTS OFF)
SELECT r.id
  FROM kb_resource_search_index si
  JOIN kb_resources_live r    ON r.id = si.resource_id
  JOIN unnest(:'vis'::uuid[]) AS v(id) ON v.id = r.id
 WHERE si.search_vector @@ websearch_to_tsquery('english', 'postgres');
