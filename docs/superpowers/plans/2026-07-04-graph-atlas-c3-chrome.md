# Graph Atlas C3 — Atlas Chrome Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the C3 chrome layer over the shipped Atlas map — a selected-element detail panel (TrailRail, borrowing the old ResourcePeek design + the R5 event trail), a team-scoped search accelerator, a legend, and ScopeBar filters — plus the one thin backend read search needs and the edge-id prerequisite trails need.

**Architecture:** Additive over the shipped Atlas engine. One new backend read (`atlas_search`) composes the existing scope-agnostic `unified_search` with `resources_in_team_scope`; one additive migration threads `AtlasEdge.id` through the neighborhood projection. The frontend keeps the "URL is the state model" invariant — selection/filters live in the URL via `nav.ts` builders; no Svelte stores. Chrome sits in a left dock (search/filters/legend) with the map to its right and TrailRail as a right panel on selection.

**Tech Stack:** Rust (Axum, sqlx runtime `query_as`, ts-rs), PostgreSQL (SQL functions, pgvector), SvelteKit 5 (runes, `$app/navigation`/`$app/stores`), Vitest, cargo-nextest, cargo-make.

**Spec:** [`docs/superpowers/specs/2026-07-04-graph-atlas-c3-chrome-design.md`](../specs/2026-07-04-graph-atlas-c3-chrome-design.md).
**Branch:** `jct/graph-atlas-c3-chrome` (already created; spec committed on it).

## Global Constraints

- **DATABASE_URL** for local dev/tests: `postgresql://temper:temper@localhost:5437/temper_development` (Docker Postgres on 5437). `cargo make` tasks set it; bare `cargo`/`nextest` need it exported.
- **SQL macros are offline-cached.** After adding/changing any `sqlx::query!`-family call, regenerate: `cargo sqlx prepare --workspace -- --all-features`; for test-target SQL also `cargo make prepare-e2e` / `prepare-api`. This plan uses **runtime `query_as`** (no macro) for the new reads (mirrors existing graph_service reads + `unified_search`), so no `.sqlx` regen is needed for those — but the e2e/test helpers that use `sqlx::query!` may.
- **Shipped migrations are immutable.** Never edit an applied migration file. New behavior = a new additive migration. Next filenames: **`20260704000008_*.sql`** then `20260704000009_*.sql` (current MAX is `20260704000007`, after the sibling remote-source PR landed `…0006`/`…0007`).
- **ts-rs regen:** after adding/changing a `#[cfg_attr(feature="typescript", ...)]` type in temper-core, run `cargo make generate-ts-types` and **commit the regenerated `packages/temper-ui/src/lib/types/generated/*.ts`** (even unrelated regenerated files that change).
- **All public types implement `Debug`.** Lint suppression uses `#[expect(..., reason="...")]`, never `#[allow]`.
- **e2e access-tier gate:** access/visibility-scoped reads MUST have an e2e test (`#![cfg(feature = "test-db")]`, `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`); `test-db`-only predicate tests are a false signal for access changes.
- **Frontend:** never emit raw ANSI/color hex in components — source all Atlas color from `palette.ts`. Never hand-model a Rust wire type in TS; import the generated type.
- **Subagent gate (per task):** run `cargo fmt`, then the relevant `cargo make check` slice / `bun run check` + tests, before handing back. Controller runs DB/e2e tests + commits (implementer subagents stall on backgrounded cargo).
- **After a temper-cli/bin change** (none expected here) rebuild the bin; not applicable to this plan.

---

## Task 1: Backend — `atlas_search` team-scoped search read

**Files:**
- Create: `migrations/20260704000008_graph_atlas_search.sql`
- Create/Modify: `crates/temper-core/src/types/graph_atlas.rs` (add `AtlasSearchHit`)
- Modify: `crates/temper-core/src/types/mod.rs:71` (re-export)
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `atlas_search` fn)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler + query struct)
- Modify: `crates/temper-api/src/routes.rs` (register route)
- Test: `tests/e2e/tests/graph_atlas_search_e2e.rs` (new)

**Interfaces:**
- Produces (Rust): `temper_core::types::graph_atlas::AtlasSearchHit`; `graph_service::atlas_search(pool, profile_id, team_id, query, limit) -> ApiResult<Vec<AtlasSearchHit>>`.
- Produces (HTTP): `GET /api/teams/{id}/graph/search?q=<str>&limit=<n>` → `Json<Vec<AtlasSearchHit>>`.
- Produces (TS): `AtlasSearchHit` in `packages/temper-ui/src/lib/types/generated/graph_atlas.ts`.
- Consumes: existing SQL `unified_search`, `resources_in_team_scope`, `team_viewable_by`, `kb_resource_homes`, `kb_properties`, `kb_cogmap_region_members`, `resources_visible_to`.

- [ ] **Step 1: Write the SQL migration**

Create `migrations/20260704000008_graph_atlas_search.sql`:

```sql
-- C3 SearchAccelerator: team-scoped name-locate over the Atlas graph.
-- Reuses the scope-agnostic unified_search blend (weights/visibility unchanged),
-- bounding it to resources_in_team_scope(profile, team) and projecting each hit
-- to Atlas display attrs (doc_type, home) + an optional best-affinity region.
-- v1: NULL embedding (FTS + graph-off name-locate); graph_expand = false.
CREATE FUNCTION atlas_search(
    p_profile uuid,
    p_team    uuid,
    p_query   text,
    p_limit   int
) RETURNS TABLE(
    node_id uuid,
    title text,
    doc_type text,
    home text,
    region_id uuid,
    combined_score real,
    fts_score real,
    vector_score real,
    graph_score real
)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT array_agg(resource_id) AS ids
        FROM resources_in_team_scope(p_profile, p_team)
    ),
    hits AS (
        SELECT u.resource_id, u.combined_score, u.fts_score, u.vector_score, u.graph_score
        FROM unified_search(
            p_profile,               -- $1 principal
            p_query,                 -- $2 query text
            NULL::vector,            -- $3 embedding (NULL → vector term zeroed)
            ARRAY[]::uuid[],         -- $4 seed_ids
            0,                       -- $5 depth
            ARRAY[]::text[],         -- $6 edge_types
            NULL,                    -- $7 context_id
            NULL,                    -- $8 doc_type
            false,                   -- $9 graph_expand
            p_limit,                 -- $10 limit
            0,                       -- $11 offset
            (SELECT ids FROM scope)  -- $12 scope_ids
        ) u
    ),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT
        r.id AS node_id,
        r.title,
        d.dt AS doc_type,
        h.home,
        reg.region_id,
        hits.combined_score::real,
        hits.fts_score::real,
        hits.vector_score::real,
        hits.graph_score::real
    FROM hits
    JOIN kb_resources r ON r.id = hits.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT m.region_id
        FROM kb_cogmap_region_members m
        WHERE m.member_table = 'kb_resources' AND m.member_id = r.id
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) reg ON true
    ORDER BY hits.combined_score DESC, r.id;
$$;
```

- [ ] **Step 2: Apply the migration and verify it loads**

Run:
```bash
cd /Users/petetaylor/projects/tasker-systems/temper
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run
```
Expected: applies `20260704000008_graph_atlas_search` with no error. If Docker Postgres isn't up: `cargo make docker-up` first.

- [ ] **Step 3: Add the `AtlasSearchHit` wire type**

In `crates/temper-core/src/types/graph_atlas.rs`, after `AtlasNode`, add (mirror the exact derive/ts-rs stack):

```rust
/// A team-scoped search hit on the Atlas canvas. `node_id` is `kb_resources.id`
/// (identical to `AtlasNode.id`, so the UI can drill straight to it). Scores are
/// the `unified_search` blend, inherited verbatim. `region_id` is a best-affinity
/// territory hint (may be `None`); the camera jump uses `node_id` alone.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasSearchHit {
    pub node_id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub home: NodeHome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_id: Option<Uuid>,
    pub combined_score: f32,
    pub fts_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
}
```

