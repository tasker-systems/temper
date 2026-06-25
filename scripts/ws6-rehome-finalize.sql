-- scripts/ws6-rehome-finalize.sql
-- WS6 re-home FINALIZE: drop the emptied temper_next schema. POINT OF NO CHEAP RETURN.
-- Run ONLY after ws6-rehome-public.sql committed AND post-verify (SQL + live app) passed.
-- Run: psql "<conn>" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-finalize.sql

-- Guard: refuse to drop if temper_next still holds any data-bearing or definitional
-- relation (table/view/matview/sequence) — that would mean the re-home move was
-- incomplete and dropping now could destroy live objects.
DO $$
DECLARE n int;
BEGIN
  SELECT count(*) INTO n
  FROM pg_class c JOIN pg_namespace ns ON ns.oid = c.relnamespace
  WHERE ns.nspname = 'temper_next' AND c.relkind IN ('r','v','m','S');
  IF n <> 0 THEN
    RAISE EXCEPTION 'temper_next still holds % relation(s) — refusing to drop (re-home incomplete)', n;
  END IF;
  RAISE NOTICE 'temper_next is empty — dropping schema';
END $$;

DROP SCHEMA IF EXISTS temper_next CASCADE;
