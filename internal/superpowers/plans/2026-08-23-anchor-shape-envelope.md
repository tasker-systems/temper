# Anchor Shape Envelope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the shape read an anchor-level envelope, so an empty answer can say *why* it is empty — never clustered, nothing visible to you, narrowed by a lens, or unreadable.

**Architecture:** One gated SQL function computes the envelope and the region rows together, expanding `resources_visible_to` once and evaluating the access gate once. An empty anchor still yields exactly one row (`region_id` NULL) via `LEFT JOIN ... ON true`; that sentinel is what lets the empty case speak. The response type changes from a bare array to an object at every door — a deliberate, un-versioned wire break.

**Tech Stack:** PostgreSQL (sqlx compile-time-checked queries), Rust (axum, rmcp, ts-rs, utoipa, schemars), SvelteKit + TypeScript (temper-ui), cargo-make.

**Spec:** `internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md` — read it before Task 1. The plan sequences the spec; it does not replace it.

**Task:** `01a02ebd-c153-7d22-acb6-d9fdec1b0f16` · **Goal:** `019fbdb9-f287-79c0-aab6-efa0b1de12c8`

## Global Constraints

- **The gate is in SQL. Rust applies none.** `anchor_shape_select` is a pure row mapping and handlers pass the profile straight through. A new field is computed inside the SQL or it is not gated.
- **Deny is empty, never an error** — `crates/temper-services/src/backend/substrate_read.rs:1301-1305`. Do not introduce a 403/404 on this path.
- **Any count over regions or members is computed inside `vis`.** *"A caller is never told how many resources they cannot read."*
- **Never reuse `cogmap_list_rows`' `region_count`** (`migrations/20260724000010_cogmap_list_rows.sql:46`). It is keyed on the vestigial `cogmap_id` column, NULL for every context region, and is not member-gated.
- **`--all-features` on every build and clippy.** `cargo make check` for the gate.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.** All public types derive `Debug`.
- **SQL changes need the PER-CRATE `.sqlx` regeneration, not `--workspace`** (`Makefile.toml:112-121`): `cd crates/temper-services && cargo sqlx prepare -- --all-targets --all-features`. Same for `crates/temper-substrate`.
- **`cargo make check` does not run temper-ui.** `cd packages/temper-ui && bun run check` is separate and required (Task 5).
- **ts-rs drift clears only after a COMMIT**, not at `git add`.
- **Migration numbering:** `20260823*`, above `origin/main`'s highest (`20260822000030`). **Never edit an applied migration** — if the local DB has already applied `20260823000010`, reset the Docker volume rather than renumbering.
- **Redirect cargo output to a file** (`> out.txt 2>&1`), never `2>&1 | tail`.

---

## File Structure

| File | Responsibility |
|---|---|
| `migrations/20260823000010_anchor_shape_envelope.sql` | **Create.** `DROP`+`CREATE anchor_shape` with envelope columns; recreate `cogmap_shape` pinned to six columns. |
| `crates/temper-substrate/src/readback/mod.rs:1019-1085` | **Modify.** `CogmapShapeRow` unchanged; new `AnchorShapeReadback`; `anchor_shape` returns it and drops the NULL-region sentinel. |
| `crates/temper-core/src/types/cognitive_maps.rs` | **Modify.** New `AnchorShape` + `ShapeEmptiness` wire types beside `CogmapRegionRow`. |
| `crates/temper-services/src/backend/substrate_read.rs:1306-1327` | **Modify.** `anchor_shape_select` returns `AnchorShape`; maps the emptiness string to the enum. |
| `crates/temper-api/src/handlers/{contexts,cognitive_maps}.rs` | **Modify.** Response bodies and `utoipa::path` annotations. |
| `crates/temper-mcp/src/tools/cognitive_maps.rs:53`, `:626` | **Modify.** Both shape views serialize the envelope. |
| `crates/temper-client/src/{contexts,cognitive_maps}.rs` | **Modify.** Both `shape()` return types. |
| `packages/temper-ui/src/lib/server/graph-query.ts`, `src/lib/graph/readout.ts` | **Modify.** Unwrap `.regions` at the two `apiGet` sites. |
| `migrations/20260823000020_anchor_staleness.sql` | **Create.** `cogmap_staleness` regions arm onto the anchor pair. |
| `crates/temper-services/src/services/materialize_service.rs:26-31` | **Modify.** `CogmapId` → `HomeAnchor`. |
| `crates/temper-cli/src/cli.rs:1079` | **Modify.** Replace the false emptiness claim. |

---

### Task 1: The SQL function and the substrate readback

The whole envelope, computed once and read back typed. The tree stays green: `anchor_shape_select` keeps returning `Vec<CogmapRegionRow>` here, so no caller above it moves yet.

