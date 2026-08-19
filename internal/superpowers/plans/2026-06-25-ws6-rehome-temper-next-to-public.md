# WS6 re-home `temper_next` → `public` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the post-flip split-schema prod state — live data in `temper_next`, dead legacy in `public`, a `search_path` default carrying the indirection — into a single unified `public` schema, and prove `migrations/` reproduces it from scratch.

**Architecture:** A one-shot operational transition (NOT a sqlx migration). `scripts/ws6-rehome-public.sql` drops legacy `public` objects and relocates the canonical objects from `temper_next` into `public` via `ALTER … SET SCHEMA` (extensions never move — they already live in `public`), rewrites `_sqlx_migrations` to the 3 canonical baseline rows, and reverts the `search_path` default. A separate `scripts/ws6-rehome-finalize.sql` drops the emptied `temper_next` last, only after post-verify. Validated end-to-end on a throwaway Neon branch before a go/no-go-gated prod run.

**Tech Stack:** PostgreSQL 17 (Neon prod) / 18 (local Docker), `psql`, `neonctl`, `sqlx`, `cargo-make`.

## Global Constraints

- **Prod is single-user.** Blast radius is contained; downtime acceptable. Still: backup branch + go/no-go gate before any prod write. (`project_arc1_breaking_change_branch_posture`)
- **Never persist a prod connection string to a file.** Always inline `psql "$(neonctl connection-string …)"` in one command. (Credential-leakage classifier blocks file writes.)
- **Neon coordinates:** project-id `crimson-fog-23541670`, org-id `org-wild-snow-32921543`, prod branch `main`, role `neondb_owner`.
- **Admin DB ops are run by the orchestrator directly, never delegated to a thinly-briefed subagent.** (`feedback_admin_cli_no_lossy_subagents`) — destructive prod steps (Task 6) are NOT subagent-dispatchable.
- **Extensions (`vector`, `pg_uuidv7`) stay in `public`.** They are never dropped, never moved. The drop filters exclude extension-owned objects via `pg_depend.deptype = 'e'`.
- **`migrations/` is the from-scratch truth.** It is NOT modified by the re-home; only Task 4 may patch it to fix a genuine parity drift.
- **Baseline verification numbers (prod, 2026-06-25):** visible resources = **1264**, vector chunks = **14840**. Captured fresh in Task 2; these are the expected post-re-home values.

### Reusable connection helpers (used throughout)

```bash
# Prod (main)
PROD='--project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 --role-name neondb_owner'
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f <file>

# A child branch (rehearsal/backup), same flags with a different branch name
psql "$(neonctl connection-string <branch> $PROD)" -v ON_ERROR_STOP=1 -f <file>

# Local fresh DB (for checksum capture + parity reference)
LOCAL='postgresql://temper:temper@localhost:5437/temper_development'
```

---

## File Structure

- `scripts/ws6-rehome-public.sql` — **Create.** Steps 1–5: drop legacy, relocate canonical, reconcile `_sqlx_migrations`, revert `search_path`. Wrapped in a single `BEGIN/COMMIT`.
- `scripts/ws6-rehome-finalize.sql` — **Create.** Step 6: `DROP SCHEMA temper_next CASCADE`. Run last, after post-verify.
- `scripts/ws6-rehome-verify.sql` — **Create.** The 5-probe verification set, reused for rehearsal post-check and prod post-verify.
- `scripts/ws6-rehome-sqlx-baseline.sql` — **Create.** The captured, ready-to-run `INSERT` of the 3 canonical `_sqlx_migrations` rows (generated in Task 1, pasted into `ws6-rehome-public.sql`).
- `docs/guides/ws6-rehome-to-public-runbook.md` — **Create.** Operator runbook.
- `docs/guides/ws6-flip-runbook.md` — **Modify.** Superseded banner.
- `docs/guides/ws6-endgame-collapse-runbook.md` — **Modify.** Superseded banner.
- `docs/superpowers/specs/2026-06-25-ws6-parity-report.md` — **Create (Task 4).** The `migrations/` ↔ prod schema diff adjudication.
- `migrations/*` — **Modify only if Task 4 finds genuine drift.**

---

## Task 1: Capture the canonical `_sqlx_migrations` baseline rows

A fresh local DB migrated from `migrations/` produces `_sqlx_migrations` with exactly the 3 canonical rows and their **real sqlx-computed checksums**. We capture them as a ready-to-run `INSERT` (no hand-faked checksums).

