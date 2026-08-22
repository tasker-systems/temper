# Data-Artifact Shape Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give a data-artifact family a declarable shape, so a later session learns whether the structured data it retrieved conforms to anything — without the registry ever becoming a gate on writing.

**Architecture:** A new `kb_data_artifact_shapes` table homed polymorphically over `(kb_contexts, kb_cogmaps)`, keyed per home so a shape never verdicts data its declarer cannot read. Conformance is validated in Rust (there is no in-database JSON Schema validator), synchronously at commit and asynchronously for the pre-existing backlog on the existing `kb_workflow_jobs` anchor queue. A verdict is a disposable read-model, trusted only while its `(shape_id, shape_version, content_hash)` triple still matches.

**Tech Stack:** Rust (sqlx, `jsonschema` 0.45, ts-rs, utoipa, schemars), PostgreSQL 18 local / 17 Neon, cargo-make.

**Spec:** `internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md`

---

## How to read the code blocks in this plan

This plan follows `implementation-grounding.md`, which forbids authoring invented code bodies into a
plan — *the sketch wins*: an implementer builds the code block rather than the correct prose beside
it, and an invented block is reliably stale on arrival.

So the code blocks here are one of two things, and they are labelled:

- **QUOTED** — real text read off disk at plan time, with a `file:line` citation. Trust it, but
  re-read the file: the plan may have aged.
- **CONTRACT** — the names, signatures and types a task must produce, plus a citation of the
  existing thing to copy. **Not** an implementation. Write the body from the cited incumbent, not
  from this plan.

Every task also carries a **CONFORM / EXTEND / AMEND** tag per step group:
CONFORM = honor an existing load-bearing constraint (cite the disk thing).
EXTEND = build beyond an existing affordance (cite the spec section).
AMEND = deliberately change an existing thing (cite both).

---

## Global Constraints

- **Migrations are immutable once applied, and must number above `main`.** The highest on `main` is
  `20260821000010_graph_entry_read.sql`. This plan uses `20260822000010` and `20260822000020`.
- **Every migration ends with `declare_migration(<number>, 'additive'|…, '<description>')`.** The
  description is a paragraph, not a line — see any existing migration.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.** All public types implement `Debug`. All
  MPSC channels bounded.
- **`--all-features` on every build and clippy invocation.**
- **Editing `migrations/` does not trigger a sqlx rebuild.** After adding a migration,
  `touch crates/temper-migrate/src/lib.rs` — **not** the crate under test, which only re-exports
  `MIGRATOR` (`crates/temper-substrate/src/lib.rs:39`). A stale migration set runs silently.
- **Never edit an already-applied migration**, even a comment: it trips the sqlx checksum and
  `db-migrate` fails. Unmerged ⇒ reset the Docker volume rather than renumbering.
- **SQL changes require a regenerated query cache.** `cargo make prepare-services` for
  `temper-services`. Deleting a query's last Rust caller orphans its `.sqlx` entry — remove it.
- **`cargo make check` is the gate.** The pre-commit hook is NOT equivalent — it misses
  `ts-rs-drift` and reads the worktree rather than the index. Run `cargo make check` before
  committing, and note that **ts-rs drift only clears after a COMMIT**, not after `git add`.
- **Commit all regenerated codegen together** — ts-rs bindings, `openapi.json`, the temper-rb gem,
  temper-ts `schema.ts`, and the `agent-skills/` projection ride along with the change that caused
  them.
- **Auth changes touch `temper-api` AND `temper-mcp`.** Never one alone.
- **Specs and plans live in `internal/superpowers/`**, never `docs/` — `docs/` is public.

### Vocabulary fixed by the spec — do not rename

| Concept | Term | Source |
|---|---|---|
| Enforcement mode | `advisory` (default) · `enforcing` | spec §6 |
| Shape state | `NeverDeclared` · `DeclaredSatisfied` · `DeclaredNotSatisfied` · `DeclaredNotYetChecked` | **already reserved** at `crates/temper-substrate/src/payloads.rs:652-657` |
| SQL shape-state literals | `never_declared` · `declared_satisfied` · `declared_not_satisfied` · `declared_not_yet_checked` | serde `snake_case` on `ShapeState` |
| CLI surface | `temper data-artifact schema {list,show,declare}` | spec §10 |

---

## File Structure

**Created**

