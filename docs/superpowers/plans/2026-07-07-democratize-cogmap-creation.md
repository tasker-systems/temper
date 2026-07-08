# Democratize cogmap creation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let any authenticated profile create a non-reserved cognitive map (becoming its grant-holder), and let a team Owner/Maintainer who administers a map bind it to their (non-root) team — without system-admin.

**Architecture:** Enforce all three behaviors **server-side** so MCP / HTTP / CLI inherit them. Create-authz drops the surface `is_system_admin` gate; the backend genesis path gains a reserved-id guard (caller-supplied ids honored only for admins) — the creator-grant it already mints then applies to non-admins for free. Bind/unbind gains a two-sided gate centralized in the service layer.

**Tech Stack:** Rust (axum, sqlx, rmcp), PostgreSQL + pgvector. Design spec: `docs/superpowers/specs/2026-07-07-democratize-cogmap-creation-design.md`.

## Global Constraints

- **Additive-only on `main`** — no destructive schema changes; this plan touches no migrations.
- **Auth before writes** — every authorization check precedes any mutation.
- **Persistence stays in the service layer** — no inline SQL in surfaces; the bind gate is one service helper both surfaces call (mirrors `access_service::can_administer_grant`).
- **SQL macros** — new `sqlx::query_scalar!()` in `temper-services` (lib) requires regenerating the workspace cache: `cargo sqlx prepare --workspace -- --all-features`. The e2e tests use runtime `sqlx::query(...)` (no cache).
- **e2e is the honest test layer** — real Axum + real Postgres + real JWT; `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`. Run with `cargo make test-e2e`.
- **Verify before done** — `cargo make check` (offline sqlx) must pass before any commit.
- **No concurrent `test-e2e` runs** (shared harness constraint).

---

### Task 1: Democratize create + reserved-id hardening

Drop the `is_system_admin` surface gate on genesis; the backend honors a caller-supplied id **only for admins** (non-admins always get server-minted ids). The creator-grant already minted at `db_backend.rs:1584-1602` then makes the new map authorable by its non-admin creator with no further change.

**Files:**
- Modify: `crates/temper-services/src/backend/db_backend.rs:1465-1484` (id-resolution + doc)
- Modify: `crates/temper-mcp/src/tools/cognitive_maps.rs:186-207` (drop gate + doc)
- Modify: `crates/temper-api/src/handlers/cognitive_maps.rs:88-121` (drop gate + doc + utoipa 403)
- Modify: `crates/temper-cli/src/cli.rs:920-924` and `crates/temper-cli/src/commands/cogmap.rs:186-188` (help text: ids honored for admins only)
- Test: `tests/e2e/tests/genesis_cogmap_e2e.rs` (replace the non-admin-denied test)

**Interfaces:**
- Consumes: `access_service::is_system_admin(pool, ProfileId) -> ApiResult<bool>` (existing); `DbBackend.profile_id: ProfileId`, `DbBackend.pool: PgPool` (existing private fields).
- Produces: no new public signatures. Behavioral contract: `POST /api/cognitive-maps` succeeds for any authenticated profile; the returned `CreateCogmapOutcome.cogmap_id` equals a caller-supplied id **only** when the caller is `is_system_admin`, else a server-minted uuidv7.

- [ ] **Step 1: Rewrite the non-admin genesis e2e to expect success + server-minted id**

In `tests/e2e/tests/genesis_cogmap_e2e.rs`, replace the entire `non_admin_genesis_is_denied` test (lines 113-161) with:

```rust
// ── (b) non-admin genesis now SUCCEEDS, but its caller-supplied id is IGNORED (server-minted) ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_genesis_succeeds_with_server_minted_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // A SECOND user with system access (a `watcher` of temper-system) but NOT admin.
    let second_token = common::generate_second_user_jwt();
    let second_id = provision_profile(&app, &second_token).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system'
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("add second user as watcher");

    let req = genesis_request(); // supplies fixed cogmap_id / telos_resource_id
    let resp = app
        .reqwest_client
        .post(app.url("/api/cognitive-maps"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&req)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a non-admin may now genesis a non-reserved map"
    );
    let body: serde_json::Value = resp.json().await.expect("json parse");
    assert_eq!(body["created"], true, "the map was created");

    // Reserved-id hardening: the caller-supplied id was IGNORED; the server minted a fresh one.
    let returned_id: Uuid = body["cogmap_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(
        returned_id,
        req.cogmap_id.unwrap(),
        "a non-admin's supplied id must be ignored and server-minted"
    );
    let requested_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kb_cogmaps WHERE id = $1)")
            .bind(req.cogmap_id.unwrap())
            .fetch_one(&pool)
            .await
            .expect("exists query");
    assert!(!requested_exists, "nothing was written at the caller-supplied id");

    // Creator-grant: the non-admin creator holds `grant` on and can AUTHOR the new map.
    let can_grant: bool =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_cogmaps', $2)")
            .bind(second_id)
            .bind(returned_id)
            .fetch_one(&pool)
            .await
            .expect("can grant query");
    assert!(can_grant, "the creator holds can_grant on its new map");

    let authorable: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(second_id)
        .bind(returned_id)
        .fetch_one(&pool)
        .await
        .expect("authorable query");
    assert!(authorable, "the creator can author its new map immediately");
}
```

