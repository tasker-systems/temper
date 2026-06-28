# cogmap_shape Read Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the SQL function `cogmap_shape` — an agent/UI's read of a cognitive map's materialized regions — onto live read paths (api + mcp + cli), where today it has no Rust binding and is reachable only from scenario fixtures.

**Architecture:** Mirror the existing `search` read path exactly. A query binding lives in `temper-substrate::readback` (returning a crate-local row, because `temper-substrate` cannot depend on `temper-core`); `temper-api`'s service-direct wrapper (`backend::substrate_read`) maps that row to a `temper-core` wire type with `ts-rs` derives; the API handler, MCP tool, CLI command, and `temper-client` sub-client are thin shells over the wrapper. Reads stay service-direct (not the `Backend` trait), per the read-path rule.

**Tech Stack:** Rust, axum (API), rmcp (MCP), clap (CLI), sqlx (runtime `query_as`, no compile-time macro because the SQL function is unqualified and self-gating), `ts-rs` (TS type generation), utoipa (OpenAPI).

## Global Constraints

These apply to every task (from `CLAUDE.md` Code Quality Rules + the surfacing spec):

- **Reads are service-direct.** `list`/`show`/`get_meta`/`search`/`cogmap_shape` reads do NOT go through the `Backend` trait. They live in `temper-api::backend::substrate_read` and call `temper-substrate::readback`. Never inline `sqlx::query!()` in a surface (handler/tool/CLI action).
- **Typed structs over inline JSON.** No `serde_json::json!()` for structured data — define a struct.
- **Shared types at boundaries live in `temper-core`** with `ts-rs` derives; both Rust and TS consume the generated type. Never hand-write a zod/TS mirror.
- **`temper-substrate` cannot use `temper-core`** (core is a dev-dep only). Binding-layer rows are substrate-local; the `temper-api` wrapper does the row→wire mapping.
- **Runtime `sqlx::query`/`query_as`, NOT the `query!` macros**, for any SQL that calls the substrate functions/visibility helpers — established by `readback::fts_search`/`vector_search`/`unified_search` (module note at `crates/temper-substrate/src/readback/mod.rs:17`). No `.sqlx` cache regen is required for runtime queries.
- **The access gate is INSIDE `cogmap_shape` SQL.** `cogmap_shape(p_cogmap, p_principal_kind, p_principal_id, p_lens)` only returns rows when `cogmap_readable_by_profile(p_principal_id, p_cogmap)` (for `p_principal_kind = 'profile'`). A non-readable map yields zero rows — NOT an error. Do not add a second visibility check in the wrapper.
- **`#[expect(clippy::too_many_arguments)]` is a smell to fix**, not suppress — use a params struct past 5 domain args.
- **All public types implement `Debug`.** Never emit raw ANSI from the CLI — route through `crate::format::render` / `crate::output`.
- **Error escalation:** genuine faults propagate as errors (→ 500); they are never swallowed or mapped to an empty result.

**Out of scope (deferred to follow-on tasks, per the user's "thin vertical" scoping):**
- The analytics readouts `cogmap_telos` / `cogmap_staleness` / `cogmap_regulation` and the five `cogmap_region_*` scalar metrics.
- The live temper-ui component (the `ts-rs` type is exported here so the UI task can consume it, but no Svelte work).
- Full CLI↔API↔DB e2e with region seeding in the e2e harness (the sibling invocation-envelope task owns extending that harness; do NOT fake-seed it here).

---

## Reference: the verified `cogmap_shape` contract

SQL (`migrations/20260624000002_canonical_functions.sql:423-437`):

```sql
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count int)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion, reg.label, reg.member_count
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      AND ( (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
         OR (p_principal_kind = 'cogmap'  AND p_principal_id = p_cogmap) );
$$;
```

Return columns and their Rust decode types (note `content_cohesion` and `label` are nullable in `kb_cogmap_regions` so they flow through as NULL): `region_id uuid` → `Uuid`; `lens_id uuid` → `Uuid`; `salience double precision` (col NOT NULL) → `f64`; `content_cohesion double precision` (col nullable) → `Option<f64>`; `label text` (col nullable) → `Option<String>`; `member_count int` (col NOT NULL) → `i32`.

We surface only `p_principal_kind = 'profile'` (a logged-in human/agent). The `'cogmap'` principal kind is for substrate-internal map-to-map reads and is not a surface concern.

---

## Task 1: `CogmapRegionRow` wire type in `temper-core`

**Files:**
- Create: `crates/temper-core/src/types/cognitive_maps.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod cognitive_maps;`)

**Interfaces:**
- Produces: `temper_core::types::cognitive_maps::CogmapRegionRow { region_id: Uuid, lens_id: Uuid, salience: f64, content_cohesion: Option<f64>, label: Option<String>, member_count: i32 }` — derives `Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow`, and (feature-gated) `ts_rs::TS` + `utoipa::ToSchema`. Field order and names match the `cogmap_shape` SQL return exactly so `query_as::<_, CogmapRegionRow>` decodes directly in Task 3.

