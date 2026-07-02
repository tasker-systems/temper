# `temper admin saml` provisioning + `temper context share` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add operator-provisioning surfaces — a typed SAML emitter (`temper admin saml provision|map-group|verify`) and a `temper context share|unshare` writer for `kb_team_contexts` — so standing up a self-hosted SAML instance needs no hand-written keys, env, or SQL.

**Architecture:** Two limbs. **Limb 2 (context-share)** mirrors the shipped `cogmap bind`/`unbind` stack almost 1:1 (wire types → service with `is_system_admin` gate → API handler+route → client → CLI). **Limb 1 (SAML emitter)** is a pure typed core (one `SamlProvisionConfig` struct renders env + SQL, consistency-by-construction; Rust-native ed25519 PKCS#8 keygen) wrapped by a `temper admin saml` command shell that emits by default and, with `--apply`, shells out to `psql` (mirroring `open_in_editor`). Emit is inert (no auth); `--apply`/`--from-seen`/`verify --db` are gated by possession of `DATABASE_URL` + `psql`.

**Tech Stack:** Rust (clap, dialoguer, ed25519-dalek, pkcs8, rand, base64, sqlx, axum, reqwest), the temper workspace crates (`temper-core`, `temper-services`, `temper-api`, `temper-client`, `temper-cli`). Spec: `docs/superpowers/specs/2026-07-02-admin-saml-provisioning-and-context-share-design.md`.

## Global Constraints

- **Typed structs over inline JSON/SQL-strings** — the emitter renders from one `SamlProvisionConfig`; no `serde_json::json!()` for known shapes. (CLAUDE.md code-quality)
- **Auth before writes** — `is_system_admin` at the TOP of each context-share service fn, before any mutation. (CLAUDE.md)
- **Persistence stays in the service layer** — no inline `sqlx::query!()` in handlers/CLI. Context-share SQL lives in `context_service`. (CLAUDE.md)
- **sqlx compile-time macros** — after any `sqlx::query!()` change run `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` (service-target cache) and `cargo make prepare-api`/`prepare-e2e` as touched. `cargo make check` runs `SQLX_OFFLINE=true` — the honest local probe. (CLAUDE.md)
- **Cross-runtime wire types** live in `temper-core` with the full derive stack: `#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]` + `ts(export, export_to = "context.ts")`, `#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]`, `#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]`, `Serialize`/`Deserialize`. Regenerate with `cargo make generate-ts-types` and **commit all regenerated `.ts` that change**, even unrelated files.
- **All public types implement `Debug`.** (CLAUDE.md)
- **No `[workspace.dependencies]` table exists** — pin dep versions directly in `crates/temper-cli/Cargo.toml`. `base64 = "0.22"` is already used elsewhere — match it.
- **Access/membership-semantics changes need the e2e tier** — `test-db` green is a false signal for context-share; run `cargo make test-e2e`. After editing the `temper` binary that e2e spawns, `cargo build -p temper-cli --bin temper` first (nextest rebuilds the lib, not the bin).
- **Branch:** `jct/admin-saml-provisioning-context-share` (already created; the spec is committed there). Commit per task.

---

## File Structure

**Limb 2 — context-share (mirror `cogmap bind`):**
- Modify `crates/temper-core/src/types/context.rs` — add `ShareContextRequest`, `ShareContextOutcome`, `UnshareContextOutcome`.
- Modify `crates/temper-services/src/services/context_service.rs` — add `share`/`unshare` (is_system_admin-gated).
- Modify `crates/temper-api/src/handlers/contexts.rs` — add `share_team`/`unshare_team`; `crates/temper-api/src/routes.rs` — routes; `crates/temper-api/src/openapi.rs` — register.
- Modify `crates/temper-client/src/contexts.rs` — add `share_team`/`unshare_team`.
- Modify `crates/temper-cli/src/cli.rs` (`ContextAction`), `crates/temper-cli/src/commands/context_cmd.rs` (share/unshare remote + `resolve_context_id`), `crates/temper-cli/src/main.rs` (dispatch).
- Test: `tests/e2e/tests/context_share_e2e.rs` (new).

**Limb 1 — SAML emitter (new, `temper-cli`-local operator tooling):**
- Create `crates/temper-cli/src/saml/mod.rs` — pure core: `SamlProvisionConfig`, keygen, env + SQL rendering. Unit-tested, no I/O.
- Create `crates/temper-cli/src/commands/admin_saml.rs` — the I/O shell: prompts, emit/`--env-out`, `--apply`/`--from-seen`/`verify` via `psql`, verify probes.
- Modify `crates/temper-cli/src/cli.rs` (`AdminAction::Saml`, new `AdminSamlAction`), `crates/temper-cli/src/main.rs` (dispatch), `crates/temper-cli/Cargo.toml` (deps), `crates/temper-cli/src/lib.rs` (`pub mod saml;`).

**Docs:**
- Modify `docs/guides/self-hosting-saml.md`, `docs/guides/org-bootstrap.md`.

---

# LIMB 2 — Context-share (Beat A)

### Task A1: Context-share wire types + service (`is_system_admin`-gated)

**Files:**
- Modify: `crates/temper-core/src/types/context.rs`
- Modify: `crates/temper-services/src/services/context_service.rs`
- Test: inside `context_service.rs` `#[cfg(test)]` (an `#[sqlx::test]`)

**Interfaces:**
- Produces: `ShareContextRequest { team_id: Uuid }`, `ShareContextOutcome { context_id, team_id, shared: bool }`, `UnshareContextOutcome { context_id, team_id, unshared: bool }`; `context_service::share(pool, caller: ProfileId, context_id: Uuid, req: &ShareContextRequest) -> ApiResult<ShareContextOutcome>` and `context_service::unshare(pool, caller: ProfileId, context_id: Uuid, team_id: Uuid) -> ApiResult<UnshareContextOutcome>`.

- [ ] **Step 1: Add the wire types.** In `crates/temper-core/src/types/context.rs`, append (mirroring `BindTeamRequest`/`BindTeamOutcome`/`UnbindTeamOutcome` from `cognitive_maps.rs`):

```rust
/// Request body for `POST /api/contexts/{id}/teams` — share a context into a team's read-reach.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareContextRequest {
    /// The team whose members (and DAG descendants) gain read-reach into the context.
    pub team_id: Uuid,
}

/// Result of sharing a context into a team. `shared` is `false` when the share already
/// existed (idempotent no-op).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareContextOutcome {
    pub context_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call inserted the share; `false` when it already existed.
    pub shared: bool,
}

/// Result of unsharing a context from a team. `unshared` is `false` when no share existed.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnshareContextOutcome {
    pub context_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call deleted a share; `false` when none existed.
    pub unshared: bool,
}
```

Confirm `Uuid` and `Serialize`/`Deserialize` are already imported at the top of `context.rs` (they are — `ContextRow` uses them). If `Uuid` is not imported, add `use uuid::Uuid;`.

- [ ] **Step 2: Write the failing service test.** In `context_service.rs`, add to the `#[cfg(test)]` module (create one if absent, mirroring other service tests — use `#[sqlx::test]`):

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn share_is_admin_gated_and_idempotent(pool: PgPool) {
    // Non-admin caller → Forbidden.
    let (admin, non_admin, team_id, context_id) = seed_admin_team_context(&pool).await;
    let req = ShareContextRequest { team_id };
    let denied = share(&pool, non_admin, *context_id, &req).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)));

    // Admin → shares; first call inserts, second is a no-op.
    let first = share(&pool, admin, *context_id, &req).await.unwrap();
    assert!(first.shared);
    let second = share(&pool, admin, *context_id, &req).await.unwrap();
    assert!(!second.shared);

    // Unshare removes it; second unshare is a no-op.
    let u1 = unshare(&pool, admin, *context_id, team_id).await.unwrap();
    assert!(u1.unshared);
    let u2 = unshare(&pool, admin, *context_id, team_id).await.unwrap();
    assert!(!u2.unshared);
}
```

Add a `seed_admin_team_context` helper in the same test module that: inserts two `kb_profiles`; creates a team; makes profile 1 an `owner` of `temper-system` and sets `gating_team_slug='temper-system'` (so `is_system_admin` is true for the admin — mirror `cogmap_authz_test.rs:33-41` for the admin-minting idiom); creates a context owned by profile 1; returns `(admin_profile_id, non_admin_profile_id, team_id, context_id)`. Use runtime `sqlx::query(...)` for the fixture inserts (test-fixture writes must NOT use the compile-time macro — it breaks the `SQLX_OFFLINE` check per project convention).

- [ ] **Step 3: Run it to verify it fails.**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db share_is_admin_gated_and_idempotent`
Expected: FAIL — `share`/`unshare` not defined.