| File | Responsibility |
|---|---|
| `migrations/20260822000010_data_artifact_shapes.sql` | Registry table, `shape_declared` event type, shape-in-force resolver, declare wrapper + projector |
| `migrations/20260822000020_data_artifact_verdicts.sql` | Verdict read-model table, verdict upsert fn, read-path rewrite to report real shape-state, registry enumeration reads |
| `crates/temper-substrate/tests/data_artifact_shapes.rs` | Witnesses for the registry and the verdict model |
| `crates/temper-services/src/services/shape_service.rs` | Shape CRUD + reconciliation driver (SQL lives here, never in handlers) |
| `crates/temper-core/src/types/data_artifact_shape.rs` | Wire types: `ShapeView`, `ShapeDeclareRequest`, `EnforcementMode` |
| `crates/temper-api/src/handlers/data_artifact_shapes.rs` | Thin handlers |
| `crates/temper-mcp/src/tools/data_artifact_shapes.rs` | MCP tool bodies |

**Modified** — all paths verified present at plan time

| File | Change |
|---|---|
| `crates/temper-substrate/src/payloads.rs:648-657` | `ShapeState` gains the three reserved variants; new `ShapeDeclared` payload |
| `crates/temper-substrate/src/events.rs:54,140,185,320,498,547` | `EventKind::ShapeDeclared`, `SeedAction::ShapeDeclare`, `Fired::Shape` |
| `crates/temper-substrate/src/events.rs:892-960` | `DataArtifactCommit` arm gains shape resolution + validation |
| `crates/temper-substrate/src/readback/mod.rs:2117` | `parse_shape_state` gains three arms |
| `crates/temper-substrate/src/writes.rs:1447-1493` | `declare_shape` / `declare_shape_with` beside `commit_data_artifact_with` |
| `crates/temper-substrate/src/replay.rs` | Replay arm for `shape_declared` |
| `crates/temper-core/src/types/workflow_job.rs:72-97` | `Persona::Shape`; `DispatchType::ShapeReconcile` |
| `crates/temper-services/src/backend/substrate_read.rs:1524-1640` | Registry reads alongside the artifact reads |
| `crates/temper-api/src/routes.rs:57-59` | Three new `.routes(routes!(…))` lines |
| `crates/temper-mcp/src/service.rs:486-512` | Three new `#[tool]` registrations |
| `crates/temper-cli/src/cli.rs:906` | `DataArtifactAction::Schema` nested subcommand |
| `crates/temper-cli/src/commands/data_artifact.rs` | Schema action dispatch |
| `crates/temper-client/src/data_artifacts.rs` | Client methods |
| `crates/temper-cli/templates/shared/data-artifacts.md` | Teach shapes |

---

## Beat A — the registry substrate

### Task 1: The registry table, event type, and declare act

**Tag: EXTEND** — authorized by spec §3, §4, §8. The predecessor spec left the registry unbuilt
(`2026-08-20-resource-owned-data-artifacts-design.md:140-144`).

**Files:**
- Create: `migrations/20260822000010_data_artifact_shapes.sql`
- Create: `crates/temper-substrate/tests/data_artifact_shapes.rs`
- Modify: `crates/temper-substrate/src/payloads.rs`, `events.rs`, `writes.rs`, `replay.rs`
- Modify: `crates/temper-substrate/Cargo.toml` (add `jsonschema`, already at `0.45` in
  `crates/temper-workflow/Cargo.toml:10` — use the same version)

**Interfaces:**
- Produces: `ShapeId`, `payloads::ShapeDeclared`, `payloads::EnforcementMode`,
  `EventKind::ShapeDeclared`, `SeedAction::ShapeDeclare`, `Fired::Shape(ShapeId)`,
  `writes::declare_shape_with(pool, DeclareShapeParams, EventContext) -> Result<ShapeId>`,
  SQL `data_artifact_shape_declare(p_payload, p_emitter, p_metadata, p_invocation, p_correlation)`
  and `_data_artifact_shape_in_force(p_resource, p_kind_owner_table, p_kind_owner_id, p_kind)`.

**CONTRACT — the table.** Copy the column-comment density and the assert/fold posture of
`kb_data_artifacts` (`migrations/20260820000020_data_artifacts.sql:14-45`).

```
kb_data_artifact_shapes
  id                    UUID PRIMARY KEY          -- no DEFAULT: identity-as-input, as kb_data_artifacts
  home_anchor_table     VARCHAR(64) NOT NULL CHECK (home_anchor_table IN ('kb_contexts','kb_cogmaps'))
  home_anchor_id        UUID NOT NULL
  kind_owner_table      VARCHAR(64) NOT NULL CHECK (kind_owner_table IN ('kb_profiles','kb_teams'))
  kind_owner_id         UUID NOT NULL
  artifact_kind         TEXT NOT NULL
  schema                JSONB NOT NULL
  enforcement           TEXT NOT NULL CHECK (enforcement IN ('advisory','enforcing'))
  shape_version         INT  NOT NULL
  asserted_by_event_id  UUID NOT NULL REFERENCES kb_events(id)
  last_event_id         UUID NOT NULL REFERENCES kb_events(id)
  is_folded             BOOLEAN NOT NULL DEFAULT false
  created               TIMESTAMPTZ NOT NULL DEFAULT now()
```

