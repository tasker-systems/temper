# WS6 Flip Dump→Synthesize→Load Runbook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the tooling + runbook that performs the WS6 hard cutover by running synthesis on a local PG17 container and moving only the `temper_next` namespace over the WAN in bulk — no in-region runner.

**Architecture:** A throwaway PG17 pgvector container (`docker-compose.flip.yml`, port 5438) clones prod's `public` schema; synthesis runs against localhost (fast, atomic risk dissolved); the resulting `temper_next` schema+data is dumped and DROP/recreated onto prod. Three `cargo make` guard-rail tasks wrap the footgun-prone bulk steps (`flip-dump-public`, `flip-synthesize-local`, `flip-load-next`); the snapshot, flag-flip, redeploy, verify, and rollback stay explicit manual steps in an operator runbook. The same scripts drive both the Neon-branch rehearsal (step D) and the real flip (step E) — only the connection strings differ.

**Tech Stack:** Docker Compose, PostgreSQL 17 (pgvector), `neonctl`, `pg_dump`/`pg_restore`/`psql` (client v17), cargo-make, the existing `temper-next synthesize` binary.

## Global Constraints

- **Local synthesis target is PostgreSQL 17**, not the PG18 dev DB on :5437 — keeps every restore on the safe newer-reads-older direction (Neon = PG17). Flip container runs on **:5438**.
- **Round-trip scope is `temper_next` only** (schema + data). `public.*` is synthesis *input*, dumped out only; it is never mutated on prod (frozen + read-only by synthesis).
- **Land-back is full-schema** `DROP SCHEMA temper_next CASCADE` + restore, in one transaction — not data-only. Removes the schema-drift assumption and avoids trigger gymnastics (`pg_dump` orders triggers after the `COPY`).
- **Write-freeze is operator discipline** (single-user); the Neon snapshot taken at freeze is the rollback net. `public.*` stays dormant in place — rename-aside/drop is the separate migration-endgame spec.
- **`uuid_generate_v7()`** in the flip container comes from the portable shim `tools/flip/uuid_portable.sql`, loaded before the artifact — a bare PG17 image has neither `pg_uuidv7` nor native `uuidv7()`.
- **Connection strings are parameters**, never hardcoded to prod: `FLIP_SOURCE_URL` (dump `public` from), `FLIP_TARGET_URL` (load `temper_next` into), `FLIP_LOCAL_URL` (the :5438 container, default `postgresql://temper:temper@localhost:5438/temper_development`).
- **`temper-next synthesize` already bails (nonzero exit) on §8 body-parity mismatch** (`crates/temper-next/src/synthesis/mod.rs:364`) — scripts trust the exit code; no output parsing.

**Spec:** `docs/superpowers/specs/2026-06-21-ws6-flip-dump-synthesize-load-runbook-design.md`

---

### Task 1: PG17 flip container + portable uuid shim

Stands up the throwaway PG17 container and the uuid shim it needs. This is infrastructure tooling, not application code — verification is "bring it up and prove the capabilities the later scripts depend on," not a unit test.

**Files:**
- Create: `docker-compose.flip.yml`
- Create: `tools/flip/uuid_portable.sql`
- Modify: `Makefile.toml` (add `flip-db-up`, `flip-db-down`)

**Interfaces:**
- Produces: a running PG17 server at `FLIP_LOCAL_URL` (default `postgresql://temper:temper@localhost:5438/temper_development`) with the `vector` extension and a working `public.uuid_generate_v7()`. Consumed by Tasks 2 and 3.

- [ ] **Step 1: Create the compose file**

`docker-compose.flip.yml`:
```yaml
# Throwaway PG17 container for the WS6 flip (dump → local-synthesize → load-back).
# PG17 matches Neon prod (the dev DB in docker-compose.yml is PG18 on :5437, wrong
# major version for the flip). Port 5438 so it coexists with the dev DB. Ephemeral:
# `up -d` for the flip, `down -v` after — holds only transient prod data.
name: temper-flip

services:
  temper-flip-postgres:
    image: pgvector/pgvector:0.8.2-pg17-trixie
    container_name: temper-flip-postgres
    environment:
      POSTGRES_DB: temper_development
      POSTGRES_USER: temper
      POSTGRES_PASSWORD: temper
      POSTGRES_HOST_AUTH_METHOD: trust
    ports:
      - "5438:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U temper -d temper_development"]
      interval: 5s
      timeout: 3s
      retries: 5
```