- [ ] **Step 4: Implement `share`/`unshare`.** In `context_service.rs`, add `use crate::services::access_service;` if absent, plus the new types to the `pub use`/imports, then append (mirroring `cogmap_service::bind_team`/`unbind_team`):

```rust
/// Share a context into a team's read-reach (write a `kb_team_contexts` row).
///
/// Auth before writes: admin-only (interim gate, mirroring `cogmap bind` — its
/// structural sibling; a later RBAC arc may relax it). Idempotent —
/// `INSERT … ON CONFLICT DO NOTHING`; `shared: false` when it already existed.
pub async fn share(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    req: &ShareContextRequest,
) -> ApiResult<ShareContextOutcome> {
    if !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::Forbidden);
    }
    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_team_contexts (context_id, team_id)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        RETURNING context_id
        "#,
        context_id,
        req.team_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(ShareContextOutcome { context_id, team_id: req.team_id, shared: inserted.is_some() })
}

/// Unshare a context from a team (delete the `kb_team_contexts` row). Admin-only, no-op safe.
pub async fn unshare(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<UnshareContextOutcome> {
    if !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::Forbidden);
    }
    let result = sqlx::query!(
        "DELETE FROM kb_team_contexts WHERE context_id = $1 AND team_id = $2",
        context_id,
        team_id,
    )
    .execute(pool)
    .await?;
    Ok(UnshareContextOutcome { context_id, team_id, unshared: result.rows_affected() > 0 })
}
```

Add `ShareContextRequest, ShareContextOutcome, UnshareContextOutcome` to the existing `pub use temper_core::types::context::{…}` line.

- [ ] **Step 5: Regenerate the sqlx cache.**

Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services`
Expected: `.sqlx/` updated; no errors.

- [ ] **Step 6: Run the test to verify it passes.**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db share_is_admin_gated_and_idempotent`
Expected: PASS.

- [ ] **Step 7: Regenerate TS types + commit.**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/context.rs crates/temper-services/src/services/context_service.rs crates/temper-services/.sqlx .sqlx packages/temper-ui
git commit -m "feat(context): context-share wire types + service (is_system_admin-gated)"
```

(Commit whatever `.ts` regenerated — ride-along regenerated types.)

---

### Task A2: Context-share API handlers + routes

**Files:**
- Modify: `crates/temper-api/src/handlers/contexts.rs`
- Modify: `crates/temper-api/src/routes.rs:80-84`
- Modify: `crates/temper-api/src/openapi.rs`

**Interfaces:**
- Consumes: `context_service::share`/`unshare`, `ShareContextRequest`/`ShareContextOutcome`/`UnshareContextOutcome`.
- Produces: `POST /api/contexts/{id}/teams`, `DELETE /api/contexts/{id}/teams/{team_id}`.

- [ ] **Step 1: Add the handlers.** In `contexts.rs`, extend the `context_service` import to include the new types, add `use uuid::Uuid;` if absent, and append (mirroring `cognitive_maps::bind_team`/`unbind_team` — the auth gate lives in the service):

```rust
#[utoipa::path(
    post,
    path = "/api/contexts/{id}/teams",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = ShareContextRequest,
    responses(
        (status = 200, description = "Context shared (or idempotent no-op)", body = ShareContextOutcome),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn share_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ShareContextRequest>,
) -> ApiResult<Json<ShareContextOutcome>> {
    let outcome =
        context_service::share(&state.pool, ProfileId::from(auth.0.profile.id), context_id, &body)
            .await?;
    Ok(Json(outcome))
}

#[utoipa::path(
    delete,
    path = "/api/contexts/{id}/teams/{team_id}",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("team_id" = Uuid, Path, description = "Team ID to unshare"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context unshared (or no-op)", body = UnshareContextOutcome),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn unshare_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((context_id, team_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<UnshareContextOutcome>> {
    let outcome =
        context_service::unshare(&state.pool, ProfileId::from(auth.0.profile.id), context_id, team_id)
            .await?;
    Ok(Json(outcome))
}
```

Update the `context_service::{ self, ContextCreateRequest, ContextRow, ContextRowWithCounts }` import to also bring `ShareContextRequest, ShareContextOutcome, UnshareContextOutcome` (they are re-exported from `context_service` via the `pub use`).

- [ ] **Step 2: Register the routes.** In `routes.rs`, in the `gated` router next to the existing context routes (lines 80-84), add:

```rust
        .route(
            "/api/contexts/{id}/teams",
            post(handlers::contexts::share_team),
        )
        .route(
            "/api/contexts/{id}/teams/{team_id}",
            delete(handlers::contexts::unshare_team),
        )
