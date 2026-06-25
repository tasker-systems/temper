# WS6 Endgame Collapse Runbook (live cutover)

> **⚠️ SUPERSEDED (2026-06-25).** The rename-promote described below (rename `temper_next` →
> `public`) is **Neon-blocked**: `neondb_owner` cannot relocate the `vector` extension out of
> `public`. Production was cut over via a search-path flip and then re-homed into `public` by
> per-object `ALTER … SET SCHEMA`. See
> [ws6-rehome-to-public-runbook.md](./ws6-rehome-to-public-runbook.md). This document is retained
> for historical context only.

Operator checklist for the **live schema collapse**: renaming the already-live
`temper_next` to the canonical `public`, retiring the stale `public`, and redeploying
the collapsed code. This is the destructive, one-shot production step that the code
plan (`docs/superpowers/plans/2026-06-22-ws6-endgame-collapse-code.md`) was written to
make safe.

Design spec: `docs/superpowers/specs/2026-06-22-ws6-migration-endgame-design.md`
(§"Executable collapse sequence"). Canonical-layer graft: `docs/superpowers/specs/2026-06-22-ws6-canonical-layer-draft.sql`.

> **Posture.** Single-user (arc-1); brief operator-controlled downtime is acceptable.
> **Prod is already on `temper_next`** (the flip ran 2026-06-21). `public` is **stale** —
> it is NOT a rollback target. The rollback target is a Neon snapshot of the **live**
> (`temper_next`) state. Every destructive step gates on (a) a held snapshot and (b) an
> explicit confirmation of the target connection (the `flip-load-next`-against-main scare
> is why).

---

## Pre-flight

1. [ ] **Green code branch.** The code plan is merged/ready: `surface_parity_next` is the
   eight-surface gate (un-ignored), `cargo make test-all` green against the local collapsed
   schema, and the deployable build carries no `temper_next.`-qualified SQL, no
   `kb_backend_selection` read, and no boot-time `migrate!`.

2. [ ] **`neonctl` authenticated**, version ≥ 2.26:
   ```bash
   neonctl --version
   neonctl projects list --org-id org-wild-snow-32921543
   ```

3. [ ] **Extension/uuid-homing rehearsal on a throwaway PG17 branch (the highest-risk DDL).**
   This is BLOCKER-1; validate it before touching main.
   ```bash
   neonctl branches create --name ws6-collapse-rehearsal-<date> --parent main \
     --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
   # On the rehearsal branch, run steps 4 (rename aside), 5 (extension homing),
   # 6 (graft), 7 (promote), then:
   #   - confirm a ::vector cast + an HNSW/IVF index still resolve in the new public
   #   - confirm uuid_generate_v7() mints valid v7 UUIDs in the new public
   # Then run the eight-surface parity gate against the branch (flag already 'next').
   neonctl branches delete <branch-id> --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
   ```
   Do not proceed to the real cutover until the rehearsal is boring (green).

---

## Freeze

4. [ ] **Stop all writes to production.** Operator discipline (single writer): no ingest
   job, no cron/agent, no other CLI/browser session against prod. The only live-path
   reference to `public.*` is the prod→next profile bridge in the *pre-collapse* code; the
   write-freeze guarantees it is unexercised during the window.

---

## Snapshot

5. [ ] **Snapshot the LIVE (`temper_next`) state** — the rollback target:
   ```bash
   neonctl branches create --name ws6-collapse-rollback-<date> --parent main \
     --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
   ```
   **Record the branch name + id.** This is the only rollback point (`public` is stale).

5b. [ ] **PERSISTENT BACKUP GATE — operator hard-stop. Do NOT run any step ≥ 6 until this is done.**
   Elevate the step-5 branch (or cut a parallel one) into a **durable, explicitly-retained**
   point-in-time backup — protected from Neon's default branch/PITR expiry so it survives as the
   permanent "restore to exactly pre-flip" target long after the cutover (distinct from the
   operational rollback branch, which may be cleaned up once the flip is confirmed). This is the
   last point where rollback is a single lookup; steps 6–9 are destructive schema renames. Record
   its identifier + restore command inline here before proceeding:
   - Durable backup branch / id: `__________`
   - Restore command:            `neonctl branches restore … __________`

   Executed manually by the operator, or by the agent once `neonctl` is authenticated.

---

## DDL sequence (one operator session, against the recorded target)

> Confirm the target connection string is the intended prod branch **before every
> destructive statement.** Steps 6–9 are DDL (transactional in PG); run 6–9 in one
> transaction where practical, committing before the redeploy (10).

6. [ ] **Rename the stale schema aside:**
   ```sql
   ALTER SCHEMA public RENAME TO public_legacy;
   ```

7. [ ] **Relocate the shared infrastructure into the surviving schema** (BLOCKER-1 fix —
   rehearsed in pre-flight step 3):
   ```sql
   -- vector: relocate the extension (pgvector is relocatable)
   ALTER EXTENSION vector SET SCHEMA temper_next;
   -- uuid: re-create a self-contained generator in temper_next (do NOT relocate pg_uuidv7),
   -- mirroring tools/flip/uuid_portable.sql:
   CREATE OR REPLACE FUNCTION temper_next.uuid_generate_v7() RETURNS uuid
   LANGUAGE sql VOLATILE PARALLEL SAFE AS $$
     SELECT encode(set_bit(set_bit(overlay(
       uuid_send(gen_random_uuid())
       PLACING substring(int8send((extract(epoch FROM clock_timestamp())*1000)::bigint) FROM 3)
       FROM 1 FOR 6), 52, 1), 53, 1), 'hex')::uuid;
   $$;
   ```

