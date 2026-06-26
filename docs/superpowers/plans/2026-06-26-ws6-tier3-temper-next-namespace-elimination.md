# WS6 Tier 3 — Eliminate the `temper_next` Namespace — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `temper-substrate`'s `sqlx::query!` macros off the `temper_next` Postgres namespace onto `public`, convert its write-path artifact tests to `#[sqlx::test]` ephemeral per-test databases, and delete every piece of build/CI/test machinery that existed only to serve `temper_next`.

**Architecture:** Two independent severings. (1) Compile-time: re-prepare substrate's `.sqlx` cache against `public` and fold it into the workspace cache — its macros are unqualified, so the cache is byte-identical. (2) Runtime: replace the shared, self-resetting `temper_next` namespace with `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, which gives each test a fresh migrated database. With both severed, the CI exclusions, per-crate cache, reset harness, nextest serialization group, and `prepare-next`/`test-next` make tasks all lose their reason to exist.

**Tech Stack:** Rust, sqlx 0.8 (`#[sqlx::test]`, `sqlx::migrate!`), cargo-nextest, cargo-make, PostgreSQL 18 (Docker, port 5437), ONNX Runtime (bge-base-en-v1.5, for embed-dependent tests).

**Spec:** `docs/superpowers/specs/2026-06-26-ws6-tier3-temper-next-namespace-elimination-design.md`

## Global Constraints

- **`--all-features`** on all clippy/check/build invocations (workspace convention).
- **`#[expect(lint, reason = "...")]`** not `#[allow]`.
- **All `cargo make` tasks force `SQLX_OFFLINE=true`** — the committed workspace `.sqlx` cache is the offline source of truth. After changing substrate SQL, regenerate with `cargo sqlx prepare --workspace -- --all-features`.
- **Dev/test DB:** `postgresql://temper:temper@localhost:5437/temper_development` (Docker Postgres; start with `cargo make docker-up`).
- **ONNX required for embed-dependent artifact tests.** The `artifact-tests` feature pulls `temper-ingest`'s `embed` feature (bge-768). Locally, ensure the ONNX runtime is available (the repo bakes the LFS model + `libonnxruntime`; if running raw, set `ORT_DYLIB_PATH`). Without it, embed tests fail to load the model.
- **Production seed coexistence:** `temper_substrate::MIGRATOR` applies all three canonical migrations including `…03_canonical_seed.sql` (the L0 kernel). `bootseed::seed_system` is idempotent against it. Cogmap-scoped assertions are unaffected; tests sensitive to a *clean, unseeded* baseline (replay/equivalence/global-count) call `common::reset_schema(&pool)` first (see Conventions).
- **Keep each commit green.** During the test rewrite, converted (`#[sqlx::test]`) and unconverted (`connect()` + `reset_artifact`) tests coexist in the crate; verify converted batches by running their specific `--test` targets, not the whole suite, until the harness is removed in Task 10.

---

## Conventions: the canonical test-conversion recipe

Every write-path test conversion (Tasks 2–8) applies this same transformation. It is proven by the already-converted `crates/temper-substrate/tests/context_shape.rs`.

**Before** (the `connect()` + `reset_artifact()` pattern):

```rust
#[tokio::test]
async fn some_test() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();   // present in scenario tests
    // ... rest of the test, all using &pool ...
}
```

**After** (`#[sqlx::test]` + injected pool):

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn some_test(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();   // unchanged
    // ... rest of the test, all using &pool ...
}
```

Per-occurrence rules:

1. `#[tokio::test]` → `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`.
2. Add `pool: sqlx::PgPool` as the test fn's only parameter.
3. **Delete** the leading `common::reset_artifact();` and `let pool = substrate::connect().await.unwrap();` (and any `let pool = ...` re-bind after a mid-body reset).
4. A **mid-body** `common::reset_artifact();` (one that appears *after* the test has already done work — e.g. snapshot→reset→replay) → replace with `common::reset_schema(&pool).await;` (defined in Task 2). It drops + rebuilds the schema **in the same database** to a clean, unseeded `01+02` state — the exact semantics the old `reset_artifact` gave.
5. A test that must start from a **clean, unseeded** schema (the production seed would perturb a global count or a replay/projection diff) → make `common::reset_schema(&pool).await;` its **first** line, restoring the pre-seed baseline.
6. Drop now-unused imports the compiler flags (commonly `use temper_substrate::substrate;` when `substrate::load`/`cogmap_by_name` are no longer referenced — but keep it when they are).
7. Leave **all assertions and SQL bodies unchanged** unless the file hardcodes `temper_next` in SQL (Task 7 only).

