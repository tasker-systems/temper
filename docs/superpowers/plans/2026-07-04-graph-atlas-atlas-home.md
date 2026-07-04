# Graph Atlas — Atlas Home Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow the C2 `@me` home from `you → teams` into the full membership graph `you → teams → cogmaps`, with cogmap doors that open a team-independent cogmap-scoped panorama.

**Architecture:** Two net-new **service-direct reads** (`GET /api/graph/home`, `GET /api/graph/cogmaps/{id}/panorama`) over new `STABLE` SQL functions gated by the existing `cogmap_visible_maps` / `cogmap_readable_by_profile` predicates. The panorama returns the **existing `TerritoryOverview` shape**, so it reuses the shipped `TierPanorama` renderer and R3 region-drill unchanged. Frontend grows `homeLayout` to three columns and adds `?cogmap=` addressing. Two additive C2 fixes ride along (cogmap-name label, door single-click).

**Tech Stack:** Rust (Axum, sqlx macros, ts-rs), PostgreSQL (STABLE SQL fns + `.sqlx` caches), SvelteKit 5 (runes, no `$effect`), d3 (named submodules), vitest (node env).

**Design spec:** `docs/superpowers/specs/2026-07-04-graph-atlas-atlas-home-design.md`

## Global Constraints

- **Reads stay service-direct; writes go through `DbBackend`.** These are reads — service fns in `crates/temper-services/src/services/graph_service.rs`, called from thin handlers in `crates/temper-api/src/handlers/graph.rs`. Never inline `sqlx::query!()` in a handler.
- **Typed structs over inline JSON.** Wire types in `temper-core` with ts-rs derives; never a hand-written zod mirror.
- **New SQL** goes in ONE new timestamped migration file; `LANGUAGE sql STABLE`; additive `CREATE`; no `SET search_path`. **Never edit a shipped migration.** At execution time, renumber the new migration to sort after the latest file already on `main`.
- **sqlx caches:** after any SQL change run `cargo sqlx prepare --workspace -- --all-features`, then per-crate `cargo make prepare-services` and `cargo make prepare-api`, and `cargo make prepare-e2e` for e2e test SQL. `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.
- **Access changes are gated at the e2e tier**, not test-db alone — every new read gets an e2e access test (member sees / non-member 404-as-absence).
- **Wire types regenerate** via `cargo make generate-ts-types`; commit ALL regenerated `*.ts` even if unrelated files move.
- **Frontend gates:** `bun run check` (0 errors) and `bun run test` (vitest) in `packages/temper-ui`. Svelte 5 runes only — **no `$effect`**; `$derived(pureFn(...))` + `$state` for local UI. Generated types are read-only, imported from `$lib/types/generated/*`.
- **Deny-as-absence:** visibility-gate failures return `ApiError::NotFound` (404), never 403.

---

### Task 1: Backend — Atlas Home read (`GET /api/graph/home`)

The full `you → teams → cogmaps` membership payload with count hints, from one read.

**Files:**
- Create: `migrations/<ts>_graph_atlas_home_reads.sql` (SQL fns; shared with Task 2)
- Create: `crates/temper-core/src/types/graph_home.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (re-export)
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `atlas_home`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add `atlas_home` handler)
- Modify: `crates/temper-api/src/routes.rs` (add route)
- Test: `tests/e2e/tests/graph_atlas_home_e2e.rs`

**Interfaces:**
- Produces (SQL): `graph_home_teams(p_profile uuid) RETURNS TABLE(team_id uuid, slug text, name text, resource_count int, cogmap_count int)`; `graph_home_cogmaps(p_profile uuid) RETURNS TABLE(cogmap_id uuid, name text, team_ids uuid[], region_count int, facet_count int)`.
- Produces (wire): `AtlasHome { teams: Vec<HomeTeam>, cogmaps: Vec<HomeCogmap> }`, `HomeTeam { id: Uuid, slug: String, name: String, resource_count: i32, cogmap_count: i32 }`, `HomeCogmap { id: Uuid, name: String, team_ids: Vec<Uuid>, region_count: i32, facet_count: i32 }`.
- Produces (service): `graph_service::atlas_home(pool: &PgPool, profile_id: ProfileId) -> ApiResult<AtlasHome>`.
- Produces (route): `GET /api/graph/home`.

- [ ] **Step 1: Write the SQL functions**

Create `migrations/<ts>_graph_atlas_home_reads.sql` (this file is extended in Task 2; write the header + these two fns now):

```sql
-- ─────────────────────────────────────────────────────────────────────────────
-- Atlas Home reads — the you→teams→cogmaps membership graph + cogmap panorama.
-- (Renumber to sort after the latest main migration at execution time.)
-- ─────────────────────────────────────────────────────────────────────────────

-- Home teams: the profile's member teams, with per-team resource + cogmap counts.
-- Mirrors list_teams' membership set (kb_team_members), adds the two counts.
CREATE FUNCTION graph_home_teams(p_profile uuid)
RETURNS TABLE(team_id uuid, slug text, name text, resource_count int, cogmap_count int)
LANGUAGE sql STABLE AS $$
    SELECT t.id, t.slug, t.name,
           (SELECT count(*) FROM resources_in_team_scope(p_profile, t.id))::int,
           (SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.team_id = t.id)::int
    FROM kb_teams t
    JOIN kb_team_members tm ON tm.team_id = t.id
    WHERE tm.profile_id = p_profile AND t.is_active
    ORDER BY t.name;
$$;

-- Home cogmaps: the profile's visible cogmaps, each with the visible team ids it
-- joins (the bipartite edges) and region/facet counts. Gated by cogmap_visible_maps.
CREATE FUNCTION graph_home_cogmaps(p_profile uuid)
RETURNS TABLE(cogmap_id uuid, name text, team_ids uuid[], region_count int, facet_count int)
LANGUAGE sql STABLE AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id)),
    member_teams AS (
        SELECT tm.team_id FROM kb_team_members tm WHERE tm.profile_id = p_profile
    )
    SELECT c.id, c.name,
           COALESCE(
               array_agg(DISTINCT tc.team_id)
                   FILTER (WHERE tc.team_id IS NOT NULL AND tc.team_id IN (SELECT team_id FROM member_teams)),
               '{}'
           ),
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = c.id)::int
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    GROUP BY c.id, c.name;
$$;
```

Apply it: `sqlx migrate run` (with `DATABASE_URL` set).

- [ ] **Step 2: Write the wire types**

Create `crates/temper-core/src/types/graph_home.rs`:

```rust
//! Wire types for the Atlas Home read (`GET /api/graph/home`) — the
//! you→teams→cogmaps membership graph. See
//! docs/superpowers/specs/2026-07-04-graph-atlas-atlas-home-design.md.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A member team as a home door, with size hints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeTeam {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub resource_count: i32,
    pub cogmap_count: i32,
}

/// A visible cogmap as a home door. `team_ids` are the visible member teams this
/// cogmap joins — i.e. the bipartite team→cogmap edges (a shared cogmap lists >1).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeCogmap {
    pub id: Uuid,
    pub name: String,
    pub team_ids: Vec<Uuid>,
    pub region_count: i32,
    pub facet_count: i32,
}

/// The full membership home: you → member teams → visible cogmaps.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasHome {
    pub teams: Vec<HomeTeam>,
    pub cogmaps: Vec<HomeCogmap>,
}
```

Add to `crates/temper-core/src/types/mod.rs` (match the existing `pub mod` + re-export style used for `graph_scope`):

```rust
pub mod graph_home;
```

- [ ] **Step 3: Write the failing e2e test**

Create `tests/e2e/tests/graph_atlas_home_e2e.rs` (mirror `graph_atlas_slice_e2e.rs`'s helpers — copy `provision_profile`, `create_team`, `add_member`; add cogmap + join helpers):

```rust
//! HTTP e2e for GET /api/graph/home (Atlas Home membership read).
//! Access-tier gate: a member sees their teams+cogmaps with counts; a shared
//! cogmap lists multiple team_ids.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

// Harness pattern verified against graph_atlas_slice_e2e.rs:
//   #[sqlx::test(migrator = "temper_api::MIGRATOR")] fn(pool: sqlx::PgPool)
//   let app = common::setup(pool.clone()).await;  // app.token = a member JWT
//   common::generate_test_jwt(sub, email) for other identities.

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send().await.expect("profile request failed");
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str().unwrap().parse().unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug).fetch_one(pool).await.unwrap()
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')")
        .bind(team).bind(profile).execute(pool).await.unwrap();
}

async fn create_cogmap(pool: &sqlx::PgPool, name: &str) -> Uuid {
    // kb_cogmaps requires a telos_resource_id; create a throwaway resource for it.
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id")
        .bind(format!("{name}-telos")).fetch_one(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id")
        .bind(name).bind(telos).fetch_one(pool).await.unwrap()
}

async fn join_cogmap(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap).bind(team).execute(pool).await.unwrap();
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn home_returns_member_teams_and_shared_cogmap_edges(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let team_a = create_team(&pool, "home-a").await;
    let team_b = create_team(&pool, "home-b").await;
    add_member(&pool, team_a, profile).await;
    add_member(&pool, team_b, profile).await;

    let shared = create_cogmap(&pool, "shared-map").await;
    join_cogmap(&pool, shared, team_a).await;
    join_cogmap(&pool, shared, team_b).await;

    let body: temper_core::types::graph_home::AtlasHome = app.reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send().await.unwrap()
        .json().await.unwrap();

    assert!(body.teams.iter().any(|t| t.slug == "home-a"));
    let sc = body.cogmaps.iter().find(|c| c.name == "shared-map").expect("shared cogmap present");
    assert_eq!(sc.team_ids.len(), 2, "shared cogmap lists both member teams");
}
```

- [ ] **Step 4: Run the e2e test to verify it fails**

Run: `cargo make prepare-e2e && cargo make test-e2e -- graph_atlas_home`
Expected: FAIL — `/api/graph/home` returns 404 (route not registered yet).

- [ ] **Step 5: Write the service function**

In `crates/temper-services/src/services/graph_service.rs`, add (import `AtlasHome, HomeTeam, HomeCogmap` from `temper_core::types::graph_home` at the top with the other type imports):

```rust
/// Atlas Home — the you→teams→cogmaps membership graph with count hints.
/// No entry gate: the read is inherently self-scoped (member teams +
/// cogmap_visible_maps), so it returns exactly what the caller may see.
pub async fn atlas_home(pool: &PgPool, profile_id: ProfileId) -> ApiResult<AtlasHome> {
    let teams: Vec<HomeTeam> = sqlx::query_as::<_, (Uuid, String, String, i32, i32)>(
        "SELECT team_id, slug, name, resource_count, cogmap_count FROM graph_home_teams($1)",
    )
    .bind(profile_id.as_uuid())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, slug, name, resource_count, cogmap_count)| HomeTeam {
        id, slug, name, resource_count, cogmap_count,
    })
    .collect();

    let cogmaps: Vec<HomeCogmap> = sqlx::query_as::<_, (Uuid, String, Vec<Uuid>, i32, i32)>(
        "SELECT cogmap_id, name, team_ids, region_count, facet_count FROM graph_home_cogmaps($1)",
    )
    .bind(profile_id.as_uuid())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, name, team_ids, region_count, facet_count)| HomeCogmap {
        id, name, team_ids, region_count, facet_count,
    })
    .collect();

    Ok(AtlasHome { teams, cogmaps })
}
```

- [ ] **Step 6: Write the handler + register the route**

In `crates/temper-api/src/handlers/graph.rs`, add (import `AtlasHome` from `temper_core::types::graph_home`):

```rust
/// GET /api/graph/home — the you→teams→cogmaps membership home.
#[utoipa::path(
    get,
    path = "/api/graph/home",
    tag = "Graph",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Atlas membership home", body = AtlasHome))
)]
pub async fn atlas_home(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<AtlasHome>> {
    graph_service::atlas_home(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}
```

In `crates/temper-api/src/routes.rs`, next to the other `/api/graph/*` routes:

```rust
        .route("/api/graph/home", get(handlers::graph::atlas_home))
```

- [ ] **Step 7: Regenerate sqlx caches + TS types, run the test**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make generate-ts-types   # produces packages/temper-ui/src/lib/types/generated/graph_home.ts
cargo make test-e2e -- graph_atlas_home
```
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add migrations crates/temper-core crates/temper-services crates/temper-api tests/e2e \
        packages/temper-ui/src/lib/types/generated/graph_home.ts .sqlx crates/*/.sqlx tests/e2e/.sqlx