```

Confirm `post`, `delete` are imported (they are: `use axum::routing::{delete, get, post, put};`).

- [ ] **Step 3: Register in OpenAPI.** In `openapi.rs`, add `crate::handlers::contexts::share_team,` and `crate::handlers::contexts::unshare_team,` to the `paths(...)` list, and add the three new schemas to `components(schemas(...))` (mirror how `BindTeamRequest`/`BindTeamOutcome` are registered).

- [ ] **Step 4: Verify it compiles + clippy clean.**

Run: `cargo make check`
Expected: PASS (fmt + clippy + docs + TS all green; the honest `SQLX_OFFLINE` probe).

- [ ] **Step 5: Commit.**

```bash
git add crates/temper-api/src/handlers/contexts.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs
git commit -m "feat(context): share/unshare API handlers + routes"
```

---

### Task A3: Context-share client method

**Files:**
- Modify: `crates/temper-client/src/contexts.rs`

**Interfaces:**
- Consumes: `ShareContextRequest`, `ShareContextOutcome`, `UnshareContextOutcome`.
- Produces: `ContextClient::share_team(&self, context_id: Uuid, body: &ShareContextRequest) -> Result<ShareContextOutcome>`, `ContextClient::unshare_team(&self, context_id: Uuid, team_id: Uuid) -> Result<UnshareContextOutcome>`.

- [ ] **Step 1: Add the methods.** In `contexts.rs`, extend the `temper_core::types::context::{…}` import with the three new types, then add to `impl ContextClient` (mirroring `CognitiveMapClient::bind_team`/`unbind_team`):

```rust
    /// POST /api/contexts/{id}/teams — share the context into a team (admin-gated, idempotent).
    pub async fn share_team(
        &self,
        context_id: Uuid,
        body: &ShareContextRequest,
    ) -> Result<ShareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/contexts/{id}/teams/{team_id} — unshare (admin-gated, no-op safe).
    pub async fn unshare_team(
        &self,
        context_id: Uuid,
        team_id: Uuid,
    ) -> Result<UnshareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
```

- [ ] **Step 2: Verify it compiles.**

Run: `cargo build -p temper-client`
Expected: PASS.

- [ ] **Step 3: Commit.**

```bash
git add crates/temper-client/src/contexts.rs
git commit -m "feat(context): share_team/unshare_team client methods"
```

---

### Task A4: `temper context share`/`unshare` CLI

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`ContextAction`, ~lines 505-525)
- Modify: `crates/temper-cli/src/commands/context_cmd.rs`
- Modify: `crates/temper-cli/src/main.rs` (ContextAction dispatch, ~lines 286-306)

**Interfaces:**
- Consumes: `ContextClient::share_team`/`unshare_team`, `crate::actions::cogmap::resolve_team_id`.
- Produces: `context_cmd::share_remote`, `context_cmd::unshare_remote`, `context_cmd::resolve_context_id`.

- [ ] **Step 1: Add the clap variants.** In `cli.rs`, in `enum ContextAction`, add:

```rust
    /// Share a context into a team's read-reach (admin-only). The context ref is a UUID or the
    /// `@handle/slug` / `+team-slug/slug` form (from `context list`); `@me` shorthand is not accepted.
    Share {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to share into: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Unshare a context from a team (admin-only).
    Unshare {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to unshare: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
```

- [ ] **Step 2: Add the resolver + remote fns.** In `context_cmd.rs`, add imports `use uuid::Uuid;` and `use temper_core::types::context::ShareContextRequest;`, then:

```rust
/// Resolve a context ref (a bare UUID, or the `@handle/slug` / `+team-slug/slug` form that
/// `context list` renders) to its context id. `@me` shorthand is NOT resolved here — an operator
/// sharing a context addresses it by the concrete owner shown in the list (or by UUID).
pub async fn resolve_context_id(client: &temper_client::TemperClient, context: &str) -> Result<Uuid> {
    if let Ok(id) = Uuid::parse_str(context) {
        return Ok(id);
    }
    let (owner, slug) = context.split_once('/').ok_or_else(|| {
        TemperError::BadRequest(format!(
            "invalid context ref {context:?}: use a UUID or `@handle/slug` / `+team-slug/slug`"
        ))
    })?;
    if owner == "@me" {
        return Err(TemperError::BadRequest(
            "`@me` is not accepted for share — use your `@handle/slug` (see `context list`) or the context UUID"
                .to_owned(),
        ));
    }
    let contexts = client
        .contexts()
        .list()
        .await
        .map_err(crate::commands::client_err)?;
    contexts
        .into_iter()
        .find(|c| c.owner_ref == owner && c.slug == slug)
        .map(|c| *c.id)
        .ok_or_else(|| {
            TemperError::Api(format!("context '{context}' not found among the contexts you can see"))
        })
}

/// `temper context share <context_ref> <team>` — share a context into a team (admin-only).
pub async fn share_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id(client, context).await?;
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let outcome = client
        .contexts()
        .share_team(context_id, &ShareContextRequest { team_id })
        .await
        .map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper context unshare <context_ref> <team>` — unshare a context from a team (admin-only).
pub async fn unshare_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id(client, context).await?;
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let outcome = client
        .contexts()
        .unshare_team(context_id, team_id)
        .await
        .map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}
```

Confirm `c.id` is a `ContextId` newtype needing `*c.id` to reach the inner `Uuid` (it is — `ContextRowWithCounts.id: ContextId`). Confirm `crate::actions::cogmap::resolve_team_id` is `pub` (it is — reused by `admin::promote_remote`).

- [ ] **Step 3: Wire the dispatch.** In `main.rs`, in the `Commands::Context { action } => match action { … }` block, add arms (mirror how `ContextAction::Create` routes through `runtime::with_client`):

```rust
            ContextAction::Share { context, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::share_remote(
                            client, &context, &team, output_format,
                        )
                        .await
                    })
                })
            }
            ContextAction::Unshare { context, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::context_cmd::unshare_remote(
                            client, &context, &team, output_format,
                        )
                        .await
                    })
                })
            }
```

- [ ] **Step 4: Verify it compiles + rebuild the bin.**

Run: `cargo build -p temper-cli --bin temper`
Expected: PASS. (Rebuilding the bin is required before any e2e that spawns `temper`.)

- [ ] **Step 5: Commit.**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/context_cmd.rs crates/temper-cli/src/main.rs
git commit -m "feat(context): temper context share/unshare CLI"
```

---

### Task A5: Context-share e2e (access-semantics gate)

**Files:**
- Create: `tests/e2e/tests/context_share_e2e.rs`

**Interfaces:**
- Consumes: the running Axum server + real Postgres via the e2e harness (`tests/e2e/tests/common/mod.rs`).

- [ ] **Step 1: Write the e2e test.** Model it on an existing access-semantics e2e (e.g. `tests/e2e/tests/admin_surface_e2e.rs`, which mints the first admin via the SQL root step). The test must: bootstrap a first admin (owner of `temper-system`, `gating_team_slug='temper-system'`); create a second (non-admin) profile; create a team `T` and a context `C` owned by the admin's profile; assert a non-admin `context share C +T` gets `403`; assert the admin `context share C +T` succeeds and that a resource homed in `C` becomes visible to a member of `T` (drive `resources_visible_to`/a scoped `resource list` as the `T` member); assert `context unshare C +T` reverses it. Use the harness's `temper`-binary driver (the `spawn_blocking` + `Command` pattern in `common/mod.rs`) for the CLI path, or hit the client directly — match whichever the sibling e2e uses.

- [ ] **Step 2: Rebuild the bin, then run.**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e`
Expected: the new test passes; suite green.

- [ ] **Step 3: Prepare e2e sqlx cache if the test uses macro queries + commit.**

```bash
cargo make prepare-e2e   # only if the test added macro queries
git add tests/e2e/tests/context_share_e2e.rs tests/e2e/.sqlx
git commit -m "test(e2e): context-share access semantics (admin-gated, widens read-reach)"
```

---

# LIMB 1 — SAML emitter (Beats B–E)

### Task B1: Emitter deps + keygen core

**Files:**
- Modify: `crates/temper-cli/Cargo.toml`
- Create: `crates/temper-cli/src/saml/mod.rs`
- Modify: `crates/temper-cli/src/lib.rs` (add `pub mod saml;`)

**Interfaces:**
- Produces: `saml::GeneratedKey { pem: String, kid: String }`, `saml::generate_signing_key(kid_override: Option<String>, now_yyyymm: &str) -> Result<GeneratedKey>`, `saml::generate_reconcile_secret() -> String` (base64, ≥32 bytes).

- [ ] **Step 1: Add dependencies.** In `crates/temper-cli/Cargo.toml` `[dependencies]`, add (direct version literals — no workspace table):

```toml
base64 = "0.22"
ed25519-dalek = { version = "2", features = ["pkcs8", "rand_core"] }
pkcs8 = { version = "0.10", features = ["pem"] }
rand = "0.8"
```

- [ ] **Step 2: Write the failing keygen test.** Create `crates/temper-cli/src/saml/mod.rs`:

```rust
//! SAML provisioning emitter — pure core (no I/O). One `SamlProvisionConfig` renders the
//! consistent env bundle + SQL; keygen produces a PKCS#8 Ed25519 PEM the TypeScript AS
//! (`packages/temper-cloud/src/oauth/keys.ts` → jose `importPKCS8`) accepts verbatim.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_emits_pkcs8_pem_and_kid() {
        let k = generate_signing_key(None, "2026-07").unwrap();
        assert!(k.pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(k.pem.trim_end().ends_with("-----END PRIVATE KEY-----"));
        assert_eq!(k.kid, "as-2026-07");
        // Re-parse proves it's a valid PKCS#8 Ed25519 key (what jose will do).
        use pkcs8::DecodePrivateKey;
        ed25519_dalek::SigningKey::from_pkcs8_pem(&k.pem).unwrap();

        let overridden = generate_signing_key(Some("custom-kid".into()), "2026-07").unwrap();
        assert_eq!(overridden.kid, "custom-kid");
    }

    #[test]
    fn reconcile_secret_is_strong_and_unique() {
        let a = generate_reconcile_secret();
        let b = generate_reconcile_secret();
        assert_ne!(a, b);
        // ≥32 raw bytes → ≥43 base64 chars (unpadded) / ≥44 (padded).
        assert!(a.len() >= 43);
    }
}
```

- [ ] **Step 3: Run it to verify it fails.**

Run: `cargo test -p temper-cli saml::tests`
Expected: FAIL — functions/types not defined.

- [ ] **Step 4: Implement keygen.** Prepend to `saml/mod.rs` (above the test module):

```rust
use crate::error::{Result, TemperError};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use pkcs8::EncodePrivateKey;
use rand::RngCore as _;

/// A generated AS signing key: the PKCS#8 PEM plus its published key id.
#[derive(Debug, Clone)]
pub struct GeneratedKey {
    pub pem: String,
    pub kid: String,
}

/// Generate an Ed25519 signing key as a PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`), compatible
/// with the TypeScript AS's `importPKCS8(pem, "EdDSA")`. `kid` defaults to `as-<YYYY-MM>`.
pub fn generate_signing_key(kid_override: Option<String>, now_yyyymm: &str) -> Result<GeneratedKey> {
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);
    let pem = signing
        .to_pkcs8_pem(pkcs8::LineEnding::LF)
        .map_err(|e| TemperError::Config(format!("PKCS#8 encode: {e}")))?
        .to_string();
    let kid = kid_override.unwrap_or_else(|| format!("as-{now_yyyymm}"));
    Ok(GeneratedKey { pem, kid })
}

