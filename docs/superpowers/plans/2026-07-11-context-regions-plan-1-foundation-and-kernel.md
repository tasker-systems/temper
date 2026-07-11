# Context Regions — Plan 1: Foundation and Kernel (T1–T4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [`docs/superpowers/specs/2026-07-11-context-regions-and-wayfinding-design.md`](../specs/2026-07-11-context-regions-and-wayfinding-design.md)
**Temper goal:** `019f512a-024b-7461-9403-24ac02044665` (tasks T1–T9)
**This plan covers:** T1, T2, T3, T4.

**Goal:** Make the region producer anchor-agnostic and give it an embedding-primary affinity kernel, so that regions can form over a *context* (1,643 resources, zero facets, near-monotone edge graph) and not just a curated cognitive map.

**Architecture:** One producer, two regimes. The `Lens` row is the switch. `affinity()` gains a third term — a **sparse exact-kNN cosine** — whose weight `w_cos` is `0.0` for cogmaps (reproducing today's behavior bit-for-bit) and `1.0` for contexts. Everything keyed on `cogmap_id` widens to a polymorphic `(home_anchor_table, home_anchor_id)` pair, reusing the existing `temper_core::types::home::HomeAnchor` enum. Schema lands additively; `main` stays auto-deployable.

**Tech Stack:** Rust (temper-substrate, temper-core), PostgreSQL 18 local / 17 Neon with pgvector, sqlx (compile-time-checked macros + offline cache), cargo-nextest, cargo-make.

## Global Constraints

Copied verbatim from CLAUDE.md and the spec. **Every task's requirements implicitly include this section.**

- **Migrations use `uuid_generate_v7()`, never native `uuidv7()`.** Native passes PG18 (dev/CI) and **breaks Neon PG17 (prod)**. This has bitten before (PR #270).
- **Never edit a shipped migration.** They are sqlx checksum-locked — even a comment change breaks `sqlx migrate run`. Always add a new migration.
- **Additive-only on `main`.** Every commit's schema must work against the *previous* commit's code, because `main` auto-deploys to live Vercel projects. No `DROP`, no `RENAME`, no `NOT NULL` on a new column without a default.
- **SQL lives in the persistence layer**, never inline in a surface. Substrate write/readback core is `temper-substrate/src/{writes,readback}`; service logic is `temper-services/src/services/`.
- **After changing any SQL, regenerate the sqlx caches, in this order:**
  `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-api`.
  Run from the repo root **with a cold build cache** — a warm cache writes a near-empty `.sqlx` over the good one.
- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known structure.
- **Params structs** for functions with more than 5 domain-related parameters. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **`cargo make check` must pass before any commit.** It is the honest offline probe of the committed sqlx caches (it forces `SQLX_OFFLINE=true`).
- **Every `#[sqlx::test]` file needs `#![cfg(feature = "test-db")]`** (or `artifact-tests` in temper-substrate) at the top, or the unit-test CI job fails to compile.
- **Determinism is a hard requirement.** Region formation must be reproducible: same corpus + same lens → same `membership_fingerprint`. This forbids ANN/HNSW anywhere in the formation path.
- **Print the object before you widen it — this plan's reconstructions are not trustworthy.** T1's execution found that every SQL body, column name, and comment this plan quotes came from an architecture sweep, and three of them were wrong: a "gap" that a later migration had already closed, a function reconstructed as plpgsql when it is `LANGUAGE sql`, and a column (`granted_by_event_id`) that does not exist. T2–T4 come from the same sweep. Before editing any function or writing any fixture: `cargo sqlx migrate run` (a dev DB one migration behind lies to you *and* breaks the `sqlx::query!` macros), then `psql "$DATABASE_URL" -c "\sf <fn>" -c "\d <table>"`. Widen the **printed** body.

## The One Test That Governs Everything

> **With `w_cos = 0.0`, every existing cogmap scenario fixture must produce identical region membership and an identical `membership_fingerprint`.**

This is the regression floor for the entire arc. It is asserted in Task 3 Step 2 and re-asserted in Task 4 Step 8. If it ever goes red, stop — do not "fix" the fixture.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `migrations/20260712000010_context_read_predicates.sql` | **Create.** `contexts_readable_by` as THE context read-set (fixing the flat team-owned arm); the five copies route through it; `context_authorable_by_profile` narrowed to direct-membership + role; `'context'` principal kind in `resources_readable_by`. | T1 |
| `crates/temper-services/tests/context_read_predicate_test.rs` | **Create.** Integration tests for the above. | T1 |
| `migrations/20260712000030_region_anchor_expand.sql` | **Create.** M1 additive schema: anchor pair on the 4 region tables, new lens columns, `kb_contexts` shape columns, transitional `COMMENT`s. | T2 |
| `crates/temper-core/src/types/home.rs` | **Modify.** Add `HomeAnchor::{table, uuid, from_parts}` — the SQL-binding helpers the producer needs. | T3 |
| `crates/temper-substrate/src/substrate.rs` | **Modify.** `load()` takes `HomeAnchor`; three hard-wired `'kb_cogmaps'` predicates widen; lens resolution becomes anchor-aware. | T3 |
| `crates/temper-substrate/src/write.rs` | **Modify.** `materialize`/`incremental_materialize` take `HomeAnchor`; fold/create/assert widen; **persist member `affinity`**. | T3 |
| `crates/temper-substrate/src/replay.rs` | **Modify.** `formation_touched_count_since` / `content_touched_resources_since` widen off `'kb_cogmaps'`. | T3 |
| `migrations/20260712000040_region_anchor_functions.sql` | **Create.** `cogmap_region_centrality` home-filter fix; `region_materialize` + `_project_region_materialized` accept an anchor; `region_materialized` event schema widens. | T3 |
| `crates/temper-substrate/src/affinity.rs` | **Modify.** `Lens` gains `w_cos`/`knn_k`/`cos_floor`; `affinity()` gains the `knn_sim` term; add `Lens::workflow_default()`. | T4 |
| `crates/temper-substrate/src/knn.rs` | **Create.** The sparse exact-kNN builder. One responsibility: pooled embeddings → a symmetric sparse neighbour map. | T4 |
| `migrations/20260712000050_workflow_default_lens.sql` | **Create.** Seed the `workflow-default` context lens. | T4 |

---

## Task 1: Context access predicates (authz prerequisites) — ✅ LANDED

Spec §3.8. **No dependencies.** Blocks T8.

> **Rewritten 2026-07-11 mid-execution.** The original was built on a stale premise and contained a
> test that could never fail. Both are recorded below, because the *reason* they were wrong applies
> directly to T2–T4, which came out of the same architecture sweep.

### What the original got wrong

1. **"The context arm of `anchor_readable_by_profile` ignores `kb_access_grants` entirely."** False.
   True on 2026-06-30, fixed on 2026-07-01 by `20260701000004_anchor_readable_context_grant.sql` —
   the migration that quotes the very header the spec cited.
2. **The `resources_readable_by` test asserted `count == 0` with the comment "today it raises."** It
   does not raise: it is `LANGUAGE sql`, a `UNION` with `WHERE p_principal_kind = …` guards, so an
   unknown kind silently returns zero rows. The test passed against the unmigrated schema.
3. **The fixture used `kb_access_grants.granted_by_event_id`.** No such column — it is
   `granted_by_profile_id`. (Likewise `kb_edges` has `edge_kind`, an enum, not `edge_type`;
   `relates_to` is a `label`.)

**Every one of these came from trusting a reconstruction over the running database.** See the new
Global Constraint: *print the object before you widen it.*

### The model (nowhere written down, which is how it rotted)

The team DAG is an org enclosure hierarchy — `EPD ▸ engineering ▸ payroll-group ▸ squad-two`.
Membership is transitive upward. Two axes, and they are **not** the same axis:

- **READ inherits UP the chain**, never sideways. A squad-two member reads engineering's and EPD's
  contexts; `security-it-ops` stays invisible.
- **WRITE requires DIRECT membership** in the owning team, with an authoring role
  (`owner`/`maintainer`/`member`; `watcher` is read-only).

### The two defects verification actually found

**Read was too narrow.** The context-read rule was written out **five times** — `context_visible_to`,
`resources_visible_to` (branch 5), `edges_visible_to`, `graph_home_contexts`,
`resources_in_team_scope` — and every copy gated the team-**owned** arm on *direct* membership. A
squad-two member could read a context *shared to* engineering but not the one engineering *owns*.
The copies had already drifted: `graph_home_contexts` had gone flat on the **share** arm too, and its
`candidates` CTE calls itself "a proven superset (same branches)" of `context_visible_to` — true only
while both were equally wrong. Widening the predicate alone would have made it a **subset** and
dropped contexts out of the graph view.

**Write was too wide.** `context_authorable_by_profile` ancestor-expanded, producing a
write-wider-than-read inversion on the same object: a squad-two member could **author into**
engineering's context while unable to **read** it. And **0 of 15** access predicates consulted
`kb_team_members.role`, so a `watcher` could author.

### What shipped

`migrations/20260712000010_context_read_predicates.sql`:

- [x] **`contexts_readable_by(p_profile) → SETOF context_id`** — THE context read-set. Four arms:
      personal; owned by an enclosing team (**the fix**); shared to an enclosing team; explicit
      read-grant.
- [x] **`context_readable_by_profile`** is its boolean grain; **`context_visible_to`** and
      **`anchor_readable_by_profile`**'s `kb_contexts` arm delegate to that.
- [x] **`resources_visible_to`, `edges_visible_to`, `graph_home_contexts`, `resources_in_team_scope`**
      all route their context arms through the read-set. Five copies → one.
- [x] **`context_authorable_by_profile` NARROWED** — direct membership + authoring role. The only
      non-additive change in the arc; taken now because the deployment is a handful of alpha testers.
      Explicit write-**grants** still reach through `team_ancestors` (delegation is deliberate).
- [x] **`resources_readable_by` gains the `'context'` kind** — the context's own interior, under the
      same soft-delete floor as `resources_accessible_to_cogmap`.

`crates/temper-services/tests/context_read_predicate_test.rs` — 10 tests, all on one EPD-shaped
fixture: read inherits up; read never flows sideways or down; the pre-existing branches survive;
write does *not* inherit up; a watcher cannot author; an explicit write-grant still reaches; the
`'context'` kind returns the interior (asserted non-empty); other kinds unchanged; the graph view
lists what the profile can read; edges homed in an enclosing team's context are visible.

### Gotcha that cost a cycle

`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` embeds the migration set at **compile time**.
A new migration file does not bust the crate's build cache — tests fail with a phantom "function does
not exist" until `cargo clean -p temper-substrate`.

### Verification

```bash
cargo nextest run -p temper-services --features test-db --test context_read_predicate_test  # 10 passed
cargo make test-db && cargo make test-e2e
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api
cargo make check
```

Unlike the original task, `test-db`/`test-e2e` are **not** a pure widen-only check any more: the write
narrowing legitimately revokes privilege, so a test that newly denies a *write* may be correct. A test
that newly denies a *read* is still a dropped branch.

### Adversarial review + the one leak it found

Because this task reworks the access substrate and revises a latent bug, a scenario-based adversarial
security review ran over six heterogeneous team-DAG shapes (multi-parent/diamond, degenerate topology,
lifecycle/soft-delete, roles/principals, grant interactions, container/interior/edges), each **probed
empirically against the live DB** in rolled-back transactions, each finding refuted under three
independent lenses. Two findings were refuted (a hoped-for multi-parent sideways leak — `team_ancestors`'
`UNION` dedups and multi-parent reach is intended; and cycle non-termination — the recursive CTE
terminates). **One survived:** `can_modify_resource` had no soft-delete write floor, a real leak that
*committed* via the body-only PATCH path (see spec §3.9 item 3). Fixed in
`20260712000020_can_modify_active_floor.sql`, bundled into the same PR (#378) as the same write-authz
surface. Tests: `a_tombstone_is_unmodifiable_on_every_arm` + `the_write_floor_does_not_touch_live_resources`
(predicate grain) and `body_only_update_of_a_deleted_resource_is_rejected` (surface grain, in
`tests/e2e/tests/resource_crud_test.rs` — the body-only shape is what actually exercises the hole).

The method is the takeaway: every real defect in this arc — the stale-header gap, the six flat copies,
and this leak — was found by *running the predicate*, never by reading it. Adversarial + empirical is
the pattern for authz work here.

---

## Task 2: M1 additive schema — polymorphic anchor + new lens columns — ✅ LANDED

Spec §3.6. **No dependencies.**

> **Corrected 2026-07-11 during execution**, after printing every object against the live DB.
> Three defects in what this task originally said — recorded because T3/T4 came from the same sweep:
>
> 1. **The filename collided.** `20260712000020` was taken by `can_modify_active_floor.sql` — the
>    soft-delete write-floor fix the T1 adversarial review found, merged the same day. This plan was
>    written before it existed. **Everything downstream shifted by one slot**: T2 → `…000030`,
>    T3 → `…000040`, T4 → `…000050`. The tables above and below now reflect that.
> 2. **The migration omitted `ALTER COLUMN cogmap_id DROP NOT NULL`.** `cogmap_id` is `NOT NULL` on
>    `kb_cogmap_regions` *and* `kb_cogmap_components` (verified). Without the drop, T3's producer
>    INSERT fails on every context region. The plan *did* flag this — but in **Task 3's** section as
>    a back-reference, not in the SQL that executes it, which is precisely how it got missed. It is
>    now in the Step-1 SQL below, where it runs.
> 3. **"Contexts gain the shape columns kb_cogmaps already has" was half-true.** `kb_cogmaps` has
>    `shape_materialized_event_id` but **no `telos_centroid`** — that column is new on both anchors.
>    (Coherent: a cogmap's telos is a *declared* resource, `telos_resource_id NOT NULL`, whose
>    embedding is read directly; a context's telos is *computed* from its goal census, so it must be
>    snapshotted to be compared against. But the framing misled.)

**Files:**
- Create: `migrations/20260712000030_region_anchor_expand.sql`

**Interfaces:**
- Produces: `kb_cogmap_{regions,components,lenses,region_members}.home_anchor_table` / `.home_anchor_id`
- Produces: `kb_cogmap_lenses.{w_cos, knn_k, cos_floor, kappa_anchor_prior, telos_halflife_days, sw_in_progress, sw_backlog, sw_done, damper_paused, damper_completed}`
- Produces: `kb_contexts.{shape_materialized_event_id, telos_centroid}`
- Consumed by: T3 (producer), T4 (kernel), T5 (telos), T6 (clocks), T7 (wayfind).

- [ ] **Step 1: Write the migration**

Create `migrations/20260712000030_region_anchor_expand.sql`:

```sql
-- M1 of expand → migrate → contract (spec §3.6). PURELY ADDITIVE: main stays auto-deployable.
--
-- The four region tables are keyed on `cogmap_id NOT NULL`. kb_resource_homes and kb_edges already
-- solved this with a polymorphic (anchor_table, anchor_id) pair; the region tier is the last place
-- that hasn't. This migration adds the pair, backfills it, and leaves cogmap_id in place, dual-written,
-- so the PREVIOUS commit's code keeps working against the NEW schema.
--
-- M3 (drop cogmap_id; rename kb_cogmap_* -> kb_*) is an operator-run cutover, DEFERRED INDEFINITELY.
-- Until then the table names lie: kb_cogmap_regions will hold context regions. The COMMENTs at the
-- bottom of this file carry that honesty. Naming follows confidence, not the other way round.

-- ---------------------------------------------------------------------------
-- 1. The polymorphic anchor pair on the four region tables.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_cogmap_regions
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

ALTER TABLE kb_cogmap_components
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

-- On lenses the pair is NULLABLE-as-a-pair: (NULL, NULL) = a global default lens, which is how
-- telos-default and telos-default-propheavy are seeded today (cogmap_id IS NULL).
ALTER TABLE kb_cogmap_lenses
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

-- A context region has no cogmap. cogmap_id is NOT NULL on both tables today, so the T3 producer's
-- INSERT would fail on every context region without this. Dropping NOT NULL only WIDENS what is
-- accepted — the pre-M2 code path always supplies cogmap_id — so it is safe on auto-deploying main.
--
-- Note what this gives up: cogmap_id carries `REFERENCES kb_cogmaps(id) ON DELETE CASCADE`, which is
-- what reaps a cogmap's regions today. A context region has cogmap_id IS NULL, so it inherits no
-- cascade, and the anchor pair cannot carry an FK because it is polymorphic (the same trade
-- kb_edges and kb_resource_homes already make). Reaping context regions on context delete is the
-- producer's job, not the schema's.
ALTER TABLE kb_cogmap_regions    ALTER COLUMN cogmap_id DROP NOT NULL;
ALTER TABLE kb_cogmap_components ALTER COLUMN cogmap_id DROP NOT NULL;

UPDATE kb_cogmap_regions    SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id;
UPDATE kb_cogmap_components SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id;
UPDATE kb_cogmap_lenses     SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
    WHERE cogmap_id IS NOT NULL;

-- Live-region and live-component lookups now key on the anchor. The old cogmap_id indexes stay
-- until M3 (the previous commit's code still uses them).
CREATE INDEX idx_kb_cogmap_regions_anchor
    ON kb_cogmap_regions(home_anchor_table, home_anchor_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_cogmap_components_anchor_live
    ON kb_cogmap_components(home_anchor_table, home_anchor_id, lens_id) WHERE NOT is_folded;

-- Region members may now be context resources.
ALTER TABLE kb_cogmap_region_members
    DROP CONSTRAINT kb_cogmap_region_members_member_table_check,
    ADD CONSTRAINT kb_cogmap_region_members_member_table_check
        CHECK (member_table IN ('kb_resources', 'kb_cogmaps', 'kb_contexts'));

-- ---------------------------------------------------------------------------
-- 2. Contexts gain the shape columns.
--
-- shape_materialized_event_id mirrors kb_cogmaps. telos_centroid is NEW on both anchors — a cogmap
-- does not carry one because its telos is a DECLARED resource (kb_cogmaps.telos_resource_id, NOT
-- NULL), whose embedding can be read directly. A context has no declared telos: it is COMPUTED from
-- the liveness-weighted goal census (spec §3.4), so it must be snapshotted to be compared against.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_contexts
    ADD COLUMN shape_materialized_event_id UUID REFERENCES kb_events(id),
    -- The telos snapshot at last materialize. Gate 1 of the two-clock trigger (spec §3.5) compares
    -- the CURRENT liveness-weighted goal centroid against this; drift past epsilon refreshes salience
    -- WITHOUT re-clustering. Also what makes anchor_telos_drift() computable.
    ADD COLUMN telos_centroid vector(768);

-- ---------------------------------------------------------------------------
-- 3. Lens columns for the context regime.
--
-- DEFAULTS ARE THE POINT: every existing lens row gets w_cos = 0.0, which reproduces today's
-- declared-graph-only cogmap behavior BIT-FOR-BIT. Nothing about cogmaps changes.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_cogmap_lenses
    -- Formation: the embedding term (spec §3.1).
    ADD COLUMN w_cos               DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN knn_k               INT              NOT NULL DEFAULT 12,
    ADD COLUMN cos_floor           DOUBLE PRECISION NOT NULL DEFAULT 0.55,
    -- Wayfind: the anchor-kind prior (spec §3.7, consumed in T7).
    ADD COLUMN kappa_anchor_prior  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    -- Telos: goal liveness from the task census (spec §3.4, consumed in T5).
    ADD COLUMN telos_halflife_days DOUBLE PRECISION NOT NULL DEFAULT 30.0,
    ADD COLUMN sw_in_progress      DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    ADD COLUMN sw_backlog          DOUBLE PRECISION NOT NULL DEFAULT 0.35,
    ADD COLUMN sw_done             DOUBLE PRECISION NOT NULL DEFAULT 0.15,
    ADD COLUMN damper_paused       DOUBLE PRECISION NOT NULL DEFAULT 0.3,
    ADD COLUMN damper_completed    DOUBLE PRECISION NOT NULL DEFAULT 0.4;

-- ---------------------------------------------------------------------------
-- 4. Transitional COMMENTs. The names are wrong until M3 and we say so.
-- ---------------------------------------------------------------------------
COMMENT ON TABLE kb_cogmap_regions IS
    'TRANSITIONAL NAME. Holds regions for ANY anchor — contexts as well as cogmaps. Key on '
    '(home_anchor_table, home_anchor_id); `cogmap_id` is VESTIGIAL (dual-written for the pre-M2 code '
    'path, never read by new code). M3 drops cogmap_id and renames this to kb_regions. See '
    'docs/superpowers/specs/2026-07-11-context-regions-and-wayfinding-design.md §3.6.';
COMMENT ON COLUMN kb_cogmap_regions.cogmap_id IS
    'VESTIGIAL. Superseded by (home_anchor_table, home_anchor_id). NULL-meaningless for context '
    'regions. Dropped in M3. Do not read this in new code.';
COMMENT ON TABLE kb_cogmap_components IS
    'TRANSITIONAL NAME — see kb_cogmap_regions. Renamed to kb_components in M3.';
COMMENT ON COLUMN kb_cogmap_components.cogmap_id IS 'VESTIGIAL — see kb_cogmap_regions.cogmap_id.';
COMMENT ON TABLE kb_cogmap_lenses IS
    'TRANSITIONAL NAME — see kb_cogmap_regions. Renamed to kb_lenses in M3. A lens is IMMUTABLE: '
    'editing means asserting a new row. w_cos = 0.0 is the cogmap regime (declared graph only); '
    'w_cos > 0 is the context regime (embedding-primary). See spec §3.1–§3.2.';
COMMENT ON COLUMN kb_cogmap_lenses.w_cos IS
    'Weight on the sparse exact-kNN cosine affinity term. 0.0 = the cogmap regime, byte-identical to '
    'pre-2026-07 behavior. The context lens sets this to 1.0 — in a context the embedding is the '
    'PRIMARY signal of regionality, not a second-order readout.';
COMMENT ON COLUMN kb_cogmap_lenses.w_prop IS
    'Facet-overlap weight. Held at cogmap parity (0.4) in the context lens even though contexts carry '
    'ZERO facets today. A lens weight is meaning-when-present, not a frequency prior: zeroing it would '
    'make the discipline provably unrewarded, and an information system that returns no signal for '
    'signal provided gets routed around. See spec §3.2.';
COMMENT ON COLUMN kb_contexts.telos_centroid IS
    'Snapshot of the liveness-weighted goal centroid at last materialize. Gate 1 of the two-clock '
    'trigger compares the current telos against this; drift past epsilon refreshes salience without '
    're-clustering. See spec §3.5.';
```

- [ ] **Step 2: Verify it applies on a clean database and is PG17-portable**

```bash
cargo make docker-down && cargo make docker-up
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
sqlx migrate run
```

Expected: applies clean. Then grep your own migration for the portability trap:

```bash
rg -n "uuidv7\(\)" migrations/20260712000030_region_anchor_expand.sql
```

Expected: **no matches.** A hit means you used the PG18-native function, which passes here and breaks Neon PG17 in prod.

- [ ] **Step 3: Verify the backfill and the defaults**

```bash
psql "$DATABASE_URL" -c \
  "SELECT home_anchor_table, count(*) FROM kb_cogmap_regions GROUP BY 1;" -c \
  "SELECT name, w_cos, knn_k, cos_floor FROM kb_cogmap_lenses;"
```

Expected: every pre-existing region row has `home_anchor_table = 'kb_cogmaps'` (or the table is empty on a fresh DB), and **every seeded lens has `w_cos = 0.0`**. That zero is the regression floor.

- [ ] **Step 4: Run the existing region suite untouched — nothing may change**

```bash
cargo make test-artifacts
```

Expected: PASS. This migration is additive; no existing behavior may move.

- [ ] **Step 5: Regenerate sqlx caches and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make check
```

- [ ] **Step 6: Commit**

```bash
git add migrations/20260712000030_region_anchor_expand.sql .sqlx crates/*/.sqlx
git commit -m "feat(schema): M1 — polymorphic anchor on the region tables + context-regime lens columns

Additive expand phase (spec §3.6). The four region tables gain
(home_anchor_table, home_anchor_id), backfilled from cogmap_id, which stays and
is dual-written so main remains auto-deployable. kb_contexts gains
shape_materialized_event_id and telos_centroid. Lenses gain w_cos / knn_k /
cos_floor / kappa_anchor_prior and the telos-liveness constants.

Every existing lens defaults to w_cos = 0.0 — cogmap behavior is byte-identical.

M3 (drop cogmap_id, rename kb_cogmap_* -> kb_*) is deferred indefinitely; the
table and column COMMENTs carry that honesty in the interim."
```

---

## Task 3: Anchor-generalize the producer — ✅ LANDED

Spec §3.6 (M2), §3.9. **Depends on T2.**

This is the mechanical widening — no behavior change. `w_cos` is still 0 everywhere, so at the end of this task the producer can *address* a context but forms nothing useful in one. That's expected; T4 is what makes it work. Two live bugs get fixed here because this work is what surfaced them.

> **Corrected 2026-07-11 during execution.** Five defects in what this task said. **T4 comes from the
> same architecture sweep — assume the same failure rate.**
>
> 1. **The `cogmap_region_centrality` body below is fiction.** The live function is a two-CTE form
>    (`mem`, then `internal` summing `e.weight` where BOTH endpoints are in `mem`), times
>    `count(*) FROM mem` — not the triple-JOIN `sum(e.weight) * max(r.member_count)` quoted here. The
>    home-filter fix is real; it was written against the printed body. **Third SQL reconstruction in
>    this arc to be wrong.**
> 2. **The regression test as specified could never fail.** It asserts the membership fingerprint
>    equals *itself* across two materializes. But `component_fingerprint` hashes member **UUIDs**, and
>    the seed loader mints fresh UUIDs into every ephemeral `#[sqlx::test]` database — so that is a
>    DETERMINISM check, not a BEHAVIOR-PRESERVATION check. It stays green even if the refactor
>    reshuffles which resources cluster together, which is the one thing the floor exists to catch.
>    (T1 shipped a test with the same "cannot fail" shape. Twice is a pattern.) **What shipped**
>    instead: a UUID-free canonical signature — each live region as the sorted set of its member
>    *titles*, regions sorted among themselves — captured as a golden from the CURRENT producer, then
>    re-asserted against the anchor-keyed one.
> 3. **Four surfaces the file list omits**, every one keyed on `cogmap_id`: `drift.rs`
>    (`live_components` **and** a second `payload->>'cogmap_id'` probe in
>    `touched_since_last_materialize`); `events.rs` + `payloads.rs` (`SeedAction::Materialize` and the
>    `RegionMaterialized` struct — the event schema is a **Rust** change, not just SQL);
>    `write.rs:last_materialize_watermark` (a third payload probe); and `temper-services`
>    (`db_backend.rs` ×2, `materialize_service.rs`) — the production callers.
> 4. **`replay()` re-projects historical `region_materialized` events** (`replay.rs:322`). The strict
>    `RAISE EXCEPTION` on an unknown `home_anchor_table` specified below would therefore **raise on
>    every pre-T3 payload and break ledger replay outright.** `kb_events` is append-only; those rows
>    are immortal. Both SQL functions and the Rust ledger probes now **dual-read** — anchor pair, else
>    the old `cogmap_id`.
> 5. **The payload schema is generated, not hand-written.** `RegionMaterialized` → schemars →
>    `tests/fixtures/payloads/region_materialized.v1.schema.json` → boot-seed → `kb_event_types`.
>    Change the struct, run `UPDATE_SCHEMA=1 cargo test -p temper-substrate --features scenario-schema
>    --test payload_schema`, and carry the *generated* JSON into the migration (fresh DBs get it from
>    the boot-seed; the migration's `UPDATE` is what fixes an already-seeded prod).
>
> **The payload widened by dual-write, not swap** (decided during execution): `home_anchor_table` +
> `home_anchor_id` become required, and `cogmap_id` **stays** as an optional property, written for
> cogmap acts. A swap would have left `last_materialize_watermark` unable to find any prior act on the
> first pass after deploy — incremental would silently skip the moved-member readout refresh, once,
> with no error. `_event_append` does **not** validate payloads against `payload_schema` (verified), so
> there is no deploy-ordering hazard either way.

**Files:**
- Modify: `crates/temper-core/src/types/home.rs`
- Modify: `crates/temper-substrate/src/substrate.rs`
- Modify: `crates/temper-substrate/src/write.rs`
- Modify: `crates/temper-substrate/src/replay.rs`
- Create: `migrations/20260712000040_region_anchor_functions.sql`

**Interfaces:**
- Consumes: `HomeAnchor` (`temper_core::types::home`), the anchor columns from T2.
- Produces: `HomeAnchor::table(&self) -> &'static str`, `HomeAnchor::uuid(&self) -> Uuid`, `HomeAnchor::from_parts(table: &str, id: Uuid) -> Option<HomeAnchor>`
- Produces: `substrate::load(pool: &PgPool, anchor: HomeAnchor, lens_name: &str) -> Result<Substrate>`
- Produces: `write::materialize(pool: &PgPool, anchor: HomeAnchor, lens_name: &str, emitter: EntityId) -> Result<MaterializeOutcome>`
- Produces: `write::incremental_materialize(pool: &PgPool, anchor: HomeAnchor, lens_name: &str, emitter: EntityId) -> Result<MaterializeOutcome>`
- Consumed by: T4, T5, T6, T7.

- [ ] **Step 1: Add the SQL-binding helpers to `HomeAnchor`**

`temper-core/src/types/home.rs` already has the enum. Add the helpers the producer needs — this is the whole reason the type exists, and it prevents every call site from re-deriving the string literal:

```rust
use uuid::Uuid;

impl HomeAnchor {
    /// The `anchor_table` / `home_anchor_table` SQL discriminant.
    pub fn table(&self) -> &'static str {
        match self {
            HomeAnchor::Context(_) => "kb_contexts",
            HomeAnchor::Cogmap(_) => "kb_cogmaps",
        }
    }

    /// The bare UUID, for binding alongside [`table`].
    ///
    /// [`table`]: HomeAnchor::table
    pub fn uuid(&self) -> Uuid {
        match self {
            HomeAnchor::Context(c) => c.uuid(),
            HomeAnchor::Cogmap(m) => m.uuid(),
        }
    }

    /// Reconstruct from a `(table, id)` row pair. `None` for an unrecognized discriminant, which
    /// escalates at the call site rather than defaulting to a wrong anchor kind.
    pub fn from_parts(table: &str, id: Uuid) -> Option<Self> {
        match table {
            "kb_contexts" => Some(HomeAnchor::Context(ContextId::from(id))),
            "kb_cogmaps" => Some(HomeAnchor::Cogmap(CogmapId::from(id))),
            _ => None,
        }
    }
}
```

Add tests in the same file's `mod tests`:

```rust
    #[test]
    fn home_anchor_table_and_uuid_round_trip_through_from_parts() {
        let c = HomeAnchor::Context(ContextId::new());
        assert_eq!(c.table(), "kb_contexts");
        assert_eq!(HomeAnchor::from_parts(c.table(), c.uuid()), Some(c.clone()));

        let m = HomeAnchor::Cogmap(CogmapId::new());
        assert_eq!(m.table(), "kb_cogmaps");
        assert_eq!(HomeAnchor::from_parts(m.table(), m.uuid()), Some(m.clone()));

        assert_eq!(HomeAnchor::from_parts("kb_teams", Uuid::nil()), None);
    }
```

`HomeAnchor` derives `PartialEq` but not `Copy`/`Eq`/`Hash`; add `Copy, Eq, Hash` to the derive list — it is two ids, and the producer threads it through hot loops.

- [ ] **Step 2: Pin the regression floor BEFORE touching the producer**

This is the most important step in the plan. Capture today's fingerprints so the refactor is provably behavior-preserving.

```bash
cargo make test-artifacts 2>&1 | tee /tmp/regions-before.txt
```

Then add a golden-fingerprint test. Find how the existing scenario tests assert (`rg -n "fingerprint" crates/temper-substrate/tests/`) and follow that pattern. Create `crates/temper-substrate/tests/anchor_refactor_regression.rs`:

```rust
#![cfg(feature = "artifact-tests")]

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;

/// THE REGRESSION FLOOR (spec §5). Materializing a cogmap through the anchor-generalized producer
/// must produce byte-identical component fingerprints to the cogmap-keyed producer it replaces.
/// If this goes red, the refactor changed behavior — do not adjust the expectation.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_materialize_is_unchanged_under_the_anchor_producer(pool: PgPool) -> sqlx::Result<()> {
    // Load the onboarding-cogmap seed the existing suite uses.
    let cogmap = temper_substrate::scenario::bootseed::load_fixture(&pool, "onboarding-cogmap")
        .await
        .expect("seed loads");

    let emitter = temper_substrate::test_support::system_emitter(&pool).await;
    let out = temper_substrate::write::materialize(
        &pool,
        HomeAnchor::Cogmap(cogmap),
        "telos-default",
        emitter,
    )
    .await
    .expect("materialize");

    // Fingerprints are the membership-determining hash. Same corpus + same lens => same set.
    let mut fps: Vec<String> = sqlx::query_scalar(
        "SELECT fingerprint FROM kb_cogmap_components \
         WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$1 AND NOT is_folded \
         ORDER BY fingerprint",
    )
    .bind(cogmap.uuid())
    .fetch_all(&pool)
    .await?;
    fps.sort();

    // Materialize AGAIN — determinism: an idempotent re-run reuses every component.
    let again = temper_substrate::write::incremental_materialize(
        &pool,
        HomeAnchor::Cogmap(cogmap),
        "telos-default",
        emitter,
    )
    .await
    .expect("re-materialize");
    assert_eq!(again.changed, 0, "an unchanged corpus must re-cluster nothing");
    assert_eq!(again.stale, 0);

    let mut fps2: Vec<String> = sqlx::query_scalar(
        "SELECT fingerprint FROM kb_cogmap_components \
         WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$1 AND NOT is_folded \
         ORDER BY fingerprint",
    )
    .bind(cogmap.uuid())
    .fetch_all(&pool)
    .await?;
    fps2.sort();

    assert_eq!(fps, fps2, "fingerprints must be stable across materializes");
    assert!(!fps.is_empty(), "the seed must produce at least one component");
    let _ = out;
    Ok(())
}
```

> The helper names above (`scenario::bootseed::load_fixture`, `test_support::system_emitter`, `MaterializeOutcome.changed/.stale`) are **guesses from the architecture map, not verified**. Before writing this test, run:
> ```bash
> rg -n "fn load_fixture|fn system_emitter|struct MaterializeOutcome" crates/temper-substrate/src/
> rg -n "materialize" crates/temper-substrate/tests/ | head -20
> ```
> and use the real names. Copy the setup out of an existing artifact test rather than inventing it.

- [ ] **Step 3: Run the regression test against the CURRENT producer — it must pass before you refactor**

Temporarily call the existing `materialize_cogmap(pool, cogmap, ...)` instead of `materialize(pool, HomeAnchor::Cogmap(...), ...)`.

```bash
cargo make test-artifacts -- anchor_refactor_regression
```

Expected: PASS. **A green baseline here is what makes the refactor safe.** If it fails now, the test setup is wrong, not the producer.

- [ ] **Step 4: Widen `substrate::load`**

In `crates/temper-substrate/src/substrate.rs`, replace the three hard-wired `'kb_cogmaps'` predicates. The signature becomes `load(pool: &PgPool, anchor: HomeAnchor, lens_name: &str)`.

Because the anchor table is now a bound *value*, the `sqlx::query!` macros still work — bind it as `$1` and the id as `$2`:

```rust
pub async fn load(pool: &PgPool, anchor: HomeAnchor, lens_name: &str) -> Result<Substrate> {
    let anchor_table = anchor.table();
    let anchor_id = anchor.uuid();

    // resources homed in the anchor (context OR cogmap)
    let nodes: Vec<ResourceId> = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_resource_homes WHERE anchor_table=$1 AND anchor_id=$2",
        anchor_table,
        anchor_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(ResourceId::from)
    .collect();

    // declared edges homed in the anchor, both endpoints resources
    let edge_rows = sqlx::query!(
        "SELECT source_id, target_id, edge_kind::text AS \"kind!\", label, weight \
         FROM kb_edges WHERE home_anchor_table=$1 AND home_anchor_id=$2 \
           AND source_table='kb_resources' AND target_table='kb_resources' AND NOT is_folded",
        anchor_table,
        anchor_id,
    )
    .fetch_all(pool)
    .await?;
    // ... edge mapping unchanged ...

    // facets on those resources — UNCHANGED. Facets hang off resources, not the anchor.
    // (Contexts carry zero facets today. That is a fact about the corpus, not the kernel:
    // this query stays anchor-agnostic so a facet asserted in a context DOES contribute.)

    // the named lens for this anchor, else the global default (home_anchor_table IS NULL)
    let lr = sqlx::query!(
        "SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor, \
                s_telos, s_ref, s_central, resolution \
         FROM kb_cogmap_lenses \
         WHERE name=$3 AND (home_anchor_table IS NULL OR (home_anchor_table=$1 AND home_anchor_id=$2)) \
         ORDER BY home_anchor_table NULLS LAST LIMIT 1",
        anchor_table,
        anchor_id,
        lens_name,
    )
    .fetch_one(pool)
    .await?;
    // ... Lens construction, now including w_cos / knn_k / cos_floor ...
}
```

Note `ORDER BY home_anchor_table NULLS LAST` — an anchor-specific lens still wins over the global default, exactly as `ORDER BY cogmap_id NULLS LAST` did.

Also rename `cogmap_by_name` → keep it (it's still used) and add a peer:

```rust
pub async fn context_by_slug(pool: &PgPool, owner: ProfileId, slug: &str) -> Result<ContextId> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM kb_contexts WHERE owner_table='kb_profiles' AND owner_id=$1 AND slug=$2",
        owner.uuid(),
        slug,
    )
    .fetch_one(pool)
    .await?;
    Ok(ContextId::from(id))
}
```

- [ ] **Step 5: Widen `write.rs` — and persist member affinity (bug §3.9.1)**

Rename `materialize_cogmap` → `materialize` and `incremental_materialize_cogmap` → `incremental_materialize`, both taking `anchor: HomeAnchor`. Widen `fold_live_regions`, `fold_live_components`, `create_component`, and `assert_region` to write **both** the anchor pair and (for cogmaps) `cogmap_id`, so the pre-M2 code path keeps working:

```rust
// assert_region — dual-write cogmap_id during the expand window (spec §3.6 M1).
// cogmap_id is NULL for context regions; the old code path never reads those rows.
let cogmap_id: Option<Uuid> = match ctx.anchor {
    HomeAnchor::Cogmap(m) => Some(m.uuid()),
    HomeAnchor::Context(_) => None,
};
let region: Uuid = sqlx::query(
    "INSERT INTO kb_cogmap_regions \
       (id, cogmap_id, home_anchor_table, home_anchor_id, lens_id, component_id, centroid, \
        salience, label, member_count, asserted_by_event_id, last_event_id) \
     VALUES ($1, $2, $3, $4, $5, $6, $7::vector, 0.0, NULL, $8, $9, $9) RETURNING id",
)
// ... binds ...
```

> ✅ **Already handled — T2 shipped the `DROP NOT NULL`.** `cogmap_id` was `NOT NULL` on both
> `kb_cogmap_regions` and `kb_cogmap_components`; `20260712000030_region_anchor_expand.sql` drops it
> on both, so the INSERT below can write `cogmap_id = NULL` for a context region.
>
> Note the consequence for this step: cogmap_id's `ON DELETE CASCADE` from `kb_cogmaps` is what reaps
> a cogmap's regions today. A context region has `cogmap_id IS NULL` and the anchor pair carries no FK
> (it's polymorphic), so **nothing reaps context regions on context delete** — that is now the
> producer's job, not the schema's. Don't assume the cascade covers you.

Then fix bug §3.9.1. `kb_cogmap_region_members.affinity` is never written, yet `graph_region_members`, `graph_region_territories`, `graph_cogmap_territories`, and `atlas_search` all `ORDER BY m.affinity DESC NULLS LAST` — so every derived region label in the product today is arbitrary. Define it as **the member's average-link affinity to the rest of its component** and persist it:

```rust
/// Average-link affinity of `m` to the other members of its region — "how core is this member".
/// This is what the four `ORDER BY affinity DESC` readers have always wanted and never had.
/// A singleton region yields 0.0 (no peers to be central to).
fn member_affinity(m: ResourceId, members: &[ResourceId], sub: &Substrate) -> f64 {
    let peers: Vec<ResourceId> = members.iter().copied().filter(|&x| x != m).collect();
    if peers.is_empty() {
        return 0.0;
    }
    let total: f64 = peers
        .iter()
        .map(|&p| affinity(m, p, &sub.edges, &sub.facets, &sub.lens))
        .sum();
    total / peers.len() as f64
}
```

and in the member INSERT:

```rust
for m in members {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
         VALUES ($1,'kb_resources',$2,$3)",
    )
    .bind(region)
    .bind(m.uuid())
    .bind(member_affinity(*m, members, sub))
    .execute(&mut *tx)
    .await?;
}
```

`assert_region` therefore needs the `Substrate` (for edges/facets/lens) threaded into `AssertRegionCtx`. It is already a params struct — add a `sub: &'a Substrate` field rather than a seventh positional argument.

- [ ] **Step 6: Widen `replay.rs`**

`formation_touched_count_since` and `content_touched_resources_since` both filter `e.producing_anchor_table = 'kb_cogmaps'`. Bind the anchor instead. **Events are already anchored to contexts** — `steward_ingest_delta` counts them today — so this is a widened filter, not new plumbing.

```rust
pub async fn formation_touched_count_since(
    pool: &PgPool,
    anchor: HomeAnchor,
    watermark: Option<EventId>,
) -> Result<i64> {
    let n = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events e \
           JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE e.producing_anchor_table = $1 AND e.producing_anchor_id = $2 \
            AND ($3::uuid IS NULL OR e.id > $3) \
            AND et.name = ANY($4)",
        anchor.table(),
        anchor.uuid(),
        watermark.map(|w| w.uuid()),
        &FORMATION_EVENT_NAMES[..],
    )
    .fetch_one(pool)
    .await?;
    Ok(n.unwrap_or(0))
}
```

- [ ] **Step 7: SQL functions — anchor the event, fix the centrality home-filter (bug §3.9.2)**

Create `migrations/20260712000040_region_anchor_functions.sql`.

> **⚠️ Open this migration with a catch-up backfill — it is not optional.**
>
> T2 shipped and deployed; its backfill anchored every row that existed **at migration time**. But the
> producer only starts dual-writing the anchor pair in *this* task — so every region and component
> materialized in the **T2 → T3 window** lands with `home_anchor_id IS NULL`, and T2's one-shot
> backfill has already run and will never catch them.
>
> That breaks the fold below. `fold_live_regions` / `fold_live_components` now find live rows **by
> anchor**; a NULL-anchor row does not match, so it is never folded and **survives as a live region
> alongside the freshly-created ones** — duplicate live regions, inflated member counts, stale rows in
> every region read.
>
> ```sql
> -- Catch-up backfill for rows materialized in the T2→T3 window (see above).
> UPDATE kb_cogmap_regions    SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
>     WHERE home_anchor_id IS NULL AND cogmap_id IS NOT NULL;
> UPDATE kb_cogmap_components SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
>     WHERE home_anchor_id IS NULL AND cogmap_id IS NOT NULL;
> ```
>
> Verify `SELECT count(*) FILTER (WHERE home_anchor_id IS NULL) FROM kb_cogmap_regions;` is **0**
> afterwards. Cheap insurance whether or not rows have accumulated.

Then:

```sql
-- Producer-side SQL for the anchor-generalized region tier (spec §3.6 M2, §3.9).

-- BUG FIX (spec §3.9.2). cogmap_region_centrality sums kb_edges.weight with NO home_anchor filter,
-- so it already counts edges asserted OUTSIDE the map. Under a polymorphic anchor that would
-- silently mix context and cogmap edges into one region's centrality. Restrict to edges homed in
-- the region's own anchor.
CREATE OR REPLACE FUNCTION cogmap_region_centrality(p_region uuid)
RETURNS double precision
LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(e.weight), 0) * coalesce(max(r.member_count), 0)
      FROM kb_cogmap_regions r
      JOIN kb_cogmap_region_members ma ON ma.region_id = r.id
      JOIN kb_cogmap_region_members mb ON mb.region_id = r.id
      JOIN kb_edges e
        ON  e.source_table = 'kb_resources' AND e.source_id = ma.member_id
        AND e.target_table = 'kb_resources' AND e.target_id = mb.member_id
        AND NOT e.is_folded
        -- THE FIX: the edge must be homed in the SAME anchor as the region.
        AND e.home_anchor_table = r.home_anchor_table
        AND e.home_anchor_id    = r.home_anchor_id
     WHERE r.id = p_region;
$$;

COMMENT ON FUNCTION cogmap_region_centrality(uuid) IS
    'Internal declared-affinity density x size. Edges are home-filtered to the region''s own anchor '
    '(added 2026-07-12) — previously unfiltered, which counted edges from outside the map.';

-- region_materialize / _project_region_materialized: anchor the event at the region''s anchor
-- rather than hard-coding ('kb_cogmaps', cogmap_id), and project the watermark onto whichever
-- anchor table it belongs to.
CREATE OR REPLACE FUNCTION _project_region_materialized(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_table text := p_payload->>'home_anchor_table';
    v_id    uuid := (p_payload->>'home_anchor_id')::uuid;
BEGIN
    IF v_table = 'kb_cogmaps' THEN
        UPDATE kb_cogmaps  SET shape_materialized_event_id = p_event WHERE id = v_id;
    ELSIF v_table = 'kb_contexts' THEN
        UPDATE kb_contexts SET shape_materialized_event_id = p_event WHERE id = v_id;
    ELSE
        RAISE EXCEPTION 'region_materialized: unknown home_anchor_table %', v_table;
    END IF;
END;
$$;
```

The `region_materialized` **event-type JSON schema** (seeded in `20260624000003_canonical_seed.sql:54`) has `cogmap_id` as a *required* payload field. Widen it — required becomes `[home_anchor_table, home_anchor_id, lens_id, watermark_event_id, membership_fingerprint, region_ids]`:

```bash
psql "$DATABASE_URL" -c \
  "SELECT payload_schema FROM kb_event_types WHERE name='region_materialized';"
```

and `UPDATE kb_event_types SET payload_schema = ... WHERE name = 'region_materialized';` in the same migration, carrying the real current schema forward with the two keys swapped. **Do not hand-write the schema from memory** — print it and edit it.

Also rewrite `region_materialize(p_payload, p_emitter)` to anchor the appended event at `(home_anchor_table, home_anchor_id)` instead of `('kb_cogmaps', cogmap_id)`. Print the current body with `\sf region_materialize` and widen it.

> `kb_events` is **append-only** (a trigger blocks UPDATE/DELETE). This migration changes the *schema* of future events, not any existing row. Old `region_materialized` events carrying `cogmap_id` stay valid and unread.

- [ ] **Step 8: Run the regression test against the NEW producer**

```bash
cargo make test-artifacts -- anchor_refactor_regression
```

Expected: PASS, **with the same fingerprints as Step 3.** This is the proof the refactor is behavior-preserving.

- [ ] **Step 9: Run the whole substrate suite**

```bash
cargo make test-artifacts
cargo nextest run -p temper-substrate
diff <(grep -oE "^\s+(PASS|FAIL).*" /tmp/regions-before.txt | sort) \
     <(cargo make test-artifacts 2>&1 | grep -oE "^\s+(PASS|FAIL).*" | sort)
```

Expected: no test moved from PASS to FAIL. Do **not** trust nextest's per-binary "Summary" line — `--no-fail-fast` makes it meaningless. Trust the exit code, or grep for `error: test run failed`.

- [ ] **Step 10: Run the callers — a backend-command change needs the API integration target**

```bash
cargo nextest run -p temper-api --features test-db --test relationship_handler_test
cargo make test-e2e
```

- [ ] **Step 11: Regenerate sqlx caches and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make check
```

- [ ] **Step 12: Commit**

```bash
git add crates/temper-core/src/types/home.rs \
        crates/temper-substrate/src/{substrate.rs,write.rs,replay.rs} \
        crates/temper-substrate/tests/anchor_refactor_regression.rs \
        migrations/20260712000040_region_anchor_functions.sql \
        .sqlx crates/*/.sqlx
git commit -m "refactor(substrate): anchor-generalize the region producer

load() / materialize() / incremental_materialize() take a HomeAnchor instead of
a CogmapId; the three hard-wired 'kb_cogmaps' predicates in load(), the fold /
create / assert writers, and replay's event counters all widen to the
(home_anchor_table, home_anchor_id) pair. cogmap_id is dual-written through the
expand window. Behavior is byte-identical: anchor_refactor_regression pins the
component fingerprints.

Fixes two live bugs this work surfaced (spec §3.9):

- kb_cogmap_region_members.affinity was NEVER written, yet four readers
  ORDER BY it — so every 'top member' and derived region label in the product
  was arbitrary. Now persisted as the member's average-link affinity to the rest
  of its component.
- cogmap_region_centrality summed kb_edges.weight with no home_anchor filter,
  counting edges from outside the map. Now restricted to the region's own anchor."
```

---

## Task 4: The `w_cos` kernel

Spec §3.1, §3.2. **Depends on T3.** This is the heart of the arc.

**Files:**
- Create: `crates/temper-substrate/src/knn.rs`
- Modify: `crates/temper-substrate/src/affinity.rs`
- Modify: `crates/temper-substrate/src/substrate.rs` (carry pooled embeddings into `Substrate`)
- Create: `migrations/20260712000050_workflow_default_lens.sql`

**Interfaces:**
- Consumes: `Lens` (T3), `Substrate` (T3), the lens columns (T2).
- Produces: `knn::KnnGraph`, `knn::build(embeddings: &HashMap<ResourceId, Vec<f32>>, k: usize, floor: f64) -> KnnGraph`, `KnnGraph::sim(a, b) -> f64`
- Produces: `Lens::workflow_default()`
- Produces: `affinity(a, b, edges, facets, knn, lens) -> f64` — **note the new `knn` parameter**; every caller updates.

- [ ] **Step 1: Write the failing kernel tests**

Add to `crates/temper-substrate/src/affinity.rs`'s `mod tests`:

```rust
    #[test]
    fn w_cos_zero_reproduces_the_declared_only_kernel() {
        // THE REGRESSION FLOOR, at unit grain. A cogmap lens must be blind to the embedding term.
        let (a, b) = ids();
        let lens = Lens::telos_default(); // w_cos == 0.0
        let knn = KnnGraph::from_pairs(&[(a, b, 0.99)]); // maximal similarity
        assert_eq!(
            affinity(a, b, &[], &[], &knn, &lens),
            0.0,
            "with w_cos=0 a near-identical pair must still have zero affinity"
        );
    }

    #[test]
    fn w_cos_contributes_the_knn_similarity_when_the_pair_is_a_neighbour() {
        let (a, b) = ids();
        let lens = Lens { w_cos: 1.0, ..Lens::telos_default() };
        let knn = KnnGraph::from_pairs(&[(a, b, 0.8)]);
        assert!((affinity(a, b, &[], &[], &knn, &lens) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn a_pair_outside_the_knn_graph_contributes_nothing_however_similar() {
        // Sparsity is the whole point: cosine is DENSE, so only the top-k above the floor may
        // contribute. Otherwise the affinity graph is complete and connected_components is useless.
        let (a, b) = ids();
        let lens = Lens { w_cos: 1.0, ..Lens::telos_default() };
        let knn = KnnGraph::from_pairs(&[]); // b is not among a's neighbours
        assert_eq!(affinity(a, b, &[], &[], &knn, &lens), 0.0);
    }

    #[test]
    fn declared_edges_and_cosine_are_additive_not_exclusive() {
        // The context regime: cosine is primary, the declared graph is weak supervision ON TOP.
        let (a, b) = ids();
        let lens = Lens { w_cos: 1.0, w_near: 0.35, ..Lens::telos_default() };
        let knn = KnnGraph::from_pairs(&[(a, b, 0.6)]);
        let edges = vec![Edge { src: a, tgt: b, kind: EdgeKind::Near, weight: 1.0, label: None }];
        // 0.35*1.0 (declared) + 1.0*0.6 (inferred) = 0.95
        assert!((affinity(a, b, &edges, &[], &knn, &lens) - 0.95).abs() < 1e-9);
    }

    #[test]
    fn workflow_default_holds_the_deliberate_signals_at_cogmap_parity() {
        // Spec §3.2. A weight is meaning-when-present, not a frequency prior. Contexts carry zero
        // facets TODAY — that is a fact about the corpus, not the kernel. Zeroing these would make
        // the discipline provably unrewarded, and an information system that returns no signal for
        // signal provided gets routed around. If someone ever asserts an express edge or a facet in
        // a context, it MUST count.
        let ctx = Lens::workflow_default();
        let map = Lens::telos_default();
        assert_eq!(ctx.w_express, map.w_express);
        assert_eq!(ctx.w_contains, map.w_contains);
        assert_eq!(ctx.w_prop, map.w_prop);
        // and the regime switch is on
        assert_eq!(ctx.w_cos, 1.0);
        assert_eq!(map.w_cos, 0.0);
    }
```

Add to a new `crates/temper-substrate/src/knn.rs`:

```rust
    #[test]
    fn build_keeps_only_top_k_above_the_floor_and_is_symmetric() {
        let a = ResourceId::from(Uuid::from_u128(1));
        let b = ResourceId::from(Uuid::from_u128(2));
        let c = ResourceId::from(Uuid::from_u128(3));
        // a is close to b (0.9), far from c (0.2). Floor 0.55 excludes c.
        let embs = HashMap::from([
            (a, vec![1.0, 0.0]),
            (b, vec![0.95, 0.31]),  // cos(a,b) ~ 0.95
            (c, vec![0.0, 1.0]),    // cos(a,c) = 0.0
        ]);
        let g = build(&embs, 2, 0.55);
        assert!(g.sim(a, b) > 0.9);
        assert_eq!(g.sim(a, c), 0.0, "below the floor contributes nothing");
        assert_eq!(g.sim(a, b), g.sim(b, a), "the graph is symmetric");
    }

    #[test]
    fn k_bounds_the_neighbour_count() {
        // 5 mutually-similar nodes, k=2 => each node keeps at most 2 neighbours (before symmetrizing).
        let ids: Vec<ResourceId> =
            (1..=5).map(|i| ResourceId::from(Uuid::from_u128(i))).collect();
        let embs: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, vec![1.0, i as f32 * 0.01]))
            .collect();
        let g = build(&embs, 2, 0.0);
        for &id in &ids {
            assert!(g.neighbours(id).len() <= 4, "symmetrized degree stays bounded by ~2k");
        }
    }

    #[test]
    fn build_is_deterministic() {
        // Determinism is a hard requirement — membership_fingerprint depends on it. This is also
        // why we compute exact kNN and never touch HNSW.
        let ids: Vec<ResourceId> =
            (1..=8).map(|i| ResourceId::from(Uuid::from_u128(i))).collect();
        let embs: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, vec![(i as f32).sin(), (i as f32).cos()]))
            .collect();
        let a = build(&embs, 3, 0.0);
        let b = build(&embs, 3, 0.0);
        for &x in &ids {
            for &y in &ids {
                assert_eq!(a.sim(x, y), b.sim(x, y), "identical inputs must give identical graphs");
            }
        }
    }
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo nextest run -p temper-substrate knn affinity
```

Expected: FAIL to compile — `KnnGraph` not found, `Lens::workflow_default` not found, `affinity` takes 5 arguments not 6.

- [ ] **Step 3: Implement `knn.rs`**

```rust
//! The sparse exact-kNN affinity graph — the context regime's primary signal (spec §3.1).
//!
//! Cosine is DENSE: every pair of resources has a nonzero similarity. Dropping a raw cosine into
//! `affinity()` would make the affinity graph COMPLETE — `connected_components` returns one blob,
//! the pre-pass that makes agglomeration tractable stops doing anything, and cost goes Θ(n²).
//! So the embedding term contributes a *sparsified* graph: each node's top-k neighbours above a
//! similarity floor, and nothing else.
//!
//! Computed EXACTLY, never via HNSW. Two reasons, and the second is the binding one:
//!   1. A scoped corpus is small enough to scan (the same reasoning as the #358 search fix).
//!   2. An approximate index is not reproducible across index rebuilds, and `membership_fingerprint`
//!      depends on formation being deterministic.
//!
//! SCALE CEILING (spec §7): this is O(n²) in pairwise cosines. Comfortable at ~1k nodes
//! (@me/temper is 1,012), fine at a few thousand, NOT fine at 50k. When a context crosses that,
//! the options are blocked/tiled exact computation or accepting an approximate index and giving up
//! fingerprint determinism. Revisit here.

use std::collections::HashMap;

use crate::ids::ResourceId;

#[derive(Debug, Default, Clone)]
pub struct KnnGraph {
    /// Symmetric: an entry exists under both (a,b) and (b,a).
    sims: HashMap<(ResourceId, ResourceId), f64>,
    adj: HashMap<ResourceId, Vec<ResourceId>>,
}

impl KnnGraph {
    /// Similarity of the pair, or 0.0 if `b` is not a retained neighbour of `a`.
    pub fn sim(&self, a: ResourceId, b: ResourceId) -> f64 {
        self.sims.get(&(a, b)).copied().unwrap_or(0.0)
    }

    pub fn neighbours(&self, a: ResourceId) -> &[ResourceId] {
        self.adj.get(&a).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Test constructor: build directly from explicit (a, b, sim) triples.
    pub fn from_pairs(pairs: &[(ResourceId, ResourceId, f64)]) -> Self {
        let mut g = KnnGraph::default();
        for &(a, b, s) in pairs {
            g.sims.insert((a, b), s);
            g.sims.insert((b, a), s);
            g.adj.entry(a).or_default().push(b);
            g.adj.entry(b).or_default().push(a);
        }
        g
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..a.len().min(b.len()) {
        dot += a[i] as f64 * b[i] as f64;
        na += (a[i] as f64).powi(2);
        nb += (b[i] as f64).powi(2);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Build the symmetric sparse kNN graph. Deterministic: neighbour selection sorts by
/// (similarity DESC, resource-id ASC), so exact float ties fall to a stable id order rather than
/// hash-map iteration order.
pub fn build(
    embeddings: &HashMap<ResourceId, Vec<f32>>,
    k: usize,
    floor: f64,
) -> KnnGraph {
    // Sort the node list so the outer loop is deterministic too.
    let mut nodes: Vec<ResourceId> = embeddings.keys().copied().collect();
    nodes.sort_by_key(|r| r.uuid());

    let mut g = KnnGraph::default();
    for &a in &nodes {
        let ea = &embeddings[&a];
        let mut cands: Vec<(ResourceId, f64)> = nodes
            .iter()
            .filter(|&&b| b != a)
            .map(|&b| (b, cosine(ea, &embeddings[&b])))
            .filter(|&(_, s)| s >= floor)
            .collect();
        // similarity DESC, then id ASC — a total order, so no float tie falls to iteration order.
        cands.sort_by(|x, y| {
            y.1.partial_cmp(&x.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| x.0.uuid().cmp(&y.0.uuid()))
        });
        for (b, s) in cands.into_iter().take(k) {
            // Symmetrize: b is a's neighbour, so the pair binds in both directions. A pair kept by
            // EITHER endpoint's top-k survives — this is a mutual-OR kNN graph, which keeps a
            // hub-and-spoke topology (goals!) from being severed by a popular node's k limit.
            g.sims.insert((a, b), s);
            g.sims.insert((b, a), s);
            g.adj.entry(a).or_default().push(b);
            g.adj.entry(b).or_default().push(a);
        }
    }
    for v in g.adj.values_mut() {
        v.sort_by_key(|r| r.uuid());
        v.dedup();
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    // ... the three tests from Step 1 ...
}
```

Register it: add `pub mod knn;` to `crates/temper-substrate/src/lib.rs`.

- [ ] **Step 4: Extend `Lens` and `affinity()`**

In `affinity.rs`:

```rust
#[derive(Clone, Debug)]
pub struct Lens {
    pub w_express: f64,
    pub w_contains: f64,
    pub w_leads_to: f64,
    pub w_near: f64,
    pub w_prop: f64,
    /// Weight on the sparse exact-kNN cosine term. 0.0 = the cogmap regime (declared graph only,
    /// byte-identical to pre-2026-07 behavior). > 0 = the context regime, where the embedding is the
    /// PRIMARY signal of regionality rather than a second-order readout. See spec §3.1.
    pub w_cos: f64,
    pub knn_k: usize,
    pub cos_floor: f64,
    pub s_telos: f64,
    pub s_ref: f64,
    pub s_central: f64,
    pub resolution: f64,
}

impl Lens {
    pub fn telos_default() -> Self {
        Lens {
            w_express: 1.0,
            w_contains: 1.0,
            w_leads_to: 0.6,
            w_near: 0.3,
            w_prop: 0.4,
            w_cos: 0.0,   // the cogmap regime: declared graph only
            knn_k: 12,
            cos_floor: 0.55,
            s_telos: 0.5,
            s_ref: 0.3,
            s_central: 0.2,
            resolution: 0.5,
        }
    }

    /// The context regime (spec §3.2). MUST mirror the seeded `workflow-default` row.
    ///
    /// Note what is NOT zeroed: `w_express`, `w_contains`, and `w_prop` are held at cogmap parity
    /// even though contexts carry zero facets and almost no express/contains edges today. A lens
    /// weight is a rate of exchange — what a signal is WORTH when present — not a prior on how often
    /// it appears. Sparsity already handles itself: a pair with no express edge contributes zero from
    /// that term regardless of the weight. And an express edge asserted mid-session is MORE evidential
    /// than one a steward asserts as its job, because the rarity is what makes it informative.
    ///
    /// The binding reason is a feedback loop: a weight of 0.0 makes the discipline provably
    /// unrewarded, and an information system that returns no signal for signal provided gets routed
    /// around. If asserting a facet in a context visibly tightens a region, the discipline pays for
    /// itself — and that is the only mechanism by which contexts ever BECOME better-structured.
    pub fn workflow_default() -> Self {
        Lens {
            w_express: 1.0,     // parity — deliberate, rare, high-information
            w_contains: 1.0,    // parity
            w_prop: 0.4,        // parity
            w_leads_to: 0.9,    // `advances` — cheap to create, but it IS the hub topology (§3.3)
            w_near: 0.35,       // `relates_to` — cheapest, most abundant. Real but weak.
            w_cos: 1.0,         // the regime switch: inferred similarity is PRIMARY here
            knn_k: 12,
            cos_floor: 0.55,
            s_telos: 0.6,
            s_ref: 0.15,        // contexts have shallower provenance depth than distilled nodes
            s_central: 0.25,
            resolution: 0.5,
        }
    }
    // w_kind unchanged
}

/// Symmetric affinity (spec §3.1). Three additive terms:
///   - declared edges, lens-weighted by kind          (weak supervision in a context)
///   - facet overlap, lens-weighted                   (weak supervision in a context)
///   - sparse exact-kNN cosine, lens-weighted         (PRIMARY in a context; zero in a cogmap)
///
/// Labels are not reserved: every label is ordinary positive relatedness, so contradiction BINDS
/// (shared frame), never separates.
pub fn affinity(
    a: ResourceId,
    b: ResourceId,
    edges: &[Edge],
    facets: &[Facet],
    knn: &KnnGraph,
    lens: &Lens,
) -> f64 {
    let edge_sum: f64 = edges
        .iter()
        .filter(|e| (e.src == a && e.tgt == b) || (e.src == b && e.tgt == a))
        .filter(|e| !e.weight.is_nan())
        .map(|e| lens.w_kind(e.kind) * e.weight)
        .sum();
    edge_sum
        + lens.w_prop * facet_overlap(a, b, facets)
        + lens.w_cos * knn.sim(a, b)
}
```

- [ ] **Step 5: Carry pooled embeddings and the kNN graph into `Substrate`**

In `substrate.rs`, add to `load()` — **skip the query entirely when `w_cos == 0.0`**, so the cogmap path pays nothing:

```rust
    // Pooled per-resource embeddings — the same pool-per-concept-then-mean the centroid readout uses
    // (write.rs populate_readouts), so formation and readout agree on what a resource's vector IS.
    // Skipped entirely when the lens is declared-only: a cogmap must not pay for a signal it ignores.
    let knn = if lens.w_cos == 0.0 {
        KnnGraph::default()
    } else {
        let rows = sqlx::query!(
            "SELECT ch.resource_id, avg(ch.embedding)::text AS \"vec!\" \
               FROM kb_chunks ch \
               JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded \
              WHERE ch.is_current AND ch.resource_id = ANY($1) \
              GROUP BY ch.resource_id",
            &nodes as &[ResourceId],
        )
        .fetch_all(pool)
        .await?;
        let embs: HashMap<ResourceId, Vec<f32>> = rows
            .into_iter()
            .filter_map(|r| parse_pgvector(&r.vec).map(|v| (ResourceId::from(r.resource_id), v)))
            .collect();
        knn::build(&embs, lens.knn_k, lens.cos_floor)
    };
```

> `avg(vector)` returns a `vector`, which sqlx does not map natively. Two options — **check which the codebase already uses** before choosing:
> ```bash
> rg -n "pgvector|::text.*vector|Vector<" crates/temper-substrate/src/ crates/temper-services/src/ | head
> ```
> If a `pgvector` Rust type is already a dependency, bind it directly. If the codebase round-trips through `::text` anywhere, follow that. Do not introduce a third convention.

Add `pub knn: KnnGraph` to `struct Substrate`, and thread `&sub.knn` through every `affinity(...)` call site in `cluster.rs` and `write.rs`. The compiler will find them all.

- [ ] **Step 6: Run the kernel unit tests**

```bash
cargo nextest run -p temper-substrate knn affinity
```

Expected: PASS.

- [ ] **Step 7: Seed the `workflow-default` lens**

Create `migrations/20260712000050_workflow_default_lens.sql`. Mirror how `telos-default` is seeded in `20260624000003_canonical_seed.sql:68` (it goes through `lens_create(...)` — print it with `\sf lens_create` and check whether that function needs widening for the new columns first; if it does, widen it here).

```sql
-- The context regime lens (spec §3.2). Global (home_anchor_table IS NULL), so every context picks it
-- up without per-context authoring.
--
-- Everything DELIBERATE is at cogmap parity — w_express, w_contains, w_prop are NOT zeroed despite
-- contexts carrying zero facets today. See the COMMENT on kb_cogmap_lenses.w_prop and spec §3.2.
INSERT INTO kb_cogmap_lenses (
    id, home_anchor_table, home_anchor_id, cogmap_id, name, selection_kind,
    w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor,
    s_telos, s_ref, s_central, resolution,
    kappa_anchor_prior, telos_halflife_days,
    sw_in_progress, sw_backlog, sw_done, damper_paused, damper_completed,
    asserted_by_event_id
)
SELECT uuid_generate_v7(), NULL, NULL, NULL, 'workflow-default', 'homed',
       1.0, 1.0, 0.9, 0.35, 0.4, 1.0, 12, 0.55,
       0.6, 0.15, 0.25, 0.5,
       0.6, 30.0,
       1.0, 0.35, 0.15, 0.3, 0.4,
       e.id
  FROM kb_events e ORDER BY e.id LIMIT 1;
```

> `asserted_by_event_id` is `NOT NULL REFERENCES kb_events(id)`. The `SELECT ... FROM kb_events LIMIT 1` above is a placeholder. **Check how `20260624000003_canonical_seed.sql` obtains its event id** — it almost certainly appends a real `lens_asserted` event through `lens_create`. Use that path; a lens is a declared, event-sourced artifact, and short-circuiting the ledger here would be exactly the kind of quiet inconsistency the event model exists to prevent.

- [ ] **Step 8: RE-ASSERT THE REGRESSION FLOOR**

```bash
cargo make test-artifacts
```

Expected: **every cogmap fixture identical.** `w_cos = 0.0` on every pre-existing lens row means the new term is multiplied by zero and the `KnnGraph` is never even built. If a single cogmap fingerprint moved, the kernel change leaked into the declared-only path — stop and find it. Do not adjust a fixture.

- [ ] **Step 9: Prove the kernel does something on real data**

Write `crates/temper-substrate/tests/context_region_smoke.rs`:

```rust
#![cfg(feature = "artifact-tests")]

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;

/// The context regime forms non-degenerate regions. With the declared-only kernel, a context
/// clusters into all-singletons (zero facets, near-monotone edges). With w_cos it must not — and it
/// must not collapse into one blob either. Both failure modes are the ones the sparse-kNN
/// construction exists to prevent.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_forms_more_than_one_region_and_fewer_than_n(pool: PgPool) -> sqlx::Result<()> {
    let ctx = /* load a context fixture with >= 20 embedded resources */;
    let emitter = /* system emitter */;

    let n_resources: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes WHERE anchor_table='kb_contexts' AND anchor_id=$1",
    )
    .bind(ctx.uuid())
    .fetch_one(&pool)
    .await?;

    temper_substrate::write::materialize(
        &pool, HomeAnchor::Context(ctx), "workflow-default", emitter,
    )
    .await
    .expect("materialize");

    let n_regions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(ctx.uuid())
    .fetch_one(&pool)
    .await?;

    assert!(n_regions > 1, "not one blob");
    assert!(n_regions < n_resources, "not all singletons — this is what w_cos is FOR");
    Ok(())
}
```

You will need a context fixture with real embeddings. **T9 builds the `ContextDef` scenario DSL properly**; for this smoke test, either reuse an existing seed's resources re-homed into a context, or build the fixture inline. Note in the test which you did and why, so T9 knows what to replace.

- [ ] **Step 10: Full suite, caches, check**

```bash
cargo make test-artifacts
cargo nextest run -p temper-substrate
cargo make test-db
cargo make test-e2e
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make check
```

- [ ] **Step 11: Commit**

```bash
git add crates/temper-substrate/src/{knn.rs,affinity.rs,substrate.rs,lib.rs} \
        crates/temper-substrate/tests/context_region_smoke.rs \
        migrations/20260712000050_workflow_default_lens.sql \
        .sqlx crates/*/.sqlx
git commit -m "feat(substrate): the w_cos kernel — sparse exact-kNN affinity for the context regime

In a cogmap the declared graph is primary and the embedding is a readout. In a
context the embedding is PRIMARY and the declared graph is weak supervision —
prod carries 1,643 context-homed resources with ZERO facets, so the declared-only
kernel clusters a context into all-singletons.

affinity() gains a third additive term: w_cos * knn_sim(a,b), where knn_sim is a
SPARSE exact-kNN graph, not a raw cosine. Cosine is dense — a raw term would make
the affinity graph complete and connected_components useless. Exact, never HNSW:
an approximate index is not reproducible, and membership_fingerprint depends on
formation being deterministic.

The workflow-default lens holds w_express / w_contains / w_prop at COGMAP PARITY.
A lens weight is meaning-when-present, not a frequency prior — zeroing them would
make the discipline provably unrewarded, and an information system that returns no
signal for signal provided gets routed around.

Regression floor holds: w_cos = 0.0 on every pre-existing lens, every cogmap
fixture fingerprint byte-identical."
```

---

## Self-Review

**Spec coverage (this plan's scope, T1–T4):**

| Spec § | Covered by |
|---|---|
| §3.1 one producer / two regimes; sparse exact-kNN | T4 steps 3–6 |
| §3.2 lens weights = meaning-when-present | T4 step 4 (`workflow_default`) + T4 step 1 (test) + T2 (COMMENT) |
| §3.3 goals as hub topology | T4 (`w_leads_to = 0.9`); the mutual-OR kNN symmetrization in `knn::build` is what keeps a hub from being severed by its own k limit |
| §3.6 M1 additive schema | T2 |
| §3.6 M2 code on the anchor pair | T3 |
| §3.8 authz prerequisites | T1 |
| §3.9.1 member affinity never written | T3 step 5 |
| §3.9.2 centrality home-filter | T3 step 7 |
| §5 regression floor | T3 step 2/8, T4 step 8 |
| §7 scale ceiling documented in code | T4 step 3 (`knn.rs` module doc) |

**Deferred to Plans 2–3 (correctly out of scope here):** §3.4 telos/liveness (T5), §3.5 two clocks (T6), §3.7 wayfind + orientation reads (T7, T8), §5 scenario DSL (T9).

**Known unverified assumptions, flagged inline rather than hidden:** the `MaterializeOutcome` field names, the scenario-fixture loader helpers, the `lens_create` signature, the `kb_access_grants` column names, and the pgvector round-trip convention. Each has a `rg`/`psql` command attached that resolves it before the code is written. **Do not guess past these — resolve them.**

**Type consistency:** `HomeAnchor` is the single anchor type end to end (T3 → T4). `affinity()` takes `&KnnGraph` in every call site after T4 step 4. `Lens.knn_k` is `usize` in Rust and `INT` in SQL — cast at the boundary in `load()`.

---

## Execution Handoff

Plan complete. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute in this session with checkpoints for review.

Note that T1 and T2 are **independent** and can run in parallel. T3 depends on T2; T4 depends on T3.