git commit -m "feat(atlas): GET /api/graph/home — you→teams→cogmaps membership read"
```

---

### Task 2: Backend — cogmap-scoped panorama (`GET /api/graph/cogmaps/{id}/panorama`)

The interior a cogmap door opens onto. Reuses the `TerritoryOverview` shape.

**Files:**
- Modify: `migrations/<ts>_graph_atlas_home_reads.sql` (append two fns — same file as Task 1)
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `cogmap_panorama`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler + `CogmapPanoramaQuery`)
- Modify: `crates/temper-api/src/routes.rs` (add route)
- Test: `tests/e2e/tests/graph_cogmap_panorama_e2e.rs`

**Interfaces:**
- Consumes (wire): existing `TerritoryOverview`, `Territory`, `OrphanNode`, `Bridge`, `TerritoryKind` from `temper_core::types::graph_territory`.
- Produces (SQL): `graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)`; `graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid) RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)`.
- Produces (service): `graph_service::cogmap_panorama(pool, profile_id: ProfileId, cogmap_id: Uuid, lens_id: Option<Uuid>) -> ApiResult<TerritoryOverview>`.
- Produces (route): `GET /api/graph/cogmaps/{id}/panorama[?lens_id=]`.

- [ ] **Step 1: Append the SQL functions**

Append to `migrations/<ts>_graph_atlas_home_reads.sql`:

```sql
-- Cogmap-scoped territories: this cogmap's live regions for the given lens.
-- Keyed on cogmap (not team), gated by cogmap_readable_by_profile.
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

