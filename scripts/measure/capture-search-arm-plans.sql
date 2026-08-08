-- Capture the query plans of both search arms, on all three shapes they take.
--
-- Committed rather than throwaway because its whole purpose is to be run TWICE — once before a
-- refactor of the arms' interiority and once after — and a comparison is only as good as the
-- guarantee that both sides ran the identical statement against the identical corpus. A pair of
-- hand-typed psql sessions cannot give that guarantee; this file can.
--
-- Usage:
--   psql "$DATABASE_URL" -X -f scripts/measure/capture-search-arm-plans.sql > before.txt
--   ... apply the migration ...
--   psql "$DATABASE_URL" -X -f scripts/measure/capture-search-arm-plans.sql > after.txt
--   diff before.txt after.txt
--
-- Expects the measurement corpus (`cargo make seed-corpus 20`). On a near-empty corpus every arm
-- seq-scans and the capture proves nothing — the plans would agree because there is no index
-- decision left to make.
--
-- WHY auto_explain AND NOT `EXPLAIN SELECT * FROM search_wide(...)`. `search_wide` is
-- LANGUAGE plpgsql, so an EXPLAIN of a call to it reports one line — `Function Scan on
-- search_wide` — and says nothing whatever about the plan inside, which is the only thing this
-- measurement is about. auto_explain with log_nested_statements reaches the statements the
-- function actually executes. `search_exact` is LANGUAGE sql and may or may not inline depending
-- on the planner's view of it; routing both through the same mechanism means the two arms are
-- captured on equal terms rather than one by luck.
--
-- COSTS CANNOT BE TURNED OFF HERE. auto_explain has no `log_costs` — the knob exists on EXPLAIN,
-- not on the module — so every plan carries `(cost=… rows=… width=…)`. Those are ESTIMATES and
-- move with statistics, which would make consecutive runs differ for reasons that are not the
-- refactor. The companion `capture-search-arm-plans.sh` strips them, so the recorded comparison is
-- structural: node types, join orders and index conditions, which are the things a
-- behaviour-preserving change must not move. Run the .sh, not this file directly.

\set ON_ERROR_STOP on
\pset pager off

LOAD 'auto_explain';
SET auto_explain.log_min_duration = 0;
SET auto_explain.log_nested_statements = on;
SET auto_explain.log_analyze = off;
SET auto_explain.log_verbose = on;
SET auto_explain.log_triggers = off;
SET auto_explain.log_format = 'json';
SET client_min_messages = log;

-- The principal: reaches ~2/3 of the corpus, so the visibility gate is doing real work rather than
-- passing everything or nothing. Resolved by handle, never hardcoded — ids are minted per seed.
SELECT id AS principal FROM kb_profiles WHERE handle = 'ana' \gset

-- A real anchor for the scoped branch.
SELECT id AS anchor FROM kb_contexts WHERE slug = 'platform-notes' \gset

-- A real query vector, taken from the corpus itself so it sits inside a topic cluster rather than
-- in empty space. ORDER BY id makes the choice stable across runs — a different vector would draw
-- a different top-k and the two captures would not be comparable.
SELECT embedding::text AS emb
  FROM kb_chunks
 WHERE is_current AND embedding IS NOT NULL
 ORDER BY id
 LIMIT 1 \gset

\echo '################ SHAPE 1 — search_exact, with a doc_type filter ################'
SELECT count(*) FROM search_exact(:'principal', 'postgres', NULL, NULL, 'note', 10, 0);

\echo '################ SHAPE 2 — search_wide, UNSCOPED (the approximate top-k branch) ################'
SELECT count(*) FROM search_wide(:'principal', :'emb'::vector, 100, NULL, NULL, NULL, 10, 0);

\echo '################ SHAPE 3 — search_wide, SCOPED (the exhaustive branch) ################'
SELECT count(*) FROM search_wide(:'principal', :'emb'::vector, 100, 'kb_contexts', :'anchor', NULL, 10, 0);

\echo '################ END ################'