**Files:**
- Create: `migrations/20260823000010_anchor_shape_envelope.sql`
- Modify: `crates/temper-substrate/src/readback/mod.rs:1019-1085`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:1306-1327` (adaptation only)
- Test: `crates/temper-substrate/tests/anchor_shape_envelope.rs` (create)

**Interfaces:**
- Produces: `temper_substrate::readback::AnchorShapeReadback { population: i32, emptiness: Option<String>, materialized_at: Option<DateTime<Utc>>, regions: Vec<CogmapShapeRow> }` and `readback::anchor_shape(pool, anchor: HomeAnchor, principal: ProfileId, lens_id: Option<LensId>) -> Result<AnchorShapeReadback>`.
- `emptiness` is the raw SQL text (`"never_clustered"` etc.) or `None`. Task 3 maps it to the enum — **the substrate tier does not know the wire enum.**

- [ ] **Step 1: Read the function you are replacing, in full**

Read `migrations/20260713000050_region_visible_member_count.sql:41-135`. The prose at `:41-77` argues why the member gate exists and why the `cogmap` self-read arm is exempt; that argument is still binding. The new function **mirrors its clauses**, it does not reinvent them.

- [ ] **Step 2: Write the migration**

Create `migrations/20260823000010_anchor_shape_envelope.sql`:

```sql
-- The shape read gains an anchor-level envelope, so an empty answer can say why it is empty.
--
-- Non-additive on purpose: Postgres cannot CREATE OR REPLACE across a return-type change, so this
-- DROPs and re-CREATEs. Design: internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md
--
-- The region select, the member gate and the cogmap self-read exemption are carried over UNCHANGED
-- from 20260713000050:99-130. The argument for each is at 20260713000050:41-77 and still holds.
-- What is new is: (a) `regs` no longer applies p_lens (it is the ALL-LENS set, so `population` is a
-- real denominator rather than a restatement of the row count), and (b) the LEFT JOIN ON true, which
-- guarantees exactly one row even for an empty or unreadable anchor — the sentinel the envelope
-- speaks from.

DROP FUNCTION IF EXISTS anchor_shape(text, uuid, text, uuid, uuid);

