# Canonical migrations in `public` + Neon baseline-reset — design

**Date:** 2026-06-23
**Goal:** `substrate-kernel-to-cognitive-map` (WS6 endgame)
**Branch:** `jct/ws6-endgame-flip`
**Status:** design approved; implementation plan to follow (writing-plans)

## Context

The WS6 endgame flip (Task 8) collapsed `temper-api` to a single backend over a single
schema. The entire production Rust graph (temper-api, temper-mcp, temper-client, temper-cli,
temper-agents, the Vercel `api/` adapters) now compiles clean against the collapsed shape —
committed as the atomic WIP `f66cb9e` on this branch.

That work left the flip's **tail** redirected by a new endgame decision:

> The canonical schema = `migrations/` building the substrate in **`public`**, not the
> `temper_next` namespace. `temper_next` was only ever the proving-ground beside legacy
> `public`; the endgame promotes the fresh schema to *be* `public`. `migrations/` becomes the
> single source of truth; `schema-artifact/` retires entirely; the `schema_drift` guard retires.

This spec covers that redirect: authoring the canonical baseline, deleting `schema-artifact/`
(rehoming the proving-ground test fixtures it carries), repointing the dev-DB / e2e harness /
sqlx caches to `public`, and the **Neon baseline-reset** — the `_sqlx_migrations` reconciliation
the cutover runbook explicitly punted as "the bootstrap-export spec's job"
(`docs/guides/ws6-endgame-collapse-runbook.md` step 9). **This spec is that job.**

It does **not** re-do the collapsed code. The code is location-agnostic (every SQL statement is
unqualified, resolving against `search_path`), so `public` vs `temper_next` is purely a
provisioning concern. Nothing in `f66cb9e` is wasted.

## The unification this enables

Because the collapsed SQL is unqualified, **one baseline SQL body can build either target**:

- **Production / dev / e2e** → default `search_path`, DDL lands in `public`.
- **temper-next proving-ground tests** → harness sets `search_path = temper_next, public`
  first, the *same* DDL lands in an isolated `temper_next` test namespace.

So "single source" is genuinely achievable: `migrations/` is the lone master, and the
proving-ground suite loads *that same baseline* into an isolated namespace via a `search_path`
wrapper. No second copy of the schema; no `schema_drift` guard to enforce a two-copy invariant.

Consequence: **temper-next keeps its own `temper_next`-bound `.sqlx` cache unchanged** — its
macros still resolve against its isolated test namespace. Only **temper-api** and **e2e** caches
move to `public`.

## Components

### 1. The canonical baseline (3 ordered migration files)

Replace the entire 46-file legacy lineage with a fresh baseline at a new timestamp
(`20260624000001…`), authored as the substrate-in-public. Three ordered files mirror the
artifact's structure, each independently reviewable:

| File | Content | Derived from |
|------|---------|--------------|
| `…01_canonical_schema.sql` | DDL: enums, tables, indexes, the grafted identity/infra layer | `schema-artifact/01_schema.sql`, minus `CREATE SCHEMA temper_next` / `SET search_path` |
| `…02_canonical_functions.sql` | All SQL functions (substrate + graph + access predicates) | `schema-artifact/02_functions.sql`, minus the namespace preamble |
| `…03_canonical_seed.sql` | System boot-seed (see §2) | `seeds/system.yaml` + `payloads/` snapshots + the audit below |

The 46 legacy migration files **delete** (git history preserves them; matches the repo's
"remove dead code, no premature backward-compat" ethos). `migrations/templates/` and
`migrations/CLAUDE.md` stay.

The bodies are authored by stripping the namespace wrapper from the artifact, NOT by a
generator — `tools/gen-install-migration.sh` and the artifact-as-master relationship retire with
`schema-artifact/`.

### 2. The system-seed manifest (audited)

An audit of every `INSERT`/DML across the legacy lineage + the `install_temper_next` / graft
migrations, cross-checked against which tables the substrate (`01_schema.sql`) actually still
has, produced this manifest.

**Required system seeds** (`…03_canonical_seed.sql`) — "what any temper system needs":

1. **`system` profile + entity** — `kb_profiles (handle='system', display_name='System',
   system_access='admin')` and a `kb_entities (name='system')` row. The substrate requires a
   NOT NULL event emitter; every seeded event is attributed to this actor. (Mirrors
   `temper_next::scenario::bootseed::seed_system`.)