- [ ] **Step 2: Run the new test to verify it fails**

Run: `cargo make test-e2e 2>&1 | tee /tmp/t1.log; grep -E "non_admin_genesis_succeeds|FAIL|FORBIDDEN|assertion" /tmp/t1.log`
Expected: FAIL — the request currently returns `403 FORBIDDEN` (existing surface gate), so `assert_eq!(status, OK)` fails.

- [ ] **Step 3: Drop the MCP surface gate**

In `crates/temper-mcp/src/tools/cognitive_maps.rs`, replace the doc-comment + admin gate on `cogmap_create` (lines 186-206). Delete the `is_admin` check block (lines 197-206) and reword the doc:

```rust
/// Genesis (create) a new cognitive map. Any authenticated profile may create a NON-RESERVED map and
/// becomes its grant-holder (the backend mints a read+write+grant on the new map). The reserved-id
/// guard lives in the backend: a caller-supplied `cogmap_id`/`telos_resource_id` is honored only for a
/// system-admin, so a non-admin can never place a map at a chosen (e.g. reserved) id. The map is born
/// with an EMPTY charter (see [`CogmapCreateInput`]).
pub async fn cogmap_create(
    svc: &TemperMcpService,
    input: CogmapCreateInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = CreateCognitiveMap {
```

(The rest of the function body — building `cmd`, `DbBackend::new`, `create_cognitive_map`, error mapping — is unchanged. Remove the now-unused `access_service` import only if nothing else in the file uses it; `access_service` is still used elsewhere, so leave the `use` line.)

- [ ] **Step 4: Drop the HTTP surface gate**

In `crates/temper-api/src/handlers/cognitive_maps.rs`, edit the `genesis` handler (lines 98-121) and its utoipa `responses` (line 94). Remove the `is_system_admin` block (lines 103-109) and reword:

```rust
        (status = 200, description = "Genesis applied (or idempotent no-op)", body = CreateCogmapOutcome),
        (status = 403, description = "Caller lacks system access (invite-only middleware)"),
        (status = 409, description = "A concurrent genesis conflicted; retry"),
    )
)]
pub async fn genesis(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(request): Json<CreateCogmapRequest>,
) -> ApiResult<Json<CreateCogmapOutcome>> {
    // Genesis is open to any authenticated profile. The reserved-id guard and the creator-grant live
    // in the backend command (`create_cognitive_map`): a caller-supplied id is honored only for a
    // system-admin, and the creator is granted read+write+grant on the new map.
    let profile_id = ProfileId::from(auth.0.profile.id);
    let cmd = CreateCognitiveMap {
        request,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), profile_id);
    let out = backend
        .create_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
```

If `access_service` becomes unused in this file after removing the check, delete its `use` import to satisfy clippy (`cargo make check` will flag it if so).

- [ ] **Step 5: Add the reserved-id guard in the backend**

In `crates/temper-services/src/backend/db_backend.rs`, replace the id-resolution block (lines 1474-1484) with the admin-gated version. Also update the doc-comment at lines 1465-1466.

Doc-comment (near line 1465) — replace the "surface gates on is_system_admin first" sentence with:

```rust
    /// Create is open to any authenticated profile (no surface admin gate). Two guards live HERE: a
    /// caller-supplied `cogmap_id`/`telos_resource_id` is honored only for a system-admin (else
    /// server-minted — a non-admin can never choose a reserved id), and the creator is granted
    /// read+write+grant on the new map (below).
```

Id-resolution (lines 1474-1484):