**Files:**
- Create: `scripts/ws6-rehome-sqlx-baseline.sql`

**Interfaces:**
- Produces: `scripts/ws6-rehome-sqlx-baseline.sql` — a single `INSERT INTO public._sqlx_migrations (...) VALUES (...);` with 3 rows, consumed verbatim by Task 3's Step 4.

- [ ] **Step 1: Reset the local DB from `migrations/`**

```bash
cargo make docker-up
cargo make db-reset   # drops, recreates, runs migrations/ (the 3 canonical files)
```

- [ ] **Step 2: Verify the local ledger has exactly the 3 canonical rows**

```bash
psql "$LOCAL" -c "SELECT version, description FROM public._sqlx_migrations ORDER BY version;"
```

Expected: exactly 3 rows — `20260624000001`, `20260624000002`, `20260624000003`. If more/fewer, STOP: `migrations/` is not the clean 3-file set on this machine.

- [ ] **Step 3: Generate the ready-to-run INSERT artifact**

```bash
psql "$LOCAL" -At -o scripts/ws6-rehome-sqlx-baseline.sql <<'SQL'
SELECT
  E'-- Canonical _sqlx_migrations baseline (captured from a clean `cargo make db-reset`).\n'
  || E'-- Real sqlx checksums — do not edit by hand.\n'
  || 'INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES'
  || E'\n'
  || string_agg(
       format('  (%s, %L, now(), true, %L::bytea, %s)',
              version, description, '\x' || encode(checksum,'hex'), execution_time),
       E',\n' ORDER BY version)
  || ';';
SQL
cat scripts/ws6-rehome-sqlx-baseline.sql
```

Expected: a 3-row `INSERT` with `'\x...'::bytea` checksums. Eyeball that each checksum is a long hex string and the descriptions match the migration filenames.

- [ ] **Step 4: Commit**

```bash
git add scripts/ws6-rehome-sqlx-baseline.sql
git commit -m "WS6 re-home: capture canonical _sqlx_migrations baseline (real sqlx checksums)"
```

---

## Task 2: Author the reusable verification probe set + capture prod baseline

Five probes that define "healthy." Reused for rehearsal post-check (Task 3) and prod post-verify (Task 6). Authored to run identically against any branch.

**Files:**
- Create: `scripts/ws6-rehome-verify.sql`

**Interfaces:**
- Produces: `scripts/ws6-rehome-verify.sql` — read-only; prints labelled counts. Consumed by Tasks 3 and 6.

- [ ] **Step 1: Write the verify script**

```sql
-- scripts/ws6-rehome-verify.sql — read-only health probes (schema-agnostic; resolves via search_path)
\echo === probe 1: visible resources (expect 1264) ===
SELECT count(*) AS visible_resources FROM kb_resources WHERE is_active = true;

\echo === probe 2: vector chunks (expect 14840) ===
SELECT count(*) AS vector_chunks FROM kb_chunks;

\echo === probe 3: a representative search join resolves (expect > 0 rows, no error) ===
SELECT count(*) AS joinable_chunk_content FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id;

\echo === probe 4: schema topology (expect target shape) ===
SELECT
  (SELECT count(*) FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations') AS public_tables,
  (SELECT count(*) FROM pg_tables WHERE schemaname='temper_next') AS temper_next_tables,
  (SELECT setting FROM pg_settings WHERE name='search_path') AS session_search_path;

\echo === probe 5: extensions intact in public ===
SELECT extname, n.nspname FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace
WHERE extname IN ('vector','pg_uuidv7') ORDER BY 1;
```

- [ ] **Step 2: Run against current prod to capture the pre-state baseline**

```bash
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql
```

Expected (pre-re-home): probe 1 = 1264, probe 2 = 14840, probe 3 > 0, probe 4 = `public_tables≈27, temper_next_tables=35, session_search_path='temper_next, public'`, probe 5 = both extensions in `public`. Record these numbers in the commit message.

- [ ] **Step 3: Commit**

```bash
git add scripts/ws6-rehome-verify.sql
git commit -m "WS6 re-home: verification probe set + prod pre-state baseline (1264 resources / 14840 chunks)"
```

---

## Task 3: Author the re-home scripts and rehearse end-to-end on a Neon branch

The core deliverable. Author `ws6-rehome-public.sql` (steps 1–5, one transaction) + `ws6-rehome-finalize.sql` (step 6), then prove the whole thing on a throwaway branch.

