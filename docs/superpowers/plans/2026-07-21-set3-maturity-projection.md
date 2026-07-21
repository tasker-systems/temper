# Set 3 — Evidential-Standing Maturity Projection — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Materialize a per-resource *evidential standing* — the shape vector `(independence-discounted-breadth, adversarial-survival, contradiction-balance, freshness)` — over `kb_block_provenance` + `kb_edges`, memoized as component scalars and read as a shape-plus-lossy-band chip, never as a stored band.

**Architecture:** Two net-new projection tables (`kb_resource_standing` component-memo, `kb_independence_pairs` flattened pairwise-independence memo), a set of STABLE SQL producer functions that model on the existing `cogmap_region_*` / `resource_blocks` reinforcement roll-ups, an application-orchestrated "standing clock" (Rust, on the provenance/edge write path — the repo has **no** recompute-on-write DB triggers and **no** materialized views) that refreshes the memo on drift, and a read-time band function computed from the memoized components (the wayfind `NULL→memoized / non-null→recompute` precedent). The shape surfaces through a dedicated `standing` read (API + CLI), modelled on the existing `edge_service` — not by bloating every `ResourceDetail`.

**Tech Stack:** PostgreSQL (PG17 Neon + PG18 local — version-portable SQL only), sqlx migrations + macros, Rust (`temper-substrate` write/readback, `temper-services` backend, `temper-core` wire types with ts-rs/utoipa/schemars derives), `temper-api` Axum handler, `temper-cli` clap command.

## Global Constraints

Copied verbatim from spec `019f81e8` (`docs/superpowers/specs/2026-07-20-evidential-standing-substrate-breakdown-and-lead-seams-design.md`) and the repo CLAUDE.md. **Every task implicitly includes these.**

- **Standing is not truth.** "Standing is not truth, and the system cannot close the gap between them — only make its shape visible." The maturity axis measures *defensibility on present evidence*, never *is-this-claim-true*. (spec, Bedrock preamble.)
- **Standing IS the vector**, band is a lossy read-time chip: "Any band label … is a **lossy read-time summary** over that shape, always presented *with* the shape, **never instead of it.**" (spec §1.1.) **NEVER store a band/maturity enum on `kb_resources`** (spec §1.3 AMEND).
- **Two orthogonal axes never collapse**: maturity/corroboration (provisional→reinforced→near-canonical) vs validity/health (live→scarred→retired). (spec §1.2.)
- **Recursion cut at depth 1**: `R_indep` is a *leaf tally*, "cheap to read, **not** a recursive standing computation." (spec §2.1.)
- **Conservative silence default**: an unasserted pair is **NOT independent** (assumed to share a cause) until an explicit `independent-of` edge proves otherwise. (spec §2.4.)
- **Additive-only-on-`main`**: no editing shipped migrations (checksum-locked); extend a shipped function via a **new** DROP+CREATE migration. (CLAUDE.md.) Projection tables are recomputable and **NOT** append-only-guarded (only event/standing *logs* are).
- **Version-portable SQL**: runs on PG17 (Neon) and PG18 (local/CI); no version-specific features.
- **After changing SQL**: `cargo sqlx prepare --workspace -- --all-features`; test-target queries also need `cargo make prepare-services` / `cargo make prepare-e2e`.
- **Subject of standing = any `kb_resource`.** `finding_id` is a `kb_resources.id`. Set 3 is buildable over existing resources+provenance+edges with **no** findings-board (Set 2) dependency.
- **Scar-emission is deferred to Set 5.** Set 3 *reads* live edge/provenance state only. It builds **no** scar writer and does **not** touch the `kb_edges` source/target CHECK. `is_scarred` / `is_corrected` are read-filters that light up when Set 5 (adversary) and a future scar path emit.

---

## Grounding dossier (verified 2026-07-21 against current disk)

Every "conform to X" below cites a real object. The spec's day-old citations were revalidated; **three spec claims were falsified** and are handled as AMEND (see task tags). Implementers: these are the *only* pre-grounded facts. Anything not cited here, verify on disk before use.

**G1 — provenance accretion + the incumbent reinforcement tally.**
- `kb_block_provenance` DDL: `migrations/20260624000001_canonical_schema.sql:599-615`. Columns: `block_id → kb_content_blocks(id)`, `source_kind provenance_source_kind` (`'event'|'resource'|'remote'`), `source_id`, `contributed_by_event_id`, `accretion_seq INT`, `is_corrected BOOLEAN DEFAULT false`, `created`. UNIQUE is `(block_id, source_kind, source_id, contributed_by_event_id)` — **not** on `accretion_seq`; monotonicity is caller-supplied, not DB-enforced.
- **`is_corrected` has NO writer anywhere** — it is a read-filter that is always `false` today. (Full-repo grep.) The scar *writer* is out of Set 3 scope.
- **Incumbent R_parent tally already exists** — model on it, do not reinvent counting:
  - `resource_blocks(...)` per-block: `count(pr.id) FILTER (WHERE NOT pr.is_corrected)` — `migrations/20260624000002_canonical_functions.sql:371-389`.
  - `cogmap_region_reference_standing(p_region)` region roll-up: `count(p.*)` over member blocks' provenance `WHERE NOT p.is_corrected` — `migrations/20260624000002_canonical_functions.sql:474-483`.
- Blocks: `kb_content_blocks` (note the name), `resource_id → kb_resources(id)`, `UNIQUE(resource_id, seq)`, `is_folded` — `migrations/20260624000001_canonical_schema.sql:541-555`. Provenance rolls up to a resource **only** through `kb_content_blocks.resource_id`.

