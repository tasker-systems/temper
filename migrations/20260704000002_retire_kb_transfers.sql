-- Retire the dead `kb_transfers` table + `transfer_status` enum (teams-in-temper goal, scope #6).
--
-- `kb_transfers` was born in 20260624000001_canonical_schema.sql for a two-step offer/accept
-- ownership-transfer handshake that was never wired. Resource ownership shipped instead as
-- in-place REASSIGNMENT (20260703140000_resource_reassign_fns.sql), which mutates
-- kb_resource_homes.owner_profile_id directly and never touched kb_transfers. The table has
-- ZERO references in live code (only the canonical-schema CREATE and one substrate identity
-- graft test), and carries NO rows in production.
--
-- This DROP is destructive DDL, not additive — but the additive-only-on-`main` invariant guards
-- against dropping tables with live dependents or data. kb_transfers has neither, so removing it
-- is safe to auto-deploy. IF EXISTS keeps it idempotent across targets at differing cadences.
DROP TABLE IF EXISTS kb_transfers;
DROP TYPE IF EXISTS transfer_status;