```rust
        // Reserved-id hardening: honor a caller-supplied id ONLY for a system-admin. A non-admin's ids
        // are ignored and the server mints fresh uuidv7s, so a non-admin can never place a map at a
        // chosen (e.g. reserved L0/system) id. Explicit-id genesis stays operator work.
        let caller_is_admin =
            crate::services::access_service::is_system_admin(&self.pool, self.profile_id)
                .await
                .map_err(|e| TemperError::Api(e.to_string()))?;
        let requested_cogmap_id = if caller_is_admin { cmd.request.cogmap_id } else { None };
        let requested_telos_id = if caller_is_admin { cmd.request.telos_resource_id } else { None };
        let cogmap_id = requested_cogmap_id
            .map(CogmapId::from)
            .unwrap_or_else(|| CogmapId::from(uuid::Uuid::now_v7()));
        let telos_resource_id = requested_telos_id
            .map(ResourceId::from)
            .unwrap_or_else(|| ResourceId::from(uuid::Uuid::now_v7()));
        let cogmap_uuid = uuid::Uuid::from(cogmap_id);
```

- [ ] **Step 6: Fix stale CLI help text**

In `crates/temper-cli/src/cli.rs`, the `Create` variant doc (around lines 920-924) — change "POSTs to `/api/cognitive-maps` (admin-gated, idempotent)" and "Ids absent from the manifest are minted client-side …" to:

```rust
    /// Genesis (create) a new cognitive map from a committed manifest.
    ///
    /// Reads the authored genesis manifest (name, telos title, optional ids + telos charter),
    /// embeds the charter client-side, and POSTs to `/api/cognitive-maps` (open to any authenticated
    /// profile; idempotent). Manifest/`--id` ids are honored only for a system-admin — a non-admin
    /// always receives a server-minted id.
```

In `crates/temper-cli/src/commands/cogmap.rs` (lines 186-188), update the `create` fn doc-comment similarly (drop "stable, reproducible" absolutes; note admin-only id honoring).

- [ ] **Step 7: Run the e2e genesis suite to verify it passes**

Run: `cargo make test-e2e 2>&1 | tee /tmp/t1b.log; grep -E "genesis_cogmap|test result|FAIL" /tmp/t1b.log`
Expected: PASS — `admin_genesis_creates_then_is_idempotent` (admin explicit-id still honored) and `non_admin_genesis_succeeds_with_server_minted_id` both green.

- [ ] **Step 8: Quality gate + commit**

Run: `cargo make check`
Expected: clean (no unused-import or clippy warnings).

```bash
git add crates/temper-services/src/backend/db_backend.rs \
        crates/temper-mcp/src/tools/cognitive_maps.rs \
        crates/temper-api/src/handlers/cognitive_maps.rs \
        crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/cogmap.rs \
        tests/e2e/tests/genesis_cogmap_e2e.rs
git commit -m "feat(cogmap): democratize genesis — any authed profile creates; reserved-id honored admin-only"
```

---

### Task 2: Two-sided bind/unbind gate

Replace the `is_system_admin` gate in `bind_team`/`unbind_team` with: `is_system_admin OR (can_manage(team) AND can_grant(map) AND team != gating team)`. Centralize in one service helper both surfaces already call.

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (add `profile_can_grant` + `is_gating_team`; DRY-refactor `can_administer_grant`)
- Modify: `crates/temper-services/src/services/cogmap_service.rs` (add `can_bind`; rewrite both gates + module/​fn docs)
- Modify: `crates/temper-cli/src/cli.rs:957-968` and `crates/temper-cli/src/commands/cogmap.rs:63` (drop "admin-only" from Bind/Unbind help)
- Test: `tests/e2e/tests/bind_cogmap_e2e.rs` (replace the non-admin-denied test with the democratized matrix)

**Interfaces:**
- Consumes: `access_service::is_system_admin` (existing); `team_service::role_on_team(pool, team_id: Uuid, ProfileId) -> ApiResult<Option<TeamRole>>` and `team_service::can_manage(TeamRole) -> bool` (existing `pub(crate)`); the SQL functions `can('kb_profiles', <uuid>, 'grant', <text>, <uuid>)` and `cogmap_authorable_by_profile`.
- Produces:
  - `access_service::profile_can_grant(pool, ProfileId, subject_table: &str, subject_id: Uuid) -> ApiResult<bool>` (`pub(crate)`)
  - `access_service::is_gating_team(pool, team_id: Uuid) -> ApiResult<bool>` (`pub(crate)`)
  - `cogmap_service::can_bind(pool, ProfileId, cogmap_id: Uuid, team_id: Uuid) -> ApiResult<bool>` (private)