8. [ ] **Apply the canonical-layer graft/reconcile/carry-over** — the validated draft
   `docs/superpowers/specs/2026-06-22-ws6-canonical-layer-draft.sql` (substitute the real
   `LEGACY` schema name = `public_legacy`). It reconciles `kb_profiles` (re-add
   `email`/`preferences`), grafts the 7 substrate-absent infra tables + enums, and
   `INSERT…SELECT`s the identity/auth/seed rows from `public_legacy` into `temper_next`.
   Re-confirm its live-diff half (row counts: 5 profiles / 5 auth_links / …) against the
   snapshot — the DDL half was verified byte-faithful to the cited migrations.

8c. [ ] **Align emitter-entity names with the de-hardcoded resolver** (code change in this branch).
   The collapsed write path resolves the per-surface emitter entity by **`<handle>@<surface>`**
   (`temper_next::writes::resolve_emitter` joins `kb_entities`→`kb_profiles`), replacing the former
   hardcoded `pete@<surface>` literal. The live `kb_entities` rows were created by the now-retired
   synthesis bootstrap with the legacy `pete@` naming, so rename any whose local-part no longer
   matches the owner's handle — otherwise every authenticated write 500s on a missing emitter:
   ```sql
   UPDATE kb_entities e
      SET name = p.handle || '@' || split_part(e.name, '@', 2)
     FROM kb_profiles p
    WHERE p.id = e.profile_id
      AND e.name LIKE '%@%'
      AND split_part(e.name, '@', 1) <> p.handle;
   ```
   (Newly auto-provisioned profiles get `<handle>@{web,cli,mcp}` from `resolve_from_claims`; this
   step only fixes the pre-existing synthesized rows.)

9. [ ] **Promote:**
   ```sql
   ALTER SCHEMA temper_next RENAME TO public;
   ```
   The canonical schema is now `public` — the connection default — owning its extensions +
   uuid generator (step 7).

9b. [ ] **Reconcile `_sqlx_migrations` to the canonical baseline (mark-as-applied, NOT replay).**
   The promoted `public` is structurally artifact-faithful but its `_sqlx_migrations` still lists the
   retired legacy lineage. The schema already exists — do NOT replay DDL. (This replaces the prior
   "no reconciliation needed / bootstrap-export spec's job" punt; the canonical-migrations-in-public
   spec owns it: `docs/superpowers/specs/2026-06-23-canonical-migrations-in-public-design.md` §5.)
   1. **Structural safety check (HARD GATE):** `pg_dump --schema-only` of live `public` vs. a fresh
      DB built from `migrations/` — the diff must be empty (both derive from the same artifact).
      Account for extension residency + the uuid shim: the canonical baseline self-provisions the
      `vector` extension and `uuid_generate_v7()`, whereas here they were relocated into the surviving
      schema at step 7 — reconcile that benign provisioning difference rather than hand-waving the
      diff. Abort the reconciliation if anything structural differs.
   2. **Compute the baseline checksums** sqlx expects: `sqlx migrate info --source migrations` against
      a fresh baseline DB (or read the `_sqlx_migrations` rows it writes there).
   3. **Mark-as-applied** on the live DB: `TRUNCATE _sqlx_migrations;` then `INSERT` the 3 baseline
      rows (`version`, `description`, `checksum`, `success=true`, `installed_on=now()`,
      `execution_time=0`).
   4. **Verify:** `sqlx migrate info --source migrations` shows all 3 **applied**, and a
      `sqlx migrate run` against the live DB is a clean no-op. The deployment is now migration-aligned
      with the canonical set.

---

## Redeploy + verify

10. [ ] **Redeploy the Vercel app** (both `api` and `mcp` functions) with the collapsed code.
    The pre-collapse process reads schema names the rename changed, so a running process
    cannot survive the rename — the redeploy must be coincident.
    ```bash
    vercel --prod
    ```

11. [ ] **Verify the eight-surface parity gate over the live schema** + a live smoke check:
    ```bash
    temper resource list
    temper resource show <ref>
    temper resource search <query>
    # plus a graph read and a context cursor read
    ```
    Expected: every surface resolves; no 5xx; ids preserved.

---

## Unfreeze

12. [ ] **Resume writes.** New writes land in the one `public` schema — no flag, no
    search_path hooks.

---

## Drop the stale schema (point of no return)

13. [ ] **After the retention window**, drop `public_legacy` — gated on the held snapshot,
    the 2 Flag-2 content-hash spot-checks, AND the dependency guard returning clean:
    ```sql
    -- (a) vector resident in canonical public, NOT public_legacy:
    SELECT n.nspname FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
     WHERE e.extname = 'vector';                         -- expect: public
    -- (b) no canonical object depends on public_legacy:
    SELECT c.relname, rc.relname FROM pg_depend d
     JOIN pg_class c  ON c.oid  = d.objid    JOIN pg_namespace n  ON n.oid  = c.relnamespace
     JOIN pg_class rc ON rc.oid = d.refobjid JOIN pg_namespace rn ON rn.oid = rc.relnamespace
     WHERE n.nspname='public' AND rn.nspname='public_legacy';   -- expect: zero rows
    ```
    Only with (a) = `public` and (b) = zero rows:
    ```sql
    DROP SCHEMA public_legacy CASCADE;
    ```

---

## Rollback

- **Before the drop (step 13):** restore by repointing to the snapshot branch (step 5).
  `public_legacy` also still exists in place — but the canonical data lives in the renamed
  schema, so the snapshot is the clean target.
- **After the drop:** snapshot restore only. This is the point of no return; it gates on the
  eight-surface gate being green (step 11) and a held snapshot.