CREATE FUNCTION anchor_shape(
    p_anchor_table   text,
    p_anchor_id      uuid,
    p_principal_kind text,
    p_principal_id   uuid,
    p_lens           uuid DEFAULT NULL
)
RETURNS TABLE(
    population       integer,
    emptiness        text,
    materialized_at  timestamptz,
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        -- Computed ONCE for both the rows and the population. Empty for a non-profile principal.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    ),
    gate AS (
        -- Always exactly one row (no FROM), which is what keeps `env` non-empty for an anchor that
        -- is unreadable OR does not exist. Disjunction carried over from 20260713000050:126-132.
        SELECT (
            (p_principal_kind = 'profile'
                 AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
            OR (p_principal_kind = 'cogmap'
                 AND p_anchor_table = 'kb_cogmaps'
                 AND p_principal_id = p_anchor_id)
        ) AS readable
    ),
    regs AS (
        SELECT reg.id AS region_id, reg.lens_id, reg.salience, reg.content_cohesion,
               COALESCE(reg.label, seen.rep_title) AS label,
               CASE
                   WHEN p_principal_kind = 'cogmap' THEN reg.member_count
                   ELSE seen.visible_members
               END AS member_count
        FROM kb_cogmap_regions reg
        CROSS JOIN LATERAL (
            SELECT count(*)::int AS visible_members,
                   (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
            FROM kb_cogmap_region_members m
            JOIN vis v ON v.resource_id = m.member_id
            JOIN kb_resources r ON r.id = m.member_id AND r.is_active
            WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ) seen
        WHERE reg.home_anchor_table = p_anchor_table
          AND reg.home_anchor_id    = p_anchor_id
          AND NOT reg.is_folded
          -- A region you can see nothing in is not a region you can see. (Cogmap arm exempt.)
          AND (p_principal_kind = 'cogmap' OR seen.visible_members > 0)
          AND (SELECT readable FROM gate)
        -- DELIBERATELY no p_lens predicate: `regs` is the ALL-LENS set. The lens narrows the ROWS
        -- returned, below; it must not narrow the denominator.
    ),
    clock AS (
        SELECT a.eid, ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    env AS (
        SELECT
            CASE WHEN g.readable THEN (SELECT count(*)::int FROM regs) ELSE 0 END AS population,
            CASE WHEN g.readable THEN (SELECT k.materialized_at FROM clock k) ELSE NULL END
                AS materialized_at,
            -- Precedence is load-bearing. Rule 2 MUST precede rule 3, or a never-clustered anchor
            -- reports 'nothing_visible' and the distinction this function exists to draw is lost.
            CASE
                WHEN NOT g.readable                        THEN 'unreadable_or_absent'
                WHEN (SELECT k.eid FROM clock k) IS NULL   THEN 'never_clustered'
                WHEN (SELECT count(*) FROM regs) = 0       THEN 'nothing_visible'
                WHEN (SELECT count(*) FROM regs rr
                       WHERE p_lens IS NULL OR rr.lens_id = p_lens) = 0 THEN 'lens_narrowed'
                ELSE NULL
            END AS emptiness
        FROM gate g
    )
    SELECT env.population, env.emptiness, env.materialized_at,
           r.region_id, r.lens_id, r.salience, r.content_cohesion, r.label, r.member_count
    FROM env
    LEFT JOIN (SELECT * FROM regs rr WHERE p_lens IS NULL OR rr.lens_id = p_lens) r ON true
    ORDER BY r.salience DESC NULLS LAST, r.region_id;
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions plus an anchor-level envelope, for EITHER anchor kind. Returns AT LEAST ONE ROW always: an empty or unreadable anchor yields a single row with region_id NULL, carrying the envelope. `population` is the member-gated region count across ALL lenses (a real denominator under a lens filter); `emptiness` names why the row set is empty (unreadable_or_absent / never_clustered / nothing_visible / lens_narrowed, NULL when non-empty); `materialized_at` is the shape watermark, NULL when never clustered. Deny and absent collapse into ONE arm and disclose neither population nor clock — no existence oracle. The gate is inside the SQL. The member gate, label fallback and cogmap self-read exemption are carried unchanged from 20260713000050.';

-- The wrapper is dead (no SQL or Rust caller reaches it), but DROPping anchor_shape strands it, and
-- a non-additive migration should not also be a silent removal. Pinned to the six original columns
-- by explicit select-list. Retiring the name belongs to M3 (20260713000010:185).
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count integer)
LANGUAGE sql STABLE AS $$
    SELECT s.region_id, s.lens_id, s.salience, s.content_cohesion, s.label, s.member_count
      FROM anchor_shape('kb_cogmaps', p_cogmap, p_principal_kind, p_principal_id, p_lens) s
     WHERE s.region_id IS NOT NULL;
$$;
```

- [ ] **Step 3: Apply the migration and confirm the five outcomes by hand**

```bash
cargo make db-migrate > /tmp/migrate.txt 2>&1; tail -20 /tmp/migrate.txt
```

Expected: the migration applies. If it fails on a checksum for an already-applied `20260823000010`, reset the Docker volume — **do not renumber, and do not edit the applied file.**

- [ ] **Step 4: Write the failing substrate test**

Create `crates/temper-substrate/tests/anchor_shape_envelope.rs`.

**Reuse, do not fork:** `crates/temper-substrate/tests/common/context_fixture.rs` is the shared fixture for a real embedded context with materialized regions — its own header records that it was *extracted rather than copied* because it encodes non-obvious formation constraints. For the cogmap-anchor cases and the region/member helpers, model on `crates/temper-substrate/tests/cogmap_shape_readback.rs:15-91` (`insert_cogmap_resource`, `add_member`, `insert_region`). Read both before writing a line of fixture code; **do not invent new helpers where these exist.**

Cover exactly the five outcomes:

```rust
#![cfg(feature = "artifact-tests")]
//! `readback::anchor_shape` — the surface tier plus its anchor-level envelope. Proves each of the
//! five outcomes the envelope distinguishes, including the two that were byte-identical before it.

// ... fixtures modelled on cogmap_shape_readback.rs:15-91 ...

#[sqlx::test]
async fn a_never_clustered_anchor_says_so(pool: PgPool) {
    // An anchor the principal CAN read, with shape_materialized_event_id NULL.
    let out = readback::anchor_shape(&pool, anchor, principal, None).await.unwrap();
    assert!(out.regions.is_empty());
    assert_eq!(out.population, 0);
    assert_eq!(out.emptiness.as_deref(), Some("never_clustered"));
    assert!(out.materialized_at.is_none());
}

#[sqlx::test]
async fn nothing_visible_is_not_never_clustered(pool: PgPool) {
    // Readable anchor, materialized, but every region's members are invisible to this principal.
    // THIS IS THE PAIR THE OLD READ COULD NOT SEPARATE — both were `[]`.
    let out = readback::anchor_shape(&pool, anchor, stranger, None).await.unwrap();
    assert!(out.regions.is_empty());
    assert_eq!(out.emptiness.as_deref(), Some("nothing_visible"));
    assert!(out.materialized_at.is_some(), "the clock is disclosed to a reader");
}

#[sqlx::test]
async fn an_unreadable_anchor_discloses_neither_population_nor_clock(pool: PgPool) {
    // Materialized, populated, but this principal cannot read the anchor at all.
    let out = readback::anchor_shape(&pool, anchor, outsider, None).await.unwrap();
    assert_eq!(out.emptiness.as_deref(), Some("unreadable_or_absent"));
    assert_eq!(out.population, 0, "must not leak the size of an anchor it cannot read");
    assert!(out.materialized_at.is_none(), "must not leak the clock either");
}

#[sqlx::test]
async fn an_absent_anchor_is_indistinguishable_from_an_unreadable_one(pool: PgPool) {
    let absent = HomeAnchor::Context(ContextId::from(Uuid::new_v4()));
    let out = readback::anchor_shape(&pool, absent, principal, None).await.unwrap();
    assert_eq!(out.emptiness.as_deref(), Some("unreadable_or_absent"));
    assert_eq!(out.population, 0);
    assert!(out.materialized_at.is_none());
}

#[sqlx::test]
async fn population_is_all_lenses_while_rows_are_lens_narrowed(pool: PgPool) {
    // Two regions under two DIFFERENT lenses, both visible. Read with one lens.
    let out = readback::anchor_shape(&pool, anchor, principal, Some(lens_a)).await.unwrap();
    assert_eq!(out.regions.len(), 1, "the lens narrows the rows");
    assert_eq!(out.population, 2, "the denominator is ALL lenses — this is what makes it a denominator");
}

#[sqlx::test]
async fn a_lens_that_matches_nothing_says_so_rather_than_going_silent(pool: PgPool) {
    // Regions exist and are visible, but none under the requested lens.
    let out = readback::anchor_shape(&pool, anchor, principal, Some(empty_lens)).await.unwrap();
    assert!(out.regions.is_empty());
    assert!(out.population > 0);
    assert_eq!(out.emptiness.as_deref(), Some("lens_narrowed"));
}

#[sqlx::test]
async fn a_populated_read_carries_no_emptiness(pool: PgPool) {
    let out = readback::anchor_shape(&pool, anchor, principal, None).await.unwrap();
    assert!(!out.regions.is_empty());
    assert_eq!(out.emptiness, None);
    assert_eq!(out.population as usize, out.regions.len());
}
```

- [ ] **Step 5: Run the tests — they must fail on the OLD return type**

```bash
cargo make test-artifacts > /tmp/t1.txt 2>&1; tail -40 /tmp/t1.txt
```

Expected: compile failure — `anchor_shape` returns `Vec<CogmapShapeRow>`, which has no `.population`. That is the correct red.

> **`cargo make test*` cancels on first failure** (nextest fail-fast), so "1 failed" is a **lower bound**, never a count.

- [ ] **Step 6: Implement the readback**

In `crates/temper-substrate/src/readback/mod.rs`, keep `CogmapShapeRow` (`:1024-1035`) **exactly as it is** and add beside it:

```rust
/// An anchor's surface tier plus the anchor-level envelope, as returned by `anchor_shape`.
/// Substrate-local; `temper-services` maps this to the `AnchorShape` wire type.
///
/// `emptiness` is the SQL function's raw discriminant (`"never_clustered"`, `"nothing_visible"`,
/// `"lens_narrowed"`, `"unreadable_or_absent"`) or `None` when the row set is non-empty. The
/// substrate tier deliberately does not know the wire enum — the mapping, and the error on an
/// unrecognized arm, live at the service boundary where drift between SQL and Rust must surface.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorShapeReadback {
    pub population: i32,
    pub emptiness: Option<String>,
    pub materialized_at: Option<chrono::DateTime<chrono::Utc>>,
    pub regions: Vec<CogmapShapeRow>,
}
```

Then rewrite `anchor_shape` (`:1052-1085`). Every column of a set-returning function reads as nullable to sqlx, so the `!` overrides carry the function's contract — and `region_id` is now **genuinely nullable** (the sentinel), so it loses its override:

```rust
pub async fn anchor_shape(
    pool: &PgPool,
    anchor: HomeAnchor,
    principal: ProfileId,
    lens_id: Option<LensId>,
) -> Result<AnchorShapeReadback> {
    let rows = sqlx::query!(
        r#"SELECT population      AS "population!",
                  emptiness,
                  materialized_at,
                  region_id,
                  lens_id,
                  salience,
                  content_cohesion,
                  label,
                  member_count
             FROM anchor_shape($1, $2, 'profile', $3, $4)"#,
        anchor.table(),
        anchor.uuid(),
        principal.uuid(),
        lens_id.map(LensId::uuid),
    )
    .fetch_all(pool)
    .await?;

    // The function guarantees at least one row: an empty anchor yields the envelope with a NULL
    // region_id. An empty `rows` here would mean the guarantee was broken, so read the envelope
    // defensively rather than indexing.
    let head = rows.first();
    let population = head.map(|r| r.population).unwrap_or(0);
    let emptiness = head.and_then(|r| r.emptiness.clone());
    let materialized_at = head.and_then(|r| r.materialized_at);

    let regions = rows
        .iter()
        .filter_map(|r| {
            // Drop the sentinel. When region_id is present the function guarantees lens_id,
            // salience and member_count are too.
            let region_id = r.region_id?;
            Some(CogmapShapeRow {
                region_id: RegionId::from(region_id),
                lens_id: LensId::from(r.lens_id.expect("lens_id accompanies region_id")),
                salience: r.salience.expect("salience accompanies region_id"),
                content_cohesion: r.content_cohesion,
                label: r.label.clone(),
                member_count: r.member_count.expect("member_count accompanies region_id"),
            })
        })
        .collect();

    Ok(AnchorShapeReadback { population, emptiness, materialized_at, regions })
}
```

- [ ] **Step 7: Keep the tree green above the substrate**

`anchor_shape_select` (`crates/temper-services/src/backend/substrate_read.rs:1306-1327`) now gets a struct. Change **only** the one line that consumes it so the crate still compiles — the signature stays `Vec<CogmapRegionRow>` until Task 3:

```rust
    let rows = readback::anchor_shape(pool, anchor, profile_id, lens_id.map(LensId::from))
        .await
        .map_err(api_err)?
        .regions;
```

- [ ] **Step 8: Regenerate the per-crate sqlx caches**

```bash
cd crates/temper-substrate && cargo sqlx prepare -- --all-targets --all-features > /tmp/prep1.txt 2>&1
cd ../temper-services && cargo sqlx prepare -- --all-targets --all-features > /tmp/prep2.txt 2>&1
```

**Not `--workspace`** — it clobbers per-crate caches. Note that dropping the old query orphans its `.sqlx` entry; `prepare` removes it.

- [ ] **Step 9: Run the tests to green**

```bash
cargo make test-artifacts > /tmp/t1b.txt 2>&1; tail -40 /tmp/t1b.txt
cargo make check > /tmp/c1.txt 2>&1; tail -30 /tmp/c1.txt
```

Expected: all seven new tests pass; `check` clean.

> If nextest sits at 0% CPU for a long time on fresh binaries, that is **macOS Gatekeeper**, not a hang.

- [ ] **Step 10: Commit**

```bash
git add migrations/20260823000010_anchor_shape_envelope.sql \
        crates/temper-substrate/src/readback/mod.rs \
        crates/temper-substrate/tests/anchor_shape_envelope.rs \
        crates/temper-services/src/backend/substrate_read.rs \
        crates/temper-substrate/.sqlx crates/temper-services/.sqlx
git commit -m "feat(shape): anchor_shape returns an anchor-level envelope

An empty anchor now yields one row with region_id NULL, carrying the
population, the clustered clock, and a named reason the row set is empty.
Deny and absent collapse into one arm that discloses neither."
```

---

### Task 2: The wire types

**Files:**
- Modify: `crates/temper-core/src/types/cognitive_maps.rs` (add beside `CogmapRegionRow` at `:41-64`)
- Test: `crates/temper-core/tests/` — the existing ts-rs export test picks these up

**Interfaces:**
- Produces: `temper_core::types::cognitive_maps::{AnchorShape, ShapeEmptiness}`. Task 3 consumes both.

- [ ] **Step 1: Read the neighbour first**

Read `crates/temper-core/src/types/cognitive_maps.rs:41-64` (`CogmapRegionRow`, for the derive stack) and `crates/temper-core/src/types/api.rs:137-142` (the house precedent for a ts-rs-exported wire enum with `rename_all`).

- [ ] **Step 2: Add the types**

```rust
/// Why a shape read came back with no regions. Absent (`None`) when the read returned rows.
///
/// `UnreadableOrAbsent` is deliberately ONE arm for two situations — a caller who cannot read the
/// anchor and an anchor that does not exist must stay indistinguishable, or the envelope becomes an
/// existence oracle. It discloses neither the population nor the clock. The other three arms are
/// only ever reached by a caller who passed the anchor gate, for whom "this anchor exists" is not a
/// disclosure.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShapeEmptiness {
    /// The anchor has regions this caller could see, but a `lens` filter excluded all of them.
    /// `population` is > 0 — the caller is looking at a narrowed view, not an empty anchor.
    LensNarrowed,
    /// The anchor has been clustered, but every region in it is invisible to this caller.
    NothingVisible,
    /// The anchor has never been materialized. `materialized_at` is `None`.
    NeverClustered,
    /// The caller cannot read this anchor, OR it does not exist. One arm on purpose.
    UnreadableOrAbsent,
}

