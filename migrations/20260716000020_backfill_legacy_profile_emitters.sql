-- Backfill the four per-surface emitter entities (`<handle>@web|cli|mcp|sdk`) and the `default`
-- context for legacy profiles that never ran `provision_profile_entities`.
--
-- THE BUG. `writes::resolve_emitter` (temper-substrate/src/writes.rs:50) is a `fetch_one` against
-- the `<handle>@<surface>` natural key with no lazy creation — its own doc says so. A profile with
-- no emitter therefore 500s on its FIRST WRITE, with a real HTTP 500 reading
-- "no emitter entity <handle>@cli for the resolved profile". Two approved, active human profiles in
-- production carry zero `kb_entities` and zero contexts (`gm-anirudh`, created 2026-04-10;
-- `lohjishan`, 2026-05-22). Both predate the canonical schema (20260624000001) and were carried in
-- by a legacy import, so `provision_profile_entities` — which creates the emitters and the default
-- context together, in one function (`profile_service.rs:451`) — never ran for them. It is latent
-- only because neither has ever written; it stops being latent the moment either returns.
--
-- Note the sign-in path does not heal them: `resolve_or_create_from_claims` provisions only on its
-- brand-new-profile branch (step 5). An existing profile returns early from the auth-link lookup or
-- from email reconciliation, provisioning nothing. So these two rows stay broken until a migration
-- fixes them.
--
-- WHY 20260709000030's GUARD MISSED THEM. The sdk backfill guards on `EXISTS (<handle>@web)`,
-- restricting itself to profiles that already went through `provision_profile_entities`. These two
-- have no `@web`, so that guard STRUCTURALLY EXCLUDES EXACTLY THE PROFILES THAT NEED HELP. Any
-- migration copying that guard shape skips them again. Do not copy it here.
--
-- THE GUARD: "HAS NO ENTITIES AT ALL". The predicate is the bug itself — a profile with zero
-- `kb_entities` has never been provisioned by anything and cannot emit on any surface. It is stated
-- structurally, and it ENUMERATES NO EXCEPTIONS, which is the whole point:
--
--   * `system` is excluded because it HAS its emitter — the bare entity whose name is its handle,
--     seeded by 20260624000003_canonical_seed.sql. It never resolves through `resolve_emitter`.
--     Keying on `handle <> 'system'` would be dishonest, and 20260709000030 rejects handle-keying
--     in as many words.
--   * CONNECTION PROFILES are excluded because they HAVE their emitter: `connection_service`
--     mints `<handle>@webhook` inline (connection_service.rs:140) rather than calling
--     `provision_profile_entities`. A connection emits over one webhook surface and can never use
--     web/cli/mcp/sdk, so manufacturing four emitters for it would be exactly the error the `system`
--     exclusion exists to avoid. Production has no connections today, so this is latent here — but
--     self-hosted installs consume this repo on their own cadence, and this migration ships.
--   * ANY FUTURE PROFILE SHAPE that mints its own emitter is excluded for free, because it will
--     have an entity. An earlier draft of this guard keyed on `e.name = p.handle` (the system
--     shape) and would have swept connection profiles in — the exception-enumerating shape rots the
--     moment a new principal kind appears, which is precisely what connections demonstrated.
--
-- The cost of stating it this way is that this migration does NOT repair a PARTIALLY provisioned
-- profile (one whose provisioning loop failed midway, leaving `@web` but no `@mcp`). That is a
-- different bug with a different predicate; 20260709000030 already handled the one known instance
-- (the `sdk` surface). Distinguishing "per-surface-shaped but incomplete" from "a shape that mints
-- its own emitter" cannot be done without enumerating shapes, which is the trap above. Repairing
-- partials honestly needs its own task.
--
-- `anonymous` IS PROVISIONED, deliberately. It has zero entities and `system_access = 'none'`, so
-- it will likely never emit. Provisioning it is fail-safe and costs four unused rows; excluding it
-- would need its own honest predicate, which is more machinery than the exclusion is worth. If it
-- never emits, the rows are inert. If it somehow does, it does not 500.
--
-- WHY THE TWO INSERTS GUARD DIFFERENTLY. They are not the same question, and collapsing them
-- would cause real damage. The DEFAULT CONTEXT additionally requires the profile to have NO
-- CONTEXTS AT ALL. A profile with zero contexts never had provisioning run; a profile that HAS
-- contexts has since made its own choices, and this migration does not second-guess them. This
-- matters concretely: live production shows `j-cole-taylor` holds six contexts and NONE of them is
-- slugged `default`. A naive "provision a default for every profile lacking one" guard would
-- silently resurrect a context that account does not have and did not ask for. Absence of a
-- `default` is not evidence of a missing provisioning run; absence of EVERY context is.
--
-- The guards being independent has a deliberate consequence worth stating, because it looks like a
-- miss and is not. A legacy profile that signs in BEFORE this migration lands can create a context
-- (that path fires no event, so it needs no emitter) and only then hit the 500 on a resource write.
-- When this migration subsequently runs, that profile HAS a context, so it gets its emitters —
-- which is what the 500 needed — and NO default context, because by then it has made its own
-- choice. Both outcomes are correct. The two production rows this targets are dormant (zero
-- resources), so they take the other branch and receive both halves.
--
-- IDEMPOTENT BY CONSTRUCTION, not by guard. `kb_entities` gained `UNIQUE (profile_id, name)` in
-- 20260709000040 and `kb_contexts` carries `UNIQUE (owner_table, owner_id, slug)`, so both inserts
-- use `ON CONFLICT DO NOTHING` — the same clause `provision_profile_entities` itself uses. This
-- retires 20260709000030's race caveat wholesale: that migration could only mitigate by deploy
-- ordering because no unique constraint existed to infer. One does now. A profile being provisioned
-- concurrently with this migration is a no-op collision, not a duplicate, so there is NO operator
-- quiet-window requirement here.
--
-- `id` is omitted from both inserts so the column default (`uuid_generate_v7()`) fires. Never call
-- native `uuidv7()` — it does not exist on Neon's PostgreSQL 17, where production runs.
--
-- The surface list is `Surface::ALL` (temper-workflow/src/operations/surface.rs:38) — note the
-- marker for `ApiHttp` is `web`, deliberately distinct from its serde form. A new variant there
-- obliges a new additive backfill; it does not change this one.
--
-- Additive-only: nothing is dropped or altered, so the additive-only-on-`main` invariant holds.