- [ ] **Step 1: Write the failing e2e matrix for democratized bind**

In `tests/e2e/tests/bind_cogmap_e2e.rs`, add a helper and replace `non_admin_bind_is_denied_and_writes_nothing` (lines 252-307) with the four tests below. First add this helper near `team_with_visible_resource`:

```rust
/// Make `profile` a member of a fresh NON-gating team at `role`. Returns the team id.
async fn team_with_role(pool: &sqlx::PgPool, profile: Uuid, role: &str) -> Uuid {
    let team_id = Uuid::now_v7();
    let slug = format!("role-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team_id)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)")
        .bind(team_id)
        .bind(profile)
        .bind(role)
        .execute(pool)
        .await
        .expect("insert membership");
    team_id
}

/// Provision the second (non-admin) user with system access; returns (token, profile_id).
async fn second_user(app: &common::E2eTestApp, pool: &sqlx::PgPool) -> (String, Uuid) {
    let token = common::generate_second_user_jwt();
    let id = provision_profile(app, &token).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system'
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("second user system access");
    (token, id)
}

/// The second user genesis-creates their OWN map (non-admin genesis) → they hold can_grant on it.
/// Returns the server-minted cogmap id.
async fn second_user_creates_map(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/cognitive-maps"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "name": "My map", "telos_title": "My telos" }))
        .send()
        .await
        .expect("genesis request");
    assert_eq!(resp.status(), StatusCode::OK, "non-admin genesis succeeds");
    let body: serde_json::Value = resp.json().await.expect("json");
    body["cogmap_id"].as_str().unwrap().parse().unwrap()
}

async fn bind_status(app: &common::E2eTestApp, token: &str, cogmap_id: Uuid, team_id: Uuid) -> StatusCode {
    app.reqwest_client
        .post(app.url(&format!("/api/cognitive-maps/{cogmap_id}/teams")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "team_id": team_id }))
        .send()
        .await
        .expect("bind request")
        .status()
}

// ── (c) a team Maintainer who administers the map may bind it to their team ───────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn maintainer_with_map_grant_binds_own_map(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (token, second_id) = second_user(&app, &pool).await;
    let cogmap_id = second_user_creates_map(&app, &token).await; // holds can_grant via creator-grant
    let team_id = team_with_role(&pool, second_id, "maintainer").await;

    assert_eq!(bind_status(&app, &token, cogmap_id, team_id).await, StatusCode::OK);
    assert!(binding_exists(&pool, cogmap_id, team_id).await, "the binding was written");

    // Symmetric unbind: the same principal may unbind.
    let unbind = app
        .reqwest_client
        .delete(app.url(&format!("/api/cognitive-maps/{cogmap_id}/teams/{team_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("unbind request");
    assert_eq!(unbind.status(), StatusCode::OK, "maintainer may unbind their own map");
    assert!(!binding_exists(&pool, cogmap_id, team_id).await, "the binding was removed");
}

// ── (d) a mere team Member (not can_manage) is denied even holding the map grant ──────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_cannot_bind(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (token, second_id) = second_user(&app, &pool).await;
    let cogmap_id = second_user_creates_map(&app, &token).await;
    let team_id = team_with_role(&pool, second_id, "member").await; // not can_manage

    assert_eq!(bind_status(&app, &token, cogmap_id, team_id).await, StatusCode::FORBIDDEN);
    assert!(!binding_exists(&pool, cogmap_id, team_id).await, "a denied bind writes nothing");
}

// ── (e) a team Maintainer who does NOT administer the map is denied (map-side gate) ───────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn maintainer_without_map_grant_cannot_bind(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // The ADMIN owns this map (second user holds no grant on it).
    let admins_map = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&genesis_request())
        .await
        .expect("admin genesis")
        .cogmap_id;

    let (token, second_id) = second_user(&app, &pool).await;
    let team_id = team_with_role(&pool, second_id, "maintainer").await;

    assert_eq!(bind_status(&app, &token, admins_map, team_id).await, StatusCode::FORBIDDEN);
    assert!(!binding_exists(&pool, admins_map, team_id).await, "a denied bind writes nothing");
}

// ── (f) binding to the gating/root team stays admin-only (escalation guard) ───────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn bind_to_gating_team_denied_for_non_admin(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (token, second_id) = second_user(&app, &pool).await;
    let cogmap_id = second_user_creates_map(&app, &token).await; // holds can_grant on their map

    // Promote the second user to MAINTAINER of the gating team (temper-system).
    let gating_team_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .expect("gating team id");
    sqlx::query("UPDATE kb_team_members SET role = 'maintainer' WHERE team_id = $1 AND profile_id = $2")
        .bind(gating_team_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("promote to maintainer of gating team");

    assert_eq!(
        bind_status(&app, &token, cogmap_id, gating_team_id).await,
        StatusCode::FORBIDDEN,
        "binding to the gating team must stay admin-only even for a maintainer"
    );
    assert!(!binding_exists(&pool, cogmap_id, gating_team_id).await, "no escalation binding written");
}
```