**Files:**
- Create: `scripts/ws6-rehome-public.sql`
- Create: `scripts/ws6-rehome-finalize.sql`

**Interfaces:**
- Consumes: `scripts/ws6-rehome-sqlx-baseline.sql` (Task 1), `scripts/ws6-rehome-verify.sql` (Task 2).
- Produces: the two prod-ready transition scripts, consumed by Task 6.

- [ ] **Step 1: Body-qualification gate — confirm canonical functions are namespace-free**

```bash
grep -nE 'temper_next\.|[^a-z_]public\.' migrations/20260624000002_canonical_functions.sql | grep -v '^\s*--'
```

Expected: **no output** (function bodies carry no schema-qualified literals, so they resolve under `search_path=public` after the move). If any line appears, STOP and assess — the move would leave a dangling reference.

- [ ] **Step 2: Write `scripts/ws6-rehome-public.sql` (steps 1–5)**

```sql
-- scripts/ws6-rehome-public.sql
-- WS6 re-home: temper_next -> public. Steps 1–5, atomic. Idempotent guards throughout.
-- Run with: psql "<conn>" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-public.sql
-- Does NOT drop temper_next (see ws6-rehome-finalize.sql) and does NOT need fully-qualified
-- names — but every statement here IS schema-qualified for safety under any search_path.

BEGIN;

-- ---- Guard: assert the expected split state before mutating ----
DO $$
DECLARE legacy_n int; next_n int;
BEGIN
  SELECT count(*) INTO legacy_n FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations';
  SELECT count(*) INTO next_n   FROM pg_tables WHERE schemaname='temper_next';
  IF next_n = 0 THEN
    RAISE EXCEPTION 'temper_next has no tables — re-home already applied or wrong DB (legacy_n=%, next_n=%)', legacy_n, next_n;
  END IF;
  RAISE NOTICE 'Pre-state OK: % legacy public tables, % temper_next tables', legacy_n, next_n;
END $$;

-- ---- Step 1: drop legacy public data tables (everything except _sqlx_migrations and extension-owned) ----
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

-- ---- Step 2b: drop legacy public enums (non-extension) ----
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

-- ---- Step 3a: relocate canonical enums temper_next -> public (before tables is fine; OID-stable) ----
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

-- ---- Step 3c: relocate canonical functions temper_next -> public ----
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
TRUNCATE public._sqlx_migrations;
-- >>> PASTE the contents of scripts/ws6-rehome-sqlx-baseline.sql HERE (the 3-row INSERT) <<<

-- ---- Step 5: revert the search_path default (drop the flip hack) ----
ALTER DATABASE neondb SET search_path TO public;

-- ---- Post-transaction guard: assert the unified shape ----
DO $$
DECLARE pub_n int; next_n int;
BEGIN
  SELECT count(*) INTO pub_n  FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations';
  SELECT count(*) INTO next_n FROM pg_tables WHERE schemaname='temper_next';
  IF pub_n < 35 OR next_n <> 0 THEN
    RAISE EXCEPTION 'Post-state wrong: public=% (want 35), temper_next=% (want 0)', pub_n, next_n;
  END IF;
  RAISE NOTICE 'Post-state OK: % public tables, % temper_next tables', pub_n, next_n;
END $$;

COMMIT;
```

When writing the file, replace the `>>> PASTE … <<<` marker with the literal `INSERT` from `scripts/ws6-rehome-sqlx-baseline.sql` (Task 1, Step 3).

- [ ] **Step 3: Write `scripts/ws6-rehome-finalize.sql` (step 6)**

```sql
-- scripts/ws6-rehome-finalize.sql
-- WS6 re-home FINALIZE: drop the emptied temper_next. Point of no cheap return.
-- Run ONLY after post-verify on prod passes.
DO $$
DECLARE n int;
BEGIN
  SELECT count(*) INTO n FROM pg_class c JOIN pg_namespace ns ON ns.oid=c.relnamespace
  WHERE ns.nspname='temper_next' AND c.relkind IN ('r','S','v');
  IF n <> 0 THEN
    RAISE EXCEPTION 'temper_next is not empty (% relations) — refusing to drop', n;
  END IF;
END $$;
DROP SCHEMA IF EXISTS temper_next CASCADE;
```

- [ ] **Step 4: Create a fresh rehearsal branch off prod**

```bash
neonctl branches create --name ws6-rehome-rehearsal-2026-06-25 --parent main \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
```