/// Generate a strong shared reconcile secret: 32 random bytes, base64 (standard, padded).
pub fn generate_reconcile_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
```

Add `pub mod saml;` to `crates/temper-cli/src/lib.rs` (next to the other `pub mod` declarations).

- [ ] **Step 5: Run the test to verify it passes.**

Run: `cargo test -p temper-cli saml::tests`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/temper-cli/Cargo.toml crates/temper-cli/src/saml/mod.rs crates/temper-cli/src/lib.rs Cargo.lock
git commit -m "feat(saml): emitter keygen core (ed25519 PKCS#8 + reconcile secret)"
```

---

### Task B2: `SamlProvisionConfig` + env rendering (consistency-by-construction)

**Files:**
- Modify: `crates/temper-cli/src/saml/mod.rs`

**Interfaces:**
- Consumes: `GeneratedKey`, `generate_reconcile_secret`.
- Produces: `saml::SamlProvisionConfig` (fields below), `SamlProvisionConfig::render_env(&self) -> String`.

- [ ] **Step 1: Write the failing env-consistency test.** Add to the `tests` module:

```rust
    fn sample_config() -> SamlProvisionConfig {
        SamlProvisionConfig {
            instance_url: "https://temper.acme.com".into(),
            api_origin: "https://temper.acme.com".into(),
            idp_key: "acme-okta".into(),
            signing_key_pem: "-----BEGIN PRIVATE KEY-----\nAAA\n-----END PRIVATE KEY-----\n".into(),
            signing_kid: "as-2026-07".into(),
            reconcile_secret: "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0MDE=".into(),
            clients: vec![
                ("temper-cli".into(), vec!["https://temper.acme.com/api/auth/cli-callback".into()]),
                ("temper-ui".into(), vec!["https://app.acme.com/auth/callback".into()]),
            ],
            access_ttl_secs: 900,
            refresh_ttl_secs: 2_592_000,
            idp_cert: "-----BEGIN CERTIFICATE-----\nX\n-----END CERTIFICATE-----".into(),
            idp_sso_url: "https://idp.acme.com/sso".into(),
            idp_entity_id: "http://www.okta.com/x".into(),
            nameid_format: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent".into(),
            email_attr: "email".into(),
            stable_id_attr: "uid".into(),
            groups_attr: Some("groups".into()),
        }
    }

    #[test]
    fn env_shared_values_are_consistent_by_construction() {
        let env = sample_config().render_env();
        let get = |k: &str| env.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap();
        // The whole point: shared values are equal because they are derived from one source.
        assert_eq!(get("AS_ISSUER"), get("AUTH_ISSUER"));
        assert_eq!(get("AS_AUDIENCE"), get("AUTH_AUDIENCE"));
        assert_eq!(get("AS_AUDIENCE"), "https://temper.acme.com/api");
        assert_eq!(get("AUTH_PROVIDER_NAME"), "saml:acme-okta");
        assert_eq!(get("JWKS_URL"), "https://temper.acme.com/oauth/jwks");
        assert_eq!(get("INTERNAL_RECONCILE_URL"), "https://temper.acme.com/internal/saml/reconcile");
        // AS_CLIENTS is valid JSON of the client→redirects map.
        let clients: serde_json::Value = serde_json::from_str(get("AS_CLIENTS")).unwrap();
        assert_eq!(clients["temper-cli"][0], "https://temper.acme.com/api/auth/cli-callback");
    }
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `cargo test -p temper-cli saml::tests::env_shared_values_are_consistent_by_construction`
Expected: FAIL — `SamlProvisionConfig`/`render_env` not defined.

- [ ] **Step 3: Implement the struct + env rendering.** Add to `saml/mod.rs`. Build `AS_CLIENTS` from a typed `BTreeMap<String, Vec<String>>` via `serde_json::to_string` (typed source → JSON, not a hand-built string):

```rust
use std::collections::BTreeMap;