- [ ] **Step 2: Create the portable uuid shim**

`tools/flip/uuid_portable.sql`:
```sql
-- Portable public.uuid_generate_v7() for the PG17 flip container.
--
-- The flip container is a bare pgvector PG17 image: it has neither Neon's
-- `pg_uuidv7` extension nor PG18's native `uuidv7()`, and prod's `public` dump
-- supplies `uuid_generate_v7()` via the pg_uuidv7 EXTENSION (whose CREATE EXTENSION
-- cannot restore here). Synthesis calls uuid_generate_v7() for internal chunk/event/
-- revision ids (external resource/profile/context ids are preserved verbatim), so
-- the container needs a working generator. These internal ids only need to be valid,
-- unique, time-sortable v7 UUIDs — the exact bytes are immaterial.
--
-- Build a v7 UUID from current epoch millis (48-bit timestamp) + random bits, with
-- the version (7) and variant (10) nibbles set per RFC 9562.
CREATE OR REPLACE FUNCTION public.uuid_generate_v7() RETURNS uuid
LANGUAGE sql VOLATILE PARALLEL SAFE AS $$
  SELECT encode(
    set_bit(
      set_bit(
        overlay(
          uuid_send(gen_random_uuid())
          PLACING substring(int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint) FROM 3)
          FROM 1 FOR 6
        ),
        52, 0
      ),
      53, 1
    ),
    'hex'
  )::uuid;
$$;
```

- [ ] **Step 3: Add the up/down cargo-make tasks**

In `Makefile.toml`, add:
```toml
[tasks.flip-db-up]
description = "Bring up the throwaway PG17 flip container (:5438) + load the portable uuid shim"
script = '''
set -euo pipefail
docker compose -f docker-compose.flip.yml up -d --wait
LOCAL="${FLIP_LOCAL_URL:-postgresql://temper:temper@localhost:5438/temper_development}"
psql "$LOCAL" -v ON_ERROR_STOP=1 -c 'CREATE EXTENSION IF NOT EXISTS vector;'
psql "$LOCAL" -v ON_ERROR_STOP=1 -f tools/flip/uuid_portable.sql
echo "flip container up at $LOCAL (vector + uuid_generate_v7 ready)"
'''

[tasks.flip-db-down]
description = "Tear down the throwaway PG17 flip container and its volume"
script = '''
docker compose -f docker-compose.flip.yml down -v
'''
```

- [ ] **Step 4: Bring it up and verify PG17 + extensions + uuid**

Run:
```bash
cargo make flip-db-up
U=postgresql://temper:temper@localhost:5438/temper_development
psql "$U" -tAc "SHOW server_version;"
psql "$U" -tAc "SELECT extname FROM pg_extension WHERE extname='vector';"
# version nibble must be 7 for every sample (do NOT use `uuid_generate_v7() < uuid_generate_v7()`
# inline — within one millisecond the timestamp prefix is identical and random bits decide order,
# so it flakes ~50/50). Check the version nibble + a deterministic cross-ms compare instead:
psql "$U" -tAc "SELECT string_agg(substring(uuid_generate_v7()::text from 15 for 1),'') FROM generate_series(1,5);"
A=$(psql "$U" -tAc "SELECT uuid_generate_v7();"); psql "$U" -tAc "SELECT pg_sleep(0.005);" >/dev/null
B=$(psql "$U" -tAc "SELECT uuid_generate_v7();"); psql "$U" -tAc "SELECT '$A'::uuid < '$B'::uuid;"
```
Expected: `server_version` starts `17`; `vector`; `77777` (every sample is UUID version 7); `t` (the earlier uuid sorts first across a millisecond boundary).

> If `pgvector/pgvector:0.8.2-pg17-trixie` 404s on pull, fall back to `pgvector/pgvector:pg17` in the compose file and re-run.

- [ ] **Step 5: Commit**

```bash
git add docker-compose.flip.yml tools/flip/uuid_portable.sql Makefile.toml
git commit -m "WS6 flip: throwaway PG17 container (:5438) + portable uuid_generate_v7 shim"
```

