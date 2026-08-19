# Invocation Envelope Surfacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the already-built invocation envelope (`invocation_open` / `invocation_close`) on all three surfaces (API + MCP + CLI), making "trace agentic flows" reachable.

**Architecture:** Follows the existing `relationship_assert` vertical exactly: a shared command in `temper-core` → a `Backend` trait method → the `NextBackend` impl (resolve, auth, dispatch) → a thin `temper-next` `writes::` wrapper → the already-built `SeedAction` / `fire` / SQL functions. The three surfaces diverge only in transport/parsing and converge on the shared command via `select_backend`.

**Tech Stack:** Rust workspace (temper-core / temper-next / temper-api / temper-mcp / temper-cli / temper-client), async-trait, sqlx (temper_next artifact namespace), clap v4, rmcp (`#[tool]`), axum, utoipa, cargo-make + cargo-nextest.

## Global Constraints

- **Precondition:** WS6 chunk-5 flip is the live path; `NextBackend` is real, legacy `DbBackend` is dead. Implementation is gated on the sibling neon-branch verification / flip landing — **do not start until that is in.**
- **Typed structs over inline JSON** — every wire/command shape is a struct, never `serde_json::json!()` (except the opaque `outcome` payload, which is intentionally `serde_json::Value`).
- **Shared types at boundaries** — wire types live in `temper-core`; both API and client reuse them. Cross-surface enums (`Disposition`) live in `temper-core` and map to `temper_next::payloads::Disposition` in `NextBackend` (the `map_edge_kind`/`map_polarity` pattern).
- **Service/substrate owns SQL; surfaces dispatch through the `Backend` trait for writes.** Never inline `sqlx` in a surface.
- **Auth before writes** — `NextBackend` checks `cogmap_readable_by_profile` before any fire. `invocation_open` SQL enforces ONLY the delegation gate (team-sharing), not profile access — the backend must add the profile gate.
- **MCP enum params must inline** — `Disposition` carries `#[cfg_attr(feature = "mcp", schemars(inline))]` (a `$ref` enum reaches Anthropic tool-use as `null`).
- **temper-next sqlx macros target the `temper_next` namespace** — after adding any `sqlx::query!`/`query_scalar!` in temper-next, regenerate the per-crate cache with `cargo make prepare-next` (never `cargo sqlx prepare --workspace`).
- **All `cargo make` tasks force `SQLX_OFFLINE=true`** — `cargo make check` is the honest local probe of the committed caches.

---

## File Structure

**temper-core (commands + wire types):**
- Create `crates/temper-core/src/types/invocation.rs` — `Disposition` enum (cross-surface).
- Create `crates/temper-core/src/types/invocation_requests.rs` — `OpenInvocationRequest`, `CloseInvocationRequest`, `InvocationAck`.
- Modify `crates/temper-core/src/types/mod.rs` — register the two modules.
- Modify `crates/temper-core/src/operations/commands.rs` — `OpenInvocation`, `CloseInvocation`.
- Modify `crates/temper-core/src/operations/backend.rs` — two trait methods + imports.

**temper-next (substrate glue):**
- Modify `crates/temper-next/src/writes.rs` — `OpenParams`, `open_invocation`, `close_invocation`.

**temper-api (backend impls + HTTP surface):**
- Modify `crates/temper-api/src/backend/next_backend.rs` — real impls + `map_disposition`.
- Modify `crates/temper-api/src/backend/db_backend.rs` — `NotImplemented` stubs.
- Modify `crates/temper-api/src/handlers/edges.rs` (or new `handlers/invocations.rs`) — `open` / `close` handlers.
- Modify `crates/temper-api/src/routes.rs` — two routes.

**temper-mcp (MCP surface):**
- Create `crates/temper-mcp/src/tools/invocations.rs` — input structs + tool handlers.
- Modify `crates/temper-mcp/src/tools/mod.rs` — `pub mod invocations;`.
- Modify `crates/temper-mcp/src/service.rs` — two `#[tool]` registrations.

**temper-client + temper-cli (CLI surface):**
- Create `crates/temper-client/src/invocations.rs` — `InvocationClient`.
- Modify `crates/temper-client/src/lib.rs` — module + `invocations()` accessor.
- Modify `crates/temper-cli/src/cli.rs` — `Commands::Invocation` + `InvocationAction` + `CliDisposition`.
- Create `crates/temper-cli/src/commands/invocation.rs` — dispatch.
- Modify `crates/temper-cli/src/commands/mod.rs` — `pub mod invocation;`.
- Modify `crates/temper-cli/src/main.rs` — dispatch arm.

---

## Task 1: temper-core — Disposition, commands, and wire types

**Files:**
- Create: `crates/temper-core/src/types/invocation.rs`
- Create: `crates/temper-core/src/types/invocation_requests.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-core/src/operations/commands.rs`
- Test: inline `#[cfg(test)]` in `invocation.rs` and `commands.rs`

**Interfaces:**
- Produces: `temper_core::types::invocation::Disposition { Completed, Failed, Abandoned }`;
  `temper_core::types::invocation_requests::{OpenInvocationRequest, CloseInvocationRequest, InvocationAck}`;
  `temper_core::operations::commands::{OpenInvocation, CloseInvocation}` (re-exported via `operations`).

- [ ] **Step 1: Write the failing test** — append to a new `crates/temper-core/src/types/invocation.rs`:

```rust
//! Cross-surface invocation types. `Disposition` mirrors
//! `temper_next::payloads::Disposition`; `NextBackend` maps between them
//! (the `map_edge_kind` pattern) since `temper-core` does not depend on
//! `temper-next`.

use serde::{Deserialize, Serialize};

/// Terminal outcome of an invocation. Mirrors the Postgres / temper-next
/// `Disposition`. `open` is NOT representable here — closing requires a
/// terminal value.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline into MCP input schemas — Anthropic tool-use does not resolve `$ref`.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Failed,
    Abandoned,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Disposition::Completed).unwrap(),
            serde_json::json!("completed")
        );
        let back: Disposition = serde_json::from_value(serde_json::json!("abandoned")).unwrap();
        assert_eq!(back, Disposition::Abandoned);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core disposition_serializes_snake_case`
Expected: FAIL — `invocation.rs` module not declared (compile error: unresolved module).

- [ ] **Step 3: Register the module + add the wire types**

In `crates/temper-core/src/types/mod.rs`, add (alongside the other `pub mod` lines):

```rust
pub mod invocation;
pub mod invocation_requests;
```

Create `crates/temper-core/src/types/invocation_requests.rs`:

```rust
//! Wire types for the `/api/invocations` endpoints. Shared between
//! `temper-api` (OpenAPI source) and `temper-client` (typed request builder).
//!
//! Cogmap/entity ids are raw temper_next UUIDs, not resource refs: cogmaps and
//! entities are not resource-addressable. They come from the agent's launch /
//! delegation context, not `parse_ref`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::invocation::Disposition;

/// Request body for `POST /api/invocations`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct OpenInvocationRequest {
    /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`).
    pub trigger_kind: String,
    /// The cogmap the invocation operates on (temper_next cogmap id).
    pub originating_cogmap: Uuid,
    /// Optional delegating-parent cogmap (must share a team with the originating
    /// cogmap — enforced by the substrate delegation gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_cogmap: Option<Uuid>,
}

/// Request body for `POST /api/invocations/{id}/close`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct CloseInvocationRequest {
    pub disposition: Disposition,
    /// Opaque terminal outcome payload (agent-defined shape).
    #[serde(default)]
    pub outcome: serde_json::Value,
}

/// Acknowledgement returned by the open endpoint — carries the minted
/// invocation id, fed back into the close call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct InvocationAck {
    pub invocation_id: Uuid,
}
```

- [ ] **Step 4: Add the command structs**

In `crates/temper-core/src/operations/commands.rs`, after `FoldRelationship` (around line 156), add:

```rust
/// Open an invocation envelope — the trace primitive. `originating_cogmap` /
/// `parent_cogmap` are temper_next cogmap ids (not resource refs). The
/// invocation id is minted by the backend and returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenInvocation {
    pub trigger_kind: String,
    pub originating_cogmap: uuid::Uuid,
    pub parent_cogmap: Option<uuid::Uuid>,
    pub origin: Surface,
}

/// Close an invocation with a terminal disposition + opaque outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseInvocation {
    pub invocation: uuid::Uuid,
    pub disposition: crate::types::invocation::Disposition,
    pub outcome: serde_json::Value,
    pub origin: Surface,
}
```

Then verify the `operations` module re-exports them. In `crates/temper-core/src/operations/mod.rs`, the existing `pub use commands::{...}` list must include `OpenInvocation, CloseInvocation` (add them to the brace list).

- [ ] **Step 5: Add a command round-trip test**

Append to the `#[cfg(test)] mod tests` block in `commands.rs`:

```rust
    #[test]
    fn open_invocation_round_trips() {
        let cmd = OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: uuid::Uuid::now_v7(),
            parent_cogmap: None,
            origin: Surface::Mcp,
        };
        let v = serde_json::to_value(&cmd).unwrap();
        let back: OpenInvocation = serde_json::from_value(v).unwrap();
        assert_eq!(back, cmd);
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core --features web-api,mcp invocation && cargo nextest run -p temper-core open_invocation_round_trips disposition_serializes_snake_case`
Expected: PASS.

- [ ] **Step 7: Verify all feature combos compile (ts-rs / utoipa / schemars)**

Run: `cargo make check`
Expected: clean (fmt + clippy + docs).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/invocation.rs crates/temper-core/src/types/invocation_requests.rs crates/temper-core/src/types/mod.rs crates/temper-core/src/operations/commands.rs crates/temper-core/src/operations/mod.rs
git commit -m "feat(temper-core): invocation envelope commands + wire types"
```

---

## Task 2: temper-next — writes wrappers (real DB proof)

**Files:**
- Modify: `crates/temper-next/src/writes.rs`
- Test: `crates/temper-next/tests/invocation_envelope.rs` (extend — it already has the `setup`/`genesis`/`system_actor` harness)

**Interfaces:**
- Consumes: existing `SeedAction::InvocationOpen/Close`, `fire`, `begin_scoped`, `CogmapId`, `EntityId`, `InvocationId`, `payloads::Disposition`.
- Produces:
  - `writes::OpenParams { trigger_kind: String, originating: CogmapId, parent: Option<CogmapId>, scoped_entity: EntityId, emitter: EntityId }`
  - `writes::open_invocation(pool: &PgPool, p: OpenParams) -> anyhow::Result<InvocationId>`
  - `writes::close_invocation(pool: &PgPool, invocation: InvocationId, originating: CogmapId, disposition: payloads::Disposition, outcome: serde_json::Value, emitter: EntityId) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing test** — append to `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#[tokio::test]
async fn writes_open_then_close_round_trips() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-writes").await;

    let inv = temper_next::writes::open_invocation(
        &pool,
        temper_next::writes::OpenParams {
            trigger_kind: "manual".to_string(),
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM kb_invocations WHERE id=$1")
        .bind(inv.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "open");

    temper_next::writes::close_invocation(
        &pool,
        inv,
        cog,
        temper_next::payloads::Disposition::Completed,
        serde_json::json!({"concepts": 2}),
        emitter,
    )
    .await
    .unwrap();

    let (status, closed): (String, bool) =
        sqlx::query_as("SELECT status, closed_at IS NOT NULL FROM kb_invocations WHERE id=$1")
            .bind(inv.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert!(closed, "closed_at set");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests writes_open_then_close_round_trips`