/// The single source of truth for a SAML provisioning run. Every shared value across the two
/// Vercel functions is DERIVED from these fields, so `AS_AUDIENCE == AUTH_AUDIENCE`,
/// `AUTH_ISSUER == AS_ISSUER`, `AUTH_PROVIDER_NAME == saml:<idp_key>`, and the one
/// `INTERNAL_RECONCILE_SECRET` cannot drift.
#[derive(Debug, Clone)]
pub struct SamlProvisionConfig {
    pub instance_url: String,
    pub api_origin: String,
    pub idp_key: String,
    pub signing_key_pem: String,
    pub signing_kid: String,
    pub reconcile_secret: String,
    pub clients: Vec<(String, Vec<String>)>,
    pub access_ttl_secs: u32,
    pub refresh_ttl_secs: u32,
    pub idp_cert: String,
    pub idp_sso_url: String,
    pub idp_entity_id: String,
    pub nameid_format: String,
    pub email_attr: String,
    pub stable_id_attr: String,
    pub groups_attr: Option<String>,
}

impl SamlProvisionConfig {
    fn issuer(&self) -> &str { self.instance_url.trim_end_matches('/') }
    fn audience(&self) -> String { format!("{}/api", self.issuer()) }
    fn sp_entity_id(&self) -> String { format!("{}/saml/metadata", self.issuer()) }
    fn acs_url(&self) -> String { format!("{}/oauth/saml/acs", self.issuer()) }
    fn provider_name(&self) -> String { format!("saml:{}", self.idp_key) }

    fn clients_json(&self) -> String {
        let map: BTreeMap<&str, &Vec<String>> =
            self.clients.iter().map(|(c, r)| (c.as_str(), r)).collect();
        serde_json::to_string(&map).expect("client map serializes")
    }

    /// Render the full env bundle (AS-side + api-side + shared). Emit-only — the operator pastes
    /// these into both Vercel functions (or a .env). Shared values are equal by construction.
    pub fn render_env(&self) -> String {
        let issuer = self.issuer();
        let audience = self.audience();
        format!(
            "# ── Authorization Server (temper-cloud) ──────────────────────────\n\
             AS_ISSUER={issuer}\n\
             AS_AUDIENCE={audience}\n\
             AS_SIGNING_KEY_PKCS8={key}\n\
             AS_SIGNING_KID={kid}\n\
             AS_CLIENTS={clients}\n\
             AS_ACCESS_TTL_SECONDS={access}\n\
             AS_REFRESH_TTL_SECONDS={refresh}\n\
             # ── temper-api ───────────────────────────────────────────────────\n\
             JWKS_URL={issuer}/oauth/jwks\n\
             AUTH_ISSUER={issuer}\n\
             AUTH_AUDIENCE={audience}\n\
             AUTH_PROVIDER_NAME={provider}\n\
             # ── shared (BOTH functions, identical value) ─────────────────────\n\
             INTERNAL_RECONCILE_SECRET={secret}\n\
             INTERNAL_RECONCILE_URL={api}/internal/saml/reconcile\n",
            issuer = issuer,
            audience = audience,
            key = self.signing_key_pem.replace('\n', "\\n"),
            kid = self.signing_kid,
            clients = self.clients_json(),
            access = self.access_ttl_secs,
            refresh = self.refresh_ttl_secs,
            provider = self.provider_name(),
            secret = self.reconcile_secret,
            api = self.api_origin.trim_end_matches('/'),
        )
    }
}
```

Add `use serde_json;` only if not already reachable (temper-cli depends on `serde_json`). Note the PEM's newlines are escaped to `\n` so the multi-line key is a single env value (matching how `AS_SIGNING_KEY_PKCS8` is stored as PEM contents).

- [ ] **Step 4: Run the test to verify it passes.**

Run: `cargo test -p temper-cli saml::tests`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/temper-cli/src/saml/mod.rs
git commit -m "feat(saml): SamlProvisionConfig + consistent env rendering"
```

---

### Task B3: SQL rendering (`kb_saml_idp` + `kb_saml_group_mappings`)

**Files:**
- Modify: `crates/temper-cli/src/saml/mod.rs`

**Interfaces:**
- Produces: `SamlProvisionConfig::render_idp_sql(&self) -> String`; free fn `saml::render_group_mapping_sql(idp_key: &str, group_value: &str, team_id: uuid::Uuid, role: &str) -> String`.

- [ ] **Step 1: Write the failing SQL test.** Add to `tests`:

```rust
    #[test]
    fn idp_sql_has_all_columns_and_escapes_quotes() {
        let mut cfg = sample_config();
        cfg.idp_key = "a'quote".into(); // SQL-escaping must double it.
        let sql = cfg.render_idp_sql();
        assert!(sql.contains("INSERT INTO kb_saml_idp"));
        assert!(sql.contains("is_active"));
        assert!(sql.contains("groups_attr"));
        assert!(sql.contains("'a''quote'"));
        assert!(sql.contains("saml/metadata")); // derived sp_entity_id
    }

    #[test]
    fn group_mapping_sql_renders() {
        let team = uuid::Uuid::nil();
        let sql = render_group_mapping_sql("acme-okta", "engineering", team, "member");
        assert!(sql.contains("INSERT INTO kb_saml_group_mappings"));
        assert!(sql.contains("'engineering'"));
        assert!(sql.contains(&team.to_string()));
        assert!(sql.contains("'member'"));
    }
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `cargo test -p temper-cli saml::tests::idp_sql_has_all_columns_and_escapes_quotes`
Expected: FAIL — `render_idp_sql`/`render_group_mapping_sql` not defined.

- [ ] **Step 3: Implement SQL rendering.** Add to `saml/mod.rs` (a small `sql_str` helper single-quotes and doubles embedded quotes — these are operator-emitted artifacts, not runtime queries):

```rust
/// Single-quote a string literal for emitted SQL, doubling embedded quotes.
fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

impl SamlProvisionConfig {
    /// Render the `kb_saml_idp` INSERT for this IdP (active row). Emit-only unless `--apply`.
    pub fn render_idp_sql(&self) -> String {
        let groups = match &self.groups_attr {
            Some(g) => sql_str(g),
            None => "NULL".to_owned(),
        };
        format!(
            "INSERT INTO kb_saml_idp (\n  \
             idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,\n  \
             sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr\n\
             ) VALUES (\n  \
             {idp_key}, true, {cert}, {sso}, {entity},\n  \
             {sp}, {acs}, {nameid}, {email}, {stable}, {groups}\n);\n",
            idp_key = sql_str(&self.idp_key),
            cert = sql_str(&self.idp_cert),
            sso = sql_str(&self.idp_sso_url),
            entity = sql_str(&self.idp_entity_id),
            sp = sql_str(&self.sp_entity_id()),
            acs = sql_str(&self.acs_url()),
            nameid = sql_str(&self.nameid_format),
            email = sql_str(&self.email_attr),
            stable = sql_str(&self.stable_id_attr),
            groups = groups,
        )
    }
}

