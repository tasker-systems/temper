# WS6 Tier 3 — Eliminate the `temper_next` namespace

**Date:** 2026-06-26
**Status:** Design — approved direction, pending spec review
**Goal alignment:** substrate-kernel-to-cognitive-map. This is the Option-2-adjacent
namespace collapse the WS6 Spec B crate-split deferred. It also closes the documented
crate↔namespace mismatch (`temper-substrate` crate over a `temper_next` namespace) from
the crate-split spec's "Known compromises #2".
**Predecessor:** `docs/superpowers/specs/2026-06-25-ws6-spec-b-substrate-workflow-crate-split-design.md`
(rename + workflow extraction, merged as #172/#175/#176). Tier 1 of the post-rename audit
(dead flip/re-home tooling) landed in #176. This spec is Tier 2 + Tier 3 of that audit.

## Problem

`temper-substrate` is the only crate whose `sqlx::query!` macros resolve against a
non-`public` Postgres namespace, `temper_next`. That single fact forces a whole apparatus:

- A per-crate offline cache `crates/temper-substrate/.sqlx`, prepared with
  `search_path=temper_next` (the `prepare-next` make task).
- A separate runtime namespace the artifact tests build, reset, and tear down via a
  `00_namespace_reset` fixture + `PGOPTIONS=-csearch_path=temper_next,public` wrappers
  (`crates/temper-substrate/tests/common/mod.rs`), serialized through the
  `temper-substrate-write` nextest group because they share one mutable namespace.
- A `test-next` make task that points the test pool at `temper_next` via the search_path
  connection option.
- CI special-casing: `--exclude temper-substrate` from the `--workspace` clippy + doc
  passes (with separate offline `-p temper-substrate` steps) in `code-quality.yml`, and
  `--exclude temper-substrate` from the E2E + Coverage jobs in `test-rust.yml`
  (`temper-agents` is excluded transitively because it depends on substrate).

Production has been fully single-schema `public` since the 2026-06-25 re-home. `temper_next`
no longer reflects any production reality — it is pure test/build scaffolding. The substrate
write-path artifact tests **do not run in any CI job**, so the entire cognitive-map substrate
write path is untested in CI.

## Key findings (grounded in current code)

These observations make the elimination tractable and low-risk:

1. **Compile-time resolution is schema-agnostic.** `crates/temper-substrate/src` has 65
   `query!`/`query_as!`/`query_scalar!` macro invocations across 9 files, all using
   **unqualified** table names. The committed `.sqlx` cache contains **zero** references to
   `temper_next`. The canonical schema (`migrations/20260624000001_canonical_schema.sql` +
   `…02_canonical_functions.sql`) builds the identical substrate in whatever schema the
   connection's `search_path` selects. Re-preparing the cache against `public` therefore
   produces byte-identical cache entries — the move is mechanical.

2. **Runtime tests need no schema at compile time.** The substrate `tests/` use **0**
   compile-time macros and **122** runtime `sqlx::query()` calls. They are valid against any
   schema-identical namespace; nothing about them binds to `temper_next` except the harness
   that builds it.

3. **The ephemeral-DB pattern already exists in this crate.** `temper_substrate::MIGRATOR`
   (`crates/temper-substrate/src/lib.rs:28`, `sqlx::migrate!("../../migrations")`) is already
   defined, and `crates/temper-substrate/tests/context_shape.rs` already runs on
   `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` with an injected `pool`. This is
   the same convention temper-api uses (`temper_api::MIGRATOR`, 13 test files). Full
   elimination is converting the remaining write-path tests to this proven pattern.

4. **The production seed coexists with bootseed.** `MIGRATOR` applies all three canonical
   migrations including `…03_canonical_seed.sql` (the L0 kernel + system actor). The
   write-path tests call `scenario::bootseed::seed_system`, which is **idempotent by design**
   (`ON CONFLICT (handle) DO UPDATE` for the system profile, existence-checks for the system
   entity and global lenses, `ON CONFLICT (name)` for event types). Its doc comment states it
   exists precisely so "the migration's synthesis bootstrap can seed the registry." So an
   ephemeral DB carrying the production seed + a `seed_system` top-up is consistent, which is
   why `context_shape` already works on the full migrator.

## Design

### Decision: decouple, then fully eliminate

Compile-time macro resolution and runtime test isolation are independent concerns that both
currently lean on `temper_next`. We sever both:

- **Compile-time → `public`.** Re-prepare substrate's macro cache against `public`, so its
  macros resolve like every other crate's.
- **Runtime → ephemeral per-test databases.** Convert the write-path artifact tests to
  `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, removing the `temper_next` runtime
  namespace entirely.

With both severed, every piece of machinery that exists only to serve `temper_next` is
deleted.

### Component 1 — macro cache moves to `public`

- Drop the `temper_next` search_path from cache preparation. Because substrate's `tests/`
  carry no compile-time macros, the workspace prepare ritual
  (`cargo sqlx prepare --workspace -- --all-features`) now covers substrate's `src` fully.
- **Delete** the per-crate `crates/temper-substrate/.sqlx` cache; substrate's queries fold
  into the workspace `.sqlx` cache.
- **Delete** the `prepare-next` make task.

### Component 2 — test-harness rewrite (the bulk of the work)

Convert the **24** `artifact-tests`-gated write-path test files that still use
`substrate::connect()` + `common::reset_artifact*()` (the full set minus the already-converted
`context_shape.rs`) to the injected-pool pattern:

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn some_write_path_test(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();   // idempotent top-up over the production seed
    // ... load_seed / scenario / materialize, exactly as today
}
```

- **Migrator choice: reuse the full `temper_substrate::MIGRATOR`** (schema + functions +
  production seed). Rationale: it is the already-proven pattern (`context_shape`), bootseed is
  idempotent against it, and it tests against the real production baseline rather than a
  synthetic schema-only one. We do **not** introduce a separate schema-only migrator or a
  `test-migrations/` directory.
- **Isolation is now per-test-database.** Delete the `temper-substrate-write` nextest group
  and its serialization (`.config/nextest.toml`): `#[sqlx::test]` gives each test its own
  database, so a reset can no longer race a sibling.
- **Replace** `common::{reset_artifact, reset_artifact_with_seed, load_files}` and
  `tests/fixtures/00_namespace_reset.sql` — they have no role once tests receive a fresh
  migrated pool. `common::reset_artifact_with_seed` and `03_seed.sql` are also retired (see
  Component 5), so no test loads a hand-SQL seed.

**Verification gates (resolve during implementation, not assumed here):**
- Exact-count assertions (e.g. `scenario_load`'s `nodes.len() == 3`, `edges.len() == 2`) must
  remain correct against a production-seeded DB. They query cogmap-scoped, and the L0 kernel is
  a distinct cogmap, so they should be isolated — confirm by running.
- Any test that assumed an *empty* `public` (no production seed) must be re-grounded.

### Component 3 — CI collapse (falls out of Components 1–2)

- `code-quality.yml`: remove `--exclude temper-substrate` from the clippy and doc
  `--workspace` passes and delete the separate offline `-p temper-substrate` clippy/doc steps
  — substrate folds back into the workspace passes.
- `test-rust.yml`: remove `--exclude temper-substrate` from the E2E (`:115`) and Coverage
  (`:270`) jobs. Re-evaluate and, if now unnecessary, remove the transitive
  `--exclude temper-agents` in the same lines (`temper-cloud`'s exclusion is separate — leave
  it).

### Component 4 — enable artifact-tests in CI (Embed job)

The write-path suite currently runs in no CI job. After the rewrite its only requirements are a
Postgres DB and ONNX (bge-768), both already present in the **"Embed & MCP Round-Trip Tests"**
job (`test-rust.yml:118`, ONNX installed at `:89–102`).

- Enable `--features artifact-tests` in that job so the cognitive-map substrate write path is
  CI-covered.
- The `artifact-tests` feature gate **stays** (still local-heavy: DB + ONNX, not part of the
  default unit suite).
- **Concurrency note:** per-test-DB isolation removes the *database* reason to serialize, but
  the Embed job already runs `--test-threads 1` because server-side bge embedding contends on
  `ort` (`test-rust.yml:112–113`). The embed-heavy artifact tests inherit that constraint;
  keep `--test-threads 1` in the Embed job. Local parallelism is now possible for non-embed
  tests but is not required by this spec.
- Replace the `test-next` make task with a successor (e.g. `test-artifacts`) that runs
  `cargo nextest run -p temper-substrate --features artifact-tests` against a normal `public`
  `DATABASE_URL` (no search_path option), for local runs.

### Component 5 — Tier 2: fold in the legacy read-path tests

The three `artifact-tests-legacy` tests (`materialize.rs`, `substrate_read.rs`,
`embed_job.rs`) are **not** superseded by the write-path suite — they carry ~5 unique
assertions: the no-NaN / no-NULL readout regression guards and ≥2-emergent-regions
(`materialize`), the seeded-`telos-default`-lens-mirrors-`Lens::telos_default()` invariant and
the bogus-lens-name-errors check (`substrate_read`), and the embed-completeness invariant
(`embed_job`). Port these assertions into `#[sqlx::test]`-based tests (anchored on the existing
`tests/fixtures/seeds/onboarding-cogmap.yaml`, which already mirrors `03_seed`'s
onboarding-cogmap), then:

- **Delete** the three legacy test files and the `artifact-tests-legacy` feature
  (`Cargo.toml:18`).
- **Remove** the `artifact-tests-legacy` mention in `CLAUDE.md:147` and its doc run-line.

**Retire the in-SQL `03_seed.sql`.** The SQL-vs-YAML region-membership parity test
(`scenario_roundtrip::yaml_and_sql_seed_paths_produce_identical_region_membership`) was
transitional scaffolding: it existed only to prove the YAML scenario reproduced the structural
correctness originally established in the hand-written `03_seed.sql`. That equivalence is now
proven and the YAML seed is the single source of truth, so the in-SQL seed is fully deprecated:

- **Delete** `tests/fixtures/03_seed.sql`, the parity test, its file-local `genesis_emitter`
  helper, and `common::reset_artifact_with_seed` (its only caller).
- **Retain** the boot-seed `system.yaml` and the YAML seed/scenario fixtures
  (`seeds/onboarding-cogmap.yaml`, `scenarios/onboarding-cogmap.yaml`) — still the live inputs.
- The other two `scenario_roundtrip` tests (`passes_full_s6_runbook`,
  `baseline_matches_04b_sql_verdict`, both YAML-path) convert to `#[sqlx::test]` normally.

### Component 6 — docs & comments

- `crates/temper-substrate/src/substrate.rs` — rewrite the `connect()` comment: the dev/test
  default is now `public`; drop the `temper_next,public` framing. Audit the `temper_next`
  mentions in `writes.rs`, `events.rs`, `readback/mod.rs`, `scenario/mod.rs`, `bootseed.rs`.
- `migrations/2026062400000{1,2,3}_*.sql` — update the "temper-next proving-ground" comments to
  reflect that tests now use ephemeral `public`-schema databases.
- `CLAUDE.md` — remove the `temper_next` narrative: the "temper-substrate sqlx macros target
  the `temper_next` namespace" subsection, the `prepare-next`/`test-next` references, the
  `artifact-tests`/`artifact-tests-legacy` namespace framing (keep `artifact-tests` as the
  local+Embed-CI ONNX feature), and the `--exclude temper-substrate` SQL-checking notes.
- `crates/temper-substrate/tests/common/mod.rs` doc header — rewrite for the ephemeral-DB model.

## Out of scope

### Rejected (load-bearing decisions — resist scope creep)

- **Schema-only test migrator / `test-migrations/` directory.** Rejected in favour of reusing
  the full `temper_substrate::MIGRATOR`, because bootseed is idempotent against the production
  seed and `context_shape` already proves the full-migrator path. A curated subset migrator
  would add a symlink/duplication maintenance burden for no isolation benefit.
- **Keeping `temper_next` as a renamed runtime test namespace.** The earlier "decouple but
  keep a renamed isolation namespace" option is rejected: `#[sqlx::test]` supersedes the
  shared-namespace reset model entirely, so no named test namespace is needed.
- **Deleting the legacy read-path *assertions*.** Rejected — the three legacy tests carry ~5
  unique invariants; port them to `#[sqlx::test]`, don't drop. (Note: `03_seed.sql` itself and
  the SQL-vs-YAML parity test ARE retired — that parity was transitional scaffolding to prove
  the YAML scenario matched the original hand-SQL seed, now superseded by the proven YAML seed;
  see Component 5.)

### Deferred (legitimate, out of this spec's arc)

- **`scenario-schema` feature** (`Cargo.toml:20`) — gates the JSON-Schema snapshot test;
  orthogonal to the namespace, leave as-is.
- **`crates/temper-cli/src/backend_select.rs`** — misnamed after the retired dual-backend
  concept but not dead; optional rename is a separate QoL tidy.
- **`docs/guides/ws6-collapsed-dev-env.md`** — re-evaluate as possible doc cruft separately.
- **Local non-embed test parallelism tuning** — enabling parallel local runs for non-embed
  artifact tests is a possible later optimization, not required here.

## Acceptance criteria

- `crates/temper-substrate` builds and lints inside the standard `--workspace` clippy/doc
  passes with no per-crate exclusion or separate offline step.
- No `temper_next` references remain in code, config, make tasks, CI workflows, or `CLAUDE.md`
  (docs/superpowers history excepted). `00_namespace_reset.sql`, `prepare-next`, `test-next`,
  the per-crate `.sqlx`, and the `temper-substrate-write` nextest group are gone.
- The write-path artifact tests run green on `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`
  and execute in the Embed CI job under `--features artifact-tests`.
- `artifact-tests-legacy` is gone; its unique assertions survive as `#[sqlx::test]` tests.
- `03_seed.sql`, the SQL-vs-YAML parity test, and `reset_artifact_with_seed` are gone; the YAML
  seed/scenario fixtures are the single source of truth.
- `cargo make check` (offline, workspace `.sqlx`) and the full CI matrix pass.

## Risks

- **Production-seed bleed into scoped assertions.** Mitigated by cogmap-scoped queries +
  per-test verification; the implementation must run each converted test, not assume.
- **`ort`/bge contention in CI.** Mitigated by inheriting the Embed job's existing
  `--test-threads 1`.
- **Large mechanical diff** (~27 test files). Convert incrementally; the suite is the
  regression oracle. Keep `context_shape.rs` as the reference shape.
