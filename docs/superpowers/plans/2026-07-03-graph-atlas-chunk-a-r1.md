# Graph Atlas — Chunk A / R1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver R1 of the Graph Atlas read model — the access-navigation foundation: descendant-zone enumeration (which child teams a profile may enter) + a team-scope resource filter, surfaced as one read endpoint `GET /api/teams/{id}/graph-scope`.

**Architecture:** Additive over the existing (already DAG-transitive) access substrate — three new `LANGUAGE sql STABLE` functions in a new migration, a service-direct read in `temper-services`, a thin Axum handler + route in `temper-api`, and ts-rs wire types in `temper-core`. No new foundational data modeling; the existing `team_ancestors` / `resources_visible_to` gates are reused, and the new functions compose them (guidepost §1: new SQL read functions are welcome; only the data model is frozen).

**Tech Stack:** Rust (Axum, sqlx runtime queries, ts-rs), PostgreSQL (sqlx migrations), e2e tests via `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` + real HTTP.

**Spec:** `docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md` (read model R1; guideposts §1).
**Task:** `019f28a1-9519-7992-96d9-98f446256649` (Chunk A / R1, build/medium). **Goal:** `graph-atlas-visualization`.

## Global Constraints

- **Access-semantics change → the e2e tier is mandatory.** `cargo make test-db` green is a *false signal* for access changes; the acceptance gate is `cargo make test-e2e`, which exercises deny-code + auth + handler together. (CLAUDE.md; `feedback_access_semantics_changes_need_e2e_tier`.)
- **Migrations are additive & immutable once shipped.** New file `migrations/20260703000001_team_graph_scope_reads.sql`; never edit an applied migration. New functions use bare `CREATE FUNCTION`; `LANGUAGE sql STABLE`; **namespace-free** (no `SET search_path`; unqualified names resolve against `public`).
- **These reads use runtime `sqlx::query`/`query_scalar`, NOT the `query!` macro** — the visibility helper functions are unqualified and the sqlx describe step can't resolve them (see `crates/temper-services/src/services/edge_service.rs:24-28`). Therefore **no `.sqlx` cache regeneration is required** for this chunk (no `query!`/`query_as!` macro is added).
- **Reads are service-direct.** Never inline SQL in a handler; the handler calls a `temper-services` function. (Backend trait is writes-only.)
- **Wire types live in `temper-core` with the ts-rs derive stack; regenerate with `cargo make generate-ts-types`.** Never hand-model TS.
- **Rust standards:** all public types derive `Debug`; `--all-features` for check/clippy; `#[expect(..., reason = "...")]` over `#[allow]`; typed structs over `serde_json::json!()`.
- **Deny-as-absence:** an unauthorized principal gets `404 NotFound` (never a leak, never a 500).

## File Structure

- `migrations/20260703000001_team_graph_scope_reads.sql` — **Create.** The three SQL functions: `team_descendants`, `team_child_zones`, `resources_in_team_scope`.
- `crates/temper-core/src/types/graph_scope.rs` — **Create.** Wire types `TeamRef`, `TeamZone`, `TeamScopeView` (ts-rs, serde, FromRow where applicable).
- `crates/temper-core/src/types/mod.rs` — **Modify.** Register `pub mod graph_scope;` + `pub use`.
- `crates/temper-services/src/services/team_service.rs` — **Modify.** Add `graph_scope(pool, profile_id, team_id)` service-direct read.
- `crates/temper-api/src/handlers/teams.rs` — **Modify.** Add `graph_scope` handler.
- `crates/temper-api/src/routes.rs` — **Modify.** Add `.route("/api/teams/{id}/graph-scope", get(handlers::teams::graph_scope))`.
- `tests/e2e/tests/team_graph_scope_sql_test.rs` — **Create.** SQL-level `#[sqlx::test]` proving the three functions' semantics.
- `tests/e2e/tests/team_graph_scope_e2e.rs` — **Create.** HTTP e2e proving the endpoint + access asymmetry (the acceptance gate).