/// An anchor's materialized regions, with the anchor-level facts that let an empty answer say why
/// it is empty. Returned by `anchor_shape` for EITHER anchor kind.
///
/// `population` is the region count this principal can see across **all** lenses, member-gated —
/// so under a `lens` filter it is strictly greater than `regions.len()`, and equal to it otherwise.
/// It is a denominator, not a restatement of the row count.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AnchorShape {
    /// The regions themselves, most salient first — narrowed by `lens` when one was supplied.
    pub regions: Vec<CogmapRegionRow>,
    /// Visible regions in this anchor across ALL lenses. `0` for a caller who cannot read it.
    pub population: i32,
    /// Why `regions` is empty; `None` when it is not.
    pub emptiness: Option<ShapeEmptiness>,
    /// When the anchor was last clustered. `None` means never — or that the caller cannot read it.
    pub materialized_at: Option<DateTime<Utc>>,
}
```

> **Do not fold `rename_all` into another `#[serde(...)]` attribute.** ts-rs discards an entire serde attribute when any part of it is unsupported, which once silently dropped a `rename` at `crates/temper-core/src/types/managed_meta.rs:49-55`.

- [ ] **Step 3: Verify it compiles and the bindings export**

```bash
cargo check -p temper-core --all-features > /tmp/t2.txt 2>&1; tail -20 /tmp/t2.txt
cargo test -p temper-core --all-features export_bindings -- --test-threads=1 > /tmp/t2b.txt 2>&1; tail -20 /tmp/t2b.txt
```