**The uniqueness index is the whole ruling and must be partial on `NOT is_folded`:**

```
UNIQUE (home_anchor_table, home_anchor_id, kind_owner_table, kind_owner_id, artifact_kind)
  WHERE NOT is_folded
```

> Note the deliberate contrast with `kb_data_artifacts`, which carries **no** such index and says so
> at `20260820000020_data_artifacts.sql:38-41`. Artifacts are a has-many collection; a shape in force
> is singular per family per home. Write that contrast into the migration comment — the next reader
> will otherwise assume the sibling table's rule applies.

**CONTRACT — `_data_artifact_shape_in_force`.** This function MUST call the two existing resolvers
rather than restating them:

- `_data_artifact_anchor(p_resource)` → the home. **QUOTED**, `20260820000020_data_artifact.sql`:
  its header says the cogmap tiebreak is "carried verbatim from `_property_owner_anchor` … and is
  load-bearing … Re-deriving it is how the two drift." The same applies here.
- `_data_artifact_kind_owner(p_resource)` → the default namespace, when the caller names none.

It returns at most one row: `(shape_id, shape_version, schema, enforcement)`.

- [ ] **Step 1: Write the failing witnesses**

Create `crates/temper-substrate/tests/data_artifact_shapes.rs`. Copy the harness conventions
verbatim from the sibling suite — `#![cfg(feature = "artifact-tests")]`, `mod common;`,
`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, and the `system_actor` /
`bootseed` fixtures at `crates/temper-substrate/tests/data_artifacts.rs:1-45`.

Four witnesses, each of which must **fail against current state for the right reason** (the table
does not exist yet):

| Test | Asserts |
|---|---|
| `a_shape_is_declared_and_found_in_force` | Declare for `(home, owner, kind)`; `_data_artifact_shape_in_force` returns it for a resource homed there |
| `a_shape_in_one_home_is_not_in_force_in_another` | **The ruling's bite probe.** Two contexts, same `(kind_owner, kind)`. Declaring in C₁ must leave a resource homed in C₂ reporting no shape in force. Remove `home_anchor_*` from the lookup and this test must fail |
| `a_cogmap_homed_resource_resolves_its_shape` | The polymorphic arm — a cogmap-homed resource finds a cogmap-homed shape |
| `declaring_twice_folds_the_prior_and_bumps_the_version` | Assert/fold; the folded row survives, `shape_version` is chain depth |

> **A witness that cannot fail is not a witness.** `a_shape_in_one_home_is_not_in_force_in_another`
> is the one that pins ruling 2 — write it so that widening the lookup to `(kind_owner, kind)` makes
> it go red, and verify that by actually widening the lookup once and watching it fail.

- [ ] **Step 2: Run them and confirm they fail**

```bash
cargo make test-artifacts 2>&1 | tail -40
```

Expected: compile failure or `relation "kb_data_artifact_shapes" does not exist`.

- [ ] **Step 3: Write the migration**

Create `migrations/20260822000010_data_artifact_shapes.sql` with the table, indexes, the
`shape_declared` event type registration, `_data_artifact_shape_in_force`,
`_project_data_artifact_shape_declared`, and `data_artifact_shape_declare`.

Copy the wrapper's four moves — validate, resolve anchor, append event, project — from
`data_artifact_commit` (`20260820000020_data_artifacts.sql`). The refusal for an unrecognized
enforcement term must **name the vocabulary**, exactly as the intent refusal does; that is the goal
clause `a-declined-act-teaches-its-vocabulary` and it is why the CHECK constraint alone is not
enough.

Register the event type with a **TYPED** `payload_schema` (the committed schemars snapshot), matching
how `data_artifact_committed` is registered, and spell `category` explicitly.

- [ ] **Step 4: Add the Rust types and the fire action**

`payloads.rs`: `ShapeDeclared`, `EnforcementMode`. `events.rs`: the `EventKind` variant plus its two
string arms, the `SeedAction::ShapeDeclare` variant, the `Fired::Shape` variant and its extractor.
`writes.rs`: `DeclareShapeParams` + `declare_shape` / `declare_shape_with`, mirroring
`CommitDataArtifactParams` at `writes.rs:1447-1493`. `replay.rs`: the replay arm.

- [ ] **Step 5: Regenerate the payload schema snapshot and migrate**

```bash
touch crates/temper-migrate/src/lib.rs
cargo make db-migrate
UPDATE_SCHEMA=1 cargo make test-schema
```

Paste the regenerated `data_artifact_shape_declared.v1.schema.json` into the migration's
`payload_schema` literal verbatim — repo == registry == Rust types. Hand-editing either half breaks
the chain silently.

- [ ] **Step 6: Run the witnesses green**

```bash
cargo make test-artifacts 2>&1 | tail -40
```

- [ ] **Step 7: Prove the bite**

Temporarily drop `home_anchor_table`/`home_anchor_id` from the `_data_artifact_shape_in_force`
lookup. Re-run. `a_shape_in_one_home_is_not_in_force_in_another` must go red. **Restore from a file
copy — never `git checkout`** to undo a probe edit.

- [ ] **Step 8: Commit**

```bash
cargo make check
git add migrations/20260822000010_data_artifact_shapes.sql crates/temper-substrate/
git commit -m "feat(substrate): data-artifact shape registry — table, declare act, shape-in-force resolver"
```

---

### Task 2: `ShapeState` gains its three reserved variants

**Tag: CONFORM** — the names are already reserved at
`crates/temper-substrate/src/payloads.rs:652-657`. Do not invent alternatives.

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs:648-657`
- Modify: `crates/temper-substrate/src/readback/mod.rs:2117-2123`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:1567,1641`

**Interfaces:**
- Produces: a four-variant `ShapeState`; `parse_shape_state` accepting four literals.

**QUOTED** — `crates/temper-substrate/src/readback/mod.rs:2117-2123`, the decoder to extend:

```rust
fn parse_shape_state(s: &str) -> Result<crate::payloads::ShapeState> {
    Ok(match s {
        "never_declared" => crate::payloads::ShapeState::NeverDeclared,
        other => anyhow::bail!("unrecognized shape_state from database: {other}"),
    })
}
```

- [ ] **Step 1: Write the failing test**

In `crates/temper-substrate/tests/data_artifact_shapes.rs`, a test that `parse_shape_state` round-trips
all four literals and that an unknown literal still returns `Err` — the `bail!` default is
load-bearing and must survive. Its doc comment already says why: "a `""` or `NULL` is a decode error,
not a silent 'looks fine.'"

- [ ] **Step 2: Run it, confirm it fails**

```bash
cargo nextest run -p temper-substrate --features artifact-tests parse_shape_state 2>&1 | tail -20
```

- [ ] **Step 3: Add the variants and the arms**

Uncomment the three reserved variants (keeping their doc comments, which already state each one's
meaning), add three `match` arms to `parse_shape_state`, and extend the two exhaustive matches in
`substrate_read.rs:1567,1641`.

- [ ] **Step 4: Run green, then commit**

```bash
cargo make test-artifacts && cargo make check
git commit -am "feat(substrate): ShapeState gains DeclaredSatisfied/NotSatisfied/NotYetChecked"
```

---

## Beat B — commit-time verdict

### Task 3: Validate at commit, refuse only when enforcing

**Tag: CONFORM** to the register's synchronous Then (spec §7.1); **EXTEND** for the enforcement
branch (spec §6).

**Files:**
- Modify: `crates/temper-substrate/src/events.rs:892-960` (the `DataArtifactCommit` arm)
- Modify: `crates/temper-substrate/tests/data_artifact_shapes.rs`

**Interfaces:**
- Consumes: `_data_artifact_shape_in_force` (Task 1), `EnforcementMode` (Task 1).
- Produces: a commit that carries a verdict; a refusal carrying the validation failure.

**The ordering wrinkle, and why it is not optional.** The commit arm currently computes the hash and
calls SQL, and **when the caller omits `kind_owner` the namespace is resolved by the SQL wrapper, not
by Rust** — this is deliberate. **QUOTED**, `crates/temper-substrate/src/events.rs:908-919`:

```rust
// A placeholder is NOT written here. When the caller names no namespace the field
// is omitted from the JSON entirely, and the SQL wrapper fills it in before
// appending the event — so the stored payload always carries a resolved namespace
// and replay never has to re-derive one.
```

So Rust cannot know the family before the wrapper runs. **Do not re-derive the namespace in Rust** —
that restates `_data_artifact_kind_owner` and is exactly the drift `plan-verification.md` warns
about. Instead, within the same transaction, call `_data_artifact_shape_in_force` first (it resolves
home and namespace through the incumbent resolvers), validate in Rust, then call
`data_artifact_commit` with the verdict.

- [ ] **Step 1: Write the failing witnesses**

| Test | Asserts |
|---|---|
| `a_conforming_commit_records_declared_satisfied` | Verdict is `DeclaredSatisfied` synchronously |
| `an_advisory_shape_records_non_conformance_without_refusing` | **The posture's bite probe.** Commit succeeds, artifact is retrievable whole, verdict is `DeclaredNotSatisfied`. If this ever refuses, ruling 4 has been lost |
| `an_enforcing_shape_refuses_and_says_what_failed` | Refusal carries the validation error, and **no artifact row is written** |
| `a_commit_with_no_shape_in_force_stays_never_declared` | `persistence-never-requires-a-prior-declaration` still holds |

- [ ] **Step 2: Run, confirm failure**

```bash
cargo make test-artifacts 2>&1 | tail -40
```

- [ ] **Step 3: Add `jsonschema` to temper-substrate and implement**

Match the workspace version — `jsonschema = "0.45"` (`crates/temper-workflow/Cargo.toml:10`).

- [ ] **Step 4: Green, then commit**

```bash
cargo make test-artifacts && cargo make check
git commit -am "feat(substrate): commit-time conformance verdict, refusing only under an enforcing shape"
```

---

## Beat C — the verdict read-model and the read path

### Task 4: Verdict table and the staleness triple

**Tag: EXTEND** (spec §7.4, §7.5). Note §12: this ruling has **no incumbent** — the comparable
substrate tables are event-backed. Build it as specified and do not go looking for a pattern to
copy that the spec has already said is absent.

**Files:**
- Create: `migrations/20260822000020_data_artifact_verdicts.sql`
- Modify: `crates/temper-substrate/tests/data_artifact_shapes.rs`

**CONTRACT — the table.** Not event-sourced. Rebuildable from artifacts + shapes at any time.

```
kb_data_artifact_verdicts
  artifact_id    UUID PRIMARY KEY REFERENCES kb_data_artifacts(id) ON DELETE CASCADE
  shape_id       UUID NOT NULL REFERENCES kb_data_artifact_shapes(id) ON DELETE CASCADE
  shape_version  INT  NOT NULL
  content_hash   TEXT NOT NULL     -- the artifact hash this verdict was computed over
  satisfied      BOOLEAN NOT NULL
  detail         JSONB             -- what failed, when not satisfied
  checked_at     TIMESTAMPTZ NOT NULL DEFAULT now()