---

### Task 1: SQL functions — descendant walk, child-zone enumeration, team-scope filter

**Files:**
- Create: `migrations/20260703000001_team_graph_scope_reads.sql`
- Test: `tests/e2e/tests/team_graph_scope_sql_test.rs`

**Interfaces:**
- Produces (SQL, called by later tasks):
  - `team_descendants(p_team uuid) RETURNS TABLE(team_id uuid)` — DAG-down closure `{self} ∪ all descendants`.
  - `team_child_zones(p_profile uuid, p_scope uuid) RETURNS TABLE(team_id uuid)` — direct children of `p_scope` the profile can *enter* (member of that child or any of its descendants).
  - `resources_in_team_scope(p_profile uuid, p_team uuid) RETURNS TABLE(resource_id uuid)` — resources visible to the profile bound at `p_team`'s own scope (team + ancestors; **excludes descendants' private bindings**).

- [ ] **Step 1: Write the failing SQL-level test**

Create `tests/e2e/tests/team_graph_scope_sql_test.rs`:

```rust
//! SQL-level semantics for the R1 team-graph-scope functions (Chunk A).
//! Proves the new functions directly against the migrated schema — fast feedback
//! on the DAG walk + access asymmetry, before the HTTP endpoint exists (that is
//! `team_graph_scope_e2e.rs`). Access-semantics change → also gated at the e2e HTTP tier.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    let body: serde_json::Value = resp.json().await.expect("profile json");
    body["id"].as_str().unwrap().parse().unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create team")
}

async fn link_parent(pool: &sqlx::PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link parent/child");
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')")
        .bind(team)
        .bind(profile)
        .execute(pool)
        .await
        .expect("add member");
}

/// Team-anchored read grant (the upward-transitive visibility mechanism).
async fn grant_read_to_team(pool: &sqlx::PgPool, resource: Uuid, team: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_access \
             (resource_id, anchor_table, anchor_id, can_read, granted_by_profile_id) \
         VALUES ($1, 'kb_teams', $2, true, $3)",
    )
    .bind(resource)
    .bind(team)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant read to team");
}

/// team_descendants walks DOWN the DAG (mirror of team_ancestors).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_descendants_walks_down(pool: sqlx::PgPool) {
    let eng = create_team(&pool, "tgs-eng").await;
    let group = create_team(&pool, "tgs-group").await;
    let squad_a = create_team(&pool, "tgs-squad-a").await;
    link_parent(&pool, eng, group).await;
    link_parent(&pool, group, squad_a).await;

    let mut ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM team_descendants($1) ORDER BY team_id")
            .bind(eng)
            .fetch_all(&pool)
            .await
            .expect("team_descendants");
    let mut expected = vec![eng, group, squad_a];
    ids.sort();
    expected.sort();
    assert_eq!(ids, expected, "descendants = self + group + squad_a");
}

/// team_child_zones returns a direct child only when the profile can reach it
/// (member of the child or any of its descendants); non-reachable children are excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn child_zones_are_reachable_children_only(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    // eng ─ squad-a (member reaches via being in squad-a) ; eng ─ squad-b (not a member)
    let eng = create_team(&pool, "cz-eng").await;
    let squad_a = create_team(&pool, "cz-squad-a").await;
    let squad_b = create_team(&pool, "cz-squad-b").await;
    link_parent(&pool, eng, squad_a).await;
    link_parent(&pool, eng, squad_b).await;
    add_member(&pool, squad_a, member).await;

    let zones: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM team_child_zones($1, $2) ORDER BY team_id")
            .bind(member)
            .bind(eng)
            .fetch_all(&pool)
            .await
            .expect("team_child_zones");
    assert_eq!(zones, vec![squad_a], "only squad-a is enterable; squad-b excluded");
}

/// resources_in_team_scope includes a team's own bindings + ancestors,
/// and EXCLUDES a descendant's private bindings (no downward leak).
///
/// Uses TEAM READ-GRANTS (kb_resource_access), which are the upward-transitive
/// mechanism — team-OWNED contexts are deliberately flat in resources_visible_to
/// and would not demonstrate ancestor inheritance.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_scope_excludes_descendant_privates(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    let eng = create_team(&pool, "ts-eng").await;
    let squad_a = create_team(&pool, "ts-squad-a").await;
    link_parent(&pool, eng, squad_a).await;
    add_member(&pool, squad_a, member).await; // member reaches eng upward

    // A resource read-granted to eng (ancestor of squad-a).
    let eng_res: Uuid =
        sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ('eng doc','temper://ts/eng') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    grant_read_to_team(&pool, eng_res, eng, member).await;

    // A resource read-granted to squad-a (a DESCENDANT of eng — a "private").
    let sq_res: Uuid =
        sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ('squad doc','temper://ts/sq') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    grant_read_to_team(&pool, sq_res, squad_a, member).await;

    // Scope = eng: sees eng's own resource, NOT squad-a's (descendant private).
    let in_eng_scope: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_in_team_scope($1, $2)")
            .bind(member)
            .bind(eng)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(in_eng_scope.contains(&eng_res), "eng scope includes eng's own resource");
    assert!(!in_eng_scope.contains(&sq_res), "eng scope EXCLUDES squad-a's private resource");

    // Scope = squad-a: sees squad-a's own resource AND eng's (upward inheritance).
    let in_sq_scope: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_in_team_scope($1, $2)")
            .bind(member)
            .bind(squad_a)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(in_sq_scope.contains(&sq_res), "squad-a scope includes its own resource");
    assert!(in_sq_scope.contains(&eng_res), "squad-a scope inherits eng (ancestor) upward");
}
```