**G2 — edges + the label pattern.**
- `edge_kind` closed enum `('express','contains','leads_to','near')`, `edge_polarity ('forward','inverse')` — `migrations/20260624000001_canonical_schema.sql:87,89-95`.
- `kb_edges` DDL `migrations/20260624000001_canonical_schema.sql:628-650`: `source_table CHECK IN ('kb_resources','kb_cogmaps')`, `source_id`, `target_table` (same CHECK), `target_id`, `edge_kind`, `polarity DEFAULT 'forward'`, `label TEXT`, `weight DOUBLE PRECISION DEFAULT 1.0`, `home_anchor_table CHECK IN ('kb_contexts','kb_cogmaps')`, `is_folded`. Active-edge uniqueness: `(source_table, source_id, target_table, target_id, edge_kind, COALESCE(label,''), home_anchor_table, home_anchor_id) WHERE NOT is_folded`.
- **AMEND (spec §2.2 falsified)**: `AnchorTable` includes `kb_edges` in the *wire* schema (`migrations/20260624000003_canonical_seed.sql:35`) **but the `kb_edges` table CHECK forbids an edge as source/target** — meta-edges are **not persistable**. The spec's "meta-edge onto the independence edge, no new machinery" does not exist. Set 3 does not build it (deferred to Set 5).
- **Edges scar via `is_folded`, not `is_corrected`** (`is_corrected` is not a `kb_edges` column).
- `contradicts` is a **label** matched via `e.label = ANY(p_opposed_labels)` (default `{'contradicts'}`) — `cogmap_region_internal_tension(...)` `migrations/20260624000002_canonical_functions.sql:505-521`. **This is the contradiction-term precedent.** `polarity='inverse'` is **not** associated with `contradicts` anywhere (spec's inverse framing is new usage, not a precedent).
- Edge writers: `relationship_assert(p_payload, p_emitter, ...)` creates an express edge with label+weight (`migrations/20260624000002_canonical_functions.sql:823-833`); `relationship_fold(...)` sets `is_folded=true` (`:838-867`); `relationship_reweight` `:1210`; `relationship_retype` `:1186`. **`fold_relationship` does not exist** (it is `relationship_fold`). `relationship_corrected` event type is seeded but has **no projector**.

**G3 — the memoize/refresh house style (spec §2.3/§1.3 mechanism falsified).**
- **Zero `CREATE MATERIALIZED VIEW` and zero recompute-on-write DB triggers in the repo.** Triggers are only append-only guards + membership-sync.
- Memo = **plain columns** on the base table: `kb_cogmap_regions.salience` (memoized blend) + its stored component columns `telos_alignment, reference_standing, centrality` — `migrations/20260624000001_canonical_schema.sql:725-738`.
- Refresh = **Rust clock on the write path**, gated by a SQL drift function vs a snapshot column:
  - Orchestrator `crates/temper-services/src/backend/region_clocks.rs:78-82` (`DbBackend::tick_region_clocks`, "fired inline on every resource write", errors swallowed).
  - Drift gate SQL `anchor_telos_drift(...)` `migrations/20260712000070_two_clocks.sql:147-172`, epsilon in `kb_cogmap_lenses.telos_drift_epsilon`.
  - Cheap refresh `crates/temper-substrate/src/write.rs:851-916` (`refresh_salience`): recompute drifted component column → re-blend memo from stored components → re-snapshot baseline + fire event. Parity with full materialize pinned by `crates/temper-substrate/tests/context_two_clocks.rs:112`.
- Read-time recompute-from-components: wayfind `migrations/20260629000007_wayfind_scope.sql:41-49` — `CASE WHEN p_lens IS NULL THEN r.salience ELSE <re-blend from component columns> END`.

**G4 — read surface + type home (maturity projection is GREENFIELD).**
- No `maturity`/`corroborat`/`StandingShape`/`contradiction_balance` in source. `reinforce_count`, principal-admission `Standing` enum (`crates/temper-principal/src/standing.rs:10` — **different axis, do not collide**), and `reference_standing` salience component are all that match.
- Resource read view: `readback::resource_row → ResourceRowParity` (substrate-local, `crates/temper-substrate/src/readback/mod.rs:304,368`) → `ResourceDetail` wire type (`crates/temper-workflow/src/types/resource.rs:187`). Handler `crates/temper-api/src/handlers/resources.rs:102`. **No standing field today.**
- Edges are a **separate** service call: `edge_service::list_resource_edges` (`crates/temper-services/src/services/edge_service.rs:29`) behind `GET /api/resources/{id}/edges`. **Model the standing read on this.**
- Closest analog to a standing shape vector: `CogmapRegionMetricsRow` (`crates/temper-core/src/types/cognitive_maps.rs`) — a 5-scalar derived vector with the full derive stack (ts-rs+utoipa+schemars+serde+FromRow). **This is the type-home template.** Substrate can't depend on core → substrate-local row struct + core wire type.

**G5 — migration + test conventions.**
- Next slot: `20260721000010_*.sql` (highest on disk is `20260720000100_is_system_admin_nonempty_gating_slug.sql`; a fresh date resets the sequence). Leave a gap for sibling sessions.
- Migration tests: `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` under the `artifact-tests` feature, seed + assert against an ephemeral `public`-schema DB (`crates/temper-substrate/tests/access_scenario.rs:29-32`). Run with `cargo make test-artifacts`.
- `MIGRATOR` = `sqlx::migrate!("../../migrations")` (`crates/temper-substrate/src/lib.rs:35`).

---

## File structure

**Create:**
- `migrations/20260721000010_evidential_standing_memo.sql` — both memo tables + all SQL producers + the read/band functions.
- `crates/temper-substrate/src/write.rs` additions — `refresh_resource_standing`, `refresh_independence_pairs` wrappers (same file as `refresh_salience`).
- `crates/temper-substrate/src/backend/standing_clock.rs` **or** additions to `region_clocks.rs` — the standing clock tick. (Decision in Task 6.)
- `crates/temper-substrate/src/readback/mod.rs` additions — `resource_standing(...)` producer + `StandingShapeRow`.
- `crates/temper-core/src/types/standing.rs` — `StandingShape` wire type (+ register in `types/mod.rs`).
- `crates/temper-services/src/services/standing_service.rs` — `resource_standing(...)` service (model on `edge_service.rs`).
- `crates/temper-api/src/handlers/resources.rs` additions — `GET /api/resources/{id}/standing` handler.
- `crates/temper-cli/src/actions/standing.rs` + `crates/temper-cli/src/commands/` wiring — `temper resource standing <ref>`.
- Tests: `crates/temper-substrate/tests/evidential_standing.rs` (artifact-tests); `tests/e2e/tests/resource_standing_test.rs`.

**Modify:** `crates/temper-substrate/src/lib.rs` / `readback/mod.rs` exports; `crates/temper-core/src/types/mod.rs`; `crates/temper-api` router; `crates/temper-cli` command tree; generated `.sqlx` caches; generated ts-rs types.

---

## Phase A — SQL substrate (Tasks 1–4)

All Phase A work lands in one migration file `migrations/20260721000010_evidential_standing_memo.sql`, built up task by task. Each task adds its objects and an `artifact-tests` test. Tag: **EXTEND** unless noted (the projection concept is greenfield, authorized by spec §"maturity as a materialized projection" + §1.1/§2.x).

### Task 1: Component-memo table + `r_parent` and `freshness` producers

**Files:**
- Create: `migrations/20260721000010_evidential_standing_memo.sql`
- Test: `crates/temper-substrate/tests/evidential_standing.rs`

**Interfaces:**
- Produces: table `kb_resource_standing(finding_id PK, indep_breadth, adversarial_survival, challenge_count, contradiction_balance, freshness, r_parent, refreshed_event_id, updated)`; `resource_r_parent(uuid) → double precision`; `resource_freshness(uuid) → double precision`.

**Design tags:**
- `kb_resource_standing` table — **EXTEND** (spec §1.3: memoize components, band at read). Projection table, **not** append-only-guarded (G3).
- `resource_r_parent` — **CONFORM** to the incumbent tally `cogmap_region_reference_standing` (`migrations/20260624000002_canonical_functions.sql:474-483`): `count(...) FILTER (WHERE NOT is_corrected)` over the finding's non-folded blocks' provenance. Do not invent a new counting rule.

- [ ] **Step 1: Write the failing test** — `crates/temper-substrate/tests/evidential_standing.rs`

```rust
#![cfg(feature = "artifact-tests")]
mod common;
use sqlx::Row;

// A resource with N provenance rows across its blocks has r_parent = N (is_corrected excluded),
// mirroring the incumbent cogmap_region_reference_standing count.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn r_parent_counts_uncorrected_provenance(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    // ⚠️ Plan/reality gap: the exact seed helpers for a resource + blocks + provenance are in
    // crates/temper-substrate/tests/common/ and tests/content_mutation.rs — VERIFY their real
    // signatures on disk (e.g. how content_mutation.rs seeds a resource with provenance) and reuse
    // them; do not hand-roll INSERTs. Assert resource_r_parent(finding) equals the seeded count.
    let finding = common::seed_resource_with_provenance(&pool, 3).await; // <-- confirm real helper
    let r: f64 = sqlx::query_scalar("SELECT resource_r_parent($1)")
        .bind(finding).fetch_one(&pool).await.unwrap();
    assert_eq!(r, 3.0, "r_parent counts uncorrected provenance rows over the finding's blocks");
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo make test-artifacts` (or `cargo nextest run -p temper-substrate --features artifact-tests r_parent_counts_uncorrected_provenance`)
Expected: FAIL — `function resource_r_parent(uuid) does not exist`.

- [ ] **Step 3: Add the table + producers to the migration.** Write into `migrations/20260721000010_evidential_standing_memo.sql`:

```sql
-- Evidential-standing component-memo (spec 019f81e8 §1.3 AMEND: memoize components,
-- compute band/shape AT READ; NEVER a stored band on kb_resources). Projection table
-- => recomputable, NOT append-only-guarded (only logs are; cf 20260720000040).
CREATE TABLE kb_resource_standing (
    finding_id            UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    indep_breadth         DOUBLE PRECISION NOT NULL DEFAULT 0,   -- independence-discounted breadth (§2.1)
    adversarial_survival  DOUBLE PRECISION NOT NULL DEFAULT 0,   -- N withstood; 0 = no challenges yet (§1)
    challenge_count       INT              NOT NULL DEFAULT 0,   -- distinguishes 0-challenges from N-withstood
    contradiction_balance DOUBLE PRECISION NOT NULL DEFAULT 0,   -- supports − contradicts, vector-sum (§1)
    freshness             DOUBLE PRECISION NOT NULL DEFAULT 0,   -- reversible decay off R_parent recency
    r_parent              DOUBLE PRECISION NOT NULL DEFAULT 0,   -- breadth term (reinforce_count)
    refreshed_event_id    UUID REFERENCES kb_events(id),         -- watermark of last refresh
    updated               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- R_parent: reinforcement breadth of the finding. CONFORM to cogmap_region_reference_standing
-- (canonical_functions.sql:474-483) — count of uncorrected provenance over the finding's live blocks.
CREATE FUNCTION resource_r_parent(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(count(p.*), 0)::double precision
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_finding AND NOT b.is_folded;
$$;

-- Freshness: reversible fade clocked off R_parent recency (spec §1 decay, §2.1). Half-life form,
-- returns 1.0 at "just reinforced" decaying toward 0. FRESHNESS_HALFLIFE_DAYS is a tunable default.
CREATE FUNCTION resource_freshness(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH last AS (
        SELECT max(p.created) AS at
        FROM kb_content_blocks b
        JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
        WHERE b.resource_id = p_finding AND NOT b.is_folded
    )
    SELECT CASE WHEN (SELECT at FROM last) IS NULL THEN 0.0
                ELSE pow(0.5, extract(epoch FROM (now() - (SELECT at FROM last))) / (30.0 * 86400.0))
           END::double precision;  -- 30-day half-life; TUNE in Task 4's surfacing pass
$$;
```

> Implementer note (GD-2): after writing, run the two functions against a seeded DB and paste the output into the PR — executed grounding, not just "it compiles."

- [ ] **Step 4: Run the test, verify it passes.** Run: `cargo make test-artifacts`. Expected: PASS.

- [ ] **Step 5: Regenerate sqlx cache + commit.**

```bash
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260721000010_evidential_standing_memo.sql crates/temper-substrate/tests/evidential_standing.rs .sqlx
git commit -m "feat(standing): kb_resource_standing memo + r_parent/freshness producers (Set 3 Task 1)"
```

### Task 2: `kb_independence_pairs` memo + `refresh_independence_pairs` + independence-breadth producer

**Files:** Modify `migrations/20260721000010_evidential_standing_memo.sql`; add tests to `evidential_standing.rs`.

**Interfaces:**
- Produces: table `kb_independence_pairs(finding_id, base_a, base_b, weight, is_scarred, edge_id, PK(finding_id, base_a, base_b))`; `resource_bases(uuid) → TABLE(source_kind, source_id)`; `refresh_independence_pairs(uuid) → void`; `resource_independence_breadth(uuid) → double precision`.

**Design tags:**
- Table + refresh — **EXTEND** (spec §2.3: flattened memo, own DDL). `is_scarred` derives from the `independent-of` edge being folded/superseded — Set 3 **reads** it (scar writer is Set 5).
- Silence default — **CONFORM to spec §2.4 invariant (quote it in the code comment)**: "an unasserted pair is **NOT independent** … until an explicit `independent-of` edge proves otherwise." Breadth rises **only** on affirmative independence.
- Base definition — **EXTEND**: a base of finding F = a distinct `(source_kind='resource', source_id)` in `kb_block_provenance` over F's non-folded blocks (the evidentiary resource-contributors). An independence pair = an active `express` edge `label='independent-of'` whose two endpoints are both bases of F. (Endpoints are resources per G2.)

- [ ] **Step 1: Write two failing tests** in `evidential_standing.rs`:

```rust
// (a) SILENCE = CORRELATED: with bases but no independent-of edge, breadth reflects a single
// effective independent cluster (not N). Breadth must NOT rise from mere multiplicity.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn silence_default_is_correlated(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let finding = common::seed_resource_with_resource_bases(&pool, 3).await; // 3 resource-bases, no indep edges
    sqlx::query("SELECT refresh_independence_pairs($1)").bind(finding).execute(&pool).await.unwrap();
    let breadth: f64 = sqlx::query_scalar("SELECT resource_independence_breadth($1)")
        .bind(finding).fetch_one(&pool).await.unwrap();
    assert!(breadth <= 1.0, "silence default: 3 unasserted bases count as one correlated cluster, not 3");
}

// (b) AFFIRMATIVE INDEPENDENCE raises breadth: assert one independent-of edge between two bases.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn affirmed_independence_raises_breadth(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let (finding, base_a, base_b, _c) = common::seed_finding_three_bases(&pool).await;
    // ⚠️ Plan/reality gap: build the edge via relationship_assert (canonical_functions.sql:823) with
    // edge_kind='express', label='independent-of', endpoints base_a/base_b, home = the finding's context.
    // VERIFY relationship_assert's real payload shape on disk before constructing it.
    common::assert_independent_of(&pool, finding, base_a, base_b).await;
    sqlx::query("SELECT refresh_independence_pairs($1)").bind(finding).execute(&pool).await.unwrap();
    let breadth: f64 = sqlx::query_scalar("SELECT resource_independence_breadth($1)")
        .bind(finding).fetch_one(&pool).await.unwrap();
    assert!(breadth > 1.0, "one affirmed independent pair raises effective independent rank above 1");
}
```

- [ ] **Step 2: Run, verify both fail** (`function ... does not exist`). Run: `cargo make test-artifacts`.

- [ ] **Step 3: Add the objects.** Append to the migration:

```sql
-- Flattened pairwise-independence memo (spec §2.3). One row per affirmatively-asserted
-- independent-of pair among a finding's evidentiary bases. Silence default (§2.4): a pair
-- with NO row is NOT independent (assumed correlated) — breadth rises only on affirmation.
-- is_scarred = the underlying independent-of edge was folded/superseded (§2.1). Set 3 READS
-- this; the scar WRITER is Set 5.
CREATE TABLE kb_independence_pairs (
    finding_id  UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    base_a      UUID NOT NULL,                              -- resource-base; canonical order base_a < base_b
    base_b      UUID NOT NULL,
    weight      DOUBLE PRECISION NOT NULL DEFAULT 1.0,      -- independence estimate × R_indep
    is_scarred  BOOLEAN NOT NULL DEFAULT false,
    edge_id     UUID NOT NULL REFERENCES kb_edges(id),
    PRIMARY KEY (finding_id, base_a, base_b)
);
CREATE INDEX idx_kb_independence_pairs_finding ON kb_independence_pairs(finding_id) WHERE NOT is_scarred;

-- Evidentiary bases of a finding = distinct resource-source provenance over its live blocks.
CREATE FUNCTION resource_bases(p_finding uuid)
RETURNS TABLE(source_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT p.source_id
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_finding AND NOT b.is_folded
      AND p.source_kind = 'resource';
$$;
```

Then `refresh_independence_pairs` and `resource_independence_breadth`. **These are the novel core of Set 3 — do not treat the sketch below as a spec.** Model the rebuild on how `refresh_salience` (`crates/temper-substrate/src/write.rs:851-916`) recomputes a region's members, and quote spec §2.1/§2.3/§2.4 in the body comments. Required shape:

```sql
-- Rebuild the finding's independence memo from live independent-of edges among its bases.
-- Model: region member recompute. is_scarred := the edge is folded (superseded). No recursion.
CREATE FUNCTION refresh_independence_pairs(p_finding uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM kb_independence_pairs WHERE finding_id = p_finding;
    INSERT INTO kb_independence_pairs (finding_id, base_a, base_b, weight, is_scarred, edge_id)
    SELECT p_finding,
           least(e.source_id, e.target_id), greatest(e.source_id, e.target_id),
           e.weight, e.is_folded, e.id
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND e.label = 'independent-of'
      AND e.source_id IN (SELECT source_id FROM resource_bases(p_finding))
      AND e.target_id IN (SELECT source_id FROM resource_bases(p_finding))
    ON CONFLICT (finding_id, base_a, base_b) DO UPDATE
      SET weight = EXCLUDED.weight, is_scarred = EXCLUDED.is_scarred, edge_id = EXCLUDED.edge_id;
END;
$$;

-- Independence-discounted breadth (§2.1 terminating leaf-tally, no recursion; §2.4 silence=correlated).
-- Effective independent rank ≈ 1 (the base correlated cluster) + Σ non-scarred pair weights.
-- The exact aggregation is the tuning target of Task 4; keep it TERMINATING and monotone in
-- affirmed, non-scarred independence.
CREATE FUNCTION resource_independence_breadth(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN EXISTS (SELECT 1 FROM resource_bases(p_finding)) THEN 1.0 ELSE 0.0 END
         + coalesce((SELECT sum(weight) FROM kb_independence_pairs
                     WHERE finding_id = p_finding AND NOT is_scarred), 0.0);
$$;
```

> ⚠️ **Plan/reality gap for the implementer**: `refresh_independence_pairs` reads `kb_edges` directly. Confirm on disk that no access gate is required here (this is a memo rebuild, not a user-facing read; the *read* path in Task 4 carries the gate). Confirm `least/greatest` on `uuid` is valid on PG17+18 (it is — comparison operators exist for uuid), else canonicalize in a subquery.

- [ ] **Step 4: Run, verify both pass.** Run: `cargo make test-artifacts`.
- [ ] **Step 5: Prepare + commit.**

```bash
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260721000010_evidential_standing_memo.sql crates/temper-substrate/tests/evidential_standing.rs .sqlx
git commit -m "feat(standing): independence_pairs memo + silence-default breadth (Set 3 Task 2)"
```

### Task 3: `contradiction_balance` + `adversarial_survival` producers

**Files:** Modify the migration; add tests.

**Interfaces:**
- Produces: `resource_contradiction_balance(uuid) → double precision`; `resource_adversarial_survival(uuid) → TABLE(challenge_count int, survived double precision)`.

**Design tags:**
- Contradiction — **CONFORM** to the label-match pattern of `cogmap_region_internal_tension` (`canonical_functions.sql:505-521`), resource-scoped: vector-sum `Σ supports − Σ contradicts` over `express` edges incident to the finding. Quote spec §1 "vector-sum … 5 supports + 4 contradicts is a live concern".
- Adversarial-survival — **EXTEND** (spec §1: distinguish 0 challenges from N withstood). The challenge/survived edge label vocabulary is **Set 5's** to define; Task 3 builds the reader against a *provisional* label set and returns `challenge_count=0` until Set 5 emits. **Do not** invent Set 5's vocabulary as canonical — use clearly-named placeholder labels and comment that Set 5 finalizes them.

- [ ] **Step 1: Failing tests:**

```rust
// contradiction_balance is a vector-sum: 2 supports − 1 contradicts = +1.0 (weights 1.0).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn contradiction_balance_is_vector_sum(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let finding = common::seed_finding_with_edges(&pool, /*supports*/ 2, /*contradicts*/ 1).await;
    let bal: f64 = sqlx::query_scalar("SELECT resource_contradiction_balance($1)")
        .bind(finding).fetch_one(&pool).await.unwrap();
    assert_eq!(bal, 1.0, "2 supports − 1 contradicts = +1.0");
}
// no adversary yet ⇒ challenge_count = 0 (absence of challenge is NOT survival).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn zero_challenges_is_not_survival(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let finding = common::seed_resource_with_provenance(&pool, 1).await;
    let row = sqlx::query("SELECT challenge_count, survived FROM resource_adversarial_survival($1)")
        .bind(finding).fetch_one(&pool).await.unwrap();
    let c: i32 = row.get("challenge_count"); let s: f64 = row.get("survived");
    assert_eq!((c, s), (0, 0.0), "no challenges: 0 count, 0 survival — distinct from N-withstood");
}
```

- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Add producers.** Append to the migration (model on `cogmap_region_internal_tension`):

```sql
-- Contradiction balance (§1 vector-sum): Σ weight(support-labelled) − Σ weight(contradicts-labelled)
-- over express edges incident to the finding. CONFORM to internal_tension's label-match idiom.
CREATE FUNCTION resource_contradiction_balance(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(CASE WHEN e.label = 'contradicts' THEN -e.weight ELSE e.weight END), 0)::double precision
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND NOT e.is_folded
      AND e.label = ANY (ARRAY['supports','corroborates','contradicts'])  -- support/oppose label set; Set 5/6 may extend
      AND (( e.source_table='kb_resources' AND e.source_id = p_finding)
        OR ( e.target_table='kb_resources' AND e.target_id = p_finding));
$$;

-- Adversarial survival (§1): N challenges withstood, distinct from 0 challenges. The adversary's
-- challenge/survived label vocabulary is SET 5's to finalize; these placeholders return 0 until then.
CREATE FUNCTION resource_adversarial_survival(p_finding uuid)
RETURNS TABLE(challenge_count int, survived double precision) LANGUAGE sql STABLE AS $$
    SELECT count(*) FILTER (WHERE e.label = 'challenged')::int,
           coalesce(sum(e.weight) FILTER (WHERE e.label = 'survived-challenge'), 0)::double precision
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND NOT e.is_folded
      AND (( e.source_table='kb_resources' AND e.source_id = p_finding)
        OR ( e.target_table='kb_resources' AND e.target_id = p_finding));
$$;
```

- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Prepare + commit** (`feat(standing): contradiction-balance + adversarial-survival readers (Set 3 Task 3)`).

### Task 4: `refresh_resource_standing`, read-time `standing_band`, and the access-gated `resource_standing_shape` read

**Files:** Modify the migration; add tests.

**Interfaces:**
- Produces: `refresh_resource_standing(uuid, uuid) → void` (finding, emitter); `standing_band(indep_breadth, challenge_count, survived, contradiction_balance, freshness) → text`; `resource_standing_shape(uuid, text, uuid) → TABLE(finding_id, indep_breadth, adversarial_survival, challenge_count, contradiction_balance, freshness, r_parent, band text)`.

**Design tags:**
- `refresh_resource_standing` — **CONFORM** to `refresh_salience` (`write.rs:851-916`): recompute components → UPSERT the memo → stamp `refreshed_event_id`. (SQL here; the Rust clock that *calls* it is Task 6.)
- `standing_band` — **EXTEND** (spec §1.1: band is a lossy read-time chip). **Read-time only; never stored.** Thresholds are named constants = the tunable "exact thresholds" Set 3 owns; this is the surfacing/tuning pass.
- `resource_standing_shape` — **CONFORM** to the access-gated-inside-SQL read idiom of `anchor_shape` / `resource_blocks` (gate via `resources_readable_by(p_principal_kind, p_principal_id)`); **CONFORM** to wayfind's memoized-vs-recompute (return memoized components, band computed live). **This is the read gate — it must enforce the full canonical visibility predicate, not a subset.**

- [ ] **Step 1: Failing tests** — band transitions + refresh-parity + read gate:

```rust
// Band is a read-time function over components, never stored. A finding with breadth 1, no
// survival, negative contradiction balance reads 'provisional'.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn band_is_read_time_over_components(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let band: String = sqlx::query_scalar("SELECT standing_band(1.0, 0, 0.0, -2.0, 0.5)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(band, "provisional");
    let strong: String = sqlx::query_scalar("SELECT standing_band(4.0, 2, 3.0, 5.0, 0.9)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(strong, "near-canonical");
}
// refresh_resource_standing lands the same components a fresh recompute would (parity — model on
// context_two_clocks.rs:112). No stored band column exists on kb_resources (assert the AMEND).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn refresh_lands_where_recompute_would(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await; bootseed::seed_system(&pool).await.unwrap();
    let finding = common::seed_resource_with_provenance(&pool, 2).await;
    sqlx::query("SELECT refresh_resource_standing($1, $2)")
        .bind(finding).bind(common::system_emitter()).execute(&pool).await.unwrap();
    let memo: f64 = sqlx::query_scalar("SELECT r_parent FROM kb_resource_standing WHERE finding_id=$1")
        .bind(finding).fetch_one(&pool).await.unwrap();
    let live: f64 = sqlx::query_scalar("SELECT resource_r_parent($1)").bind(finding).fetch_one(&pool).await.unwrap();
    assert_eq!(memo, live, "memoized r_parent == live recompute");
    // AMEND guard: kb_resources must have NO maturity/band column.
    let has_band: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name='kb_resources' AND column_name IN ('maturity','standing','band'))")
        .fetch_one(&pool).await.unwrap();
    assert!(!has_band, "spec §1.3 AMEND: no stored band on kb_resources");
}
```

- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Add the functions.** Append to the migration. `standing_band` thresholds are the tunable defaults — comment each with its rationale and quote spec §1.1:

```sql
-- Read-time band chip (spec §1.1: LOSSY summary over the shape, presented WITH it, never stored).
-- Thresholds are the tunable defaults Set 3 owns (the "exact thresholds" surfacing pass).
CREATE FUNCTION standing_band(
    p_indep_breadth double precision, p_challenge_count int, p_survived double precision,
    p_contradiction_balance double precision, p_freshness double precision)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        -- near-canonical: ≥3 effective-independent breadth, ≥1 survived challenge, balance clearly positive
        WHEN p_indep_breadth >= 3.0 AND p_survived >= 1.0 AND p_contradiction_balance > 1.0 THEN 'near-canonical'
        -- reinforced: ≥2 effective-independent breadth and not under live contradiction
        WHEN p_indep_breadth >= 2.0 AND p_contradiction_balance >= 0.0 THEN 'reinforced'
        ELSE 'provisional'
    END;
$$;

-- Cheap refresh: recompute components, UPSERT the memo, stamp the watermark. CONFORM to
-- refresh_salience (write.rs:851). p_emitter threads the actor for the watermark event linkage.
CREATE FUNCTION refresh_resource_standing(p_finding uuid, p_emitter uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_ch int; v_surv double precision;
BEGIN
    PERFORM refresh_independence_pairs(p_finding);
    SELECT challenge_count, survived INTO v_ch, v_surv FROM resource_adversarial_survival(p_finding);
    INSERT INTO kb_resource_standing
        (finding_id, indep_breadth, adversarial_survival, challenge_count, contradiction_balance, freshness, r_parent, updated)
    VALUES (p_finding, resource_independence_breadth(p_finding), v_surv, v_ch,
            resource_contradiction_balance(p_finding), resource_freshness(p_finding), resource_r_parent(p_finding), now())
    ON CONFLICT (finding_id) DO UPDATE SET
        indep_breadth=EXCLUDED.indep_breadth, adversarial_survival=EXCLUDED.adversarial_survival,
        challenge_count=EXCLUDED.challenge_count, contradiction_balance=EXCLUDED.contradiction_balance,
        freshness=EXCLUDED.freshness, r_parent=EXCLUDED.r_parent, updated=now();
END;
$$;

-- Access-gated read: memoized components + read-time band. CONFORM to resource_blocks' gate
-- (resources_readable_by) and wayfind's memoized-vs-recompute. Falls back to a live recompute
-- when the memo row is absent (freshness must be live at read anyway — it is time-decayed).
CREATE FUNCTION resource_standing_shape(p_finding uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(finding_id uuid, indep_breadth double precision, adversarial_survival double precision,
              challenge_count int, contradiction_balance double precision, freshness double precision,
              r_parent double precision, band text)
LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ), comp AS (
        SELECT g.fid,
               resource_independence_breadth(g.fid) AS ib,
               (SELECT survived FROM resource_adversarial_survival(g.fid)) AS surv,
               (SELECT challenge_count FROM resource_adversarial_survival(g.fid)) AS ch,
               resource_contradiction_balance(g.fid) AS cb,
               resource_freshness(g.fid) AS fr,
               resource_r_parent(g.fid) AS rp
        FROM gated g
    )
    SELECT fid, ib, surv, ch, cb, fr, rp, standing_band(ib, ch, surv, cb, fr) FROM comp;
$$;
```

> ⚠️ **Plan/reality gap**: `resource_standing_shape` recomputes components live rather than reading `kb_resource_standing` — deliberate, because `freshness` is time-decayed and must be current at read (the memo stores a snapshot). Confirm this matches how the team wants read cost to behave; if the memo should be authoritative for the non-decaying components, read them from `kb_resource_standing` and recompute only `freshness` + `band`. Verify `resources_readable_by`'s real signature on disk (`canonical_functions.sql`) before wiring the gate — [read-gate must match full canonical visibility].

- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Prepare + commit** (`feat(standing): refresh + read-time band + gated shape read (Set 3 Task 4)`). **Phase A complete — SQL substrate lands and is artifact-tested. Natural session boundary.**

---

## Phase B — Rust projection + wire type (Tasks 5–7)

### Task 5: Substrate refresh wrappers

**Files:** Modify `crates/temper-substrate/src/write.rs`; test in `crates/temper-substrate/tests/evidential_standing.rs`.

**Interfaces:**
- Produces: `pub async fn refresh_resource_standing(pool: &PgPool, finding: ResourceId, emitter: ProfileId) -> Result<()>` (or the crate's real id newtypes/emitter type — verify against `refresh_salience`'s signature on disk).

**Design tag:** **CONFORM** to `write::refresh_salience` (`write.rs:851`) — thin wrapper over the SQL `refresh_resource_standing`, same error type, same `sqlx::query!` macro style.

- [ ] **Step 1: Failing test** — call the Rust wrapper, assert the memo row is written.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement the wrapper.** ⚠️ Model on `refresh_salience`'s exact signature (id newtypes, `Result` alias, `pool` vs `tx`). Do not invent the emitter type — copy it.
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Prepare (`cargo make prepare-services` if the query lives in a test target) + commit.**

### Task 6: The standing clock — refresh on the write path

**Files:** Create `crates/temper-substrate/src/backend/standing_clock.rs` (or extend `region_clocks.rs` — decide by reading how `tick_region_clocks` is invoked); wire it into the resource/edge write path; test.

**Interfaces:**
- Consumes: Task 5's `refresh_resource_standing`.
- Produces: a `tick`-style entry called on the provenance/edge write path, errors logged-and-swallowed (CONFORM to `region_clocks.rs:33-39`).

**Design tag:** **CONFORM** to `region_clocks.rs` (the "fired inline on every resource write, no cron, no trigger" pattern). The trigger event for a standing refresh is: a `block_mutated`/provenance accretion on the finding, or an edge assert/fold incident to the finding.

- [ ] **Step 1: Failing test** — after a provenance write to a finding through the real write path, its `kb_resource_standing` row reflects the new `r_parent` without an explicit refresh call.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement.** ⚠️ **Plan/reality gap — this is the highest-risk task.** First trace *where* `tick_region_clocks` is called from (`crates/temper-services/src/backend/`), and hang the standing tick off the same seam. Determine the finding(s) affected by an edge write (both endpoints if they are resources). Keep it drift/dirty-gated so a hot write path is not doing full recomputes every call — but a simpler first cut (always-refresh-the-touched-finding) is acceptable if the gate is noted as a follow-up. Surface the choice in the PR.
- [ ] **Step 4: Run, verify pass** (`cargo make test-artifacts` + targeted substrate tests).
- [ ] **Step 5: Commit.**

### Task 7: `StandingShape` wire type

**Files:** Create `crates/temper-core/src/types/standing.rs`; register in `crates/temper-core/src/types/mod.rs`; substrate-local `StandingShapeRow` in `readback/mod.rs`; `readback::resource_standing(...)` producer.

**Interfaces:**
- Produces: `StandingShape { finding_id, indep_breadth, adversarial_survival, challenge_count, contradiction_balance, freshness, r_parent, band }` with the full derive stack; `readback::resource_standing(pool, principal, finding) -> Result<Option<StandingShapeRow>, ReadbackError>`.

**Design tag:** **CONFORM** to `CogmapRegionMetricsRow` (`crates/temper-core/src/types/cognitive_maps.rs`) for the derive stack, and to `readback::anchor_shape` (`readback/mod.rs:866`) for the substrate-local row + gated SQL read. Substrate can't depend on core → two structs (substrate row + core wire), mapped in the api/services wrapper.

- [ ] **Step 1: Failing test** — `readback::resource_standing` returns the shape for a readable finding, `None`/error for an unreadable one (gate test).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** the core type (copy the `#[cfg_attr(feature="typescript"...)]` derive block verbatim from `CogmapRegionMetricsRow`), the substrate row, and the `resource_standing` readback calling `resource_standing_shape($1,'profile',$2)`.
- [ ] **Step 4: Run, verify pass; regenerate ts types** (`cargo make generate-ts-types` — commit the generated `cognitive_maps.ts`/new `standing.ts` under `crates/temper-core/.../generated` and any `temper-ui` consumer).
- [ ] **Step 5: Prepare + commit.**

---

## Phase C — Read surface (Tasks 8–9)

### Task 8: `GET /api/resources/{id}/standing`

**Files:** Create `crates/temper-services/src/services/standing_service.rs`; add the handler to `crates/temper-api/src/handlers/resources.rs`; wire the route; regen openapi.

**Interfaces:**
- Consumes: Task 7's `readback::resource_standing` + `StandingShape`.
- Produces: `standing_service::resource_standing(pool, principal, finding) -> Result<StandingShape, ...>`; handler `-> ApiResult<Json<StandingShape>>`.

**Design tag:** **CONFORM** to `edge_service::list_resource_edges` + the `/edges` handler (thin handler → service → readback; middleware provides the authed profile; gate inside the SQL). [auth gate drift across surfaces]: the read gate lives in `resource_standing_shape`'s `resources_readable_by`, applied identically here.

- [ ] **Step 1: Failing test** (temper-api `--features test-db`, model on the `/edges` handler test): authed GET returns the shape; unreadable finding returns 404/403 per the `/edges` precedent.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** service + handler + route. Add `#[utoipa::path]` doc mirroring the `/edges` handler.
- [ ] **Step 4: Run, verify pass; regen openapi trio** — `cargo make openapi` (spec + temper-rb gem + temper-ts schema), then **`git add`** the regenerated artifacts (the drift gate compares against git; unstaged regen still reds `cargo make check`).
- [ ] **Step 5: Commit.**

### Task 9: `temper resource standing <ref>` CLI + e2e

**Files:** Create `crates/temper-cli/src/actions/standing.rs`; add the subcommand under `resource`; `tests/e2e/tests/resource_standing_test.rs`.

**Interfaces:**
- Consumes: Task 8's endpoint via `temper-client`.
- Produces: `temper resource standing <ref>` printing the shape vector + band chip (agent-first JSON default; band always shown *with* the shape, never instead — spec §1.1).

**Design tag:** **CONFORM** to CLI "thin command → fat action" (fundamentals) and the existing `resource show --edges` action for client wiring + `parse_ref` addressing.

- [ ] **Step 1: Failing e2e test** — through the real CLI↔API↔DB stack: create a resource with provenance, `temper resource standing <ref>`, assert the JSON carries all shape components + a `band` and that the shape is present alongside the band.
- [ ] **Step 2: Run, verify fail** (`cargo make test-e2e`).
- [ ] **Step 3: Implement** the action (resolve ref via `parse_ref`, GET the endpoint via the client), command wiring, output through `output/` helpers (never raw ANSI).
- [ ] **Step 4: Run, verify pass** — `cargo make test-e2e`; **also `cargo make prepare-e2e`** if e2e macros changed.
- [ ] **Step 5: Full verification + commit** — `cargo make check` (fmt, clippy, docs, machete, openapi + drift gates) and `cargo make test-artifacts` before the final commit.

---

## What Set 3 deliberately does NOT build (deferred, per spec + the two forks)

- **The scar writer** (`is_corrected` / independence-edge scarring emission) — deferred to Set 5. Set 3 only *reads* `is_folded`/`is_corrected` filters; they light up when a writer ships.
- **Meta-edges / `kb_edges` CHECK widening** — the spec §2.2 "meta-edge onto the edge" is not persistable today; representing an independence-claim challenge is Set 5's spec, not Set 3's. Set 3 reads live edge state (fold/weight) as the scar signal.
- **The adversary's challenge/survived label vocabulary** — Set 3 reads a provisional label set returning 0; Set 5 finalizes it.
- **Steward tend/reap consumption of standing** — Set 4.
- **Findings board as a distinct home** — Set 2; Set 3 scores any `kb_resource`.

## Self-review

- **Spec coverage**: §1.1 shape-primary + lossy band → Tasks 4 (`standing_band` read-time), 7/8/9 (shape carried, band with it). §1.2 orthogonal axes → validity axis untouched; maturity memo is separate (no collapse). §1.3 AMEND (no stored band) → Task 4 asserts absence of a band column. §2.1 two-count/terminating → `r_parent` (Task 1) + `resource_independence_breadth` leaf-tally (Task 2). §2.2 edge representation → `independent-of` express edge (Task 2). §2.3 memo → `kb_independence_pairs` + refresh (Task 2), Rust clock (Task 6). §2.4 silence=correlated → Task 2 test. Contradiction vector-sum + adversarial-survival 0-vs-N → Task 3. Materialization = components + read-time band via Rust clock → Tasks 4–6. Thresholds/shape presentation → Task 4 + Tasks 7–9.
- **Falsified-spec-claims handled**: meta-edge (deferred), trigger→Rust-clock (Task 6), is_corrected-writer (deferred) — all tagged AMEND with the disk citation.
- **Placeholder scan**: novel SQL bodies for `refresh_independence_pairs` / `resource_independence_breadth` / `resource_standing_shape` are marked "not a spec — model on the cited precedent, verify on disk" per GD-4; thresholds are named tunable defaults, not TBD.
- **Type consistency**: `StandingShape` field names (`indep_breadth, adversarial_survival, challenge_count, contradiction_balance, freshness, r_parent, band`) are identical across the SQL `resource_standing_shape` columns (Task 4), the substrate row + core type (Task 7), the service/handler (Task 8), and the CLI output (Task 9).