**Per-batch verification command** (run only the targets converted in that task; needs Docker + ONNX):

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test <name1> --test <name2> ...
```

Expected: the named targets PASS. (Raw `cargo` validates macros live against `public`, which now carries the canonical schema — consistent with the offline cache.)

---

## Task 1: Move the macro cache to `public`; collapse compile-time CI

**Files:**
- Delete: `crates/temper-substrate/.sqlx/` (entire per-crate cache directory)
- Modify: workspace `.sqlx/` (regenerated to include substrate's queries)
- Modify: `Makefile.toml` (delete the `prepare-next` task)
- Modify: `.github/workflows/code-quality.yml` (remove substrate exclusion + separate offline steps)

**Interfaces:**
- Produces: substrate's `src` macros now resolve against `public`; the workspace `.sqlx` cache covers them. No per-crate cache, no `prepare-next`.

- [ ] **Step 1: Ensure the dev DB has the canonical schema in `public`**

```bash
cargo make docker-up
cargo make db-reset   # applies migrations/ (01 schema, 02 functions, 03 seed) to public
```

- [ ] **Step 2: Delete the per-crate cache and regenerate the workspace cache against `public`**

```bash
rm -rf crates/temper-substrate/.sqlx
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

Expected: `.sqlx/` (workspace root) now contains substrate's query files; `git status` shows new workspace cache entries and the deleted per-crate dir.

- [ ] **Step 3: Remove the substrate exclusion + separate offline steps in `code-quality.yml`**

Delete the two separate `temper-substrate` offline steps and drop `--exclude temper-substrate` from the workspace clippy + doc passes, so they read:

```yaml
      - name: Run Clippy lints
        run: cargo clippy --workspace --all-targets --all-features -- -D warnings

      - name: Check documentation builds
        run: cargo doc --workspace --no-deps --document-private-items
        env:
          RUSTDOCFLAGS: -D warnings
```

(Remove the now-stale comment block above the clippy step that explains the exclusion.)

- [ ] **Step 4: Delete the `prepare-next` task from `Makefile.toml`**

Remove the entire `[tasks.prepare-next]` block.

- [ ] **Step 5: Verify offline check is green**

Run: `cargo make check`
Expected: PASS — substrate is now linted inside the workspace pass against the public-built workspace cache, with no separate step.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: move temper-substrate macro cache to public; fold into workspace cache