-- Cogmap-scoped orphan facets: this cogmap's homed resources with NO live region,
-- gated per-row by resources_visible_to, ranked by visible edge-degree.
CREATE FUNCTION graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)
LANGUAGE sql STABLE AS $$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    homed AS (
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap
    ),
    region_members AS (
        -- kb_cogmap_region_members is polymorphic: (region_id, member_table, member_id).
        SELECT DISTINCT rm.member_id AS resource_id
        FROM kb_cogmap_region_members rm
        JOIN kb_cogmap_regions reg ON reg.id = rm.region_id
        WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded
          AND rm.member_table = 'kb_resources'
    )
    SELECT r.id, r.title, d.dt AS doc_type, deg.degree, p_cogmap AS anchor_id
    FROM homed
    JOIN kb_resources r ON r.id = homed.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE r.id NOT IN (SELECT resource_id FROM region_members)
    ORDER BY deg.degree DESC;
$$;
```

> `kb_cogmap_region_members` verified: `(region_id, member_table VARCHAR(64) CHECK IN ('kb_resources','kb_cogmaps'), member_id, affinity)`, PK `(region_id, member_table, member_id)` — hence the `member_table = 'kb_resources'` filter above. Apply: `sqlx migrate run` (the file is one not-yet-shipped migration, so editing it during this chunk is allowed; it becomes immutable only once merged).

- [ ] **Step 2: Write the failing e2e test**

Create `tests/e2e/tests/graph_cogmap_panorama_e2e.rs` (reuse the Task-1 helpers; add a read-grant helper for the non-member case):

```rust
//! HTTP e2e for GET /api/graph/cogmaps/{id}/panorama.
//! A reader sees the cogmap interior (TerritoryOverview); a non-reader gets 404.
#![cfg(feature = "test-db")]

