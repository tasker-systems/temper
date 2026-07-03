# Resource Ownership Reassignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire an event-sourced, authz-gated resource ownership reassignment (single + bulk) that mutates `kb_resource_homes.owner_profile_id` in place, serving offboarding + mis-attribution — and retire the dead offer/accept `kb_transfers` types.

**Architecture:** A new `resource_reassigned` event mirrors the existing `resource_rehomed` precedent (payload → `SeedAction` → `fire` → paired SQL projector/mutation fns → `replay` dispatch). A service-direct `reassign_service` (mirroring `invitation_service`) owns team-role authorization and calls a new `writes::reassign_resource_*` layer that fires the event. Thin API handlers, a client method, and CLI commands round out the surface. No `Backend` trait change.

**Tech Stack:** Rust (axum, sqlx compile-time macros, ts-rs), PostgreSQL 17/18 + pgvector, cargo-make + cargo-nextest.

## Global Constraints

- **DB is event-sourced:** `kb_resource_homes` is projected from the event stream (`replay.rs`). Owner mutation MUST be an event (`resource_reassigned`), never a bare `UPDATE`.
- **Auth before writes:** every reassign path authorizes before any mutation.
- **Typed structs over inline JSON:** no `serde_json::json!()` for structured wire data — define structs in `temper-core`.
- **sqlx macros:** production + substantive test queries use `sqlx::query!`/`query_as!`/`query_scalar!`. After any SQL/schema change regenerate caches (steps included).
- **Additive-only on `main`:** the migration adds functions + one `kb_event_types` row only. NO `DROP` (the `kb_transfers` table drop is task #6, not this plan).
- **New event names must be seeded:** `_event_append` raises `event_type % not seeded` unless a `kb_event_types` row exists. `resource_reassigned` gets a `NULL` `payload_schema` row (like `resource_rehomed`) so it stays out of the published-schema `TYPED_EVENT_NAMES` invariant.
- **Env for tests:** `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`; `cargo make docker-up` before db tests.
- **Recipient identifiers are profile UUIDs** (matching `team add-member`/`set-role`); no `@handle` resolution.
- **Commit message trailer:** end every commit body with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

---

## File Structure

**Created:**
- `migrations/20260703000003_resource_reassign_fns.sql` — event-type seed + `_project_resource_reassigned` + `resource_reassign` SQL fns.
- `crates/temper-services/src/services/reassign_service.rs` — auth + single/bulk reassign logic + tests.
- `crates/temper-api/src/handlers/reassign.rs` — thin single + bulk handlers.
- `crates/temper-core/src/types/reassign.rs` — wire types (renamed from `transfer.rs`).
- `tests/e2e/tests/reassign_test.rs` — CLI→API→DB e2e.

**Modified:**
- `crates/temper-substrate/src/payloads.rs` — `ResourceReassigned` payload.
- `crates/temper-substrate/src/events.rs` — `EventKind` + `SeedAction` + `fire` arm.
- `crates/temper-substrate/src/replay.rs` — non-authored classification + projection dispatch.
- `crates/temper-substrate/src/writes.rs` — `reassign_resource_*` fns.
- `crates/temper-services/src/services/mod.rs` — register `reassign_service`.
- `crates/temper-api/src/handlers/mod.rs`, `routes.rs`, `openapi.rs` — wire handlers/routes/schemas.
- `crates/temper-client/src/resources.rs`, `teams.rs` — client methods.
- `crates/temper-cli/src/cli.rs`, `commands/resource.rs`, `commands/team.rs` — CLI.
- `crates/temper-core/src/types/mod.rs` — re-export `reassign` (drop dead `transfer` exports).

---

## Task 1: Substrate `resource_reassigned` event + writes layer

**Files:**
- Create: `migrations/20260703000003_resource_reassign_fns.sql`
- Modify: `crates/temper-substrate/src/payloads.rs` (after `ResourceRehomed`, ~line 430)
- Modify: `crates/temper-substrate/src/events.rs` (`EventKind` ~42/65/93; `SeedAction` ~253; `event_type()` ~304; `fire` arm ~830)
- Modify: `crates/temper-substrate/src/replay.rs` (~158, ~347)
- Modify: `crates/temper-substrate/src/writes.rs` (after `delete_resource_in_tx`, ~line 328)

**Interfaces:**
- Produces: `payloads::ResourceReassigned { resource_id: ResourceId, from_profile_id: ProfileId, to_profile_id: ProfileId }`; `SeedAction::ResourceReassign { resource, from_profile, to_profile, emitter }`; `writes::reassign_resource_with(pool, resource: ResourceId, from: ProfileId, to: ProfileId, emitter: EntityId, ctx: EventContext) -> Result<()>` and `writes::reassign_resource_in_tx(conn, resource, from, to, emitter, ctx) -> Result<()>`; SQL `resource_reassign(jsonb, uuid, jsonb, uuid) -> uuid`.

- [ ] **Step 1: Write the migration**

Create `migrations/20260703000003_resource_reassign_fns.sql`:

```sql
-- Ownership reassignment: event-sourced owner change on kb_resource_homes.
-- Mirrors resource_rehomed (anchor move); this moves owner_profile_id in place.
-- Additive only: one event-type row + two functions. No table changes.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps
-- it out of the published-schema TYPED_EVENT_NAMES invariant (as resource_rehomed).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('resource_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half (replay-stable): set the resource's home owner to to_profile_id.
CREATE FUNCTION _project_resource_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resource_homes
       SET owner_profile_id = (p_payload->>'to_profile_id')::uuid
       WHERE resource_id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_reassign: resource % has no home', v_resource; END IF;
    RETURN v_resource;
END;
$$;

-- Mutation half: append the event at the resource's CURRENT home (it does not move),
-- then project. Act-correlation params mirror resource_rehome (20260629000003).
CREATE FUNCTION resource_reassign(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor
      FROM kb_resource_homes WHERE resource_id = v_resource;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_reassign: resource % has no home', v_resource; END IF;
    -- Backstop: only context-homed resources are reassignable. A cogmap interior is
    -- team-resource-derived, not personally owned (spec non-goal) — refuse at the write
    -- primitive so the invariant holds even if a future surface bypasses the service.
    IF v_anchor_tbl <> 'kb_contexts' THEN
        RAISE EXCEPTION 'resource_reassign: resource % is not context-homed (cogmap interiors are not reassignable)', v_resource;
    END IF;
    v_ev := _event_append('resource_reassigned', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_reassigned(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 2: Add the payload struct**

In `payloads.rs`, after the `ResourceRehomed` struct:

```rust
/// Reassign a resource's owner — set its home row's `owner_profile_id` to `to_profile_id`.
/// `from_profile_id` is recorded for the audit trail; the projector writes only the new owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceReassigned {
    pub resource_id: ResourceId,
    pub from_profile_id: ProfileId,
    pub to_profile_id: ProfileId,
}
```

(If `ProfileId` is not already imported in `payloads.rs`, it is — `ResourceCreated` uses it. Confirm the `use` line covers `ProfileId`.)

- [ ] **Step 3: Add EventKind variant + string mapping**

In `events.rs`: add `ResourceReassigned,` to the `EventKind` enum (near `ResourceRehomed`, ~line 42); add `EventKind::ResourceReassigned => "resource_reassigned",` to the name match (~65); add `"resource_reassigned" => EventKind::ResourceReassigned,` to the parse match (~93).

- [ ] **Step 4: Add SeedAction variant + event_type arm**

In `events.rs`, after the `ResourceRehome` `SeedAction` variant (~line 253):

```rust
    ResourceReassign {
        resource: ResourceId,
        from_profile: ProfileId,
        to_profile: ProfileId,
        emitter: EntityId,
    },