Model this on the existing `UnifiedSearchResultRow` at `crates/temper-core/src/types/api.rs:95-114` (same derive/`cfg_attr` shape).

- [ ] **Step 1: Write the failing test**

Append to the new file (tests live alongside; mirror how `api.rs` is structured):

```rust
// crates/temper-core/src/types/cognitive_maps.rs (test module at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn cogmap_region_row_serde_roundtrip_preserves_nullables() {
        let row = CogmapRegionRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            salience: 0.75,
            content_cohesion: None,
            label: Some("Migration tooling".to_string()),
            member_count: 4,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        // null nullable + present nullable both survive the round-trip
        assert!(json.contains("\"content_cohesion\":null"), "json: {json}");
        assert!(json.contains("\"label\":\"Migration tooling\""), "json: {json}");
        let back: CogmapRegionRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core --features typescript cogmap_region_row_serde_roundtrip`
Expected: FAIL — compile error, `CogmapRegionRow` not found.

- [ ] **Step 3: Write the type**

Top of `crates/temper-core/src/types/cognitive_maps.rs`:

```rust
//! Cognitive-map read-surface wire types.
//!
//! `CogmapRegionRow` is the surface tier of a materialized region — centroid-derived readouts only
//! (salience, content-cohesion, label, member_count). Member identities are NEVER carried here; the
//! interior is dereferenced per-member through `resources_visible_to` elsewhere. Mirrors the
//! `cogmap_shape` SQL return (`migrations/20260624000002_canonical_functions.sql`) field-for-field so
//! the `temper-api` read wrapper can `query_as` straight into it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One non-folded region of a cognitive map under a lens, as returned by `cogmap_shape`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct CogmapRegionRow {
    /// `kb_cogmap_regions.id` — the region's stable identity.
    pub region_id: Uuid,
    /// The lens (perspective) that produced this region.
    pub lens_id: Uuid,
    /// Computed, memoized blend (telos-alignment + reference-standing + centrality); higher = more salient.
    pub salience: f64,
    /// Mean member-to-centroid cosine; `None` until the downstream readout has been computed.
    pub content_cohesion: Option<f64>,
    /// Optional agent-authored region label.
    pub label: Option<String>,
    /// Member count (the blur the surface tier exposes; identities stay interior).
    pub member_count: i32,
}
```

Add the module declaration in `crates/temper-core/src/types/mod.rs` (keep the list alphabetical — it goes between `config` and `conflict`):

```rust
pub mod cognitive_maps;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core --features typescript cogmap_region_row_serde_roundtrip`
Expected: PASS.

- [ ] **Step 5: Regenerate TS types and confirm the export compiles**