- [ ] **Step 2: Build the e2e bin and run the test to verify it fails**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e 2>&1 | tee /tmp/r1-sql.log | grep -E "team_graph_scope_sql|FAIL|error\[|does not exist"`
Expected: FAIL — the three functions don't exist yet (`function team_descendants(uuid) does not exist`).

- [ ] **Step 3: Write the migration**

Create `migrations/20260703000001_team_graph_scope_reads.sql`:

```sql
-- Graph Atlas — Chunk A / R1: team-graph-scope read functions.
-- Design: docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md (read model R1).
--
-- Additive over the existing access substrate — reuses team_ancestors / resources_visible_to.
-- The view opens at a team position in the DAG and needs two navigation primitives the
-- existing (upward) visibility functions do not provide:
--   * team_child_zones  — enterable direct children (downward, membership-gated).
--   * resources_in_team_scope — a scope's OWN bindings (team + ancestors), no descendant leak.
-- team_descendants is the DAG-down mirror of team_ancestors, used by team_child_zones.
--
-- All LANGUAGE sql STABLE so runtime sqlx callers stay stable-checkable.
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).

-- ============================================================================
-- team_descendants: {self} ∪ all descendants (walk DOWN kb_teams_parents).
-- ============================================================================
CREATE FUNCTION team_descendants(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE down AS (
        SELECT p_team AS team_id
        UNION
        SELECT tp.child_id
        FROM kb_teams_parents tp
        JOIN down ON tp.parent_id = down.team_id
    )
    SELECT team_id FROM down;
$$;

-- ============================================================================
-- team_child_zones: direct children of p_scope the profile can ENTER — it is a
-- member of that child OR of any of the child's descendants (the door leads
-- somewhere the profile can go). Membership-gated, one level of children.
-- ============================================================================
CREATE FUNCTION team_child_zones(p_profile uuid, p_scope uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT c.child_id AS team_id
    FROM kb_teams_parents c
    WHERE c.parent_id = p_scope
      AND EXISTS (
          SELECT 1
          FROM team_descendants(c.child_id) d
          JOIN kb_team_members tm
            ON tm.team_id = d.team_id AND tm.profile_id = p_profile
      );
$$;

-- ============================================================================
-- resources_in_team_scope: resources VISIBLE to p_profile that are bound at
-- p_team's own scope — p_team and its ANCESTORS (upward inheritance), never a
-- descendant's private bindings. Intersected with resources_visible_to so it
-- can never exceed what the profile may already see (defense in depth).
-- ============================================================================
CREATE FUNCTION resources_in_team_scope(p_profile uuid, p_team uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    WITH scope_teams AS (
        SELECT a.team_id FROM team_ancestors(p_team) a
    ),
    scoped AS (
        -- team-anchored read grant on a scope team
        SELECT ra.resource_id
        FROM kb_resource_access ra
        JOIN scope_teams st ON ra.anchor_id = st.team_id
        WHERE ra.anchor_table = 'kb_teams' AND ra.can_read
        UNION
        -- resources homed in a context SHARED to a scope team
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        UNION
        -- resources homed in a context OWNED by a scope team
        SELECT h.resource_id
        FROM kb_contexts c
        JOIN scope_teams st ON c.owner_table = 'kb_teams' AND c.owner_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        UNION
        -- resources homed in a cogmap JOINED to a scope team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
    )
    SELECT s.resource_id
    FROM scoped s
    JOIN resources_visible_to(p_profile) v ON v.resource_id = s.resource_id;
$$;
```

- [ ] **Step 4: Apply the migration and run the test to verify it passes**

Run: `sqlx migrate run --source migrations && cargo make test-e2e 2>&1 | tee /tmp/r1-sql.log | grep -E "team_graph_scope_sql|PASS|FAIL"`
Expected: the three `team_graph_scope_sql_test` cases PASS. (If `sqlx migrate run` reports the DB is up to date but the test still errors "function does not exist," the sqlx-test template DB is stale — `cargo make db-reset` then re-run.)

- [ ] **Step 5: Commit**

```bash
git add migrations/20260703000001_team_graph_scope_reads.sql tests/e2e/tests/team_graph_scope_sql_test.rs
git commit -m "feat(graph): R1 team-graph-scope SQL functions (descendants, child-zones, scope filter)"
```

---

### Task 2: Wire types in temper-core

**Files:**
- Create: `crates/temper-core/src/types/graph_scope.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Test: `crates/temper-core/src/types/graph_scope.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces (consumed by Task 3 + temper-ui):
  - `TeamRef { id: Uuid, slug: String, name: String }`
  - `TeamZone { id: Uuid, slug: String, name: String, resource_count: i64 }`
  - `TeamScopeView { team: TeamRef, ancestors: Vec<TeamRef>, zones: Vec<TeamZone> }`
- Note: team ids are bare `Uuid` (there is no `TeamId` newtype in `ids.rs`; matches existing `team.rs`). Not `FromRow` — Task 3 constructs these by hand from rows, so only serde + ts-rs derives are needed.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-core/src/types/graph_scope.rs`:

```rust
//! Wire types for the Graph Atlas team-graph-scope read (R1).
//! See docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A team identity as it appears in the scope view (self, an ancestor, or a zone header).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamRef {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

/// An enterable child-team zone: a door the profile may drill into, with a size hint.
/// `resource_count` is the number of resources the profile would see within the child's
/// scope (child + its ancestors), i.e. `count(resources_in_team_scope(profile, child))`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamZone {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub resource_count: i64,
}

/// The team-scoped navigation frame for the graph view: the scope team, its reachable
/// ancestor set (DAG up-set, excludes self — presented as chips, not a linear path),
/// and the enterable child zones.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamScopeView {
    pub team: TeamRef,
    pub ancestors: Vec<TeamRef>,
    pub zones: Vec<TeamZone>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_scope_view_round_trips() {
        let view = TeamScopeView {
            team: TeamRef { id: Uuid::nil(), slug: "eng".into(), name: "Engineering".into() },
            ancestors: vec![TeamRef {
                id: Uuid::nil(),
                slug: "epd".into(),
                name: "EPD".into(),
            }],
            zones: vec![TeamZone {
                id: Uuid::nil(),
                slug: "squad-a".into(),
                name: "Squad A".into(),
                resource_count: 142,
            }],
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: TeamScopeView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
    }
}
```

- [ ] **Step 2: Register the module and run the test to verify it fails**

Modify `crates/temper-core/src/types/mod.rs` — add after `pub mod graph;` (line 24):

```rust
pub mod graph_scope;
```

and add the re-export after `pub use graph::{EdgeKind, Polarity};` (line 64):

```rust
pub use graph_scope::{TeamRef, TeamScopeView, TeamZone};
```

Run: `cargo test -p temper-core --features typescript graph_scope 2>&1 | grep -E "round_trips|error\[|test result"`
Expected: FAIL initially only if you run before creating the file; after Step 1 + this registration it should compile. If you ran Step 1's test before registering, expect "unresolved module" — registering here fixes it. (This is the minimal-implementation step for a pure-type task; the "failing" state is the unregistered module.)

- [ ] **Step 3: Verify the type test passes**

Run: `cargo test -p temper-core --features typescript graph_scope 2>&1 | grep -E "team_scope_view_round_trips|test result"`
Expected: PASS (`test result: ok. 1 passed`).

- [ ] **Step 4: Generate TypeScript types and confirm the output file**

Run: `cargo make generate-ts-types 2>&1 | grep -E "graph_scope|generated:|ERROR"`
Expected: `packages/temper-ui/src/lib/types/generated/graph_scope.ts` exists and defines `TeamRef`, `TeamZone`, `TeamScopeView`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/graph_scope.rs crates/temper-core/src/types/mod.rs packages/temper-ui/src/lib/types/generated/
git commit -m "feat(graph): R1 team-graph-scope wire types (ts-rs) + generated TS"
```

---

### Task 3: Service + handler + route + e2e access test

**Files:**
- Modify: `crates/temper-services/src/services/team_service.rs`
- Modify: `crates/temper-api/src/handlers/teams.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Test: `tests/e2e/tests/team_graph_scope_e2e.rs`

**Interfaces:**
- Consumes: SQL functions from Task 1; `TeamRef`/`TeamZone`/`TeamScopeView` from Task 2 (`temper_core::types::graph_scope`); `ProfileId` (`temper_core::types::ids`); `AuthUser` (`crate::middleware::auth`); `ApiError`/`ApiResult` (`temper_services::error`).
- Produces: `team_service::graph_scope(pool: &PgPool, profile_id: ProfileId, team_id: Uuid) -> ApiResult<TeamScopeView>`; handler `teams::graph_scope`; route `GET /api/teams/{id}/graph-scope`.

- [ ] **Step 1: Write the failing e2e access test**

Create `tests/e2e/tests/team_graph_scope_e2e.rs`:

```rust
//! HTTP e2e for GET /api/teams/{id}/graph-scope (R1) — the acceptance gate.
//! Proves the full stack (auth + handler + deny code) agrees, at the e2e access tier;
//! test-db predicate tests alone are a false signal for access changes.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn link_parent(pool: &sqlx::PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .unwrap();
}
async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')")
        .bind(team)
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
}

async fn graph_scope(
    app: &common::E2eTestApp,
    token: &str,
    team: Uuid,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/teams/{team}/graph-scope")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("graph-scope request failed");
    let status = resp.status();
    let body = resp.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A member of squad-a, viewing engineering (an ancestor), sees squad-a as an
/// enterable zone; squad-b (a sibling they do not belong to) is NOT shown; a
/// non-member of the whole tree gets 404 (deny-as-absence).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn scope_shows_reachable_zones_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token =
        common::generate_test_jwt("tgs-outsider", "tgs-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let eng = create_team(&pool, "e2e-eng").await;
    let squad_a = create_team(&pool, "e2e-squad-a").await;
    let squad_b = create_team(&pool, "e2e-squad-b").await;
    link_parent(&pool, eng, squad_a).await;
    link_parent(&pool, eng, squad_b).await;
    add_member(&pool, squad_a, member).await;

    // The member can view engineering (upward access from squad-a).
    let (status, body) = graph_scope(&app, &app.token, eng).await;
    assert_eq!(status, StatusCode::OK, "member of a descendant may view the ancestor scope");
    assert_eq!(body["team"]["slug"], "e2e-eng");
    let zone_slugs: Vec<&str> =
        body["zones"].as_array().unwrap().iter().map(|z| z["slug"].as_str().unwrap()).collect();
    assert!(zone_slugs.contains(&"e2e-squad-a"), "squad-a is an enterable zone");
    assert!(!zone_slugs.contains(&"e2e-squad-b"), "squad-b (not a member) is not shown");

    // An outsider (member of nothing under eng) is denied — deny-as-absence.
    let (status, _) = graph_scope(&app, &outsider_token, eng).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "outsider cannot view the scope");
}
```

- [ ] **Step 2: Build the bin and run the test to verify it fails**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e 2>&1 | tee /tmp/r1-http.log | grep -E "scope_shows_reachable_zones|FAIL|404|405"`
Expected: FAIL — route not registered (the GET returns 404/405 for the wrong reason, or the file doesn't compile because the handler is missing).

- [ ] **Step 3: Add the service function**

In `crates/temper-services/src/services/team_service.rs`, add these imports at the top of the file if not present:

```rust
use temper_core::types::graph_scope::{TeamRef, TeamScopeView, TeamZone};
use temper_core::types::ids::ProfileId;
```

Add the function (runtime `sqlx::query`, mirroring `edge_service`'s pattern of joining unqualified visibility functions; deny-as-absence via an up-reachability gate):

```rust
/// R1 team-graph-scope read: the scope team, its reachable ancestors, and the
/// child-team zones the profile may enter. Deny-as-absence (404) when the profile
/// cannot view the team (not a member of the team or any of its descendants).
pub async fn graph_scope(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    team_id: uuid::Uuid,
) -> ApiResult<TeamScopeView> {
    // Access gate: the profile must be a member of the team or a descendant (upward read).
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM team_descendants($1) d
            JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2
        )",
    )
    .bind(team_id)
    .bind(*profile_id)
    .fetch_one(pool)
    .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    // The scope team itself.
    let team: TeamRef = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT id, slug, name FROM kb_teams WHERE id = $1",
    )
    .bind(team_id)
    .fetch_optional(pool)
    .await?
    .map(|(id, slug, name)| TeamRef { id, slug, name })
    .ok_or(ApiError::NotFound)?;

    // Reachable ancestors (up-set, excluding self).
    let ancestors: Vec<TeamRef> = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT t.id, t.slug, t.name
           FROM team_ancestors($1) a
           JOIN kb_teams t ON t.id = a.team_id
          WHERE a.team_id <> $1
          ORDER BY t.name",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, slug, name)| TeamRef { id, slug, name })
    .collect();

    // Enterable child zones + size hint (count of resources in the child's scope).
    let zones: Vec<TeamZone> = sqlx::query_as::<_, (uuid::Uuid, String, String, i64)>(
        "SELECT t.id, t.slug, t.name,
                (SELECT count(*) FROM resources_in_team_scope($2, t.id)) AS resource_count
           FROM team_child_zones($2, $1) z
           JOIN kb_teams t ON t.id = z.team_id
          ORDER BY t.name",
    )
    .bind(team_id)
    .bind(*profile_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, slug, name, resource_count)| TeamZone { id, slug, name, resource_count })
    .collect();

    Ok(TeamScopeView { team, ancestors, zones })
}
```

Confirm `ApiError`/`ApiResult` are already imported in this file (they are used by existing team_service fns); if not, add `use crate::error::{ApiError, ApiResult};`.

- [ ] **Step 4: Add the handler**

In `crates/temper-api/src/handlers/teams.rs`, ensure these imports exist (add any missing):

```rust
use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;
use crate::middleware::auth::AuthUser;
use temper_core::types::graph_scope::TeamScopeView;
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiResult;
use temper_services::services::team_service;
use temper_services::state::AppState;
```

Add the handler:

```rust
/// GET /api/teams/{id}/graph-scope — R1 team-graph-scope navigation frame.
#[utoipa::path(
    get,
    path = "/api/teams/{id}/graph-scope",
    tag = "teams",
    params(("id" = Uuid, Path, description = "Team id to scope the graph to")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team scope view", body = TeamScopeView),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn graph_scope(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<TeamScopeView>> {
    team_service::graph_scope(&state.pool, ProfileId::from(auth.0.profile.id), team_id)
        .await
        .map(Json)
}
```

- [ ] **Step 5: Register the route**

In `crates/temper-api/src/routes.rs`, in the `gated` builder near the other `/api/teams/...` routes (around line 98), add:

```rust
        .route(
            "/api/teams/{id}/graph-scope",
            get(handlers::teams::graph_scope),
        )