Re-prepare against public (queries are unqualified -> byte-identical), drop the
per-crate .sqlx, remove the code-quality --exclude temper-substrate + separate
offline clippy/doc steps, delete prepare-next."
```

---

## Task 2: Add `reset_schema` helper + `test-artifacts` make task; convert the pattern-setters

**Files:**
- Modify: `crates/temper-substrate/tests/common/mod.rs` (add `reset_schema`; keep `reset_artifact*` for now)
- Modify: `Makefile.toml` (add `test-artifacts`)
- Modify: `crates/temper-substrate/tests/scenario_load.rs`, `scenario_steps.rs`, `corpus_smoke.rs` (convert per recipe)

**Interfaces:**
- Produces: `common::reset_schema(pool: &sqlx::PgPool)` — drops + rebuilds the substrate schema (clean, unseeded `01+02`) in the pool's database. Consumed by every production-seed-sensitive or mid-body-reset conversion (Tasks 3–8).
- Produces: `cargo make test-artifacts` — runs the full write-path suite on ephemeral DBs against a plain `public` `DATABASE_URL`.

- [ ] **Step 1: Add `reset_schema` to `common/mod.rs`**

Add (the two paths reuse the canonical migration bodies already loaded by `MIGRATOR`):

```rust
/// Reset the substrate schema IN THE CURRENT DATABASE to a clean, unseeded `01+02` baseline
/// (the public-schema analog of the retired `temper_next` `reset_artifact`). Use as a test's
/// first line when the production seed from `MIGRATOR` would perturb a global count or a
/// replay/projection diff, or to re-clean between snapshot→replay phases.
pub async fn reset_schema(pool: &sqlx::PgPool) {
    use sqlx::Executor;
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    pool.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("drop/recreate public schema");
    for f in [
        "migrations/20260624000001_canonical_schema.sql",
        "migrations/20260624000002_canonical_functions.sql",
    ] {
        let sql = std::fs::read_to_string(format!("{root}/{f}")).expect("read canonical sql");
        pool.execute(sql.as_str())
            .await
            .unwrap_or_else(|e| panic!("apply {f}: {e}"));
    }
}
```

- [ ] **Step 2: Add the `test-artifacts` task to `Makefile.toml`**

```toml
[tasks.test-artifacts]
description = "Run temper-substrate write-path artifact tests on ephemeral per-test databases (public schema). Needs Docker Postgres + ONNX."
env = { SQLX_OFFLINE = "true", DATABASE_URL = "postgresql://temper:temper@localhost:5437/temper_development" }
script = '''
cargo nextest run -p temper-substrate --features artifact-tests
'''
```

- [ ] **Step 3: Convert `scenario_load.rs`, `scenario_steps.rs`, `corpus_smoke.rs` per the recipe**

Apply the canonical recipe to each `#[tokio::test]` in these three files. `scenario_load.rs` keeps `use temper_substrate::substrate;` (it calls `substrate::load`). Example final shape for `scenario_load.rs`'s first test:

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn loads_minimal_seed_into_readable_substrate(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool).await.unwrap();
    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    assert!(loaded.keys.contains_key("telos"));
    assert!(loaded.keys.contains_key("a"));
    let sub = substrate::load(&pool, loaded.cogmap, "telos-default").await.unwrap();
    assert_eq!(sub.nodes.len(), 3, "telos + a + b are homed");
    assert_eq!(sub.edges.len(), 2, "leads_to(a->b) + express(telos->a)");
    assert_eq!(sub.facets.len(), 1, "one facet on resource a");
}
```

- [ ] **Step 4: Verify — this is the production-seed coexistence proof**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test scenario_load --test scenario_steps --test corpus_smoke
```
Expected: PASS. `scenario_load`'s exact `nodes.len()==3 / edges.len()==2 / facets.len()==1` passing against a production-seeded ephemeral DB proves cogmap-scoped assertions are isolated from the L0 kernel. **If any exact-count assertion now fails**, add `common::reset_schema(&pool).await;` as that test's first line and re-run.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: add reset_schema helper + test-artifacts task; convert scenario_load/scenario_steps/corpus_smoke to #[sqlx::test]"
```

---

## Task 3: Convert batch A (scenario/charter/content family)

**Files (convert per recipe):**
- `crates/temper-substrate/tests/bootseed.rs`
- `crates/temper-substrate/tests/charter_block_roles.rs`
- `crates/temper-substrate/tests/charter_yaml_roundtrip.rs`
- `crates/temper-substrate/tests/content_multichunk.rs`
- `crates/temper-substrate/tests/content_mutation.rs`

**Interfaces:** Consumes `common::reset_schema` (Task 2) for any mid-body reset.

- [ ] **Step 1: Apply the canonical recipe to every `#[tokio::test]` in the five files.** For any `common::reset_artifact()` that appears after the test has done work (mid-body), substitute `common::reset_schema(&pool).await;`. Leading resets are deleted (the injected pool is already clean).

- [ ] **Step 2: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test bootseed --test charter_block_roles --test charter_yaml_roundtrip \
  --test content_multichunk --test content_mutation
```
Expected: PASS. If a global/exact-count assertion fails, prepend `common::reset_schema(&pool).await;`.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert batch A (bootseed/charter/content) to #[sqlx::test]"
```

