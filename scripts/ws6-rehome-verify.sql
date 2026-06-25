-- scripts/ws6-rehome-verify.sql — read-only health probes for the WS6 re-home.
-- Schema-agnostic: every table reference resolves via the connection's search_path,
-- so the SAME probes work pre-re-home (data in temper_next) and post-re-home (data in public).
-- Reused for rehearsal post-check (plan Task 3) and prod post-verify (plan Task 6).
-- Run: psql "<conn>" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql

-- INVARIANT for probes 1–3: the re-home relocates schemas only and touches NO data,
-- so each count post-re-home must EQUAL the same count captured immediately pre-re-home
-- in the same session. The numbers below are illustrative (prod, 2026-06-25) — capture
-- fresh and compare pre vs post; do not treat the literals as a pass/fail target.

\echo === probe 1: visible resources (~1262 on 2026-06-25; post must == pre) ===
SELECT count(*) AS visible_resources FROM kb_resources WHERE is_active = true;

\echo === probe 2: vector chunks (~14852 on 2026-06-25; post must == pre) ===
SELECT count(*) AS vector_chunks FROM kb_chunks;

\echo === probe 3: a representative content join resolves (expect > 0 rows, no error) ===
SELECT count(*) AS joinable_chunk_content
FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id;

\echo === probe 4: schema topology (pre: public~27/next=35; post: public=35/next=0) ===
SELECT
  (SELECT count(*) FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations') AS public_tables,
  (SELECT count(*) FROM pg_tables WHERE schemaname='temper_next') AS temper_next_tables,
  (SELECT setting FROM pg_settings WHERE name='search_path') AS session_search_path;

\echo === probe 5: extensions intact in public (expect vector + pg_uuidv7) ===
SELECT extname, n.nspname AS schema
FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE extname IN ('vector','pg_uuidv7') ORDER BY 1;