```

**CONTRACT — the read-side shape-state expression.** A stored verdict is honored **only** when all
three of `shape_id`, `shape_version` and `content_hash` still match the currently-governing shape and
the artifact's current hash. Otherwise the artifact reports `declared_not_yet_checked`. Where no
shape is in force at all, `never_declared`.

> This is what makes `unchecked-never-reads-as-checked` hold **by construction rather than by a
> worker running on time**, and it is the single most important line in this beat. A verdict row
> that merely exists must never be sufficient.

- [ ] **Step 1: Write the failing witnesses**

| Test | Asserts |
|---|---|
| `a_stale_verdict_reads_as_not_yet_checked` | Fold the shape and re-declare (version bumps); the old verdict row still exists but the artifact reports `DeclaredNotYetChecked` |
| `rehoming_a_resource_invalidates_its_verdicts` | `resource_rehome` to a context with a different shape; artifacts report `DeclaredNotYetChecked` without any verdict row being deleted |
| `an_artifact_with_no_shape_reports_never_declared` | Regression guard on the existing behaviour |

- [ ] **Step 2: Run, confirm failure.** `cargo make test-artifacts`

- [ ] **Step 3: Write the migration**

Table, plus a **`CREATE OR REPLACE`** of the four read functions from
`migrations/20260820000030_data_artifact_reads.sql` so their hardcoded `'never_declared'::text`
literals (lines 79 and 154) become the real expression.

> **AMEND, and read the constraint before writing.** Those four functions currently return
> `shape_state text` in a fixed column order. Changing a function's **return shape** is not additive
> and cannot be a bare `CREATE OR REPLACE` — Postgres refuses. If the column list changes at all you
> must `DROP FUNCTION` first, which makes the migration non-additive and means the deployed binary
> and the schema disagree across the apply window. **Keep the return shape byte-identical** and only
> change the expression behind `shape_state`; that keeps it additive. If a new column is genuinely
> needed, that is a separate, deliberately non-additive migration with its own rollout note.

- [ ] **Step 4: Migrate, run green**

```bash
touch crates/temper-migrate/src/lib.rs
cargo make db-migrate
cargo make test-artifacts 2>&1 | tail -40
```

- [ ] **Step 5: Prove the bite**

Drop `content_hash` from the triple check. `a_stale_verdict_reads_as_not_yet_checked` must go red.
Restore from a file copy.

- [ ] **Step 6: Commit**

```bash
cargo make prepare-services && cargo make check
git add migrations/20260822000020_data_artifact_verdicts.sql crates/
git commit -m "feat(substrate): verdict read-model with the staleness triple; reads report real shape state"
```

---

### Task 5: Registry enumeration reads, visibility-gated

**Tag: CONFORM** — the gate is `resources_visible_to`'s sibling for containers. Every read in this
codebase goes through the one visibility spine.

**Files:**
- Modify: `migrations/20260822000020_data_artifact_verdicts.sql` (same migration, if not yet applied)
  **or** a new `20260822000030` if `20260822000020` has already been applied anywhere.
- Modify: `crates/temper-substrate/src/readback/mod.rs`

**Interfaces:**
- Produces: SQL `shapes_for_home(p_profile, p_anchor_table, p_anchor_id)` and
  `shape_by_id(p_profile, p_shape_id)`; Rust readback twins.

> **CONFORM warning, from the sibling migration's own header** (`20260820000030:20-27`): the gate
> must be an **INNER JOIN**, not an `array_agg` into a NULL-means-unbounded predicate. An empty
> visible set must produce zero rows. If a future change collects ids into an array, `COALESCE` the
> aggregate to `ARRAY[]::uuid[]` or the gate falls open.

- [ ] **Step 1: Write the failing witness**

`shape_reads_gate_on_home_visibility` — a principal who cannot read the home context sees zero
shapes, across both read functions.

- [ ] **Step 2–4: Fail → implement → green**, then prove the bite by removing the JOIN and watching
  the test go red. Restore from a file copy.

- [ ] **Step 5: Commit**

```bash
cargo make prepare-services && cargo make check
git commit -am "feat(substrate): visibility-gated registry enumeration reads"
```

---

## Beat D — reconciliation

### Task 6: The reconciler persona and the backfill sweep

**Tag: CONFORM** — rides the existing anchor-scoped queue. **No new queue infrastructure.**

**Files:**
- Modify: `crates/temper-core/src/types/workflow_job.rs:72-97,229-234`
- Create: `crates/temper-services/src/services/shape_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: `workflow_job_service::{enqueue_anchor, claim_anchor, complete_anchor}`
  (`crates/temper-services/src/services/workflow_job_service.rs:291,326,381`), `HomeAnchor`
  (`crates/temper-core/src/types/home.rs:13-16`).