Expected: branch created, state `ready`. (If it already exists from a prior attempt, delete and recreate so the rehearsal starts from current prod state: `neonctl branches delete ws6-rehome-rehearsal-2026-06-25 --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543`.)

- [ ] **Step 5: Run the re-home (steps 1–5) on the rehearsal branch**

```bash
psql "$(neonctl connection-string ws6-rehome-rehearsal-2026-06-25 $PROD)" -v ON_ERROR_STOP=1 \
  -f scripts/ws6-rehome-public.sql
```

Expected: `NOTICE: Pre-state OK …`, `NOTICE: Post-state OK: 35 public tables, 0 temper_next tables`, `COMMIT`. No errors.

- [ ] **Step 6: Run the verify probes on the rehearsal branch**

```bash
psql "$(neonctl connection-string ws6-rehome-rehearsal-2026-06-25 $PROD)" -v ON_ERROR_STOP=1 \
  -f scripts/ws6-rehome-verify.sql
```

Expected post-re-home: probe 1 = **1264**, probe 2 = **14840**, probe 3 > 0, probe 4 = `public_tables=35, temper_next_tables=35` (still present pre-finalize), `session_search_path` unchanged for THIS session (the default change only affects new connections), probe 5 = both extensions in `public`.

- [ ] **Step 7: Confirm the sqlx ledger is clean on the rehearsal branch**

```bash
SQLX_OFFLINE=false DATABASE_URL="$(neonctl connection-string ws6-rehome-rehearsal-2026-06-25 $PROD)" \
  cargo sqlx migrate info --source migrations
```

Expected: all 3 migrations `installed` / no `pending` / no checksum-mismatch warning.

- [ ] **Step 8: Run finalize on the rehearsal branch and re-verify**

```bash
psql "$(neonctl connection-string ws6-rehome-rehearsal-2026-06-25 $PROD)" -v ON_ERROR_STOP=1 \
  -f scripts/ws6-rehome-finalize.sql
psql "$(neonctl connection-string ws6-rehome-rehearsal-2026-06-25 $PROD)" -v ON_ERROR_STOP=1 \
  -c "SELECT count(*) AS temper_next_relations FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='temper_next';"
```

Expected: finalize succeeds; the follow-up count errors or returns 0 (schema gone). Re-run `ws6-rehome-verify.sql` probes 1–3 — still 1264 / 14840 / >0.

- [ ] **Step 9: Commit the scripts**

```bash
git add scripts/ws6-rehome-public.sql scripts/ws6-rehome-finalize.sql
git commit -m "WS6 re-home: transition + finalize scripts, rehearsed green on Neon branch"
```

- [ ] **Step 10: Delete the rehearsal branch (clean up)**

```bash
neonctl branches delete ws6-rehome-rehearsal-2026-06-25 \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
```

---

## Task 4: Prove `migrations/` ≡ post-re-home `public` (parity) and reconcile drift

`migrations/` must reproduce the unified-public schema from scratch. Compare a fresh local migrate against prod's `temper_next` (which is exactly what `public` becomes — `SET SCHEMA` changes the namespace, not the shape).

**Files:**
- Create: `docs/superpowers/specs/2026-06-25-ws6-parity-report.md`
- Modify (only if drift is real): `migrations/*`

**Interfaces:**
- Consumes: a fresh local DB from Task 1's reset.
- Produces: the adjudicated parity report; a green `migrations/` from-scratch build.

- [ ] **Step 1: Dump the fresh-from-`migrations/` reference schema (local)**

```bash
cargo make db-reset
pg_dump "$LOCAL" --schema-only --schema=public --no-owner --no-privileges \
  | grep -v '^--' | grep -v '^SET ' | grep -v '^SELECT pg_catalog' > /tmp/ws6-parity-migrations.sql
```

- [ ] **Step 2: Dump prod's `temper_next` schema, rewritten to `public` for comparison**

```bash
pg_dump "$(neonctl connection-string main $PROD)" --schema-only --schema=temper_next --no-owner --no-privileges \
  | sed 's/temper_next\./public./g; s/SCHEMA temper_next/SCHEMA public/g' \
  | grep -v '^--' | grep -v '^SET ' | grep -v '^SELECT pg_catalog' > /tmp/ws6-parity-prod.sql
```

- [ ] **Step 3: Normalize ordering and diff**

