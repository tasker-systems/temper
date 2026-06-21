# WS6 chunk-5 flip — dump → local-synthesize → load-back runbook

**Date:** 2026-06-21
**Goal:** substrate-kernel-to-cognitive-map
**Task:** WS6 chunk-5 flip — deployed-adapter next-backend enable → hard cutover (step **C**)
**Status:** design approved, pending spec review

## Problem

The WS6 hard cutover needs a final synthesis run that rebuilds the `temper_next`
substrate from current production state. Synthesis (`crates/temper-next/src/synthesis/mod.rs`)
is a **chatty, atomic, non-resumable** operation: it fires thousands of
`events::fire` calls across three sequential passes (resources → properties →
edges) plus a body-parity gate. Run from a laptop against Neon, each fire
round-trips over the WAN — the run is ~100% network wait, and the long-lived
transactions risk being reaped by Neon's idle-in-transaction timeout mid-pass.

The earlier plan answered this with a *co-located in-region synthesis runner* —
new compute pinned to Neon's region so the chatty transaction runs at ~1ms RTT.
That means building and operating a deployment vehicle (a Neon-region container,
a Vercel Sandbox job, or pushing synthesis server-side), none of which exist.

## Approach: run synthesis on localhost, move only bulk data over the WAN

Latency is the enemy, not bandwidth. So:

- Run synthesis on **localhost**, where RTT is ~0. The chatty transaction is fast
  and the idle-in-transaction reaping cannot occur. The atomic/non-resumable
  property stops mattering: a failed local run leaves production frozen and
  untouched — just re-run locally.
- Move data over the WAN only in **bulk** (`pg_dump`/`pg_restore`), which is
  throughput-bound, not latency-bound, and does not suffer the reaping that kills
  a multi-statement transaction.

This **eliminates the in-region runner entirely.** Step C's deliverable collapses
from "build + operate a runner" to a **runbook** plus a few **thin guard-rail
scripts** for the irreversible bulk steps.

### Why the transfer-back is small

Two facts (verified against the schema artifact and migrations) make the load-back
cheap and safe:

1. **`temper_next` is a fully self-contained namespace** — `schema-artifact/01_schema.sql`
   and `02_functions.sql` carry **zero** foreign-key references into `public.*`.
   It can be dumped and loaded independently of the legacy schema.
2. **`temper_next` is already installed on prod Neon** via migrations
   (`20260613000001_install_temper_next.sql` + the `4c_mutations` /
   `can_modify` / `invocation_envelope` follow-ups). Schema, functions, and seed
   are already present in production — only the **synthesized data** is missing.

Because the write-freeze keeps `public.*` stable, the `temper_next` we build
locally is consistent with production's `public.*`. So the only thing that
crosses the WAN on the way back is the **`temper_next` data** — not a full-cluster
dump. Production keeps its single-database shape, and the existing flag-flip
cutover (`UPDATE kb_backend_selection SET backend='next'`) applies unchanged.

### Locked decisions

| Decision | Choice | Rationale |
|---|---|---|
| Execution vehicle | **Localhost synthesis** (no in-region runner) | Kills latency; dissolves atomic-run risk |
| Land-back | **Load `temper_next` into the existing prod DB** | Single DB; smallest transfer; reuses flag-flip cutover |
| Local PG version | **PostgreSQL 17** (match Neon) | Keeps every restore on the safe newer-reads-older direction |
| Round-trip scope | **`temper_next` only** (schema + data) | `public.*` is synthesis input, dumped out only; stable under freeze |
| Write-freeze | **Operator discipline** (single-user) | No autonomous writers; Neon snapshot is the net |
| Rollback | **Neon snapshot/branch** + flag-flip back | `public.*` is never mutated |
| `public.*` fate | **Left dormant in place** | Rename-aside / drop deferred to the migration-endgame spec |
| Tooling depth | **Three guard-rail scripts** for the bulk steps | Footgun-proof the irreversible parts; keep the rest explicit |

### One runbook, two uses

The same procedure serves both remaining flip steps:

- **Run against a Neon *branch*** → **step D rehearsal**: throwaway, no freeze,
  proves synthesis + §9 read-floor parity on the real prod corpus.
- **Run against *main*** → **step E real flip**: with freeze, snapshot, and cutover.

Rehearse on a branch until boring, then run the identical steps against main.

## The flip sequence (against prod / main)

> Rehearsal mode = the same steps with the prod connection string pointed at a
> fresh Neon **branch** instead of main, and steps 2 (freeze) and 7 (cutover/redeploy)
> skipped. Rehearsal verifies parity and is thrown away.

### 0. Pre-flight