- Produces: `Persona::Shape`, `DispatchType::ShapeReconcile`,
  `shape_service::reconcile_anchor(pool, anchor) -> ApiResult<usize>`.

**QUOTED** — `crates/temper-core/src/types/workflow_job.rs:76-84`, the reason a **new persona** is
required rather than a new dispatch type on an existing one:

```rust
    /// The citation auditor (Set 5). A DISTINCT persona value, not a steward dispatch_type: the
    /// single-flight index is `(cogmap_id, persona, dispatch_type)`
    /// (`migrations/20260705000001_workflow_jobs.sql:43-45`), so a separate persona is what lets an
    /// auditor job and a steward job be in flight over the same cogmap at once.
```

A shape-reconcile job must be able to be in flight over a context that already has a region job. So:
a distinct `Persona::Shape`. The enum's own doc notes a new variant is "a code change, never a
migration — the column is `text`."

**Note on the payload type.** `enqueue_anchor` currently takes `RegionJobPayload`
(`workflow_job_service.rs:291-296`, struct at `workflow_job.rs:230-234`, a single `emitter: Uuid`).
Reconciliation needs the same single field. **Decide at implementation time and say which you did:**
either reuse `RegionJobPayload` (and rename it to something anchor-generic in the same commit, since
a shape job carrying a type called `Region` is a lie), or add a sibling payload type. Do not silently
pass a `RegionJobPayload` for a shape job.