- [ ] **Step 2: Run the new bind tests to verify they fail**

Run: `cargo make test-e2e 2>&1 | tee /tmp/t2.log; grep -E "maintainer_with_map_grant|member_cannot|without_map_grant|gating_team|FAIL" /tmp/t2.log`
Expected: FAIL — `maintainer_with_map_grant_binds_own_map` currently gets `403` (bind is still `is_system_admin`-only), so its `assert_eq!(OK)` fails.

- [ ] **Step 3: Add the access_service helpers + DRY-refactor**

In `crates/temper-services/src/services/access_service.rs`, add after `can_administer_grant` (after line 81), and refactor `can_administer_grant` (lines 62-81) to reuse the new probe:

```rust
async fn can_administer_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    Ok(is_system_admin(pool, caller).await?
        || profile_can_grant(pool, caller, subject_table, subject_id).await?)
}

/// Raw `can_grant` capability probe (NO `is_system_admin` OR) — the reusable primitive. Callers that
/// also admit admins compose it with `is_system_admin` themselves (see `can_administer_grant`,
/// `cogmap_service::can_bind`).
pub(crate) async fn profile_can_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, 'grant', $2, $3)",
        *caller,
        subject_table,
        subject_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}

/// Is `team_id` the configured gating/root team? An unconfigured system (`gating_team_slug` NULL)
/// has no gating team ⇒ `false`. Used by the bind gate's escalation guard: binding a map to the
/// gating team flips it into the `require_cogmap_write_admin` regime, so it stays admin-only.
pub(crate) async fn is_gating_team(pool: &PgPool, team_id: Uuid) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT EXISTS( \
           SELECT 1 FROM kb_teams t \
             JOIN kb_system_settings s ON t.slug = s.gating_team_slug \
            WHERE t.id = $1 )",
        team_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}
```

- [ ] **Step 4: Add `can_bind` and rewrite both gates in cogmap_service**

In `crates/temper-services/src/services/cogmap_service.rs`: add the `team_service` import, add `can_bind`, and replace the `is_system_admin` blocks in `bind_team` (lines 32-35) and `unbind_team` (lines 93-96). Update the module doc (lines 9-12) to describe the two-sided gate.

Add import near line 18:

```rust
use crate::services::{access_service, team_service};
```

Add the helper (e.g. after `bind_team`):

```rust
/// Two-sided bind/unbind gate. Allowed IFF `is_system_admin`, OR the caller can administer the MAP
/// (`can_grant` on it) AND may manage the TEAM (`can_manage` = Owner|Maintainer, direct membership)
/// AND the team is NOT the gating/root team. Binding to the gating team flips the map into the
/// `require_cogmap_write_admin` regime, so that stays admin-only (escalation guard).
async fn can_bind(
    pool: &PgPool,
    caller: ProfileId,
    cogmap_id: Uuid,
    team_id: Uuid,
) -> ApiResult<bool> {
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(true);
    }
    if access_service::is_gating_team(pool, team_id).await? {
        return Ok(false);
    }
    let team_ok = matches!(
        team_service::role_on_team(pool, team_id, caller).await?,
        Some(role) if team_service::can_manage(role)
    );
    if !team_ok {
        return Ok(false);
    }
    access_service::profile_can_grant(pool, caller, "kb_cogmaps", cogmap_id).await
}
```

In `bind_team`, replace lines 32-35 with:

```rust
    // Auth before writes: system-admin, OR a team manager who administers the map (non-root team).
    if !can_bind(pool, caller, cogmap_id, req.team_id).await? {
        return Err(ApiError::Forbidden);
    }
```

In `unbind_team`, replace lines 93-96 with:

```rust
    // Auth before writes: symmetric with bind — a principal who could bind may unbind.
    if !can_bind(pool, caller, cogmap_id, team_id).await? {
        return Err(ApiError::Forbidden);
    }
```

- [ ] **Step 5: Regenerate the sqlx cache**