---

## Task 4: Convert batch B (drift/readout/corpus/chunk family)

**Files (convert per recipe):**
- `crates/temper-substrate/tests/chunk_heading_carry.rs`
- `crates/temper-substrate/tests/corpus_growth.rs`
- `crates/temper-substrate/tests/drift_signal.rs`
- `crates/temper-substrate/tests/readout_tier.rs`
- `crates/temper-substrate/tests/seed_corpus_sweep.rs`

- [ ] **Step 1: Apply the canonical recipe to every `#[tokio::test]` in the five files** (mid-body resets → `reset_schema`).

- [ ] **Step 2: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test chunk_heading_carry --test corpus_growth --test drift_signal \
  --test readout_tier --test seed_corpus_sweep
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert batch B (drift/readout/corpus/chunk) to #[sqlx::test]"
```

---

## Task 5: Convert batch C (genesis/ledger/identity/incremental/access family)

**Files (convert per recipe):**
- `crates/temper-substrate/tests/cogmap_genesis_charter.rs`
- `crates/temper-substrate/tests/ledger_envelope.rs`
- `crates/temper-substrate/tests/identity_graft_test.rs`
- `crates/temper-substrate/tests/incremental_equivalence.rs`
- `crates/temper-substrate/tests/access_scenario.rs`

**Note:** `incremental_equivalence.rs` and `access_scenario.rs` each call `reset_artifact` four times — these are mostly one-reset-per-`#[tokio::test]` (leading → delete). Confirm by position; any genuinely mid-body reset → `reset_schema`.

- [ ] **Step 1: Apply the canonical recipe to every `#[tokio::test]` in the five files.**