-- ONE STATEMENT, NOT TWO, AND THAT IS LOAD-BEARING. Both inserts key off "has no entities at all",
-- so as two sequential statements the first would falsify the second's guard: the entity insert
-- lands, every target then HAS entities, and the default context is never created for anyone. A
-- data-modifying CTE closes that by construction rather than by comment — every CTE here reads the
-- SAME pre-statement snapshot and cannot observe a sibling's writes, so `targets` is evaluated once
-- and both inserts see the world as it was before either ran. Splitting this back into two
-- statements silently breaks the context half; it does not error.
--
-- `provisioned_emitters` is deliberately never referenced by the primary query, which looks like
-- dead code and is not: Postgres executes a data-modifying WITH clause "exactly once, and always to
-- completion, independently of whether the primary query reads any of their output". The emitter
-- insert is the one that matters most here, and it runs on that guarantee.
WITH targets AS (
    SELECT p.id, p.handle
      FROM kb_profiles p
     WHERE NOT EXISTS (
               SELECT 1 FROM kb_entities e WHERE e.profile_id = p.id
           )
),
provisioned_emitters AS (
    INSERT INTO kb_entities (profile_id, name, metadata)
    SELECT t.id, t.handle || '@' || s.marker, '{}'::jsonb
      FROM targets t
     CROSS JOIN (VALUES ('web'), ('cli'), ('mcp'), ('sdk')) AS s(marker)
    ON CONFLICT (profile_id, name) DO NOTHING
    RETURNING 1
)
INSERT INTO kb_contexts (owner_table, owner_id, slug, name)
SELECT 'kb_profiles', t.id, 'default', 'default'
  FROM targets t
 WHERE NOT EXISTS (
           SELECT 1 FROM kb_contexts c
            WHERE c.owner_table = 'kb_profiles' AND c.owner_id = t.id
       )
ON CONFLICT (owner_table, owner_id, slug) DO NOTHING;
