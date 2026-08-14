-- Install `pg_stat_statements`, so the deployment can be asked what a read costs.
--
-- ── WHAT IS WRONG ────────────────────────────────────────────────────────────────────────────────
--
-- There is no query-level observability on prod, and there never has been. `[measured — 2026-08-14,
-- prod, read-only]` `SELECT * FROM pg_extension` returns exactly `plpgsql`, `pg_uuidv7` and
-- `vector`. So the question "which statement is expensive, and how expensive" has no instrument, on
-- the only system whose numbers count.
--
-- This blocks a filed acceptance criterion rather than merely being nice to have. Task
-- `01a000ee-9fec-7283-baa5-75cd1580f023` ("Nothing bounds what a read costs") requires that any
-- execution bound be *"measured against real legitimate queries first, not picked"*. Today that
-- measurement is impossible, so any `statement_timeout` chosen now would be a guess wearing the
-- costume of a number. This migration is the instrument; the number is deliberately a later,
-- separate decision `[decided — 2026-08-14, Pete]`.
--
-- ── WHY THIS IS ONE LINE AND NOT A SUPPORT TICKET ────────────────────────────────────────────────
--
-- `pg_stat_statements` is ALREADY preloaded on prod and simply was never created
-- `[measured — 2026-08-14, prod]`:
--
--     shared_preload_libraries = neon,pg_stat_statements,timescaledb,pg_cron,pg_partman_bgw,…
--
-- The module is resident and collecting nothing, because no extension exposes it. `CREATE EXTENSION`
-- is the whole of the work.
--
-- ── WHY THE FAILURE IS SWALLOWED, WHICH IS NOT THIS REPO'S DEFAULT ───────────────────────────────
--
-- A migration that raises fails the build, and `vercel.json`'s `buildCommand` runs
-- `temper-migrate --additive-only` — so a migration that cannot apply does not degrade one feature,
-- it **blocks every subsequent deploy on that target until an operator intervenes**. `CREATE
-- EXTENSION pg_stat_statements` is not a trusted extension: it needs a privileged role. That holds
-- for `neondb_owner` on Neon and for the local `temper` role (both verified), but a self-hosted
-- operator running temper under a restricted role would be permanently unable to deploy — traded
-- for an observability nicety that nothing in the product reads.
--
-- So this arm is deliberately advisory. The repo's fail-loud default is for correctness, and this is
-- not correctness: no code path depends on the extension existing, and a target without it is
-- exactly as correct as one with it, merely unmeasurable. `WHEN OTHERS` is broad on purpose — the
-- failure modes are at least `insufficient_privilege` (42501) and a missing module file (58P01), and
-- enumerating them buys nothing when every outcome is the same WARNING.
--
-- ── THE CAVEAT A SELF-HOSTED READER NEEDS ────────────────────────────────────────────────────────
--
-- `[verified — 2026-08-14, local Docker `pgvector:0.8.2-pg18`, `shared_preload_libraries` empty]`
-- `CREATE EXTENSION` **succeeds** without the module preloaded; only *reading the view* then raises
-- `pg_stat_statements must be loaded via "shared_preload_libraries"`. So this migration is safe on a
-- target that cannot actually collect, and the extension's presence is NOT evidence that statistics
-- are being gathered. Nothing in temper queries the view, so that asymmetry is inert here — but an
-- operator who adds a dashboard over it must check `shared_preload_libraries`, not `pg_extension`.
--
-- ── WHAT THIS DOES NOT DO ────────────────────────────────────────────────────────────────────────
--
-- It sets no `statement_timeout`, bounds nothing, and changes no query. It makes the next decision
-- about bounding an evidenced one instead of a guess. `pg_stat_statements.max` and `.track` are left
-- at their server defaults deliberately: tuning a collector before it has collected anything is the
-- same error one layer down.

DO $$
BEGIN
    CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'pg_stat_statements not installed (%). Query-level observability is unavailable on this target; nothing else is affected.', SQLERRM;
END;
$$;

SELECT declare_migration(
    20260814000020,
    'additive',
    'Install pg_stat_statements, the missing instrument for query-level observability. Prod carries the module in shared_preload_libraries but has never created the extension (measured 2026-08-14, read-only: pg_extension holds only plpgsql, pg_uuidv7, vector), so no statement-level cost has ever been observable on the only system whose numbers count. This unblocks task 01a000ee-9fec-7283-baa5-75cd1580f023, whose acceptance requires any execution bound be measured against real legitimate queries rather than picked; the statement_timeout number itself is deliberately NOT chosen here. Creates no object temper reads, changes no query, and sets no timeout. The CREATE is wrapped so a failure WARNs instead of raising: the additive-only build gate means a raising migration blocks every subsequent deploy on that target, and CREATE EXTENSION requires a privileged role that a self-hosted operator may not have — an unacceptable trade for observability nothing in the product depends on. Verified locally that CREATE EXTENSION succeeds even where the module is not preloaded (only reading the view then raises), so the migration is safe on every target shape.'
);