Expected: FAIL — `writes::open_invocation` / `OpenParams` / `close_invocation` not found.

- [ ] **Step 3: Implement the wrappers** — add to `crates/temper-next/src/writes.rs` after the relationship writes section. (`CogmapId`, `InvocationId` may need adding to the existing `use crate::ids::{...}` import; `payloads` is already imported.)

```rust
// ── invocation envelope ──────────────────────────────────────────────────────────

/// Parameters for opening an invocation. The invocation id is minted here and
/// returned (server-mint v1; caller-supplied ids for byte-exact durable-resume
/// re-issue are a deferred runtime concern).
pub struct OpenParams {
    pub trigger_kind: String,
    pub originating: CogmapId,
    pub parent: Option<CogmapId>,
    pub scoped_entity: EntityId,
    pub emitter: EntityId,
}

/// Open an invocation envelope, returning the minted invocation id.
pub async fn open_invocation(pool: &PgPool, p: OpenParams) -> Result<InvocationId> {
    let invocation = InvocationId::from(Uuid::now_v7());
    let mut tx = begin_scoped(pool).await?;
    let opened = fire(
        &mut tx,
        SeedAction::InvocationOpen {
            invocation,
            trigger_kind: &p.trigger_kind,
            originating: p.originating,
            parent: p.parent,
            scoped_entity: p.scoped_entity,
            emitter: p.emitter,
        },
    )
    .await?
    .invocation()?;
    tx.commit().await?;
    Ok(opened)
}

/// Close an invocation with a terminal disposition + opaque outcome. The
/// originating cogmap is supplied by the caller (it knows it from the open /
/// from an auth lookup) so the `SeedAction` is constructed truthfully; the
/// substrate ignores it on close but the typed action requires it.
pub async fn close_invocation(
    pool: &PgPool,
    invocation: InvocationId,
    originating: CogmapId,
    disposition: payloads::Disposition,
    outcome: serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire(
        &mut tx,
        SeedAction::InvocationClose {
            invocation,
            disposition,
            outcome,
            originating,
            emitter,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
```