---

### Task 2: `flip-dump-public` — clone prod `public` into the container

**Files:**
- Modify: `Makefile.toml` (add `flip-dump-public`)

**Interfaces:**
- Consumes: a running flip container (Task 1); `FLIP_SOURCE_URL` env (prod or a Neon rehearsal branch).
- Produces: the flip container's `public` schema populated with the source's data, ready for synthesis input.

- [ ] **Step 1: Add the task**

In `Makefile.toml`, add:
```toml
[tasks.flip-dump-public]
description = "Dump the source's public schema and restore it into the :5438 flip container (synthesis input)"
script = '''
set -euo pipefail
: "${FLIP_SOURCE_URL:?set FLIP_SOURCE_URL to the prod/branch connection string to dump public from}"
LOCAL="${FLIP_LOCAL_URL:-postgresql://temper:temper@localhost:5438/temper_development}"

# Guard: pg client tools must be v17 (matches the PG17 source + PG17 container;
# avoids a newer-server dump that can't restore into PG17).
ver="$(pg_dump --version | grep -oE '[0-9]+' | head -1)"
[ "$ver" = "17" ] || { echo "ERROR: pg_dump must be v17 (got $ver). Put PG17 client tools on PATH."; exit 1; }

DUMP="$(mktemp -t flip_public_XXXX).dump"
echo "dumping public from source → $DUMP"
pg_dump "$FLIP_SOURCE_URL" --schema=public --no-owner --no-privileges -Fc -f "$DUMP"

echo "restoring public into $LOCAL"
# pg_restore continues past non-fatal errors by default (e.g. CREATE EXTENSION
# pg_uuidv7, which the container can't satisfy — uuid_generate_v7 is already
# provided by the Task-1 shim). --clean --if-exists makes re-runs idempotent.
pg_restore -d "$LOCAL" --no-owner --no-privileges --clean --if-exists "$DUMP" || true

# Re-assert the portable uuid shim AFTER restore in case --clean dropped a same-named
# object from the dump, so synthesis always has a working generator.
psql "$LOCAL" -v ON_ERROR_STOP=1 -f tools/flip/uuid_portable.sql
echo "public restored. rows: $(psql "$LOCAL" -tAc 'SELECT count(*) FROM public.kb_resources WHERE is_active')"
'''
```

- [ ] **Step 2: Verify against the local dev DB as a stand-in source**

The dev DB on :5437 has a real `public` schema — use it to exercise the dump→restore plumbing without Neon credentials. (This is the PG18→PG17 *risky* direction, acceptable for a plumbing smoke test only; the real flip and rehearsal use a PG17 source.)

