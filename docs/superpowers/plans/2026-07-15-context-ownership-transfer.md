# Context Ownership Transfer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire an event-sourced, authz-gated **context ownership transfer** — bind a personal context to a team by mutating `kb_contexts.(owner_table, owner_id)` in place — so a team can author a context it owns. Read-sharing stays `share_context`; write requires team ownership (the governing stance). Spec: [2026-07-15-context-ownership-transfer-design.md](../specs/2026-07-15-context-ownership-transfer-design.md).

**Architecture:** A new `context_reassigned` event mirrors the `resource_reassigned` precedent (payload → `SeedAction` → `fire` → paired SQL projector/mutation fns → `replay` dispatch). A service-direct `context_service::reassign` owns the two-sided authorization (identical in shape to the existing `can_share` gate — reused verbatim) and calls a new `writes::reassign_context_*` layer that fires the event. Thin API handler, a client method, a CLI verb, and an MCP tool round out the surface. No `Backend` trait change. Because `kb_contexts` is an `INPUT_TABLE` in replay (restored verbatim, not projected), the event's projector is an idempotent re-apply on replay — safe, and pinned by a replay-roundtrip test.

**Tech Stack:** Rust (axum, sqlx compile-time macros, ts-rs, rmcp), PostgreSQL 17/18 + pgvector, cargo-make + cargo-nextest.

## Global Constraints

- **Event-sourced owner change.** Ownership transfer is written to the ledger via `context_reassigned` (append + project), never a bare `UPDATE` in the service. The projector performs the `UPDATE kb_contexts`. (`kb_contexts` is an `INPUT_TABLE` — `replay.rs:80` — so on replay it is restored verbatim and the projector re-applies idempotently; this is safe, unlike a projection table, and is pinned by a roundtrip test.)
- **Auth before writes.** The two-sided gate runs before any mutation — reuse `context_service::can_share` verbatim (the target team becomes the new owner, same shape as a share recipient).
- **Slug uniqueness across the owner boundary.** `kb_contexts` has `UNIQUE (owner_table, owner_id, slug)`. On transfer the slug must be unique under the *new* owner — a service pre-check returns `Conflict` (409); the constraint is the backstop. Never silently re-slug.
- **Typed structs over inline JSON.** No `serde_json::json!()` for wire data — define structs in `temper-core`.
- **sqlx macros.** Production + substantive test queries use `sqlx::query!`/`query_as!`/`query_scalar!`. After any SQL/schema change regenerate caches (steps included).
- **Additive-only on `main`.** The migration adds two functions + one `kb_event_types` row. No `DROP`, no table change.
- **New event names must be seeded.** `_event_append` raises unless a `kb_event_types` row exists. `context_reassigned` gets a `NULL` `payload_schema` row (like `resource_reassigned`) so it stays out of the published-schema `TYPED_EVENT_NAMES` invariant.
- **Correlation-threaded SQL signature.** New mutation fns carry the full 5-param act-context signature `(p_payload, p_emitter, p_metadata DEFAULT '{}', p_invocation DEFAULT NULL, p_correlation DEFAULT NULL)` and forward `p_metadata`/`p_invocation`/`p_correlation` into `_event_append` — matching the post-`20260709000050` shape of every mutation fn (the `fire` arm passes all five).
- **OpenAPI is a three-artifact contract.** New response DTOs restale `openapi.json` + the temper-rb gem + temper-ts `schema.ts`. Run `cargo make openapi` and stage all three; the drift gates compare against git (gem regen needs Docker; ts needs only Node and never skips).
- **Scenario-schema derive (no snapshot churn).** The new `ContextReassigned` payload carries the `scenario-schema` `JsonSchema` derive for parity with `ResourceReassigned` — but `tests/scenario_schema.rs` snapshots only `Scenario`/`Seed`/`AccessScenario`, which don't reference the reassign payloads, so **no snapshot regen occurs** (confirmed: `ResourceReassigned` isn't in those snapshots either). Keep the derive for parity; don't chase a phantom diff.
- **Env for tests:** `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`; `cargo make docker-up` before db tests. After merging any temper-cli change, the local e2e uses the prebuilt bin — rebuild it before e2e.
- **Commit message trailer:** end every commit body with the repo's standard co-author + session trailers.