Run: `cargo make generate-ts-types`
Expected: writes `cognitive_maps.ts` into the ts-rs output dir; no errors. (This keeps `packages/temper-ui`'s generated types in sync for the later UI task.)

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/cognitive_maps.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(core): add CogmapRegionRow wire type for cogmap_shape surface"
```

---

## Task 2: `cogmap_shape` query binding in `temper-substrate::readback`

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs` (add the row struct + the binding fn)
- Create: `crates/temper-substrate/tests/cogmap_shape_readback.rs` (artifact-test)

**Interfaces:**
- Produces: `temper_substrate::readback::CogmapShapeRow { region_id: Uuid, lens_id: Uuid, salience: f64, content_cohesion: Option<f64>, label: Option<String>, member_count: i32 }` (derives `Debug, Clone, PartialEq`) and `pub async fn cogmap_shape(pool: &PgPool, cogmap_id: Uuid, principal: Uuid, lens_id: Option<Uuid>) -> anyhow::Result<Vec<CogmapShapeRow>>`. Task 3 consumes both.

This is the rigorous correctness test — region-bearing data is cheap here via the existing `fire(CogmapGenesis)` primitive (`crates/temper-substrate/tests/common/mod.rs:36-101`).

- [ ] **Step 1: Write the failing test**

Create `crates/temper-substrate/tests/cogmap_shape_readback.rs`. The seed: genesis a cogmap (gives us a cogmap + telos + events for free), join it to a fresh NON-root team, make principal `P1` a member and `P2` not, then raw-insert two regions (one folded, one not) under the global `telos-default` lens.

```rust
#![cfg(feature = "artifact-tests")]
//! `readback::cogmap_shape` — the surface-tier region read. Proves: non-folded regions surface for a
//! readable principal; the in-SQL access gate (`cogmap_readable_by_profile`) denies a non-member
//! (zero rows, not an error); folded regions are excluded; the lens filter narrows by lens.

use sqlx::PgPool;
use uuid::Uuid;

use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef; // unused-import guard: drop if not needed after writing seed

mod common;

/// One region to seed. A params struct (not a long arg list) per the >5-domain-args rule. `cogmap`/
/// `lens`/`event` are the shared fixture context; `salience`/`label`/`member_count`/`is_folded` vary
/// per region.
struct RegionSeed<'a> {
    cogmap: Uuid,
    lens: Uuid,
    /// An arbitrary existing event id, reused for both NOT NULL event FKs.
    event: Uuid,
    salience: f64,
    label: &'a str,
    member_count: i32,
    is_folded: bool,
}

/// Insert one region from a `RegionSeed`. `centroid` is an all-zero 768-vector (cogmap_shape never
/// reads it). Returns the new region id.
async fn insert_region(pool: &PgPool, seed: RegionSeed<'_>) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, content_cohesion, label, member_count,
            asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2, array_fill(0::double precision, ARRAY[768])::vector, $3, NULL, $4, $5, $6, $6, $7)
         RETURNING id",
    )
    .bind(seed.cogmap)
    .bind(seed.lens)
    .bind(seed.salience)
    .bind(seed.label)
    .bind(seed.member_count)
    .bind(seed.event)
    .bind(seed.is_folded)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_shape_surfaces_unfolded_regions_and_gates_by_readability(pool: PgPool) {
    common::seed_system(&pool).await; // boot the canonical `system` actor (see common/mod.rs)

    // Genesis a cogmap (creates the cogmap + telos + events). Reuse the helper pattern in common/mod.rs.
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "shape-test", "Shape Test").await;

    // A fresh NON-root team + two profiles: P1 a member (readable), P2 not (denied).
    let team = common::create_team(&pool, "shape-team").await;
    let p1 = common::create_profile(&pool, "member@example.com").await;
    let p2 = common::create_profile(&pool, "outsider@example.com").await;
    common::add_team_member(&pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(&pool)
        .await
        .expect("join cogmap to team");

    // Global telos-default lens (cogmap_id IS NULL), seeded by bootseed.
    let lens: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL")
            .fetch_one(&pool)
            .await
            .expect("global telos-default lens");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("any event for FK");

    let kept = insert_region(
        &pool,
        RegionSeed { cogmap, lens, event, salience: 0.9, label: "kept", member_count: 3, is_folded: false },
    )
    .await;
    let _folded = insert_region(
        &pool,
        RegionSeed { cogmap, lens, event, salience: 0.8, label: "folded-out", member_count: 2, is_folded: true },
    )
    .await;

    // Readable principal sees exactly the one non-folded region.
    let rows = temper_substrate::readback::cogmap_shape(&pool, cogmap, p1, None)
        .await
        .expect("readable read");
    assert_eq!(rows.len(), 1, "only the non-folded region surfaces: {rows:?}");
    assert_eq!(rows[0].region_id, kept);
    assert_eq!(rows[0].label.as_deref(), Some("kept"));
    assert_eq!(rows[0].member_count, 3);
    assert_eq!(rows[0].content_cohesion, None);

    // Non-member is denied by the in-SQL gate: zero rows, NOT an error.
    let denied = temper_substrate::readback::cogmap_shape(&pool, cogmap, p2, None)
        .await
        .expect("gate denial is empty, not an error");
    assert!(denied.is_empty(), "non-member must see no regions: {denied:?}");

    // Lens filter: a non-matching lens id narrows to empty for the readable principal.
    let other_lens = Uuid::now_v7();
    let filtered = temper_substrate::readback::cogmap_shape(&pool, cogmap, p1, Some(other_lens))
        .await
        .expect("lens-filtered read");
    assert!(filtered.is_empty(), "wrong lens yields no regions: {filtered:?}");
}
```

NOTE on helpers: `common::seed_system` exists (referenced by `fire_resource_with_headed_chunk`). Add thin `common::genesis_cogmap`, `common::create_team`, `common::create_profile`, `common::add_team_member` wrappers to `crates/temper-substrate/tests/common/mod.rs` if absent — `genesis_cogmap` is the genesis half of the existing `fire_resource_with_headed_chunk` (lines 85-101, returning `fired.cogmap_genesis().unwrap()`); team/profile/membership inserts are 3-line raw `INSERT`s against `kb_teams`/`kb_profiles`/`kb_team_members` (inspect those tables in `migrations/20260624000001_canonical_schema.sql` for column lists). Adding these helpers is part of this step.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo make test-artifacts` (or `cargo nextest run -p temper-substrate --features artifact-tests cogmap_shape_surfaces_unfolded_regions`)
Expected: FAIL — `readback::cogmap_shape` not found.

- [ ] **Step 3: Write the binding**

Add to `crates/temper-substrate/src/readback/mod.rs` (after `vector_search`, alongside the other read floors). Mirror the runtime-`sqlx::query` style of `fts_search` (`mod.rs:657`):

```rust
/// One surface-tier region of a cognitive map, as returned by `cogmap_shape`. Centroid-derived
/// readouts only — member identities are NEVER carried (the interior is dereferenced per-member
/// through `resources_visible_to` elsewhere). Substrate-local because `temper-substrate` cannot
/// depend on `temper-core`; the `temper-api` wrapper maps this to the `CogmapRegionRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapShapeRow {
    pub region_id: Uuid,
    pub lens_id: Uuid,
    pub salience: f64,
    pub content_cohesion: Option<f64>,
    pub label: Option<String>,
    pub member_count: i32,
}

