# Canonical Migrations in `public` + Neon Baseline-Reset — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote the substrate to be the canonical `public` schema authored as a fresh 3-file migration baseline (the single source of truth), delete `schema-artifact/` while preserving temper-next's proving-ground suite, repoint dev/test/CI provisioning off the `temper_next` search_path, and own the Neon `_sqlx_migrations` reconciliation.

**Architecture:** The collapsed code is location-agnostic — every SQL statement is unqualified and resolves against `search_path`. So one namespace-free baseline SQL body builds `public` under the default path (production/dev/e2e) and a `temper_next` test namespace under a `PGOPTIONS=-csearch_path=temper_next,public` wrapper (proving-ground tests). `migrations/` becomes the lone master; `schema-artifact/` and the `schema_drift` two-copy guard retire.

**Tech Stack:** Rust workspace (sqlx, cargo-make, cargo-nextest), PostgreSQL 18 + pgvector (Docker :5437), Neon (Postgres 17) for temperkb.io.

**Spec:** `docs/superpowers/specs/2026-06-23-canonical-migrations-in-public-design.md`

## Global Constraints

- **Branch:** `jct/ws6-endgame-flip`. This plan is the redirected *tail* of the atomic WS6 flip (committed WIP `f66cb9e`). The whole branch lands as **one PR**; nothing builds green under `SQLX_OFFLINE` until the caches are regenerated against the collapsed-in-`public` schema (Task 4). Until then, the working tree is the atomic state — commit with `--no-verify` (the pre-commit hook compiles the whole tree offline against stale caches; this is the documented WIP exception, matching `f66cb9e`).
- **Single source of truth:** `migrations/` only. No generator, no `schema-artifact/`, no `schema_drift` guard after this plan.
- **Baseline files are namespace-free:** NO `CREATE SCHEMA` / `SET search_path` in the migration bodies. The `public` landing is the default; the `temper_next` test landing is injected by the harness.
- **temper-next keeps its `temper_next`-bound `.sqlx` cache** (its macros resolve against its isolated test namespace). Only **temper-api** (`crates/temper-api/.sqlx`) and **e2e** (`tests/e2e/.sqlx`) caches move to `public`.
- **Live cutover stays the operator runbook** `docs/guides/ws6-endgame-collapse-runbook.md`. This plan writes its Neon section; it does not execute the live data flip.
- **Code quality rules** (CLAUDE.md): typed structs over inline JSON; service layer owns SQL; `sqlx::query!` macros (runtime `query` only for the pgvector `::vector` exception); params structs > 5 args.
- **Dev DB URL:** `postgresql://temper:temper@localhost:5437/temper_development`.

---

## File Structure

**Created:**
- `migrations/20260624000001_canonical_schema.sql` — substrate DDL (enums, tables, indexes, grafted identity/infra), namespace-free.
- `migrations/20260624000002_canonical_functions.sql` — all SQL functions (substrate + graph + access predicates), namespace-free.
- `migrations/20260624000003_canonical_seed.sql` — the audited system boot-seed.
- `crates/temper-next/tests/fixtures/00_namespace_reset.sql` — test-only namespace reset (the proving-ground harness wrapper).
- `crates/temper-next/tests/fixtures/seeds/`, `…/scenarios/`, `…/payloads/`, `…/access-scenarios/` — rehomed YAML/JSON (git-mv from `schema-artifact/`).

**Modified:**
- `Makefile.toml` — `db-collapsed`, `prepare-api`, `prepare-e2e` tasks (drop search_path gymnastics); `prepare-next` load preamble.
- `crates/temper-next/tests/common/mod.rs` — `load_files`, `reset_artifact`, `reset_artifact_with_seed`.
- `crates/temper-next/src/scenario/bootseed.rs` — `SYSTEM_SEED` / payloads paths.
- ~28 `crates/temper-next/tests/*.rs` — fixture path constants (`schema-artifact/…` → `tests/fixtures/…`).
- `crates/temper-api/src/services/profile_service.rs` (tests) — `#[sqlx::test]` → `#[sqlx::test(migrations = "../../migrations")]`.
- `docs/guides/ws6-endgame-collapse-runbook.md` — step 9 rewrite (Neon baseline-reset + backup gate).
- Unit + e2e test files referencing deleted services (Task 8 — enumerated at execution).

**Deleted:**
- `migrations/2026{0330..0623}*.sql` — the 46-file legacy lineage (`migrations/templates/` + `migrations/CLAUDE.md` stay).
- `schema-artifact/` — entire directory.
- `crates/temper-next/tests/schema_drift.rs`; `crates/temper-next/tests/install_migration.rs` (reshape or delete — Task 7).
- `tools/gen-install-migration.sh` (if present).

---

