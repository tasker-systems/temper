# Cognitive-map analytics read surface — design

**Date:** 2026-06-28
**Goal:** `substrate-kernel-to-cognitive-map` (WS7 — operational surface)
**Task:** `019ee5a4` — Surface `cogmap_shape` + cognitive-map analytics read side
**Predecessor:** PR #196 (`cogmap_shape` surface-tier read, merged `f75221e`)
**Surfacing spec:** `docs/superpowers/specs/2026-06-19-cognitive-map-substrate-surfacing-design.md`

## Context

PR #196 shipped the `cogmap_shape` **surface tier** read (region_id, lens_id, salience,
content_cohesion, label, member_count) across api + mcp + cli, mirroring the `search` read
path end to end. This design covers the remaining **analytics read side** of the same task:
the deeper per-region metrics and the map-level readouts that the WS7 steward agent and the
(deferred) live UI need.

Two facts found during discovery shape the whole design:

1. **The five region scalar metrics are stored columns, not read-time function calls.**
   `kb_cogmap_regions` carries `centrality`, `content_cohesion`, `internal_tension`,
   `reference_standing`, `telos_alignment` as `DOUBLE PRECISION` columns
   (`migrations/20260624000001_canonical_schema.sql:732-736`). They are computed once at
   materialization time by `populate_readouts` (`crates/temper-substrate/src/write.rs:531-538`),
   which calls the `cogmap_region_*` functions and SETs the columns. The functions are the
   **write/materialize** path; the read is a plain column select. So the analytics read does
   **no** per-region function evaluation and threads **no** `opposed_labels` parameter.

2. **`internal_tension` is the right readout and `'contradicts'` is a live label.** Tension
   *binds* — the contradicting pair co-regions (scenario S6g asserts `internal_tension > 0`
   for the `bluegreen → bigbang` `contradicts` edge in `onboarding-cogmap.yaml`). Formation is
   declared-affinity only; `internal_tension` is a downstream legible readout of a relationship's
   tension, never distance/fracture. `'contradicts'` is the only opposed label in the corpus and
   is hardcoded at materialization time (`write.rs:537`, `ARRAY['contradicts']`). Making it
   lens-configurable is an explicitly-deferred future item — it does not appear in the read surface.

## The analytics SQL functions being surfaced

From `migrations/20260624000002_canonical_functions.sql`:

| Function | Returns | Gate | Grain |
|----------|---------|------|-------|
| `cogmap_telos(p_cogmap)` | `uuid` | none | map (scalar) |
| `cogmap_staleness(p_cogmap)` | `TABLE(materialized_at, latest_touch, is_stale)` | none | map (1 row) |
| `cogmap_regulation(p_cogmap, principal_kind, principal_id)` | `TABLE(resource_id, title, body_text, edge_label)` | `resources_readable_by` | map (N rows) |
| 5 × `cogmap_region_*` | `double precision` each | none | region (materialization-time only; read from stored columns) |

The five region scalars are read from `kb_cogmap_regions` columns, not invoked at read time.

## Design

### 1. One additive migration — two gated canonical SQL functions

Per the "no view from nowhere" principle, the access gate lives **inside** canonical SQL
functions (as `cogmap_shape` does), never inlined in a surface or readback query. New migration
file `migrations/2026062800000X_cogmap_analytics_read_functions.sql`, **additive only**
(`CREATE FUNCTION` statements), honoring the immutable-migrations + additive-on-`main` invariants.

**`cogmap_region_metrics`** — per-region analytics tier, a gated column select mirroring
`cogmap_shape`'s gate and lens filter exactly:

```sql
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
```

**`cogmap_analytics`** — map-level picture in one gated row, composing the existing canonical
functions (no logic duplication). Deny → zero rows:

```sql
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

Both gates apply on the analytics path: the map-level `cogmap_readable_by_profile` wraps the
whole row, and the embedded `cogmap_regulation` additionally filters its rows through
`resources_readable_by`. `regulation` defaults to `[]` (never SQL-null) so the wire type is a
plain `Vec`.

### 2. Wire types (temper-core `crates/temper-core/src/types/cognitive_maps.rs`)

Same derive stack as the shipped `CogmapRegionRow` (`Debug, Clone, PartialEq, Serialize,
Deserialize, FromRow` + feature-gated `ts_rs::TS`, `utoipa::ToSchema`, `schemars::JsonSchema`).

```rust
pub struct CogmapRegionMetricsRow {
    pub region_id: Uuid,
    pub lens_id: Uuid,
    pub centrality: Option<f64>,
    pub content_cohesion: Option<f64>,
    pub internal_tension: Option<f64>,
    pub reference_standing: Option<f64>,
    pub telos_alignment: Option<f64>,
}

pub struct CogmapAnalyticsRow {
    pub telos_resource_id: Uuid,          // kb_cogmaps.telos_resource_id is NOT NULL
    pub staleness: CogmapStaleness,
    pub regulation: Vec<CogmapRegulationRow>,
}