The two new `query_scalar!` macros are lib queries in `temper-services`.

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: `.sqlx/` updated with the two new query hashes; command exits 0. (Requires the dev Postgres up: `cargo make docker-up`, `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.)

- [ ] **Step 6: Fix stale CLI "admin-only" bind help**

In `crates/temper-cli/src/cli.rs`, the `Bind` variant (lines 957-968) — reword:

```rust
    /// Bind a cognitive map to a team. Requires system-admin, OR that you manage the team
    /// (owner/maintainer) AND administer the map (hold a grant on it). Widens the map's reach to the
    /// team's shared resources.
    Bind {
```

and the `Unbind` doc — replace "(admin-only)" with "(same authority as bind)". In `crates/temper-cli/src/commands/cogmap.rs` line 63, drop "(admin-only)" from the `bind` fn doc-comment.

- [ ] **Step 7: Run the full bind suite to verify it passes**

Run: `cargo make test-e2e 2>&1 | tee /tmp/t2b.log; grep -E "bind_cogmap|test result|FAIL" /tmp/t2b.log`
Expected: PASS — `admin_bind_*`, `unbind_reverts_*`, `maintainer_with_map_grant_binds_own_map`, `member_cannot_bind`, `maintainer_without_map_grant_cannot_bind`, `bind_to_gating_team_denied_for_non_admin` all green.

- [ ] **Step 8: Quality gate + commit**

Run: `cargo make check`
Expected: clean (offline sqlx cache resolves the two new queries).

```bash
git add crates/temper-services/src/services/access_service.rs \
        crates/temper-services/src/services/cogmap_service.rs \
        crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/cogmap.rs \
        tests/e2e/tests/bind_cogmap_e2e.rs .sqlx
git commit -m "feat(cogmap): two-sided bind/unbind gate — team manager + map grant, root-team guarded"
```

---

### Task 3: Full verification sweep

**Files:** none (verification + cache-consistency only).

- [ ] **Step 1: Confirm the sqlx cache is complete and honest**

Run: `cargo make check`
Expected: PASS (this task forces `SQLX_OFFLINE=true`, so it is the honest probe of the committed `.sqlx` cache).

- [ ] **Step 2: Run the cogmap e2e suite in full**

Run: `cargo make test-e2e 2>&1 | tee /tmp/t3.log; grep -E "genesis_cogmap|bind_cogmap|cogmap_write_grants|test result: FAILED|error: test run failed" /tmp/t3.log`
Expected: no `FAILED` / `test run failed`; genesis + bind + `cogmap_write_grants_e2e` (regression: existing grant paths) all pass.

- [ ] **Step 3: Run the temper-services unit + integration suite**

Run: `cargo nextest run -p temper-services --features test-db 2>&1 | tail -20`
Expected: `access_service` / `cogmap_service` / `team_service` tests pass; exit 0 (do not trust the per-binary Summary line — grep for `error: test run failed`).

- [ ] **Step 4: Workspace test sweep (branch-end guard)**

Run: `cargo make test 2>&1 | tail -20`
Expected: green. Then confirm no orphaned `.sqlx` files: `git status .sqlx` shows only additions/updates, no deletions expected.

## Self-Review

- **Spec coverage:**
  - ① Create democratized → Task 1 (drop MCP+HTTP gates) ✓
  - ① Reserved-id hardening (server-mint-only for non-admins) → Task 1 Step 5 ✓
  - ② Creator grant atomic with genesis → already present (`db_backend.rs:1584-1602`); proven by Task 1 Step 1 assertions ✓
  - ③ Bind two-sided gate (can_manage AND can_grant AND ≠ gating team) → Task 2 ✓
  - ③′ Unbind symmetric → Task 2 (`can_bind` shared) + Step 1 unbind assertion ✓
  - Preserved: `require_cogmap_write_admin`, `cogmap_grant/revoke` untouched; `cogmap_write_grants_e2e` regression in Task 3 ✓
  - Full MCP+HTTP+CLI parity + doc corrections → Tasks 1 & 2 ✓
- **Placeholder scan:** no TBD/TODO; every code step shows full code; every run step has an expected result. ✓
- **Type consistency:** `can_bind(pool, ProfileId, cogmap_id: Uuid, team_id: Uuid)` used identically in `bind_team`/`unbind_team`; `profile_can_grant`/`is_gating_team` signatures match their call sites; `role_on_team`/`can_manage` used per their existing `pub(crate)` signatures. ✓