```bash
# Sort statement blocks so member ordering noise doesn't dominate the diff.
for f in /tmp/ws6-parity-migrations.sql /tmp/ws6-parity-prod.sql; do
  awk 'BEGIN{RS=";\n"} {gsub(/\n+/," "); print}' "$f" | sed 's/[[:space:]]\+/ /g' | sort > "$f.norm"
done
diff /tmp/ws6-parity-migrations.sql.norm /tmp/ws6-parity-prod.sql.norm | head -100
```

Expected: ideally empty. Realistically, a handful of differences — adjudicate each in Step 4.

- [ ] **Step 4: Adjudicate every diff line in the parity report**

Write `docs/superpowers/specs/2026-06-25-ws6-parity-report.md`. For each diff hunk, classify:
- **Prod-only operational residue** (e.g. a stray index/trigger created out-of-band) → decide drop-on-prod (add to a noted `ws6-rehome-public.sql` cleanup) or accept-and-document.
- **`migrations/` bug** (the from-scratch build is missing something prod legitimately needs) → fix `migrations/` so a fresh build matches. The from-scratch truth defines the shape (per spec); prod is matched to it, not vice versa.
- **Cosmetic** (whitespace, comment, ordering the normalizer missed) → note and ignore.

Document the verdict per hunk. If zero diffs, the report states "exact parity, no reconciliation needed."

- [ ] **Step 5: If `migrations/` was patched, re-prove a clean from-scratch build**

```bash
cargo make db-reset && cargo make check
```

Expected: migrations apply cleanly; `cargo make check` green (sqlx offline cache still valid — if schema changed, regenerate per CLAUDE.md and re-run).

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-06-25-ws6-parity-report.md migrations/ crates/ 2>/dev/null
git commit -m "WS6 re-home: migrations/ <-> prod parity report + reconciliation"
```

---

## Task 5: Author the re-home runbook and supersede the stale runbooks

**Files:**
- Create: `docs/guides/ws6-rehome-to-public-runbook.md`
- Modify: `docs/guides/ws6-flip-runbook.md` (top banner)
- Modify: `docs/guides/ws6-endgame-collapse-runbook.md` (top banner)

**Interfaces:**
- Consumes: the finalized scripts (Task 3) and verify set (Task 2).

- [ ] **Step 1: Write `docs/guides/ws6-rehome-to-public-runbook.md`**

Content: the operator procedure mirroring Task 6's sequence — prerequisites (neonctl/psql, the 4 scripts), the rehearse→backup→go/no-go→execute→post-verify→finalize flow with the exact `psql`/`neonctl` commands, the expected probe outputs (1264 / 14840), and the rollback section (search-path safety net + backup-branch restore). Note explicitly that the rename-promote in the older runbooks is **Neon-blocked** (extension-ownership wall) and this re-home supersedes it.

- [ ] **Step 2: Add a superseded banner to the two stale runbooks**

At the top of each of `docs/guides/ws6-flip-runbook.md` and `docs/guides/ws6-endgame-collapse-runbook.md`:

```markdown
> **⚠️ SUPERSEDED (2026-06-25).** The rename-promote step described below is **Neon-blocked**
> (the `vector` extension cannot be relocated out of `public` by `neondb_owner`). Production was
> cut over via a search-path flip and then re-homed into `public`. See
> [ws6-rehome-to-public-runbook.md](./ws6-rehome-to-public-runbook.md). This document is retained
> for historical context only.
```

- [ ] **Step 3: Commit**

```bash
git add docs/guides/ws6-rehome-to-public-runbook.md docs/guides/ws6-flip-runbook.md docs/guides/ws6-endgame-collapse-runbook.md
git commit -m "WS6 re-home: operator runbook + supersede the Neon-blocked rename-promote runbooks"
```

---

## Task 6: Execute on production (GATED — orchestrator-only, not subagent-dispatchable)

**Files:** none (operational).

**Interfaces:**
- Consumes: `scripts/ws6-rehome-public.sql`, `scripts/ws6-rehome-finalize.sql`, `scripts/ws6-rehome-verify.sql`.

- [ ] **Step 1: Re-capture the prod pre-state baseline**

```bash
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql
```

Expected: 1264 / 14840 / `temper_next_tables=35` / `search_path='temper_next, public'`. Confirms prod still matches the design assumptions immediately before the write.

- [ ] **Step 2: Take a fresh backup branch**

```bash
neonctl branches create --name ws6-rehome-backup-2026-06-25 --parent main \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543
```

Expected: branch `ready`. This is the point-in-time rollback target.

- [ ] **Step 3: GO/NO-GO checkpoint**

Present to the user: pre-state baseline numbers, the backup branch id, and the rehearsal result. **Wait for explicit "GO" before Step 4.** No subagent may pass this gate.

- [ ] **Step 4: Execute steps 1–5 on prod**

```bash
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-public.sql
```

Expected: `Pre-state OK` → `Post-state OK: 35 public tables, 0 temper_next tables` → `COMMIT`.

- [ ] **Step 5: Post-verify against prod — SQL probes**

```bash
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-verify.sql
SQLX_OFFLINE=false DATABASE_URL="$(neonctl connection-string main $PROD)" cargo sqlx migrate info --source migrations
```

Expected: 1264 / 14840 / >0; `migrate info` all-applied, zero pending, no checksum mismatch.

- [ ] **Step 6: Post-verify against prod — live app (the real test)**

```bash
curl -fsS https://temperkb.io/api/health
temper search "knowledge" --context temper --format json | head -40   # exercises a new app->DB connection on the reverted default
temper resource list --type session --context temper | head -20
```

Expected: health OK; search returns rows; list works. This confirms the **deployed app** resolves the unified `public` over a fresh connection (new default `search_path=public`). If anything fails here, do NOT finalize — use the rollback (Step 8).

- [ ] **Step 7: Finalize — drop `temper_next` (point of no cheap return)**

Only if Steps 5–6 are fully green:

```bash
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 -f scripts/ws6-rehome-finalize.sql
psql "$(neonctl connection-string main $PROD)" -v ON_ERROR_STOP=1 \
  -c "SELECT nspname FROM pg_namespace WHERE nspname IN ('public','temper_next') ORDER BY 1;"