- A **green branch-rehearsal** of this runbook exists (synthesis §8 parity clean
  + §9 read-floor parity clean on the real corpus — the latter is step D's harness).
- `neonctl` authenticated (`neonctl projects list` works); version ≥ 2.26.
- The **throwaway PG17 flip container** is up and empty (see *PG-version & the flip
  container* below): `docker compose -f docker-compose.flip.yml up -d`. It runs a
  PG17 pgvector image on a **distinct port (5438)** so it never touches the PG18
  dev DB on :5437 — both can coexist, and nothing else competes locally.
- PostgreSQL 17 client tools available — the scripts auto-resolve them via
  `FLIP_PG17_BIN` (default the Homebrew `postgresql@17` keg); no manual PATH
  change needed.
- Disk headroom for the `public` dump + local restore.

### 1. (informational) Confirm scope

No autonomous writer touches prod (ingest pipeline, cron, scheduled agents).
Single-user posture: the operator is the only writer.

### 2. Freeze writes

Operator stops writing for the window. (No code/infra change — single-user.)

### 3. Snapshot prod → rollback point

`neonctl branches create` from `main`. **Record the branch id/name and the LSN/timestamp.**
This is the rollback target. (`public.*` will also be untouched, but the snapshot
is the belt to the flag-flip's suspenders.)

### 4. Dump `public` → local (synthesis input)

- `pg_dump` prod **`public`** schema (data + schema) → local file.
- Create the fresh local PG17 DB; install the `vector` extension and load the
  portable `uuid_generate_v7()` shim (`tools/flip/uuid_portable.sql`);
  `pg_uuidv7` is intentionally not installed locally.
- `pg_restore` / `psql` the `public` dump into the local DB.

### 5. Build `temper_next` locally

- Ensure a **clean** `temper_next` schema + functions exist locally (load
  `schema-artifact/00_namespace_reset.sql`, `01_schema.sql`, `02_functions.sql` — the
  same set the synthesis tests' `reset_artifact()` loads). **Do NOT load `03_seed.sql`:**
  it is a demo scenario fixture (the alice/bob onboarding-cogmap for `04_scenarios.sql`),
  not system seed — its 20 demo resources have no `public` counterpart, so loading it
  makes the §8 body-parity gate flag all of them (caught in the 2026-06-21 Neon
  rehearsal: 15 spurious divergers, all `03_seed` demo rows). Synthesis seeds the system
  actors it needs itself via `bootstrap::run`.
- Run synthesis against localhost:

  ```bash
  DATABASE_URL=postgresql://temper:temper@localhost:5438/temper_development \
    cargo run -p temper-next --bin temper-next -- synthesize   # --limit 0 = all rows
  ```

  `temper-next synthesize` (`crates/temper-next/src/main.rs:54`) sets
  `search_path = temper_next, public` per connection and runs the three passes.
- Synthesis's built-in **§8 body-parity gate** must report **zero mismatches**.

### 6. Transfer `temper_next` back into prod

- `pg_dump --schema=temper_next` from local (full: **schema + data**) → file.
- On prod, in one transaction: `DROP SCHEMA temper_next CASCADE;` then restore the
  dump. This replaces prod's dormant `temper_next` **wholesale** with an exact
  replica of the local one, so any drift between the migration-installed prod
  schema and the artifact-built local schema cannot bite.
- **No trigger gymnastics:** `pg_dump` orders `CREATE TRIGGER` statements *after*
  the `COPY` data in the dump, so triggers don't exist while the projected rows
  load — no re-firing, no double-apply, and no `session_replication_role` needed.
  The load is bulk/throughput, immune to idle-in-transaction reaping.
- **Safety:** the flag is still `legacy`, so prod's `temper_next` is dormant
  throughout this step, and the step-3 Neon snapshot covers the `DROP`. A failed
  or partial load is harmless — re-run.

> *Why full-schema over data-only:* a `--data-only` load assumes prod's
> `temper_next` columns are bit-identical to the local one. Prod's schema came
> from `migrations/20260613000001_install_temper_next.sql` (+ follow-ups); the
> local one is built from `schema-artifact/01_schema.sql` + `02_functions.sql`.
> They're *meant* to match, but a full DROP/recreate removes the assumption from
> the irreversible step rather than trusting it.

### 7. Cutover

- `UPDATE kb_backend_selection SET backend='next' WHERE id=true;` on prod.
- **Redeploy** the Vercel app (api + mcp). The flag is read **once at API startup**
  (`crates/temper-api/src/main.rs:34`); a running process won't pick up the change
  without a restart/redeploy.
- **Verify** prod surfaces serve from `temper_next`: `temper resource list` /
  `show` / `search` / a graph read return the expected rows at the §9 read-floor.

### 8. Unfreeze

Resume writes. They now land in `temper_next` via the NextBackend write path.

## Rollback

`public.*` is never mutated (frozen + synthesis reads it only), so rollback is cheap:

1. `UPDATE kb_backend_selection SET backend='legacy' WHERE id=true;`
2. Redeploy → the API is back on the untouched legacy schema.

If anything deeper is wrong, restore the **Neon snapshot branch** from step 3.
`temper_next`'s loaded data can be left in place (dormant under `legacy`) or
`TRUNCATE`d; either way it does not affect the legacy read path.

## Cleanup (deferred)

`public.*` is left **dormant in place** after a successful flip. Renaming it aside
and dropping it is explicitly **out of scope here** — it belongs to the
*migration-endgame* spec (schema promotion + legacy drop + namespace collapse).
Retain the step-3 Neon snapshot branch for a retention window before deleting.

## Tooling: three guard-rail scripts

A committed **`docker-compose.flip.yml`** (PG17 pgvector on :5438) plus the
footgun-prone bulk steps wrapped as `cargo make` tasks (scripts under
`tools/bin/` or inline in `Makefile.toml`, matching the existing task style).
Everything else — snapshot, flag `UPDATE`, redeploy, verify — stays an **explicit
manual step** in the runbook; those are the irreversible ones and must not be one
keystroke.

| Task | Wraps | Guards against |
|---|---|---|
| `flip-dump-public` | `pg_dump` prod `public` → restore into the running :5438 PG17 container (extensions first) | wrong schema scope, missing extensions, wrong search_path |
| `flip-synthesize-local` | load `temper_next` clean schema/functions (00/01/02, no 03_seed) locally → `temper-next synthesize` → assert §8 parity clean | forgetting the parity gate; running against the wrong DB |
| `flip-load-next` | `pg_dump --schema=temper_next` (full) local → prod `DROP SCHEMA temper_next CASCADE` + restore, in one txn | schema drift vs prod; partial load leaving a half-replaced schema |

Each task takes the source/target connection strings as parameters (env or
arguments) so the **same scripts drive both the branch rehearsal and the real
flip** — only the target connection string differs.

### PG-version & the flip container

Synthesis input is dumped **from** prod PG17 and restored **into** the local flip
container; the `temper_next` result is then dumped **from** local and restored
**into** prod PG17. Restoring a PG17 dump into a newer server is well-supported;
dumping from a newer server and restoring into PG17 is the **risky** direction.
Running local on **PG17** keeps every restore on the safe side and removes
cross-version risk entirely.

The dev DB (`docker-compose.yml`) is `pgvector/pgvector:0.8.2-pg18-trixie` on
**:5437** — wrong major version for this. So the flip uses a **dedicated,
throwaway PG17 container**, committed as **`docker-compose.flip.yml`**:

- Image: a PG17 pgvector tag (e.g. `pgvector/pgvector:0.8.2-pg17-trixie`) — matches
  Neon's major version and provides the `vector` extension.
- Port **5438** (not 5437) and a distinct container name, so it coexists with the
  PG18 dev DB without conflict. Nothing else competes for these ports locally.
- **`uuid_generate_v7()` needs a portable shim in the container.** The existing
  `20260420000012_uuidv7_portability.sql` only covers Neon (`pg_uuidv7` extension)
  and PG18 (native `uuidv7()`). A bare PG17 pgvector image has *neither*, and
  prod's `public` dump supplies `uuid_generate_v7()` via the `pg_uuidv7`
  **extension** — whose `CREATE EXTENSION` won't restore into the container. So
  the flip setup loads a small self-contained plpgsql `public.uuid_generate_v7()`
  (`tools/flip/uuid_portable.sql`) before the artifact. Synthesis still preserves
  every **external** id (resources/profiles/contexts); this shim only backs the
  **internal** generated ids (chunk/event/revision), which need only be valid,
  unique, sortable v7 UUIDs — their exact bytes are immaterial. (A custom PG17
  image bundling `pg_uuidv7` would be more faithful but is unnecessary for that.)
- Ephemeral: `docker compose -f docker-compose.flip.yml up -d` for the flip,
  `down -v` after. It holds only transient prod data, so it is torn down (volume
  included) once the flip/rehearsal completes.

## Out of scope

**Rejected** (load-bearing — resist re-introducing):
- *In-region synthesis runner* (Neon-region container, Vercel Sandbox, server-side
  synthesis). Localhost synthesis + bulk transfer makes co-location unnecessary.
- *Full-cluster dump round-trip.* Only `temper_next` data crosses back; `public.*`
  is stable under freeze and stays put.
- *App-level maintenance/read-only mode.* Single-user operator discipline suffices;
  adding a 503 freeze flag is unjustified code for a one-time flip.

**Deferred** (in scope elsewhere / later):
- *Rename-aside + drop of `public.*`* → migration-endgame spec.
- *Re-minted-id continuity* — already resolved: synthesis **preserves** prod ids
  (`synthesis/mod.rs:149-155`, `bootstrap.rs`, PR #124 identity-as-input), so
  external refs survive the flip. The task's "mints fresh ids" note is stale.

## References

- Strategy: `docs/superpowers/specs/2026-06-16-ws6-flip-readiness-strategy.md`
- Adjudication (§D hard cutover): `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`
- Synthesis core: `crates/temper-next/src/synthesis/mod.rs`, `bootstrap.rs`, `source.rs`, `parity.rs`
- Entrypoint: `crates/temper-next/src/main.rs` (`synthesize`), `substrate.rs` (connect/search_path)
- Cutover flag: `crates/temper-api/src/services/backend_selection_service.rs`, read at `main.rs:34`
- Schema artifact: `schema-artifact/{00_namespace_reset,01_schema,02_functions,03_seed}.sql`
- temper_next install migrations: `migrations/20260613000001_install_temper_next.sql` (+ 4c/can_modify/invocation follow-ups)
