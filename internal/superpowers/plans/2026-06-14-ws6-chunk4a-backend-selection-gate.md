# WS6 Chunk 4a — Backend-Selection Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the in-DB backend-selection gate: a `public` config flag (default `legacy`, zero behavior change) read once per process, plus a selector seam that every API handler and MCP tool routes backend construction through — with the `next` arm erroring cleanly until 4b lands `NextBackend`.

**Architecture:** A singleton `kb_backend_selection` row holds `'legacy' | 'next'`. It's read once at startup into a `BackendSelection` field on `AppState` (which both API handlers and MCP tools already hold). Two selector helpers consume it: `select_backend` (returns `Box<dyn Backend>` for the six trait-method call sites) and `require_legacy_backend` (returns a concrete `DbBackend` for relationship/edge sites whose methods aren't on the trait yet — these stay legacy in 4a but refuse `next` so a process never half-switches). With the flag defaulting `legacy`, the seam is a pure indirection over today's `DbBackend::new` — existing behavior is byte-identical.

**Tech Stack:** Rust, sqlx (compile-time macros, `public` namespace → temper-api `.sqlx`), axum, async-trait, rmcp (MCP), cargo-nextest.

---

## File Structure

- **Create** `migrations/20260614000001_backend_selection_flag.sql` — singleton flag table + seed row.
- **Create** `crates/temper-api/src/backend/selection.rs` — `BackendSelection` enum + `select_backend` + `require_legacy_backend`.
- **Create** `crates/temper-api/src/services/backend_selection_service.rs` — `read(pool) -> BackendSelection` (the only SQL).
- **Modify** `crates/temper-core/src/error.rs` — add `NotImplemented(String)` variant.
- **Modify** `crates/temper-api/src/error.rs` — map `NotImplemented` → `ApiError::Internal`.
- **Modify** `crates/temper-api/src/backend/mod.rs` + `services/mod.rs` — module wiring + re-exports.
- **Modify** `crates/temper-api/src/state.rs` — `backend_selection` field, `legacy` default in `new`, `with_backend_selection` builder.
- **Modify** `crates/temper-api/src/main.rs`, `api/axum.rs`, `api/mcp.rs` — read flag at startup, apply to state.
- **Modify** `tests/e2e/tests/common/mod.rs` — harness reads flag (production parity).
- **Modify** `crates/temper-api/src/handlers/{resources,ingest,meta}.rs` — trait-method sites → `select_backend`.
- **Modify** `crates/temper-api/src/handlers/edges.rs` — relationship sites → `require_legacy_backend`.
- **Modify** `crates/temper-mcp/src/tools/resources.rs` — trait-method sites → `select_backend`.
- **Modify** `crates/temper-mcp/src/tools/relationships.rs` — relationship sites → `require_legacy_backend`.
- **Create** e2e test `tests/e2e/tests/backend_selection_gate.rs` — flag=next wiring on api + mcp.

---

## Task 1: The flag table migration

**Files:**
- Create: `migrations/20260614000001_backend_selection_flag.sql`
- Test: `crates/temper-api/src/services/backend_selection_service.rs` (added in Task 2; the migration is exercised there)

- [ ] **Step 1: Write the migration**

Create `migrations/20260614000001_backend_selection_flag.sql`:

```sql
-- WS6 chunk 4a (§D): in-DB backend-selection gate.
-- A singleton config row in `public` choosing which substrate the surfaces
-- dispatch to. Default 'legacy' => install is zero behavior change. The flip
-- (chunk 5) is a trivial one-row UPDATE migration. Governs SURFACES, not
-- substrate, so it lives in `public`, not `temper_next`.

CREATE TABLE public.kb_backend_selection (
    id         boolean     PRIMARY KEY DEFAULT true,
    backend    text        NOT NULL DEFAULT 'legacy'
                           CHECK (backend IN ('legacy', 'next')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT kb_backend_selection_singleton CHECK (id = true)
);

INSERT INTO public.kb_backend_selection (id, backend) VALUES (true, 'legacy');
```

- [ ] **Step 2: Apply the migration to the dev DB**

Run: `cargo make docker-up && DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run --source migrations`
Expected: `Applied 20260614000001/migrate backend selection flag`