```

And in the `event_type()` match (~304): `SeedAction::ResourceReassign { .. } => EventKind::ResourceReassigned,`

(Confirm `ProfileId` is in scope in `events.rs`; add to the `use crate::ids::{…}` line if missing.)

- [ ] **Step 5: Add the fire arm**

In `events.rs`, after the `SeedAction::ResourceRehome` arm in the `fire`/`fire_with` dispatch (~line 830):

```rust
        SeedAction::ResourceReassign {
            resource,
            from_profile,
            to_profile,
            emitter,
        } => {
            let payload = payloads::ResourceReassigned {
                resource_id: resource,
                from_profile_id: from_profile,
                to_profile_id: to_profile,
            };
            let id = sqlx::query_scalar!(
                "SELECT resource_reassign($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_reassign returned null")?;
            Ok(Fired::Resource(ResourceId::from(id)))
        }
```

(`ctx_meta` / `ctx_inv` are the act-context locals already bound in `fire_with` — identical to the `ResourceRehome` arm above it.)

- [ ] **Step 6: Add replay classification + projection dispatch**

In `replay.rs`: add `| EventKind::ResourceReassigned` to the non-authored-mutation classification list (~line 158, alongside `EventKind::ResourceRehomed`). Add the projection dispatch arm (~line 347):

```rust
            EventKind::ResourceReassigned => {
                sqlx::query("SELECT _project_resource_reassigned($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
```

- [ ] **Step 7: Add the writes-layer fns**

In `writes.rs`, after `delete_resource_in_tx` (~line 328):

```rust
/// Reassign a resource's owner (event-sourced, in-place). Un-attributed convenience.
pub async fn reassign_resource(
    pool: &PgPool,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
) -> Result<()> {
    reassign_resource_with(pool, resource, from, to, emitter, EventContext::default()).await
}

/// [`reassign_resource`] under an explicit [`EventContext`] — the `resource_reassigned`
/// act is correlated to the caller's invocation + stamped with its authorship.
pub async fn reassign_resource_with(
    pool: &PgPool,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    reassign_resource_in_tx(&mut tx, resource, from, to, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant — fires on a caller-supplied connection (no begin/commit),
/// so the bulk path can reassign N resources atomically.
pub async fn reassign_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::ResourceReassign {
            resource,
            from_profile: from,
            to_profile: to,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}
```

(Confirm `ProfileId` is imported in `writes.rs` — `KernelCreateParams` uses it, so it is.)

- [ ] **Step 8: Regenerate sqlx cache + verify compile**

The migration must apply before the `query_scalar!` in Step 5 can typecheck.

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up
cargo sqlx migrate run
cargo sqlx prepare --workspace -- --all-features
cargo make check
```

Expected: migration applies clean; `check` passes (fmt + clippy + docs + machete + TS). If clippy flags an unused `reassign_resource` (the un-attributed convenience isn't called until Task 3 uses `_with`), keep it — it's the documented API-symmetry sibling of `delete_resource`; add `#[allow(dead_code)]` only if machete/clippy hard-errors, and remove it once Task 3 lands.

- [ ] **Step 9: Commit**

```bash
git add migrations/ crates/temper-substrate/ .sqlx/
git commit -m "$(cat <<'EOF'
feat(reassign): substrate resource_reassigned event + writes layer

Event-sourced owner change on kb_resource_homes, mirroring resource_rehomed.
Additive migration (event-type seed + projector + mutation fns).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `reassign_service` — single-resource (owner + admin auth)

**Files:**
- Create: `crates/temper-services/src/services/reassign_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (add `pub mod reassign_service;`)

**Interfaces:**
- Consumes: `writes::reassign_resource_with`, `writes::resolve_emitter` (temper-substrate); `team_service::{role_on_team, can_manage}`; `ApiError`/`ApiResult` (`crate::error`); `ProfileId` (`temper_core::types::ids`).
- Produces: `reassign_service::reassign_resource(pool, caller: ProfileId, resource_id: Uuid, to_profile_id: Uuid) -> ApiResult<()>`.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-services/src/services/reassign_service.rs`. Start with the module doc + the test module (mirrors `invitation_service.rs` seed helpers):

```rust
//! Resource ownership reassignment over `kb_resource_homes`, event-sourced via
//! `writes::reassign_resource_with`. Service-direct (no Backend-trait command),
//! same precedent as `invitation_service` / `team_service`. Authorization
//! precedes every write.
//!
//! Two authorized paths: the current owner may reassign their own resource to
//! anyone (mis-attribution self-fix); a team admin may reassign a resource
//! scoped to a team they manage, to a member of that team (offboarding +
//! admin-assisted mis-attribution).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::ProfileId;

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use temper_core::types::team::TeamRole;

    async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle).fetch_one(pool).await.unwrap();
        // Every profile needs a `<handle>@web` emitter entity for resolve_emitter.
        sqlx::query(
            "INSERT INTO kb_entities (id, name, profile_id) \
             VALUES (uuid_generate_v7(), $1 || '@web', $2)",
        )
        .bind(handle).bind(id).execute(pool).await.unwrap();
        ProfileId::from(id)
    }

    async fn mk_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (id, slug, owner_profile_id) \
             VALUES (uuid_generate_v7(), $1, $2) RETURNING id",
        )
        .bind(slug).bind(*owner).fetch_one(pool).await.unwrap()
    }

    async fn mk_homed_resource(pool: &PgPool, ctx: Uuid, owner: ProfileId) -> Uuid {
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('r','r') RETURNING id",
        ).fetch_one(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        ).bind(rid).bind(ctx).bind(*owner).execute(pool).await.unwrap();
        rid
    }

    /// A resource homed in a cogmap (map interior). `anchor_id` needs no real cogmap row —
    /// `kb_resource_homes.anchor_id` has no FK (the schema's polymorphic-anchor note), and the
    /// reassign guard only inspects `anchor_table`.
    async fn mk_cogmap_homed_resource(pool: &PgPool, owner: ProfileId) -> Uuid {
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('node','node') RETURNING id",
        ).fetch_one(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_cogmaps', uuid_generate_v7(), $2, $2)",
        ).bind(rid).bind(*owner).execute(pool).await.unwrap();
        rid
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
        ).bind(slug).fetch_one(pool).await.unwrap()
    }

    async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1,$2,$3::team_role,'native'::team_member_source)",
        ).bind(team).bind(*p).bind(role).execute(pool).await.unwrap();
    }

    async fn share_ctx(pool: &PgPool, ctx: Uuid, team: Uuid) {
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1,$2)")
            .bind(ctx).bind(team).execute(pool).await.unwrap();
    }

    async fn owner_of(pool: &PgPool, resource: Uuid) -> Uuid {
        sqlx::query_scalar!("SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id=$1", resource)
            .fetch_one(pool).await.unwrap()
    }

    async fn visible_to(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
        sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
            *profile, resource,
        ).fetch_one(pool).await.unwrap().unwrap_or(false)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_can_reassign_and_visibility_follows(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "alice-ctx", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(&pool, alice, r, *bob).await.expect("owner reassigns");

        assert_eq!(owner_of(&pool, r).await, *bob);
        assert!(visible_to(&pool, bob, r).await, "new owner sees it");
        // originator floor is untouched, so alice still sees it too.
        assert!(visible_to(&pool, alice, r).await, "originator retains access");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reassign_emits_event(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        reassign_resource(&pool, alice, r, *bob).await.unwrap();
        let n = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
             WHERE t.name='resource_reassigned' AND (e.payload->>'resource_id')::uuid=$1", r,
        ).fetch_one(&pool).await.unwrap().unwrap();
        assert_eq!(n, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reassign_to_current_owner_is_noop(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        reassign_resource(&pool, alice, r, *alice).await.expect("idempotent no-op");
        assert_eq!(owner_of(&pool, r).await, *alice);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn stranger_cannot_reassign(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let mallory = mk_profile(&pool, "mallory").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(&pool, mallory, r, *bob).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_reassign_cogmap_homed_resource_even_as_owner(pool: PgPool) {
        // The owner-path hole: alice owns a cogmap-homed node, but map interiors are not
        // reassignable. Must be rejected BEFORE the owner-path auth would allow it.
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let r = mk_cogmap_homed_resource(&pool, alice).await;
        let err = reassign_resource(&pool, alice, r, *bob).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_admin_can_reassign_scoped_resource_to_member(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;   // current owner + departing
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_context(&pool, "shared", alice).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(&pool, admin, r, *steward).await.expect("admin reassigns to member");
        assert_eq!(owner_of(&pool, r).await, *steward);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_to_non_member(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        let ctx = mk_context(&pool, "shared", alice).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(&pool, admin, r, *outsider).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_unscoped_resource(pool: PgPool) {
        // resource NOT shared to the admin's team → out of reach.
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_context(&pool, "private", alice).await; // NOT shared
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(&pool, admin, r, *steward).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo nextest run -p temper-services --features test-db reassign_service
```
Expected: FAIL — `reassign_resource` not found. (Confirm `kb_contexts` / `kb_entities` column names against the live schema if a seed helper errors — `psql \d kb_contexts`, `\d kb_entities`; adjust the INSERTs, not the assertions.)

- [ ] **Step 3: Implement `reassign_resource`**

Add to `reassign_service.rs` (above the test module):

```rust
/// A resource's home owner + the anchor it's homed under.
struct HomeRow {
    owner: Uuid,
    anchor_table: String,
    anchor_id: Uuid,
}

async fn home_of(pool: &PgPool, resource: Uuid) -> ApiResult<HomeRow> {
    sqlx::query_as!(
        HomeRow,
        "SELECT owner_profile_id AS owner, anchor_table, anchor_id \
           FROM kb_resource_homes WHERE resource_id = $1",
        resource,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Is there a team T where caller manages T, `resource` is homed in a context shared
/// to T, and `to` is a member of T? (admin-path reach: from-scope + into-scope).
async fn admin_reach(pool: &PgPool, caller: ProfileId, resource: Uuid, to: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM kb_team_contexts tc
            JOIN kb_resource_homes h
              ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
            JOIN kb_team_members cm
              ON cm.team_id = tc.team_id AND cm.profile_id = $2
                 AND cm.role IN ('owner','maintainer')
            JOIN kb_team_members tm
              ON tm.team_id = tc.team_id AND tm.profile_id = $3
            WHERE h.resource_id = $1
        ) AS "exists!: bool"
        "#,
        resource, *caller, to,
    )
    .fetch_one(pool)
    .await?)
}

/// Reassign a resource's owner to `to_profile_id`. Auth: current owner (any target)
/// OR team-admin over a team the resource is scoped to, to a member of that team.
/// Reassigning to the current owner is an idempotent no-op.
pub async fn reassign_resource(
    pool: &PgPool,
    caller: ProfileId,
    resource_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<()> {
    let home = home_of(pool, resource_id).await?;

    // Only context-homed resources are reassignable. A cogmap-homed resource is a map
    // interior (team-resource-derived, not personally owned) — the owner path would
    // otherwise let its owner flip it, so guard here for BOTH paths. The admin path's
    // reach query already excludes non-context homes structurally; this is the owner-path
    // closure + a single clear 400 regardless of caller.
    if home.anchor_table != "kb_contexts" {
        return Err(ApiError::BadRequest(
            "cannot reassign a cogmap-homed resource; map interiors are not personally owned".to_string(),
        ));
    }

    // Idempotent no-op — but still authorize, so an unauthorized caller can't probe.
    let authorized = home.owner == *caller
        || admin_reach(pool, caller, resource_id, to_profile_id).await?;
    if !authorized {
        return Err(ApiError::Forbidden);
    }
    if home.owner == to_profile_id {
        return Ok(());
    }

    // Auth passed — emit the event as the caller.
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::reassign_resource_with(
        pool,
        temper_substrate::ids::ResourceId::from(resource_id),
        home.owner.into(),
        to_profile_id.into(),
        emitter,
        temper_substrate::writes::EventContext::default(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}
```

Confirm the exact paths for `ResourceId`, `EventContext`, and `resolve_emitter` re-exports (`temper_substrate::ids::ResourceId`, `temper_substrate::writes::EventContext`) against `crates/temper-substrate/src/lib.rs`; adjust the `use`/path if the crate re-exports them elsewhere. `ProfileId`/`Uuid` `.into()` conversions: `ProfileId: From<Uuid>` exists (used throughout). Map `anyhow::Error` from writes to `ApiError::Internal` — confirm `ApiError` has an `Internal(String)` variant (mirror how other services wrap `anyhow`; if the variant differs, use the same one `db_backend` uses via `api_err`).

- [ ] **Step 4: Register the module**

In `crates/temper-services/src/services/mod.rs` add (alphabetical): `pub mod reassign_service;`

- [ ] **Step 5: Regenerate the per-crate test cache + run tests**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make prepare-services
cargo nextest run -p temper-services --features test-db reassign_service
```
Expected: all seven tests PASS.

- [ ] **Step 6: `cargo make check` + commit**

```bash
cargo make check
git add crates/temper-services/ .sqlx/
git commit -m "$(cat <<'EOF'
feat(reassign): single-resource reassign service (owner + admin auth)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `reassign_service` — bulk team reassignment

**Files:**
- Modify: `crates/temper-services/src/services/reassign_service.rs`

**Interfaces:**
- Consumes: Task 2's helpers; `writes::reassign_resource_in_tx`; `team_service::{role_on_team, can_manage}`.
- Produces: `reassign_service::reassign_team_resources(pool, caller: ProfileId, team_id: Uuid, from_profile_id: Uuid, to_profile_id: Uuid) -> ApiResult<Vec<Uuid>>` (returns the reassigned resource ids).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `reassign_service.rs`:

```rust
    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_reassigns_only_owned_and_scoped(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let other = mk_profile(&pool, "other").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;

        let shared = mk_context(&pool, "shared", leaver).await;
        share_ctx(&pool, shared, team).await;
        let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

        let in_scope = mk_homed_resource(&pool, shared, leaver).await;   // owned+scoped → moves
        let out_scope = mk_homed_resource(&pool, private, leaver).await;  // owned, not scoped → stays
        let not_leaver = mk_homed_resource(&pool, shared, other).await;   // scoped, other owner → stays

        let moved = reassign_team_resources(&pool, admin, team, *leaver, *steward)
            .await.expect("bulk reassign");

        assert_eq!(moved, vec![in_scope]);
        assert_eq!(owner_of(&pool, in_scope).await, *steward);
        assert_eq!(owner_of(&pool, out_scope).await, *leaver);
        assert_eq!(owner_of(&pool, not_leaver).await, *other);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_non_manager_forbidden(pool: PgPool) {
        let stranger = mk_profile(&pool, "stranger").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, steward, "member").await;
        let err = reassign_team_resources(&pool, stranger, team, *leaver, *steward)
            .await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_into_non_member_forbidden(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        let err = reassign_team_resources(&pool, admin, team, *leaver, *outsider)
            .await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_empty_match_is_ok(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        let moved = reassign_team_resources(&pool, admin, team, *leaver, *steward)
            .await.expect("empty is ok");
        assert!(moved.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db reassign_service::tests::bulk
```
Expected: FAIL — `reassign_team_resources` not found.

- [ ] **Step 3: Implement `reassign_team_resources`**

Add to `reassign_service.rs`:

```rust
/// Bulk-reassign, from `from_profile_id` to `to_profile_id`, every resource owned by
/// `from` and homed in a context shared to `team_id`. Auth: caller manages the team AND
/// `to` is a member of it. One transaction; returns the reassigned resource ids.
pub async fn reassign_team_resources(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    from_profile_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<Vec<Uuid>> {
    use temper_services::services::team_service::{can_manage, role_on_team};

    // Auth before writes: caller manages the team, and `to` is a member of it.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    let to_is_member = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2) AS "e!: bool""#,
        team_id, to_profile_id,
    )
    .fetch_one(pool)
    .await?;
    if !to_is_member {
        return Err(ApiError::Forbidden);
    }

    // Scope read: resources owned by `from` AND homed in a context shared to the team.
    let targets: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        WHERE tc.team_id = $1 AND h.owner_profile_id = $2
        "#,
        team_id, from_profile_id,
    )
    .fetch_all(pool)
    .await?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = pool.begin().await?;
    for &rid in &targets {
        temper_substrate::writes::reassign_resource_in_tx(
            &mut tx,
            temper_substrate::ids::ResourceId::from(rid),
            from_profile_id.into(),
            to_profile_id.into(),
            emitter,
            temper_substrate::writes::EventContext::default(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    tx.commit().await?;
    Ok(targets)
}
```

(The `use temper_services::services::team_service…` may need to be `crate::services::team_service` depending on the crate-internal path — use `crate::` since this file is inside temper-services.)

- [ ] **Step 4: Prepare cache + run tests**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make prepare-services
cargo nextest run -p temper-services --features test-db reassign_service
```
Expected: all (Task 2 + Task 3) tests PASS.

- [ ] **Step 5: `cargo make check` + commit**

```bash
cargo make check
git add crates/temper-services/ .sqlx/
git commit -m "$(cat <<'EOF'
feat(reassign): bulk team-scoped reassignment (offboarding)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Wire types — rename `transfer.rs` → `reassign.rs`, retire dead types

**Files:**
- Rename: `crates/temper-core/src/types/transfer.rs` → `crates/temper-core/src/types/reassign.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (~line 82 re-export)

**Interfaces:**
- Produces: `ReassignResourceRequest { to_profile_id: Uuid }`, `ReassignAck { resource_id: Uuid, to_profile_id: Uuid }`, `BulkReassignRequest { from_profile_id: Uuid, to_profile_id: Uuid }` (kept), `BulkReassignAck { resource_ids: Vec<Uuid> }`.

- [ ] **Step 1: Replace the file contents**

`git mv crates/temper-core/src/types/transfer.rs crates/temper-core/src/types/reassign.rs`, then replace its contents (delete `ResourceTransfer`, `TransferRequest`, `TransferStatus`, and their tests; keep `BulkReassignRequest`; add the new types). Mirror the derive attributes used on `CreateInvitationRequest` in `types/invitation.rs` (ts-rs + web-api ToSchema). Result:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// API request to reassign a single resource's owner (resource id is in the path).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignResourceRequest {
    pub to_profile_id: Uuid,
}

/// API response acknowledging a single reassignment.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignAck {
    pub resource_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API request for bulk team reassignment (from_profile → to_profile).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignRequest {
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API response acknowledging a bulk reassignment.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignAck {
    pub resource_ids: Vec<Uuid>,
}
```

(Confirm which of `typescript`/`web-api` feature names + derive macros `CreateInvitationRequest` actually uses, and match exactly. If `invitation.rs` gates ToSchema differently, copy that.)

- [ ] **Step 2: Update the module re-export**

In `crates/temper-core/src/types/mod.rs`, replace the `transfer` module declaration/re-export with:

```rust
pub use reassign::{BulkReassignAck, BulkReassignRequest, ReassignAck, ReassignResourceRequest};
```

Also update the `mod transfer;` / `mod reassign;` declaration line. Grep for any remaining `transfer::` / `ResourceTransfer` / `TransferRequest` / `TransferStatus` references across the workspace and remove/adjust:

```bash
grep -rn "ResourceTransfer\|TransferRequest\|TransferStatus\|types::transfer\|transfer::" crates/ --include=*.rs
```
Expected after edits: no hits except possibly the DB enum name in migrations (leave those — table drop is #6).

- [ ] **Step 3: Regenerate TS types + verify**

```bash
cargo make generate-ts-types
cargo make check
```
Expected: `reassign.ts` generated; no dangling `transfer.ts` import breaks; check passes.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/ packages/
git commit -m "$(cat <<'EOF'
refactor(reassign): retire dead transfer types, add reassign wire types

Deletes ResourceTransfer/TransferRequest/TransferStatus (offer/accept dropped);
keeps BulkReassignRequest. kb_transfers table DROP deferred to task #6.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: API handlers + routes + OpenAPI

**Files:**
- Create: `crates/temper-api/src/handlers/reassign.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs` (add `pub mod reassign;`)
- Modify: `crates/temper-api/src/routes.rs` (add two gated routes)
- Modify: `crates/temper-api/src/openapi.rs` (register paths + schemas)

**Interfaces:**
- Consumes: `reassign_service::{reassign_resource, reassign_team_resources}`; wire types from Task 4; `AuthUser`, `AppState`.
- Produces: `POST /api/resources/{id}/reassign`, `POST /api/teams/{id}/reassign`.

- [ ] **Step 1: Write the handlers**

Create `crates/temper-api/src/handlers/reassign.rs` (mirror `handlers/invitations.rs` shape):

```rust
//! Resource ownership reassignment handlers — thin: extract `AuthUser`, dispatch one
//! `reassign_service` call. Service-direct, same precedent as `invitations`.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::reassign::{
    BulkReassignAck, BulkReassignRequest, ReassignAck, ReassignResourceRequest,
};
use temper_services::error::ApiResult;
use temper_services::services::reassign_service;
use temper_services::state::AppState;

#[utoipa::path(
    post,
    path = "/api/resources/{id}/reassign",
    tag = "Reassign",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = ReassignResourceRequest,
    responses(
        (status = 200, description = "Owner reassigned", body = ReassignAck),
        (status = 403, description = "Forbidden (not owner, or admin reach not satisfied)"),
        (status = 404, description = "Resource has no home / not found"),
    )
)]
pub async fn reassign_resource(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(body): Json<ReassignResourceRequest>,
) -> ApiResult<Json<ReassignAck>> {
    reassign_service::reassign_resource(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        resource_id,
        body.to_profile_id,
    )
    .await?;
    Ok(Json(ReassignAck {
        resource_id,
        to_profile_id: body.to_profile_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/teams/{id}/reassign",
    tag = "Reassign",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = BulkReassignRequest,
    responses(
        (status = 200, description = "Team resources reassigned", body = BulkReassignAck),
        (status = 403, description = "Forbidden (caller does not manage the team, or target not a member)"),
    )
)]
pub async fn reassign_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<BulkReassignRequest>,
) -> ApiResult<Json<BulkReassignAck>> {
    let ids = reassign_service::reassign_team_resources(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        body.from_profile_id,
        body.to_profile_id,
    )
    .await?;
    Ok(Json(BulkReassignAck { resource_ids: ids }))
}
```

- [ ] **Step 2: Register the handler module + routes**

`handlers/mod.rs`: add `pub mod reassign;`. In `routes.rs`, add to the **gated** router (alongside the resource + team routes):

```rust
        .route(
            "/api/resources/{id}/reassign",
            post(handlers::reassign::reassign_resource),
        )
        .route(
            "/api/teams/{id}/reassign",
            post(handlers::reassign::reassign_team),
        )
```

- [ ] **Step 3: Register OpenAPI paths + schemas**

In `openapi.rs`, add to the `paths(...)`: `crate::handlers::reassign::reassign_resource,` and `crate::handlers::reassign::reassign_team,`. Add to `components(schemas(...))`: `temper_core::types::reassign::ReassignResourceRequest`, `ReassignAck`, `BulkReassignRequest`, `BulkReassignAck`. Extend the openapi assertion test (mirroring the invitation one, ~openapi.rs:221) with `assert!(json.contains("/api/resources/{id}/reassign"));` and `assert!(json.contains("/api/teams/{id}/reassign"));`.

- [ ] **Step 4: Verify + commit**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make check
cargo nextest run -p temper-api --features test-db --test openapi 2>/dev/null || cargo nextest run -p temper-api openapi
git add crates/temper-api/
git commit -m "$(cat <<'EOF'
feat(reassign): API handlers + gated routes for single + bulk reassign

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `temper-client` methods

**Files:**
- Modify: `crates/temper-client/src/resources.rs` (single reassign)
- Modify: `crates/temper-client/src/teams.rs` (bulk reassign)

**Interfaces:**
- Consumes: wire types from Task 4; the HTTP client (`self.http`/`post` helper — mirror existing methods in each file).
- Produces: `resources().reassign(id: Uuid, req: &ReassignResourceRequest) -> Result<ReassignAck>`; `teams().reassign(team_id: Uuid, req: &BulkReassignRequest) -> Result<BulkReassignAck>`.

- [ ] **Step 1: Add the resources client method**

In `crates/temper-client/src/resources.rs`, mirror an existing POST method (e.g. how `teams.rs` `invite` posts). Add:

```rust
    /// Reassign a resource's owner. POST /api/resources/{id}/reassign.
    pub async fn reassign(
        &self,
        id: uuid::Uuid,
        req: &temper_core::types::reassign::ReassignResourceRequest,
    ) -> Result<temper_core::types::reassign::ReassignAck> {
        self.http
            .post_json(&format!("/api/resources/{id}/reassign"), req)
            .await
    }
```

(Use the exact HTTP-helper method name + signature the sibling methods in this file use — grep the file for `post_json` / `post` and match it, including how the base URL + auth header are applied.)

- [ ] **Step 2: Add the teams client method**

In `crates/temper-client/src/teams.rs`, next to `invite`:

```rust
    /// Bulk-reassign a team's resources. POST /api/teams/{id}/reassign.
    pub async fn reassign(
        &self,
        team_id: uuid::Uuid,
        req: &temper_core::types::reassign::BulkReassignRequest,
    ) -> Result<temper_core::types::reassign::BulkReassignAck> {
        self.http
            .post_json(&format!("/api/teams/{team_id}/reassign"), req)
            .await
    }
```

- [ ] **Step 3: Verify + commit**

```bash
cargo make check
git add crates/temper-client/
git commit -m "$(cat <<'EOF'
feat(reassign): temper-client reassign methods

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: CLI commands

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (resource + team subcommand enums)
- Modify: `crates/temper-cli/src/commands/resource.rs` (dispatch + action)
- Modify: `crates/temper-cli/src/commands/team.rs` (dispatch + action)

**Interfaces:**
- Consumes: `client.resources().reassign`, `client.teams().reassign`; `parse_ref` for the `<ref>`.
- Produces: `temper resource reassign <ref> --to <uuid>`; `temper team reassign <team> --from <uuid> --to <uuid>`.

- [ ] **Step 1: Add the CLI subcommand variants**

In `cli.rs`, add to the resource subcommand enum (mirror the existing `Delete`/`Update` variants' clap attrs):

```rust
    /// Reassign a resource's owner (mis-attribution self-fix, or team-admin over scope).
    Reassign {
        /// Resource ref (UUID or decorated slug-uuid).
        r#ref: String,
        /// Recipient profile UUID.
        #[arg(long)]
        to: String,
    },
```

And to the team subcommand enum:

```rust
    /// Bulk-reassign a departing member's team-scoped resources (offboarding).
    Reassign {
        /// Team slug or UUID.
        team: String,
        /// Current owner (departing) profile UUID.
        #[arg(long)]
        from: String,
        /// New owner profile UUID (must be a team member).
        #[arg(long)]
        to: String,
    },
```

- [ ] **Step 2: Add the resource dispatch + action**

In `commands/resource.rs`, add a match arm for `Reassign { r#ref, to }` (mirror the `Delete`/`Update` arms — build the client, resolve fmt), calling a new action:

```rust
/// Reassign a resource's owner via the API.
pub async fn reassign_remote(
    client: &temper_client::TemperClient,
    r#ref: &str,
    to: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let id = temper_workflow::operations::parse_ref(r#ref)
        .map_err(|e| TemperError::Api(format!("invalid ref '{}': {e}", r#ref)))?;
    let to_profile_id = uuid::Uuid::parse_str(to)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{to}': {e}")))?;
    let req = temper_core::types::reassign::ReassignResourceRequest { to_profile_id };
    let ack = client
        .resources()
        .reassign(id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&ack, fmt)?);
    Ok(())
}
```

(Confirm `parse_ref`'s return type + how other resource commands turn a `<ref>` into a `Uuid` — match that exact call; some surfaces use a `ResourceRef`/`parse_ref(...).uuid()` shape.)

- [ ] **Step 3: Add the team dispatch + action**

In `commands/team.rs`, add (mirror `remove_member_remote`, which already parses profile UUIDs + resolves the team via `resolve_team_id`):

```rust
/// Bulk-reassign a departing member's team-scoped resources (owner/maintainer).
pub async fn reassign_remote(
    client: &temper_client::TemperClient,
    team: &str,
    from: &str,
    to: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let from_profile_id = uuid::Uuid::parse_str(from)
        .map_err(|e| TemperError::Api(format!("invalid from id '{from}': {e}")))?;
    let to_profile_id = uuid::Uuid::parse_str(to)
        .map_err(|e| TemperError::Api(format!("invalid to id '{to}': {e}")))?;
    let req = temper_core::types::reassign::BulkReassignRequest { from_profile_id, to_profile_id };
    let ack = client
        .teams()
        .reassign(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&ack, fmt)?);
    Ok(())
}
```

Wire both new subcommand variants to these functions in their respective command dispatchers (grep `resource.rs` / `team.rs` for the `match` over the subcommand enum and add the arms).

- [ ] **Step 4: Rebuild the CLI bin + smoke-check help**

The local e2e uses a prebuilt bin, so rebuild before Task 8.

```bash
cargo build -p temper-cli --bin temper
./target/debug/temper resource reassign --help
./target/debug/temper team reassign --help
cargo make check
```
Expected: help shows the new flags; check passes.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/
git commit -m "$(cat <<'EOF'
feat(reassign): CLI `resource reassign` + `team reassign`

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: E2E — CLI → API → DB owner change

**Files:**
- Create: `tests/e2e/tests/reassign_test.rs`

**Interfaces:**
- Consumes: the e2e harness in `tests/e2e/tests/common/` (real Axum server + Postgres + JWT fixtures + `temper` bin driver). Mirror an existing team/resource e2e test's setup.

- [ ] **Step 1: Write the e2e test**

Create `tests/e2e/tests/reassign_test.rs`. Mirror the closest existing e2e (grep `tests/e2e/tests/` for a test that seeds two profiles + a resource and drives `temper resource ...`). The test must:

```rust
#![cfg(feature = "test-db")]
// Drive `temper resource reassign` end-to-end and assert the owner change lands
// in kb_resource_homes and flips resources_visible_to.
//
// Skeleton (fill server/auth/bin plumbing from common/ harness):
// 1. Start the harness (real Axum + test DB), authenticated as owner "alice".
// 2. Seed profile "bob" (+ his web emitter entity) and a resource homed in alice's context.
// 3. Run: temper resource reassign <resource-ref> --to <bob-uuid>  (as alice).
// 4. Assert exit 0.
// 5. Query the DB: owner_profile_id == bob; EXISTS in resources_visible_to(bob).
```

Follow the harness's exact API for spawning the server, minting a JWT for the acting profile, and invoking the `temper` bin. Keep it to the single-resource path (bulk is covered by service tests).

- [ ] **Step 2: Regenerate e2e cache + run**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo build -p temper-cli --bin temper   # e2e uses the prebuilt bin
cargo make prepare-e2e
cargo make test-e2e
```
Expected: the new test PASSES alongside the suite.

- [ ] **Step 3: Full verification + commit**

```bash
cargo make check
cargo make test           # unit
cargo make test-db        # integration (services + api)
git add tests/e2e/ .sqlx/ tests/e2e/.sqlx/
git commit -m "$(cat <<'EOF'
test(reassign): e2e CLI->API->DB owner reassignment

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final Verification (before PR)

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up
cargo make check
cargo nextest run --workspace --exclude temper-cloud   # avoid ort feature-unification (see CLAUDE.md)
cargo make test-db
cargo make test-e2e
```

Confirm: no `ResourceTransfer`/`TransferRequest`/`TransferStatus` references remain (`grep -rn` clean); `.sqlx/` caches committed (workspace + `crates/temper-services/.sqlx` + `crates/temper-api/.sqlx` + `tests/e2e/.sqlx`); `reassign.ts` present under the TS types output.

## Self-Review Notes (spec → task coverage)

- Event-sourced owner change → Task 1. Owner + constrained-admin auth → Task 2. Bulk offboarding scope → Task 3. Dead-type retirement (types only; table→#6) → Task 4. API surface (gated) → Task 5. Client → Task 6. CLI (UUID recipients) → Task 7. E2E visibility assertion → Task 8. Cogmap-homed exclusion: the **admin path** and **bulk scope** exclude it structurally (their queries join `anchor_table='kb_contexts'`), but the **owner path** and the raw write primitive do not — so it is closed explicitly by (a) a service guard in `reassign_resource` (rejects non-`kb_contexts` homes with `BadRequest`, covering the owner path) and (b) a `RAISE EXCEPTION` backstop in the `resource_reassign` SQL fn. Test: `cannot_reassign_cogmap_homed_resource_even_as_owner` (Task 2).
