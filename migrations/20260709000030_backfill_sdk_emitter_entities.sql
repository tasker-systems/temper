-- Backfill the `sdk` per-surface emitter entity (`<handle>@sdk`) for every already-provisioned
-- profile. Pairs with `Surface::Sdk` and the `Surface::ALL`-driven loop in
-- `temper-services/src/services/profile_service.rs`, which provisions it for profiles created
-- from here on.
--
-- MUST DEPLOY BEFORE any client can send the `sdk` surface marker. `writes::resolve_emitter` is a
-- `fetch_one` against the `<handle>@<surface>` natural key — there is no lazy creation, so an `sdk`
-- write against a profile with no `@sdk` entity 500s. This is the migrate-ahead-of-deploy skew
-- shape: this migration lands, then the surface marker starts travelling over the wire.
--
-- Guarded with NOT EXISTS rather than ON CONFLICT: `kb_entities` has no unique constraint on
-- (profile_id, name) — only the `id` primary key and an index on `profile_id`. Mirrors the guard in
-- 20260624000003_canonical_seed.sql, which documents exactly this. Idempotent: a second run inserts
-- nothing.
--
-- The `EXISTS (<handle>@web)` predicate restricts the backfill to profiles that carry the
-- per-surface emitter set, i.e. those that went through `provision_profile_entities`. It
-- deliberately excludes the `system` profile, whose emitter is the bare entity `system` (see
-- 20260624000003_canonical_seed.sql) and which never resolves through `resolve_emitter`. Keying on
-- that structural fact rather than on `handle <> 'system'` keeps the guard honest if another
-- unprovisioned principal ever appears.
--
-- Additive-only: nothing is dropped or altered, so the additive-only-on-`main` invariant holds.
--
-- OPERATOR NOTE — apply this migration when the four-surface provisioning loop is not concurrently
-- creating profiles (migrate first, then deploy the code; or migrate in a quiet window).
-- `provision_profile_entities` commits each emitter as its own auto-commit statement, so a profile
-- being provisioned *while* this runs can have its `@web` observed by the SELECT before its `@sdk`
-- is committed — both writers then insert `@sdk`. With no unique constraint on (profile_id, name)
-- the duplicate survives. It does not 500: `resolve_emitter`'s `fetch_one` is `fetch_optional` +
-- RowNotFound (sqlx has no too-many-rows error), so it silently returns whichever row Postgres
-- yields first. The cost is split, nondeterministic attribution for that one profile, not an outage.
--
-- The race is pre-existing in shape — the provisioning loop is already unguarded and untransacted,
-- so two concurrent first-requests can already double-insert `@web`. Closing it properly means a
-- unique constraint on (profile_id, name), which is its own task (it must first prove no duplicates
-- exist in production). Deploy ordering is the cheap mitigation.

INSERT INTO kb_entities (profile_id, name, metadata)
SELECT p.id, p.handle || '@sdk', '{}'::jsonb
  FROM kb_profiles p
 WHERE EXISTS (
           SELECT 1 FROM kb_entities e
            WHERE e.profile_id = p.id AND e.name = p.handle || '@web'
       )
   AND NOT EXISTS (
           SELECT 1 FROM kb_entities e
            WHERE e.profile_id = p.id AND e.name = p.handle || '@sdk'
       );