- [ ] **Step 3: Verify the row exists and constraints hold**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "SELECT id, backend FROM kb_backend_selection;"
psql postgresql://temper:temper@localhost:5437/temper_development -c "INSERT INTO kb_backend_selection (id, backend) VALUES (false, 'legacy');" 2>&1 | grep -q "violates check constraint" && echo "SINGLETON OK"
psql postgresql://temper:temper@localhost:5437/temper_development -c "UPDATE kb_backend_selection SET backend = 'bogus';" 2>&1 | grep -q "violates check constraint" && echo "ENUM OK"
```
Expected: one row `t | legacy`, then `SINGLETON OK`, then `ENUM OK`.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260614000001_backend_selection_flag.sql
git commit -m "WS6 4a: add kb_backend_selection flag table (default legacy)"
```

---

## Task 2: `BackendSelection` enum, error variant, and read service

**Files:**
- Modify: `crates/temper-core/src/error.rs`
- Modify: `crates/temper-api/src/error.rs`
- Create: `crates/temper-api/src/backend/selection.rs`
- Create: `crates/temper-api/src/services/backend_selection_service.rs`
- Modify: `crates/temper-api/src/backend/mod.rs`, `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Add the `NotImplemented` error variant**

In `crates/temper-core/src/error.rs`, add to the `TemperError` enum (after `Conflict`):

```rust
    /// A code path that is wired but not yet available (e.g. the `next`
    /// backend before WS6 4b lands `NextBackend`). Distinct from `Api` —
    /// it is a deliberate, temporary "not here yet", not an upstream failure.
    #[error("not implemented: {0}")]
    NotImplemented(String),
```

- [ ] **Step 2: Map the variant in the API error translation**

In `crates/temper-api/src/error.rs`, in the `From<TemperError> for ApiError` match (alongside the other arms, near `TemperError::Api(s) => ApiError::Internal(s)`), add:

```rust
            TemperError::NotImplemented(s) => ApiError::Internal(format!("not implemented: {s}")),
```

- [ ] **Step 3: Define `BackendSelection`**

Create `crates/temper-api/src/backend/selection.rs`:

```rust
//! Backend-selection gate (WS6 chunk 4a, §D).
//!
//! A process reads the `kb_backend_selection` flag once at startup into
//! [`BackendSelection`] and stores it on `AppState`. Surfaces construct their
//! backend through [`select_backend`] / [`require_legacy_backend`] rather than
//! calling `DbBackend::new` directly, so the flip (chunk 5) is one config row
//! + one redeploy.

use temper_core::error::TemperError;

/// Which substrate the surfaces dispatch reads/writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSelection {
    /// Today's `public.*` schema via `DbBackend`.
    Legacy,
    /// The `temper_next.*` substrate via `NextBackend` (lands in 4b).
    Next,
}

impl BackendSelection {
    /// Parse the stored flag value. Encapsulated so the stringly form never
    /// leaks past this boundary.
    pub(crate) fn from_db(value: &str) -> Result<Self, TemperError> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "next" => Ok(Self::Next),
            other => Err(TemperError::Config(format!(
                "unknown backend selection flag value: {other:?}"
            ))),
        }
    }
}
```

- [ ] **Step 4: Write the read service**

Create `crates/temper-api/src/services/backend_selection_service.rs`:

```rust
//! Reads the singleton `kb_backend_selection` flag. The only SQL touching
//! the gate table (service layer owns SQL).

use sqlx::PgPool;
use temper_core::error::TemperError;

use crate::backend::selection::BackendSelection;

/// Read the current backend selection. The table is seeded with exactly one
/// row by migration `20260614000001`, so a missing row is a hard error.
pub async fn read(pool: &PgPool) -> Result<BackendSelection, TemperError> {
    let value = sqlx::query_scalar!(
        "SELECT backend FROM kb_backend_selection WHERE id = true"
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| TemperError::Api(format!("read backend selection: {e}")))?
    .ok_or_else(|| {
        TemperError::Config("kb_backend_selection row missing (migration not run?)".into())
    })?;

    BackendSelection::from_db(&value)
}
```

- [ ] **Step 5: Wire the modules**

In `crates/temper-api/src/backend/mod.rs`, add `pub mod selection;` and `pub use selection::BackendSelection;`.
In `crates/temper-api/src/services/mod.rs`, add `pub mod backend_selection_service;`.

- [ ] **Step 6: Write the failing test**

Append to `crates/temper-api/src/services/backend_selection_service.rs`:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../../migrations")]
    async fn read_defaults_to_legacy(pool: PgPool) {
        let sel = read(&pool).await.expect("read flag");
        assert_eq!(sel, BackendSelection::Legacy);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn read_reflects_next(pool: PgPool) {
        sqlx::query!("UPDATE kb_backend_selection SET backend = 'next' WHERE id = true")
            .execute(&pool)
            .await
            .unwrap();
        let sel = read(&pool).await.expect("read flag");
        assert_eq!(sel, BackendSelection::Next);
    }
}
```