/// Render one `kb_saml_group_mappings` INSERT (`group → (team, role)`).
pub fn render_group_mapping_sql(idp_key: &str, group_value: &str, team_id: uuid::Uuid, role: &str) -> String {
    format!(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role)\n\
         VALUES ({idp}, {group}, '{team}', {role})\nON CONFLICT DO NOTHING;\n",
        idp = sql_str(idp_key),
        group = sql_str(group_value),
        team = team_id,
        role = sql_str(role),
    )
}
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cargo test -p temper-cli saml::tests`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/temper-cli/src/saml/mod.rs
git commit -m "feat(saml): kb_saml_idp + group-mappings SQL rendering"
```

---

### Task C1: `temper admin saml provision` command (emit + `--env-out`)

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`AdminAction`, new `AdminSamlAction`)
- Create: `crates/temper-cli/src/commands/admin_saml.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (add `pub mod admin_saml;`)
- Modify: `crates/temper-cli/src/main.rs` (dispatch)

**Interfaces:**
- Consumes: `saml::{SamlProvisionConfig, generate_signing_key, generate_reconcile_secret}`.
- Produces: `admin_saml::provision(args…)`.

- [ ] **Step 1: Add the clap surface.** In `cli.rs`, add a `Saml` arm to `AdminAction`, and a new `AdminSamlAction` enum with the `Provision` variant (add `MapGroup`/`Verify` in Tasks D/E). The interactive-vs-switched pattern mirrors `init.rs` (`--no-interactive` + per-field flags):

```rust
    /// SAML provisioning: generate keys + emit the consistent env bundle and SQL (operator tooling).
    Saml {
        #[command(subcommand)]
        action: AdminSamlAction,
    },
```

```rust
#[derive(Subcommand)]
pub enum AdminSamlAction {
    /// Generate the AS signing key + reconcile secret and emit the env bundle + kb_saml_idp SQL.
    ///
    /// Interactive by default; pass --no-interactive with the flags below for scripted runs.
    /// Emits to stdout unless --env-out / --sql-out are given; --apply runs the SQL via psql.
    Provision {
        #[arg(long)]
        no_interactive: bool,
        #[arg(long)]
        instance_url: Option<String>,
        /// API origin the AS calls for reconcile (defaults to --instance-url).
        #[arg(long)]
        api_origin: Option<String>,
        #[arg(long)]
        idp_key: Option<String>,
        /// Path to the IdP signing certificate (PEM).
        #[arg(long)]
        idp_cert_file: Option<String>,
        #[arg(long)]
        idp_sso_url: Option<String>,
        #[arg(long)]
        idp_entity_id: Option<String>,
        #[arg(long, default_value = "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent")]
        nameid_format: String,
        #[arg(long, default_value = "email")]
        email_attr: String,
        #[arg(long, default_value = "uid")]
        stable_id_attr: String,
        /// Assertion attribute carrying the group list (omit for authn-only).
        #[arg(long)]
        groups_attr: Option<String>,
        /// Override the signing key id (default `as-<YYYY-MM>`).
        #[arg(long)]
        kid: Option<String>,
        /// Repeatable `client_id=redirect_uri` for AS_CLIENTS (e.g. temper-cli=https://…/cli-callback).
        #[arg(long = "client")]
        clients: Vec<String>,
        /// Write the env bundle here instead of stdout (chmod 0600 — contains the private key).
        #[arg(long)]
        env_out: Option<String>,
        /// Write the SQL here instead of stdout.
        #[arg(long)]
        sql_out: Option<String>,
        /// Run the kb_saml_idp SQL against $DATABASE_URL via psql (default: emit only).
        #[arg(long)]
        apply: bool,
    },
}
```

- [ ] **Step 2: Implement the command.** Create `crates/temper-cli/src/commands/admin_saml.rs`. Gather via prompts when interactive (mirror `init::gather_answers` — `Input`/`Select`/`Confirm`, `.map_err(prompt_err)`); when `--no-interactive`, require the flags (mirror `init::self_host_from_flags` error style). Build `SamlProvisionConfig`, then emit. For `now_yyyymm`, format the current month from `chrono::Local::now()` (already a dep). Parse `--client c=uri` into the `clients` vec (error on a missing `=`). For `--apply`, delegate to a shared `apply_sql_via_psql` helper (Task C2). Skeleton:

```rust
//! `temper admin saml` command shell — I/O around the pure `crate::saml` core. Emit-by-default.

use crate::error::{Result, TemperError};
use crate::saml::{self, SamlProvisionConfig};

#[allow(clippy::too_many_arguments)] // interactive+switched surface; grouped in a follow-up if it grows
pub fn provision(/* the clap fields, by value */) -> Result<()> {
    // 1. Resolve every field: prompt when interactive, else require the flag.
    // 2. Read --idp-cert-file into a String.
    // 3. let key = saml::generate_signing_key(kid, &current_yyyymm())?;
    //    let secret = saml::generate_reconcile_secret();
    // 4. Build SamlProvisionConfig { … } from the resolved values.
    // 5. let env = cfg.render_env(); let sql = cfg.render_idp_sql();
    // 6. Emit env (stdout or --env-out, chmod 0600) and sql (stdout or --sql-out).
    // 7. If --apply: crate::commands::admin_saml::apply_sql_via_psql(&sql)?;
    //    else print a hint: "paste the env into BOTH Vercel functions, then apply the SQL."
    todo!("filled in below")
}
```

Fill each numbered step concretely — prompts for the interactive branch, flag-required errors for `--no-interactive`, `std::fs::write` for `--env-out`/`--sql-out`, and on unix set mode 0600 on the env file (`std::os::unix::fs::PermissionsExt`). Use `output::hint`/`output::warning` for guidance and `println!` for the emitted artifacts (so stdout stays pipeable).

- [ ] **Step 3: Wire dispatch.** In `main.rs`, add the `AdminAction::Saml { action } => match action { AdminSamlAction::Provision { … } => temper_cli::commands::admin_saml::provision(…) }` arm. `provision` needs no client (pure emit) — call it directly, not through `with_client`.

- [ ] **Step 4: Verify + manual smoke.**

Run:
```bash
cargo build -p temper-cli --bin temper
./target/debug/temper admin saml provision --no-interactive \
  --instance-url https://temper.example.com --idp-key demo \
  --idp-cert-file /dev/stdin --idp-sso-url https://idp/sso --idp-entity-id http://idp \
  --client temper-cli=https://temper.example.com/api/auth/cli-callback <<<'-----BEGIN CERTIFICATE-----