In `crates/temper-core/src/types/mod.rs:71`, add `AtlasSearchHit` to the `graph_atlas::{...}` re-export:
```rust
pub use graph_atlas::{AtlasEdge, AtlasNode, AtlasSearchHit, AtlasSubgraph, NodeHome, SliceRequest};
```

- [ ] **Step 4: Write the failing service test (unit, in graph_service or a service test)**

Add the service function stub returning `todo!()` first so the e2e compiles later, but the primary test is the e2e (Step 8). For a fast inner-loop unit, add to `crates/temper-services` an integration test only if the crate has the harness; otherwise rely on the e2e. **Skip a service-level unit test** — the read is pure SQL composition; the e2e access-tier test (Step 8) is the real gate. Proceed to implement.

- [ ] **Step 5: Implement the `atlas_search` service read**

In `crates/temper-services/src/services/graph_service.rs`, add (mirror `neighborhood_slice`'s gate + runtime `query_as`):

```rust
/// C3 — team-scoped Atlas search. Service-direct read. Deny-as-absence (404)
/// when the profile cannot view the team. Ranking + visibility inherited from
/// `unified_search`; hits bounded to `resources_in_team_scope`.
pub async fn atlas_search(
    pool: &PgPool,
    profile_id: ProfileId,
    team_id: Uuid,
    query: &str,
    limit: i64,
) -> ApiResult<Vec<AtlasSearchHit>> {
    let viewable: bool = sqlx::query_scalar("SELECT team_viewable_by($1, $2)")
        .bind(profile_id.as_uuid())
        .bind(team_id)
        .fetch_one(pool)
        .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    let rows = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<Uuid>, f32, f32, f32, f32)>(
        "SELECT node_id, title, doc_type, home, region_id, combined_score, fts_score, vector_score, graph_score \
         FROM atlas_search($1, $2, $3, $4)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(query)
    .bind(limit.min(50) as i32)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(node_id, title, doc_type, home, region_id, combined_score, fts_score, vector_score, graph_score)| {
            AtlasSearchHit {
                node_id,
                title,
                doc_type,
                home: if home.as_deref() == Some("cogmap") { NodeHome::Cogmap } else { NodeHome::Context },
                region_id,
                combined_score,
                fts_score,
                vector_score,
                graph_score,
            }
        })
        .collect())
}
```

Add imports at the top of `graph_service.rs` if missing: `use temper_core::types::graph_atlas::{AtlasSearchHit, NodeHome};` (extend the existing graph_atlas import line).

- [ ] **Step 6: Add the API handler + query struct**

In `crates/temper-api/src/handlers/graph.rs`, add:

```rust
/// Query parameters for `GET /api/teams/{id}/graph/search`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct AtlasSearchQuery {
    /// The search term (name-locate over the team-scoped visible graph).
    pub q: String,
    /// Max hits (default 15, capped 50 server-side).
    pub limit: Option<i64>,
}

/// GET /api/teams/{id}/graph/search — C3 SearchAccelerator.
#[utoipa::path(
    get,
    path = "/api/teams/{id}/graph/search",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Team id"), AtlasSearchQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Search hits", body = [AtlasSearchHit]),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn atlas_search(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Query(q): Query<AtlasSearchQuery>,
) -> ApiResult<Json<Vec<AtlasSearchHit>>> {
    graph_service::atlas_search(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        &q.q,
        q.limit.unwrap_or(15),
    )
    .await
    .map(Json)
}
```

Add `AtlasSearchHit` to the `temper_core::types::graph_atlas::{...}` import at the top of the handler file.

- [ ] **Step 7: Register the route**

In `crates/temper-api/src/routes.rs`, alongside the other graph routes (~line 92-108), add:
```rust
        .route(
            "/api/teams/{id}/graph/search",
            get(handlers::graph::atlas_search),
        )
```

- [ ] **Step 8: Write the access-tier e2e test**

Create `tests/e2e/tests/graph_atlas_search_e2e.rs` (mirror `graph_territory_overview_e2e.rs` + `team_graph_scope_e2e.rs` harness helpers). It must assert: (a) a member finds an in-scope resource by title; (b) a resource the profile can't read does NOT appear; (c) an outsider gets 404.

```rust
//! HTTP e2e for GET /api/teams/{id}/graph/search (C3) — access-tier gate.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

// (copy provision_profile, create_team, add_member, create_resource,
//  create_team_context, home_resource from graph_territory_overview_e2e.rs;
//  copy the kb_access_grants insert helper from team_graph_scope_e2e.rs)

async fn search(app: &common::E2eTestApp, token: &str, team: Uuid, q: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/teams/{team}/graph/search?q={q}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn atlas_search_scopes_to_team_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token = common::generate_test_jwt("as-e2e-outsider", "as-e2e-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let team = create_team(&pool, "as-e2e-team").await;
    add_member(&pool, team, member).await;
    let ctx = create_team_context(&pool, team, "as-e2e-ctx").await;

    let in_scope = create_resource(&pool, "Findable Widget", "temper://as-e2e/findable").await;
    home_resource(&pool, in_scope, "kb_contexts", ctx, member).await;

    // A resource NOT homed in the team scope — must not surface.
    let _out_of_scope = create_resource(&pool, "Findable Widget Hidden", "temper://as-e2e/hidden").await;

    // (a) member finds the in-scope resource by title token
    let (status, body) = search(&app, &app.token, team, "Findable").await;
    assert_eq!(status, StatusCode::OK, "member gets 200: {body:?}");
    let hits = body.as_array().expect("array of hits");
    let ids: Vec<&str> = hits.iter().filter_map(|h| h["node_id"].as_str()).collect();
    assert!(ids.contains(&in_scope.to_string().as_str()), "in-scope resource surfaces: {body:?}");

    // (b) out-of-scope resource does not appear
    assert!(!ids.contains(&_out_of_scope.to_string().as_str()), "out-of-scope resource is not returned: {body:?}");

    // (c) outsider → 404 deny-as-absence
    let (status, _) = search(&app, &outsider_token, team, "Findable").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "non-member denied as absence");
}
```

- [ ] **Step 9: Run the e2e test (controller runs DB tests)**

Run:
```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo build -p temper-cli --bin temper >/dev/null 2>&1 || true
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo test -p temper-e2e --features test-db --test graph_atlas_search_e2e 2>&1 | tail -30
```
Expected: `atlas_search_scopes_to_team_and_denies_outsiders ... ok`. (Use plain `cargo test --test <name>` — a fresh e2e binary hangs at nextest `--list` on macOS.)

- [ ] **Step 10: Regenerate TS types + gate + commit**

Run:
```bash
cargo make generate-ts-types 2>&1 | tail -5
cargo fmt
cargo make check 2>&1 | tail -20
```
Expected: `graph_atlas.ts` now contains `AtlasSearchHit`; `cargo make check` green. Then commit:
```bash
git add migrations/20260704000008_graph_atlas_search.sql \
  crates/temper-core/src/types/graph_atlas.rs crates/temper-core/src/types/mod.rs \
  crates/temper-services/src/services/graph_service.rs \
  crates/temper-api/src/handlers/graph.rs crates/temper-api/src/routes.rs \
  tests/e2e/tests/graph_atlas_search_e2e.rs \
  packages/temper-ui/src/lib/types/generated/graph_atlas.ts
git commit -m "feat(atlas): C3 atlas_search — team-scoped search read (reuses unified_search)"
```

---

## Task 2: Backend — thread `AtlasEdge.id` through the neighborhood projection

Edge trails (Task 5) need each rendered edge to carry its `kb_edges.id` so it can be addressed for `readTrail('edge', id)`. Today `AtlasEdge` and `graph_traverse_scoped` omit it.

**Files:**
- Create: `migrations/20260704000009_graph_traverse_edge_id.sql`
- Modify: `crates/temper-core/src/types/graph_atlas.rs` (`AtlasEdge` — add `id`)
- Modify: `crates/temper-services/src/services/graph_service.rs` (`neighborhood_slice` mapping)
- Test: extend `tests/e2e/tests/*neighborhood*` (or the slice e2e) to assert edges carry a non-null `id`.

**Interfaces:**
- Produces: `AtlasEdge.id: Uuid` (wire + TS); `graph_traverse_scoped` returns `id uuid` as its first column.

- [ ] **Step 1: Find the shipped `graph_traverse_scoped` definition**

Run:
```bash
grep -rn "CREATE FUNCTION graph_traverse_scoped" /Users/petetaylor/projects/tasker-systems/temper/migrations/
```
Read the function body it points to (in `20260703130000_graph_atlas_chunk_b_reads.sql`). Note its exact param list and `RETURNS TABLE(...)` columns — the new migration reproduces the body verbatim with an added `id` column.

- [ ] **Step 2: Write the DROP/CREATE migration**

Create `migrations/20260704000009_graph_traverse_edge_id.sql`. Changing a `RETURNS TABLE` shape requires DROP then CREATE (CREATE OR REPLACE cannot alter the return type):

```sql
-- C3: expose kb_edges.id from graph_traverse_scoped so rendered AtlasEdges can be
-- addressed for R5 edge trails. Additive column; body otherwise unchanged from
-- 20260703130000 (shipped migration stays immutable).
DROP FUNCTION IF EXISTS graph_traverse_scoped(uuid, uuid, uuid[], int, text[]);

CREATE FUNCTION graph_traverse_scoped(
    -- <copy the EXACT param list from the shipped definition>
) RETURNS TABLE(
    id uuid,            -- NEW: the kb_edges.id of the induced edge
    source_id uuid,
    target_id uuid,
    edge_kind edge_kind,
    polarity polarity,
    label text,
    weight double precision
)
LANGUAGE sql STABLE AS $$
    -- <copy the shipped body, adding e.id AS id to the final SELECT list;
    --  the recursive CTE already walks kb_edges e, so e.id is in scope>
$$;
```

**Note to implementer:** you MUST paste the shipped function's real param list and body here (read it in Step 1) — do not invent it. The only change is adding `id uuid` as the first RETURNS column and `e.id` (the edge row's id) as the first SELECT column. If the shipped body aggregates/UNIONs edges such that a single `e.id` isn't directly available, select the underlying `kb_edges.id` at the point each edge enters the result set.