If `Uuid` / `CogmapId` / `InvocationId` are not already in scope in `writes.rs`, add them to the existing imports (`use crate::ids::{... CogmapId, InvocationId};` and `use uuid::Uuid;`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests writes_open_then_close_round_trips`
Expected: PASS. (Requires the temper_next artifact loaded + Docker Postgres; this is the `temper-next-write` group.)

- [ ] **Step 5: Regenerate the temper-next sqlx cache (no new macros expected, but confirm)**

The wrappers call `fire` (whose `invocation_open`/`invocation_close` macros already exist in the cache) and add no new `query!` macros, so the cache should be unchanged. Confirm:

Run: `cargo make prepare-next && git status --short crates/temper-next/.sqlx`
Expected: no changes to `.sqlx`. If anything changed, include it in the commit.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/writes.rs crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): writes::open_invocation / close_invocation wrappers"
```

---

## Task 3: Backend trait methods + NextBackend impls + DbBackend stubs

**Files:**
- Modify: `crates/temper-core/src/operations/backend.rs`
- Modify: `crates/temper-api/src/backend/next_backend.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Test: inline unit test for `map_disposition` in `next_backend.rs`

**Interfaces:**
- Consumes: `OpenInvocation`, `CloseInvocation` (Task 1); `writes::OpenParams`, `writes::open_invocation`, `writes::close_invocation` (Task 2).
- Produces: `Backend::open_invocation(cmd) -> CommandOutput<Uuid>`, `Backend::close_invocation(cmd) -> CommandOutput<()>` (the invocation id is returned as a backend-opaque `Uuid`, mirroring the edge-handle precedent — `temper-core` cannot name `temper_next::InvocationId`).

> Trait method + both impls land in ONE task: a trait method with no default breaks every impl until implemented, so they form a single compile unit.

- [ ] **Step 1: Write the failing test** — add to the `#[cfg(test)] mod tests` in `next_backend.rs`:

```rust
    #[test]
    fn map_disposition_covers_all_variants() {
        use temper_core::types::invocation::Disposition as C;
        use temper_next::payloads::Disposition as N;
        assert!(matches!(map_disposition(C::Completed), N::Completed));
        assert!(matches!(map_disposition(C::Failed), N::Failed));
        assert!(matches!(map_disposition(C::Abandoned), N::Abandoned));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db,next-backend map_disposition_covers_all_variants`
Expected: FAIL — `map_disposition` not found.

- [ ] **Step 3: Add the trait methods**

In `crates/temper-core/src/operations/backend.rs`, extend the `use super::commands::{...}` import with `CloseInvocation, OpenInvocation`, and add to the `Backend` trait (after `fold_relationship`):

```rust
    // ── invocation envelope (cognitive-map surfacing, first slice) ──
    // Returns the invocation id as a backend-opaque `Uuid` (temper-core cannot
    // name temper_next::InvocationId), mirroring the edge-handle return.

    async fn open_invocation(
        &self,
        cmd: OpenInvocation,
    ) -> Result<CommandOutput<Uuid>, TemperError>;

    async fn close_invocation(&self, cmd: CloseInvocation) -> Result<CommandOutput<()>, TemperError>;
```

- [ ] **Step 4: Implement on NextBackend**

In `crates/temper-api/src/backend/next_backend.rs`: add `CloseInvocation, OpenInvocation` to the `temper_core::operations::{...}` import. Add the mapper near `map_polarity`:

```rust
/// temper-core Disposition → temper-next payload Disposition (1:1).
fn map_disposition(d: temper_core::types::invocation::Disposition) -> temper_next::payloads::Disposition {
    use temper_core::types::invocation::Disposition as C;
    use temper_next::payloads::Disposition as N;
    match d {
        C::Completed => N::Completed,
        C::Failed => N::Failed,
        C::Abandoned => N::Abandoned,
    }
}
```

Add the two impls inside the `impl Backend for NextBackend` block (after `fold_relationship`):

```rust
    async fn open_invocation(
        &self,
        cmd: OpenInvocation,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        // Auth before any write: invocation_open SQL enforces only the delegation
        // gate (team-sharing), NOT profile access — gate it here.
        let readable: bool =
            sqlx::query_scalar("SELECT temper_next.cogmap_readable_by_profile($1, $2)")
                .bind(owner.uuid())
                .bind(cmd.originating_cogmap)
                .fetch_one(&self.pool)
                .await
                .map_err(api_err)?;
        if !readable {
            return Err(TemperError::NotFound(format!(
                "cogmap {} not found",
                cmd.originating_cogmap
            )));
        }
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let inv = writes::open_invocation(
            &self.pool,
            writes::OpenParams {
                trigger_kind: cmd.trigger_kind,
                originating: temper_next::ids::CogmapId::from(cmd.originating_cogmap),
                parent: cmd.parent_cogmap.map(temper_next::ids::CogmapId::from),
                // v1: the invocation is scoped to the acting entity itself.
                scoped_entity: emitter,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(inv.uuid()))
    }

    async fn close_invocation(
        &self,
        cmd: CloseInvocation,
    ) -> Result<CommandOutput<()>, TemperError> {
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        // Resolve the invocation's originating cogmap (also the auth subject).
        let originating: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT originating_cogmap_id FROM temper_next.kb_invocations WHERE id = $1",
        )
        .bind(cmd.invocation)
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?;
        let originating = originating.ok_or_else(|| {
            TemperError::NotFound(format!("invocation {} not found", cmd.invocation))
        })?;
        let readable: bool =
            sqlx::query_scalar("SELECT temper_next.cogmap_readable_by_profile($1, $2)")
                .bind(owner.uuid())
                .bind(originating)
                .fetch_one(&self.pool)
                .await
                .map_err(api_err)?;
        if !readable {
            return Err(TemperError::Forbidden);
        }
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::close_invocation(
            &self.pool,
            temper_next::ids::InvocationId::from(cmd.invocation),
            temper_next::ids::CogmapId::from(originating),
            map_disposition(cmd.disposition),
            cmd.outcome,
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(()))
    }
```

- [ ] **Step 5: Implement NotImplemented stubs on DbBackend**

In `crates/temper-api/src/backend/db_backend.rs`: add `CloseInvocation, OpenInvocation` to its `temper_core::operations::{...}` import, then add inside `impl Backend for DbBackend` (the legacy path is dead post-flip; the envelope is temper_next-only):

```rust
    async fn open_invocation(
        &self,
        _cmd: OpenInvocation,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        Err(TemperError::NotImplemented(
            "invocation envelope is only supported on the temper_next backend".to_string(),
        ))
    }

    async fn close_invocation(
        &self,
        _cmd: CloseInvocation,
    ) -> Result<CommandOutput<()>, TemperError> {
        Err(TemperError::NotImplemented(
            "invocation envelope is only supported on the temper_next backend".to_string(),
        ))
    }
```

> If `cargo make check` reports any OTHER `Backend` impl (e.g. a test mock) missing these methods, add the same `NotImplemented` stub there.

- [ ] **Step 6: Run the unit test + full check**

Run: `cargo nextest run -p temper-api --features test-db,next-backend map_disposition_covers_all_variants && cargo make check`
Expected: PASS + clean. (`map_disposition`'s test confirms the mapping; the live open/close path is proven at the writes layer in Task 2 and end-to-end in Task 7. NextBackend straddles `public` + `temper_next`, so a backend-level live test needs the cross-schema harness — covered by Task 7.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/operations/backend.rs crates/temper-api/src/backend/next_backend.rs crates/temper-api/src/backend/db_backend.rs
git commit -m "feat(backend): open/close invocation on the Backend trait (NextBackend real, DbBackend stub)"
```

---

## Task 4: MCP surface — invocation tools

**Files:**
- Create: `crates/temper-mcp/src/tools/invocations.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs`
- Test: inline deserialize test in `invocations.rs`

**Interfaces:**
- Consumes: `Backend::open_invocation/close_invocation`, `select_backend`, `OpenInvocation/CloseInvocation`, `InvocationAck`, `Disposition`.
- Produces: MCP tools `invocation_open`, `invocation_closed`.

- [ ] **Step 1: Write the failing test** — create `crates/temper-mcp/src/tools/invocations.rs`:

```rust
//! Invocation tools — open and close the agent-trace envelope. Each mirrors one
//! HTTP endpoint in `temper-api/src/handlers/invocations.rs` and dispatches
//! through `select_backend` (the same write path the HTTP handlers use).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::select_backend;
use temper_core::error::TemperError;
use temper_core::operations::{CloseInvocation, OpenInvocation, Surface};
use temper_core::types::invocation::Disposition;
use temper_core::types::invocation_requests::InvocationAck;
use temper_core::types::ids::ProfileId;

use crate::service::TemperMcpService;

/// MCP input for invocation_open.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenInvocationInput {
    /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`).
    pub trigger_kind: String,
    /// The cogmap this invocation operates on (temper_next cogmap UUID).
    pub originating_cogmap: Uuid,
    /// Optional delegating-parent cogmap UUID (must share a team).
    pub parent_cogmap: Option<Uuid>,
}

/// MCP input for invocation_closed.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseInvocationInput {
    /// The invocation id returned by invocation_open.
    pub invocation_id: Uuid,
    /// Terminal disposition — one of `completed`, `failed`, `abandoned`.
    pub disposition: Disposition,
    /// Opaque terminal outcome payload (agent-defined shape).
    #[serde(default)]
    pub outcome: serde_json::Value,
}

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(_) => {
            rmcp::ErrorData::invalid_params(format!("{action}: not found"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: not permitted"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

pub async fn invocation_open(
    svc: &TemperMcpService,
    input: OpenInvocationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = OpenInvocation {
        trigger_kind: input.trigger_kind,
        originating_cogmap: input.originating_cogmap,
        parent_cogmap: input.parent_cogmap,
        origin: Surface::Mcp,
    };
    let backend = select_backend(
        svc.api_state.backend_selection,
        pool,
        profile_id,
        "mcp".to_string(),
        Surface::Mcp,
    )
    .map_err(|e| map_err(e, "select_backend"))?;
    let out = backend
        .open_invocation(cmd)
        .await
        .map_err(|e| map_err(e, "invocation_open"))?;
    let ack = InvocationAck {
        invocation_id: out.value,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn invocation_closed(
    svc: &TemperMcpService,
    input: CloseInvocationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = CloseInvocation {
        invocation: input.invocation_id,
        disposition: input.disposition,
        outcome: input.outcome,
        origin: Surface::Mcp,
    };
    let backend = select_backend(
        svc.api_state.backend_selection,
        pool,
        profile_id,
        "mcp".to_string(),
        Surface::Mcp,
    )
    .map_err(|e| map_err(e, "select_backend"))?;
    backend
        .close_invocation(cmd)
        .await
        .map_err(|e| map_err(e, "invocation_closed"))?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        "{\"status\":\"closed\"}".to_string(),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_input_deserializes() {
        let input: OpenInvocationInput = serde_json::from_value(serde_json::json!({
            "trigger_kind": "manual",
            "originating_cogmap": "019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        }))
        .unwrap();
        assert_eq!(input.trigger_kind, "manual");
        assert!(input.parent_cogmap.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-mcp open_input_deserializes`
Expected: FAIL — `tools::invocations` module not declared.

- [ ] **Step 3: Register the module + tools**

In `crates/temper-mcp/src/tools/mod.rs` add: `pub mod invocations;`

In `crates/temper-mcp/src/service.rs`, inside the `#[tool_router]` impl block (after `list_events`), add:

```rust
    #[tool(
        description = "Open an invocation envelope — the agent-trace primitive. Records that an agent run has begun, scoped to a cogmap. Returns an invocation_id to pass to invocation_closed and to correlate the run's authored acts."
    )]
    async fn invocation_open(
        &self,
        Parameters(input): Parameters<tools::invocations::OpenInvocationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_open(self, input).await
    }

    #[tool(
        description = "Close an invocation envelope with a terminal disposition (completed / failed / abandoned) and an opaque outcome payload. Use the invocation_id returned by invocation_open."
    )]
    async fn invocation_closed(
        &self,
        Parameters(input): Parameters<tools::invocations::CloseInvocationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_closed(self, input).await
    }
```

- [ ] **Step 4: Run test + check**

Run: `cargo nextest run -p temper-mcp open_input_deserializes && cargo make check`
Expected: PASS + clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-mcp/src/tools/invocations.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): invocation_open / invocation_closed tools"
```

---

## Task 5: API surface — handlers, routes, temper-client

**Files:**
- Modify: `crates/temper-api/src/handlers/edges.rs` (add `open_invocation` / `close_invocation` handlers; or a new `handlers/invocations.rs` + `handlers/mod.rs` decl — match the crate's handler-module convention)
- Modify: `crates/temper-api/src/routes.rs`
- Create: `crates/temper-client/src/invocations.rs`
- Modify: `crates/temper-client/src/lib.rs`
- Test: a route-registration assertion is impractical without AppState; this task's gate is `cargo make check` + the e2e in Task 7. Add a `temper-client` compile-level doc test is unnecessary.

**Interfaces:**
- Consumes: `OpenInvocation/CloseInvocation`, `select_backend`, `OpenInvocationRequest/CloseInvocationRequest/InvocationAck`.
- Produces: `POST /api/invocations`, `POST /api/invocations/{id}/close`; `client.invocations().open(&req)` / `.close(id, &req)`.

- [ ] **Step 1: Add the handlers** — in `crates/temper-api/src/handlers/edges.rs` (it already imports `select_backend`, `AuthUser`, `DeviceId`, `Surface`, `ProfileId`), add the request/ack imports and two handlers:

```rust
use temper_core::operations::{CloseInvocation, OpenInvocation};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};