X
-----END CERTIFICATE-----'
```
Expected: prints an env bundle with matching `AS_AUDIENCE`/`AUTH_AUDIENCE` and a `kb_saml_idp` INSERT.

- [ ] **Step 5: `cargo make check` + commit.**

```bash
cargo make check
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/admin_saml.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/main.rs
git commit -m "feat(saml): temper admin saml provision (emit env + idp SQL)"
```

---

### Task C2: `--apply` via `psql` subprocess

**Files:**
- Modify: `crates/temper-cli/src/commands/admin_saml.rs`

**Interfaces:**
- Produces: `admin_saml::apply_sql_via_psql(sql: &str) -> Result<()>` (used by `provision --apply` and, later, `map-group --apply`).

- [ ] **Step 1: Implement the helper.** Mirror `commands::config::open_in_editor`'s `std::process::Command` + error-mapping idiom, but feed SQL via stdin and require `DATABASE_URL`:

```rust
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Run emitted SQL against `$DATABASE_URL` via `psql` (fail-fast on errors). Requires `psql` on
/// PATH and `DATABASE_URL` set — the same operator-with-DB-credentials contract as
/// `scripts/bootstrap/system-bootstrap.sh --run-root`.
pub fn apply_sql_via_psql(sql: &str) -> Result<()> {
    let db = std::env::var("DATABASE_URL").map_err(|_| {
        TemperError::Config("--apply needs DATABASE_URL (the direct/unpooled Neon URL)".into())
    })?;
    let mut child = Command::new("psql")
        .arg(&db)
        .arg("--set=ON_ERROR_STOP=1")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| TemperError::Config(format!("failed to launch psql (is it installed?): {e}")))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(sql.as_bytes())
        .map_err(|e| TemperError::Config(format!("writing SQL to psql: {e}")))?;
    let status = child
        .wait()
        .map_err(|e| TemperError::Config(format!("waiting on psql: {e}")))?;
    if !status.success() {
        return Err(TemperError::Config(format!("psql exited with {status}")));
    }
    Ok(())
}
```

- [ ] **Step 2: Add a doc/behavior test.** Add a unit test asserting `apply_sql_via_psql` errors cleanly when `DATABASE_URL` is unset:

```rust
#[test]
fn apply_requires_database_url() {
    // SAFETY: single-threaded test; no other test reads DATABASE_URL concurrently here.
    let saved = std::env::var("DATABASE_URL").ok();
    std::env::remove_var("DATABASE_URL");
    let err = apply_sql_via_psql("SELECT 1;").unwrap_err();
    assert!(format!("{err}").contains("DATABASE_URL"));
    if let Some(v) = saved { std::env::set_var("DATABASE_URL", v); }
}
```

(Guard against the other tests' env by running this test file single-threaded if needed, or gate it behind `#[ignore]` + a manual run — note it in the step.)

- [ ] **Step 3: Verify + commit.**

Run: `cargo test -p temper-cli admin_saml && cargo make check`

```bash
git add crates/temper-cli/src/commands/admin_saml.rs
git commit -m "feat(saml): --apply runs emitted SQL via psql (operator DB-cred path)"
```

---

### Task D1: `temper admin saml map-group` (+ `--from-seen`)

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`AdminSamlAction::MapGroup`)
- Modify: `crates/temper-cli/src/commands/admin_saml.rs`
- Modify: `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: `saml::render_group_mapping_sql`, `crate::actions::cogmap::resolve_team_id`, `apply_sql_via_psql`.
- Produces: `admin_saml::map_group(…)`, `admin_saml::from_seen(idp_key)`.

- [ ] **Step 1: Add the clap variant.**

```rust
    /// Emit a kb_saml_group_mappings INSERT for `group → (+team, role)` (run AFTER teams exist).
    MapGroup {
        #[arg(long)]
        idp_key: String,
        /// The IdP-asserted group value.
        group: String,
        /// Team to map into: a slug (optionally `+`-prefixed) or a UUID.
        team: String,
        #[arg(long, default_value = "member")]
        role: String,
        /// Instead of emitting a mapping, list groups the IdP has actually asserted
        /// (reads kb_saml_seen_groups via psql; needs DATABASE_URL).
        #[arg(long)]
        from_seen: bool,
        /// Run the INSERT against $DATABASE_URL via psql (default: emit only).
        #[arg(long)]
        apply: bool,
    },
```

- [ ] **Step 2: Implement.** `map_group` resolves the team via the client (`resolve_team_id`), renders the SQL via `saml::render_group_mapping_sql`, then emits or `--apply`s. When `--from-seen`, instead run a `psql -tA -c` SELECT and print the rows. `map_group` needs the authenticated client for team resolution, so route it through `with_client`; `from_seen` is psql-only (no client). Add to `admin_saml.rs`:

```rust
pub async fn map_group(
    client: &temper_client::TemperClient,
    idp_key: &str,
    group: &str,
    team: &str,
    role: &str,
    apply: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let sql = saml::render_group_mapping_sql(idp_key, group, team_id, role);
    if apply {
        apply_sql_via_psql(&sql)?;
        crate::output::success(format!("mapped '{group}' → {team} ({role})"));
    } else {
        println!("{sql}");
    }
    let _ = fmt;
    Ok(())
}