pub struct CogmapStaleness {
    pub materialized_at: Option<DateTime<Utc>>,  // shape_materialized_event_id is nullable
    pub latest_touch: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

pub struct CogmapRegulationRow {
    pub resource_id: Uuid,
    pub title: String,
    pub body_text: Option<String>,
    pub edge_label: String,
}
```

All five metric columns are nullable `DOUBLE PRECISION` → `Option<f64>`. The timestamp type is
`chrono::DateTime<Utc>`, matching the temper-core wire-type precedent (`context.rs`,
`invitation.rs`, etc.).

`CogmapRegionMetricsRow` derives `FromRow` and is read directly. `CogmapAnalyticsRow` is
**nested** (a struct + a `Vec`), so the readback binding maps the flat SQL row →
`CogmapAnalyticsRow`: build `CogmapStaleness` from the three flat columns and deserialize the
`regulation jsonb` column into `Vec<CogmapRegulationRow>` via serde (e.g. an internal flat
`FromRow` row with `sqlx::types::Json<Vec<CogmapRegulationRow>>`, then map to the public type).

### 3. The two vertical stacks (mirror the PR #196 `cogmap_shape` path)

Reads stay **service-direct** (not the `Backend` trait), per the read-path rule. Runtime
`sqlx::query`/`query_as` at the readback layer (as `cogmap_shape` uses — the `jsonb`/`vector`
involvement keeps these off the compile-time macro).

| Layer | File | region-metrics | analytics |
|-------|------|----------------|-----------|
| substrate readback | `crates/temper-substrate/src/readback/mod.rs` | `cogmap_region_metrics(pool, CogmapId, ProfileId, Option<Uuid>) -> Vec<CogmapRegionMetricsRow>` | `cogmap_analytics(pool, CogmapId, ProfileId) -> Option<CogmapAnalyticsRow>` |
| api service | `crates/temper-api/src/backend/substrate_read.rs` | `cogmap_region_metrics_select` | `cogmap_analytics_select` |
| api handler | `crates/temper-api/src/handlers/cognitive_maps.rs` | `region_metrics` — `GET /api/cognitive-maps/{id}/region-metrics?lens=` | `analytics` — `GET /api/cognitive-maps/{id}/analytics` |
| openapi | `crates/temper-api/src/openapi.rs` | register path + schemas | register path + schemas |
| client | `crates/temper-client/src/cognitive_maps.rs` | `CognitiveMapClient::region_metrics(cogmap, lens)` | `::analytics(cogmap)` |
| mcp tool | `crates/temper-mcp/src/tools/cognitive_maps.rs` + `service.rs` | `cogmap_region_metrics` (input `{cogmap, lens}`) | `cogmap_analytics` (input `{cogmap}`) |
| cli | `crates/temper-cli/src/cli.rs`, `commands/cogmap.rs`, `main.rs` | `temper cogmap region-metrics <ref> [--lens <ref>]` | `temper cogmap analytics <ref>` |

MCP input types (`CogmapRegionMetricsInput`, `CogmapAnalyticsInput`) live in temper-core
beside `CogmapShapeInput` and take ref strings, resolved with
`temper_workflow::operations::parse_ref(x)?.0` (the post-crate-split path; CLAUDE.md's
`temper_core::operations` mention is stale). The api handler keeps its `Uuid` HTTP boundary and
converts to the `CogmapId`/`ProfileId` newtypes at the readback call (matching the #194/#195
convention the `cogmap_shape` binding adopted).

### 4. Deny / error semantics

- **region-metrics**: deny → empty `Vec` → `200 []` (the gate is in the SQL; mirrors `cogmap_shape`).
- **analytics**: deny → `None` → **`404 Not Found`** (a single object; map-not-readable is
  indistinguishable from not-found, which is the correct non-leaking posture).

### 5. Testing (mirror PR #196 placement)

- **Substrate artifact tests** (`crates/temper-substrate`, `artifact-tests` feature) — rigorous
  correctness where region seeding is cheap:
  - region-metrics surfaces all five scalars for a materialized region;
    `internal_tension > 0` for the `contradicts` pair (S6g semantics); deny (a profile not
    joined to the map's team) → empty.
  - analytics returns the telos id, the three staleness fields, and the regulation list; deny → `None`.
  - Reuse the `fire(CogmapGenesis)` + materialization path (or the `onboarding-cogmap` fixture)
    so the metrics are populated, not raw-inserted, where the metric values themselves are asserted.
- **temper-api** lighter plumbing test against root-joined L0 (as the `cogmap_shape` api test does):
  the endpoints return `200` with a well-formed body for the root-joined kernel map.
- **Caveat:** never run two `cargo make test-artifacts` concurrently — they exhaust the shared
  Postgres connection pool and produce spurious setup-phase failures.

### 6. sqlx / verification

- New migration changes the schema surface → run migrations locally, then regenerate caches as
  needed: `cargo sqlx prepare --workspace -- --all-features`, plus `cargo make prepare-api` /
  `prepare-e2e` if any **test-target** macro query touches the new functions. Readback runtime
  queries need no `.sqlx` entry (as with `cogmap_shape`).
- Full gate before PR: `cargo make check` + unit + `test-db` + `test-artifacts`.

## Out of scope (deferred — stated to resist creep)

- **Live temper-ui consumer** — replacing the static-SVG `ConceptGraph.svelte` with live
  regions/analytics. The wire types are exported here; the Svelte work is a separate follow-on.
- **`internal_tension` opposed-labels lens-configurability** — a materialization-time concern
  (`write.rs` hardcodes `ARRAY['contradicts']`), separately deferred. Does not touch this read surface.
- **Backend-trait routing for reads** — reads stay service-direct by the read-path rule.

## Acceptance

- `cogmap_region_metrics` and `cogmap_analytics` reachable from cli + mcp + api against the
  single post-flip substrate backend.
- Gate is in the canonical SQL functions only (no inlined gate in any surface/readback query);
  region-metrics deny → `[]`, analytics deny → `404`.
- `internal_tension` surfaced from the stored column; no `opposed_labels` parameter on the read surface.
- Unblocks the WS7 steward agent (telos/regulation/shape/metrics) and the live cognitive-map UI.