- [ ] **Step 2: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test cogmap_genesis_charter --test ledger_envelope --test identity_graft_test \
  --test incremental_equivalence --test access_scenario
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert batch C (genesis/ledger/identity/incremental/access) to #[sqlx::test]"
```

---

## Task 6: Convert the replay/equivalence mid-body resetters

**Files:**
- `crates/temper-substrate/tests/replay_roundtrip.rs`
- `crates/temper-substrate/tests/seed_load_path_equivalence.rs`

**Why separate:** These snapshot→reset→replay within one test body, so they depend on `reset_schema`'s clean-unseeded semantics, and they diff *projections*, which the production seed could perturb.

**Interfaces:** Consumes `common::reset_schema` and `common::telos_default_partition`.

- [ ] **Step 1: Convert `seed_load_path_equivalence.rs`.** Make `common::reset_schema(&pool).await;` the **first** line (clean, unseeded baseline — matches the old `reset_artifact` start), delete `connect()`, inject `pool`. Replace each mid-body `common::reset_artifact()` (the snapshot→reset→replay step and the Path-B re-clean) with `common::reset_schema(&pool).await;`. For Path B's second `let pool = substrate::connect()...`, drop the re-bind and reuse the same injected `pool` after `reset_schema`. The `bootseed::seed_system`, `loader::load_seed`, `embed::embed_chunks`, `materialize_cogmap`, `replay::{dump_projections,snapshot,replay}`, and `common::telos_default_partition` calls stay.

- [ ] **Step 2: Convert `replay_roundtrip.rs`** the same way — `reset_schema` first line, mid-body resets → `reset_schema`, single injected pool.

- [ ] **Step 3: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test replay_roundtrip --test seed_load_path_equivalence
```
Expected: PASS — projection diffs are byte-identical. If a diff includes unexpected production-seed rows, confirm `reset_schema` runs before the first snapshot.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert replay/equivalence tests (mid-body reset_schema) to #[sqlx::test]"
```

---

## Task 7: Convert the SQL-hardcoded files (de-qualify `temper_next`)

**Files:**
- `crates/temper-substrate/tests/graph_functions.rs` (`SET LOCAL search_path TO temper_next, public` ×2)
- `crates/temper-substrate/tests/write_path_mutations.rs` (`SET LOCAL` ×3, `FROM temper_next.kb_edges` ×2)
- `crates/temper-substrate/tests/invocation_envelope.rs` (`SET LOCAL` ×4, `table_schema='temper_next'` ×2)

**Why separate:** Beyond the recipe, these embed `temper_next` in SQL strings. On an ephemeral `public` DB the schema is the connection default, so the qualification must go.

- [ ] **Step 1: Apply the canonical recipe to every test in the three files** (inject pool, drop `connect()`/leading reset).

- [ ] **Step 2: De-qualify the SQL in each file:**
  - Delete every `sqlx::query("SET LOCAL search_path TO temper_next, public")...` statement (and its `.execute(&mut *tx)`/`.execute(&pool)` chain). The ephemeral DB's default `public` search_path is already correct.
  - `FROM temper_next.kb_edges` → `FROM kb_edges` (both occurrences in `write_path_mutations.rs`).
  - `WHERE table_schema='temper_next'` → `WHERE table_schema='public'` (both `information_schema` introspection queries in `invocation_envelope.rs`, lines 34 and 48).

- [ ] **Step 3: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test graph_functions --test write_path_mutations --test invocation_envelope
```
Expected: PASS — the introspection queries find the tables/columns in `public`; the edge counts read from the unqualified tables.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert SQL-hardcoded tests (graph_functions/write_path_mutations/invocation_envelope), de-qualify temper_next"
```

---

## Task 8: Convert `scenario_roundtrip.rs` and retire the SQL-vs-YAML parity test

**Files:**
- Modify: `crates/temper-substrate/tests/scenario_roundtrip.rs`

**Why:** Two of its three tests are YAML-path and convert normally; the third (`yaml_and_sql_seed_paths_produce_identical_region_membership`) is the transitional SQL-vs-YAML parity proof and is retired per the spec.

- [ ] **Step 1: Convert `passes_full_s6_runbook` and `baseline_matches_04b_sql_verdict`** per the recipe (inject pool, drop `connect()`/leading `reset_artifact`; `bootseed::seed_system`, `runner::run_scenario`, `loader::load_seed`, `embed::embed_chunks`, `materialize_cogmap`, `verify_ledger_roundtrip`, and the `VERDICT_SQL` query all stay).

- [ ] **Step 2: Delete the parity test and its now-orphaned helper.** Remove the entire `yaml_and_sql_seed_paths_produce_identical_region_membership` test fn and the file-local `genesis_emitter` async helper (used only by it). Remove the now-unused `ONBOARDING_SEED`/`load_seed_yaml` only if `baseline_matches_04b_sql_verdict` no longer references them — it does reference `load_seed_yaml()`, so keep them. Update the file's `//!` doc header to drop the third bullet (the `…identical_region_membership` description).

- [ ] **Step 3: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests --test scenario_roundtrip
```
Expected: PASS — two tests run, the parity test is gone.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: convert scenario_roundtrip; retire transitional SQL-vs-YAML parity test + genesis_emitter helper"
```

---

## Task 9: Tier 2 — port the legacy read-path assertions; delete the legacy tests + feature

**Files:**
- Create: `crates/temper-substrate/tests/readout_invariants.rs`
- Modify: `crates/temper-substrate/tests/scenario_load.rs` (add lens-binding + lens-mirror tests)
- Delete: `crates/temper-substrate/tests/materialize.rs`, `substrate_read.rs`, `embed_job.rs`
- Modify: `crates/temper-substrate/Cargo.toml` (delete `artifact-tests-legacy` feature)

**Interfaces:** The five unique legacy assertions, re-homed onto `#[sqlx::test]` anchored on the YAML seed `tests/fixtures/seeds/onboarding-cogmap.yaml` (loaded via `loader::load_seed`), not `03_seed.sql`.

- [ ] **Step 1: Create `readout_invariants.rs` with the materialize/embed guards (from `materialize.rs` + `embed_job.rs`)**