mod common;
use reqwest::StatusCode;
use uuid::Uuid;

// ... reuse provision_profile / create_team / add_member / create_cogmap / join_cogmap
//     from graph_atlas_home_e2e.rs (copy or a shared common module) ...

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_denies_non_reader_as_absence(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _profile = provision_profile(&app, &app.token).await;

    // A cogmap joined to NO team the caller belongs to → not readable.
    let orphan_map = create_cogmap(&pool, "unreachable-map").await;

    let status = app.reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{orphan_map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send().await.unwrap()
        .status();
    assert_eq!(status, StatusCode::NOT_FOUND, "deny-as-absence, not 403");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_returns_overview_for_reader(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;
    let team = create_team(&pool, "pano-team").await;
    add_member(&pool, team, profile).await;
    let map = create_cogmap(&pool, "readable-map").await;
    join_cogmap(&pool, map, team).await;

    let resp = app.reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _body: temper_core::types::graph_territory::TerritoryOverview =
        resp.json().await.unwrap(); // shape decodes = renderer-compatible
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo make prepare-e2e && cargo make test-e2e -- graph_cogmap_panorama`
Expected: FAIL — route not registered (404 for both, so `panorama_returns_overview_for_reader` fails on OK assertion).

- [ ] **Step 4: Write the service function**

In `graph_service.rs` (reuses `Territory/OrphanNode/Bridge/TerritoryKind/TerritoryOverview` already imported for R2):

```rust
/// Cogmap-scoped panorama (enter-a-cogmap). Deny-as-absence via
/// cogmap_readable_by_profile. Returns the R2 TerritoryOverview shape so the
/// frontend renders it with the shipped TierPanorama.
pub async fn cogmap_panorama(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: Uuid,
    lens_id: Option<Uuid>,
) -> ApiResult<TerritoryOverview> {
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile_id.as_uuid())
        .bind(cogmap_id)
        .fetch_one(pool)
        .await?;
    if !readable {
        return Err(ApiError::NotFound);
    }

    // Default lens (D2): the lens with the most live regions for THIS cogmap;
    // fall back to the global telos-default if the cogmap has no materialized region.
    let lens: Uuid = match lens_id {
        Some(l) => l,
        None => sqlx::query_scalar(
            "SELECT COALESCE(
                 (SELECT lens_id FROM kb_cogmap_regions
                   WHERE cogmap_id = $1 AND NOT is_folded
                   GROUP BY lens_id ORDER BY count(*) DESC LIMIT 1),
                 (SELECT id FROM kb_cogmap_lenses
                   WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1))",
        )
        .bind(cogmap_id)
        .fetch_one(pool)
        .await?,
    };

    let territories: Vec<Territory> =
        sqlx::query_as::<_, (Uuid, Uuid, Option<String>, i32, f64)>(
            "SELECT region_id, cogmap_id, label, member_count, salience \
             FROM graph_cogmap_territories($1, $2, $3)",
        )
        .bind(profile_id.as_uuid())
        .bind(cogmap_id)
        .bind(lens)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(region_id, cogmap_id, label, member_count, salience)| Territory {
            id: region_id,
            kind: TerritoryKind::Region,
            label,
            member_count,
            salience: Some(salience),
            anchor_id: cogmap_id,
        })
        .collect();

    const ORPHAN_LIMIT: usize = 50;
    let orphan_nodes: Vec<OrphanNode> =
        sqlx::query_as::<_, (Uuid, String, Option<String>, i32, Uuid)>(
            "SELECT id, title, doc_type, degree, anchor_id FROM graph_cogmap_orphan_nodes($1, $2)",
        )
        .bind(profile_id.as_uuid())
        .bind(cogmap_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .take(ORPHAN_LIMIT)
        .map(|(id, title, doc_type, degree, anchor_id)| OrphanNode {
            id, title, doc_type, degree, anchor_id,
            anchor_label: None, // filled by Task 3's name join once merged; None-safe until then
        })
        .collect();

    // A single cogmap panorama has no cross-cogmap bridges.
    Ok(TerritoryOverview { territories, orphan_nodes, bridges: Vec::new() })
}
```

> **Ordering note:** the `anchor_label` field on `OrphanNode` is added in Task 3. If Task 2 lands first, omit that line; if Task 3 lands first, include it. Keep the two tasks' `OrphanNode` construction consistent at merge.

- [ ] **Step 5: Write the handler + route**

In `handlers/graph.rs`:

```rust
/// Query parameters for `GET /api/graph/cogmaps/{id}/panorama`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CogmapPanoramaQuery {
    /// Optional lens override; defaults to the cogmap's primary lens.
    pub lens_id: Option<Uuid>,
}

