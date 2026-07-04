-- T7c Task 9 (enum half) — add the 'remote' provenance source kind (external URLs).
--
-- This is its OWN migration on purpose: PostgreSQL commits an `ALTER TYPE … ADD VALUE` before the new
-- value may be *used*, and a value added in a transaction cannot be referenced in that same
-- transaction. sqlx runs each migration in its own transaction and commits between them, so the
-- storage migration (…007) that references 'remote' at runtime runs strictly after this one commits.
-- Additive-only-on-main: a new enum value, no edit to the birth type (20260624000001:105).
ALTER TYPE provenance_source_kind ADD VALUE IF NOT EXISTS 'remote';