## Task 1: Author the canonical baseline schema + functions; retire the legacy lineage

**Files:**
- Create: `migrations/20260624000001_canonical_schema.sql`, `migrations/20260624000002_canonical_functions.sql`
- Delete: the 46 legacy `migrations/*.sql` files
- Source: `schema-artifact/01_schema.sql`, `schema-artifact/02_functions.sql`

**Interfaces:**
- Produces: a `migrations/` lineage of exactly 3 baseline files (the seed lands in Task 2) that `sqlx migrate run` applies into `public`, building the full substrate (the same shape `temper_next` had).

- [ ] **Step 1: Generate the schema baseline body**

Copy `schema-artifact/01_schema.sql` → `migrations/20260624000001_canonical_schema.sql`, then strip the namespace preamble: delete the `SET search_path TO temper_next, public;` lines and any `CREATE SCHEMA`/`DROP SCHEMA` (the artifact body has none; only the leading `SET search_path` at the top). Rewrite the file header comment from "one-shot artifact, NOT a migration / Namespace: everything lands in `temper_next`" to "canonical baseline — builds the substrate in the connection-default schema (`public` in production; a `temper_next` test namespace under the proving-ground harness wrapper)."

- [ ] **Step 2: Generate the functions baseline body**

Copy `schema-artifact/02_functions.sql` → `migrations/20260624000002_canonical_functions.sql`, strip the leading `SET search_path TO temper_next, public;`. Function BODIES are already unqualified (they relied on search_path) — leave them. Rewrite the header comment the same way.