`--test-threads=1` is required: the `export_bindings_*` tests all write one file and race otherwise.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/cognitive_maps.rs crates/temper-core/bindings/
git commit -m "feat(shape): AnchorShape and ShapeEmptiness wire types"
```

---

### Task 3: The service boundary and both API handlers

**Files:**
- Modify: `crates/temper-services/src/backend/substrate_read.rs:1306-1327`
- Modify: `crates/temper-api/src/handlers/contexts.rs:234-264` and `crates/temper-api/src/handlers/cognitive_maps.rs:176-205`
- Test: `crates/temper-api/tests/context_orientation_test.rs` (nine call sites), `crates/temper-api/tests/cogmap_shape_handler_test.rs`

**Interfaces:**
- Consumes: `AnchorShapeReadback` (Task 1), `AnchorShape` / `ShapeEmptiness` (Task 2).
- Produces: `anchor_shape_select(pool, profile_id, anchor, lens_id) -> ApiResult<AnchorShape>`. Tasks 4 and 5 consume this signature.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-api/tests/context_orientation_test.rs`, using the file's own `insert_context_region` (`:29`) and `grant_context_read` (`:70`) helpers — **read them first; do not write new ones.** The load-bearing case is the pair the old read could not separate:

```rust
#[sqlx::test]
async fn a_never_clustered_context_is_distinguishable_from_one_with_nothing_visible(pool: PgPool) {
    // Context A: readable, never materialized (shape_materialized_event_id stays NULL).
    // Context B: readable, materialized, but its only region's members are invisible to `reader`.
    let a = anchor_shape_select(&pool, ProfileId::from(reader), anchor_a, None).await.unwrap();
    let b = anchor_shape_select(&pool, ProfileId::from(reader), anchor_b, None).await.unwrap();

    assert!(a.regions.is_empty() && b.regions.is_empty(), "both are empty — as they were before");
    assert_eq!(a.emptiness, Some(ShapeEmptiness::NeverClustered));
    assert_eq!(b.emptiness, Some(ShapeEmptiness::NothingVisible));
    assert_ne!(a.emptiness, b.emptiness, "this is the whole point of the task");
}

#[sqlx::test]
async fn population_is_member_gated_across_two_principals(pool: PgPool) {
    // The task's first acceptance criterion. Asserted UNDER A LENS, because without one
    // `population` equals `regions.len()` and the criterion is met by the row count alone.
    let wide = anchor_shape_select(&pool, ProfileId::from(grantee), anchor, Some(lens_a)).await.unwrap();
    let narrow = anchor_shape_select(&pool, ProfileId::from(stranger), anchor, Some(lens_a)).await.unwrap();
    assert!(wide.population > narrow.population, "reach decides the denominator, not just the rows");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo make test-db > /tmp/t3.txt 2>&1; tail -40 /tmp/t3.txt
```