```

- [ ] **Step 6: Build the bin and run the e2e test to verify it passes**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e 2>&1 | tee /tmp/r1-http.log | grep -E "scope_shows_reachable_zones|team_graph_scope|test result|FAIL"`
Expected: `scope_shows_reachable_zones_and_denies_outsiders` PASS, and the Task-1 SQL tests still PASS.

- [ ] **Step 7: Full gate + commit**

Run: `cargo make check 2>&1 | tail -20`
Expected: fmt + clippy + docs + typecheck + biome all green.

```bash
git add crates/temper-services/src/services/team_service.rs \
        crates/temper-api/src/handlers/teams.rs \
        crates/temper-api/src/routes.rs \
        tests/e2e/tests/team_graph_scope_e2e.rs
git commit -m "feat(graph): R1 GET /api/teams/{id}/graph-scope endpoint + e2e access test"
```

---

## Self-Review

**1. Spec coverage (R1):**
- "descendant-zone enumeration (downward, membership-gated)" → `team_child_zones` (Task 1) + zones in the endpoint (Task 3). ✅
- "team-scope filter — restrict to T's own bindings, exclude descendants' privates" → `resources_in_team_scope` (Task 1), proven by `team_scope_excludes_descendant_privates`. ✅
- "ancestor breadcrumb" → `TeamScopeView.ancestors` (Task 2/3), via `team_ancestors`. ✅
- "proven at the e2e access tier" → `team_graph_scope_e2e.rs` + `cargo make test-e2e` gate. ✅
- "wire types generate cleanly; no hand-modeled TS" → Task 2 + `cargo make generate-ts-types`. ✅