Run:
```bash
cargo make docker-up   # ensure dev DB on :5437 is up
FLIP_SOURCE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo make flip-dump-public
```
Expected: ends with `public restored. rows: <N>` where `<N>` matches the dev DB's active resource count:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -tAc 'SELECT count(*) FROM public.kb_resources WHERE is_active'
```

- [ ] **Step 3: Confirm `uuid_generate_v7()` still works post-restore**

Run:
```bash
psql postgresql://temper:temper@localhost:5438/temper_development -tAc "SELECT uuid_generate_v7() IS NOT NULL;"
```
Expected: `t`.

- [ ] **Step 4: Commit**

```bash
git add Makefile.toml
git commit -m "WS6 flip: flip-dump-public — clone source public into the PG17 container"
```

---

### Task 3: `flip-synthesize-local` — build `temper_next` on localhost

**Files:**
- Modify: `Makefile.toml` (add `flip-synthesize-local`)

**Interfaces:**
- Consumes: the flip container with `public` populated (Task 2); the schema artifact `schema-artifact/{00_namespace_reset,01_schema,02_functions,03_seed}.sql`; the `temper-next` binary.
- Produces: the flip container's `temper_next` schema populated with synthesized resources/properties/edges, §8 body-parity verified clean (the binary bails otherwise).

- [ ] **Step 1: Add the task**

In `Makefile.toml`, add:
```toml
[tasks.flip-synthesize-local]
description = "Load a clean temper_next artifact into the flip container and synthesize from its public state"
script = '''
set -euo pipefail
LOCAL="${FLIP_LOCAL_URL:-postgresql://temper:temper@localhost:5438/temper_development}"

# Clean temper_next: 00 resets the namespace (DROP/CREATE temper_next only — public
# untouched), 01/02 install schema+functions, 03 seeds system rows. This is the
# SAME source the synthesis tests load, so the local temper_next schema is known and
# deterministic regardless of the source's temper_next state.
for f in 00_namespace_reset 01_schema 02_functions 03_seed; do
  echo "loading schema-artifact/$f.sql"
  psql "$LOCAL" -v ON_ERROR_STOP=1 -f "schema-artifact/$f.sql" >/dev/null
done

# Synthesize from the restored public state. The binary sets search_path=temper_next,
# public per connection and runs resource/property/edge passes + the §8 body-parity
# gate, which bails (nonzero exit) on any mismatch — so a clean exit IS the proof.
echo "synthesizing (all rows)…"
DATABASE_URL="$LOCAL" cargo run -p temper-next --bin temper-next -- synthesize
'''
```

- [ ] **Step 2: Run synthesis against the Task-2 data**

Run (after Task 2 has populated `public` in the container):
```bash
cargo make flip-synthesize-local
```
Expected: ends with `synthesized: N resource(s), M property(ies), K edge(s)` and a **zero exit code**. If §8 parity fails, the binary prints `body-text parity gate failed (§8): …` and exits nonzero — that is a real finding, not a flake.

- [ ] **Step 3: Verify `temper_next` is populated**

Run:
```bash
psql postgresql://temper:temper@localhost:5438/temper_development -tAc "SELECT count(*) FROM temper_next.kb_resources;"
```
Expected: a count matching the `N` reported by synthesize (equal to the active resource count cloned in Task 2).

- [ ] **Step 4: Commit**

```bash
git add Makefile.toml
git commit -m "WS6 flip: flip-synthesize-local — clean artifact + synthesize on localhost"
```

---

### Task 4: `flip-load-next` — DROP/recreate `temper_next` on the target

**Files:**
- Modify: `Makefile.toml` (add `flip-load-next`)

**Interfaces:**
- Consumes: the flip container with `temper_next` synthesized (Task 3); `FLIP_TARGET_URL` env (prod or the same Neon rehearsal branch).
- Produces: the target's `temper_next` schema replaced wholesale with an exact replica of the local one. The target's `public.*` and the `kb_backend_selection` flag are untouched.

- [ ] **Step 1: Add the task**

In `Makefile.toml`, add:
```toml
[tasks.flip-load-next]
description = "Dump the local temper_next (schema+data) and DROP/recreate it on the target, in one transaction"
script = '''
set -euo pipefail
: "${FLIP_TARGET_URL:?set FLIP_TARGET_URL to the prod/branch connection string to load temper_next into}"
LOCAL="${FLIP_LOCAL_URL:-postgresql://temper:temper@localhost:5438/temper_development}"

ver="$(pg_dump --version | grep -oE '[0-9]+' | head -1)"
[ "$ver" = "17" ] || { echo "ERROR: pg_dump must be v17 (got $ver)."; exit 1; }

# Full dump (schema + data) of ONLY temper_next. pg_dump emits CREATE TRIGGER AFTER
# the COPY data, so triggers never fire during the load — no double-apply, no
# session_replication_role needed.
DUMP="$(mktemp -t flip_next_XXXX).sql"
echo "dumping temper_next (schema+data) → $DUMP"
pg_dump "$LOCAL" --schema=temper_next --no-owner --no-privileges -f "$DUMP"

# Replace the target's dormant temper_next wholesale, in one transaction. Safe because
# the flag is still 'legacy' (temper_next unread) and the operator has taken a Neon
# snapshot. --single-transaction => all-or-nothing; a failure leaves temper_next as-was.
echo "DROP+restore temper_next on target (single transaction)…"
{ echo "DROP SCHEMA IF EXISTS temper_next CASCADE;"; cat "$DUMP"; } \
  | psql "$FLIP_TARGET_URL" -v ON_ERROR_STOP=1 --single-transaction
echo "target temper_next replaced. rows: $(psql "$FLIP_TARGET_URL" -tAc 'SELECT count(*) FROM temper_next.kb_resources')"
'''
```

- [ ] **Step 2: Verify a full local round-trip onto a second local DB**

Use the dev DB on :5437 as a stand-in *target* to prove the dump + DROP/recreate + transaction wrapping work end to end without touching prod. First give :5437 a temper_next to overwrite, then load the container's into it.

Run:
```bash
# Seed an arbitrary temper_next on the stand-in target so DROP has something to drop.
for f in 00_namespace_reset 01_schema 02_functions 03_seed; do \
  psql postgresql://temper:temper@localhost:5437/temper_development -v ON_ERROR_STOP=1 -f "schema-artifact/$f.sql" >/dev/null; done