Expected: compile failure — `Vec<CogmapRegionRow>` has no `.emptiness`.

- [ ] **Step 3: Map the readback to the wire type**

Rewrite `anchor_shape_select` in `crates/temper-services/src/backend/substrate_read.rs`. The doc comment above it (`:1300-1305`) about the gate living in the SQL still holds — keep it and extend it:

```rust
pub async fn anchor_shape_select(
    pool: &PgPool,
    profile_id: ProfileId,
    anchor: HomeAnchor,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<AnchorShape> {
    let out = readback::anchor_shape(pool, anchor, profile_id, lens_id.map(LensId::from))
        .await
        .map_err(api_err)?;

    // The SQL discriminant is mapped here, exhaustively. An unrecognized arm means the migration and
    // this match have drifted, which is a deploy-time bug — surface it rather than coercing it to a
    // plausible variant and hiding it.
    let emptiness = match out.emptiness.as_deref() {
        None => None,
        Some("lens_narrowed") => Some(ShapeEmptiness::LensNarrowed),
        Some("nothing_visible") => Some(ShapeEmptiness::NothingVisible),
        Some("never_clustered") => Some(ShapeEmptiness::NeverClustered),
        Some("unreadable_or_absent") => Some(ShapeEmptiness::UnreadableOrAbsent),
        Some(other) => {
            return Err(ApiError::Internal(format!(
                "anchor_shape returned an unknown emptiness arm {other:?} — migration and \
                 ShapeEmptiness have drifted"
            )))
        }
    };

    Ok(AnchorShape {
        regions: out
            .regions
            .into_iter()
            .map(|r| CogmapRegionRow {
                region_id: r.region_id,
                lens_id: r.lens_id,
                salience: r.salience,
                content_cohesion: r.content_cohesion,
                label: r.label,
                member_count: r.member_count,
            })
            .collect(),
        population: out.population,
        emptiness,
        materialized_at: out.materialized_at,
    })
}
```

- [ ] **Step 4: Update both handlers**

In `crates/temper-api/src/handlers/contexts.rs` and `crates/temper-api/src/handlers/cognitive_maps.rs`, change the return type to `ApiResult<Json<AnchorShape>>` and — **this is the part that is easy to miss** — the `body =` in the `utoipa::path` annotation from `Vec<CogmapRegionRow>` to `AnchorShape`, plus the response description. The generated `openapi.json` is only as right as this annotation.

Update the note above `contexts.rs:225` (*"a caller who cannot read the context gets an empty list rather than a 403"*) to say the caller now gets `emptiness: unreadable_or_absent`, which still discloses nothing.

- [ ] **Step 5: Fix the remaining call sites**

Nine in `context_orientation_test.rs`, plus `cogmap_shape_handler_test.rs` and `tests/e2e/tests/context_orientation_e2e.rs`. Most become `.regions` on the existing assertion. **Do not blanket-replace** — `unreadable_context_is_empty_not_error` (`:214`) should additionally assert the new arm, and `a_region_with_no_visible_members_is_returned_by_neither_door` (`:376`) is now a `nothing_visible` case.

- [ ] **Step 6: Run to green**

```bash
cargo make test-db > /tmp/t3b.txt 2>&1; tail -40 /tmp/t3b.txt
cargo make check > /tmp/c3.txt 2>&1; tail -30 /tmp/c3.txt
```

> `access_gate_test` self-races; if it flakes, isolate with `--test-threads=1` before treating it as a regression. `test-db` TIMEOUTs are contention, not regression — isolate before diagnosing.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-services crates/temper-api tests/e2e
git commit -m "feat(shape): both shape routes return the envelope