**2. Placeholder scan:** No TBD/stub steps; every code step shows complete code. ✅

**3. Type consistency:** `graph_scope`/`TeamRef`/`TeamZone`/`TeamScopeView` names and field types (`id: Uuid`, `resource_count: i64`) are identical across Task 2 (definition), Task 2's re-export, and Task 3 (construction). `ProfileId` is dereferenced (`*profile_id`) at every bind boundary. SQL function names (`team_descendants`, `team_child_zones`, `resources_in_team_scope`, `team_ancestors`) match between the migration and every caller. Route path `/api/teams/{id}/graph-scope` matches the handler `#[utoipa::path]` and the e2e URL. ✅

## Notes for the executor / controller

- **Controller runs the DB/e2e tiers**, not the implementer subagent (background cargo stalls implementers): the implementer writes code + the focused test; the controller runs `cargo build -p temper-cli --bin temper` (nextest rebuilds the lib, not the bin) then `cargo make test-e2e`, plus the full `cargo make check`, and commits. (`feedback_sdd_subagents_stall_on_backgrounded_cargo`, `feedback_nextest_does_not_rebuild_spawned_temper_bin`, `feedback_implementer_subagents_must_run_fmt`.)
- **Zone-count semantics** (`resource_count` = ancestor-inclusive scope count) is a deliberate v1 choice ("what you'd see on entering"); revisit if it reads as double-counting (goal open question).
- **No `.sqlx` regeneration** this chunk (runtime queries only). If a future change converts any of these to `query!`/`query_as!` macros, run `cargo make prepare-e2e`.