FLIP_TARGET_URL=postgresql://temper:temper@localhost:5437/temper_development cargo make flip-load-next
```
Expected: ends with `target temper_next replaced. rows: <N>` where `<N>` equals the container's `temper_next.kb_resources` count from Task 3 Step 3.

- [ ] **Step 3: Confirm the target's `public` and flag were untouched**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -tAc "SELECT count(*) FROM public.kb_resources;"
psql postgresql://temper:temper@localhost:5437/temper_development -tAc "SELECT backend FROM kb_backend_selection WHERE id=true;"
```
Expected: the `public` count is unchanged from before the load, and `backend` is still `legacy` — `flip-load-next` never touches either.

- [ ] **Step 4: Commit**

```bash
git add Makefile.toml
git commit -m "WS6 flip: flip-load-next — DROP/recreate temper_next on the target in one txn"
```

---

### Task 5: Operator runbook

The human-facing procedure that orders the three scripts among the irreversible manual steps (snapshot, flag-flip, redeploy, verify, rollback). Distinct from the design spec: this is the checklist the operator follows on flip night.

**Files:**
- Create: `docs/guides/ws6-flip-runbook.md`

**Interfaces:**
- Consumes: `flip-db-up`, `flip-dump-public`, `flip-synthesize-local`, `flip-load-next` (Tasks 1–4); `neonctl`.

- [ ] **Step 1: Write the runbook**

Create `docs/guides/ws6-flip-runbook.md` with these sections, each a numbered checklist the operator ticks:

1. **Pre-flight** — green branch-rehearsal on record; `neonctl projects list` works; PG17 client tools on PATH (`pg_dump --version` → 17); `cargo make flip-db-up` succeeds; disk headroom.
2. **Freeze** — operator stops writing; confirm no autonomous writer (ingest/cron/agent) is live.
3. **Snapshot** — `neonctl branches create --name flip-rollback-<date>` from main; **record the branch name/id** (the rollback target).
4. **Dump public → local** — `export FLIP_SOURCE_URL=<prod conn>`; `cargo make flip-dump-public`.
5. **Synthesize locally** — `cargo make flip-synthesize-local`; **must exit 0** (clean §8 parity).
6. **Load temper_next → prod** — `export FLIP_TARGET_URL=<prod conn>`; `cargo make flip-load-next`.
7. **Cutover** — `psql "$FLIP_TARGET_URL" -c "UPDATE kb_backend_selection SET backend='next' WHERE id=true;"`, then **redeploy** the Vercel app (api + mcp) — the flag is read once at API startup (`crates/temper-api/src/main.rs:34`).
8. **Verify** — `temper resource list` / `show` / `search` + a graph read against prod return expected rows from `temper_next`.
9. **Unfreeze** — resume writes (now landing in `temper_next`).
10. **Cleanup** — `cargo make flip-db-down`; retain the snapshot branch for a retention window. `public.*` left dormant (drop = migration-endgame spec).

Include a **Rollback** section: `UPDATE kb_backend_selection SET backend='legacy' WHERE id=true` + redeploy → back on the untouched legacy `public.*`; if deeper, restore the snapshot branch from step 3.

Include a **Rehearsal (step D)** note at top: run steps 1, 4–6, 8 with `FLIP_SOURCE_URL`/`FLIP_TARGET_URL` both pointed at a fresh `neonctl branches create` branch, skipping freeze/snapshot/cutover/redeploy. Throw the branch away after.

- [ ] **Step 2: Verify the runbook references resolve**

