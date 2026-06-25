# WS6 Re-home Runbook: `temper_next` → `public`

Operator checklist for the **post-flip namespace collapse**: retiring the search-path
flip by relocating the canonical substrate from `temper_next` into `public`, then dropping
`temper_next`. This is the final WS6 step — after it, production runs a single unqualified
`public` schema identical in shape to a fresh `migrations/` build.

Design spec: `docs/superpowers/specs/2026-06-25-ws6-rehome-temper-next-to-public-design.md`
Plan: `docs/superpowers/plans/2026-06-25-ws6-rehome-temper-next-to-public.md`
Parity proof: `docs/superpowers/specs/2026-06-25-ws6-parity-report.md`

> **Why a re-home, not a rename.** The original WS6 runbooks promote `temper_next` by
> **renaming** it to `public`. That is **Neon-blocked**: `neondb_owner` cannot relocate the
> `vector` extension out of `public`, and the rename collides with the extension-bearing
> `public`. The re-home instead drops the legacy `public` objects and moves the canonical
> objects **into** `public` per-object (`ALTER … SET SCHEMA public`). The extensions
> (`vector`, `pg_uuidv7`) already live in `public` and **never move**.

> **Posture.** Single-user (arc-1); brief operator-controlled downtime is acceptable. The
> live app keeps working throughout: app connections resolve `search_path = temper_next, public`,
> and after the move the canonical tables are in `public`, so the public fallback still finds
> them even before the default is reverted.

## Prerequisites

- `neonctl` authenticated; `psql` available.
- Neon coordinates: project `crimson-fog-23541670`, org `org-wild-snow-32921543`, branch `main`,
  role `neondb_owner`.
- The four committed scripts:
  - `scripts/ws6-rehome-public.sql` — steps 1–5, atomic (drop legacy, relocate canonical,
    reconcile `_sqlx_migrations`, revert `search_path` default).
  - `scripts/ws6-rehome-finalize.sql` — step 6, `DROP SCHEMA temper_next` (point of no cheap return).
  - `scripts/ws6-rehome-verify.sql` — read-only health probes.
  - `scripts/ws6-rehome-sqlx-baseline.sql` — the canonical ledger rows (already inlined into
    `ws6-rehome-public.sql`; kept for regeneration).

> **Never persist a prod connection string to a file.** Always inline
> `psql "$(neonctl connection-string …)"` in one command.

## Procedure

### 1. Rehearse on a throwaway branch (skip only if already done this session)

```bash
neonctl branches create --name ws6-rehome-rehearsal-$(date +%F) --parent main \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
BR=ws6-rehome-rehearsal-$(date +%F)
psql "$(neonctl connection-string $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql   # capture pre
psql "$(neonctl connection-string $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-public.sql
psql "$(neonctl connection-string $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql   # post: public=35/next=0
SQLX_OFFLINE=false DATABASE_URL="$(neonctl connection-string $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" cargo sqlx migrate info --source migrations
psql "$(neonctl connection-string $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-finalize.sql
neonctl branches delete $BR --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
```

**Expect:** pre = post for resources/chunks (the move touches no data); post topology
`public_tables=35, temper_next_tables=0, search_path=public`; `migrate info` all 3 installed,
no checksum mismatch; finalize drops `temper_next`.

### 2. Capture the prod pre-state baseline

```bash
psql "$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql
```

Record `visible_resources` and `vector_chunks` — these are the **invariant** the post-verify
must reproduce exactly.

### 3. Take a fresh backup branch

```bash
neonctl branches create --name ws6-rehome-backup-$(date +%F) --parent main \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
```

This is the point-in-time rollback target.

### 4. GO / NO-GO

Confirm: rehearsal green, pre-state baseline recorded, backup branch `ready`. Proceed only on
an explicit GO.

### 5. Execute steps 1–5 on prod

```bash
psql "$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-public.sql
```

**Expect:** `Pre-state OK` → `Post-state OK: 35 public tables, 0 temper_next tables` → `COMMIT`.

### 6. Post-verify — SQL + live app

```bash
psql "$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql
SQLX_OFFLINE=false DATABASE_URL="$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" cargo sqlx migrate info --source migrations
curl -fsS https://temperkb.io/api/health
temper search "knowledge" --context temper --format json | head -20
temper resource list --type session --context temper | head
```

**Expect:** resources/chunks == pre-state baseline; `migrate info` clean; health 200; search and
list return rows over a fresh app→DB connection on the reverted `public` default. **If anything
here fails, do NOT finalize — go to Rollback.**

### 7. Finalize — drop `temper_next` (point of no cheap return)

Only if step 6 is fully green:

```bash
psql "$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-finalize.sql
psql "$(neonctl connection-string main --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner)" -v ON_ERROR_STOP=1 -c "SELECT nspname FROM pg_namespace WHERE nspname IN ('public','temper_next');"
```

**Expect:** only `public` remains.

## Rollback

- **Before finalize (step 7):** the move already put the canonical tables in `public`, so the
  app keeps working. If a partial/odd state appears, re-apply the safety net and investigate:
  ```bash
  psql "$(neonctl connection-string main … --role-name neondb_owner)" -c "ALTER DATABASE neondb SET search_path TO temper_next, public;"
  ```
- **Catastrophic:** restore prod from `ws6-rehome-backup-<date>` via Neon branch restore.
- **After finalize:** `temper_next` is gone; recovery is the backup branch only. This is why
  finalize runs last, after a green post-verify.