/// The surface-tier read of a cognitive map's materialized regions (spec §A surfacing; SQL
/// `cogmap_shape`). The access gate is INSIDE the SQL: a principal who cannot read the map gets zero
/// rows (never an error). Folded regions are excluded by the function; `lens_id = None` returns all
/// lenses, `Some(l)` narrows to that lens.
///
/// Runtime `sqlx::query` (NOT the `query!` macros) — the SQL is unqualified and self-gating; see the
/// module-level note. Read-only.
pub async fn cogmap_shape(
    pool: &PgPool,
    cogmap_id: Uuid,
    principal: Uuid,
    lens_id: Option<Uuid>,
) -> Result<Vec<CogmapShapeRow>> {
    let rows = sqlx::query(
        "SELECT region_id, lens_id, salience, content_cohesion, label, member_count
           FROM cogmap_shape($1, 'profile', $2, $3)",
    )
    .bind(cogmap_id)
    .bind(principal)
    .bind(lens_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| CogmapShapeRow {
            region_id: r.get("region_id"),
            lens_id: r.get("lens_id"),
            salience: r.get("salience"),
            content_cohesion: r.get("content_cohesion"),
            label: r.get("label"),
            member_count: r.get("member_count"),
        })
        .collect())
}
```

(`use sqlx::Row;` is already imported at the top of the module — `mod.rs:26`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo make test-artifacts` (or the scoped nextest filter from Step 2)
Expected: PASS — three assertions (surface, deny, lens-filter) green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-substrate/src/readback/mod.rs crates/temper-substrate/tests/cogmap_shape_readback.rs crates/temper-substrate/tests/common/mod.rs
git commit -m "feat(substrate): readback binding for cogmap_shape (surface-tier region read)"
```

---

## Task 3: service wrapper + API handler + route in `temper-api`

**Files:**
- Modify: `crates/temper-api/src/backend/substrate_read.rs` (add `cogmap_shape_select`)
- Modify: `crates/temper-api/src/handlers/cognitive_maps.rs` (add the `shape` handler)
- Modify: `crates/temper-api/src/routes.rs:87` (add the GET route next to the existing PUT)
- Create: `crates/temper-api/tests/cogmap_shape_handler_test.rs` (test-db integration test)

**Interfaces:**
- Consumes: `temper_substrate::readback::{cogmap_shape, CogmapShapeRow}` (Task 2); `temper_core::types::cognitive_maps::CogmapRegionRow` (Task 1).
- Produces: `temper_api::backend::substrate_read::cogmap_shape_select(pool: &PgPool, profile_id: ProfileId, cogmap_id: Uuid, lens_id: Option<Uuid>) -> ApiResult<Vec<CogmapRegionRow>>` (consumed by the MCP tool in Task 5) and the route `GET /api/cognitive-maps/{id}/shape?lens=<uuid>`.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/cogmap_shape_handler_test.rs`. The L0 system-default cogmap is born by migration and root-joined (readable by any approved profile), so it exercises the readable plumbing without seeding regions (it returns `Ok(vec![])` — L0 has no materialized regions). The region-bearing correctness is already proven in Task 2; this test proves the api wrapper resolves, gates, and maps.

```rust
#![cfg(feature = "test-db")]
//! `substrate_read::cogmap_shape_select` — the api-side service-direct wrapper over the
//! `readback::cogmap_shape` binding. Proves the readable path returns Ok against the root-joined L0
//! map, and that a non-readable map yields an empty (not errored) result.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::substrate_read::cogmap_shape_select;
use temper_core::types::ids::ProfileId;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_shape_is_readable_and_returns_ok(pool: PgPool) {
    // L0 is root-joined → readable by any approved profile. No regions materialized → Ok(empty).
    let profile = common::fixtures::create_test_profile(&pool, "reader@example.com").await;
    let rows = cogmap_shape_select(&pool, ProfileId::from(profile), L0_COGMAP, None)
        .await
        .expect("readable L0 shape read must be Ok");
    assert!(rows.is_empty(), "L0 has no materialized regions yet: {rows:?}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unknown_cogmap_is_empty_not_error(pool: PgPool) {
    // A random cogmap id the profile cannot read: the in-SQL gate yields zero rows, never an error.
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let rows = cogmap_shape_select(&pool, ProfileId::from(profile), Uuid::now_v7(), None)
        .await
        .expect("non-readable map is empty, not an error");
    assert!(rows.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_shape_handler_test`