- [ ] **Step 3: Delete the legacy lineage**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
# Delete every legacy migration EXCEPT the new 20260624 baseline files (legacy ends at 20260623).
# A naive glob like 2026062*.sql would also match the new 20260624 files — exclude them explicitly.
ls migrations/2026*.sql | grep -v '/20260624' | xargs git rm
ls migrations/*.sql
```
Expected: only `20260624000001_canonical_schema.sql` and `20260624000002_canonical_functions.sql` remain (the seed lands in Task 2). Confirm `migrations/templates/` and `migrations/CLAUDE.md` are untouched. If `tools/gen-install-migration.sh` exists, `git rm` it.

- [ ] **Step 4: Verify the baseline builds public on a throwaway DB**

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS vector;"
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/20260624000001_canonical_schema.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/20260624000002_canonical_functions.sql
psql "$DATABASE_URL" -c "\dt" | grep -c kb_         # table count
psql "$DATABASE_URL" -c "\df has_system_access"     # access predicate present
```
Expected: both files load with no error; `kb_resources`, `kb_events`, `kb_edges`, `kb_cogmaps`, `kb_system_settings`, `kb_event_types` all present; `has_system_access` listed.
> NOTE: this leaves the dev DB's `public` rebuilt from the baseline — intended; Task 3 makes that the standard `db-collapsed` behavior. (Restore the legacy dev DB with `cargo make docker-down && docker-up` if you need it mid-task.)

- [ ] **Step 5: Commit**

```bash
git add migrations/
git commit --no-verify -m "ws6-flip: canonical baseline schema+functions in public; retire legacy lineage"
```

---

## Task 2: Author the audited canonical system seed

**Files:**
- Create: `migrations/20260624000003_canonical_seed.sql`
- Source: `schema-artifact/seeds/system.yaml`, `schema-artifact/payloads/<name>.v1.schema.json`, `crates/temper-next/src/scenario/bootseed.rs` (the seeding logic to mirror), spec §2 manifest.

**Interfaces:**
- Consumes: the schema+functions from Task 1 (`lens_create`, `kb_event_types`, `kb_profiles`, `kb_system_settings`).
- Produces: a fresh `public` DB that boots with the system actor, the 27-name event-type registry, 2 global lenses, and the access-gate settings row.

- [ ] **Step 1: Author the seed in dependency order**

`migrations/20260624000003_canonical_seed.sql` — namespace-free, ordered so each insert's dependencies exist first:

```sql
-- Canonical system boot-seed: what any temper system needs (spec §2). Namespace-free —
-- resolves against the connection-default schema. Mirrors temper_next::scenario::bootseed::seed_system.

-- 1. The canonical system actor (events require a NOT NULL emitter). handle is UNIQUE.
INSERT INTO kb_profiles (handle, display_name, system_access)
VALUES ('system', 'System', 'admin')
ON CONFLICT (handle) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO kb_entities (profile_id, name, metadata)
SELECT id, 'system', '{}'::jsonb FROM kb_profiles WHERE handle = 'system'
ON CONFLICT DO NOTHING;

-- 2. The access-gate settings row (id=1 CHECK). Without it has_system_access/is_system_admin
--    read an empty CTE and return NULL (spec §2, item 4).
INSERT INTO kb_system_settings (id, access_mode) VALUES (1, 'open')
ON CONFLICT (id) DO NOTHING;

-- 3. The event-type registry (27 names). payload_schema inlined from the committed
--    payloads/<name>.v1.schema.json snapshots; a name with no snapshot stays NULL (permissive).
INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES
  ('resource_created', '<inlined JSON or NULL>'::jsonb, 1),
  -- … all 27 names from system.yaml, in order …
  ('lens_created', '<inlined JSON or NULL>'::jsonb, 1)
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema, schema_version = EXCLUDED.schema_version;

-- 4. The 2 global system lenses, event-sourced via lens_create (cogmap_id NULL), attributed to
--    the system actor. Exact weight vectors from system.yaml.
SELECT lens_create(
  NULL,                              -- cogmap_id (global)
  'telos-default',
  (SELECT id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id
    WHERE p.handle = 'system' AND e.name = 'system'),  -- emitter
  1.0, 1.0, 0.6, 0.3, 0.4,           -- w_express, w_contains, w_leads_to, w_near, w_prop
  0.5, 0.3, 0.2, 0.5                 -- s_telos, s_ref, s_central, resolution
);
SELECT lens_create(
  NULL, 'telos-default-propheavy',
  (SELECT id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id
    WHERE p.handle = 'system' AND e.name = 'system'),
  1.0, 1.0, 0.1, 0.3, 1.2,
  0.5, 0.3, 0.2, 0.5
);
```
> VERIFY-THEN-ACT: confirm `lens_create`'s exact parameter list and order against `migrations/20260624000002_canonical_functions.sql` (the artifact's `02_functions.sql` definition) before finalizing — the positional args above are from `system.yaml`'s field set and MUST match the function signature. Adjust to named-arg or correct positional order as the signature dictates. Likewise confirm `kb_event_types` columns (`name`, `payload_schema`, `schema_version`) and the `kb_entities` unique constraint for the `ON CONFLICT`.

- [ ] **Step 2: Inline the payload schemas**

For each of the 27 event-type names, read `schema-artifact/payloads/<name>.v1.schema.json`; if present, inline its JSON as the `payload_schema` literal; if absent, use `NULL`. (Mirror `bootseed.rs` exactly — same names get NULL there.) Keep the JSON compact; a name with a snapshot must carry it so the registry==repo contract survives schema-artifact deletion.

- [ ] **Step 3: Verify the seed on a fresh baseline DB**

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/20260624000003_canonical_seed.sql
psql "$DATABASE_URL" -tAc "SELECT count(*) FROM kb_event_types"            # expect 27
psql "$DATABASE_URL" -tAc "SELECT count(*) FROM kb_cogmap_lenses WHERE cogmap_id IS NULL"  # expect 2
psql "$DATABASE_URL" -tAc "SELECT access_mode FROM kb_system_settings WHERE id=1"          # expect open
psql "$DATABASE_URL" -tAc "SELECT has_system_access((SELECT id FROM kb_profiles WHERE handle='system'))"  # expect t
```
Expected: 27 / 2 / open / t. The `has_system_access` returning `t` (not NULL) confirms the settings-row fix.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260624000003_canonical_seed.sql
git commit --no-verify -m "ws6-flip: canonical system seed (actor, event-type registry, lenses, settings)"
```

---

## Task 3: Repoint dev-DB provisioning + DATABASE_URL to `public`

**Files:**
- Modify: `Makefile.toml` (`db-collapsed`, `prepare-api`, `prepare-e2e`)

**Interfaces:**
- Consumes: the 3-file baseline (Tasks 1–2).
- Produces: `cargo make db-collapsed` builds the substrate in `public`; a plain `DATABASE_URL` (no `?options=…search_path…`) validates every temper-api macro live.

- [ ] **Step 1: Rewrite the `db-collapsed` task**

In `Makefile.toml`, replace the `db-collapsed` script body (currently loops `schema-artifact/{00,01,02}.sql` and prints a search_path export hint):

```toml
[tasks.db-collapsed]
description = "Build the canonical substrate in public for collapsed-schema dev."
env = { SQLX_OFFLINE = "false" }
script = '''
DB="${DATABASE_URL:-postgresql://temper:temper@localhost:5437/temper_development}"
psql "$DB" -v ON_ERROR_STOP=1 -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS vector;"
for f in migrations/20260624000001_canonical_schema.sql \
         migrations/20260624000002_canonical_functions.sql \
         migrations/20260624000003_canonical_seed.sql; do
  psql "$DB" -v ON_ERROR_STOP=1 -f "$f" >/dev/null
done
echo "substrate built in public. export DATABASE_URL=\"$DB\" (no search_path option needed)."
'''
```

- [ ] **Step 2: Drop the search_path option from the prepare tasks**

In `Makefile.toml`, find the `prepare-api` and `prepare-e2e` tasks and remove `?options=-csearch_path%3Dtemper_next,public` from their `DATABASE_URL` (line ~89 is the pattern). They become plain `cargo sqlx prepare` against the public DB. Leave `prepare-next` for Task 6.

- [ ] **Step 3: Verify live macro validation against public**

```bash
cargo make db-collapsed
export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development"
SQLX_OFFLINE=false cargo check -p temper-api --all-features 2>&1 | tail -5
```
Expected: `Finished` with **0 errors** — every temper-api macro now resolves against the public substrate. (This is the live driver from the flip; it proves the collapsed code is correct against the baseline before any cache exists.)

- [ ] **Step 4: Commit**

```bash
git add Makefile.toml
git commit --no-verify -m "ws6-flip: db-collapsed builds substrate in public; drop search_path gymnastics"
```

---

## Task 4: Regenerate the temper-api + e2e sqlx caches against `public`

**Files:**
- Modify: `crates/temper-api/.sqlx/` (regenerated), `tests/e2e/.sqlx/` (regenerated)

**Interfaces:**
- Consumes: live-green temper-api (Task 3).
- Produces: the first **`SQLX_OFFLINE` green** checkpoint of the branch.

- [ ] **Step 1: Regenerate both caches**

```bash
cargo make prepare-api
cargo make prepare-e2e
```

- [ ] **Step 2: Verify offline build is green**

```bash
SQLX_OFFLINE=true cargo check -p temper-api --all-features 2>&1 | tail -3
SQLX_OFFLINE=true cargo check -p temper-e2e --all-features 2>&1 | tail -3
```
Expected: both `Finished` with 0 errors. This is the first point the offline build (what CI and the pre-commit hook run) is green.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/.sqlx tests/e2e/.sqlx
git commit --no-verify -m "ws6-flip: regenerate temper-api + e2e sqlx caches against public"
```
> Still `--no-verify`: the temper-next suite + the test surface aren't green until Tasks 6 & 8.

---

## Task 5: Repoint in-crate `#[sqlx::test]` and revert the bare-test deferral

**Files:**
- Modify: `crates/temper-api/src/services/profile_service.rs` (test module)

**Interfaces:**
- Consumes: the migrating-into-public baseline.
- Produces: profile_service tests run against the migrated public substrate (resolves the flip's deferred bare-`#[sqlx::test]` = empty-DB revert).

- [ ] **Step 1: Restore the migrations attribute**

In `profile_service.rs`, change each test's `#[sqlx::test]` (bare, reverted during the flip) back to `#[sqlx::test(migrations = "../../migrations")]`. Now that `migrations/` builds the substrate in `public`, the migrator applies the baseline and the tests get the real schema.

- [ ] **Step 2: Verify**

```bash
cargo nextest run -p temper-api --features test-db -E 'test(profile)' 2>&1 | tail -10
```
Expected: profile_service tests pass against the baseline-migrated ephemeral DB.
> VERIFY-THEN-ACT: if a test relied on the legacy System/Anonymous profiles or seeded doc-types (now gone), update its fixtures to the substrate shape (create the profile it needs) rather than re-adding legacy seed. Report if any test's intent can't be satisfied by the substrate.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/services/profile_service.rs
git commit --no-verify -m "ws6-flip: profile_service tests migrate from migrations/ (public)"
```

---

## Task 6: Rehome the proving-ground fixtures + rewire the temper-next harness

**Files:**
- Create: `crates/temper-next/tests/fixtures/00_namespace_reset.sql`; rehomed `tests/fixtures/{seeds,scenarios,payloads,access-scenarios}/`
- Modify: `crates/temper-next/tests/common/mod.rs`; `crates/temper-next/src/scenario/bootseed.rs`; ~28 `crates/temper-next/tests/*.rs`; `Makefile.toml` (`prepare-next`)

**Interfaces:**
- Consumes: the namespace-free baseline files (Task 1).
- Produces: `cargo nextest run -p temper-next --features artifact-tests` green, loading the baseline into a `temper_next` test namespace via `PGOPTIONS`.

- [ ] **Step 1: git-mv the fixtures**

```bash
mkdir -p crates/temper-next/tests/fixtures
git mv schema-artifact/seeds          crates/temper-next/tests/fixtures/seeds
git mv schema-artifact/scenarios       crates/temper-next/tests/fixtures/scenarios
git mv schema-artifact/payloads        crates/temper-next/tests/fixtures/payloads
git mv schema-artifact/access-scenarios crates/temper-next/tests/fixtures/access-scenarios
# Legacy read-path SQL fixtures (only if artifact-tests-legacy still lives — Task 7 Step 4):
git mv schema-artifact/03_seed.sql     crates/temper-next/tests/fixtures/03_seed.sql 2>/dev/null || true
git mv schema-artifact/04_scenarios.sql crates/temper-next/tests/fixtures/04_scenarios.sql 2>/dev/null || true
```

- [ ] **Step 2: Add the test-only namespace reset**

`crates/temper-next/tests/fixtures/00_namespace_reset.sql`:
```sql
-- TEST-ONLY: owns + resets the isolated temper_next proving-ground namespace. Production never
-- runs this (the baseline migration builds public under the default search_path). The harness
-- injects search_path=temper_next,public via PGOPTIONS when loading the baseline body files.
DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
```

- [ ] **Step 3: Rewire `load_files` to wrap the baseline under temper_next**

In `crates/temper-next/tests/common/mod.rs`, the namespace-free baseline files won't self-set search_path, so inject it via `PGOPTIONS` for the body files (the reset file runs without it):

```rust
fn load_files(files: &[&str]) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for artifact tests");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for f in files {
        // Reset file lives in tests/fixtures/; baseline body files live in migrations/.
        let (path, set_search_path) = if *f == "00_namespace_reset" {
            (format!("{}/crates/temper-next/tests/fixtures/{f}.sql", root), false)
        } else {
            (format!("{root}/migrations/{f}.sql"), true)
        };
        let mut cmd = std::process::Command::new("psql");
        if set_search_path {
            cmd.env("PGOPTIONS", "-csearch_path=temper_next,public");
        }
        let status = cmd
            .args([url.as_str(), "-q", "-v", "ON_ERROR_STOP=1", "-f", &path])
            .status()
            .expect("failed to run psql (is it on PATH?)");
        assert!(status.success(), "psql -f {f}.sql failed during reset");
    }
}
```
Update the two callers to the baseline filenames:
```rust
pub fn reset_artifact() {
    load_files(&["00_namespace_reset",
                 "20260624000001_canonical_schema",
                 "20260624000002_canonical_functions"]);
}
pub fn reset_artifact_with_seed() {
    // Legacy read-path topology (artifact-tests-legacy). 03_seed stays a fixture file.
    load_files(&["00_namespace_reset",
                 "20260624000001_canonical_schema",
                 "20260624000002_canonical_functions"]);
    // …then load tests/fixtures/03_seed.sql under the same PGOPTIONS (inline a fixtures-path arm
    // or a second helper). See Task 7 Step 4 re: whether this path still exists.
}
```
> VERIFY-THEN-ACT: confirm `reset_artifact_with_seed`'s consumers still exist after the flip's synthesis/legacy-read retirement (Task 7 Step 4). If `artifact-tests-legacy` is already retired, delete this helper and skip the `03_seed`/`04` rehome in Step 1.

- [ ] **Step 4: Rewire `bootseed.rs` paths**

In `crates/temper-next/src/scenario/bootseed.rs`, change `SYSTEM_SEED` and the `payloads_dir` constants from `/../../schema-artifact/seeds/system.yaml` and `/../../schema-artifact/payloads` to `/../tests/fixtures/seeds/system.yaml` and `/../tests/fixtures/payloads` (resolve from `CARGO_MANIFEST_DIR` = the crate root; adjust the relative depth so it points at `crates/temper-next/tests/fixtures/`).
> NOTE: `bootseed.rs` is `src/` (not a test), so the path must be valid from the crate dir. Use `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seeds/system.yaml")`.

- [ ] **Step 5: Sweep the remaining fixture-path constants**

```bash
grep -rln "schema-artifact" crates/temper-next/
```
For each hit (the ~28 test files + scenario model + access runner), rewrite `/../../schema-artifact/<sub>/…` → the rehomed `tests/fixtures/<sub>/…` (or `CARGO_MANIFEST_DIR + /tests/fixtures/…` for `src/` files). Update `Makefile.toml` `prepare-next` if it loads `schema-artifact/*.sql` (repoint to the migrations baseline under `search_path=temper_next`).

- [ ] **Step 6: Verify the proving-ground suite**

```bash
cargo make prepare-next   # regenerate temper-next's temper_next-bound cache against the wrapped baseline
cargo nextest run -p temper-next --features artifact-tests 2>&1 | tail -15
```
Expected: the `temper-next-write` group (scenario/seed/charter/content/ledger/replay) passes; no `schema-artifact` path errors.
> VERIFY-THEN-ACT: `prepare-next`'s load preamble must now build the `temper_next` namespace from the baseline (reset + schema + functions under `PGOPTIONS`), mirroring `load_files`. Update its script to match Step 3.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-next Makefile.toml
git commit --no-verify -m "ws6-flip: rehome proving-ground fixtures; harness loads baseline into temper_next namespace"
```

---

## Task 7: Retire `schema_drift.rs` / `install_migration.rs`; delete `schema-artifact/`

**Files:**
- Delete: `crates/temper-next/tests/schema_drift.rs`; `crates/temper-next/tests/install_migration.rs`; `schema-artifact/`
- Possibly modify: `.config/nextest.toml`, `crates/temper-next/Cargo.toml` (if they name the retired tests)

**Interfaces:**
- Consumes: a green proving-ground suite (Task 6).
- Produces: `schema-artifact/` gone; no two-copy invariant; the tree references the artifact only in historical docs.

- [ ] **Step 1: Establish what each retiring test asserted**

```bash
sed -n '1,40p' crates/temper-next/tests/schema_drift.rs
sed -n '1,40p' crates/temper-next/tests/install_migration.rs
```
`schema_drift.rs` proved "lineage reconstructs the artifact" — moot (no two copies). `install_migration.rs` proved the generated install migration ≡ artifact — moot (no generator). Confirm neither asserts something still needed (e.g. a structural property of the baseline). If `install_migration.rs` has a still-valid "the migrations build a loadable schema" assertion, **reshape** it to load the baseline into a throwaway DB and assert table presence rather than deleting outright.

- [ ] **Step 2: Delete (or reshape) the guards**

```bash
git rm crates/temper-next/tests/schema_drift.rs
git rm crates/temper-next/tests/install_migration.rs   # or edit per Step 1
```
Remove any reference to them in `.config/nextest.toml` test groups.

- [ ] **Step 3: Add the event-type vocabulary drift-assert**

Since the seed SQL (`…_canonical_seed.sql`) and the test bootseed (`tests/fixtures/seeds/system.yaml`) both encode the 27 event-type names, add one small guard so they can't drift. In a temper-next test (e.g. `tests/bootseed.rs`):
```rust
#[test]
fn seed_migration_event_types_match_system_yaml() {
    let yaml_names = temper_next::scenario::bootseed::system_event_type_names().unwrap();
    let migration = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260624000003_canonical_seed.sql"
    )).unwrap();
    for name in &yaml_names {
        assert!(migration.contains(&format!("('{name}',")),
            "event type `{name}` is in system.yaml but missing from the canonical seed migration");
    }
}
```

- [ ] **Step 4: Delete `schema-artifact/`**

```bash
git rm -r schema-artifact/
grep -rln "schema-artifact" crates/ tests/ Makefile.toml .config/ api/ packages/
```
Expected: the second command returns **no code/config hits** (only `docs/` historical references may remain — leave those; optionally add a one-line note that the artifact retired into `migrations/` + `crates/temper-next/tests/fixtures/`).

- [ ] **Step 5: Verify temper-next still green**

```bash
cargo nextest run -p temper-next --features artifact-tests 2>&1 | tail -8
cargo nextest run -p temper-next 2>&1 | tail -8   # pure-core tests (affinity/cluster), no feature
```
Expected: both green; the new drift-assert passes.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit --no-verify -m "ws6-flip: retire schema_drift/install_migration guards; delete schema-artifact/"
```

---

## Task 8: The test surface — rewrite/delete tests referencing deleted services

**Files:**
- Modify/Delete: unit test modules in `crates/temper-api/` + e2e files in `tests/e2e/tests/` referencing deleted services (`sync`/`doc_type`/`search`/`relationship`/`meta`/`ingest` services, `backend_selection`, `temper-events`).

**Interfaces:**
- Consumes: the collapsed service layer (from `f66cb9e`) + the green offline build (Task 4).
- Produces: `cargo make test-db` + `cargo make test-e2e` compile and pass against the substrate.

- [ ] **Step 1: Enumerate the broken test surface**

```bash
SQLX_OFFLINE=true cargo nextest run -p temper-api --features test-db --no-run 2>&1 | grep -E "error\[|cannot find|unresolved" | head -40
SQLX_OFFLINE=true cargo nextest run -p temper-e2e --features test-db --no-run 2>&1 | grep -E "error\[|cannot find|unresolved" | head -40
```
This yields the concrete file list. Expected referents (from the flip ledger): `edge_ingest`, `relationship_projection`, `resource_update_reconcile`, `audit`, `mcp_ingest`, `mcp_round_trip`, `doc_type`, `list_meta_select`, `sync` tests.

- [ ] **Step 2: Disposition each file**

For each broken file, apply one of:
- **Delete** — the test exercised a retired surface (sync routes, backend selection, doc-type-by-id, the `/api/events?` feed). git rm it.
- **Rewrite to substrate** — the behavior still exists via the collapsed path (e.g. edge reads via `list_resource_edges`, meta reads via `read_selector`). Repoint imports to the surviving types/services and the `DbBackend` trait; drop `device_id`/`selection`/`backend_selection` params.

Record the disposition per file in the commit message (one line each) so the reviewer can audit coverage loss.
> VERIFY-THEN-ACT: a *delete* is only valid when the behavior itself retired. If behavior survives but moved, rewrite. If you can't tell, escalate rather than delete (losing coverage silently is the failure mode this step guards against).

- [ ] **Step 3: Verify the DB + e2e suites compile and pass**

```bash
cargo make test-db 2>&1 | tail -20
cargo make test-e2e 2>&1 | tail -20
```
Expected: both compile; passing (modulo embed-gated tests, which need the Embed CI job — note any you skip).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit --no-verify -m "ws6-flip: rewrite/delete test surface for deleted services (per-file dispositions in body)"
```

---

## Task 9: Flip-tail Tasks 9/10 — surface-parity gate + synthesis deletion

**Files:**
- Delete: `crates/temper-next/src/synthesis/`; the `Synthesize` CLI subcommand; `parity.rs`/`parity_reads.rs` (KEEP `corpus_parity_reads.rs`)
- Modify: e2e `surface_parity_next.rs` (de-split), `mcp_round_trip.rs`; the events surface in the parity gate

**Interfaces:**
- Consumes: green DB + e2e suites (Task 8).
- Produces: the 8-surface parity gate green over the substrate; synthesis machinery gone.

> This task is the flip plan's Tasks 9/10 (`docs/superpowers/plans/2026-06-22-ws6-endgame-collapse-code.md`), redirected to `public`. Execute it from that plan's task bodies; it is listed here so this plan's green-up is complete. Key beats:

- [ ] **Step 1: Delete synthesis** — `git rm -r crates/temper-next/src/synthesis/`; remove the `Synthesize` subcommand + its CLI wiring; delete `parity.rs` + `parity_reads.rs`; keep `corpus_parity_reads.rs` (the durable read-floor). Remove `synthesis::run` / `seed_and_synthesize` / `seed_prod_shape_fixture` consumers in `tests/common/mod.rs` (now dead). Fix the dangling `temper_next::MIGRATOR`-based synthesis tests.

- [ ] **Step 2: De-split + un-ignore the parity gate** — `surface_parity_next.rs` and `mcp_round_trip.rs` lose their `_next` fork (one schema now); the 8-surface gate runs un-ignored; drop the retired events-list (surface 9).

- [ ] **Step 3: Verify**
```bash
cargo nextest run -p temper-next 2>&1 | tail -8
cargo make test-e2e 2>&1 | tail -15
```
Expected: temper-next green without synthesis; the parity gate green.

- [ ] **Step 4: Commit** — `git commit --no-verify -m "ws6-flip: delete synthesis; un-split + un-ignore the 8-surface parity gate"`

---

## Task 10: Neon baseline-reset + persistent-backup gate (runbook)

**Files:**
- Modify: `docs/guides/ws6-endgame-collapse-runbook.md` (step 9)

**Interfaces:**
- Consumes: nothing in-code (docs/operator surface).
- Produces: the runbook owns the `_sqlx_migrations` reconciliation + the durable-backup hard-stop.

- [ ] **Step 1: Insert the persistent-backup gate before the destructive steps**

In the runbook, immediately before step 9's `ALTER SCHEMA temper_next RENAME TO public` (and before any `_sqlx_migrations` truncate), add a hard-stop checkbox:

```markdown
8b. [ ] **PERSISTENT BACKUP GATE (operator hard-stop — do not proceed past this line until done).**
    Cut an explicitly-retained point-in-time backup of the live Neon DB — a snapshot/branch
    outside Neon's default PITR expiry window — as the DURABLE rollback target (distinct from the
    ephemeral pre-cutover branch named in the header). Record its identifier + restore command
    inline below before continuing:

    - Backup id / branch: `__________`
    - Restore command:    `neonctl branches restore … __________`

    Executed manually by the operator, or by the agent once `neonctl` is authenticated.
```

- [ ] **Step 2: Rewrite step 9's punt into the mark-as-applied reconciliation**

Replace the parenthetical *"No `_sqlx_migrations` reconciliation needed… the bootstrap-export spec's job"* with:

```markdown
9b. [ ] **Reconcile `_sqlx_migrations` to the canonical baseline (mark-as-applied, NOT replay).**
    The promoted `public` is structurally artifact-faithful but its `_sqlx_migrations` still lists
    the 46 retired legacy rows. The schema already exists — do NOT replay DDL.

    i.   Structural safety check (HARD GATE):
         `pg_dump --schema-only` of live `public` vs. a fresh DB built from `migrations/`; diff must
         be empty (both derive from the same artifact). Abort the reconciliation if it is not.
    ii.  Compute the 3 baseline checksums sqlx will expect:
         `sqlx migrate info --source migrations` against a fresh baseline DB (or read the
         `_sqlx_migrations` rows it writes there).
    iii. On Neon:  `TRUNCATE _sqlx_migrations;`  then `INSERT` the 3 baseline rows
         (version, description, checksum, success=true, installed_on=now(), execution_time=0).
    iv.  Verify:  `sqlx migrate info --source migrations`  against Neon shows all 3 **applied**,
         and `sqlx migrate run` is a clean no-op.
```

- [ ] **Step 3: Verify the runbook is internally consistent**

Re-read steps 8→11: the backup gate precedes the rename; the reconciliation follows the rename; no remaining reference to "bootstrap-export spec's job." Confirm the redeploy step (10) still coincident with the rename.

- [ ] **Step 4: Commit**

```bash
git add docs/guides/ws6-endgame-collapse-runbook.md
git commit --no-verify -m "ws6-flip: runbook owns Neon baseline-reset + persistent-backup gate"
```

---

## Task 11: End-of-branch verification, final caches, PR

**Files:**
- Modify: all 3 `.sqlx` caches (final regen); no source changes expected.

**Interfaces:**
- Consumes: every prior task green.
- Produces: a single PR for the whole atomic flip.

- [ ] **Step 1: Final cache regen (clean slate)**

```bash
cargo make db-collapsed
cargo make prepare-api && cargo make prepare-e2e && cargo make prepare-next
```

- [ ] **Step 2: Full offline gate (what CI runs)**

```bash
cargo make check 2>&1 | tail -20
```
Expected: fmt + clippy (`--workspace --all-features`) + docs + machete + TS checks all pass — **green for the first time without `--no-verify`**.

- [ ] **Step 3: Full test suites**

```bash
cargo make test-all 2>&1 | tail -25
cargo nextest run -p temper-next --features artifact-tests 2>&1 | tail -8
```
Expected: workspace + integration + e2e + TS green; proving-ground green. (Embed-gated tests run in the Embed CI job — note locally-skipped ones.)
> Per CLAUDE.md: do NOT trust nextest's per-binary "Summary" line under `--no-fail-fast`; grep for `error: test run failed` / `FAIL [` or check the exit code.

- [ ] **Step 4: Consolidated code-review**

Run `/code-review high` over the full branch diff (the whole flip + this tail). Address findings. This is the consolidated end-of-branch review (hybrid-execution: review at end-of-plan, not per task).

- [ ] **Step 5: Squash-fixup the `--no-verify` WIP commits if desired, then open the PR**

```bash
git add -A && git commit -m "ws6-flip: final caches + green cargo make check"   # NO --no-verify — must pass the hook now
git push -u origin jct/ws6-endgame-flip
gh pr create --title "WS6 endgame flip: collapse to one backend + canonical migrations in public" --body "<summary + runbook pointer + the live-cutover note>"
```
Expected: the pre-commit hook passes (caches current); CI green; PR opened.

---

## Self-Review

**Spec coverage:**
- §1 canonical baseline (3 files) → Tasks 1–2. ✓
- §2 system-seed manifest (incl. the `kb_system_settings` NULL-gate catch) → Task 2. ✓
- §3 schema-artifact deletion + fixture rehome → Tasks 6–7. ✓
- §4 dev-DB / harness / cache repoint → Tasks 3–6. ✓
- §5 Neon baseline-reset + the §0 backup gate → Task 10. ✓
- Redirected flip-tail (test surface, Tasks 9/10) → Tasks 8–9. ✓
- End-of-branch verification + code-review + PR → Task 11. ✓

**Placeholder scan:** The two `<inlined JSON …>` and `<summary …>` markers are genuine fill-from-source instructions with the source named (payloads snapshots; the branch summary), not vague TODOs. The `VERIFY-THEN-ACT` notes are deliberate live-inspection gates (lens_create signature, legacy-read-path existence, per-file test disposition) with explicit defaults and escalation rules — the implementation-grounding discipline, not placeholders.

**Type consistency:** `reset_artifact`/`reset_artifact_with_seed`/`load_files` signatures consistent across Tasks 6–9; `system_event_type_names()` (existing temper-next API) reused in Task 7's drift-assert; baseline filenames (`20260624000001_canonical_schema`, `…02_canonical_functions`, `…03_canonical_seed`) used identically in Tasks 1–3, 6, 10.

**Known coupling:** Task 9 is governed by the flip plan's Tasks 9/10 — kept as a thin pointer to avoid duplicating those task bodies (DRY), since they predate this plan and own the synthesis-deletion detail.