- [ ] **Step 1: Write the failing witnesses**

| Test | Asserts |
|---|---|
| `declaring_a_shape_enqueues_one_reconcile_job` | One job, anchored to the shape's home |
| `declaring_twice_collapses_to_one_in_flight_job` | The single-flight index does its job — the second enqueue returns `None`, not an error |
| `reconciliation_verdicts_the_pre_existing_backlog` | Artifacts committed **before** the declaration move from `DeclaredNotYetChecked` to a real verdict |
| `a_reconcile_tick_with_no_work_touches_nothing` | **Guard against a hollow green.** A tick that claims zero jobs must not be mistakable for a successful sweep — assert on verdicts written, not on the tick returning `Ok` |

> The last one exists because a `claimed 0 job(s)` tick has previously been mistaken for evidence
> that a worker path runs. It tests nothing unless it asserts on the work product.

- [ ] **Step 2: Run, confirm failure.** `cargo make test-db`

- [ ] **Step 3: Implement.** Enqueue on declare, on amend, and on `resource_rehome`. All SQL in
  `shape_service`, never in a handler or tool.

- [ ] **Step 4: Green, then commit**

```bash
cargo make test-db && cargo make prepare-services && cargo make check
git commit -am "feat(services): shape reconciliation on the anchor-scoped job queue"
```