Expected: FAIL — `cogmap_shape_select` not found.

(Remember to export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` and have `cargo make docker-up` running, per CLAUDE.md.)

- [ ] **Step 3: Write the service wrapper**

Add to `crates/temper-api/src/backend/substrate_read.rs` (after `search_select`). It maps the substrate row to the core wire type — the same row→wire shape `search_select` uses for `ScoredHit` → `UnifiedSearchResultRow`:

```rust
use temper_core::types::cognitive_maps::CogmapRegionRow;

/// `cogmap_shape` — the surface-tier read of a cognitive map's materialized regions. Service-direct
/// (reads bypass the Backend trait). The access gate lives in the SQL function: a principal who cannot
/// read the map gets an empty vec, never an error. Maps the substrate-local row to the wire type.
pub async fn cogmap_shape_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionRow>> {
    let rows = readback::cogmap_shape(pool, cogmap_id, *profile_id, lens_id)
        .await
        .map_err(api_err)?;
    Ok(rows
        .into_iter()
        .map(|r| CogmapRegionRow {
            region_id: r.region_id,
            lens_id: r.lens_id,
            salience: r.salience,
            content_cohesion: r.content_cohesion,
            label: r.label,
            member_count: r.member_count,
        })
        .collect())
}
```

(`api_err`, `readback`, `ProfileId`, `PgPool`, `ApiResult` are already imported/used in this file by `search_select`.)

- [ ] **Step 4: Run the wrapper test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_shape_handler_test`
Expected: PASS — both cases green.

- [ ] **Step 5: Write the API handler**

Add to `crates/temper-api/src/handlers/cognitive_maps.rs`. Use an axum `Query` extractor for the optional `lens`. Mirror the `search` handler shape (`handlers/search.rs:22`):

```rust
use axum::extract::Query;
use serde::Deserialize;
use temper_core::types::cognitive_maps::CogmapRegionRow;

/// Query params for the shape read. `lens` is optional (omit → all lenses).
#[derive(Debug, Deserialize)]
pub struct ShapeQuery {
    pub lens: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/shape",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Materialized regions (surface tier)", body = Vec<CogmapRegionRow>),
        (status = 401, description = "Unauthorized", body = crate::error::ErrorBody),
    )
)]
pub async fn shape(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionRow>>> {
    crate::backend::substrate_read::cogmap_shape_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens,
    )
    .await
    .map(Json)
}
```

(`ApiResult` is already imported; `ApiError` import stays for the existing `reconcile`. Add `use axum::extract::Query;` to the existing `use axum::extract::{Path, State};` line.)

- [ ] **Step 6: Register the route**

In `crates/temper-api/src/routes.rs`, extend the existing cognitive-maps route block (currently `routes.rs:87-90`):

```rust
        .route(
            "/api/cognitive-maps/{id}",
            put(handlers::cognitive_maps::reconcile),
        )
        .route(
            "/api/cognitive-maps/{id}/shape",
            get(handlers::cognitive_maps::shape),
        )
```

(`get` is already imported in `routes.rs` — it's used by `/api/health` etc.)

- [ ] **Step 7: Register the OpenAPI path**

Add `handlers::cognitive_maps::shape` to the utoipa `paths(...)` list in `crates/temper-api/src/openapi.rs` (find the existing `cognitive_maps::reconcile` entry and add the sibling). Then run the existing openapi assertion test to confirm the path is registered:

Run: `cargo nextest run -p temper-api --features test-db --test '*' openapi` (or the specific openapi test module). Confirm `/api/cognitive-maps/{id}/shape` appears.

- [ ] **Step 8: Verify the whole crate builds + tests pass**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_shape_handler_test`
Then: `cargo make check`
Expected: PASS / clean.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/src/backend/substrate_read.rs crates/temper-api/src/handlers/cognitive_maps.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs crates/temper-api/tests/cogmap_shape_handler_test.rs
git commit -m "feat(api): GET /api/cognitive-maps/{id}/shape service-direct read"
```

---

## Task 4: `temper-client` sub-client method

**Files:**
- Modify: `crates/temper-client/src/cognitive_maps.rs` (add `shape`)

**Interfaces:**
- Consumes: `temper_core::types::cognitive_maps::CogmapRegionRow` (Task 1).
- Produces: `CognitiveMapClient::shape(&self, cogmap_id: Uuid, lens_id: Option<Uuid>) -> Result<Vec<CogmapRegionRow>>` (consumed by the CLI action in Task 6).

- [ ] **Step 1: Write the failing test**

Add a unit test for the path/query construction (the existing `cognitive_maps.rs` has no test module — add one). Keep it to URL shaping, since live HTTP is covered by the api integration test:

```rust
#[cfg(test)]
mod tests {
    use uuid::Uuid;

    // The shape endpoint path is /api/cognitive-maps/{id}/shape with an optional ?lens= query.
    fn shape_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
        let base = format!("/api/cognitive-maps/{cogmap_id}/shape");
        match lens {
            Some(l) => format!("{base}?lens={l}"),
            None => base,
        }
    }

    #[test]
    fn shape_path_omits_lens_when_none() {
        let id = Uuid::from_u128(7);
        assert_eq!(shape_path(id, None), format!("/api/cognitive-maps/{id}/shape"));
    }

    #[test]
    fn shape_path_includes_lens_when_some() {
        let id = Uuid::from_u128(7);
        let lens = Uuid::from_u128(9);
        assert_eq!(
            shape_path(id, Some(lens)),
            format!("/api/cognitive-maps/{id}/shape?lens={lens}")
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-client shape_path`
Expected: FAIL — `shape_path` not found (it's defined in the test, so this fails only if you haven't added it yet; write the test first, watch it compile-fail, then add the method that uses the same path logic).

- [ ] **Step 3: Add the client method**

In `crates/temper-client/src/cognitive_maps.rs`, add the import and the method. Mirror `reconcile_cognitive_map` (same file, lines 32-43) but with `GET` and the query param built by the same `shape_path` logic (promote `shape_path` from the test to a private fn so prod and test share it — DRY):

```rust
use temper_core::types::cognitive_maps::CogmapRegionRow;

// (add near the top, beside the existing reconcile imports)

impl<'a> CognitiveMapClient<'a> {
    // ... existing new() + reconcile_cognitive_map() ...

    /// GET /api/cognitive-maps/{id}/shape[?lens=] — the surface-tier read of a map's materialized
    /// regions. Returns the non-folded regions visible to the authenticated principal (empty if the
    /// principal cannot read the map).
    pub async fn shape(
        &self,
        cogmap_id: Uuid,
        lens_id: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionRow>> {
        let token = self.http.resolve_token()?;
        let path = shape_path(cogmap_id, lens_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}

/// `/api/cognitive-maps/{id}/shape` with an optional `?lens=` query — shared by the method and its test.
fn shape_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/shape");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}
```

Then delete the duplicate `shape_path` from the test module and have the tests call the module-level one (`use super::shape_path;`). Confirm `HttpClient::get` exists (mirror of `.post`/`.put` used in this file and `search.rs`); if the helper is named differently, match the existing GET helper used by other GET sub-clients (e.g. `crates/temper-client/src/contexts.rs`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-client shape_path`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-client/src/cognitive_maps.rs
git commit -m "feat(client): CognitiveMapClient::shape sub-client method"
```

---

## Task 5: MCP tool `cogmap_shape`

**Files:**
- Create: `crates/temper-mcp/src/tools/cognitive_maps.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs` (add `pub mod cognitive_maps;`)
- Modify: `crates/temper-mcp/src/service.rs` (add the `#[tool]` method)
- Modify: `crates/temper-core/src/types/cognitive_maps.rs` (add the `CogmapShapeInput` params type with `mcp`-gated `JsonSchema`)

**Interfaces:**
- Consumes: `temper_api::backend::substrate_read::cogmap_shape_select` (Task 3).
- Produces: the MCP tool `cogmap_shape` taking `{ cogmap: String (ref), lens: Option<String> (ref) }`.

The input type goes in `temper-core` (shared boundary type), gated on the `mcp` feature for the `schemars::JsonSchema` derive — and enums/refs must be inlined per the repo gotcha (`project_mcp_enum_params_must_inline`); this type has no enums so it's straightforward.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-core/src/types/cognitive_maps.rs` test module — the ref-parse contract the tool relies on (parse the cogmap ref to a UUID via the canonical resolver):

```rust
    #[test]
    fn cogmap_shape_input_ref_parses_trailing_uuid() {
        // The tool resolves `cogmap` via parse_ref (trailing-UUID-only; slug half ignored).
        let id = Uuid::from_u128(0x42);
        let decorated = format!("my-map-{id}");
        let (parsed, _slug) = temper_core::operations::parse_ref(&decorated).expect("parse ref");
        assert_eq!(parsed, id);
    }
```

(If `parse_ref` lives at `temper_workflow::operations::parse_ref` rather than `temper_core::operations::parse_ref`, adjust — verify with `grep -rn "pub fn parse_ref" crates/`. The CLI uses `temper_workflow::operations::parse_ref` at `commands/cogmap.rs:21`; CLAUDE.md names `temper_core::operations::parse_ref`. Use whichever the grep confirms is public, and keep the test in the crate that owns it.)

- [ ] **Step 2: Run test to verify it fails / confirm parse_ref location**

Run: `grep -rn "pub fn parse_ref" crates/` then `cargo nextest run -p temper-core cogmap_shape_input_ref_parses` (or `-p temper-workflow` if that's where it lives).
Expected: confirms the resolver path; test compiles once the path is right.

- [ ] **Step 3: Add the input params type**

In `crates/temper-core/src/types/cognitive_maps.rs`:

```rust
/// MCP/surface input for the cogmap shape read. `cogmap` is a ref (UUID or decorated
/// `sluggify(title)-<uuid>`); `lens` is an optional lens ref to narrow the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapShapeInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// Optional lens ref to filter regions; omit for all lenses.
    pub lens: Option<String>,
}
```

- [ ] **Step 4: Write the MCP tool**

Create `crates/temper-mcp/src/tools/cognitive_maps.rs`, mirroring `tools/search.rs`:

```rust
//! Cognitive-map read tools. `cogmap_shape` reads the surface tier (materialized regions) of a map.

use rmcp::model::CallToolResult;

use temper_core::types::cognitive_maps::CogmapShapeInput;
use temper_core::types::ids::ProfileId;

use crate::service::TemperMcpService;

pub async fn cogmap_shape(
    svc: &TemperMcpService,
    input: CogmapShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    // Resolve refs → UUIDs (trailing-UUID-only; slug half ignored). Use the same resolver the CLI uses.
    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    let lens_id = match input.lens.as_deref() {
        Some(l) => Some(
            temper_workflow::operations::parse_ref(l)
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad lens ref: {e}"), None))?
                .0,
        ),
        None => None,
    };

    let rows = temper_api::backend::substrate_read::cogmap_shape_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        lens_id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_shape failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}
```

Add `pub mod cognitive_maps;` to `crates/temper-mcp/src/tools/mod.rs`.

- [ ] **Step 5: Register the `#[tool]`**

In `crates/temper-mcp/src/service.rs`, add beside the `search` tool (after `service.rs:269`), mirroring its shape exactly:

```rust
    #[tool(
        description = "Read a cognitive map's surface tier: its materialized regions (salience, cohesion, label, member count) under an optional lens. Pass the map by ref."
    )]
    async fn cogmap_shape(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_shape(self, input).await
    }
```

Confirm `temper-mcp`'s `Cargo.toml` enables the `mcp` feature on its `temper-core` dependency (it must, for the other `Parameters<...>` tools to derive `JsonSchema`). The new `CogmapShapeInput` rides that existing feature.

- [ ] **Step 6: Build + verify**

Run: `cargo nextest run -p temper-mcp` then `cargo make check`
Expected: PASS / clean. (The MCP round-trip e2e in the Embed CI job will exercise the live tool; no new e2e needed here.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/types/cognitive_maps.rs crates/temper-mcp/src/tools/cognitive_maps.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): cogmap_shape tool over the shape read surface"
```

---

## Task 6: CLI `temper cogmap shape`

**Files:**
- Create: `crates/temper-cli/src/actions/cogmap.rs` (the testable action) — OR add to an existing cogmap action module if one exists (grep `crates/temper-cli/src/actions/` first).
- Modify: `crates/temper-cli/src/commands/cogmap.rs` (add the `shape` command shell)
- Modify: the CLI subcommand enum that routes `cogmap` subcommands (grep `enum.*Cogmap` / `Cogmap {` under `crates/temper-cli/src/commands/mod.rs` and the top-level CLI definition)

**Interfaces:**
- Consumes: `CognitiveMapClient::shape` (Task 4); `temper_core::types::cognitive_maps::CogmapRegionRow` (Task 1).
- Produces: `temper cogmap shape <cogmap_ref> [--lens <ref>]` printing the regions via `crate::format::render` (JSON/TOON per the agent-first output defaults).

- [ ] **Step 1: Write the failing test**

Create `crates/temper-cli/src/actions/cogmap.rs` with a pure render test (mirror `actions/search.rs`'s `render_search_results_json_is_passthrough_array`, `actions/search.rs:155`):

```rust
//! `temper cogmap shape` business logic — thin wrapper over the cognitive-maps client. Cloud-only.

use temper_core::types::cognitive_maps::CogmapRegionRow;

use crate::error::Result;

/// Call the shape API for the given cogmap (and optional lens), both already resolved to UUIDs.
pub async fn shape_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionRow>> {
    client
        .cognitive_maps()
        .shape(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn render_shape_rows_json_is_passthrough_array() {
        let rows: Vec<CogmapRegionRow> = vec![CogmapRegionRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            salience: 0.5,
            content_cohesion: None,
            label: Some("region".to_string()),
            member_count: 2,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"region_id\""), "json: {out}");
        assert!(out.contains("\"member_count\""), "json: {out}");
    }
}
```

Add `pub mod cogmap;` to `crates/temper-cli/src/actions/mod.rs` if `actions/cogmap.rs` is new.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli render_shape_rows_json_is_passthrough_array`
Expected: FAIL — module/function not found.

- [ ] **Step 3: Implement the action (already written above) and the command shell**

In `crates/temper-cli/src/commands/cogmap.rs`, add the `shape` command (mirror `reconcile`'s ref-parse + `with_client` + render shape, lines 14-41):

```rust
/// `temper cogmap shape <cogmap_ref> [--lens <ref>]` — read the map's materialized regions.
pub fn shape(cogmap_ref: &str, lens_ref: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let lens_id = lens_ref
        .map(|l| temper_workflow::operations::parse_ref(l).map(|p| p.0))
        .transpose()?;

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::shape_api(&client, cogmap_id, lens_id).await })
    })?;

    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
```

(`with_client` is the right runtime helper here — this is a client-dependent async call, per `runtime_helper_choice`.)

- [ ] **Step 4: Wire the subcommand**

Find the clap enum routing `cogmap` subcommands (grep `Cogmap` in `crates/temper-cli/src/commands/mod.rs` and the CLI arg definitions — the existing `reconcile` subcommand shows the pattern). Add a `Shape` variant:

```rust
/// Read a cognitive map's materialized regions (surface tier).
Shape {
    /// The cognitive map, by ref (UUID or slug-<uuid>).
    cogmap: String,
    /// Optional lens ref to filter regions.
    #[arg(long)]
    lens: Option<String>,
},
```

And in the match arm that dispatches cogmap subcommands, add:

```rust
CogmapCommand::Shape { cogmap, lens } => {
    commands::cogmap::shape(&cogmap, lens.as_deref(), fmt)
}
```

(Match the exact enum/dispatch names found by grep — the variant for `reconcile` is right beside it.)

- [ ] **Step 5: Run test + build the binary**

Run: `cargo nextest run -p temper-cli render_shape_rows_json_is_passthrough_array`
Then: `cargo build -p temper-cli`
Expected: PASS / builds.

- [ ] **Step 6: Manual smoke against L0 (optional, needs a running stack + auth)**

If a local stack is up and you're authenticated:
Run: `temper cogmap shape 00000000-0000-0000-0005-000000000001`
Expected: `[]` (L0 has no materialized regions) — proves the full CLI→client→API→DB path resolves and gates without error.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/cogmap.rs crates/temper-cli/src/actions/mod.rs crates/temper-cli/src/commands/cogmap.rs crates/temper-cli/src/commands/mod.rs
git commit -m "feat(cli): temper cogmap shape — read materialized regions"
```

---

## Final verification (whole-plan)

- [ ] `cargo make check` — fmt + clippy (`-D warnings`) + docs + machete + TS typecheck/biome, all clean.
- [ ] `cargo make test` — unit tests pass.
- [ ] `cargo make test-db` — temper-api integration tests pass (incl. `cogmap_shape_handler_test`).
- [ ] `cargo make test-artifacts` — temper-substrate artifact tests pass (incl. `cogmap_shape_readback`).
- [ ] `cargo make generate-ts-types` — `cognitive_maps.ts` regenerated and committed if changed.
- [ ] Reinstall the PATH binary if you smoke-tested the CLI: `cargo install --path crates/temper-cli` (per `reinstall_temper_after_cli_merge`).
- [ ] Consolidated spec + code-quality review at the end of the plan (not per-task), per the subagent-review-cadence.

## Self-Review notes (author)

- **Spec coverage:** acceptance criterion "`cogmap_shape` (+ region readouts) reachable from cli/mcp/api against the live backend" — `cogmap_shape` is covered across all three surfaces (Tasks 3/5/6); the *region readouts* (telos/staleness/regulation + 5 scalars) are explicitly deferred per the chosen thin-vertical scope (Global Constraints → Out of scope). "Surface onto the NATIVE shape, not the reconstruction shim" — satisfied: the binding reads `kb_cogmap_regions` directly via the single post-flip backend; there is no shim involved.
- **Type consistency:** `CogmapRegionRow` (wire, temper-core) and `CogmapShapeRow` (substrate-local) carry identical fields/types; the map is in `cogmap_shape_select` (Task 3). `cogmap_shape` (readback) ↔ `cogmap_shape_select` (api) ↔ `shape` (handler/client/cli) ↔ `cogmap_shape` (mcp tool) — names verified against this plan's own task interfaces.
- **Known verify-at-execution points (flagged, not placeholders):** (1) exact `parse_ref` module path (`temper_core` vs `temper_workflow`) — grep in Task 5 Step 2; (2) the CLI cogmap subcommand enum/dispatch names — grep in Task 6 Step 4; (3) `HttpClient::get` helper name — confirm against an existing GET sub-client in Task 4 Step 3; (4) substrate test `common` helper names (`seed_system`/`genesis_cogmap`/team helpers) — add the thin wrappers in Task 2 Step 1. Each has an exemplar cited.
