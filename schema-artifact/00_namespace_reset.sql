-- Destructive namespace reset — TEST-ONLY preamble (NOT part of the additive install migration).
-- The artifact body (01_schema.sql) is namespace-resident DDL with no DROP; this file is what the
-- test harness prepends to own + reset the namespace. The production install migration prepends a
-- run-once `CREATE SCHEMA temper_next;` instead (see tools/gen-install-migration.sh).
DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
SET search_path TO temper_next, public;
