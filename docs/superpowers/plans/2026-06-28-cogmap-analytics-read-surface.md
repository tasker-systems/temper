# Cognitive-map analytics read surface — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the cognitive-map analytics read side — per-region scalar metrics and the map-level telos/staleness/regulation picture — across api + mcp + cli, mirroring the shipped `cogmap_shape` read vertical (PR #196).

**Architecture:** Two grain-aligned reads behind gated canonical SQL functions. `cogmap_region_metrics` is a gated column select over the materialized scalar columns on `kb_cogmap_regions` (the per-region analytics tier, sibling to `cogmap_shape`'s surface tier). `cogmap_analytics` composes the existing `cogmap_telos` / `cogmap_staleness` / `cogmap_regulation` functions into one gated map-level row. Reads stay service-direct (not the `Backend` trait); the access gate lives inside the SQL functions. Each surface layer (substrate readback → api service+handler → client → mcp → cli) is a thin pass-through mirroring the `cogmap_shape` path file-for-file.

**Tech Stack:** Rust (sqlx runtime queries, axum, utoipa, rmcp, clap), PostgreSQL (pgvector), ts-rs codegen, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-06-28-cogmap-analytics-read-surface-design.md`

## Global Constraints

- **Reads are service-direct** — never routed through the `Backend` trait. Never inline `sqlx::query!()` in a surface; SQL lives in the substrate readback layer (runtime `sqlx::query`, as `cogmap_shape` uses — the `jsonb`/`vector` involvement keeps these off the compile-time macros, so no `.sqlx` cache entry is needed).
- **Access gate lives in the canonical SQL functions only** — no inlined gate in any readback/api/handler code. region-metrics deny → empty `Vec` → `200 []`; analytics deny → `None` → `404`.
- **No `opposed_labels` on the read surface** — `internal_tension` is read from the stored column; opposed-labels is a materialization-time concern (deferred).
- **Shipped migrations are immutable** — add a NEW additive migration (`CREATE FUNCTION` only). Do not edit any applied migration file.
- **Wire types in temper-core** with the full derive stack matching `CogmapRegionRow`: `Debug, Clone, PartialEq, Serialize, Deserialize` + `FromRow` (row types only) + feature-gated `ts_rs::TS` (`typescript`), `utoipa::ToSchema` (`web-api`), `schemars::JsonSchema` (`mcp`). Timestamps are `chrono::DateTime<Utc>` (the temper-core precedent).
- **substrate cannot depend on temper-core** — substrate readback returns substrate-local structs; the temper-api wrapper maps them to the temper-core wire types.
- **Ref resolution** uses `temper_workflow::operations::parse_ref(x)?.0` (post-crate-split path; CLAUDE.md's `temper_core::operations` mention is stale).
- **Verification gate before PR:** `cargo make check` + unit + `test-db` + `test-artifacts`. Never run two `cargo make test-artifacts` concurrently (shared Postgres pool exhaustion → spurious setup-phase failures). For bare `cargo`/`nextest` against `#[sqlx::test]`, export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `migrations/20260628000001_cogmap_analytics_read_functions.sql` | Create | The two gated canonical SQL functions (additive). |
| `crates/temper-core/src/types/cognitive_maps.rs` | Modify | Wire types + MCP input types for both reads. |
| `crates/temper-substrate/src/readback/mod.rs` | Modify | Substrate-local row structs + `cogmap_region_metrics` / `cogmap_analytics` bindings. |
| `crates/temper-substrate/tests/cogmap_analytics_readback.rs` | Create | Artifact tests for both bindings (surface/deny/lens/metrics/regulation). |
| `crates/temper-api/src/backend/substrate_read.rs` | Modify | `cogmap_region_metrics_select` / `cogmap_analytics_select` wrappers (local→wire mapping). |
| `crates/temper-api/src/handlers/cognitive_maps.rs` | Modify | `region_metrics` / `analytics` axum handlers + utoipa annotations. |
| `crates/temper-api/src/routes.rs` | Modify | Register the two GET routes. |
| `crates/temper-api/src/openapi.rs` | Modify | Register handler paths + response schemas. |
| `crates/temper-api/tests/cogmap_analytics_handler_test.rs` | Create | api plumbing test against root-joined L0. |
| `crates/temper-client/src/cognitive_maps.rs` | Modify | `region_metrics` / `analytics` client methods + path builders. |
| `crates/temper-mcp/src/tools/cognitive_maps.rs` | Modify | `cogmap_region_metrics` / `cogmap_analytics` tool handlers. |
| `crates/temper-mcp/src/service.rs` | Modify | Register the two `#[tool]` methods. |
| `crates/temper-cli/src/cli.rs` | Modify | `CogmapCmd::RegionMetrics` / `CogmapCmd::Analytics` variants. |
| `crates/temper-cli/src/actions/cogmap.rs` | Modify | `region_metrics_api` / `analytics_api` thin client wrappers. |
| `crates/temper-cli/src/commands/cogmap.rs` | Modify | `region_metrics` / `analytics` command bodies. |
| `crates/temper-cli/src/main.rs` | Modify | Dispatch the two new `CogmapCmd` arms. |

---

## Task 1: temper-core wire types + MCP input types

**Files:**
- Modify: `crates/temper-core/src/types/cognitive_maps.rs` (append after `CogmapRegionRow`, before `#[cfg(test)]`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `CogmapRegionMetricsRow { region_id: Uuid, lens_id: Uuid, centrality: Option<f64>, content_cohesion: Option<f64>, internal_tension: Option<f64>, reference_standing: Option<f64>, telos_alignment: Option<f64> }`
  - `CogmapStaleness { materialized_at: Option<DateTime<Utc>>, latest_touch: Option<DateTime<Utc>>, is_stale: bool }`
  - `CogmapRegulationRow { resource_id: Uuid, title: String, body_text: Option<String>, edge_label: String }`
  - `CogmapAnalyticsRow { telos_resource_id: Uuid, staleness: CogmapStaleness, regulation: Vec<CogmapRegulationRow> }`
  - `CogmapRegionMetricsInput { cogmap: String, lens: Option<String> }`
  - `CogmapAnalyticsInput { cogmap: String }`

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/temper-core/src/types/cognitive_maps.rs`:

```rust
    #[test]
    fn cogmap_region_metrics_row_serde_roundtrip_preserves_nullables() {
        let row = CogmapRegionMetricsRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            centrality: Some(4.0),
            content_cohesion: None,
            internal_tension: Some(1.5),
            reference_standing: Some(0.0),
            telos_alignment: None,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        assert!(json.contains("\"content_cohesion\":null"), "json: {json}");
        assert!(json.contains("\"internal_tension\":1.5"), "json: {json}");
        let back: CogmapRegionMetricsRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }

    #[test]
    fn cogmap_analytics_row_nests_staleness_and_regulation() {
        let row = CogmapAnalyticsRow {
            telos_resource_id: Uuid::from_u128(9),
            staleness: CogmapStaleness {
                materialized_at: None,
                latest_touch: None,
                is_stale: true,
            },
            regulation: vec![CogmapRegulationRow {
                resource_id: Uuid::from_u128(3),
                title: "Deploy safely".to_string(),
                body_text: Some("body".to_string()),
                edge_label: "operationalized_by".to_string(),
            }],
        };
        let json = serde_json::to_string(&row).expect("serialize");
        assert!(json.contains("\"is_stale\":true"), "json: {json}");
        assert!(json.contains("\"edge_label\":\"operationalized_by\""), "json: {json}");
        let back: CogmapAnalyticsRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core cogmap_region_metrics_row_serde_roundtrip_preserves_nullables`
Expected: FAIL — `cannot find type CogmapRegionMetricsRow`.

- [ ] **Step 3: Write minimal implementation**

In `crates/temper-core/src/types/cognitive_maps.rs`, add the chrono import near the top imports:

```rust
use chrono::{DateTime, Utc};
```

Then append, after the `CogmapRegionRow` struct (before the `#[cfg(test)]` module):

```rust
/// MCP/surface input for the per-region analytics read. `cogmap` is a ref (UUID or decorated
/// `sluggify(title)-<uuid>`); `lens` is an optional lens ref to narrow the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionMetricsInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// Optional lens ref to filter regions; omit for all lenses.
    pub lens: Option<String>,
}

/// MCP/surface input for the map-level analytics read. `cogmap` is a ref (UUID or decorated form).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapAnalyticsInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
}

/// The per-region analytics tier (the five materialized scalar readouts) as returned by
/// `cogmap_region_metrics`. Sibling to `CogmapRegionRow`'s surface tier; member identities are still
/// never carried. Each metric is `Option<f64>` (the columns are nullable until materialization computes
/// them; `telos_alignment` stays `None` when the telos has no embedded chunks).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionMetricsRow {
    /// `kb_cogmap_regions.id` — the region's stable identity.
    pub region_id: Uuid,
    /// The lens (perspective) that produced this region.
    pub lens_id: Uuid,
    /// Internal declared-affinity mass × size.
    pub centrality: Option<f64>,
    /// Mean member-to-centroid cosine.
    pub content_cohesion: Option<f64>,
    /// Summed weight of opposed (`contradicts`) declared edges among members — tension binds, never fractures.
    pub internal_tension: Option<f64>,
    /// Summed reinforce_count over member blocks.
    pub reference_standing: Option<f64>,
    /// Cosine of the region centroid to the cogmap's telos-resource embedding.
    pub telos_alignment: Option<f64>,
}

/// Map-level staleness readout (`cogmap_staleness`): when the shape was last materialized, the latest
/// touch to the map's regions/edges, and whether the read is stale. Staleness is LEGIBLE — reported,
/// never blocking. `materialized_at` is `None` when the map has never been materialized.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapStaleness {
    pub materialized_at: Option<DateTime<Utc>>,
    pub latest_touch: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

/// One regulation concept (`cogmap_regulation`): a concept-resource the charter `express`-edges to
/// (label e.g. `operationalized_by`), filtered to those the principal can read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegulationRow {
    pub resource_id: Uuid,
    pub title: String,
    pub body_text: Option<String>,
    pub edge_label: String,
}

/// The map-level analytics picture as returned by `cogmap_analytics`: the telos charter resource id,
/// staleness, and the regulation set. Per-region scalar metrics are a SEPARATE read
/// (`cogmap_region_metrics`). The access gate is INSIDE the SQL: a principal who cannot read the map
/// gets zero rows, surfaced here as `None` (→ 404 at the api boundary).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapAnalyticsRow {
    /// `kb_cogmaps.telos_resource_id` — the charter resource (NOT NULL).
    pub telos_resource_id: Uuid,
    pub staleness: CogmapStaleness,
    pub regulation: Vec<CogmapRegulationRow>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core cogmap`
Expected: PASS (the two new tests + the existing `cogmap_*` tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/cognitive_maps.rs
git commit -m "feat(core): cogmap analytics wire types (region-metrics, analytics, staleness, regulation)"
```

---

## Task 2: SQL migration + substrate readback bindings + artifact tests

**Files:**
- Create: `migrations/20260628000001_cogmap_analytics_read_functions.sql`
- Modify: `crates/temper-substrate/src/readback/mod.rs` (append after `cogmap_shape`, ~line 789)
- Create: `crates/temper-substrate/tests/cogmap_analytics_readback.rs`

**Interfaces:**
- Consumes: `temper_substrate::ids::{CogmapId, ProfileId}`, the `system`/genesis/team helpers in `tests/common/mod.rs`.
- Produces (substrate-local, in `readback`):
  - `pub struct CogmapRegionMetricsRow { region_id: Uuid, lens_id: Uuid, centrality: Option<f64>, content_cohesion: Option<f64>, internal_tension: Option<f64>, reference_standing: Option<f64>, telos_alignment: Option<f64> }`
  - `pub struct CogmapStaleness { materialized_at: Option<DateTime<Utc>>, latest_touch: Option<DateTime<Utc>>, is_stale: bool }`
  - `pub struct CogmapRegulationRow { resource_id: Uuid, title: String, body_text: Option<String>, edge_label: String }` — derives `Deserialize` (decoded from the `json_agg` column).
  - `pub struct CogmapAnalyticsRow { telos_resource_id: Uuid, staleness: CogmapStaleness, regulation: Vec<CogmapRegulationRow> }`
  - `pub async fn cogmap_region_metrics(pool: &PgPool, cogmap_id: CogmapId, principal: ProfileId, lens_id: Option<Uuid>) -> Result<Vec<CogmapRegionMetricsRow>>`
  - `pub async fn cogmap_analytics(pool: &PgPool, cogmap_id: CogmapId, principal: ProfileId) -> Result<Option<CogmapAnalyticsRow>>`

- [ ] **Step 1: Write the migration**

Create `migrations/20260628000001_cogmap_analytics_read_functions.sql`:

```sql
-- Cognitive-map analytics read side (task 019ee5a4, WS7). Additive: two gated canonical read
-- functions, siblings to cogmap_shape. The access gate lives INSIDE each function (cogmap_shape's
-- "no view from nowhere" pattern): a principal who cannot read the map gets zero rows, never an error.

-- Per-region analytics tier: the five materialized scalar readouts, read from the stored
-- kb_cogmap_regions columns (NOT recomputed). Gate + lens filter IDENTICAL to cogmap_shape.
CREATE FUNCTION cogmap_region_metrics(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, centrality double precision,
              content_cohesion double precision, internal_tension double precision,
              reference_standing double precision, telos_alignment double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.centrality, reg.content_cohesion,
           reg.internal_tension, reg.reference_standing, reg.telos_alignment
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      AND (
        (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
        OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap)
      );
$$;

-- Map-level analytics: telos charter id + staleness + the regulation set, composed from the existing
-- canonical functions in one gated row. cogmap_staleness yields exactly one row, so the map-readable
-- gate in WHERE makes the whole function deny → zero rows. regulation defaults to [] (never SQL-null).
CREATE FUNCTION cogmap_analytics(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(telos_resource_id uuid, materialized_at timestamptz,
              latest_touch timestamptz, is_stale boolean, regulation jsonb)
LANGUAGE sql STABLE AS $$
    SELECT cogmap_telos(p_cogmap),
           s.materialized_at, s.latest_touch, s.is_stale,
           COALESCE(
             (SELECT json_agg(r) FROM cogmap_regulation(p_cogmap, p_principal_kind, p_principal_id) r),
             '[]'::json)::jsonb
    FROM cogmap_staleness(p_cogmap) s
    WHERE (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
       OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap);
$$;
```

Confirm the filename sorts after the current last migration (`20260627000003_*`): `ls migrations/ | tail`.

- [ ] **Step 2: Write the failing tests**

Create `crates/temper-substrate/tests/cogmap_analytics_readback.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! `readback::cogmap_region_metrics` + `readback::cogmap_analytics` — the analytics read side.
//! Proves: per-region metrics surface (stored columns) for a readable principal, are gated (deny →
//! empty), are lens-filtered, and exclude folded regions; the map-level analytics row carries the
//! telos id + staleness, surfaces a readable regulation edge through the `json_agg` composition, and
//! denies a non-member (None, not an error).

use sqlx::PgPool;
use temper_substrate::ids::{CogmapId, ProfileId};
use uuid::Uuid;

mod common;

/// Insert one region with explicit metric columns (analytics reads stored columns, so seed them
/// directly — no materialization run needed). `centroid` is an all-zero 768-vector. Returns its id.
struct MetricSeed {
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    centrality: f64,
    internal_tension: f64,
    is_folded: bool,
}

async fn insert_region_with_metrics(pool: &PgPool, s: MetricSeed) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, centrality, content_cohesion,
            internal_tension, reference_standing, telos_alignment, label, member_count,
            asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2, array_fill(0::double precision, ARRAY[768])::vector, 0.5, $3, 0.25,
            $4, 7.0, 0.9, 'r', 2, $5, $5, $6)
         RETURNING id",
    )
    .bind(s.cogmap)
    .bind(s.lens)
    .bind(s.centrality)
    .bind(s.internal_tension)
    .bind(s.event)
    .bind(s.is_folded)
    .fetch_one(pool)
    .await
    .expect("insert region with metrics")
}

/// Shared fixture: a genesis cogmap joined to a fresh team; p1 is a member (readable), p2 is not.
/// Returns (cogmap, telos, lens, event, p1, p2).
async fn fixture(pool: &PgPool) -> (Uuid, Uuid, Uuid, Uuid, Uuid, Uuid) {
    common::seed_system(pool).await;
    let (cogmap, telos) = common::genesis_cogmap(pool, "analytics-test", "Analytics Test").await;
    let team = common::create_team(pool, "analytics-team").await;
    let p1 = common::create_profile(pool, "member@example.com").await;
    let p2 = common::create_profile(pool, "outsider@example.com").await;
    common::add_team_member(pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .expect("join cogmap to team");
    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("global telos-default lens");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("any event for FK");
    (cogmap, telos, lens, event, p1, p2)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn region_metrics_surface_gate_and_lens(pool: PgPool) {
    let (cogmap, _telos, lens, event, p1, p2) = fixture(&pool).await;

    let kept = insert_region_with_metrics(
        &pool,
        MetricSeed { cogmap, lens, event, centrality: 4.0, internal_tension: 1.5, is_folded: false },
    )
    .await;
    let _folded = insert_region_with_metrics(
        &pool,
        MetricSeed { cogmap, lens, event, centrality: 9.0, internal_tension: 0.0, is_folded: true },
    )
    .await;

    // Readable principal sees exactly the non-folded region, with the stored scalars.
    let rows = temper_substrate::readback::cogmap_region_metrics(
        &pool, CogmapId::from(cogmap), ProfileId::from(p1), None,
    )
    .await
    .expect("readable read");
    assert_eq!(rows.len(), 1, "only the non-folded region surfaces: {rows:?}");
    assert_eq!(rows[0].region_id, kept);
    assert_eq!(rows[0].centrality, Some(4.0));
    assert_eq!(rows[0].internal_tension, Some(1.5), "tension surfaces from the stored column");
    assert_eq!(rows[0].reference_standing, Some(7.0));

    // Non-member is denied by the in-SQL gate: zero rows, not an error.
    let denied = temper_substrate::readback::cogmap_region_metrics(
        &pool, CogmapId::from(cogmap), ProfileId::from(p2), None,
    )
    .await
    .expect("gate denial is empty, not an error");
    assert!(denied.is_empty(), "non-member sees no metrics: {denied:?}");

    // Wrong lens narrows to empty.
    let filtered = temper_substrate::readback::cogmap_region_metrics(
        &pool, CogmapId::from(cogmap), ProfileId::from(p1), Some(Uuid::now_v7()),
    )
    .await
    .expect("lens-filtered read");
    assert!(filtered.is_empty(), "wrong lens yields no metrics: {filtered:?}");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn analytics_telos_staleness_regulation_and_deny(pool: PgPool) {
    let (cogmap, telos, _lens, event, p1, p2) = fixture(&pool).await;

    // Seed a readable regulation edge: a target resource OWNED by p1 (→ visible via
    // resources_visible_to), and an `express` edge telos → target labeled `operationalized_by`.
    let target: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('Deploy safely', 'temper://reg/t') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert target resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(target)
    .bind(cogmap)
    .bind(p1)
    .execute(&pool)
    .await
    .expect("home target to p1");
    sqlx::query(
        "INSERT INTO kb_edges
           (source_table, source_id, target_table, target_id, edge_kind, label,
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
         VALUES ('kb_resources', $1, 'kb_resources', $2, 'express', 'operationalized_by',
            'kb_cogmaps', $3, $4, $4)",
    )
    .bind(telos)
    .bind(target)
    .bind(cogmap)
    .bind(event)
    .execute(&pool)
    .await
    .expect("insert express edge");

    // Readable principal: telos id, staleness present, regulation carries the one readable concept.
    let got = temper_substrate::readback::cogmap_analytics(
        &pool, CogmapId::from(cogmap), ProfileId::from(p1),
    )
    .await
    .expect("readable analytics read")
    .expect("readable principal gets Some");
    assert_eq!(got.telos_resource_id, telos);
    assert!(got.staleness.is_stale, "never-materialized map reads as stale");
    assert_eq!(got.regulation.len(), 1, "one readable regulation concept: {:?}", got.regulation);
    assert_eq!(got.regulation[0].resource_id, target);
    assert_eq!(got.regulation[0].edge_label, "operationalized_by");
    assert_eq!(got.regulation[0].title, "Deploy safely");

    // Non-member: the in-SQL gate yields zero rows → None.
    let denied = temper_substrate::readback::cogmap_analytics(
        &pool, CogmapId::from(cogmap), ProfileId::from(p2),
    )
    .await
    .expect("gate denial is None, not an error");
    assert!(denied.is_none(), "non-member must get None");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run (export `DATABASE_URL` first per Global Constraints):
`cargo nextest run -p temper-substrate --features artifact-tests --test cogmap_analytics_readback`
Expected: FAIL — `cannot find function cogmap_region_metrics in module readback`.

- [ ] **Step 4: Write the readback bindings**

In `crates/temper-substrate/src/readback/mod.rs`, after `cogmap_shape` (ends ~line 789), append. The `use chrono::{DateTime, Utc};` import already exists at the top of the file (line 24) — do not re-add it. Add `use serde::Deserialize;` to the file's imports if not present.

```rust
/// One region's analytics-tier scalar metrics, as returned by `cogmap_region_metrics`. The stored
/// readout columns of `kb_cogmap_regions` (computed once at materialization). Substrate-local; the
/// `temper-api` wrapper maps this to the `CogmapRegionMetricsRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapRegionMetricsRow {
    pub region_id: Uuid,
    pub lens_id: Uuid,
    pub centrality: Option<f64>,
    pub content_cohesion: Option<f64>,
    pub internal_tension: Option<f64>,
    pub reference_standing: Option<f64>,
    pub telos_alignment: Option<f64>,
}

/// The per-region analytics tier read (`cogmap_region_metrics`). Gate IS in the SQL (deny → empty);
/// folded regions excluded; `lens_id = None` → all lenses, `Some(l)` → that lens. Runtime `sqlx::query`.
pub async fn cogmap_region_metrics(
    pool: &PgPool,
    cogmap_id: CogmapId,
    principal: ProfileId,
    lens_id: Option<Uuid>,
) -> Result<Vec<CogmapRegionMetricsRow>> {
    let rows = sqlx::query(
        "SELECT region_id, lens_id, centrality, content_cohesion, internal_tension,
                reference_standing, telos_alignment
           FROM cogmap_region_metrics($1, 'profile', $2, $3)",
    )
    .bind(cogmap_id)
    .bind(principal)
    .bind(lens_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| CogmapRegionMetricsRow {
            region_id: r.get("region_id"),
            lens_id: r.get("lens_id"),
            centrality: r.get("centrality"),
            content_cohesion: r.get("content_cohesion"),
            internal_tension: r.get("internal_tension"),
            reference_standing: r.get("reference_standing"),
            telos_alignment: r.get("telos_alignment"),
        })
        .collect())
}

/// One regulation concept from `cogmap_analytics`'s `json_agg` column. `Deserialize` decodes it out of
/// the aggregated `jsonb`; field names match the `cogmap_regulation` return columns.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CogmapRegulationRow {
    pub resource_id: Uuid,
    pub title: String,
    pub body_text: Option<String>,
    pub edge_label: String,
}

/// Map-level staleness, mirrored from `cogmap_staleness` columns. Substrate-local.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapStaleness {
    pub materialized_at: Option<DateTime<Utc>>,
    pub latest_touch: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

/// The map-level analytics picture (`cogmap_analytics`): telos id + staleness + regulation set.
/// Substrate-local; the `temper-api` wrapper maps this to the `CogmapAnalyticsRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapAnalyticsRow {
    pub telos_resource_id: Uuid,
    pub staleness: CogmapStaleness,
    pub regulation: Vec<CogmapRegulationRow>,
}

/// The map-level analytics read (`cogmap_analytics`). Gate IS in the SQL: a principal who cannot read
/// the map gets zero rows → `None` (never an error). Runtime `sqlx::query`; `regulation` is decoded
/// from the function's `json_agg` `jsonb` column.
pub async fn cogmap_analytics(
    pool: &PgPool,
    cogmap_id: CogmapId,
    principal: ProfileId,
) -> Result<Option<CogmapAnalyticsRow>> {
    let row = sqlx::query(
        "SELECT telos_resource_id, materialized_at, latest_touch, is_stale, regulation
           FROM cogmap_analytics($1, 'profile', $2)",
    )
    .bind(cogmap_id)
    .bind(principal)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let regulation: sqlx::types::Json<Vec<CogmapRegulationRow>> = r.get("regulation");
        CogmapAnalyticsRow {
            telos_resource_id: r.get("telos_resource_id"),
            staleness: CogmapStaleness {
                materialized_at: r.get("materialized_at"),
                latest_touch: r.get("latest_touch"),
                is_stale: r.get("is_stale"),
            },
            regulation: regulation.0,
        }
    }))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test cogmap_analytics_readback`
Expected: PASS (both tests). Run alone — do not run concurrently with another artifact-test invocation.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260628000001_cogmap_analytics_read_functions.sql \
        crates/temper-substrate/src/readback/mod.rs \
        crates/temper-substrate/tests/cogmap_analytics_readback.rs
git commit -m "feat(substrate): cogmap_region_metrics + cogmap_analytics read bindings (gated SQL)"
```

---

## Task 3: temper-api service wrappers + handlers + routes + openapi + plumbing test

**Files:**
- Modify: `crates/temper-api/src/backend/substrate_read.rs` (append after `cogmap_shape_select`, ~line 418)
- Modify: `crates/temper-api/src/handlers/cognitive_maps.rs` (append after `shape`)
- Modify: `crates/temper-api/src/routes.rs` (after the `/shape` route, ~line 92)
- Modify: `crates/temper-api/src/openapi.rs` (paths ~line 41, schemas ~line 82)
- Create: `crates/temper-api/tests/cogmap_analytics_handler_test.rs`

**Interfaces:**
- Consumes: `readback::{cogmap_region_metrics, cogmap_analytics, CogmapRegionMetricsRow as SubMetricsRow, CogmapAnalyticsRow as SubAnalyticsRow}`; wire types from Task 1.
- Produces:
  - `pub async fn cogmap_region_metrics_select(pool: &PgPool, profile_id: ProfileId, cogmap_id: Uuid, lens_id: Option<Uuid>) -> ApiResult<Vec<CogmapRegionMetricsRow>>`
  - `pub async fn cogmap_analytics_select(pool: &PgPool, profile_id: ProfileId, cogmap_id: Uuid) -> ApiResult<Option<CogmapAnalyticsRow>>`
  - Handlers `region_metrics`, `analytics`; routes `GET /api/cognitive-maps/{id}/region-metrics`, `GET /api/cognitive-maps/{id}/analytics`.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/cogmap_analytics_handler_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! `substrate_read::{cogmap_region_metrics_select, cogmap_analytics_select}` — the api-side
//! service-direct wrappers. Proves the readable path returns Ok against the root-joined L0 map (region
//! metrics empty until materialized; analytics Some with the L0 telos), and a non-readable map yields
//! empty / None.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::substrate_read::{cogmap_analytics_select, cogmap_region_metrics_select};
use temper_core::types::ids::ProfileId;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);
const L0_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_region_metrics_readable_empty(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "reader1@example.com").await;
    let rows = cogmap_region_metrics_select(&pool, ProfileId::from(profile), L0_COGMAP, None)
        .await
        .expect("readable L0 region-metrics must be Ok");
    assert!(rows.is_empty(), "L0 has no materialized regions yet: {rows:?}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_analytics_readable_some_with_telos(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "reader2@example.com").await;
    let got = cogmap_analytics_select(&pool, ProfileId::from(profile), L0_COGMAP)
        .await
        .expect("readable L0 analytics must be Ok")
        .expect("L0 is root-joined → Some");
    assert_eq!(got.telos_resource_id, L0_TELOS, "L0 telos charter resource");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unknown_cogmap_metrics_empty_analytics_none(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let unknown = Uuid::now_v7();
    let metrics = cogmap_region_metrics_select(&pool, ProfileId::from(profile), unknown, None)
        .await
        .expect("non-readable map metrics is empty, not an error");
    assert!(metrics.is_empty());
    let analytics = cogmap_analytics_select(&pool, ProfileId::from(profile), unknown)
        .await
        .expect("non-readable map analytics is None, not an error");
    assert!(analytics.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_analytics_handler_test`
Expected: FAIL — `cannot find function cogmap_region_metrics_select`.

- [ ] **Step 3: Write the service wrappers**

In `crates/temper-api/src/backend/substrate_read.rs`, after `cogmap_shape_select` (~line 418), append. Mirror its imports (`CogmapId`, `readback`, `api_err`); add the new wire-type imports (`CogmapRegionMetricsRow`, `CogmapAnalyticsRow`, `CogmapStaleness`, `CogmapRegulationRow`) to the existing `temper_core::types::cognitive_maps::…` use at the top of the file.

```rust
/// `cogmap_region_metrics` — the per-region analytics tier. Service-direct; gate is in the SQL
/// (deny → empty). Maps the substrate-local row to the wire type.
pub async fn cogmap_region_metrics_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionMetricsRow>> {
    let rows = readback::cogmap_region_metrics(pool, CogmapId::from(cogmap_id), profile_id, lens_id)
        .await
        .map_err(api_err)?;
    Ok(rows
        .into_iter()
        .map(|r| CogmapRegionMetricsRow {
            region_id: r.region_id,
            lens_id: r.lens_id,
            centrality: r.centrality,
            content_cohesion: r.content_cohesion,
            internal_tension: r.internal_tension,
            reference_standing: r.reference_standing,
            telos_alignment: r.telos_alignment,
        })
        .collect())
}

/// `cogmap_analytics` — the map-level analytics picture. Service-direct; gate is in the SQL
/// (deny → `None`, surfaced as 404 by the handler). Maps the substrate-local row to the wire type.
pub async fn cogmap_analytics_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
) -> ApiResult<Option<CogmapAnalyticsRow>> {
    let got = readback::cogmap_analytics(pool, CogmapId::from(cogmap_id), profile_id)
        .await
        .map_err(api_err)?;
    Ok(got.map(|a| CogmapAnalyticsRow {
        telos_resource_id: a.telos_resource_id,
        staleness: CogmapStaleness {
            materialized_at: a.staleness.materialized_at,
            latest_touch: a.staleness.latest_touch,
            is_stale: a.staleness.is_stale,
        },
        regulation: a
            .regulation
            .into_iter()
            .map(|r| CogmapRegulationRow {
                resource_id: r.resource_id,
                title: r.title,
                body_text: r.body_text,
                edge_label: r.edge_label,
            })
            .collect(),
    }))
}
```

- [ ] **Step 4: Run the service-wrapper test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_analytics_handler_test`
Expected: PASS (all three tests).

- [ ] **Step 5: Add the handlers**

In `crates/temper-api/src/handlers/cognitive_maps.rs`, extend the `use temper_core::types::cognitive_maps::…` line to also import `CogmapAnalyticsRow, CogmapRegionMetricsRow`, then append after `shape`:

```rust
#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/region-metrics",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Per-region analytics-tier scalar metrics", body = Vec<CogmapRegionMetricsRow>),
        (status = 401, description = "Unauthorized", body = crate::error::ErrorBody),
    )
)]
pub async fn region_metrics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionMetricsRow>>> {
    crate::backend::substrate_read::cogmap_region_metrics_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/analytics",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Map-level analytics (telos, staleness, regulation)", body = CogmapAnalyticsRow),
        (status = 404, description = "Map not found or not readable"),
        (status = 401, description = "Unauthorized", body = crate::error::ErrorBody),
    )
)]
pub async fn analytics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
) -> ApiResult<Json<CogmapAnalyticsRow>> {
    crate::backend::substrate_read::cogmap_analytics_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
    )
    .await?
    .map(Json)
    .ok_or(ApiError::NotFound)
}
```

(`ApiError` is already imported in this file — line 15. `ShapeQuery` is already defined here — reuse it for region-metrics.)

- [ ] **Step 6: Register the routes**

In `crates/temper-api/src/routes.rs`, after the `/api/cognitive-maps/{id}/shape` route (~line 92), add two routes mirroring it:

```rust
        .route(
            "/api/cognitive-maps/{id}/region-metrics",
            get(handlers::cognitive_maps::region_metrics),
        )
        .route(
            "/api/cognitive-maps/{id}/analytics",
            get(handlers::cognitive_maps::analytics),
        )
```

- [ ] **Step 7: Register OpenAPI paths + schemas**

In `crates/temper-api/src/openapi.rs`: after `crate::handlers::cognitive_maps::shape,` (line 41) add:

```rust
        crate::handlers::cognitive_maps::region_metrics,
        crate::handlers::cognitive_maps::analytics,
```

After `temper_core::types::cognitive_maps::CogmapRegionRow,` (line 82) add:

```rust
        temper_core::types::cognitive_maps::CogmapRegionMetricsRow,
        temper_core::types::cognitive_maps::CogmapAnalyticsRow,
        temper_core::types::cognitive_maps::CogmapStaleness,
        temper_core::types::cognitive_maps::CogmapRegulationRow,
```

Update the openapi assertion test (~line 148) — add an assertion alongside the existing `/shape` one:

```rust
        assert!(json.contains("/api/cognitive-maps/{id}/region-metrics"));
        assert!(json.contains("/api/cognitive-maps/{id}/analytics"));
```

- [ ] **Step 8: Verify the crate builds and the openapi test passes**

Run: `cargo nextest run -p temper-api --features test-db --test cogmap_analytics_handler_test`
Run: `cargo nextest run -p temper-api openapi`
Expected: PASS. Then `cargo check -p temper-api --all-features` — clean.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/src/backend/substrate_read.rs \
        crates/temper-api/src/handlers/cognitive_maps.rs \
        crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs \
        crates/temper-api/tests/cogmap_analytics_handler_test.rs
git commit -m "feat(api): region-metrics + analytics endpoints (service-direct, gated, 404-on-deny)"
```

---

## Task 4: temper-client methods

**Files:**
- Modify: `crates/temper-client/src/cognitive_maps.rs` (add methods + path builders + tests)

**Interfaces:**
- Consumes: `CogmapRegionMetricsRow`, `CogmapAnalyticsRow` (Task 1).
- Produces: `CognitiveMapClient::region_metrics(cogmap_id, lens_id) -> Result<Vec<CogmapRegionMetricsRow>>`, `::analytics(cogmap_id) -> Result<CogmapAnalyticsRow>`.

- [ ] **Step 1: Write the failing tests**

In `crates/temper-client/src/cognitive_maps.rs`, add to the `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn region_metrics_path_omits_and_includes_lens() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            region_metrics_path(id, None),
            format!("/api/cognitive-maps/{id}/region-metrics")
        );
        let lens = Uuid::from_u128(9);
        assert_eq!(
            region_metrics_path(id, Some(lens)),
            format!("/api/cognitive-maps/{id}/region-metrics?lens={lens}")
        );
    }

    #[test]
    fn analytics_path_is_plain() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            analytics_path(id),
            format!("/api/cognitive-maps/{id}/analytics")
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-client region_metrics_path_omits_and_includes_lens`
Expected: FAIL — `cannot find function region_metrics_path`.

- [ ] **Step 3: Add methods + path builders**

Extend the `use temper_core::types::cognitive_maps::…` line (line 12) to also import `CogmapAnalyticsRow, CogmapRegionMetricsRow`. Inside `impl<'a> CognitiveMapClient<'a>`, after `shape`, add:

```rust
    /// GET /api/cognitive-maps/{id}/region-metrics[?lens=] — the per-region analytics tier (the five
    /// scalar metrics). Empty if the principal cannot read the map.
    pub async fn region_metrics(
        &self,
        cogmap_id: Uuid,
        lens_id: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionMetricsRow>> {
        let token = self.http.resolve_token()?;
        let path = region_metrics_path(cogmap_id, lens_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id}/analytics — the map-level analytics picture (telos id, staleness,
    /// regulation). 404 if the map is not found or not readable.
    pub async fn analytics(&self, cogmap_id: Uuid) -> Result<CogmapAnalyticsRow> {
        let token = self.http.resolve_token()?;
        let path = analytics_path(cogmap_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
```

After the existing `shape_path` fn (~line 64), add:

```rust
/// `/api/cognitive-maps/{id}/region-metrics` with an optional `?lens=` query.
fn region_metrics_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/region-metrics");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}

/// `/api/cognitive-maps/{id}/analytics`.
fn analytics_path(cogmap_id: Uuid) -> String {
    format!("/api/cognitive-maps/{cogmap_id}/analytics")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-client cognitive`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-client/src/cognitive_maps.rs
git commit -m "feat(client): CognitiveMapClient::region_metrics + ::analytics"
```

---

## Task 5: temper-mcp tools

**Files:**
- Modify: `crates/temper-mcp/src/tools/cognitive_maps.rs` (add two handlers)
- Modify: `crates/temper-mcp/src/service.rs` (register two `#[tool]` methods)

**Interfaces:**
- Consumes: `CogmapRegionMetricsInput`, `CogmapAnalyticsInput` (Task 1); `substrate_read::{cogmap_region_metrics_select, cogmap_analytics_select}` (Task 3).
- Produces: tool handlers `cogmap_region_metrics`, `cogmap_analytics`.

- [ ] **Step 1: Add the tool handlers**

In `crates/temper-mcp/src/tools/cognitive_maps.rs`, extend the `use temper_core::types::cognitive_maps::…` import to add `CogmapAnalyticsInput, CogmapRegionMetricsInput`, then append after `cogmap_shape`:

```rust
pub async fn cogmap_region_metrics(
    svc: &TemperMcpService,
    input: CogmapRegionMetricsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

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

    let rows = temper_api::backend::substrate_read::cogmap_region_metrics_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        lens_id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_region_metrics failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

pub async fn cogmap_analytics(
    svc: &TemperMcpService,
    input: CogmapAnalyticsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let got = temper_api::backend::substrate_read::cogmap_analytics_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_analytics failed: {e}"), None))?;

    match got {
        Some(analytics) => {
            let text = serde_json::to_string_pretty(&analytics)
                .unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
        }
        None => Err(rmcp::ErrorData::invalid_params(
            "cognitive map not found or not readable".to_string(),
            None,
        )),
    }
}
```

- [ ] **Step 2: Register the tools**

In `crates/temper-mcp/src/service.rs`, after the `cogmap_shape` `#[tool]` method (~line 281), add:

```rust
    #[tool(
        description = "Read a cognitive map's per-region analytics metrics (centrality, content cohesion, internal tension, reference standing, telos alignment) under an optional lens. Pass the map by ref."
    )]
    async fn cogmap_region_metrics(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapRegionMetricsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_region_metrics(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's map-level analytics: its telos charter resource id, staleness, and the regulation concept set. Pass the map by ref."
    )]
    async fn cogmap_analytics(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapAnalyticsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_analytics(self, input).await
    }
```

- [ ] **Step 3: Verify the crate builds**

Run: `cargo check -p temper-mcp --all-features`
Expected: clean. (rmcp aggregates `#[tool]` methods on the impl via its macro; no manual registry edit needed — confirm the existing `cogmap_shape` registration required no other site, which it did not.)

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp/src/tools/cognitive_maps.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): cogmap_region_metrics + cogmap_analytics tools"
```

---

## Task 6: temper-cli command

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`CogmapCmd` variants, ~line 456)
- Modify: `crates/temper-cli/src/actions/cogmap.rs` (client wrappers + tests)
- Modify: `crates/temper-cli/src/commands/cogmap.rs` (command bodies)
- Modify: `crates/temper-cli/src/main.rs` (dispatch arms, ~line 380)

**Interfaces:**
- Consumes: `client.cognitive_maps().region_metrics/analytics` (Task 4); `parse_ref`; `OutputFormat`/`render`.
- Produces: CLI `temper cogmap region-metrics <ref> [--lens <ref>]`, `temper cogmap analytics <ref>`.

- [ ] **Step 1: Write the failing test**

In `crates/temper-cli/src/actions/cogmap.rs`, add to `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn render_region_metrics_rows_json_is_passthrough_array() {
        use temper_core::types::cognitive_maps::CogmapRegionMetricsRow;
        let rows: Vec<CogmapRegionMetricsRow> = vec![CogmapRegionMetricsRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            centrality: Some(4.0),
            content_cohesion: None,
            internal_tension: Some(1.5),
            reference_standing: Some(7.0),
            telos_alignment: None,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"internal_tension\""), "json: {out}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli render_region_metrics_rows_json_is_passthrough_array`
Expected: FAIL to compile (test references nothing new yet — it will actually pass since it only uses Task 1 types; if it passes, that's fine — it guards the render path). If it passes immediately, proceed; the failing-first signal for this task is the missing CLI wiring in the next steps.

- [ ] **Step 3: Add the action wrappers**

In `crates/temper-cli/src/actions/cogmap.rs`, extend the import to add `CogmapAnalyticsRow, CogmapRegionMetricsRow`, then after `shape_api`:

```rust
/// Call the region-metrics API for the given cogmap (and optional lens), both resolved to UUIDs.
pub async fn region_metrics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionMetricsRow>> {
    client
        .cognitive_maps()
        .region_metrics(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the analytics API for the given cogmap (resolved to a UUID).
pub async fn analytics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
) -> Result<CogmapAnalyticsRow> {
    client
        .cognitive_maps()
        .analytics(cogmap_id)
        .await
        .map_err(crate::commands::client_err)
}
```

- [ ] **Step 4: Add the command bodies**

In `crates/temper-cli/src/commands/cogmap.rs`, after `shape`:

```rust
/// `temper cogmap region-metrics <cogmap_ref> [--lens <ref>]` — read the per-region analytics metrics.
pub fn region_metrics(cogmap_ref: &str, lens_ref: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let lens_id = lens_ref
        .map(|l| temper_workflow::operations::parse_ref(l).map(|p| p.0))
        .transpose()?;

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            crate::actions::cogmap::region_metrics_api(client, cogmap_id, lens_id).await
        })
    })?;

    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap analytics <cogmap_ref>` — read the map-level analytics (telos, staleness, regulation).
pub fn analytics(cogmap_ref: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::analytics_api(client, cogmap_id).await })
    })?;

    let rendered = crate::format::render(&row, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
```

- [ ] **Step 5: Add the CLI enum variants**

In `crates/temper-cli/src/cli.rs`, inside `enum CogmapCmd`, after `Shape { … }` (before the closing `}` at line 457):

```rust
    /// Read a cognitive map's per-region analytics metrics.
    RegionMetrics {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Optional lens ref to filter regions.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Read a cognitive map's map-level analytics (telos, staleness, regulation).
    Analytics {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
    },
```

- [ ] **Step 6: Add the dispatch arms**

In `crates/temper-cli/src/main.rs`, inside the `Commands::Cogmap { cmd } => match cmd { … }` block, after the `CogmapCmd::Shape` arm (~line 380):

```rust
            CogmapCmd::RegionMetrics { cogmap, lens } => {
                commands::cogmap::region_metrics(&cogmap, lens.as_deref(), output_format)
            }
            CogmapCmd::Analytics { cogmap } => {
                commands::cogmap::analytics(&cogmap, output_format)
            }
```

- [ ] **Step 7: Run tests + build to verify**

Run: `cargo nextest run -p temper-cli cogmap`
Run: `cargo check -p temper-cli --all-features`
Expected: PASS / clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/actions/cogmap.rs \
        crates/temper-cli/src/commands/cogmap.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper cogmap region-metrics + analytics"
```

---

## Final verification (run before opening the PR)

- [ ] `cargo make check` — fmt + clippy (`-D warnings`) + machete, offline sqlx. Expected: clean. (All new queries are runtime `sqlx::query`, so no `.sqlx` regeneration is required; if check complains about an unexpected cache entry, run `cargo sqlx prepare --workspace -- --all-features` and re-check.)
- [ ] `cargo make test` — unit tests (temper-core/client/cli serde + path tests). Expected: green.
- [ ] `cargo make test-db` — api plumbing tests incl. `cogmap_analytics_handler_test`. Expected: green.
- [ ] `cargo make test-artifacts` — substrate `cogmap_analytics_readback`. Expected: green. Run this alone (no concurrent artifact-test run).
- [ ] `cargo make generate-ts-types` — regenerate the `cognitive_maps.ts` exports for the new wire types (so temper-ui can later consume them); commit any regenerated `.ts`.
- [ ] Reinstall the PATH binary if hand-testing the CLI: `cargo install --path crates/temper-cli`.

## Self-Review notes (already reconciled)

- **Spec coverage:** region-metrics (Tasks 2/3/4/5/6) ✓; analytics telos+staleness+regulation (Tasks 2/3/4/5/6) ✓; gate-in-SQL + deny asymmetry (Task 2 SQL, Task 3 handler 404) ✓; no `opposed_labels` on surface ✓; additive migration ✓; wire types in temper-core with full derive stack ✓; substrate-local↔wire mapping ✓; tests at substrate (correctness) + api (plumbing) ✓.
- **Type consistency:** substrate-local `CogmapRegionMetricsRow`/`CogmapAnalyticsRow`/`CogmapStaleness`/`CogmapRegulationRow` (Task 2) map field-for-field to the temper-core wire types (Task 1) in the api wrappers (Task 3); names and field types match across tasks.
- **Out of scope (unchanged):** live temper-ui consumer; `internal_tension` lens-configurability; Backend-trait routing for reads.