```

Expected: finalize succeeds; only `public` remains (no `temper_next`).

- [ ] **Step 8: Rollback path (only if a step fails)**

- **Before finalize (Step 7):** re-apply the safety net so the still-present `temper_next` is reachable again, then investigate:
  ```bash
  psql "$(neonctl connection-string main $PROD)" -c "ALTER DATABASE neondb SET search_path TO temper_next, public;"
  ```
  (Note: after the move, the tables are in `public` anyway, so the app already works; this only matters if a partial failure left objects split.)
- **Catastrophic:** restore prod from `ws6-rehome-backup-2026-06-25` per Neon branch-restore, or repoint via `neonctl`.

- [ ] **Step 9: Update memory and close out**

Update `project_ws6_flip_already_executed.md` (and add a re-home note) to record: prod is now single-schema `public`, `temper_next` retired, `search_path` default reverted. Save a session note via `temper resource create --type session`.

---

## Self-Review

**Spec coverage:**
- Posture (one-shot script, `migrations/` untouched) → Tasks 3, 5 + Global Constraints. ✓
- DDL steps 1–6 → Task 3 Steps 2–3 (script bodies). ✓
- Extension-owned exclusion (`pg_depend.deptype='e'`) → Task 3 Step 2 (Steps 1, 2a, 2b). ✓
- `_sqlx_migrations` reconciliation with real checksums → Task 1 + Task 3 Step 2 (Step 4). ✓
- Revert search-path / drop temper_next → Task 3 Steps 2 (Step 5) & 3. ✓
- Parity deliverable → Task 4. ✓
- Execution sequencing (rehearse→backup→go/no-go→prod→verify) → Tasks 3 (rehearse) + 6 (prod). ✓
- Rollback → Task 6 Step 8. ✓
- Supersede stale runbooks → Task 5 Step 2. ✓
- Out-of-scope (F1, CD gating) → carried in spec; not planned here. ✓
- Open verification items (trigger check, body-grep gate, checksum capture) → Task 3 Step 1 + Task 1. ✓ (Trigger check is implicitly covered: legacy triggers drop with their tables in Step 1; the post-state guard + verify probes would catch a broken canonical trigger.)

**Placeholder scan:** The only deferred content is the captured `INSERT` (Task 1 → pasted in Task 3) and the per-hunk parity verdicts (Task 4, which is itself an adjudication task by nature). Both have exact generating mechanisms, not vague instructions. No "TBD"/"add error handling"/"similar to" placeholders.

**Type consistency:** Branch names (`ws6-rehome-rehearsal-2026-06-25`, `ws6-rehome-backup-2026-06-25`), script paths, project/org/role ids, and probe expectations (1264 / 14840 / 35 / 0) are used identically across Tasks 2, 3, and 6.