- [ ] **Step 3: Apply + verify**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run
```
Expected: applies cleanly.

- [ ] **Step 4: Add `id` to `AtlasEdge`**

In `crates/temper-core/src/types/graph_atlas.rs`, add `pub id: Uuid,` as the first field of `AtlasEdge`:
```rust
pub struct AtlasEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: f64,
}
```
(Keep the existing derives/attrs unchanged; only add the field. Verify the existing `label` optionality matches the current definition before editing.)

- [ ] **Step 5: Thread `id` through `neighborhood_slice`**

In `graph_service.rs::neighborhood_slice`, update the `graph_traverse_scoped` tuple type + mapping to include the new leading `id` column:
```rust
    let walked = sqlx::query_as::<_, (Uuid, Uuid, Uuid, EdgeKind, Polarity, Option<String>, f64)>(
        "SELECT id, source_id, target_id, edge_kind, polarity, label, weight \
         FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    // ... binds unchanged ...
    .fetch_all(pool)
    .await?;

    let edges: Vec<AtlasEdge> = walked
        .iter()
        .map(|(id, source, target, edge_kind, polarity, label, weight)| AtlasEdge {
            id: *id,
            source: *source,
            target: *target,
            edge_kind: *edge_kind,
            polarity: *polarity,
            label: label.clone(),
            weight: *weight,
        })
        .collect();

    // node_ids collection uses walked tuple — update destructuring:
    let mut node_ids: Vec<Uuid> = req.seeds.clone();
    for (_, s, t, ..) in &walked {
        node_ids.push(*s);
        node_ids.push(*t);
    }
```

- [ ] **Step 6: Regenerate TS + assert edge id in e2e**

Run `cargo make generate-ts-types` (updates `graph_atlas.ts` `AtlasEdge`). Extend the existing neighborhood-slice e2e (find it: `grep -rln "graph/slice\|neighborhood" tests/e2e/tests/`) with an assertion that returned edges carry a non-null `id`. If no neighborhood e2e exists, add a minimal one mirroring Task 1's harness that seeds two homed resources + one edge between them and asserts `body["edges"][0]["id"]` is a UUID string.

- [ ] **Step 7: Gate + commit**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo test -p temper-e2e --features test-db --test <neighborhood_test_name> 2>&1 | tail -20
cargo fmt && cargo make check 2>&1 | tail -20
git add migrations/20260704000009_graph_traverse_edge_id.sql \
  crates/temper-core/src/types/graph_atlas.rs \
  crates/temper-services/src/services/graph_service.rs \
  packages/temper-ui/src/lib/types/generated/graph_atlas.ts tests/e2e/tests/
git commit -m "feat(atlas): thread AtlasEdge.id through graph_traverse_scoped for edge trails"
```

---

## Task 3: Frontend foundation — `nav.ts` selection + filter params

Shared by TrailRail (edge selection) and ScopeBar (filters). Pure functions, unit-tested — the established pattern.

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts` (extend)

**Interfaces:**
- Produces: `Selection` type; `parseSelection(url)`, `buildEdgeSelectUrl(base, edgeId)`, `clearSelectionUrl(base)`, `selectedElement(focus, url)`; extended `GraphFilters` with `edgeKinds: string[]`, `docTypes: string[]`; `buildFiltersUrl(base, partial)`.

- [ ] **Step 1: Write failing tests for selection + filters**

Append to `nav.test.ts`:
```ts
import {
	buildEdgeSelectUrl, clearSelectionUrl, parseSelection, selectedElement,
	parseFilters, buildFiltersUrl
} from './nav';

describe('edge selection (?sel)', () => {
	it('parses ?sel=edge:e1', () => {
		expect(parseSelection(url('?sel=edge:e1'))).toEqual({ kind: 'edge', id: 'e1' });
	});
	it('none when absent/malformed', () => {
		expect(parseSelection(url(''))).toEqual({ kind: 'none' });
		expect(parseSelection(url('?sel=node:n1'))).toEqual({ kind: 'none' }); // only edges use ?sel
	});
	it('buildEdgeSelectUrl sets ?sel, leaves ?focus/?team intact', () => {
		expect(buildEdgeSelectUrl(url('?team=t1&focus=node:n1'), 'e9'))
			.toBe('/graph/@me?team=t1&focus=node%3An1&sel=edge%3Ae9');
	});
	it('clearSelectionUrl drops ?sel', () => {
		expect(clearSelectionUrl(url('?team=t1&sel=edge:e9'))).toBe('/graph/@me?team=t1');
	});
	it('selectedElement prefers edge sel, else focus node', () => {
		expect(selectedElement({ kind: 'node', id: 'n1' }, url('?sel=edge:e9'))).toEqual({ kind: 'edge', id: 'e9' });
		expect(selectedElement({ kind: 'node', id: 'n1' }, url(''))).toEqual({ kind: 'node', id: 'n1' });
		expect(selectedElement({ kind: 'none' }, url(''))).toEqual({ kind: 'none' });
	});
});

describe('filters', () => {
	it('parses edge_kinds + doc_types CSV', () => {
		expect(parseFilters(url('?edge_kinds=derived,contains&doc_types=task,goal').searchParams))
			.toEqual({ lensId: null, edgeKinds: ['derived', 'contains'], docTypes: ['task', 'goal'] });
	});
	it('buildFiltersUrl sets/clears CSV params', () => {
		expect(buildFiltersUrl(url('?team=t1'), { edgeKinds: ['derived'] }))
			.toBe('/graph/@me?team=t1&edge_kinds=derived');
		expect(buildFiltersUrl(url('?team=t1&edge_kinds=derived'), { edgeKinds: [] }))
			.toBe('/graph/@me?team=t1');
	});
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd packages/temper-ui && bun run test -- nav.test.ts 2>&1 | tail -20`
Expected: FAIL — `parseSelection`/`buildEdgeSelectUrl`/etc not exported.

- [ ] **Step 3: Implement in `nav.ts`**

Add to `nav.ts`:
```ts
export type Selection =
	| { kind: 'none' }
	| { kind: 'edge'; id: string };

/** Orthogonal panel selection for edges. `?focus` still owns scope/camera/seed;
 *  `?sel=edge:<id>` selects an edge for the TrailRail panel without re-seeding. */
export function parseSelection(url: URL): Selection {
	const raw = url.searchParams.get('sel');
	if (!raw) return { kind: 'none' };
	const [kind, id] = raw.split(':', 2);
	if (id && kind === 'edge') return { kind: 'edge', id };
	return { kind: 'none' };
}

export function buildEdgeSelectUrl(base: URL, edgeId: string): string {
	return withParams(base, (p) => p.set('sel', `edge:${edgeId}`));
}

export function clearSelectionUrl(base: URL): string {
	return withParams(base, (p) => p.delete('sel'));
}

/** The element whose detail panel is shown: an explicitly-selected edge wins,
 *  else the focused node, else nothing. */
export type SelectedElement =
	| { kind: 'none' }
	| { kind: 'node'; id: string }
	| { kind: 'edge'; id: string };

export function selectedElement(focus: Focus, url: URL): SelectedElement {
	const sel = parseSelection(url);
	if (sel.kind === 'edge') return sel;
	if (focus.kind === 'node') return { kind: 'node', id: focus.id };
	return { kind: 'none' };
}
```

Widen `GraphFilters` + `parseFilters`, and add `buildFiltersUrl`:
```ts
export interface GraphFilters {
	lensId: string | null;
	edgeKinds: string[];
	docTypes: string[];
}

export function parseFilters(params: URLSearchParams): GraphFilters {
	const csv = (k: string) => {
		const v = params.get(k);
		return v ? v.split(',').filter(Boolean) : [];
	};
	return { lensId: params.get('lens_id'), edgeKinds: csv('edge_kinds'), docTypes: csv('doc_types') };
}

export function buildFiltersUrl(
	base: URL,
	patch: Partial<{ lensId: string | null; edgeKinds: string[]; docTypes: string[] }>
): string {
	return withParams(base, (p) => {
		if ('lensId' in patch) {
			if (patch.lensId) p.set('lens_id', patch.lensId);
			else p.delete('lens_id');
		}
		const setCsv = (k: string, v?: string[]) => {
			if (!v) return;
			if (v.length) p.set(k, v.join(','));
			else p.delete(k);
		};
		setCsv('edge_kinds', patch.edgeKinds);
		setCsv('doc_types', patch.docTypes);
	});
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-ui && bun run test -- nav.test.ts 2>&1 | tail -20`
Expected: PASS. Then `bun run check 2>&1 | tail` — no TS/biome errors (note `parseFilters` callers in `+page.server.ts` still compile: `filters.lensId` unchanged; new fields are additive).

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): nav.ts ?sel edge selection + edge_kinds/doc_types filter params"
```

---

## Task 4: Frontend — TrailRail pure modules (`atlasNeighbors` + `trailModel`)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/neighbors.ts` + `neighbors.test.ts`
- Create: `packages/temper-ui/src/lib/graph/atlas/trail.ts` + `trail.test.ts` (**note:** there is an OLD `src/lib/graph/trail.ts` for breadcrumbs — this new one is under `atlas/`, distinct path)

**Interfaces:**
- Produces: `atlasNeighbors(focusId, nodes: AtlasNode[], edges: AtlasEdge[]): AtlasNeighbor[]` where `AtlasNeighbor = { dir: '→' | '←'; label: string; other: AtlasNode }`; `trailModel(trail: EventTrail): TrailRow[]` where `TrailRow = { kind: string; actor: string; occurredAt: string; confidence: string | null }`.

- [ ] **Step 1: Write failing `neighbors` test**

Create `neighbors.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { atlasNeighbors } from './neighbors';
import type { AtlasNode, AtlasEdge } from '$lib/types/generated/graph_atlas';

const node = (o: Partial<AtlasNode>): AtlasNode => ({ id: 'x', title: 'X', doc_type: null, home: 'context', degree: 0, ...o });
const edge = (o: Partial<AtlasEdge>): AtlasEdge => ({ id: 'e', source: 's', target: 't', edge_kind: 'contains', polarity: 'forward', label: null, weight: 1, ...o });

describe('atlasNeighbors', () => {
	it('yields out/in neighbors, coalescing label ?? edge_kind', () => {
		const nodes = [node({ id: 'a', title: 'A' }), node({ id: 'b', title: 'B' })];
		const edges = [edge({ id: 'e1', source: 'a', target: 'b', label: null, edge_kind: 'contains' })];
		const r = atlasNeighbors('a', nodes, edges);
		expect(r).toEqual([{ dir: '→', label: 'contains', other: nodes[1] }]);
	});
	it('drops edges whose other end is absent', () => {
		expect(atlasNeighbors('a', [node({ id: 'a' })], [edge({ source: 'a', target: 'ghost' })])).toEqual([]);
	});
	it('sorts by label then title deterministically', () => {
		const nodes = [node({ id: 'a' }), node({ id: 'b', title: 'Beta' }), node({ id: 'c', title: 'Alpha' })];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: 'rel' }),
			edge({ id: 'e2', source: 'a', target: 'c', label: 'rel' })
		];
		expect(atlasNeighbors('a', nodes, edges).map((n) => n.other.title)).toEqual(['Alpha', 'Beta']);
	});
});
```

- [ ] **Step 2: Run — verify fail.** `cd packages/temper-ui && bun run test -- neighbors.test.ts` → FAIL (module missing).

- [ ] **Step 3: Implement `neighbors.ts`**

```ts
import type { AtlasNode, AtlasEdge } from '$lib/types/generated/graph_atlas';

export interface AtlasNeighbor {
	dir: '→' | '←';
	label: string;
	other: AtlasNode;
}

/** Atlas-native neighbors of `focusId` from a loaded slice. Unlike the old
 *  peek.ts builder, this is typed on AtlasNode/AtlasEdge, coalesces the nullable
 *  edge label to its edge_kind, and sorts by (label, title) — no aggregator sort. */
export function atlasNeighbors(focusId: string, nodes: AtlasNode[], edges: AtlasEdge[]): AtlasNeighbor[] {
	const byId = new Map(nodes.map((n) => [n.id, n] as const));
	const out: AtlasNeighbor[] = [];
	for (const e of edges) {
		const label = e.label ?? e.edge_kind;
		if (e.source === focusId) {
			const other = byId.get(e.target);
			if (other) out.push({ dir: '→', label, other });
		} else if (e.target === focusId) {
			const other = byId.get(e.source);
			if (other) out.push({ dir: '←', label, other });
		}
	}
	out.sort((a, b) => a.label.localeCompare(b.label) || a.other.title.localeCompare(b.other.title));
	return out;
}
```

- [ ] **Step 4: Run — verify pass.** Same command → PASS.

- [ ] **Step 5: Write failing `trail` test**

Create `atlas/trail.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { trailModel } from './trail';
import type { EventTrail } from '$lib/types/generated/element_trail';

const trail = (events: EventTrail['events']): EventTrail => ({ element_kind: 'node', element_id: 'n1', events });

describe('trailModel', () => {
	it('maps events newest-first and humanizes kind', () => {
		const t = trail([
			{ event_id: 'a', kind: 'relationship.asserted', actor_entity_id: 'u1', occurred_at: '2026-01-01T00:00:00Z', confidence: null },
			{ event_id: 'b', kind: 'relationship.reweighted', actor_entity_id: 'u1', occurred_at: '2026-02-01T00:00:00Z', confidence: 'high' }
		]);
		const rows = trailModel(t);
		expect(rows[0]).toMatchObject({ kind: 'Reweighted', occurredAt: '2026-02-01T00:00:00Z', confidence: 'high' });
		expect(rows[1]).toMatchObject({ kind: 'Asserted', confidence: null });
	});
	it('normalizes missing confidence to null', () => {
		const t = trail([{ event_id: 'a', kind: 'block.created', actor_entity_id: 'u', occurred_at: '2026-01-01T00:00:00Z', confidence: undefined as unknown as null }]);
		expect(trailModel(t)[0].confidence).toBeNull();
	});
});
```

- [ ] **Step 6: Run — verify fail.**

- [ ] **Step 7: Implement `atlas/trail.ts`**

```ts
import type { EventTrail } from '$lib/types/generated/element_trail';

export interface TrailRow {
	kind: string;
	actor: string;
	occurredAt: string;
	confidence: string | null;
}

/** Humanize an R5 EventTrail into display rows, newest-first. `kind` is the
 *  canonical dotted event type (e.g. "relationship.reweighted"); we show the
 *  trailing segment title-cased. Confidence is normalized (absent → null). */
export function trailModel(trail: EventTrail): TrailRow[] {
	return [...trail.events]
		.reverse()
		.map((e) => ({
			kind: humanizeKind(e.kind),
			actor: e.actor_entity_id,
			occurredAt: e.occurred_at,
			confidence: e.confidence ?? null
		}));
}

function humanizeKind(kind: string): string {
	const tail = kind.split('.').pop() ?? kind;
	return tail.charAt(0).toUpperCase() + tail.slice(1).replace(/_/g, ' ');
}
```
(Events arrive ordered by event id ascending = oldest-first; `.reverse()` yields newest-first.)

- [ ] **Step 8: Run — verify pass. Then `bun run check`. Commit:**
```bash
git add packages/temper-ui/src/lib/graph/atlas/neighbors.ts packages/temper-ui/src/lib/graph/atlas/neighbors.test.ts \
  packages/temper-ui/src/lib/graph/atlas/trail.ts packages/temper-ui/src/lib/graph/atlas/trail.test.ts
git commit -m "feat(atlas): TrailRail pure modules — atlasNeighbors + trailModel"
```

---

## Task 5: Frontend — Atlas left-dock shell + TrailRail panel + clickable edges

Establishes the C3 layout (dock | canvas | panel) and lands the first occupant: the TrailRail selected-element panel. Wires edge clicks → `?sel`, node/edge selection → the panel, and the load-side reads (trail + resource metadata).

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte` (the panel)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/Edge.svelte` (add `onSelect?`)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte` (wire edge click)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (add `readResourceRow`, reuse `readTrail`)
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` (load trail + resource row on selection)
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.svelte` (render `AtlasShell`)

**Interfaces:**
- Consumes: `selectedElement`, `buildEdgeSelectUrl`, `clearSelectionUrl` (Task 3); `atlasNeighbors`, `trailModel` (Task 4); `AtlasEdge.id` (Task 2); `readTrail` (existing).
- Produces: `data.selection` (SelectedElement), `data.trail` (EventTrail | null), `data.resourceRow` (ResourceRow | null) on the page bag.

- [ ] **Step 1: Make `Edge` clickable**

In `marks/Edge.svelte`, add an `onSelect?: () => void` prop and wire it (mirror NodeChip's role/tabindex/keydown):
```svelte
interface Props { x1: number; y1: number; x2: number; y2: number; edge: AtlasEdge; label?: boolean; onSelect?: () => void; }
let { x1, y1, x2, y2, edge, label = false, onSelect }: Props = $props();
```
Wrap the `<g class="edge">` with the interaction attrs when `onSelect` is set:
```svelte
<g class="edge"
	role={onSelect ? 'button' : undefined}
	tabindex={onSelect ? 0 : undefined}
	aria-label={edge.label ?? edge.edge_kind}
	onclick={onSelect}
	onkeydown={(e) => e.key === 'Enter' && onSelect?.()}
	style={onSelect ? 'cursor:pointer' : undefined}>
	<!-- existing <line> + label -->
</g>
```
Add a widened invisible hit-line under the visible line for easier clicking:
```svelte
<line {x1} {y1} {x2} {y2} stroke="transparent" stroke-width="10" />
```

- [ ] **Step 2: Wire the edge click in `TierNeighborhood`**

In `TierNeighborhood.svelte`, import `buildEdgeSelectUrl` and pass `onSelect` to each `<Edge>`:
```svelte
import { buildDrillNodeUrl, buildEdgeSelectUrl } from '$lib/graph/atlas/nav';
// ...
function selectEdge(edgeId: string) {
	goto(buildEdgeSelectUrl($page.url, edgeId), { replaceState: true });
}
```
```svelte
{#each graph.edges as e, i (i)}
	<g role="presentation" onmouseenter={() => (hoveredEdge = i)} onmouseleave={() => (hoveredEdge = null)}>
		<Edge x1={e.source.x} y1={e.source.y} x2={e.target.x} y2={e.target.y}
			edge={e.edge} label={hoveredEdge === i} onSelect={() => selectEdge(e.edge.id)} />
	</g>
{/each}
```
(`e.edge.id` now exists from Task 2.)

- [ ] **Step 3: Add server reads**

In `graph-reads.ts`, add a resource-row read (reuse the existing `/api/resources/{id}` endpoint) alongside `readTrail`:
```ts
import type { ResourceRow } from '$lib/types/generated/resource';
export const resourceRowPath = (id: string): string => `/api/resources/${id}`;
export const readResourceRow = (token: string, id: string): Promise<ResourceRow> =>
	apiGet<ResourceRow>(resourceRowPath(id), token);
```

- [ ] **Step 4: Load trail + resource row on selection**

In `+page.server.ts`, after computing `focus`/`tier`, derive `selection` and load its detail. Add to the imports: `import { selectedElement } from '$lib/graph/atlas/nav'; import { readResourceRow, readTrail } from '$lib/server/graph-reads';`. In the **team** branch (and cogmap branch if edges render there), after the existing reads:
```ts
	const selection = selectedElement(focus, url);
	const trail =
		selection.kind === 'edge' ? await readTrail(token, 'edge', selection.id)
		: selection.kind === 'node' ? await readTrail(token, 'node', selection.id)
		: null;
	const resourceRow = selection.kind === 'node' ? await readResourceRow(token, selection.id) : null;
```
Add `selection, trail, resourceRow` to **all three** return objects (as `{ kind: 'none' }` / `null` in the branches that don't compute them — keep `PageData` uniform).

- [ ] **Step 5: Build `TrailRail.svelte`** (new Atlas-native panel; borrow ResourcePeek's markup/structure, source color from `palette.ts`)

Create `TrailRail.svelte`. Props: the selection, the loaded slice (for identity + neighbors), the trail, the resource row. Render node vs edge variants + the History section:
```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { SelectedElement } from '$lib/graph/atlas/nav';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { EventTrail } from '$lib/types/generated/element_trail';
	import type { ResourceRow } from '$lib/types/generated/resource';
	import { atlasNeighbors } from '$lib/graph/atlas/neighbors';
	import { trailModel } from '$lib/graph/atlas/trail';
	import { docTypeHue } from '$lib/graph/atlas/palette';
	import { buildDrillNodeUrl, clearSelectionUrl } from '$lib/graph/atlas/nav';

	interface Props {
		selection: SelectedElement;
		subgraph: AtlasSubgraph | null;
		trail: EventTrail | null;
		resourceRow: ResourceRow | null;
	}
	let { selection, subgraph, trail, resourceRow }: Props = $props();

	const node = $derived(
		selection.kind === 'node' && subgraph ? subgraph.nodes.find((n) => n.id === selection.id) ?? null : null
	);
	const edge = $derived(
		selection.kind === 'edge' && subgraph ? subgraph.edges.find((e) => e.id === selection.id) ?? null : null
	);
	const hue = $derived(node ? docTypeHue(node.doc_type ?? null) : edge ? '#c9b183' : '#8a929e');
	const neighbors = $derived(node && subgraph ? atlasNeighbors(node.id, subgraph.nodes, subgraph.edges) : []);
	const rows = $derived(trail ? trailModel(trail) : []);

	function close() { goto(clearSelectionUrl($page.url), { replaceState: true }); }
	function refocus(id: string) { goto(buildDrillNodeUrl($page.url, id), { replaceState: true }); }
</script>

{#if selection.kind !== 'none'}
	<aside class="trail-rail" style="--hue: {hue};" data-testid="trail-rail">
		<header>
			<span class="marker">{edge ? 'EDGE' : 'NODE'} · {node?.doc_type ?? edge?.edge_kind ?? ''}</span>
			<button class="close" onclick={close}>CLOSE ✕</button>
		</header>
		<h2 class="title">{node?.title ?? (edge ? `${edge.edge_kind}` : '')}</h2>

		{#if node && neighbors.length}
			<section class="neighbors">
				<div class="label">NEIGHBORS · {neighbors.length}</div>
				{#each neighbors as n (n.other.id + n.label + n.dir)}
					<button class="nb" onclick={() => refocus(n.other.id)}>
						<span class="dir">{n.dir}</span>
						<span class="rel">{n.label}</span>
						<span class="name" style="color: {docTypeHue(n.other.doc_type ?? null)}">{n.other.title}</span>
					</button>
				{/each}
			</section>
		{/if}

		{#if node && resourceRow}
			<section class="meta">
				<div><span class="k">CONTEXT</span><span>{resourceRow.context_slug ?? '—'}</span></div>
				{#if resourceRow.cogmap_name}<div><span class="k">COGMAP</span><span>{resourceRow.cogmap_name}</span></div>{/if}
				{#if resourceRow.stage}<div><span class="k">STAGE</span><span>{resourceRow.stage}</span></div>{/if}
			</section>
		{/if}

		{#if edge}
			<section class="meta">
				<div><span class="k">POLARITY</span><span>{edge.polarity}</span></div>
				<div><span class="k">WEIGHT</span><span>{edge.weight}</span></div>
			</section>
		{/if}

		<section class="history">
			<div class="label">HISTORY · {rows.length}</div>
			{#if rows.length === 0}
				<p class="empty">No recorded history.</p>
			{:else}
				{#each rows.slice(0, 50) as row (row.actor + row.occurredAt + row.kind)}
					<div class="event">
						<span class="ekind">{row.kind}</span>
						<time datetime={row.occurredAt}>{row.occurredAt}</time>
						{#if row.confidence}<span class="conf">{row.confidence}</span>{/if}
					</div>
				{/each}
			{/if}
		</section>
	</aside>
{/if}

<style>
	.trail-rail { width: 340px; height: 100%; overflow-y: auto; background: rgba(20,23,29,0.96);
		border-left: 1px solid color-mix(in srgb, var(--hue) 33%, transparent); backdrop-filter: blur(8px); color: #c9d1d9; }
	header { display: flex; justify-content: space-between; align-items: center; padding: 14px 18px 8px; }
	.marker { font-family: monospace; font-size: 9px; letter-spacing: 0.2em; color: color-mix(in srgb, var(--hue) 80%, white); }
	.close { background: none; border: 0; color: #6a727e; font: 9px monospace; letter-spacing: 0.2em; cursor: pointer; }
	.title { margin: 0; padding: 0 18px 10px; font-family: Georgia, serif; font-size: 22px; color: var(--hue); }
	section { padding: 8px 18px; border-top: 1px solid rgba(255,255,255,0.06); }
	.label { font-family: monospace; font-size: 8.5px; letter-spacing: 0.2em; color: #6a727e; margin-bottom: 6px; }
	.nb { display: grid; grid-template-columns: 14px 70px 1fr; gap: 8px; width: 100%; text-align: left;
		background: none; border: 0; padding: 5px 0; cursor: pointer; font-size: 13px; }
	.nb .dir { color: #4a5261; } .nb .rel { font: 8px monospace; letter-spacing: 0.15em; color: #6a727e; }
	.meta > div { display: grid; grid-template-columns: 80px 1fr; gap: 10px; font: 10px monospace; padding: 3px 0; }
	.meta .k { color: #6a727e; letter-spacing: 0.15em; }
	.event { display: flex; gap: 8px; align-items: baseline; padding: 4px 0; font-size: 12px; }
	.ekind { color: var(--hue); } time { color: #6a727e; font-size: 10px; } .conf { font-size: 9px; color: #8fd8a8; }
	.empty { color: #6a727e; font-size: 12px; }
</style>
```

- [ ] **Step 6: Build the dock | canvas | panel layout directly in `+page.svelte`**

Keep the layout in the page (which already has `PageData`) — no separate shell component, no cross-dir type import. The dock holds `ScopeBar` for now (Tasks 6–8 add search/legend/filters into the same dock). `TrailRail` sits to the right when an element is selected. **`viewKey` must NOT include `selection`** — selecting an edge should not remount `AtlasCanvas` (that resets the camera); TrailRail reads selection reactively.

```svelte
<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';
	import TrailRail from '$lib/components/graph/atlas/TrailRail.svelte';
	import { selectedElement } from '$lib/graph/atlas/nav';
	import { page } from '$app/stores';

	let { data }: { data: PageData } = $props();

	const viewKey = $derived(
		`${data.teamId ?? data.cogmapId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
	const selection = $derived(selectedElement(data.focus, $page.url));
	const subgraph = $derived(data.neighborhood ?? null);
</script>

<div class="atlas-page">
	<aside class="dock">
		{#if data.scope}<ScopeBar scope={data.scope} />{:else}<nav class="scope-bar home">Atlas · your teams</nav>{/if}
	</aside>
	<div class="canvas-wrap">
		{#key viewKey}
			<AtlasCanvas
				teamId={data.teamId} cogmapId={data.cogmapId} tier={data.tier} focus={data.focus}
				territories={data.territories} slice={data.slice} neighborhood={data.neighborhood}
				teams={data.teams} cogmaps={data.cogmaps} zones={data.scope?.zones ?? []} />
		{/key}
	</div>
	{#if selection.kind !== 'none'}
		<TrailRail {selection} {subgraph} trail={data.trail} resourceRow={data.resourceRow} />
	{/if}
</div>

<style>
	.atlas-page { display: grid; grid-template-columns: 232px 1fr auto; height: 100%; min-height: 0; }
	.dock { border-right: 1px solid rgba(255, 255, 255, 0.06); overflow-y: auto; }
	.canvas-wrap { position: relative; min-width: 0; }
</style>
```
(The existing `+page.svelte` `.atlas-page` was `flex-column`; this replaces it with the 3-column grid. Preserve any existing global layout assumptions — check `.atlas-page` isn't styled elsewhere.)

- [ ] **Step 8: Verify in the running app** (controller drives this)

Run `cd packages/temper-ui && bun run dev`, open the graph route, drill to a Tier-2 neighborhood, click a node → node panel with neighbors + metadata + history; click an edge → edge panel with polarity/weight + edge history; CLOSE clears `?sel`. Confirm no console errors and the camera does NOT reset on edge select.

- [ ] **Step 9: `bun run check` + commit**
```bash
cd packages/temper-ui && bun run check 2>&1 | tail
git add packages/temper-ui/src/lib/components/graph/atlas/AtlasShell.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/marks/Edge.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte \
  packages/temper-ui/src/lib/server/graph-reads.ts \
  packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.server.ts \
  packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.svelte
git commit -m "feat(atlas): C3 left-dock shell + TrailRail selected-element panel (node+edge)"
```

---

## Task 6: Frontend — SearchAccelerator

Search-to-locate in the dock. Interactive (type → results), so it fetches an internal endpoint rather than reloading the page per keystroke — mirroring the existing `_internal/search/+server.ts` pattern.

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/graph/_search/+server.ts` (proxy to `atlas_search`)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (`atlasSearchPath` + `readAtlasSearch`)
- Test: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts` (add `atlasSearchPath` case)
- Create: `packages/temper-ui/src/lib/components/graph/atlas/SearchAccelerator.svelte`
- Modify: `+page.svelte` (mount in the dock snippet, team scope only)

**Interfaces:**
- Consumes: `AtlasSearchHit` (Task 1), `buildDrillNodeUrl`.
- Produces: `GET /graph/_search?team=<id>&q=<str>` → `AtlasSearchHit[]` (internal); `SearchAccelerator` component.

- [ ] **Step 1: Add path builder + read + failing path test**

In `graph-reads.ts`:
```ts
import type { AtlasSearchHit } from '$lib/types/generated/graph_atlas';
export const atlasSearchPath = (teamId: string, q: string, limit = 15): string =>
	`/api/teams/${teamId}/graph/search?q=${encodeURIComponent(q)}&limit=${limit}`;
export const readAtlasSearch = (token: string, teamId: string, q: string): Promise<AtlasSearchHit[]> =>
	apiGet<AtlasSearchHit[]>(atlasSearchPath(teamId, q), token);
```
In `graph-reads.paths.test.ts` add:
```ts
it('atlasSearchPath encodes q', () => {
	expect(atlasSearchPath('t1', 'a b')).toBe('/api/teams/t1/graph/search?q=a%20b&limit=15');
});
```
Run the paths test → PASS after adding the import.

- [ ] **Step 2: Internal search endpoint**

Create `routes/(app)/graph/_search/+server.ts` (mirror `_internal/search/+server.ts`):
```ts
import { json, type RequestHandler } from '@sveltejs/kit';
import { readAtlasSearch } from '$lib/server/graph-reads';

export const GET: RequestHandler = async ({ url, locals }) => {
	const team = url.searchParams.get('team');
	const q = url.searchParams.get('q')?.trim() ?? '';
	if (!team || q.length === 0) return json([]);
	const hits = await readAtlasSearch(locals.accessToken!, team, q);
	return json(hits);
};
```

- [ ] **Step 3: `SearchAccelerator.svelte`**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AtlasSearchHit } from '$lib/types/generated/graph_atlas';
	import { docTypeHue } from '$lib/graph/atlas/palette';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';

	interface Props { teamId: string; }
	let { teamId }: Props = $props();

	let q = $state('');
	let hits = $state<AtlasSearchHit[]>([]);
	let timer: ReturnType<typeof setTimeout> | null = null;

	function onInput() {
		if (timer) clearTimeout(timer);
		const term = q.trim();
		if (term.length === 0) { hits = []; return; }
		timer = setTimeout(async () => {
			const res = await fetch(`/graph/_search?team=${teamId}&q=${encodeURIComponent(term)}`);
			hits = res.ok ? await res.json() : [];
		}, 180);
	}
	function jump(hit: AtlasSearchHit) {
		goto(buildDrillNodeUrl($page.url, hit.node_id), { replaceState: true });
		q = ''; hits = [];
	}
</script>

<div class="search" data-testid="atlas-search">
	<input placeholder="Find a node…" bind:value={q} oninput={onInput} />
	{#if hits.length}
		<ul>
			{#each hits as h (h.node_id)}
				<li><button onclick={() => jump(h)}>
					<span class="dot" style="background: {h.home === 'cogmap' ? docTypeHue(h.doc_type ?? null) : 'transparent'}; border-color: {docTypeHue(h.doc_type ?? null)}"></span>
					<span class="t">{h.title}</span>
				</button></li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.search { padding: 10px 12px; }
	input { width: 100%; box-sizing: border-box; background: #14171d; border: 1px solid #2a2f38; color: #c9d1d9; border-radius: 6px; padding: 6px 9px; font-size: 13px; }
	ul { list-style: none; margin: 6px 0 0; padding: 0; max-height: 220px; overflow-y: auto; }
	li button { display: flex; gap: 8px; align-items: center; width: 100%; text-align: left; background: none; border: 0; padding: 5px 4px; cursor: pointer; color: #c9d1d9; font-size: 12px; }
	li button:hover { background: rgba(255,255,255,0.03); }
	.dot { width: 9px; height: 9px; border-radius: 50%; border: 2px solid; flex: 0 0 auto; }
</style>
```

- [ ] **Step 4: Mount in the dock** (team scope only) — inside the `.dock` `<aside>` in `+page.svelte`, above `ScopeBar`:
```svelte
<aside class="dock">
	{#if data.teamId}<SearchAccelerator teamId={data.teamId} />{/if}
	{#if data.scope}<ScopeBar scope={data.scope} filters={data.filters} />{:else}<nav class="scope-bar home">Atlas · your teams</nav>{/if}
</aside>
```
(import `SearchAccelerator`. `ScopeBar`'s `filters` prop lands in Task 8; add it now or when Task 8 runs.)

- [ ] **Step 5: Verify + commit** — `bun run dev`: type a known title in a team scope → hits list → click → camera jumps to the node's neighborhood. `bun run check`. Commit:
```bash
git add packages/temper-ui/src/routes/\(app\)/graph/_search/+server.ts \
  packages/temper-ui/src/lib/server/graph-reads.ts packages/temper-ui/src/lib/server/graph-reads.paths.test.ts \
  packages/temper-ui/src/lib/components/graph/atlas/SearchAccelerator.svelte \
  packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.svelte
git commit -m "feat(atlas): C3 SearchAccelerator — team-scoped locate + camera-jump"
```

---

## Task 7: Frontend — AtlasLegend

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/legend.ts` + `legend.test.ts` (pure model)
- Create: `packages/temper-ui/src/lib/components/graph/atlas/AtlasLegend.svelte`
- Modify: `+page.svelte` (mount in dock)

**Interfaces:**
- Produces: `legendModel(): LegendModel` deriving sections from `palette.ts` exports; `AtlasLegend` (collapsible).

- [ ] **Step 1: Failing `legend.test.ts`**
```ts
import { describe, expect, it } from 'vitest';
import { legendModel } from './legend';
import { DOC_TYPE_HUES } from './palette';

describe('legendModel', () => {
	it('lists every doc-type hue from the palette (no drift)', () => {
		const m = legendModel();
		const swatchTypes = m.docTypes.map((d) => d.docType).sort();
		expect(swatchTypes).toEqual(Object.keys(DOC_TYPE_HUES).sort());
	});
	it('groups authored vs workflow', () => {
		const m = legendModel();
		expect(m.docTypes.some((d) => d.authored)).toBe(true);
		expect(m.docTypes.some((d) => !d.authored)).toBe(true);
	});
	it('describes home + edge encodings', () => {
		const m = legendModel();
		expect(m.home).toHaveLength(2); // fill=cogmap, outline=context
		expect(m.edges.length).toBeGreaterThan(0);
	});
});
```

- [ ] **Step 2: Run — fail. Step 3: Implement `legend.ts`**
```ts
import { DOC_TYPE_HUES, docTypeHue, isAuthored, EDGE_COLORS, type AtlasDocType } from './palette';

export interface LegendSwatch { docType: string; hue: string; authored: boolean; }
export interface LegendModel {
	docTypes: LegendSwatch[];
	home: { label: string; filled: boolean }[];
	edges: { label: string; color: string }[];
}

/** Derive the legend entirely from palette.ts — the single source of truth.
 *  A colocated test asserts docType coverage stays in sync (guards drift). */
export function legendModel(): LegendModel {
	const docTypes = (Object.keys(DOC_TYPE_HUES) as AtlasDocType[]).map((dt) => ({
		docType: dt,
		hue: docTypeHue(dt),
		authored: isAuthored(dt)
	}));
	return {
		docTypes,
		home: [
			{ label: 'cogmap-homed', filled: true },
			{ label: 'context-homed', filled: false }
		],
		edges: Object.entries(EDGE_COLORS).map(([label, color]) => ({ label, color: color as string }))
	};
}
```
(If `EDGE_COLORS` keys/shape differ, adjust to its actual `as const` structure — verified in palette.ts.)

- [ ] **Step 4: Run — pass. Step 5: `AtlasLegend.svelte`** (collapsible, reads the model)
```svelte
<script lang="ts">
	import { legendModel } from '$lib/graph/atlas/legend';
	const m = legendModel();
	let open = $state(true);
</script>

<div class="legend" data-testid="atlas-legend">
	<button class="head" onclick={() => (open = !open)}>▦ Legend {open ? '▾' : '▸'}</button>
	{#if open}
		<div class="sec"><div class="lbl">DOC TYPE</div>
			{#each m.docTypes as d (d.docType)}
				<div class="row"><span class="sw" style="background:{d.authored ? d.hue : 'transparent'}; border-color:{d.hue}"></span>{d.docType}</div>
			{/each}
		</div>
		<div class="sec"><div class="lbl">HOME</div>
			{#each m.home as h (h.label)}
				<div class="row"><span class="sw" style="background:{h.filled ? '#c9d1d9' : 'transparent'}; border-color:#c9d1d9"></span>{h.label}</div>
			{/each}
		</div>
		<div class="sec"><div class="lbl">EDGES</div>
			{#each m.edges as e (e.label)}
				<div class="row"><span class="line" style="background:{e.color}"></span>{e.label}</div>
			{/each}
		</div>
	{/if}
</div>

<style>
	.legend { padding: 8px 12px; font-size: 12px; color: #c9d1d9; }
	.head { background: none; border: 0; color: #c9d1d9; cursor: pointer; font-size: 12px; padding: 4px 0; }
	.sec { padding: 6px 0; } .lbl { font: 8.5px monospace; letter-spacing: 0.2em; color: #6a727e; margin-bottom: 4px; }
	.row { display: flex; align-items: center; gap: 8px; padding: 2px 0; }
	.sw { width: 10px; height: 10px; border-radius: 50%; border: 2px solid; flex: 0 0 auto; }
	.line { width: 16px; height: 2px; flex: 0 0 auto; }
</style>
```

- [ ] **Step 6: Mount in dock** (inside the `.dock` `<aside>` in `+page.svelte`, below `ScopeBar`, always): `<AtlasLegend />`. **Step 7:** `bun run test -- legend.test.ts` + `bun run check` + verify in dev. Commit:
```bash
git add packages/temper-ui/src/lib/graph/atlas/legend.ts packages/temper-ui/src/lib/graph/atlas/legend.test.ts \
  packages/temper-ui/src/lib/components/graph/atlas/AtlasLegend.svelte \
  packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.svelte
git commit -m "feat(atlas): C3 AtlasLegend — palette-sourced, collapsible"
```

---

## Task 8: Frontend — ScopeBar filters (lens · edge-kind · doc-type)

Extend the breadcrumb-only `ScopeBar` with filter controls. Filter state lives in the URL (Task 3 helpers). Lens → `readTerritories`; edge-kind → `readNeighborhood`; doc-type → client-side dimming.

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte` (add filter controls)
- Modify: `+page.server.ts` (thread `edge_kinds` into `readNeighborhood`; expose `filters` on the bag)
- Modify: `AtlasCanvas.svelte` / `TierNeighborhood.svelte` (apply doc-type dimming)
- (Optional) enumerate lenses — reuse an existing lens list read if present; else a small addition.

**Interfaces:**
- Consumes: `parseFilters`, `buildFiltersUrl` (Task 3); `SliceRequest.edge_kinds` (existing).

- [ ] **Step 1: Thread edge-kind filter into the neighborhood read.** In `+page.server.ts`, the neighborhood read currently passes `edge_kinds: []`. Change to `filters.edgeKinds`:
```ts
	const neighborhood =
		tier === 2 && focus.kind === 'node'
			? await readNeighborhood(token, teamId, { seeds: [focus.id], depth: NEIGHBORHOOD_DEPTH, edge_kinds: filters.edgeKinds })
			: null;
```
Add `filters` to the team-branch return bag (and `filters: { lensId: null, edgeKinds: [], docTypes: [] }` to the other two branches for uniformity).

- [ ] **Step 2: Lens enumeration.** Grep for an existing lens-list endpoint/read:
```bash
grep -rn "lens" packages/temper-ui/src/lib/server crates/temper-api/src/routes.rs
```
If a lens list read exists, reuse it in the load and pass options to ScopeBar. If not, the lens picker ships as a **free-text/known-id input** for v1 (still sets `?lens_id`), and a proper enumeration read is deferred (note it in the PR). Do **not** block C3 on a new lens-list endpoint.

- [ ] **Step 3: Add filter controls to `ScopeBar.svelte`.** Extend props to accept `filters` + available `edgeKinds`/`docTypes` (derive edge-kind options from `EDGE_COLORS` keys, doc-type options from the loaded slice or `DOC_TYPE_HUES`). Render toggle chips that call `goto(buildFiltersUrl($page.url, patch), { replaceState: true })`:
```svelte
import { buildFiltersUrl } from '$lib/graph/atlas/nav';
import { EDGE_COLORS } from '$lib/graph/atlas/palette';
// props: scope, filters
function toggleEdgeKind(k: string) {
	const next = filters.edgeKinds.includes(k) ? filters.edgeKinds.filter((x) => x !== k) : [...filters.edgeKinds, k];
	goto(buildFiltersUrl($page.url, { edgeKinds: next }), { replaceState: true });
}
```
Add a small `<div class="filters">` under the breadcrumb `<nav>` with edge-kind chips + doc-type chips + a lens `<input>`.

- [ ] **Step 4: Doc-type dimming.** Pass `filters.docTypes` down to `TierNeighborhood`/`TierPanorama`; when non-empty, render chips whose `docType` is NOT in the set at reduced opacity (e.g. `opacity: 0.15`). Implement as a derived `dimmed(docType)` boolean passed to `NodeChip` (add an optional `dim?: boolean` prop that lowers opacity). Keep it purely visual (no read change).

- [ ] **Step 5: Verify + commit.** `bun run dev`: toggle an edge-kind → neighborhood re-fetches with fewer edge kinds; toggle a doc-type → non-matching chips dim; set a lens id → territories resize. `bun run check` + `bun run test`. Commit:
```bash
git add packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/NodeChip.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte \
  packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte \
  packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.server.ts
git commit -m "feat(atlas): C3 ScopeBar filters — lens + edge-kind + doc-type"
```

---

## Final gate (before PR)

- [ ] Full backend gate: `cargo fmt && cargo make check` — green.
- [ ] All graph e2e targets: `DATABASE_URL=... cargo test -p temper-e2e --features test-db --test graph_atlas_search_e2e --test <neighborhood_test>` — green (and run the embed variant if any touched read is embed-gated: `cargo make test-e2e-embed`).
- [ ] Frontend: `cd packages/temper-ui && bun run check && bun run test` — green (vitest count ≥ prior 192 + new tests).
- [ ] `cargo make generate-ts-types` produced no uncommitted diff (types are committed).
- [ ] Manual: node panel (neighbors + metadata + history), edge panel (polarity/weight + history), search jump, legend, all three filters — all work in `bun run dev`, both do-not-regress the shipped canvas.
- [ ] Merge `origin/main` locally, push branch, open PR. One consolidated end-of-plan opus review (per `feedback_subagent_review_cadence`).

## Self-review notes (coverage against spec)

- TrailRail (node+edge, ResourcePeek lineage, R5 History) → Tasks 4+5. ✅
- SearchAccelerator (reuse `unified_search`, team-scope, drill-jump) → Tasks 1+6. ✅
- AtlasLegend (palette-sourced, collapsible) → Task 7. ✅
- ScopeBar filters (lens+edge-kind+doc-type; context deferred to Chunk D) → Task 8. ✅
- Edge-id prerequisite → Task 2. ✅
- Left-dock layout → Task 5 (AtlasShell). ✅
- Access-tier e2e for the new read → Task 1 Step 8. ✅
- Theme dark-only / palette as-is → honored (no light path; all color from `palette.ts`). ✅