---

## File Structure

**Created:**
- `migrations/20260715000010_context_reassign_fns.sql` — event-type seed + `_project_context_reassigned` + `context_reassign` SQL fns.
- `crates/temper-core/src/types/` — new wire types (in `context.rs`, beside the existing share types).
- `tests/e2e/tests/context_transfer_test.rs` — CLI→API→DB owner change.

**Modified:**
- `crates/temper-substrate/src/payloads.rs` — `ContextReassigned` payload.
- `crates/temper-substrate/src/events.rs` — `EventKind` + `SeedAction` + `Fired::Context` + `event_type()` + `fire` arm + string mappings.
- `crates/temper-substrate/src/replay.rs` — non-content classification + projection dispatch.
- `crates/temper-substrate/src/writes.rs` — `reassign_context_*` fns.
- `crates/temper-substrate/src/ids.rs` (if `ContextId` absent there) — add or reuse.
- `crates/temper-services/src/services/context_service.rs` — `reassign` + slug-collision helper + tests.
- `crates/temper-api/src/handlers/contexts.rs` — thin `reassign` handler (or a new `handlers/context_reassign.rs`).
- `crates/temper-api/src/routes.rs`, `openapi.rs` — route + schema registration.
- `crates/temper-client/src/contexts.rs` — client method.
- `crates/temper-cli/src/cli.rs` (the `ContextAction` enum, ~line 780), `commands/context_cmd.rs` — `context transfer` verb.
- `crates/temper-mcp/src/tools/contexts.rs` **and** `crates/temper-mcp/src/service.rs` — `transfer_context` tool body + its `#[tool]` registration (the registration lives in `service.rs`, ~626).
- `openapi.json`, `clients/temper-rb/lib/temper/generated/*`, `clients/temper-ts/src/generated/schema.ts` — regenerated.

---

## Task 1: Substrate `context_reassigned` event + writes layer

**Files:**
- Create: `migrations/20260715000010_context_reassign_fns.sql`
- Modify: `crates/temper-substrate/src/payloads.rs` (after `ResourceReassigned`, ~line 618)
- Modify: `crates/temper-substrate/src/events.rs` (`EventKind` ~44/86/119; `SeedAction` ~329; `event_type()` ~384; `Fired` ~396; `fire` arm ~1026)
- Modify: `crates/temper-substrate/src/replay.rs` (~179 classification, ~415 dispatch)
- Modify: `crates/temper-substrate/src/writes.rs` (after `reassign_resource_in_tx`, ~line 580)

**Interfaces produced:** `payloads::ContextReassigned`; `SeedAction::ContextReassign`; `Fired::Context(ContextId)`; `writes::reassign_context_with(pool, context, from_owner, to_owner, emitter, ctx) -> Result<()>`; SQL `context_reassign(jsonb, uuid, jsonb, uuid, uuid) -> uuid`.

- [ ] **Step 1: Write the migration**

Create `migrations/20260715000010_context_reassign_fns.sql`:

```sql
-- Context ownership transfer: event-sourced owner change on kb_contexts.
-- Mirrors resource_reassign; this moves (owner_table, owner_id) in place.
-- Additive only: one event-type row + two functions. No table changes.
--
-- kb_contexts is a replay INPUT table (restored verbatim), not a projection, so this
-- projector is an idempotent re-apply on replay — see the plan's replay-roundtrip test.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps it
-- out of the published-schema TYPED_EVENT_NAMES invariant (as resource_reassigned).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('context_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half: set the context's owner to (to_owner_table, to_owner_id).
-- The UNIQUE(owner_table, owner_id, slug) constraint is the backstop for a slug
-- collision under the new owner (the service pre-checks and returns 409 first).
CREATE FUNCTION _project_context_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET owner_table = p_payload->>'to_owner_table',
           owner_id    = (p_payload->>'to_owner_id')::uuid
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_reassign: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- Mutation half: append the event anchored to the context itself, then project.
-- Full 5-param act-context signature (matches every mutation fn post-20260709000050).
CREATE FUNCTION context_reassign(p_payload jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb,
                                 p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_contexts WHERE id = v_context) THEN
        RAISE EXCEPTION 'context_reassign: context % not found', v_context;
    END IF;
    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
END;
$$;
```

> Confirm `_event_append`'s named-param signature (`p_metadata`/`p_invocation`/`p_correlation`) against `migrations/20260709000050_act_correlation_passthrough.sql:298-300` — the `resource_reassign` call there is the exact template. Confirm `kb_contexts` is an accepted `anchor_table` for `_event_append` (it is — resource homes anchor events to `'kb_contexts'`; and context-telos events already anchor to `('kb_contexts', …)`).

- [ ] **Step 2: Add the payload struct**

In `payloads.rs`, after `ResourceReassigned`:

```rust
/// Reassign a context's owner — set `kb_contexts.(owner_table, owner_id)` to the target.
/// Owner is polymorphic (`kb_profiles` | `kb_teams`), so both ends carry table + id.
/// `from_owner_*` is recorded for the audit trail; the projector writes only the new owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextReassigned {
    pub context_id: ContextId,
    pub from_owner_table: String,
    pub from_owner_id: Uuid,
    pub to_owner_table: String,
    pub to_owner_id: Uuid,
}
```

> `ContextId` is **already present** (`temper-core/src/types/ids.rs:111`, re-exported by substrate `ids.rs:13`, `From<Uuid>` via `define_id!`) and already imported in `payloads.rs:18`; `Uuid` is in scope at `payloads.rs:24`. No new newtype needed.

- [ ] **Step 3: EventKind variant + string mappings**

In `events.rs`: add `ContextReassigned,` to the `EventKind` enum (near `ResourceReassigned`, ~44); add `EventKind::ContextReassigned => "context_reassigned",` to `as_canonical_name` (~86); add `"context_reassigned" => EventKind::ContextReassigned,` to `from_canonical_name` (~119).

- [ ] **Step 4: SeedAction variant + event_type + Fired**

In `events.rs`, after `SeedAction::ResourceReassign` (~334):

```rust
    ContextReassign {
        context: ContextId,
        from_owner_table: &'a str,
        from_owner_id: Uuid,
        to_owner_table: &'a str,
        to_owner_id: Uuid,
        emitter: EntityId,
    },
```

Add to `event_type()` (~384): `SeedAction::ContextReassign { .. } => EventKind::ContextReassigned,`. Add a `Fired::Context(ContextId)` variant (~401, beside `Resource(ResourceId)`).