2. **`kb_event_types` registry** — the 27 canonical event-type names from `seeds/system.yaml`,
   each with its `payload_schema` JSON inlined from the committed `payloads/<name>.v1.schema.json`
   snapshot (a name with no snapshot stays NULL = permissive), `schema_version = 1`.
3. **2 global lenses** — `telos-default` and `telos-default-propheavy` (exact weight vectors from
   `system.yaml`), created via the **`lens_create()` function** (event-sourced / replay-faithful,
   attributed to the system actor, `cogmap_id NULL`).
4. **`kb_system_settings (id=1, access_mode='open')`** — *newly identified by the audit, NOT in
   `system.yaml`.* The access-gate predicates `has_system_access` / `is_system_admin` read
   `access_mode` + `gating_team_slug` from `kb_system_settings LIMIT 1`. With **zero rows** the
   `settings` CTE is empty and the functions return **NULL, not false** — breaking access checks.
   The legacy lineage seeded exactly `(1, 'open')` (`20260407000001_system_access_gate.sql:21`),
   the single-tenant default; the baseline must too.

**Excluded — tenant data, not system seed** (decision: keep the baseline minimal):

- The 5 named contexts (temper / storyteller / tasker / knowledge / writing) and the `Anonymous`
  profile from the legacy seed are temperkb.io-specific tenant data. Production already has them
  via the live data-flip (the WS6 chunk-5 cutover); dev / test / e2e create their own
  contexts (or fixtures provide them).
- The `kb_topics` taxonomy (declaration / deformation / `temper.bootstrap`, …) is vestigial: a
  nullable `kb_events.topic_id` FK that **no substrate function references** (emit leaves it
  NULL). Don't seed it.

**Retired — substrate dropped the table/concept, no seed possible or needed:**

- **doc-types** → `kb_doc_types` is absent from the substrate; doc-types now live as temper-core
  schemas enumerated by `DocType::ALL` (session / decision / research / base / task / concept /
  goal). The vocabulary *evolved* from the legacy 7 (ticket / milestone / board / source are
  gone) — confirming there is nothing to carry over.
- **`kb_scopes`** (temper-events porosity, retired), **`kb_backend_selection`** (the flip dropped
  the selection flag).

### 3. schema-artifact deletion + fixture rehome

`schema-artifact/` is the root of temper-next's proving-ground test suite (~30 test files load
its `scenarios/*.yaml`, `seeds/*.yaml`, `payloads/`, `access-scenarios/`; `common/mod.rs` loads
the `.sql` files; `bootseed.rs` and the scenario model reference the YAML). Deleting it
preserves the suite by rehoming:

- Move `seeds/`, `scenarios/`, `payloads/`, `access-scenarios/` (YAML + JSON-Schema) into
  `crates/temper-next/tests/fixtures/` (their only consumers).
- Rewire the ~30 test files + `common/mod.rs` + `bootseed.rs` (`SYSTEM_SEED` /
  `system_event_type_names()`) + `Makefile.toml` paths.
- The proving-ground harness loads the **public baseline migration** under a
  `search_path = temper_next, public` wrapper into an isolated test namespace.
- **Retire** `schema_drift.rs` (no two-copy invariant remains). Reshape or retire
  `install_migration.rs` (the install-migration mechanism is gone).
- `schema-artifact/` directory **deletes entirely** once the migrations are settled.

The legacy SQL fixtures `03_seed.sql` / `04*.sql` that the read-path tests use migrate into
`crates/temper-next/tests/fixtures/` alongside the YAML, OR retire with the
`artifact-tests-legacy` read path if M2 has already retired it — confirm during implementation.

### 4. Dev-DB / harness / cache repoint

- `cargo make db-collapsed` → plain `public` load of the baseline. Drop the
  `?options=-csearch_path%3Dtemper_next,public` URL gymnastics everywhere it appears (dev-DB,
  test harness, CI).
- The e2e common harness loads `migrations/` into `public`. In-crate
  `#[sqlx::test(migrations = "../../migrations")]` **now Just Works** in `public` — resolving the
  deferred bare-`#[sqlx::test]` revert in `profile_service` tests (those reverted to empty-DB
  bare form during the flip because the migrations targeted the wrong namespace).