- [ ] **Step 7: Run the tests to verify they fail, then pass after the cache regenerates**

Run: `cargo make prepare-api` (regenerates `crates/temper-api/.sqlx` so the new `query_scalar!`/`query!` compile offline — they target `public`, temper-api's namespace).
Then: `cargo nextest run -p temper-api --features test-db backend_selection_service`
Expected: `read_defaults_to_legacy` PASS, `read_reflects_next` PASS.

- [ ] **Step 8: Verify the whole crate still checks**

Run: `cargo make check`
Expected: clean (fmt, clippy, docs all OK).

- [ ] **Step 9: Commit**

```bash
git add crates/temper-core/src/error.rs crates/temper-api/src/error.rs \
        crates/temper-api/src/backend/ crates/temper-api/src/services/ \
        crates/temper-api/.sqlx
git commit -m "WS6 4a: BackendSelection enum + read service + NotImplemented error variant"
```

---

## Task 3: The selector helpers

**Files:**
- Modify: `crates/temper-api/src/backend/selection.rs`

- [ ] **Step 1: Write the failing test**

Append a test module to `crates/temper-api/src/backend/selection.rs`:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::operations::Surface;
    use temper_core::types::ids::ProfileId;
    use uuid::Uuid;

    fn pid() -> ProfileId {
        ProfileId::from(Uuid::nil())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn select_backend_legacy_returns_a_backend(pool: PgPool) {
        let b = select_backend(
            BackendSelection::Legacy,
            &pool,
            pid(),
            "api".to_string(),
            Surface::ApiHttp,
        );
        assert!(b.is_ok(), "legacy arm must construct a backend");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn select_backend_next_errors(pool: PgPool) {
        let b = select_backend(
            BackendSelection::Next,
            &pool,
            pid(),
            "api".to_string(),
            Surface::ApiHttp,
        );
        assert!(
            matches!(b, Err(TemperError::NotImplemented(_))),
            "next arm must error until 4b"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_legacy_refuses_next(pool: PgPool) {
        let ok = require_legacy_backend(
            BackendSelection::Legacy,
            &pool,
            pid(),
            "mcp".to_string(),
            Surface::Mcp,
        );
        assert!(ok.is_ok(), "legacy arm yields a concrete DbBackend");

        let err = require_legacy_backend(
            BackendSelection::Next,
            &pool,
            pid(),
            "mcp".to_string(),
            Surface::Mcp,
        );
        assert!(
            matches!(err, Err(TemperError::NotImplemented(_))),
            "relationship/edge sites must refuse next until 4c"
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::selection`
Expected: FAIL — `select_backend` / `require_legacy_backend` not found.

- [ ] **Step 3: Implement the helpers**

Add to `crates/temper-api/src/backend/selection.rs` (after the `impl BackendSelection`), with the imports at the top of the file:

```rust
use sqlx::PgPool;
use temper_core::operations::{Backend, Surface};
use temper_core::types::ids::ProfileId;

use crate::backend::DbBackend;
```

```rust
/// Construct the active backend for a trait-method call site (the six
/// `Backend` commands). Returns a boxed trait object so the `next` arm can
/// later supply `NextBackend` behind the same interface.
pub fn select_backend(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: String,
    surface: Surface,
) -> Result<Box<dyn Backend>, TemperError> {
    match selection {
        BackendSelection::Legacy => Ok(Box::new(DbBackend::new(
            pool.clone(),
            profile_id,
            device_id,
            surface,
        ))),
        BackendSelection::Next => Err(TemperError::NotImplemented(
            "next backend not yet available (WS6 4b)".into(),
        )),
    }
}

/// Construct a concrete `DbBackend` for call sites whose methods are not yet
/// on the `Backend` trait (relationship/edge writes). These stay on legacy in
/// 4a but refuse `next`, so a process never half-switches: resource ops would
/// route to a substrate these ops can't reach. The trait growth that brings
/// them under `select_backend` lands in 4c.
pub fn require_legacy_backend(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: String,
    surface: Surface,
) -> Result<DbBackend, TemperError> {
    match selection {
        BackendSelection::Legacy => Ok(DbBackend::new(pool.clone(), profile_id, device_id, surface)),
        BackendSelection::Next => Err(TemperError::NotImplemented(
            "relationship/edge writes not yet ported to the next backend (WS6 4c)".into(),
        )),
    }
}
```

Also add `pub use selection::{require_legacy_backend, select_backend};` to `crates/temper-api/src/backend/mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::selection`
Expected: all three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/backend/
git commit -m "WS6 4a: select_backend + require_legacy_backend selector helpers"
```

---

## Task 4: Wire `BackendSelection` onto `AppState` and read it at startup

**Files:**
- Modify: `crates/temper-api/src/state.rs:152-167`
- Modify: `crates/temper-api/src/main.rs:34`
- Modify: `api/axum.rs:32`
- Modify: `api/mcp.rs:34`
- Modify: `tests/e2e/tests/common/mod.rs:330`

- [ ] **Step 1: Add the field, default, and builder to `AppState`**

In `crates/temper-api/src/state.rs`, add the import `use crate::backend::BackendSelection;`, then extend the struct and impl:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwks_store: Arc<JwksKeyStore>,
    pub config: Arc<ApiConfig>,
    /// Read once per process at startup. A flip takes effect on the next
    /// redeploy — that is the cutover model, not a staleness bug.
    pub backend_selection: BackendSelection,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
            // Safe default: legacy. Production startups override via
            // `with_backend_selection` after reading the flag. Tests that
            // don't care get legacy for free.
            backend_selection: BackendSelection::Legacy,
        }
    }

    /// Override the backend selection (used by startup after reading the flag,
    /// and by tests exercising the `next` arm without a redeploy).
    #[must_use]
    pub fn with_backend_selection(mut self, selection: BackendSelection) -> Self {
        self.backend_selection = selection;
        self
    }
}
```

- [ ] **Step 2: Read the flag in the three production startups**

In `crates/temper-api/src/main.rs`, replace `let state = AppState::new(pool, jwks_store, config);` with:

```rust
    let backend_selection = temper_api::services::backend_selection_service::read(&pool)
        .await
        .expect("Failed to read backend selection flag");
    let state = AppState::new(pool, jwks_store, config).with_backend_selection(backend_selection);
```

In `api/axum.rs`, replace `let state = AppState::new(pool, jwks_store, config);` with the same two lines (the symbols there are `pool`, `jwks_store`, `config`).

In `api/mcp.rs`, replace `let api_state = AppState::new(pool, jwks_store, api_config);` with:

```rust
    let backend_selection = temper_api::services::backend_selection_service::read(&pool)
        .await
        .expect("Failed to read backend selection flag");
    let api_state =
        AppState::new(pool, jwks_store, api_config).with_backend_selection(backend_selection);
```

- [ ] **Step 3: Mirror the production read in the e2e harness**

In `tests/e2e/tests/common/mod.rs`, replace `let state = AppState::new(pool.clone(), jwks_store, api_config);` with:

```rust
    let backend_selection =
        temper_api::services::backend_selection_service::read(&pool)
            .await
            .expect("read backend selection flag");
    let state = AppState::new(pool.clone(), jwks_store, api_config)
        .with_backend_selection(backend_selection);
```

This keeps every existing e2e test on `legacy` (the seeded default) while exercising the real read path.

- [ ] **Step 4: Verify it compiles and nothing changed behaviorally**

Run: `cargo build -p temper-api && cargo build -p temper-mcp`
Expected: clean build (no call site uses `backend_selection` yet, so behavior is unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/state.rs crates/temper-api/src/main.rs \
        api/axum.rs api/mcp.rs tests/e2e/tests/common/mod.rs
git commit -m "WS6 4a: read backend-selection flag at startup, store on AppState"
```

---

## Task 5: Route API trait-method call sites through `select_backend`

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs` (4 sites: ~134, 216, 289, 329)
- Modify: `crates/temper-api/src/handlers/ingest.rs` (2 sites: ~69, 135)
- Modify: `crates/temper-api/src/handlers/meta.rs` (1 site: ~78)

> These sites call only the six `Backend` trait methods (`create_resource`, `show_resource`, `update_resource`, `delete_resource`, `list_resources`, `search_resources`). **Before editing each, confirm the method called is one of those six** — if any site calls a concrete `DbBackend`-only method, route it through `require_legacy_backend` instead (as in Task 6).

- [ ] **Step 1: Replace each `DbBackend::new(...)` with `select_backend(...)`**

At every listed site, replace the construction block, e.g.:

```rust
    let backend = DbBackend::new(
        state.pool.clone(),
        ProfileId::from(auth.0.profile.id),
        "api".to_string(),
        Surface::ApiHttp,
    );
```

with:

```rust
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        "api".to_string(),
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
```

Update each file's imports: remove the now-unused `use crate::backend::DbBackend;` (if no other site in the file needs it) and add `use crate::backend::select_backend;`. The subsequent `backend.<method>(cmd).await` calls are unchanged (`Box<dyn Backend>` dispatches the trait method).

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p temper-api`
Expected: clean (any "unused import `DbBackend`" warning means remove that import; clippy is `-D warnings`).

- [ ] **Step 3: Run the API handler tests**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: all PASS — behavior identical under the default `legacy` flag.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/handlers/resources.rs \
        crates/temper-api/src/handlers/ingest.rs \
        crates/temper-api/src/handlers/meta.rs
git commit -m "WS6 4a: route API resource handlers through select_backend"
```

---

## Task 6: Route API relationship/edge sites through `require_legacy_backend`

**Files:**
- Modify: `crates/temper-api/src/handlers/edges.rs` (4 sites: ~77, 123, 168, 213)

- [ ] **Step 1: Replace each `DbBackend::new(...)` with `require_legacy_backend(...)`**

At each site, replace the `DbBackend::new(...)` block with:

```rust
    let backend = require_legacy_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        "api".to_string(),
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
```

(Match the existing args at each site — `device_id` is whatever the site passes today.) Update imports: replace `use crate::backend::DbBackend;` with `use crate::backend::require_legacy_backend;`. The subsequent concrete calls (`backend.assert_relationship(...)`, etc.) are unchanged — `require_legacy_backend` returns a concrete `DbBackend`.

- [ ] **Step 2: Verify it compiles and tests pass**

Run: `cargo build -p temper-api && cargo nextest run -p temper-api --features test-db edges`
Expected: clean build; edge/relationship handler tests PASS (legacy default).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/handlers/edges.rs
git commit -m "WS6 4a: route API edge handlers through require_legacy_backend"
```

---

## Task 7: Route MCP resource tools through `select_backend`

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (4 sites: ~364, 622, 674, 717)

> MCP tool fns receive the service (`self`), so `self.api_state.backend_selection` and `self.api_state.pool` are both in scope (the local `pool` binding is derived from the same state). No signature changes.

- [ ] **Step 1: Replace each `DbBackend::new(...)` with `select_backend(...)`**

At each site, replace:

```rust
    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
```

with:

```rust
    let backend = select_backend(
        self.api_state.backend_selection,
        &self.api_state.pool,
        profile_id,
        "mcp".to_string(),
        Surface::Mcp,
    )
    .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
```

(If a site's local binding for the service is named differently than `self`, use that name. If `pool` is a local `&PgPool` derived from `self.api_state.pool`, you can pass it directly as `&self.api_state.pool` per above.) Update imports: replace `use temper_api::backend::DbBackend;` with `use temper_api::backend::select_backend;`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p temper-mcp`
Expected: clean.

- [ ] **Step 3: Run MCP tests**

Run: `cargo nextest run -p temper-mcp`
Expected: all PASS (legacy default).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "WS6 4a: route MCP resource tools through select_backend"
```

---

## Task 8: Route MCP relationship tools through `require_legacy_backend`

**Files:**
- Modify: `crates/temper-mcp/src/tools/relationships.rs` (4 sites: ~129, 158, 186, 214)

- [ ] **Step 1: Replace each `DbBackend::new(...)` with `require_legacy_backend(...)`**

At each site, replace:

```rust
    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
```

with:

```rust
    let backend = require_legacy_backend(
        self.api_state.backend_selection,
        &self.api_state.pool,
        profile_id,
        "mcp".to_string(),
        Surface::Mcp,
    )
    .map_err(|e| map_err(e, "select_backend"))?;
```

(`map_err` is the file's existing rmcp error mapper used at these sites; reuse it. If the local service binding isn't `self`, use that name.) Update imports: replace `use temper_api::backend::DbBackend;` with `use temper_api::backend::require_legacy_backend;`.

- [ ] **Step 2: Verify it compiles and tests pass**

Run: `cargo build -p temper-mcp && cargo nextest run -p temper-mcp`
Expected: clean; PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-mcp/src/tools/relationships.rs
git commit -m "WS6 4a: route MCP relationship tools through require_legacy_backend"
```

---

## Task 9: End-to-end gate-wiring tests (flag=next) + full verification

**Files:**
- Create: `tests/e2e/tests/backend_selection_gate.rs`

> Proves the wiring (DB flag → startup read → state → selector → surface error), not just the helper functions — per the "e2e at the production caller" discipline. The harness reads the flag (Task 4 Step 3), so flipping the row to `next` before spawn yields a `next` server.

- [ ] **Step 1: Write the e2e test**

Create `tests/e2e/tests/backend_selection_gate.rs`. Model the harness setup, auth, and request helpers on an existing e2e test (e.g. an existing file in `tests/e2e/tests/` that drives a resource read and an MCP tool — copy its `E2eTestApp` spawn + JWT setup boilerplate). The gate-specific assertions:

```rust
//! WS6 chunk 4a: with the backend-selection flag set to `next`, both the API
//! resource surface and an MCP tool must fail with the NotImplemented guard
//! (the `next` backend does not exist until 4b) — proving the selector seam is
//! actually wired into the surfaces, not just unit-tested in isolation.

#![cfg(feature = "test-db")]

mod common;

#[sqlx::test(migrations = "../../migrations")]
async fn api_resource_read_is_gated_off_under_next(pool: sqlx::PgPool) {
    sqlx::query!("UPDATE kb_backend_selection SET backend = 'next' WHERE id = true")
        .execute(&pool)
        .await
        .unwrap();

    // Spawn the in-process server against this pool (harness reads the flag).
    let app = common::E2eTestApp::spawn_with_pool(pool).await;

    // Any authenticated resource read routes through select_backend → next arm.
    let resp = app.get_authed("/api/resources").await;
    assert_eq!(
        resp.status(),
        500,
        "next arm must surface as a server error until 4b, got {}",
        resp.status()
    );
    let body = resp.text().await;
    assert!(
        body.contains("not implemented") || body.contains("next backend"),
        "error body should name the gate: {body}"
    );
}
```

> If the existing harness exposes a different spawn entry point than `spawn_with_pool`, use it; the load-bearing parts are (a) the `UPDATE ... 'next'` before spawn and (b) an authenticated request that constructs a backend. Add a second test driving an MCP tool the same way (set flag → call the tool through the MCP router → expect the internal-error). If the e2e harness has no MCP entry point, assert the MCP path at the `temper-mcp` integration level instead (set the flag on the test `AppState` via `.with_backend_selection(BackendSelection::Next)` and call the tool fn), so both surfaces are covered.

- [ ] **Step 2: Run the e2e test**

Run: `cargo make prepare-e2e` (the new `query!` is a test-target query in the e2e crate), then `cargo make test-e2e -- backend_selection_gate`
Expected: PASS — the `next` flag yields a 500 from the resource surface and the MCP path errors.

- [ ] **Step 3: Full verification**

Run:
```bash
cargo make check
cargo nextest run -p temper-api --features test-db
cargo nextest run -p temper-mcp
cargo make test-e2e
```
Expected: `cargo make check` clean; all suites green (everything still on `legacy` except the gate test).

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/backend_selection_gate.rs tests/e2e/.sqlx
git commit -m "WS6 4a: e2e gate-wiring tests — surfaces error under flag=next"
```

---

## Self-Review notes

- **Spec coverage:** flag table + default legacy (Task 1) ✓; process-start cached read on `AppState` (Tasks 2,4) ✓; `select_backend` seam across api+mcp call sites (Tasks 3,5–8) ✓; relationship/edge sites stay legacy but refuse `next` (Tasks 3,6,8) ✓; `next` arm returns a clean NotImplemented until 4b (Tasks 2,3) ✓; proof gates — migration singleton/default (Task 1), legacy byte-identical (Tasks 5–8 suites), flag=next wiring error per surface (Task 9) ✓; test seam to inject the flag without redeploy (`with_backend_selection`, Task 4) ✓.
- **Namespace:** every new query targets `public` (`kb_backend_selection`) → temper-api `.sqlx` (Task 2) + e2e `.sqlx` (Task 9). No `temper_next` query added, so `prepare-next` is untouched.
- **Type consistency:** `BackendSelection::{Legacy,Next}`, `select_backend`/`require_legacy_backend` signatures `(BackendSelection, &PgPool, ProfileId, String, Surface)`, and `TemperError::NotImplemented(String)` are used identically across all tasks.