Array to object at the HTTP boundary. The utoipa body annotations move
with the handlers, so openapi.json follows."
```

---

### Task 4: MCP views and the client

**Files:**
- Modify: `crates/temper-mcp/src/tools/cognitive_maps.rs:53-66` (cogmap) and `:626-639` (context)
- Modify: `crates/temper-client/src/contexts.rs:138-149`, `crates/temper-client/src/cognitive_maps.rs:98-109`

**Interfaces:**
- Consumes: `anchor_shape_select -> ApiResult<AnchorShape>` (Task 3).
- Produces: `ContextClient::shape(context_id, lens) -> Result<AnchorShape>`, `CogmapClient::shape(cogmap_id, lens_id) -> Result<AnchorShape>`. The CLI consumes these.

- [ ] **Step 1: Update both MCP views**

Both sites bind `rows` and `serde_json::to_string_pretty(&rows).unwrap_or_else(|| "[]".to_string())`. The variable is now the envelope, so **the `"[]"` fallback is wrong** — it would hand an agent an array where the schema says object. Change it to `"{}"` and rename the binding from `rows` to `shape` at both sites so the next reader isn't misled.

- [ ] **Step 2: Update both client methods**

Change both return types to `Result<AnchorShape>`. Update the doc comments — both currently say "Empty if the caller cannot read"; they should say the caller receives `emptiness: unreadable_or_absent`, and that an empty result now names its cause.

- [ ] **Step 3: Verify the CLI needs no change**

`crate::format::render` is generic over `Serialize` (`crates/temper-cli/src/format.rs:81-87`) and both shape commands hand it the value directly (`commands/cogmap.rs:98-99`, `commands/context_cmd.rs:398-399`). Confirm by reading, then build:

```bash
cargo make check > /tmp/c4.txt 2>&1; tail -30 /tmp/c4.txt
```

No new CLI verb is added, so `DOCUMENTED_VERBS` (`crates/temper-cli/src/cli.rs:2820`) does **not** change.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp crates/temper-client
git commit -m "feat(shape): MCP views and client carry the envelope"
```

---

### Task 5: temper-ui and the generated artifacts

**Files:**
- Modify: `packages/temper-ui/src/lib/server/graph-query.ts:106,141-143,235,241`
- Modify: `packages/temper-ui/src/lib/graph/readout.ts:82,90-96`
- Modify: the test files that build shape literals
- Regenerate: `openapi.json`, ts-rs tree, `clients/temper-ts/src/generated/schema.ts`, the Ruby gem model, the skills projection

- [ ] **Step 1: Regenerate the artifacts**

Follow the `generated-artifacts` skill. Then:

```bash
cargo make check > /tmp/c5.txt 2>&1; tail -40 /tmp/c5.txt
```

Expected: `openapi-check`, `openapi-rb-drift`, `openapi-ts-drift`, `ts-rs-drift` and `skills-drift` all green. **ts-rs drift only clears after a commit** — if it is still red at `git add`, commit and re-run before diagnosing.

- [ ] **Step 2: Unwrap `.regions` at the two apiGet sites**

In `graph-query.ts`, both `apiGet<CogmapRegionRow[]>(anchorShapePath(...))` calls become `apiGet<AnchorShape>(...)`. The multi-anchor site at `:141-143` returns `{ rows, complete }` — take `.regions` from each response. The comment at `:106` says both doors *"return `Vec<CogmapRegionRow>`"*; that is now false — correct it.

`readout.ts`'s `RegionLookup { rows: CogmapRegionRow[] }` (`:90-96`) can stay as-is: it is fed from `graph-query.ts`, so the unwrap happens upstream. Prefer that over rippling the envelope through the readout layer, which does not use it.

- [ ] **Step 3: Run the UI gate — `cargo make check` does NOT cover this**

```bash
cd packages/temper-ui && bun run check > /tmp/ui.txt 2>&1; tail -40 /tmp/ui.txt
```

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui clients/ openapi.json crates/temper-core/bindings agent-skills/
git commit -m "feat(shape): temper-ui and generated artifacts follow the envelope"
```

---

### Task 6: The clock, generalized

**Files:**
- Create: `migrations/20260823000020_anchor_staleness.sql`
- Test: `crates/temper-api/tests/context_orientation_test.rs` or a sibling

- [ ] **Step 1: Read `cogmap_staleness` and understand the trap before writing anything**

`migrations/20260624000002_canonical_functions.sql:527-551`. Its `kb_edges` arm (`:543-545`) is **already** anchor-generic; only the regions arm (`:538-541`) is stuck on `reg.cogmap_id = p_cogmap`.

> **The trap.** Leave the regions arm on `cogmap_id` while generalizing and contexts do not error and do not return nulls. `latest_touch` is NULL, so `latest_touch > materialized_at` is NULL, and the `COALESCE` at `:549` falls through to `materialized_at IS NULL` — **false** for any context that has materialized once. Every context reports `is_stale = false` permanently and nothing goes red.

- [ ] **Step 2: Write the failing test FIRST — and make it able to fail**

The witness **must use a context that has materialized AND been touched since**. A context that has only materialized cannot tell a working function from the broken one — both say `is_stale = false`.

```rust
#[sqlx::test]
async fn a_touched_context_reports_stale(pool: PgPool) {
    // 1. Materialize the context (set shape_materialized_event_id to a real event).
    // 2. Touch one of its regions with a LATER event (update last_event_id).
    // 3. is_stale must be TRUE.
    //
    // If the regions arm is left on cogmap_id this returns FALSE, silently. That is the only
    // assertion in this task that can catch the trap — a never-touched context passes either way.
    let stale: bool = sqlx::query_scalar("SELECT is_stale FROM anchor_staleness('kb_contexts', $1)")
        .bind(context)
        .fetch_one(&pool).await.unwrap();
    assert!(stale, "a context touched after materializing is stale — if this is false, the regions arm is still keyed on cogmap_id");
}
```

- [ ] **Step 3: Write the migration**

Create `anchor_staleness(p_anchor_table text, p_anchor_id uuid)` returning the same three columns. Three changes from `cogmap_staleness`, and the first is the one the whole task turns on:

```sql
    -- WAS (20260624000002:538-541) — structurally blind to context regions:
    --   SELECT ev.occurred_at FROM kb_cogmap_regions reg
    --     JOIN kb_events ev ON ev.id = reg.last_event_id
    --    WHERE reg.cogmap_id = p_cogmap
    -- NOW — keyed on the anchor pair, the same key anchor_shape uses:
            SELECT ev.occurred_at FROM kb_cogmap_regions reg
              JOIN kb_events ev ON ev.id = reg.last_event_id
             WHERE reg.home_anchor_table = p_anchor_table
               AND reg.home_anchor_id    = p_anchor_id