- Regen **temper-api** (`crates/temper-api/.sqlx`) and **e2e** (`tests/e2e/.sqlx`) caches against
  `public`. **temper-next's** cache stays `temper_next`-bound (its isolated test namespace).

### 5. Neon baseline-reset (the runbook's punted deliverable)

Runbook step 9 currently reads *"No `_sqlx_migrations` reconciliation needed… Restoring a
meaningful migrate path is the bootstrap-export spec's job."* This spec replaces that punt.

After the live promote (`ALTER SCHEMA temper_next RENAME TO public`), the live schema is
structurally artifact-faithful (built from the same artifact the baseline derives from), but its
`_sqlx_migrations` table still lists the 46 legacy rows. Reconcile by **mark-as-applied, not
replay**:

0. **Persistent backup gate (operator hard-stop, before ANY destructive change).** Before the
   schema rename or the `_sqlx_migrations` truncate, cut a **persistent, historically-preserved
   point-in-time backup** of the live Neon database — an explicitly retained snapshot/branch that
   is **not** subject to Neon's default branch/PITR expiry window. This is the durable rollback
   target (distinct from the ephemeral pre-cutover Neon branch the runbook already names). The
   plan models this as a runbook checkbox that **blocks** the destructive steps until confirmed;
   it is executed manually by the operator (or by the agent once `neonctl` is authenticated). The
   step must record the backup's identifier/restore command inline so rollback is one lookup, not
   a search.

1. **One-time structural safety check** — `pg_dump --schema-only` of live `public` vs. a fresh
   DB built from the new baseline; confirm equivalent. (By construction they match — both derive
   from `schema-artifact/01+02`; this is the verification, not a migration.)
2. **Mark-as-applied** — `TRUNCATE _sqlx_migrations;` then `INSERT` the 3 baseline rows with
   sqlx-computed checksums (version, description, checksum, `success=true`, installed_on,
   execution_time), so a subsequent `sqlx migrate run` against Neon is a clean no-op and the
   deployment is migration-aligned with the canonical set.

This appends to `docs/guides/ws6-endgame-collapse-runbook.md`, rewriting step 9. The runbook
remains the operator surface for the live cutover (the WS6 chunk-5 data flip is the live
data flip, distinct from this code/schema work).

## What stays the flip tail (out of this spec, redirected to `public`)

The committed WIP `f66cb9e` plus the remaining flip tail (the test surface — unit + e2e files
referencing deleted services; Tasks 9/10 = surface-parity gate + synthesis deletion) now target
`public` instead of `temper_next`. That is mostly a provisioning change handled by §4; the
test-surface rewrite and Tasks 9/10 are tracked by the flip plan
(`docs/superpowers/plans/2026-06-22-ws6-endgame-collapse-code.md`), not duplicated here.

## Sequencing

1. Author the 3 baseline files (schema / functions / seed) in `public` from the artifact + the
   §2 manifest. Delete the 46 legacy files.
2. Repoint dev-DB / harness / in-crate `#[sqlx::test]` to `public` (§4); regen temper-api + e2e
   caches.
3. Rehome the proving-ground fixtures; rewire the suite to load the baseline into a `temper_next`
   test namespace; retire `schema_drift.rs`; delete `schema-artifact/` (§3).
4. The test surface + Tasks 9/10 (flip-tail, redirected to `public`).
5. Append the Neon baseline-reset to the runbook (§5).
6. End-of-branch: workspace + e2e suites + `cargo make check` + consolidated code-review; PR.

## Risks / open items for the plan

- **Baseline ≡ promoted-live equivalence** is the load-bearing assumption behind the mark-as-
  applied reconciliation. The §5 `pg_dump` check is the safety net; the plan must make it a hard
  gate, not a courtesy.
- **`lens_create()` in a migration** needs the system actor (profile + entity) to exist first —
  the seed file must order the system-actor inserts before the lens calls. Verify the function's
  emitter resolution works from a plain migration context (no app-level scoping txn).
- **`install_migration.rs` / `schema_drift.rs` retirement** must not silently drop coverage the
  flip still relies on — confirm what each asserted before deleting.
- **Legacy read-path fixtures** (`03_seed.sql` / `04*.sql`): confirm whether the
  `artifact-tests-legacy` path is already retired (M2) before deciding rehome vs delete.