Run:
```bash
grep -nE 'flip-db-up|flip-dump-public|flip-synthesize-local|flip-load-next' docs/guides/ws6-flip-runbook.md
grep -n "kb_backend_selection" docs/guides/ws6-flip-runbook.md
```
Expected: every script name and the flag table appear; no task name is misspelled relative to `Makefile.toml`.

- [ ] **Step 3: Commit**

```bash
git add docs/guides/ws6-flip-runbook.md
git commit -m "WS6 flip: operator runbook (freeze → snapshot → dump → synth → load → cutover → verify)"
```

---

### Task 6: Real-data rehearsal on a fresh Neon branch (acceptance gate)

The confidence gate the user asked for: prove the whole chain works against the **real prod corpus**, not a fixture. This is also step D of the flip sequence. No prod mutation — everything targets a throwaway Neon branch.

**Files:** none (operational verification; findings may feed fixes back into Tasks 1–4).

**Interfaces:**
- Consumes: all four tasks + the runbook.

- [ ] **Step 1: Create a rehearsal branch from prod**

Run:
```bash
neonctl branches create --name flip-rehearsal-$(date +%Y%m%d) --output json
```
Capture the branch's pooled connection string as `REH=<conn>`.

- [ ] **Step 2: Run the chain against the branch**

Run:
```bash
cargo make flip-db-down ; cargo make flip-db-up
FLIP_SOURCE_URL="$REH" cargo make flip-dump-public
cargo make flip-synthesize-local
FLIP_TARGET_URL="$REH" cargo make flip-load-next
```
Expected: `flip-synthesize-local` exits 0 (clean §8 parity on the **real corpus**); `flip-load-next` ends with a `temper_next.kb_resources` count equal to the branch's active `public.kb_resources` count.

- [ ] **Step 3: Prove a NextBackend read serves from the branch's `temper_next`**

Point a local API at the branch with the next backend selected and read through it:
```bash
psql "$REH" -c "UPDATE kb_backend_selection SET backend='next' WHERE id=true;"
DATABASE_URL="$REH" cargo run -p temper-api &   # or the §9 read-floor parity harness from step D
# then: temper resource list / show / search against it
```
Expected: list/show/search/graph return the expected rows from `temper_next` — the §9 read-floor holds on production-shaped data. Stop the API afterward.

- [ ] **Step 4: Tear down**

Run:
```bash
neonctl branches delete <branch-id>
cargo make flip-db-down
```
Expected: branch and container gone.

- [ ] **Step 5: Record the rehearsal outcome**

If the chain was clean, the flip tooling is accepted and the real flip (step E) is unblocked pending an explicit go-ahead. If anything diverged (parity mismatch, missing extension/function, schema drift on load), capture it as a finding and fix the relevant Task-1–4 script before re-running this gate. Note the outcome in the session save.

---

## Self-Review

**Spec coverage:**
- Throwaway PG17 container (:5438) → Task 1 ✓
- Portable `uuid_generate_v7()` shim → Task 1 ✓
- `flip-dump-public` (public → container) → Task 2 ✓
- `flip-synthesize-local` (artifact + synthesize, §8 gate) → Task 3 ✓
- `flip-load-next` (full schema DROP/recreate, one txn) → Task 4 ✓
- Operator runbook (freeze/snapshot/cutover/redeploy/verify/rollback, deferred rename-aside) → Task 5 ✓
- One-runbook-two-uses (branch rehearsal = step D) → Task 5 note + Task 6 ✓
- Real-data confidence gate → Task 6 ✓
- Connection-string parameterization (never hardcode prod) → Global Constraints + every task ✓
- PG17-only client guard → Tasks 2, 4 ✓

**Placeholder scan:** No TBD/TODO/stray lines; every script is complete and runnable as written.

**Type/name consistency:** Task names (`flip-db-up`, `flip-db-down`, `flip-dump-public`, `flip-synthesize-local`, `flip-load-next`) and env vars (`FLIP_SOURCE_URL`, `FLIP_TARGET_URL`, `FLIP_LOCAL_URL`) are used identically across the Makefile tasks, the runbook (Task 5), and the rehearsal (Task 6). Artifact filenames (`00_namespace_reset`/`01_schema`/`02_functions`/`03_seed`) match `schema-artifact/`. The flag table/column (`kb_backend_selection`/`backend`/`id=true`) matches the verified prod query.