/// List groups the IdP has actually asserted (kb_saml_seen_groups), most-recent first.
pub fn from_seen(idp_key: &str) -> Result<()> {
    let db = std::env::var("DATABASE_URL")
        .map_err(|_| TemperError::Config("--from-seen needs DATABASE_URL".into()))?;
    let out = std::process::Command::new("psql")
        .arg(&db)
        .arg("-tA")
        .arg("-c")
        .arg(format!(
            "SELECT group_value, last_seen FROM kb_saml_seen_groups \
             WHERE idp_key = '{}' ORDER BY last_seen DESC",
            idp_key.replace('\'', "''")
        ))
        .output()
        .map_err(|e| TemperError::Config(format!("failed to launch psql: {e}")))?;
    if !out.status.success() {
        return Err(TemperError::Config(format!(
            "psql failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}
```

- [ ] **Step 3: Dispatch.** In `main.rs`, route `AdminSamlAction::MapGroup { from_seen: true, idp_key, .. }` → `admin_saml::from_seen(&idp_key)` (no client); else through `with_client` to `admin_saml::map_group(…)`.

- [ ] **Step 4: Verify + commit.**

Run: `cargo build -p temper-cli --bin temper && cargo make check`

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/admin_saml.rs crates/temper-cli/src/main.rs
git commit -m "feat(saml): temper admin saml map-group (+ --from-seen discovery)"
```

---

### Task E1: `temper admin saml verify`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`AdminSamlAction::Verify`)
- Modify: `crates/temper-cli/src/commands/admin_saml.rs`
- Modify: `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: `client.admin().get_settings()` (proves `is_system_admin`), `reqwest` for metadata probes, `psql` for the `--db` idp-row check.
- Produces: `admin_saml::verify(client, instance_url, db_check)`.

- [ ] **Step 1: Add the clap variant.**

```rust
    /// Verify a provisioned instance: AS metadata reachable, caller is a system admin
    /// (the gating_team_slug silent-403 check), and — with --db — one active kb_saml_idp row.
    Verify {
        /// Instance base URL to probe (e.g. https://temper.acme.com).
        #[arg(long)]
        instance_url: String,
        /// Also check kb_saml_idp via psql (needs DATABASE_URL).
        #[arg(long)]
        db: bool,
    },
```

- [ ] **Step 2: Implement.** Run each probe, print a pass/fail line per check with remediation, and return an error if any hard check fails. The admin check calls `client.admin().get_settings()` — success proves `is_system_admin` for the caller (this is the exact `gating_team_slug=''` → silent-403 gap from the T6 deploy). The metadata probe GETs `{instance_url}/.well-known/oauth-authorization-server` and `{instance_url}/oauth/jwks` (200 ⇒ AS mode on). The `--db` probe runs `psql -tA -c "SELECT count(*) FROM kb_saml_idp WHERE is_active"` and asserts exactly 1.

```rust
pub async fn verify(
    client: &temper_client::TemperClient,
    instance_url: &str,
    db_check: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let base = instance_url.trim_end_matches('/');
    let http = reqwest::Client::new();
    let mut ok = true;

    // 1. AS metadata + JWKS reachable ⇒ AS mode on.
    for path in ["/.well-known/oauth-authorization-server", "/oauth/jwks"] {
        let url = format!("{base}{path}");
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => crate::output::success(format!("AS reachable: {path}")),
            Ok(r) => { ok = false; crate::output::error(format!("{path} → HTTP {}", r.status())); }
            Err(e) => { ok = false; crate::output::error(format!("{path} unreachable: {e}")); }
        }
    }

    // 2. Caller is a system admin (the gating_team_slug silent-403 check).
    match client.admin().get_settings().await {
        Ok(_) => crate::output::success("caller is a system admin (is_system_admin = true)"),
        Err(e) => {
            ok = false;
            crate::output::error(format!(
                "admin check failed ({e}) — verify gating_team_slug is set AND you own that team"
            ));
        }
    }

    // 3. Optional: exactly one active kb_saml_idp row.
    if db_check {
        let db = std::env::var("DATABASE_URL")
            .map_err(|_| TemperError::Config("--db needs DATABASE_URL".into()))?;
        let out = std::process::Command::new("psql")
            .arg(&db).arg("-tA").arg("-c")
            .arg("SELECT count(*) FROM kb_saml_idp WHERE is_active")
            .output()
            .map_err(|e| TemperError::Config(format!("failed to launch psql: {e}")))?;
        let count = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if count == "1" { crate::output::success("exactly one active kb_saml_idp row"); }
        else { ok = false; crate::output::error(format!("expected 1 active kb_saml_idp row, found {count}")); }
    }

    let _ = fmt;
    if ok { Ok(()) } else { Err(TemperError::Api("one or more SAML checks failed".into())) }
}
```

Confirm `reqwest` is a `temper-cli` dep (it is) and that `reqwest::Client::new()` is available with the `rustls-tls` feature set (it is).

- [ ] **Step 3: Dispatch through `with_client`.** In `main.rs`, `AdminSamlAction::Verify { instance_url, db }` → `with_client(|client| … admin_saml::verify(client, &instance_url, db, output_format))`.

- [ ] **Step 4: Verify + commit.**

Run: `cargo build -p temper-cli --bin temper && cargo make check`

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/admin_saml.rs crates/temper-cli/src/main.rs
git commit -m "feat(saml): temper admin saml verify (AS + admin + idp-row probes)"
```

---

### Task F1: Docs — happy path + interleave note

**Files:**
- Modify: `docs/guides/self-hosting-saml.md`
- Modify: `docs/guides/org-bootstrap.md`

- [ ] **Step 1: Rewrite the SAML guide's happy path.** In `self-hosting-saml.md`, add a top-of-section note that `temper admin saml provision` generates the key + secret and emits the consistent env bundle + `kb_saml_idp` SQL in one step; that `map-group` emits `kb_saml_group_mappings` (run after teams exist); and that `verify` confirms the setup. Keep the existing manual SQL/env tables as the labeled **fallback/reference**. Show the exact commands (both interactive and `--no-interactive`).

- [ ] **Step 2: Add the interleave note to `org-bootstrap.md`.** Document that SAML brackets the org-bootstrap: `admin saml provision` + set env + apply idp SQL **before** first login → first admin logs in (JIT profile) → run the existing bootstrap (SQL root → `team create` …) → `admin saml map-group` **after** teams exist → `admin saml verify`. (Mirror the ordering in the spec §2.4.)

- [ ] **Step 3: Markdownlint + commit.**

Run: `cargo make check` (runs TS/biome; for docs, ensure no broken links — spot-check).

```bash
git add docs/guides/self-hosting-saml.md docs/guides/org-bootstrap.md
git commit -m "docs(saml): temper admin saml as the happy path + org-bootstrap interleave"
```

---

## Self-Review

**1. Spec coverage:**
- Limb 1 `provision` (keygen, consistent env, idp SQL, emit/apply/env-out) → Tasks B1, B2, B3, C1, C2. ✓
- Limb 1 `map-group` (+ `--from-seen` DB) → Task D1. ✓
- Limb 1 `verify` (AS probe + is_system_admin gap + idp row) → Task E1. ✓
- Limb 2 `context share`/`unshare` (is_system_admin-gated, kb_team_contexts) → Tasks A1–A5. ✓
- Consistency-by-construction (tested) → B2 test `env_shared_values_are_consistent_by_construction`. ✓
- Rust-native keygen compatible with TS AS → B1 test re-parses PKCS#8; the cross-runtime contract risk is documented (manual verify against `jose` noted in spec §7). ✓
- Docs happy path + interleave → F1. ✓
- Deferred (multi-IdP `idp_key` on kb_team_members; RBAC arc; auto-set Vercel env) → not implemented, by design. ✓

**2. Placeholder scan:** The only `todo!()` is the C1 skeleton, which is immediately followed by the concrete numbered fill-in instructions and a manual smoke test — the implementer writes the body from the numbered spec + the `init.rs` prompt idiom. All other steps carry complete code. (An implementer must expand C1's numbered comments into real prompt/flag code; flagged explicitly.)

**3. Type consistency:** `ShareContextRequest { team_id }`, `ShareContextOutcome { context_id, team_id, shared }`, `UnshareContextOutcome { context_id, team_id, unshared }` are defined once (A1) and used identically in A2/A3/A4. `SamlProvisionConfig` fields defined in B2 are consumed unchanged in B3/C1. `apply_sql_via_psql` (C2) is reused in D1. `resolve_team_id`/`resolve_context_id` signatures match their call sites.

## Execution Handoff

Two known softenings for the executor: **(1)** Task C1's `provision` body is specified as numbered steps + a skeleton rather than full verbatim code — it's the one genuinely-branchy interactive/switched surface; expand it against the verbatim `init.rs` idiom captured in the spec/research. **(2)** `--apply`/`--from-seen`/`verify --db` and the C2 env test exercise `psql`/live state — they get manual/smoke verification, not unit coverage; the pure core (B1–B3) and the context-share limb (A) are fully TDD-covered.