---

## Beat E — surfaces

> Beats A–D produce a working, testable registry reachable from Rust. Beat E is parity work: the
> shape of every task below is **entirely determined** by the types Beats A–D produce, and each has
> an exact existing analogue to mirror. Read the analogue, not this plan, for the body.

### Task 7: Wire types and the service layer

**Tag: CONFORM** to the existing artifact wire types.

**Files:**
- Create: `crates/temper-core/src/types/data_artifact_shape.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:1524-1640`

**The derive stack is not optional and is easy to get wrong.** **QUOTED**,
`crates/temper-core/src/types/data_artifact.rs:15-20`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
```

Three traps, all previously paid for:
- **ts-rs `export_to` must be unique across crates.** Two crates writing one filename means last
  writer wins, types vanish, exit code 0.
- **ts-rs drops the WHOLE serde attribute if any part is unsupported.** `rename` and
  `skip_serializing_if` in one attribute silently loses the rename. Split them.
- **MCP enum params need `schemars(inline)`.** `EnforcementMode` is an enum on a tool input.

- [ ] **Step 1–4:** Test → fail → implement → green.
- [ ] **Step 5: Regenerate and commit codegen together**

```bash
cargo make generate-ts-types && cargo make openapi && cargo make openapi-rb && cargo make openapi-ts
cargo make check
git add -A && git commit -m "feat(core): shape registry wire types + service layer"
```

> ts-rs drift only clears **after a commit**, not after `git add`. If `cargo make check` still
> reports `ts-rs-drift` on a clean-looking tree, commit and re-run.

### Task 8: API handlers and routes

**Tag: CONFORM.** Thin handlers; auth middleware shared, never per-handler; authorization **before**
any write.

**Files:**
- Create: `crates/temper-api/src/handlers/data_artifact_shapes.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`, `crates/temper-api/src/routes.rs:57-59`

Mirror `crates/temper-api/src/handlers/data_artifacts.rs` exactly.

> **utoipa traps:** modifiers see an EMPTY paths set; `.routes()` skips `IntoParams`. Check
> `cargo make openapi-check` and `openapi-routes-check`, not just that it compiles.

- [ ] Steps 1–5: test → fail → implement → green → `cargo make check` → commit.

### Task 9: MCP tools

**Tag: CONFORM.** An auth change touches `temper-api` **and** `temper-mcp` — this task is the second
half of Task 8, not an optional follow-up.

**Files:**
- Create: `crates/temper-mcp/src/tools/data_artifact_shapes.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`, `crates/temper-mcp/src/service.rs:486-512`

Mirror the three existing `#[tool]` registrations. Their descriptions state the visibility posture
explicitly ("Visibility-gated: you only see artifacts whose owning resource you can read") — say the
equivalent for shapes, and say that declaring is gated on authoring the home.

- [ ] Steps 1–5 as above.

### Task 10: The CLI `schema` subgroup