```rust
#![cfg(feature = "artifact-tests")]
//! Ported from the retired legacy read-path tests: the readout regression guards (no NULL
//! content_cohesion, no NaN salience/telos_alignment, >=2 emergent regions) and the embed
//! completeness invariant — now over the YAML onboarding seed on an ephemeral DB.
mod common;

use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};

const ONBOARDING_SEED: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seeds/onboarding-cogmap.yaml");

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING_SEED).unwrap()).unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn materialize_populates_finite_readouts_and_multiple_regions(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    let first = write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    let second = write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    assert_eq!(first.membership_fingerprint, second.membership_fingerprint, "reproducible");
    assert!(first.regions >= 2, "expected >=2 emergent regions on the enriched seed");

    let nulls = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE content_cohesion IS NULL AND NOT is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nulls, 0, "all live regions have a computed content_cohesion");

    let nan = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE NOT is_folded \
         AND (salience = 'NaN'::float8 OR telos_alignment = 'NaN'::float8)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nan, 0, "no live region may have NaN salience or telos_alignment");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn embed_leaves_no_current_chunk_unembedded(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let unembedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.is_current AND NOT b.is_folded AND ch.embedding IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unembedded, 0, "embed job must leave no current chunk with content unembedded");

    let embedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(embedded > 0, "expected >=1 embedded current chunk");
}
```

- [ ] **Step 2: Add the two lens tests from `substrate_read.rs` into `scenario_load.rs`**

The `substrate_read.rs` lens-binding and lens-mirror assertions anchor cleanly on the existing `MINIMAL` seed already in `scenario_load.rs`:

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn lens_name_parameter_binds_the_lens_query(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool).await.unwrap();
    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    substrate::load(&pool, loaded.cogmap, "telos-default")
        .await
        .expect("telos-default lens loads by name");
    let bogus = substrate::load(&pool, loaded.cogmap, "no-such-lens").await;
    assert!(bogus.is_err(), "loading an unknown lens name must error");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seeded_telos_default_lens_mirrors_the_rust_default(pool: sqlx::PgPool) {
    use temper_substrate::affinity::Lens;
    temper_substrate::scenario::bootseed::seed_system(&pool).await.unwrap();
    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    let sub = substrate::load(&pool, loaded.cogmap, "telos-default").await.unwrap();
    let d = Lens::telos_default();
    assert_eq!(sub.lens.w_express, d.w_express, "w_express");
    assert_eq!(sub.lens.w_contains, d.w_contains, "w_contains");
    assert_eq!(sub.lens.w_leads_to, d.w_leads_to, "w_leads_to");
    assert_eq!(sub.lens.w_near, d.w_near, "w_near");
    assert_eq!(sub.lens.w_prop, d.w_prop, "w_prop");
    assert_eq!(sub.lens.s_telos, d.s_telos, "s_telos");
    assert_eq!(sub.lens.s_ref, d.s_ref, "s_ref");
    assert_eq!(sub.lens.s_central, d.s_central, "s_central");
    assert_eq!(sub.lens.resolution, d.resolution, "resolution");
}
```

- [ ] **Step 3: Delete the three legacy test files**

```bash
git rm crates/temper-substrate/tests/materialize.rs \
       crates/temper-substrate/tests/substrate_read.rs \
       crates/temper-substrate/tests/embed_job.rs