> Confirm `SeedAction`'s lifetime `'a` covers the `&'a str` owner-table fields (it already carries `&'a str` fields, e.g. `InvocationOpen::trigger_kind`). Confirm adding a `Fired` variant doesn't break an exhaustive `match` on `Fired` elsewhere (`grep -rn "match .*Fired\|Fired::" crates/temper-substrate/src`); the accessor methods are per-variant and unaffected. Confirm `ContextId`/`Uuid` are in scope in `events.rs`.

- [ ] **Step 5: The fire arm**

In `events.rs`, after the `SeedAction::ResourceReassign` fire arm (~1049):

```rust
        SeedAction::ContextReassign {
            context,
            from_owner_table,
            from_owner_id,
            to_owner_table,
            to_owner_id,
            emitter,
        } => {
            let payload = payloads::ContextReassigned {
                context_id: context,
                from_owner_table: from_owner_table.to_string(),
                from_owner_id,
                to_owner_table: to_owner_table.to_string(),
                to_owner_id,
            };
            let id = sqlx::query_scalar!(
                "SELECT context_reassign($1,$2,$3,$4,$5)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
                ctx_corr,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("context_reassign returned null")?;
            Ok(Fired::Context(ContextId::from(id)))
        }
```

- [ ] **Step 6: Replay classification + projection dispatch** — **two** mandatory edits (both matches are exhaustive, no wildcard, so both must compile):

**(a)** Add `| EventKind::ContextReassigned` to the no-manifest `=> None` arm in `snapshot()` (`replay.rs:163-181`, beside `ResourceReassigned`) — the payload carries no chunk manifest. This *is* the "non-content classification" arm; it is **one** edit, not two (an earlier draft double-counted it).

**(b)** Add the projection-dispatch arm (~415):

```rust
            EventKind::ContextReassigned => {
                sqlx::query("SELECT _project_context_reassigned($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
```

- [ ] **Step 7: The writes-layer fn**

In `writes.rs`, after `reassign_resource_in_tx` (~580):

```rust
/// Reassign a context's owner (event-sourced, in-place) under an explicit EventContext.
pub async fn reassign_context_with(
    pool: &PgPool,
    context: ContextId,
    from_owner: (&str, Uuid),
    to_owner: (&str, Uuid),
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::ContextReassign {
            context,
            from_owner_table: from_owner.0,
            from_owner_id: from_owner.1,
            to_owner_table: to_owner.0,
            to_owner_id: to_owner.1,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
```

> Confirm `ContextId` import in `writes.rs`. No `_in_tx`/bulk variant needed for v1 (single context per call).

- [ ] **Step 8: Replay-roundtrip test**

Add a test to the substrate replay suite (grep `crates/temper-substrate/tests/` for the existing snapshot→reset→replay roundtrip harness — e.g. a `*replay*` or `*ledger*` test) that: seeds a profile + a team + a personal context, fires `reassign_context_with` to the team, snapshots, resets the namespace, replays, and asserts `kb_contexts.owner_table='kb_teams'` / `owner_id=team` post-replay (idempotent convergence). If the roundtrip harness is `artifact-tests`-gated, place it there and note `cargo make test-artifacts`.

> If no reusable roundtrip harness exists at unit grain, assert the narrower invariant directly: after `reassign_context_with`, `SELECT _project_context_reassigned(ev,payload)` a second time is a no-op (owner unchanged) — proving projector idempotency, the property replay relies on.

- [ ] **Step 9: Regenerate caches + verify compile**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up
cargo sqlx migrate run
cargo sqlx prepare --workspace -- --all-features
cargo make check
```

(No `scenario_schema.rs` snapshot change is expected — see the Global Constraint. If `check` unexpectedly reds there, treat it as a real signal, not a routine regen.)

- [ ] **Step 10: Commit** (`migrations/ crates/temper-substrate/ .sqlx/`).

---

## Task 2: `context_service::reassign` — two-sided authz + slug guard

**Files:**
- Modify: `crates/temper-services/src/services/context_service.rs` (add `reassign` + a slug-collision helper + tests; reuse `can_share`, `caller_administers_context`, `ensure_context_and_team_exist`)

**Interfaces produced:** `context_service::reassign(pool, caller: ProfileId, context_id: Uuid, to_team_id: Uuid) -> ApiResult<ReassignContextOutcome>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `context_service.rs` (reuse its seed helpers; the module already seeds profiles/teams/contexts + mints a system-admin — see `context_service.rs:496+`).

> **Target team must be non-gating.** The existing `seed_admin_team_context` helper builds the **`temper-system` gating team** and makes the caller its owner (so `is_system_admin` resolves true). But `can_share` **refuses a gating-team target** (`is_gating_team` ⇒ `Ok(false)`, `context_service.rs:378`). So every success-path test must seed a **separate, non-gating team** as the transfer target and add the caller as owner/maintainer of *it*. Only the "target is the gating/root team ⇒ Forbidden" matrix case reuses `temper-system` as the target.

Cover:

- `personal_context_transfers_to_team` — after `reassign`, the context's `owner_table='kb_teams'`/`owner_id=team`; `reassigned == true`.
- `team_member_can_author_after_transfer` — a resource homed in the context: `can_modify_resource(member, resource)` is false before and **true** after transfer for an owner/maintainer/**member**; **false** for a `watcher`; unchanged (false) for a non-member. (Reuse `can_modify_resource` via `SELECT can_modify_resource($1,$2)`.)
- `transferrer_retains_access` — the acting owner/maintainer of the target team still reads+writes post-transfer.
- authz matrix, each ⇒ `Forbidden`: caller does not administer the context; caller is not owner/maintainer of the target team; target is the gating/root team.
- `idempotent_when_already_team_owned` — second call returns `reassigned == false`, no second event.
- `slug_collision_conflict` — target team already owns a context with the same slug ⇒ `ApiError::Conflict` (or the crate's 409 variant), owner unchanged.
- `emits_context_reassigned_event` — one `context_reassigned` event with correct `to_owner_id`.
- `system_admin_bypass` — a system admin transfers a context they don't otherwise administer.

> Confirm the exact `ApiError` 409 variant name (`Conflict`?) the crate uses — grep `crates/temper-services/src/error.rs`. Confirm the `can_modify_resource` SQL name/arity. Reuse the test module's existing admin-minting idiom rather than re-seeding.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db context_service::tests
```
Expected: FAIL — `reassign` not found.

- [ ] **Step 3: Implement `reassign`**

Add to `context_service.rs`, beside `share`/`unshare`:

```rust
/// Transfer a context's ownership to a team — the single path to shared authorship.
///
/// Auth before writes: the two-sided `can_share` gate (system-admin, OR caller administers
/// the context AND manages the target team, target not the root team). The owner change is
/// event-sourced (`context_reassigned`) via the substrate writes layer.
pub async fn reassign(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    to_team_id: uuid::Uuid,
) -> ApiResult<ReassignContextOutcome> {
    if !can_share(pool, caller, context_id, to_team_id).await? {
        return Err(ApiError::Forbidden);
    }
    ensure_context_and_team_exist(pool, context_id, to_team_id).await?;

    // Current owner — for the audit fields + idempotency.
    let cur = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!", slug
             FROM kb_contexts WHERE id = $1"#,
        context_id,
    )
    .fetch_one(pool)
    .await?;
    if cur.owner_table == "kb_teams" && cur.owner_id == to_team_id {
        return Ok(ReassignContextOutcome { context_id, owner_ref: team_owner_ref(pool, to_team_id).await?, reassigned: false });
    }

    // Slug must be unique under the NEW owner — 409 rather than a silent re-slug or an
    // opaque UNIQUE violation from the projector.
    let collision = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_contexts
             WHERE owner_table='kb_teams' AND owner_id=$1 AND slug=$2) AS "e!""#,
        to_team_id, cur.slug,
    )
    .fetch_one(pool)
    .await?;
    if collision {
        return Err(ApiError::Conflict(format!(
            "team already owns a context with slug '{}'; rename before transferring", cur.slug
        )));
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::reassign_context_with(
        pool,
        temper_substrate::ids::ContextId::from(context_id),
        (cur.owner_table.as_str(), cur.owner_id),
        ("kb_teams", to_team_id),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(ReassignContextOutcome {
        context_id,
        owner_ref: team_owner_ref(pool, to_team_id).await?,
        reassigned: true,
    })
}

/// `+team-slug` decorated owner ref for the outcome (mirrors `create`'s CASE expression).
async fn team_owner_ref(pool: &PgPool, team_id: uuid::Uuid) -> ApiResult<String> {
    let slug = sqlx::query_scalar!("SELECT slug FROM kb_teams WHERE id = $1", team_id)
        .fetch_one(pool)
        .await?;
    Ok(format!("+{slug}"))
}
```

> Confirm `ApiError::Conflict(String)` exists (else use the crate's 409 variant). Confirm `temper_substrate::ids::ContextId` + `temper_substrate::events::EventContext` re-export paths (adjust to wherever substrate exposes them — mirror how `reassign_service` imports `ResourceId`/`EventContext`). `ReassignContextOutcome` is defined in Task 3; if implementing service before wire types, stub it locally then move it — but prefer doing Task 3's type first and importing it.

- [ ] **Step 4: Prepare cache + run tests**

```bash
cargo make prepare-services
cargo nextest run -p temper-services --features test-db context_service::tests
```
Expected: all new tests PASS.

- [ ] **Step 5: `cargo make check` + commit** (`crates/temper-services/ .sqlx/`).

---

## Task 3: Wire types (`temper-core`)

**Files:**
- Modify: `crates/temper-core/src/types/context.rs` (add the two types beside the share types)

**Interfaces produced:** `ReassignContextRequest { to_team_id: Uuid }`, `ReassignContextOutcome { context_id: Uuid, owner_ref: String, reassigned: bool }`.

- [ ] **Step 1: Add the types** — mirror the derive attributes already on `ShareContextRequest`/`ShareContextOutcome` in `context.rs` (ts-rs + web-api ToSchema + serde), so OpenAPI + TS pick them up identically:

```rust
/// Request to transfer a context's ownership to a team (context id is in the path).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignContextRequest {
    pub to_team_id: Uuid,
}

/// Outcome of a context ownership transfer.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignContextOutcome {
    pub context_id: Uuid,
    /// The new `+team-slug` decorated owner ref.
    pub owner_ref: String,
    /// False when the context was already team-owned by the target (idempotent no-op).
    pub reassigned: bool,
}
```

> Copy the **exact** feature-gate + `ts(export_to = …)` attributes from the neighbouring `ShareContext*` types, not this sketch, so the generated TS lands in the same module.

- [ ] **Step 2: Regenerate TS types + verify** — `cargo make generate-ts-types && cargo make check`. Commit (`crates/temper-core/ packages/` for the regenerated shared types).

---

## Task 4: API handler + route + OpenAPI (three-artifact)

**Files:**
- Modify: `crates/temper-api/src/handlers/contexts.rs` (add `reassign` handler beside `share_team`/`unshare_team`)
- Modify: `crates/temper-api/src/routes.rs` (gated route), `openapi.rs` (path + schemas)

**Interfaces produced:** `POST /api/contexts/{id}/reassign`.

- [ ] **Step 1: Handler** (mirror `contexts::share_team`):

```rust
#[utoipa::path(
    post,
    path = "/api/contexts/{id}/reassign",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = ReassignContextRequest,
    responses(
        (status = 200, description = "Context ownership transferred", body = ReassignContextOutcome),
        (status = 403, description = "Forbidden (not a context admin, or not owner/maintainer of the target team, or root team)"),
        (status = 404, description = "Context or team not found"),
        (status = 409, description = "Target team already owns a context with this slug"),
    )
)]
pub async fn reassign(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ReassignContextRequest>,
) -> ApiResult<Json<ReassignContextOutcome>> {
    let outcome = context_service::reassign(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        context_id,
        body.to_team_id,
    )
    .await?;
    Ok(Json(outcome))
}
```

- [ ] **Step 2: Route + OpenAPI** — the router is a utoipa-axum `OpenApiRouter`; routes are registered with `.routes(routes!(...))` (which *also* collects the OpenAPI path), **not** a plain `.route(...)`. Add beside the existing context team routes (`routes.rs:88-94`):
`.routes(routes!(handlers::contexts::reassign))`.
`openapi.rs` has **no `paths(...)` list** — paths are derived from the router (`openapi.rs:21-25`). The only `openapi.rs` edit is adding `ReassignContextRequest` + `ReassignContextOutcome` to `components(schemas(...))` (beside `ShareContextRequest`/`ShareContextOutcome`, `openapi.rs:43-44`). Extend the openapi assertion test (`openapi.rs:292-293`) with `assert!(json.contains("/api/contexts/{id}/reassign"));`.

- [ ] **Step 3: Regenerate the three artifacts + verify**

```bash
cargo make openapi        # openapi.json + temper-rb gem (Docker) + temper-ts schema.ts
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make check          # includes openapi-check + openapi-rb-drift + openapi-ts-drift (compared vs git)
git add crates/temper-api/ openapi.json clients/temper-rb/ clients/temper-ts/
```
If Docker is unavailable for the gem regen, generate what you can, stage it, and note in the PR that `test-ruby` CI is the backstop; the ts drift gate never skips, so it must be green locally.

- [ ] **Step 4: Commit.**

---

## Task 5: `temper-client` method

**Files:** Modify `crates/temper-client/src/contexts.rs`.

- [ ] Add, mirroring `share_team`'s body **exactly** (`contexts.rs:67-78`) — there is **no `post_json`**; the pattern is `resolve_token` → `post(&path).json(body)` → `send_json(&Method::POST, &path, req, Some(&token))`:

```rust
    /// Transfer a context's ownership to a team. POST /api/contexts/{id}/reassign.
    pub async fn reassign(
        &self,
        id: uuid::Uuid,
        body: &temper_core::types::context::ReassignContextRequest,
    ) -> Result<temper_core::types::context::ReassignContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{id}/reassign");
        let req = self.http.post(&path).json(body);
        self.http.send_json(&Method::POST, &path, req, Some(&token)).await
    }
```

> Copy the surrounding `use` (`reqwest::Method`) and exact helper signatures from `share_team`/`unshare_team` in the same file; do not invent names. `cargo make check` + commit.

---

## Task 6: CLI `temper context transfer`

**Files:** Modify `crates/temper-cli/src/cli.rs` (the `ContextAction` enum, ~line 780 — `Share` at 806, `Unshare` at 813), `crates/temper-cli/src/commands/context_cmd.rs` (dispatch + action). *(Note the file is `context_cmd.rs`, not `context.rs`.)*

- [ ] **Step 1: `ContextAction::Transfer` variant** (mirror the `Share`/`Unshare` clap attrs):

```rust
    /// Transfer a context's ownership to a team (shared authorship requires team ownership).
    Transfer {
        /// Context ref — `@me/slug`, `@handle/slug`, or UUID.
        context: String,
        /// Target team (slug, +slug, decorated ref, or UUID).
        team: String,
    },
```

- [ ] **Step 2: Action** (mirror `share_remote` in `context_cmd.rs:227`, which resolves the team via `resolve_team_id`):

```rust
pub async fn transfer_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    // IMPORTANT: use the read-side resolver that ACCEPTS `@me`, not `resolve_context_id`
    // (the share/unshare resolver DELIBERATELY REFUSES `@me`, context_cmd.rs:261-264).
    // The transfer JTBD is literally `@me/my-project → team`, so `@me` MUST resolve.
    let context_id = resolve_context_id_for_read(client, context).await?;
    let to_team_id = resolve_team_id(client, team).await?;
    let req = temper_core::types::context::ReassignContextRequest { to_team_id };
    let outcome = client
        .contexts()
        .reassign(context_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    Ok(())
}
```

Wire the `Transfer` variant to `transfer_remote` in the `ContextAction` dispatcher.

> **Confirm the `@me`-accepting resolver's exact name** — the review identified `resolve_context_id` as the *share-path* resolver that refuses `@me` (`context_cmd.rs:261-264`) and pointed at a read-side variant that accepts it. Read `context_cmd.rs` around 227-264, pick the resolver that accepts `@me`, and if none exists, either (a) generalize `resolve_context_id` with a flag, or (b) reduce the CLI to `@handle/slug`|UUID and note the `@me` limitation in `--help`. Do **not** ship a `context transfer @me/x` that 400s — that is the headline flow. `cargo build -p temper-cli --bin temper && ./target/debug/temper context transfer --help`, then `cargo make check` + commit.

---

## Task 7: MCP tool `transfer_context`

**Files:** Modify `crates/temper-mcp/src/tools/contexts.rs` (the tool body, beside `share_context` at `contexts.rs:120-164`) **and** `crates/temper-mcp/src/service.rs` (the `#[tool]` registration, beside `share_context` at ~626-632). **Both** edits are required — the body in `tools/`, the `#[tool]`-annotated delegating method in `service.rs`; without the `service.rs` method the tool is never exposed.

- [ ] **Step 1: Input struct + tool body** in `tools/contexts.rs`, mirroring `ShareContextInput`/`share_context` (`contexts.rs:43-141`) — `svc.require_profile()` → `context_service::reassign(pool, caller, input.context, input.to_team)` → `map_api_error`:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransferContextInput {
    /// Context to transfer (UUID).
    pub context: Uuid,
    /// Target team the context will be owned by (UUID).
    pub to_team: Uuid,
}
```

- [ ] **Step 2: `#[tool]` registration** in `service.rs` (~626), mirroring the `share_context` `#[tool]` method that delegates to `tools::contexts::share_context`. Add the sibling `transfer_context` delegating to `tools::contexts::transfer_context`.

> Match `share_context`'s exact auth-context extraction + error mapping in both places. This delivers the agent-first path for the op; broader team lifecycle over MCP is the separate Seq-21 task. `cargo make check` + commit.

---

## Task 8: E2E — CLI → API → DB owner change

**Files:** Create `tests/e2e/tests/context_transfer_test.rs`.

- [ ] Mirror the closest existing context/team e2e (grep `tests/e2e/tests/` for one that seeds a team + membership and drives `temper context ...` or `temper team ...`). The test must:

```
// 1. Start the harness (real Axum + test DB), authenticated as "alice".
// 2. Seed a team "acme" with alice as owner/maintainer; seed alice's personal context + a resource homed in it.
// 3. Run: temper context transfer <context-ref> +acme   (as alice).
// 4. Assert exit 0 and the printed owner_ref is "+acme".
// 5. DB asserts: kb_contexts.owner_table='kb_teams', owner_id=acme; and can_modify_resource(<another acme member>, <resource>) is TRUE.
```

- [ ] Regenerate e2e cache + run: `cargo build -p temper-cli --bin temper && cargo make prepare-e2e && cargo make test-e2e`. Commit (`tests/e2e/ tests/e2e/.sqlx/`).

---

## Final Verification (before flipping the PR out of draft)

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up
cargo make check
cargo nextest run --workspace --exclude temper-cloud
cargo make test-db
cargo make test-e2e
cargo make test-artifacts   # if the replay-roundtrip test is artifact-gated
```

Confirm: `.sqlx/` caches committed (workspace + `crates/temper-services/.sqlx` + `crates/temper-api/.sqlx` + `tests/e2e/.sqlx`); the three OpenAPI artifacts staged and drift-clean; `tests/scenario_schema.rs` snapshot updated; the new TS type present.

## Self-Review Notes (spec → task coverage)

- Event-sourced owner change (mirrors `resource_reassign`, replay-idempotent because `kb_contexts` is an input table) → Task 1. Two-sided `can_share` authz + slug-collision 409 → Task 2. Wire types → Task 3. Gated API + three-artifact OpenAPI → Task 4. Client → Task 5. CLI `context transfer` → Task 6. `transfer_context` MCP tool → Task 7. E2E owner change + member-can-author assertion → Task 8. Non-goals (resource owner/originator unmoved, no context write-grant surface, cogmap bindings untouched) are enforced by omission — this task only flips the container's owner columns; the T-D task owns residual-access follow-ups.