#[utoipa::path(
    post,
    path = "/api/invocations",
    tag = "Invocations",
    security(("bearer_auth" = [])),
    request_body = OpenInvocationRequest,
    responses(
        (status = 200, description = "Invocation opened", body = InvocationAck),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Originating cogmap not found", body = ErrorBody),
    )
)]
pub async fn open_invocation(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Json(req): Json<OpenInvocationRequest>,
) -> ApiResult<Json<InvocationAck>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = OpenInvocation {
        trigger_kind: req.trigger_kind,
        originating_cogmap: req.originating_cogmap,
        parent_cogmap: req.parent_cogmap,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend.open_invocation(cmd).await.map_err(ApiError::from)?;
    Ok(Json(InvocationAck {
        invocation_id: out.value,
    }))
}

#[utoipa::path(
    post,
    path = "/api/invocations/{id}/close",
    tag = "Invocations",
    params(("id" = Uuid, Path, description = "Invocation ID")),
    security(("bearer_auth" = [])),
    request_body = CloseInvocationRequest,
    responses(
        (status = 200, description = "Invocation closed"),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Not permitted", body = ErrorBody),
        (status = 404, description = "Invocation not found", body = ErrorBody),
    )
)]
pub async fn close_invocation(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(invocation_id): Path<Uuid>,
    Json(req): Json<CloseInvocationRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = CloseInvocation {
        invocation: invocation_id,
        disposition: req.disposition,
        outcome: req.outcome,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    backend.close_invocation(cmd).await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status": "closed"})))
}
```

- [ ] **Step 2: Register the routes** — in `crates/temper-api/src/routes.rs`, add to the authenticated router (next to the `/api/relationships` routes):

```rust
        .route("/api/invocations", post(handlers::edges::open_invocation))
        .route(
            "/api/invocations/{id}/close",
            post(handlers::edges::close_invocation),
        )
```

If `routes.rs` (or `openapi.rs`) has a `#[derive(OpenApi)] … paths(...)` list, add `handlers::edges::open_invocation, handlers::edges::close_invocation` to `paths(...)` and `OpenInvocationRequest, CloseInvocationRequest, InvocationAck` to the `components(schemas(...))` list.

- [ ] **Step 3: Add the temper-client sub-client** — create `crates/temper-client/src/invocations.rs`:

```rust
//! Typed sub-client for the `/api/invocations` write endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};

/// Sub-client for invocation open/close operations.
pub struct InvocationClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for InvocationClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationClient").finish_non_exhaustive()
    }
}

impl<'a> InvocationClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/invocations — open an invocation, returning its id.
    pub async fn open(&self, request: &OpenInvocationRequest) -> Result<InvocationAck> {
        let token = self.http.resolve_token()?;
        let path = "/api/invocations";
        let req = self.http.post(path).json(request);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// POST /api/invocations/{id}/close — close an invocation.
    pub async fn close(
        &self,
        invocation_id: Uuid,
        request: &CloseInvocationRequest,
    ) -> Result<serde_json::Value> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invocations/{invocation_id}/close");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}
```

In `crates/temper-client/src/lib.rs`: add `mod invocations;` + `pub use invocations::InvocationClient;` (mirror the `relationships` lines), and add an accessor on the client struct mirroring `relationships()`:

```rust
    /// Sub-client for invocation open/close.
    pub fn invocations(&self) -> InvocationClient<'_> {
        InvocationClient::new(&self.http)
    }
```

(Match the exact field/path the existing `relationships()` accessor uses — locate it in `lib.rs` and copy its shape.)

- [ ] **Step 4: Verify it compiles**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Regenerate the API sqlx cache (handlers add no macros, but the ritual covers test targets)**

The new handlers add no `sqlx` macros (all SQL is in NextBackend, which uses runtime `query_scalar` for the new lookups). Confirm no cache drift:

Run: `cargo make prepare-api && git status --short crates/temper-api/.sqlx`
Expected: no changes. Include any that appear.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/edges.rs crates/temper-api/src/routes.rs crates/temper-client/src/invocations.rs crates/temper-client/src/lib.rs
git commit -m "feat(api): /api/invocations open + close endpoints + temper-client sub-client"
```

---

## Task 6: CLI surface — `temper invocation open|close`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Create: `crates/temper-cli/src/commands/invocation.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Test: clap parse test in `invocation.rs`

**Interfaces:**
- Consumes: `client.invocations().open/close` (Task 5); `OpenInvocationRequest/CloseInvocationRequest`; `Disposition`.
- Produces: `temper invocation open --trigger-kind <s> --cogmap <uuid> [--parent <uuid>]`; `temper invocation close <invocation_id> --disposition <completed|failed|abandoned> [--outcome <json>]`.

- [ ] **Step 1: Add the clap types** — in `crates/temper-cli/src/cli.rs`, add a CLI-local disposition enum (near `CliEdgeKind`):

```rust
/// CLI-local enum mirroring `Disposition` for clap `value_enum` parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum CliDisposition {
    Completed,
    Failed,
    Abandoned,
}
```

Add a `Commands` variant (near `Edge`):

```rust
    /// Open / close invocation envelopes (agent-trace primitive).
    Invocation {
        #[command(subcommand)]
        action: InvocationAction,
    },
```

Add the subcommand enum (near `EdgeAction`):

```rust
#[derive(Debug, clap::Subcommand)]
pub enum InvocationAction {
    /// Open an invocation, printing the minted invocation id.
    Open {
        #[arg(long = "trigger-kind")]
        trigger_kind: String,
        /// Originating cogmap UUID.
        #[arg(long = "cogmap")]
        cogmap: uuid::Uuid,
        /// Optional delegating-parent cogmap UUID.
        #[arg(long = "parent")]
        parent: Option<uuid::Uuid>,
    },
    /// Close an invocation with a terminal disposition.
    Close {
        /// Invocation id returned by `open`.
        invocation_id: uuid::Uuid,
        #[arg(long = "disposition", value_enum)]
        disposition: CliDisposition,
        /// Opaque outcome JSON (defaults to `{}`).
        #[arg(long = "outcome")]
        outcome: Option<String>,
    },
}
```

- [ ] **Step 2: Write the failing parse test** — create `crates/temper-cli/src/commands/invocation.rs` with the dispatch + a parse test:

```rust
//! `temper invocation` subcommand dispatch. Cloud-mode-only API writes — posts
//! to the `/api/invocations` endpoints via `temper-client`.

use crate::cli::{CliDisposition, InvocationAction};
use crate::error::Result;
use crate::output;
use temper_core::types::invocation::Disposition;
use temper_core::types::invocation_requests::{CloseInvocationRequest, OpenInvocationRequest};

impl From<CliDisposition> for Disposition {
    fn from(d: CliDisposition) -> Self {
        match d {
            CliDisposition::Completed => Disposition::Completed,
            CliDisposition::Failed => Disposition::Failed,
            CliDisposition::Abandoned => Disposition::Abandoned,
        }
    }
}

pub fn run(action: InvocationAction) -> Result<()> {
    match action {
        InvocationAction::Open {
            trigger_kind,
            cogmap,
            parent,
        } => {
            let req = OpenInvocationRequest {
                trigger_kind,
                originating_cogmap: cogmap,
                parent_cogmap: parent,
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .invocations()
                        .open(&req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    output::success("Invocation opened.");
                    output::dim(format!("  invocation_id: {}", ack.invocation_id));
                    Ok(())
                })
            })
        }
        InvocationAction::Close {
            invocation_id,
            disposition,
            outcome,
        } => {
            let outcome = match outcome {
                Some(s) => serde_json::from_str(&s)
                    .map_err(|e| crate::error::Error::msg(format!("invalid --outcome JSON: {e}")))?,
                None => serde_json::Value::Object(Default::default()),
            };
            let req = CloseInvocationRequest {
                disposition: disposition.into(),
                outcome,
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    client
                        .invocations()
                        .close(invocation_id, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    output::success("Invocation closed.");
                    Ok(())
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{CliDisposition, Cli, Commands, InvocationAction};
    use clap::Parser;

    #[test]
    fn invocation_open_parses() {
        let cli = Cli::try_parse_from([
            "temper",
            "invocation",
            "open",
            "--trigger-kind=manual",
            "--cogmap=019e84ab-26ba-7560-9d34-c60d74a9fbe2",
        ])
        .expect("parse should succeed");
        match cli.command {
            Commands::Invocation {
                action: InvocationAction::Open { trigger_kind, parent, .. },
            } => {
                assert_eq!(trigger_kind, "manual");
                assert!(parent.is_none());
            }
            _ => panic!("expected Commands::Invocation / Open"),
        }
    }

    #[test]
    fn invocation_close_parses() {
        let cli = Cli::try_parse_from([
            "temper",
            "invocation",
            "close",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--disposition=completed",
        ])
        .expect("parse should succeed");
        match cli.command {
            Commands::Invocation {
                action: InvocationAction::Close { disposition, .. },
            } => assert_eq!(disposition, CliDisposition::Completed),
            _ => panic!("expected Commands::Invocation / Close"),
        }
    }
}
```

> Verify the `crate::error::Error::msg(...)` constructor exists in `temper-cli/src/error.rs`; if the CLI error type uses a different constructor (e.g. `Error::Other(String)` or `anyhow`), match it. The `client_err` helper is the same one `edge.rs` uses.

- [ ] **Step 3: Wire the module + dispatch**

In `crates/temper-cli/src/commands/mod.rs` add: `pub mod invocation;`

In `crates/temper-cli/src/main.rs`, next to the `Commands::Edge` arm (line ~360), add:

```rust
        Commands::Invocation { action } => temper_cli::commands::invocation::run(action),
```

- [ ] **Step 4: Run test to verify it fails then passes**

Run: `cargo nextest run -p temper-cli invocation_open_parses invocation_close_parses`
Expected: first FAIL (before Step 1–3 land), then PASS after.

- [ ] **Step 5: Reinstall the PATH binary (so the merged CLI behaves as built)**

Run: `cargo install --path crates/temper-cli`
Expected: installs `temper`. (Per the post-merge convention — a merged-but-not-installed CLI change silently behaves like the old binary.)

- [ ] **Step 6: Full check + commit**

Run: `cargo make check`
Expected: clean.

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/invocation.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper invocation open|close"
```

---

## Task 7: End-to-end — full-stack open→close through the live server

**Files:**
- Create: `tests/e2e/tests/invocation_envelope_test.rs`
- Modify (if needed): `tests/e2e/tests/common/mod.rs` (temper_next artifact + cogmap seeding helper)

**Interfaces:**
- Consumes: the whole vertical (Tasks 1–6) through the real Axum server + temper-client.

> **Harness dependency (read first).** The e2e harness runs against the `public`-schema test DB (`test-db`). The invocation path routes through `NextBackend`, which needs (a) `backend_selection = Next`, (b) the `temper_next` artifact namespace loaded, and (c) a seeded cogmap + the emitter entity (`pete@web`) for the authenticated profile. The existing e2e common harness does NOT set this up. This task therefore has two parts; do them in order and STOP after Step 1 if the harness work is larger than a session — a green Task 2–6 plus the Task 2 writes-level artifact proof already covers the substrate and glue. Do not write a test that silently skips.

- [ ] **Step 1: Decide harness viability**

Inspect `tests/e2e/tests/common/mod.rs` and `crates/temper-api/src/backend/selection.rs` to determine whether the e2e server can be started with `backend_selection = Next` against a DB that has the `temper_next` artifact + a seeded cogmap. Two outcomes:
- **If the harness already supports `next-backend` e2e** (a `next`-mode server fixture + artifact load exists): proceed to Step 2.
- **If not:** add a `// FOLLOW-UP:` note to the plan's tracking issue and the session note that the full-stack invocation e2e requires extending the e2e harness with temper_next artifact + cogmap seeding (a separate, larger task), and STOP here. The substrate is proven at the writes layer (Task 2); the surfaces are proven by parse/deserialize tests (Tasks 4, 6) and `cargo make check`.

- [ ] **Step 2: Write the e2e test (only if Step 1 is viable)**

```rust
//! Full-stack invocation envelope: open then close through the real Axum server
//! and temper-client, against a temper_next-backed test DB with a seeded cogmap.

mod common;

#[tokio::test]
async fn invocation_open_then_close_full_stack() {
    let ctx = common::next_backed_server_with_cogmap().await; // harness helper from Step 1
    let client = ctx.client();

    let ack = client
        .invocations()
        .open(&temper_core::types::invocation_requests::OpenInvocationRequest {
            trigger_kind: "manual".to_string(),
            originating_cogmap: ctx.cogmap_id,
            parent_cogmap: None,
        })
        .await
        .expect("open should succeed");

    client
        .invocations()
        .close(
            ack.invocation_id,
            &temper_core::types::invocation_requests::CloseInvocationRequest {
                disposition: temper_core::types::invocation::Disposition::Completed,
                outcome: serde_json::json!({"concepts": 1}),
            },
        )
        .await
        .expect("close should succeed");

    let status: String = sqlx::query_scalar(
        "SELECT status FROM temper_next.kb_invocations WHERE id = $1",
    )
    .bind(ack.invocation_id)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(status, "completed");
}
```

- [ ] **Step 3: Run + commit (only if Step 2 ran)**

Run: `cargo make test-e2e` (and `cargo make test-e2e-embed` if the harness needs the embed pipeline for cogmap genesis)
Expected: PASS.

```bash
git add tests/e2e/tests/invocation_envelope_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test(e2e): full-stack invocation open/close through NextBackend"
```

---

## Self-Review

**1. Spec coverage** (against `2026-06-19-cognitive-map-substrate-surfacing-design.md` Section 4):
- New commands ✓ (Task 1) · trait methods ✓ (Task 3) · NextBackend impls ✓ (Task 3) · `writes::` wrappers ✓ (Task 2) · MCP tools ✓ (Task 4) · API routes ✓ (Task 5) · CLI + temper-client ✓ (Tasks 5–6) · auth-before-write via `cogmap_readable_by_profile` ✓ (Task 3) · reuse of `SeedAction`/`fire`/SQL ✓ (Task 2). Testing: writes artifact test ✓ (Task 2), MCP/CLI parse tests ✓ (Tasks 4, 6), e2e ✓/flagged (Task 7). The spec's "one e2e" is honored but gated on harness viability (Task 7 Step 1) rather than silently assumed.

**2. Placeholder scan:** No TBD/TODO. Two explicit "verify against existing code" instructions (the `client.relationships()` accessor shape; the CLI error constructor) name the exact file and pattern to copy — these are grounding checks, not deferrals.

**3. Type consistency:** `OpenInvocation`/`CloseInvocation` field names match across commands (Task 1) → handlers/tools/CLI (Tasks 4–6). `Disposition` is the temper-core enum throughout; `map_disposition` (Task 3) is the only bridge to `temper_next::payloads::Disposition`. `InvocationAck { invocation_id }` is consistent across API + MCP + client. The trait returns `CommandOutput<Uuid>` (open) / `CommandOutput<()>` (close) — matched in both impls and all three surfaces.

**Known scope notes (carried from the spec):** server-mint of the invocation id (caller-supplied ids for byte-exact durable-resume re-issue deferred); `scoped_entity` = the acting emitter in v1 (distinct scoped entities deferred); backlog items 2–6 (`facet_set`, `cogmap_genesis`, …) are repeat applications of this same vertical, each its own plan.