```

- [ ] **Step 4: Delete the `artifact-tests-legacy` feature from `Cargo.toml`**

Remove the `artifact-tests-legacy = []` line and its comment block.

- [ ] **Step 5: Verify**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo nextest run -p temper-substrate --features artifact-tests \
  --test readout_invariants --test scenario_load
```
Expected: PASS — `materialize_populates_finite_readouts_and_multiple_regions` proves the `>=2 regions` + no-NaN guards on the YAML seed; the two lens tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "WS6 Tier3 (Tier2 fold-in): port legacy read-path assertions to #[sqlx::test]; delete materialize/substrate_read/embed_job + artifact-tests-legacy feature"
```

---

## Task 10: Retire the `temper_next` harness, fixtures, and nextest serialization

**Files:**
- Modify: `crates/temper-substrate/tests/common/mod.rs` (delete `reset_artifact`, `reset_artifact_with_seed`, `load_files`)
- Delete: `crates/temper-substrate/tests/fixtures/00_namespace_reset.sql`, `crates/temper-substrate/tests/fixtures/03_seed.sql`
- Modify: `.config/nextest.toml` (delete the `temper-substrate-write` group + override)
- Modify: `crates/temper-substrate/src/substrate.rs` (rewrite the `connect()` comment)
- Modify: `Makefile.toml` (delete `test-next`)

**Interfaces:** Consumes — by this point no test references `reset_artifact`/`reset_artifact_with_seed`/`load_files` or the two SQL fixtures.

- [ ] **Step 1: Confirm no remaining references before deleting**

```bash
grep -rn "reset_artifact\|reset_artifact_with_seed\|load_files\|03_seed\|00_namespace_reset" crates/temper-substrate/tests/ | grep -v "reset_schema"
```
Expected: no hits in test bodies (only the `common/mod.rs` definitions about to be removed). If any test still calls them, it was missed in Tasks 3–8 — convert it before continuing.

- [ ] **Step 2: Delete `reset_artifact`, `reset_artifact_with_seed`, and `load_files` from `common/mod.rs`.** Keep `reset_schema`, `insert_profile`, `insert_context`, `telos_default_partition`, `telos_default_readout_signature`, `fire_resource_with_headed_chunk`. Update the `//!` module doc header to describe the ephemeral-DB model (drop the `temper_next`/legacy-seed prose).

- [ ] **Step 3: Delete the SQL fixtures**

```bash
git rm crates/temper-substrate/tests/fixtures/00_namespace_reset.sql \
       crates/temper-substrate/tests/fixtures/03_seed.sql
```

- [ ] **Step 4: Delete the `temper-substrate-write` nextest group** from `.config/nextest.toml` — remove the `[test-groups] temper-substrate-write = ...` entry and the `[[profile.default.overrides]]` block whose `test-group = 'temper-substrate-write'`. (Per-test-DB isolation makes serialization unnecessary.)

- [ ] **Step 5: Rewrite the `connect()` comment in `substrate.rs`.** Replace the `temper_next,public` framing with: the connection's search_path is the database default (`public`) in production, dev, and tests; ephemeral test databases are provided by `#[sqlx::test]`.

- [ ] **Step 6: Delete the `[tasks.test-next]` block from `Makefile.toml`** (`test-artifacts` replaces it).

- [ ] **Step 7: Verify the whole suite runs together on ephemeral DBs**

```bash
cargo make docker-up
cargo make test-artifacts
```
Expected: PASS — all converted write-path tests + `readout_invariants` run together against ephemeral `public` databases, no serialization group, no `temper_next`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: delete temper_next harness (reset_artifact/load_files), 00_namespace_reset.sql, 03_seed.sql, the temper-substrate-write nextest group, and test-next; rewrite connect() comment"
```

---

## Task 11: Collapse the E2E/Coverage CI exclusions; enable artifact-tests in the Embed job

**Files:**
- Modify: `.github/workflows/test-rust.yml` (E2E `:115`, Coverage `:270`, Embed job `:118`)

**Interfaces:** Consumes — substrate macros now resolve against `public` (Task 1); the test DB in CI carries the canonical schema in `public`.

- [ ] **Step 1: Remove `--exclude temper-substrate` from the E2E job** (`test-rust.yml:115`). Re-evaluate `--exclude temper-agents` on the same line: temper-agents was excluded only because it depends on substrate; with substrate resolving against `public`, remove `--exclude temper-agents` too. Leave `--exclude temper-cloud`. Result:

```yaml
        run: cargo nextest run --workspace --exclude temper-cloud --features test-db --locked --no-fail-fast --test-threads 1
```

- [ ] **Step 2: Remove the same exclusions from the Coverage job** (`:270`):

```yaml
        run: cargo llvm-cov nextest --workspace --exclude temper-cloud --features test-db --locked --no-fail-fast --test-threads 1 --lcov --output-path lcov.info