**Tag: CONFORM** to the nested-subcommand pattern; **EXTEND** per spec §10.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:906` — add `DataArtifactAction::Schema { #[command(subcommand)] action: SchemaAction }`
- Modify: `crates/temper-cli/src/commands/data_artifact.rs`
- Modify: `crates/temper-client/src/data_artifacts.rs`

Commands, exactly as spec §10 fixes them:

```
temper data-artifact schema list --context <ref>
temper data-artifact schema show <ref>
temper data-artifact schema declare <ref> --kind <k>
```

Thin commands: parse args, call actions, format output. Business logic in `src/actions/`. Use the
existing runtime wrapper, never a raw `Runtime::new()`.

- [ ] Steps 1–5 as above, plus:

```bash
cargo make cli-reference-drift
cargo install --path crates/temper-cli   # the PATH binary is stale until you do
temper data-artifact schema --help       # verify against the real binary, not the source
```

---

## Beat F — teaching and the register

### Task 11: Teach shapes in the skill projection and docs

**Tag: CONFORM** to the shared-template pattern shipped in PR #751.

**Files:**
- Modify: `crates/temper-cli/templates/shared/data-artifacts.md`
- Modify: `docs/playbooks/commit-structured-data-as-an-artifact.md`

The template already teaches shape state as a concept. Extend it with: how to declare a shape, that
declaring is **never required** to commit, that `advisory` is the default and what `enforcing`
changes, and that a shape governs **its home**, so the same family can carry different shapes in
different contexts.

> **Keep the why-anchor.** The trap this template exists to prevent is teaching data artifacts as
> "just another way to store JSON." Shapes must not become "just schema validation" either — the
> anchor is still that the writer and the reader are different sessions separated in time.

- [ ] **Step 1: Update the template and the playbook.**
- [ ] **Step 2: Regenerate both projections**

```bash
temper skill emit && temper skill install --target claude
cargo make skills-drift && cargo make docs-coverage
```

- [ ] **Step 3: Commit** (the regenerated `agent-skills/` projection rides along).

> Also outstanding from the previous session, cheap to fix while here: the playbook is an orphan,
> not linked from a door page. Link it from the relevant door in `docs/doors/`.

### Task 12: Close the register on real evidence

**Tag: CONFORM** to outcome discipline. **Coverage is never inferred from absence.**

**Files:**
- Modify: goal `01a02163-ba6a-7b00-91f5-5f416e43f4f6` (the register body)

- [ ] **Step 1: Run the full suite and capture what actually passed**

```bash
cargo make test-all-rust 2>&1 | tee /tmp/shape-registry-suite.log; tail -40 /tmp/shape-registry-suite.log
```

> `test-all` is **red by default** — there is a known pre-existing streaming/embed timeout. Confirm
> any failure is that one before treating the run as clean. And nextest cancels on first failure, so
> "1 failed" is a **lower bound**, never a count.

- [ ] **Step 2: Update the two clause rows with the witnesses that actually exist**

`unchecked-never-reads-as-checked` moves from **partial** to covered **only if** a witness proves the
stale-verdict path (`a_stale_verdict_reads_as_not_yet_checked`) and it demonstrably bites.
`declaring-a-shape-never-destroys-what-came-before` moves from **declared-uncovered** only if a
witness proves a pre-existing non-conforming artifact survives declaration intact and retrievable.

If either witness does not exist or does not bite, **say so in the register** rather than leaving the
row reading clean. A retired check leaves a named remainder, not a gap.

- [ ] **Step 3: Update Exercise status honestly.** Tests passing is not the trigger firing. The
  trigger is an agent session declaring a shape in real work and a later session reading a verdict
  it did not write. Until that happens, say it has not.

- [ ] **Step 4: Write the register back**

```bash
temper resource show <goal-ref> --format json | jq -r .content > register.md
# edit, then re-read and diff before writing — a body write replaces the WHOLE body
cat register.md | temper resource update <goal-ref>
```

- [ ] **Step 5: `cargo make register-coverage-drift`**, then commit.

---

## Self-Review

**Spec coverage.** Every spec section maps to a task: §3 home → Task 1; §4 key → Task 1 (the partial
unique index and its bite probe); §5 authority → Tasks 5, 8, 9, 10 (the gate is applied at each read
and write surface); §6 enforcement vocabulary → Tasks 1 and 3; §7.1–7.2 commit-time Rust validation →
Task 3; §7.3 async reconciliation → Task 6; §7.4 verdict read-model → Task 4; §7.5 staleness triple →
Task 4; §7.6 ShapeState names → Task 2; §8 assert/fold → Task 1; §9 register amendments → **already
applied** to the goal body this session, so no task; §10 findability and CLI → Tasks 5 and 10.

**One gap I am naming rather than papering over.** Spec §5 rules that authority is
`context_authorable_by_profile` / `cogmap_authorable_by_profile`, but no task in Beats A–D applies
that gate — the substrate `declare` act is reachable from tests without it, and the gate first
appears at the service/API layer in Beat E. That is the same layering the artifact commit path
already uses, so it is consistent rather than novel. **But it means Beats A–D ship a write path with
no authorization gate**, and if Beat E slipped, that would be a hole. Task 6 (`shape_service`) is the
right place to add it, and it should not wait for Beat E.

**Placeholders.** None remaining. Every step names real files, real commands verified against
`tools/cargo-make/*.toml`, and real symbols verified on disk at plan time.

**Type consistency.** `ShapeState` variants match `payloads.rs:652-657` exactly. `HomeAnchor` is
consumed, never re-derived. `Persona::Shape` / `DispatchType::ShapeReconcile` are used consistently
in Task 6. The registry table name `kb_data_artifact_shapes` and verdict table
`kb_data_artifact_verdicts` are used identically in Tasks 1, 4, 5 and 6.