/// GET /api/graph/cogmaps/{id}/panorama — enter-a-cogmap Tier-0 interior.
#[utoipa::path(
    get,
    path = "/api/graph/cogmaps/{id}/panorama",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Cogmap id"), CogmapPanoramaQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cogmap panorama", body = TerritoryOverview),
        (status = 404, description = "Cogmap not readable by this profile")
    )
)]
pub async fn cogmap_panorama(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<CogmapPanoramaQuery>,
) -> ApiResult<Json<TerritoryOverview>> {
    graph_service::cogmap_panorama(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens_id,
    )
    .await
    .map(Json)
}
```

In `routes.rs`:

```rust
        .route(
            "/api/graph/cogmaps/{id}/panorama",
            get(handlers::graph::cogmap_panorama),
        )
```

- [ ] **Step 6: Regenerate caches, run the test**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make test-e2e -- graph_cogmap_panorama
```
Expected: PASS (both tests).

- [ ] **Step 7: Commit**

```bash
git add migrations crates/temper-services crates/temper-api tests/e2e .sqlx crates/*/.sqlx tests/e2e/.sqlx
git commit -m "feat(atlas): GET /api/graph/cogmaps/{id}/panorama — enter-a-cogmap interior"
```

---

### Task 3: Backend — D3: cogmap name on `OrphanNode` (sparse-territory label fix)

C2's sparse territory shows a generic "cogmap · N facets" because `OrphanNode` carries `anchor_id` but no name. Add `anchor_label`.