```

- [ ] **Step 3: Enable `artifact-tests` in the "Embed & MCP Round-Trip Tests" job** (`:118`). Add `--features artifact-tests` to its nextest invocation (alongside the existing `test-embed`/`test-db` features), keeping the job's existing `--test-threads 1` (ort/bge contention). The Embed job already installs ONNX and a Postgres service, which are the only requirements.

- [ ] **Step 4: Verify the workflow is well-formed and substrate builds in the un-excluded workspace passes locally**

```bash
cargo make check          # workspace clippy/doc now include substrate, no exclusion
cargo nextest run --workspace --exclude temper-cloud --features test-db --locked --no-fail-fast --test-threads 1
```
Expected: both PASS locally (the E2E/Coverage un-exclusion mirrors these commands). Note: `artifact-tests` is NOT in `test-db`, so this run does not execute the ephemeral-DB suite — that runs in the Embed job via `cargo make test-artifacts` equivalently.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: drop --exclude temper-substrate/temper-agents from E2E+Coverage; run artifact-tests in the Embed CI job"
```

---

## Task 12: Documentation sweep

**Files:**
- Modify: `CLAUDE.md`
- Modify: `migrations/20260624000001_canonical_schema.sql`, `…02_canonical_functions.sql`, `…03_canonical_seed.sql` (comment updates only)
- Modify: `crates/temper-substrate/src/{writes.rs,events.rs,readback/mod.rs,scenario/mod.rs,scenario/bootseed.rs}` (comment updates only)

- [ ] **Step 1: Update `CLAUDE.md`.** Remove the `temper_next` narrative: the "temper-substrate sqlx macros target the `temper_next` namespace (offline cache)" subsection, the `prepare-next`/`test-next` references, and the `artifact-tests-legacy` bullet + its run-line. Reframe `artifact-tests` as: local + Embed-CI ONNX-gated write-path tests on ephemeral `#[sqlx::test]` databases (run `cargo make test-artifacts`). Update the SQL-checking section to drop the per-crate `temper-substrate` cache exception and the `--exclude temper-substrate` notes.

- [ ] **Step 2: Update the canonical migration comments.** In the three `migrations/2026062400000{1,2,3}_*.sql` files, replace the "temper-next proving-ground / test namespace" comments with: tests apply this schema to ephemeral `public`-schema databases via `#[sqlx::test]`.

- [ ] **Step 3: Update substrate src comments.** Remove/rewrite the `temper_next` mentions in `writes.rs`, `events.rs`, `readback/mod.rs`, `scenario/mod.rs`, `bootseed.rs` (e.g. bootseed's "needs a `temper_next`-bound pool" line → "needs a substrate pool"). Comments only — no code changes.

- [ ] **Step 4: Final repo-wide verification — no `temper_next` outside historical docs**

```bash
grep -rn "temper_next" . --include=*.rs --include=*.toml --include=*.yml --include=*.sql --include=*.md \
  | grep -v "/target/" | grep -v "docs/superpowers/plans/" | grep -v "docs/superpowers/specs/"
```
Expected: no hits (all live references removed; only historical plans/specs retain the name).

- [ ] **Step 5: Full local gate**

```bash
cargo make check
cargo make test-artifacts
```
Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "WS6 Tier3: docs sweep — remove temper_next narrative from CLAUDE.md, migration + substrate src comments"
```

---

## Self-review notes (for the executor)

- **Spec coverage:** Component 1 → Task 1. Component 2 (test rewrite) → Tasks 2–8. Component 3 (CI collapse) → Tasks 1 (clippy/doc) + 11 (E2E/Coverage). Component 4 (Embed CI) → Task 11. Component 5 (Tier 2 + 03_seed retirement) → Tasks 8 (parity) + 9 (legacy port) + 10 (03_seed delete). Component 6 (docs) → Task 12.
- **The one real risk** is production-seed bleed into scoped/global assertions. It is surfaced at the earliest possible point (Task 2, Step 4) and the mitigation (`reset_schema` first line) is in the recipe. Do not assume — run each batch's targets.
- **Ordering invariant:** the harness (`reset_artifact`/fixtures/nextest group) is removed only in Task 10, after every test is converted (Tasks 2–9). Task 10 Step 1 is a guard that fails loudly if a test was missed.
