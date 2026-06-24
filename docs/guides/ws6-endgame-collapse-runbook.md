# WS6 Endgame Collapse Runbook (live cutover)

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

## Cutover sequence — the SEARCH-PATH FLIP (executed 2026-06-24; supersedes the rename-promote)

> **Neon finding (load-bearing):** the original plan — rename `public`→`public_legacy`, relocate
> `vector` into `temper_next`, rename `temper_next`→`public` — **does NOT work on Neon.**
> `ALTER EXTENSION vector SET SCHEMA temper_next` fails with *"must be owner of type
> public_legacy.vector"* — `neondb_owner` cannot relocate the `vector` extension (Neon owns it).
> The Neon-native cutover is a **search-path flip**: the collapsed code is location-agnostic
> (de-qualified SQL + no explicit `SET search_path`), so pointing the connection default at
> `temper_next, public` makes it resolve canonical tables in `temper_next` and `vector`/`uuid` via
> the `public` fallback — **no schema rename, no extension move, fully reversible.** The legacy
> `public.*` stays intact + shadowed (droppable later).
>
> Confirm the target connection string is the intended prod branch before each statement.

6. [ ] **Identity/auth carry-over into `temper_next`** — synthesis carried ONLY the corpus owner
   (1 `kb_profiles` row, 0 `kb_profile_auth_links`), so with **0 auth_links nobody can authenticate**
   after the flip. Carry the rest from the still-named `public` (no rename — read `public.*`, write
   `temper_next.*`) using the validated draft §4 (`docs/superpowers/specs/2026-06-22-ws6-canonical-layer-draft.sql`;
   its DDL/graft half is already applied to `temper_next`). Run with `search_path=temper_next, public`:
   ```sql
   SET search_path TO temper_next, public;
   INSERT INTO kb_profiles (id, handle, display_name, system_access, email, preferences, created)
   SELECT p.id, p.slug, p.display_name,
          CASE WHEN p.slug='j-cole-taylor' THEN 'admin'::system_access
               WHEN p.slug IN ('gm-anirudh','lohjishan') THEN 'approved'::system_access
               ELSE 'none'::system_access END,
          p.email, p.preferences, p.created
   FROM public.kb_profiles p
   ON CONFLICT (id) DO UPDATE SET system_access=EXCLUDED.system_access, email=EXCLUDED.email, preferences=EXCLUDED.preferences;
   INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
   SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
   FROM public.kb_profile_auth_links ON CONFLICT (id) DO NOTHING;
   INSERT INTO kb_system_settings (id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated)
   SELECT id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated
   FROM public.kb_system_settings ON CONFLICT (id) DO UPDATE SET access_mode=EXCLUDED.access_mode;
   ```
   Verify: 5 profiles / 5 auth_links in `temper_next`; `has_system_access(owner)=t`.

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

9. [ ] **THE FLIP — point the connection default at the canonical schema:**
   ```sql
   ALTER DATABASE neondb SET search_path TO temper_next, public;
   ```
   New connections now resolve canonical tables in `temper_next` and `vector`/`uuid` via the `public`
   fallback. The collapsed code carries no explicit `SET search_path` (the flip removed
   `substrate::connect`'s `after_connect`), so it picks this up on its next connection.
   **Reversible:** `ALTER DATABASE neondb SET search_path TO public;` reverts to legacy.

9b. [ ] **(Optional) Reconcile `_sqlx_migrations` for tooling alignment.** The deploy runs no
   boot-time `migrate!`, so this is housekeeping, not a gate. The canonical schema lives in
   `temper_next`; future additive migrations apply with `cargo sqlx migrate run` under
   `search_path=temper_next`. If you want `sqlx migrate info` to read clean against the canonical set,
   create `temper_next._sqlx_migrations` and mark-as-applied the 3 baseline rows (NOT replay) — see
   `docs/superpowers/specs/2026-06-23-canonical-migrations-in-public-design.md` §5. Deferred at the
   2026-06-24 cutover (not load-bearing for serving traffic).

   <details><summary>Superseded: the migration-aligned <code>public</code> reconciliation (only if a
   future Neon-permission path enables the rename-promote)</summary>

   The promoted `public` would be structurally artifact-faithful but its `_sqlx_migrations` still lists
   the retired legacy lineage. The schema already exists — do NOT replay DDL.
   1. **Structural safety check (HARD GATE):** `pg_dump --schema-only` of live `public` vs. a fresh
      DB built from `migrations/` — the diff must be empty (both derive from the same artifact).
   2. **Compute the baseline checksums** sqlx expects: `sqlx migrate info --source migrations`.
   3. **Mark-as-applied:** `TRUNCATE _sqlx_migrations;` then `INSERT` the 3 baseline rows.
   4. **Verify:** `sqlx migrate info` shows all 3 **applied**, and `sqlx migrate run` is a clean no-op.
   </details>
   The deployment is now migration-aligned
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