**Files:**
- Modify: `migrations/<ts>_graph_atlas_home_reads.sql` (replace R2's `graph_orphan_salient_nodes` via `CREATE OR REPLACE` to add a name column) — OR a small separate additive migration; keep it in the same new file.
- Modify: `crates/temper-core/src/types/graph_territory.rs` (`OrphanNode.anchor_label`)
- Modify: `crates/temper-services/src/services/graph_service.rs` (`territory_overview` + `cogmap_panorama` mapping)
- Test: extend `tests/e2e/tests/graph_territory_overview_e2e.rs` (assert a name is present)

**Interfaces:**
- Modifies (wire): `OrphanNode` gains `pub anchor_label: Option<String>`.
- Modifies (SQL): `graph_orphan_salient_nodes` + `graph_cogmap_orphan_nodes` gain a trailing `anchor_label text` column (the cogmap name).

- [ ] **Step 1: Add the field to the wire type**

In `crates/temper-core/src/types/graph_territory.rs`, in `OrphanNode`, add after `anchor_id`:

```rust
    pub anchor_id: Uuid,
    /// Human name of the home cogmap (`kb_cogmaps.name`), for the sparse territory label.
    pub anchor_label: Option<String>,
```

- [ ] **Step 2: Update the SQL to join the cogmap name**

In the migration, `CREATE OR REPLACE` `graph_orphan_salient_nodes` adding a trailing `anchor_label text` column: `JOIN kb_cogmaps cm ON cm.id = ch.cogmap_id` and `SELECT ..., cm.name AS anchor_label`. Update `graph_cogmap_orphan_nodes` (Task 2) the same way: `SELECT ..., (SELECT name FROM kb_cogmaps WHERE id = p_cogmap) AS anchor_label`.

- [ ] **Step 3: Update both service mappings**

`territory_overview` and `cogmap_panorama` — extend the orphan tuple to include the trailing `Option<String>` and set `anchor_label`:

```rust
    sqlx::query_as::<_, (Uuid, String, Option<String>, i32, Uuid, Option<String>)>(
        "SELECT id, title, doc_type, degree, anchor_id, anchor_label FROM graph_orphan_salient_nodes($1, $2)",
    )
    // ...
    .map(|(id, title, doc_type, degree, anchor_id, anchor_label)| OrphanNode {
        id, title, doc_type, degree, anchor_id, anchor_label,
    })
```

- [ ] **Step 4: Extend the e2e test**

In `graph_territory_overview_e2e.rs`, in the case that produces an orphan cogmap node, assert `orphan.anchor_label` is `Some(<the cogmap name>)`.

- [ ] **Step 5: Regenerate + run**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make generate-ts-types   # updates graph_territory.ts (OrphanNode.anchor_label)
cargo make test-e2e -- graph_territory_overview
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations crates/temper-core crates/temper-services tests/e2e \
        packages/temper-ui/src/lib/types/generated/graph_territory.ts .sqlx crates/*/.sqlx tests/e2e/.sqlx
git commit -m "fix(atlas): D3 — carry cogmap name on OrphanNode for sparse-territory labels"
```

---

### Task 4: Frontend — three-column home + D4 door-click fix

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.test.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/palette.ts` (door tokens)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte`
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (`readAtlasHome`)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` (home branch)

**Interfaces:**
- Consumes: `AtlasHome`, `HomeTeam`, `HomeCogmap` from `$lib/types/generated/graph_home`.
- Produces (layout): `layoutHome(teams: HomeTeam[], cogmaps: HomeCogmap[], size: {width;height}) -> HomeGraph`; `HomeNode.kind: 'you'|'team'|'cogmap'`.
- Produces (reads): `atlasHomePath(): string` = `/api/graph/home`; `readAtlasHome(token): Promise<AtlasHome>`.

- [ ] **Step 1: Write failing layout tests**

Extend `homeLayout.test.ts` (node-env vitest; explicit `import { describe, expect, it } from 'vitest'`):

```ts
import { describe, expect, it } from 'vitest';
import { layoutHome } from './homeLayout';
import type { HomeTeam, HomeCogmap } from '$lib/types/generated/graph_home';

const team = (id: string): HomeTeam =>
  ({ id, slug: id, name: id, resource_count: 0, cogmap_count: 0 });
const cogmap = (id: string, team_ids: string[]): HomeCogmap =>
  ({ id, name: id, team_ids, region_count: 0, facet_count: 0 });

describe('layoutHome — cogmap column', () => {
  it('places cogmaps in a third column right of teams', () => {
    const g = layoutHome([team('t1')], [cogmap('c1', ['t1'])], { width: 1000, height: 600 });
    const t = g.teams.find((n) => n.kind === 'team')!;
    const c = g.cogmaps.find((n) => n.kind === 'cogmap')!;
    expect(c.x).toBeGreaterThan(t.x);
  });

  it('draws one team→cogmap edge per membership (shared cogmap = 2 edges)', () => {
    const g = layoutHome(
      [team('t1'), team('t2')],
      [cogmap('shared', ['t1', 't2'])],
      { width: 1000, height: 600 },
    );
    expect(g.cogmapEdges).toHaveLength(2);
  });

  it('renders a cogmap with no visible team edge (no left edge)', () => {
    const g = layoutHome([team('t1')], [cogmap('lonely', [])], { width: 1000, height: 600 });
    expect(g.cogmaps).toHaveLength(1);
    expect(g.cogmapEdges).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd packages/temper-ui && bun run test homeLayout`
Expected: FAIL — `layoutHome` takes 2 args / `g.cogmaps` and `g.cogmapEdges` undefined.

- [ ] **Step 3: Extend `homeLayout.ts`**

Add `'cogmap'` to `HomeNode.kind`; add `cogmaps: HomeNode[]` and `cogmapEdges: HomeEdge[]` to `HomeGraph`; take `cogmaps` as the 2nd arg. Keep it pure. Columns: `you` `x=0.16·W`, teams `x=0.52·W` (was 0.58), cogmaps `x=0.86·W`. For each `HomeCogmap`, push one `HomeEdge` per `team_id` present in the laid-out team-node set (look up the team node's `x,y`). Distribute cogmap nodes vertically like teams (`top=0.12·H`, `span=0.76·H`). Follow the existing `homeLayout.ts` structure exactly — same `HomeEdge {fromX,fromY,toX,toY}` positional shape.

- [ ] **Step 4: Run layout tests to pass**

Run: `bun run test homeLayout`
Expected: PASS.

- [ ] **Step 5: Add palette door tokens**

In `palette.ts`, add exported tokens (reuse the existing cogmap hue family so door and territory wash agree):

```ts
export const TEAM_DOOR = { fill: 'rgba(58,138,232,0.13)', stroke: '#3a8ae8', ink: '#cfe0f6' };
export const COGMAP_DOOR = { fill: 'rgba(232,148,46,0.13)', stroke: '#e8942e', ink: '#f4d3a6' };
```

- [ ] **Step 6: Extend `TierHome.svelte`**

- Props gain `cogmaps: HomeCogmap[]`.
- `const g = $derived(layoutHome(teams, cogmaps, { width, height }))`.
- Render `g.cogmapEdges` as `<line>`, then the cogmap column as `↵` doors using `COGMAP_DOOR`; team doors switch their hard-coded hexes to `TEAM_DOOR`.
- Add count chips: team door shows `{t.resource_count} res · {t.cogmap_count} maps`; cogmap door shows `{c.region_count} regions · {c.facet_count} facets`.
- Cogmap door enter: `goto(buildCogmapUrl($page.url, c.id), { replaceState: true })` (import `buildCogmapUrl` from Task 5's `nav.ts`; if Task 5 not yet landed, stub the import and wire in Task 5).
- **D4 fix (shared door handler):** the C2 door uses `onclick` yet prod needs a double-click. Root cause: the `<g role="button" tabindex="0">` receives focus on first pointerdown and the click's default is being consumed. Fix: make both team and cogmap doors use one `enter(id)` handler bound to `onclick` **and** ensure the `<g>` is not stealing the first click — set `tabindex="0"` but add `onpointerup={() => enter(id)}` as the activation (pointerup fires on the first interaction, not gated by focus), keeping `onkeydown` Enter for a11y. Verify in Task 6's browser walk that a single click enters.

> If D4's root cause turns out to be non-trivial or unrelated to the door markup (e.g. a SvelteKit `goto` race), STOP and extract it to a separate C2 follow-up branch per the spec's D4 clause — do not expand this task.

- [ ] **Step 7: Add `readAtlasHome` + path test**

In `graph-reads.ts`, add sibling to `teamsListPath`/`listTeams`:

```ts
export const atlasHomePath = () => '/api/graph/home';
export const readAtlasHome = (token: string): Promise<AtlasHome> =>
  apiGet(atlasHomePath(), token);
```

In `graph-reads.paths.test.ts`:

```ts
it('atlasHomePath', () => { expect(atlasHomePath()).toBe('/api/graph/home'); });
```

- [ ] **Step 8: Wire the load fn home branch**

In `+page.server.ts`, in the `!teamId` (home) branch, replace `const teams = await listTeams(token)` with `const home = await readAtlasHome(token)` and return `teams: home.teams, cogmaps: home.cogmaps` (thread `cogmaps` through to `AtlasCanvas` → `TierHome`). Pass `cogmaps` as a new `AtlasCanvas` prop, forwarded to `TierHome`.

- [ ] **Step 9: Run the frontend gate**

Run: `cd packages/temper-ui && bun run check && bun run test`
Expected: 0 check errors; vitest green.

- [ ] **Step 10: Commit**

```bash
git add packages/temper-ui/src
git commit -m "feat(atlas): three-column membership home (you→teams→cogmaps) + D4 single-click door fix"
```

---

### Task 5: Frontend — enter-a-cogmap (`?cogmap=` addressing + panorama wiring)

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts` (create if absent — mirror `graph-reads.paths.test.ts` style)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (`readCogmapPanorama`)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` (cogmap branch)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte` (cogmap arm)

**Interfaces:**
- Consumes: `readCogmapPanorama` → existing `TerritoryOverview`; existing `TierPanorama.svelte`, R3 `readRegionSlice`.
- Produces (nav): `parseCogmap(url: URL): string | null` (`?cogmap=`); `buildCogmapUrl(url: URL, cogmapId: string): string` (set `cogmap`, clear `team`+`focus`); `buildHomeUrl` also clears `cogmap`.
- Produces (reads): `cogmapPanoramaPath(id, lensId?): string`; `readCogmapPanorama(token, id, lensId?): Promise<TerritoryOverview>`.

- [ ] **Step 1: Write failing nav tests**

```ts
import { describe, expect, it } from 'vitest';
import { parseCogmap, buildCogmapUrl, buildHomeUrl } from './nav';

const url = (s: string) => new URL(`https://x.io/graph/@me${s}`);

describe('cogmap addressing', () => {
  it('parses ?cogmap=', () => {
    expect(parseCogmap(url('?cogmap=abc'))).toBe('abc');
    expect(parseCogmap(url(''))).toBeNull();
  });
  it('buildCogmapUrl sets cogmap and clears team+focus', () => {
    const out = buildCogmapUrl(url('?team=t1&focus=node:n1'), 'c9');
    expect(out).toContain('cogmap=c9');
    expect(out).not.toContain('team=');
    expect(out).not.toContain('focus=');
  });
  it('buildHomeUrl clears cogmap too', () => {
    expect(buildHomeUrl(url('?cogmap=c9'))).not.toContain('cogmap=');
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd packages/temper-ui && bun run test nav`
Expected: FAIL — `parseCogmap`/`buildCogmapUrl` not exported.

- [ ] **Step 3: Implement nav additions**

In `nav.ts`: add `parseCogmap` (read `?cogmap`), `buildCogmapUrl` (set `cogmap`, delete `team` + `focus`, return relative `path?query`), and extend `buildHomeUrl` to also `.delete('cogmap')`. Match the existing builders' relative-URL return style.

- [ ] **Step 4: Run nav tests to pass**

Run: `bun run test nav`
Expected: PASS.

- [ ] **Step 5: Add `readCogmapPanorama` + path test**

In `graph-reads.ts`:

```ts
export const cogmapPanoramaPath = (id: string, lensId?: string) =>
  `/api/graph/cogmaps/${id}/panorama${lensId ? `?lens_id=${lensId}` : ''}`;
export const readCogmapPanorama = (token: string, id: string, lensId?: string): Promise<TerritoryOverview> =>
  apiGet(cogmapPanoramaPath(id, lensId), token);
```

In `graph-reads.paths.test.ts`:

```ts
it('cogmapPanoramaPath', () => {
  expect(cogmapPanoramaPath('c1')).toBe('/api/graph/cogmaps/c1/panorama');
  expect(cogmapPanoramaPath('c1', 'l2')).toBe('/api/graph/cogmaps/c1/panorama?lens_id=l2');
});
```

- [ ] **Step 6: Wire the load fn cogmap branch**

In `+page.server.ts`: parse `cogmapId = parseCogmap(url)`. Add a branch (before the `teamId` branch, since cogmap is a distinct scope): when `cogmapId` is set → `const territories = await readCogmapPanorama(token, cogmapId)`; if a `territory` focus is present, also `readRegionSlice` (tier 1). Return `{ tier, focus, territories, slice, cogmapId, teams: null, teamId: null, ... }`.

- [ ] **Step 7: Add the AtlasCanvas cogmap arm**

In `AtlasCanvas.svelte`: add a `cogmapId: string | null` prop. Extend the `{#if}` ladder so a set `cogmapId` with `territories` renders `<TierPanorama {territories} ... />` (tier 0), and a `territory` focus renders `<TierTerritory {slice} />` (tier 1) — reusing the exact same components the team path uses. No new tier renderer.

- [ ] **Step 8: Run the frontend gate**

Run: `cd packages/temper-ui && bun run check && bun run test`
Expected: 0 check errors; vitest green.

- [ ] **Step 9: Commit**

```bash
git add packages/temper-ui/src
git commit -m "feat(atlas): enter-a-cogmap — ?cogmap= addressing + cogmap-scoped panorama"
```

---

### Task 6: Integration — full gate + browser-verify

**Files:** none (verification only).

- [ ] **Step 1: Full backend + type gate**

```bash
cargo make check        # fmt + clippy + machete + sqlx offline
cargo make test-e2e     # access tier
```
Expected: green; the two new e2e files + the extended territory-overview test pass.

- [ ] **Step 2: Frontend gate**

```bash
cd packages/temper-ui && bun run check && bun run test
```
Expected: 0 errors; vitest green.

- [ ] **Step 3: Browser-verify walk (authed env)**

Load `/graph/@me` and confirm:
1. Home shows three columns `you → teams → cogmaps`; a shared cogmap draws two edges.
2. Team cards show resource/cogmap counts; cogmap doors show region/facet counts.
3. **Single click** on a cogmap door enters its panorama (D4 fixed — verify a single click, not double).
4. A region inside the cogmap panorama drills to the region slice (R3).
5. Sparse cogmap territory (from a team panorama) now shows the **real cogmap name** (D3), not "cogmap · N facets".
6. `⌂ Atlas` returns all the way home.

- [ ] **Step 4: Push + PR**

```bash
git push -u origin <branch>
gh pr create --title "Graph Atlas — Atlas Home (you→teams→cogmaps + enter-a-cogmap)" --body "..."
```

---

## Deferred (out of scope — filed, not built)

- **Tier-2 facet-node neighborhood *inside* a cogmap** — R4 is team-parameterized; a cogmap-scoped traversal is its own SQL surface. Enter-a-cogmap ships panorama + region-drill (Tier 0→1) only.
- **Multi-lens switching** inside a cogmap panorama — D2 picks one lens deterministically.

## Self-review notes

- **Spec coverage:** membership home (Task 4) · count hints (Task 1) · enter-a-cogmap panorama (Tasks 2+5) · region drill via R3 (Task 5, reuse) · visibility gating (Tasks 1,2 SQL + e2e) · D3 name (Task 3) · D4 door-click (Task 4) · deferred Tier-2 (documented). All acceptance criteria mapped.
- **Type consistency:** `AtlasHome/HomeTeam/HomeCogmap` defined in Task 1, consumed in Task 4; `TerritoryOverview` reused (Tasks 2,5); `layoutHome(teams, cogmaps, size)` + `cogmapEdges` consistent Tasks 4↔tests; `buildCogmapUrl`/`parseCogmap`/`readCogmapPanorama`/`cogmapPanoramaPath` consistent Tasks 5↔4↔tests.
- **Grounded, no open assumptions:** `kb_cogmap_region_members` polymorphic columns (Task 2) and the `#[sqlx::test]` + `common::setup`/`app.token` harness (Tasks 1–3) both verified against the live repo (`graph_atlas_slice_e2e.rs`). Task 3's extension of `graph_territory_overview_e2e.rs` inherits that file's existing harness — match its in-file helpers, do not re-import.
