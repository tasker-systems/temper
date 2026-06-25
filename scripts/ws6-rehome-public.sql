-- scripts/ws6-rehome-public.sql
-- WS6 re-home: temper_next -> public. Steps 1-5, atomic (single transaction).
-- Run: psql "<conn>" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-public.sql
--
-- Does NOT drop temper_next (see ws6-rehome-finalize.sql, run last, after post-verify).
-- Extensions (vector, pg_uuidv7) already live in public and are NEVER moved or dropped;
-- the drops in steps 1-2 exclude extension-owned objects via pg_depend.deptype='e'.
-- All ALTER ... SET SCHEMA are OID-stable: columns/FKs/triggers follow their objects.

BEGIN;

-- ---- Guard: assert the expected split state before mutating ----
DO $$
DECLARE legacy_n int; next_n int;
BEGIN
  SELECT count(*) INTO legacy_n FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations';
  SELECT count(*) INTO next_n   FROM pg_tables WHERE schemaname='temper_next';
  IF next_n = 0 THEN
    RAISE EXCEPTION 're-home already applied or wrong DB: temper_next has no tables (legacy_n=%, next_n=%)', legacy_n, next_n;
  END IF;
  RAISE NOTICE 'Pre-state OK: % legacy public tables, % temper_next tables', legacy_n, next_n;
END $$;

-- ---- Step 1: drop legacy public data tables ----
-- Everything in public except _sqlx_migrations and extension-owned tables.
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='public' AND c.relkind='r'
      AND c.relname <> '_sqlx_migrations'
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid=c.oid AND d.deptype='e')
  LOOP
    EXECUTE format('DROP TABLE IF EXISTS public.%I CASCADE', r.relname);
  END LOOP;
END $$;

-- ---- Step 2a: drop legacy public functions, EXCLUDING extension-owned ----
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT p.proname, pg_get_function_identity_arguments(p.oid) AS args
    FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
    WHERE n.nspname='public'
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid=p.oid AND d.deptype='e')
  LOOP
    EXECUTE format('DROP FUNCTION IF EXISTS public.%I(%s) CASCADE', r.proname, r.args);
  END LOOP;
END $$;

-- ---- Step 2b: drop legacy public enums (non-extension; safe once their tables are gone) ----
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT t.typname FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace
    WHERE n.nspname='public' AND t.typtype='e'
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid=t.oid AND d.deptype='e')
  LOOP
    EXECUTE format('DROP TYPE IF EXISTS public.%I CASCADE', r.typname);
  END LOOP;
END $$;

-- ---- Step 3a: relocate canonical enums temper_next -> public ----
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT t.typname FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace
    WHERE n.nspname='temper_next' AND t.typtype='e'
  LOOP
    EXECUTE format('ALTER TYPE temper_next.%I SET SCHEMA public', r.typname);
  END LOOP;
END $$;

-- ---- Step 3b: relocate canonical tables temper_next -> public ----
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='temper_next' AND c.relkind='r'
  LOOP
    EXECUTE format('ALTER TABLE temper_next.%I SET SCHEMA public', r.relname);
  END LOOP;
END $$;

-- ---- Step 3c: relocate any remaining canonical relations (views/matviews/sequences) ----
-- Canonical uses UUIDv7 (no sequences) but views exist (e.g. vault_resources_browse);
-- relocate every leftover relation so temper_next ends empty.
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT c.relname, c.relkind FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='temper_next' AND c.relkind IN ('v','m','S')
  LOOP
    IF r.relkind = 'S' THEN
      EXECUTE format('ALTER SEQUENCE temper_next.%I SET SCHEMA public', r.relname);
    ELSIF r.relkind = 'm' THEN
      EXECUTE format('ALTER MATERIALIZED VIEW temper_next.%I SET SCHEMA public', r.relname);
    ELSE
      EXECUTE format('ALTER VIEW temper_next.%I SET SCHEMA public', r.relname);
    END IF;
  END LOOP;
END $$;

-- ---- Step 3d: relocate canonical functions temper_next -> public ----
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT p.proname, pg_get_function_identity_arguments(p.oid) AS args
    FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
    WHERE n.nspname='temper_next'
  LOOP
    EXECUTE format('ALTER FUNCTION temper_next.%I(%s) SET SCHEMA public', r.proname, r.args);
  END LOOP;
END $$;

-- ---- Step 4: reconcile _sqlx_migrations to the canonical baseline ----
-- (captured from a clean `cargo sqlx migrate run` — see scripts/ws6-rehome-sqlx-baseline.sql)
TRUNCATE public._sqlx_migrations;
INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES
  (20260624000001, 'canonical schema', now(), true, E'\\xf849ba7692dd5adedc05898b304ba3a8d59895aac93283ab9e04f8ea019f3211a2d86a40ba26b6fc4c974e4a89876aae'::bytea, 42942709),
  (20260624000002, 'canonical functions', now(), true, E'\\x281062d74637ac0fa119d7ba22723eafc09131ae34fb211d3184a3c49eea10dc1ada654fe439fdf7ab516a42c1fa1214'::bytea, 5867541),
  (20260624000003, 'canonical seed', now(), true, E'\\x334837fdb87735790633a56b70c7011f2496c162db415301e17e5c7ed0be3023b864ec336f21b8154abaa743214984b1'::bytea, 2958042);

-- ---- Step 5: revert the search_path default (drop the flip hack) ----
ALTER DATABASE neondb SET search_path TO public;

-- ---- Post-transaction guard: assert the unified shape ----
DO $$
DECLARE pub_n int; next_n int;
BEGIN
  SELECT count(*) INTO pub_n  FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations';
  SELECT count(*) INTO next_n FROM pg_tables WHERE schemaname='temper_next';
  IF pub_n <> 35 OR next_n <> 0 THEN
    RAISE EXCEPTION 'Post-state wrong: public=% (want 35), temper_next=% (want 0)', pub_n, next_n;
  END IF;
  RAISE NOTICE 'Post-state OK: % public tables, % temper_next tables', pub_n, next_n;
END $$;

COMMIT;