```

Second, the `kb_edges` arm (`:543-545`) is **already** anchor-generic — change only its two literals from `'kb_cogmaps'` / `p_cogmap` to the parameters. Third, the `mat` CTE reads `shape_materialized_event_id` from whichever table `p_anchor_table` names, using the same `UNION ALL` shape as Task 1's `clock` CTE.

Keep `cogmap_staleness(uuid)` as a wrapper delegating to the new function, so `cogmap_analytics` (`migrations/20260628000001:63-77`) and `crates/temper-substrate/src/scenario/runner.rs:486` keep working untouched.

- [ ] **Step 4: Run to green, regenerate caches, commit**

```bash
cargo make db-migrate > /tmp/m6.txt 2>&1
cargo make test-db > /tmp/t6.txt 2>&1; tail -40 /tmp/t6.txt
cd crates/temper-services && cargo sqlx prepare -- --all-targets --all-features > /tmp/p6.txt 2>&1
```

```bash
git add migrations/20260823000020_anchor_staleness.sql crates/ 
git commit -m "feat(staleness): the clock reads either anchor kind

The regions arm moves off the vestigial cogmap_id onto the anchor pair.
Left as-is it would have reported every context permanently fresh."
```

---

### Task 7: `materialize_delta` takes an anchor

**Files:**
- Modify: `crates/temper-services/src/services/materialize_service.rs:26-31`
- Modify: `crates/temper-api/src/handlers/contexts.rs` (new handler), `crates/temper-api/src/routes.rs:111-113`

- [ ] **Step 1: Widen the signature**

`materialize_delta(pool, principal, cogmap_id: CogmapId, threshold)` → `anchor: HomeAnchor`. The parts beneath it already take one: `replay::formation_touched_count_since(pool, anchor, watermark)` (`crates/temper-substrate/src/replay.rs:839-843`). Replace the inline `FROM kb_cogmaps` gate query with one branching on the anchor table, keeping `anchor_readable_by_profile`.

> **Its `NotFound`-on-deny posture stays** (`materialize_service.rs:47`). That is correct for this surface and must **not** become the shape envelope's posture, nor vice versa. `MaterializeDelta.cogmap_id` is a wire field — widening it is a second wire break; carry it as the anchor pair the way `MaterializeAck` already does (`crates/temper-core/src/types/materialize.rs:135-156`).

- [ ] **Step 2: Register the context route**

Beside `handlers::contexts::materialize` (`crates/temper-api/src/routes.rs:113`), mirroring `handlers::cognitive_maps::materialize_delta` (`:145`). **No CLI verb** — out of scope, so `DOCUMENTED_VERBS` does not change.

- [ ] **Step 3: Test, regenerate artifacts, commit**

```bash
cargo make test-db > /tmp/t7.txt 2>&1; tail -40 /tmp/t7.txt
cargo make check > /tmp/c7.txt 2>&1; tail -30 /tmp/c7.txt
cd packages/temper-ui && bun run check > /tmp/ui7.txt 2>&1; tail -20 /tmp/ui7.txt
```

```bash
git add crates/ openapi.json clients/
git commit -m "feat(materialize): materialize_delta reads either anchor kind

Closes the asymmetry T8 left: a context can be materialized but could
not be asked when that last happened."
```

---

### Task 8: The claim, and the docs

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:1079`
- Modify: docs where the shape read's emptiness is described

- [ ] **Step 1: Replace the false claim**

`crates/temper-cli/src/cli.rs:1079` currently reads:

> Empty means the context has not materialized regions yet — run `context materialize`.

That is one of four causes. Replace it with text that points at the field which now answers it — the response's `emptiness` names the cause — rather than asserting one cause. Do the same for the cogmap `Shape` verb if it carries a similar claim.

- [ ] **Step 2: State the disagreement rather than letting it be discovered**

After this lands, `cogmap list`'s `region_count` and the shape envelope's `population` legitimately differ for the same map: the former is keyed on the vestigial `cogmap_id` and is not member-gated. Say so where a reader meets it — the `cogmap list` help text and/or the cognitive-maps skill page.

- [ ] **Step 3: Full verification**

```bash
cargo make check > /tmp/c8.txt 2>&1; tail -40 /tmp/c8.txt
cargo make test-db > /tmp/t8.txt 2>&1; tail -40 /tmp/t8.txt
cargo make test-artifacts > /tmp/a8.txt 2>&1; tail -40 /tmp/a8.txt
cd packages/temper-ui && bun run check > /tmp/ui8.txt 2>&1; tail -20 /tmp/ui8.txt
```

Read every output. **`cargo make test-all` has one pre-existing streaming/embed timeout** — that is not a regression from this work.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli docs/ agent-skills/
git commit -m "docs(shape): the help text stops asserting a cause it cannot know"
```

---

## After the plan

One consolidated review across the whole branch (not per-task), then `git merge origin/main`, push, and open a PR. **Do not merge locally.**

The register's clause this serves is `composition-is-legible`, and this task is `enables` — it witnesses nothing on its own. The witness is the sibling task, `01a02ebe-a3d2-7ad2-81df-541924c00e36`.
