# Machine-Principal Registration (Phase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make machine (`client_credentials`) principals a registered, revocable, attributable set — `kb_machine_clients` as a fail-closed gate — instead of anything with a valid Auth0 token silently JIT-provisioning an agent profile.

**Architecture:** A new `kb_machine_clients` table is the allowlist. `resolve_machine_from_claims` loses its create branch and becomes lookup-or-reject; because both temper-api and temper-mcp route machine principals through that one function in `temper-services`, the gate cannot drift between surfaces. Profile creation *inverts* onto a new registration service, which runs one transaction (profile + auth link + emitter entities + gating-team membership + machine-client row + grants). A `temper admin machine` CLI drives it.

**Tech Stack:** Rust, Axum, sqlx (compile-time-checked macros), PostgreSQL 17/18, clap, cargo-nextest, cargo-make.

**Spec:** [docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md](../specs/2026-07-10-machine-principal-registration-design.md). Decisions D1–D14 are binding; read them before Task 1.

## Global Constraints

- **`--all-features` on every build and clippy invocation.** `cargo make check` is the honest local probe.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.**
- **All public types derive `Debug`.**
- **Typed structs over `serde_json::json!()`.** Any request/response body with a known shape is a struct.
- **Params structs for >5 domain-related parameters.** `#[expect(clippy::too_many_arguments)]` is a smell to fix.
- **Persistence lives in `temper-services/src/services/`.** Never inline `sqlx::query!()` in an HTTP handler, MCP tool, or CLI action.
- **Auth before writes.** Authorization checks precede every mutation.
- **After changing SQL:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-api`. Per-crate last. Do **not** `git add .sqlx` wholesale — `prepare-api`/`prepare-services` materialize ~207 untracked files; `git add` only the specific `.sqlx/query-*.json` files your change produced, and `git rm` orphans by path.
- **Migrations are immutable once applied.** Never edit `20260711000010`/`20260711000011` after they land on `main`; extend with a new migration.
- **Local DB:** `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`. Start it with `cargo make docker-up`.
- **Never `git checkout -- <path>`** to undo a probe edit; it discards uncommitted work. Copy the file aside first.
- **`rg -r` is `--replace`.** Use `rg -n`.

---

## File Structure

**Create:**
- `migrations/20260711000010_machine_clients.sql` — table, indexes, column comments.
- `migrations/20260711000011_backfill_machine_clients.sql` — the steward backfill, one idempotent `INSERT`. Separate file so a test can execute it standalone.
- `crates/temper-core/src/types/machine.rs` — `MachineClient`, `ProvisionMachineRequest`, `RebindMachineRequest`, `GrantSpec`, `TeamSpec`.
- `crates/temper-services/src/services/machine_client_service.rs` — persistence: lookup, touch, list, get, insert, revoke.
- `crates/temper-services/src/services/machine_registration_service.rs` — the transactional `provision` / `rebind`.
- `crates/temper-api/src/handlers/machine_clients.rs` — thin handlers, `is_system_admin`-gated.
- `crates/temper-client/src/machine.rs` — `MachineClientsClient` sub-client.
- `crates/temper-cli/src/commands/admin_machine.rs` — CLI command surface.
- `tests/e2e/tests/machine_gate_e2e.rs` — both surfaces.

**Modify:**
- `crates/temper-services/src/services/profile_service.rs` — the inversion + the gate.
- `crates/temper-services/src/services/access_service.rs` — extract `insert_grant`, reusable inside a transaction.
- `crates/temper-services/src/services/mod.rs`, `crates/temper-core/src/types/mod.rs` — module registration.
- `crates/temper-api/src/routes.rs` — mount `/api/machine-clients` in the gated router with plain `.route()` (out of the OpenAPI contract, matching `/api/access/admin/*`).
- `crates/temper-client/src/lib.rs` — `pub fn machine_clients(&self)`.
- `crates/temper-cli/src/cli.rs` — `AdminAction::Machine { action: AdminMachineAction }`.
- `tests/e2e/tests/auth_seam_m2m_e2e.rs` — **inverts**: it currently asserts the JIT behavior being deleted.

---

## Task 1: The migration and its backfill

**Files:**
- Create: `migrations/20260711000010_machine_clients.sql`
- Create: `migrations/20260711000011_backfill_machine_clients.sql`
- Test: `crates/temper-services/src/services/machine_client_service.rs` (test module only in this task)

**Interfaces:**
- Consumes: nothing.
- Produces: table `kb_machine_clients` with columns `id, client_id, issuer, label, profile_id, team_id, registered_by_profile_id, created, last_seen_at, revoked_at, revoked_by_profile_id`.

The backfill is a **separate migration file containing exactly one statement**, so a test can `include_str!` it and execute it twice to prove idempotence. Both are additive; `20260711000010` runs first.

- [ ] **Step 1: Write the table migration**

Create `migrations/20260711000010_machine_clients.sql`:

```sql
-- Machine-principal registration (spec 2026-07-10, D2/D3/D6).
-- Registration is a GATE, not a ledger: `resolve_machine_from_claims` rejects any client_id
-- absent from this table, even bearing a perfectly valid IdP token.
CREATE TABLE kb_machine_clients (
    id                       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    client_id                TEXT        NOT NULL UNIQUE,
    issuer                   TEXT        NOT NULL DEFAULT 'auth0-m2m',
    label                    TEXT        NOT NULL,
    profile_id               UUID        NOT NULL REFERENCES kb_profiles(id),
    team_id                  UUID            NULL REFERENCES kb_teams(id),
    registered_by_profile_id UUID        NOT NULL REFERENCES kb_profiles(id),
    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at             TIMESTAMPTZ     NULL,
    revoked_at               TIMESTAMPTZ     NULL,
    revoked_by_profile_id    UUID            NULL REFERENCES kb_profiles(id)
);

CREATE INDEX idx_kb_machine_clients_profile ON kb_machine_clients(profile_id);
CREATE INDEX idx_kb_machine_clients_team    ON kb_machine_clients(team_id) WHERE team_id IS NOT NULL;

COMMENT ON TABLE kb_machine_clients IS
  'Allowlist of machine (client_credentials) principals. Fail-closed: an unregistered client_id is rejected at authentication.';
COMMENT ON COLUMN kb_machine_clients.client_id IS
  'The IdP client identifier, matching AuthClaims.external_user_id from normalize_machine. UNIQUE; its constraint index serves the authentication-path lookup.';
COMMENT ON COLUMN kb_machine_clients.issuer IS
  'Who issued this credential. Phase A writes only auth0-m2m. Phase B writes temper. Forward slot.';
COMMENT ON COLUMN kb_machine_clients.team_id IS
  'The machine OWNER -- which team a registration was performed on behalf of. NEVER consulted for authorization. Reach is kb_access_grants plus team membership, both plural; an authorization predicate written against this column would be strictly narrower than resources_visible_to. Never the agent''s own auto-provisioned personal team.';
COMMENT ON COLUMN kb_machine_clients.last_seen_at IS
  'Coarse (five-minute) liveness touch. Deliberately not precise: authentication must not write on the common path.';
COMMENT ON COLUMN kb_machine_clients.revoked_at IS
  'A revoked row is dead. Reactivation is a new registration, never an UPDATE. Rows are never deleted.';
```

No secret column exists, in this phase or ever (D1).

- [ ] **Step 2: Write the backfill migration**

Create `migrations/20260711000011_backfill_machine_clients.sql`. `registered_by_profile_id` is the agent's own profile — nobody human authorized these, and naming a human would put a lie in the accountability ledger (D13).

```sql
-- Backfill: every pre-existing auth0-m2m auth link becomes a registered client.
-- Verified against temper-cloud/main on 2026-07-10: this set is exactly the steward.
-- ONE statement, idempotent. Kept in its own file so a test can execute it standalone.
INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, registered_by_profile_id)
SELECT l.auth_provider_user_id,
       'auth0-m2m',
       'backfilled: ' || p.handle,
       l.profile_id,
       l.profile_id
  FROM kb_profile_auth_links l
  JOIN kb_profiles p ON p.id = l.profile_id
 WHERE l.auth_provider = 'auth0-m2m'
ON CONFLICT (client_id) DO NOTHING;
```

- [ ] **Step 3: Apply the migrations locally**

```bash
cargo make docker-up
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
sqlx migrate run
```

Expected: two migrations applied, no error. If you instead see `relation ... does not exist` on a *later* build, your local DB was stale — `sqlx migrate run` is the fix, not a code change.

- [ ] **Step 4: Write the failing backfill test**

Create `crates/temper-services/src/services/machine_client_service.rs` with only this test module for now:

```rust
//! Persistence for `kb_machine_clients` — the machine-principal allowlist.

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    const BACKFILL: &str = include_str!("../../../../migrations/20260711000011_backfill_machine_clients.sql");

    /// Seed a profile plus an `auth0-m2m` auth link, as prod carries for the steward.
    async fn seed_agent_link(pool: &PgPool, client_id: &str) -> Uuid {
        let profile_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $3, NULL, '{}')",
            profile_id,
            format!("agent-{client_id}"),
            format!("agent-{client_id}"),
        )
        .execute(pool)
        .await
        .expect("seed profile");

        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
             VALUES ($1, $2, 'auth0-m2m', $3, NULL, false, true, now())",
            Uuid::now_v7(),
            profile_id,
            client_id,
        )
        .execute(pool)
        .await
        .expect("seed auth link");

        profile_id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn backfill_registers_existing_m2m_links_and_is_idempotent(pool: PgPool) {
        let profile_id = seed_agent_link(&pool, "steward-client-1").await;

        sqlx::raw_sql(BACKFILL).execute(&pool).await.expect("backfill runs");

        let row = sqlx::query!(
            "SELECT profile_id, registered_by_profile_id, label, issuer, revoked_at \
               FROM kb_machine_clients WHERE client_id = $1",
            "steward-client-1",
        )
        .fetch_one(&pool)
        .await
        .expect("backfilled row exists");

        assert_eq!(row.profile_id, profile_id);
        assert_eq!(
            row.registered_by_profile_id, profile_id,
            "backfilled rows are self-registered: no human authorized them (D13)"
        );
        assert!(row.label.starts_with("backfilled: "));
        assert_eq!(row.issuer, "auth0-m2m");
        assert!(row.revoked_at.is_none());

        // Re-running is a no-op, not a duplicate-key error.
        sqlx::raw_sql(BACKFILL).execute(&pool).await.expect("backfill is idempotent");
        let count = sqlx::query_scalar!("SELECT count(*) FROM kb_machine_clients")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, Some(1));
    }
}
```

Register the module. In `crates/temper-services/src/services/mod.rs`, add in alphabetical position (after `invitation_service`):

```rust
pub mod machine_client_service;
```

- [ ] **Step 5: Run the test to verify it fails, then passes**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo nextest run -p temper-services --features test-db backfill_registers_existing
```

Expected on a workspace whose migrations you have *not* yet applied to the ephemeral test DB: it passes immediately, because `#[sqlx::test(migrator = ...)]` applies all migrations to a fresh database. To see it bite, temporarily rename the `ON CONFLICT (client_id) DO NOTHING` clause away and confirm the second `raw_sql` fails with a unique-violation. **Restore it by editing the file back — never `git checkout --`.**

- [ ] **Step 6: Regenerate the sqlx cache and commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
git add migrations/20260711000010_machine_clients.sql \
        migrations/20260711000011_backfill_machine_clients.sql \
        crates/temper-services/src/services/machine_client_service.rs \
        crates/temper-services/src/services/mod.rs
git add $(git status --porcelain .sqlx crates/temper-services/.sqlx | awk '{print $2}')
git status --short
git commit -m "G3 Phase A: kb_machine_clients table and steward backfill"
```

`git status --short` before committing is not optional: a bad pathspec can stage nothing, and `prepare-*` materializes many untracked `.sqlx` files you must not sweep in.

---

## Task 2: The persistence layer

**Files:**
- Create: `crates/temper-core/src/types/machine.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-services/src/services/machine_client_service.rs`

**Interfaces:**
- Consumes: `kb_machine_clients` (Task 1).
- Produces:
  - `temper_core::types::machine::MachineClient` (struct, fields as below)
  - `machine_client_service::lookup_by_client_id(&PgPool, &str) -> ApiResult<Option<MachineClient>>`
  - `machine_client_service::touch_last_seen(&PgPool, Uuid) -> ApiResult<bool>` (true ⇒ a write happened)
  - `machine_client_service::get(&PgPool, Uuid) -> ApiResult<MachineClient>`
  - `machine_client_service::list(&PgPool, bool) -> ApiResult<Vec<MachineClient>>` (bool = include_revoked)
  - `machine_client_service::revoke(&PgPool, Uuid, ProfileId) -> ApiResult<MachineClient>`

- [ ] **Step 1: Define the shared type**

Create `crates/temper-core/src/types/machine.rs`:

```rust
//! Machine-principal registration types. See
//! `docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered machine (`client_credentials`) principal.
///
/// No secret is stored, in this phase or ever (D1). `team_id` is the machine's
/// OWNER, never its reach (D6).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MachineClient {
    pub id: Uuid,
    pub client_id: String,
    pub issuer: String,
    pub label: String,
    pub profile_id: Uuid,
    pub team_id: Option<Uuid>,
    pub registered_by_profile_id: Uuid,
    pub created: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by_profile_id: Option<Uuid>,
}

/// One team the machine should be enrolled in, with its role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSpec {
    pub team_id: Uuid,
    /// `owner` | `maintainer` | `member` | `watcher`. Defaults to `member` at the CLI.
    pub role: String,
}

/// One cogmap grant the machine should hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantSpec {
    pub cogmap_id: Uuid,
    pub can_write: bool,
}

/// Register a new machine principal. Reach is plural and always explicit (D10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionMachineRequest {
    pub client_id: String,
    pub label: String,
    /// Recorded as `team_id`. Owner, not reach.
    pub owner_team_id: Option<Uuid>,
    pub teams: Vec<TeamSpec>,
    pub grants: Vec<GrantSpec>,
}

/// Point a fresh `client_id` at an existing agent profile (D8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebindMachineRequest {
    /// The new IdP client id.
    pub client_id: String,
    /// The existing `kb_machine_clients.id` whose profile is inherited.
    pub from_machine_client_id: Uuid,
    pub label: String,
    /// When false (the default), the old row is revoked in the same transaction.
    pub keep_old_active: bool,
}
```

In `crates/temper-core/src/types/mod.rs`, add in alphabetical position (after `invitation` / before `permission`, wherever `m` sorts):

```rust
pub mod machine;
```

- [ ] **Step 2: Write the failing service tests**

Replace the `mod tests` block in `crates/temper-services/src/services/machine_client_service.rs` — keep `seed_agent_link` and the backfill test, and add these below them, inside the same `mod tests`:

```rust
    use crate::services::machine_client_service as svc;
    use temper_core::types::ids::ProfileId;

    /// Register `client_id` against a freshly seeded agent profile.
    async fn seed_registered(pool: &PgPool, client_id: &str) -> Uuid {
        let profile_id = seed_agent_link(pool, client_id).await;
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_machine_clients (id, client_id, label, profile_id, registered_by_profile_id) \
             VALUES ($1, $2, 'test', $3, $3)",
            id,
            client_id,
            profile_id,
        )
        .execute(pool)
        .await
        .expect("seed machine client");
        id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn lookup_finds_registered_and_misses_unregistered(pool: PgPool) {
        seed_registered(&pool, "known").await;

        let hit = svc::lookup_by_client_id(&pool, "known").await.expect("lookup");
        assert!(hit.is_some(), "registered client resolves");
        assert_eq!(hit.expect("some").client_id, "known");

        let miss = svc::lookup_by_client_id(&pool, "never-registered").await.expect("lookup");
        assert!(miss.is_none(), "unregistered client must not resolve");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn touch_last_seen_is_coarse(pool: PgPool) {
        let id = seed_registered(&pool, "coarse").await;

        // First touch writes (last_seen_at was NULL).
        assert!(svc::touch_last_seen(&pool, id).await.expect("touch 1"));

        // Second touch, immediately after, does NOT write: the row is inside the
        // five-minute window. This is what keeps authentication read-only (D9).
        assert!(
            !svc::touch_last_seen(&pool, id).await.expect("touch 2"),
            "two authentications inside five minutes must produce one write"
        );

        // Age the row past the window; the next touch writes again.
        sqlx::query!(
            "UPDATE kb_machine_clients SET last_seen_at = now() - interval '6 minutes' WHERE id = $1",
            id,
        )
        .execute(&pool)
        .await
        .expect("age row");
        assert!(svc::touch_last_seen(&pool, id).await.expect("touch 3"));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoke_marks_dead_and_list_hides_by_default(pool: PgPool) {
        let id = seed_registered(&pool, "doomed").await;
        let admin = seed_agent_link(&pool, "admin-actor").await;

        let revoked = svc::revoke(&pool, id, ProfileId::from(admin)).await.expect("revoke");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_profile_id, Some(admin));

        let active = svc::list(&pool, false).await.expect("list active");
        assert!(active.iter().all(|c| c.client_id != "doomed"));

        let all = svc::list(&pool, true).await.expect("list all");
        assert!(all.iter().any(|c| c.client_id == "doomed"));
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db machine_client_service
```

Expected: FAIL to compile — `svc::lookup_by_client_id` and friends do not exist.

- [ ] **Step 4: Implement the service**

Prepend to `crates/temper-services/src/services/machine_client_service.rs`, above the test module:

```rust
//! Persistence for `kb_machine_clients` — the machine-principal allowlist.
//!
//! Read path (`lookup_by_client_id`, `touch_last_seen`) is on the authentication
//! hot path for every machine call. Write paths are admin-driven and rare.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::MachineClient;

use crate::error::{ApiError, ApiResult};

/// The authentication-path lookup. `None` ⇒ unregistered. A revoked row still
/// resolves here; the caller distinguishes (the gate needs the timestamp to
/// build a useful rejection message).
pub async fn lookup_by_client_id(pool: &PgPool, client_id: &str) -> ApiResult<Option<MachineClient>> {
    let row = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients
            WHERE client_id = $1"#,
        client_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Coarse liveness touch (D9): writes only when `last_seen_at` is NULL or older
/// than five minutes, so the common authentication is a pure read. Returns
/// whether a write actually happened.
pub async fn touch_last_seen(pool: &PgPool, id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET last_seen_at = now()
            WHERE id = $1
              AND (last_seen_at IS NULL OR last_seen_at < now() - interval '5 minutes')"#,
        id,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Load one machine client by its own id.
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<MachineClient> {
    sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Enumerate registered clients, newest first. Revoked rows are hidden unless asked for.
pub async fn list(pool: &PgPool, include_revoked: bool) -> ApiResult<Vec<MachineClient>> {
    let rows = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients
            WHERE $1 OR revoked_at IS NULL
            ORDER BY created DESC"#,
        include_revoked,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a client dead. Idempotent in effect but not in record: a second revoke of an
/// already-revoked row is a no-op that returns the existing row (the first revoker and
/// first timestamp are the truth). Grants and memberships are deliberately untouched (D11).
pub async fn revoke(pool: &PgPool, id: Uuid, revoker: ProfileId) -> ApiResult<MachineClient> {
    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(pool)
    .await?;
    get(pool, id).await
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db machine_client_service
```

Expected: PASS, 4 tests.

- [ ] **Step 6: Regenerate caches, check, commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make check
git add crates/temper-core/src/types/machine.rs crates/temper-core/src/types/mod.rs \
        crates/temper-services/src/services/machine_client_service.rs
git add $(git status --porcelain .sqlx crates/temper-services/.sqlx | awk '{print $2}')
git status --short
git commit -m "G3 Phase A: machine_client_service persistence"
```

---

## Task 3: The inversion and the gate

This is the load-bearing task. `resolve_machine_from_claims` stops creating profiles and starts rejecting unregistered clients. An existing e2e test asserts the behavior being deleted and must be inverted in the same commit.

**Files:**
- Modify: `crates/temper-services/src/services/profile_service.rs:120` (`resolve_machine_from_claims`), `:300` (`create_agent_profile_and_link`), `:340` (`provision_profile_entities`)
- Modify: `tests/e2e/tests/auth_seam_m2m_e2e.rs`

**Interfaces:**
- Consumes: `machine_client_service::{lookup_by_client_id, touch_last_seen}` (Task 2).
- Produces:
  - `profile_service::create_agent_profile_and_link(conn: &mut PgConnection, client_id: &str) -> ApiResult<(Uuid, String)>` — now `pub(crate)`, takes a connection, takes the client id directly rather than `AuthClaims` (registration has no claims).
  - `profile_service::provision_profile_entities(conn: &mut PgConnection, profile_id: Uuid, handle: &str) -> ApiResult<()>` — now `pub(crate)`, takes a connection so it can join a transaction.

- [ ] **Step 1: Write the failing gate tests**

Append to the existing `#[cfg(all(test, feature = "test-db"))] mod tests` in `crates/temper-services/src/services/profile_service.rs`. The existing helper `machine_claims(client_id)` is already there at `:697`.

```rust
    /// The bite test. Under the old code this FAILS by finding a newly created profile.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn unregistered_machine_is_rejected_and_creates_no_profile(pool: PgPool) {
        let before = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count before");

        let c = machine_claims("never-registered");
        let err = resolve_from_claims(&pool, &c)
            .await
            .expect_err("an unregistered machine must be rejected");

        match err {
            ApiError::Unauthorized(msg) => {
                assert!(msg.contains("never-registered"), "message names the client id: {msg}");
                assert!(msg.contains("not registered"), "message says why: {msg}");
                assert!(
                    msg.contains("temper admin machine provision"),
                    "message names the remedy: {msg}"
                );
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }

        let after = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count after");
        assert_eq!(before, after, "authentication must not create a profile (D3)");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoked_machine_is_rejected_distinguishably(pool: PgPool) {
        // Seed a profile + registration, then revoke it.
        let profile_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, 'agent-revoked', 'agent-revoked', NULL, '{}')",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed profile");
        sqlx::query!(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id, revoked_at) \
             VALUES ('dead-client', 'test', $1, $1, now())",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed revoked client");

        let err = resolve_from_claims(&pool, &machine_claims("dead-client"))
            .await
            .expect_err("a revoked machine must be rejected");

        match err {
            ApiError::Unauthorized(msg) => {
                assert!(msg.contains("dead-client"), "message names the client id: {msg}");
                assert!(
                    msg.contains("revoked"),
                    "revoked must be distinguishable from unregistered (D7): {msg}"
                );
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn registered_machine_resolves_to_its_profile(pool: PgPool) {
        let profile_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, 'agent-live', 'agent-live', NULL, '{}')",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed profile");
        sqlx::query!(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES ('live-client', 'test', $1, $1)",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed client");

        let profile = resolve_from_claims(&pool, &machine_claims("live-client"))
            .await
            .expect("registered machine resolves");
        assert_eq!(profile.id, profile_id);

        // The gate touched last_seen_at.
        let seen = sqlx::query_scalar!(
            "SELECT last_seen_at FROM kb_machine_clients WHERE client_id = 'live-client'"
        )
        .fetch_one(&pool)
        .await
        .expect("read last_seen");
        assert!(seen.is_some(), "the gate records liveness");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(unregistered_machine) or test(revoked_machine) or test(registered_machine)'
```

Expected: `unregistered_machine_is_rejected_and_creates_no_profile` FAILS on the profile-count assertion (the old code created one). The other two FAIL because `kb_machine_clients` is never consulted.

- [ ] **Step 3: Replace `resolve_machine_from_claims` with the gate**

In `crates/temper-services/src/services/profile_service.rs`, replace the whole function at `:120`:

```rust
/// Machine path: the registration gate (D2). Lookup-or-reject — there is no
/// create branch, because `machine_registration_service::provision` creates the
/// agent profile ahead of the machine's first call (D3).
///
/// This function is the ONLY machine-principal entry point for both temper-api and
/// temper-mcp, which is why the gate lives here and not in an Axum middleware (D4):
/// temper-mcp does not share temper-api's middleware stack, so a middleware gate would
/// drift. Rejections are specific (D7) — the caller has already proven it holds a valid,
/// correctly-audienced token, so naming the client id and the reason leaks nothing.
async fn resolve_machine_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    let client_id = claims.external_user_id.as_str();

    let Some(client) = crate::services::machine_client_service::lookup_by_client_id(pool, client_id).await? else {
        tracing::warn!(client_id, "machine gate: rejected (unregistered client)");
        return Err(ApiError::Unauthorized(format!(
            "machine client '{client_id}' is not registered with this instance. \
             An administrator must run: temper admin machine provision --client-id {client_id} --label <label>"
        )));
    };

    if let Some(revoked_at) = client.revoked_at {
        tracing::warn!(client_id, %revoked_at, "machine gate: rejected (revoked client)");
        return Err(ApiError::Unauthorized(format!(
            "machine client '{client_id}' was revoked at {}",
            revoked_at.to_rfc3339()
        )));
    }

    // Coarse liveness (D9). Failure to touch must not fail the request.
    if let Err(err) = crate::services::machine_client_service::touch_last_seen(pool, client.id).await {
        tracing::warn!(client_id, ?err, "machine gate: last_seen_at touch failed (ignored)");
    }

    get_by_id(pool, ProfileId::from(client.profile_id)).await
}
```

- [ ] **Step 4: Make the creation helpers transaction-capable**

Registration needs profile + link + emitters in one transaction, so both helpers must take a connection rather than a pool. Change the signature at `:300`:

```rust
/// Create an agent profile and its default machine auth link. Email is SQL NULL
/// (a machine has none); display name / handle derive from the client id.
///
/// Takes a connection so registration can run it inside a transaction. No longer
/// called from the authentication path — `provision` owns it now (D3).
pub(crate) async fn create_agent_profile_and_link(
    conn: &mut sqlx::PgConnection,
    client_id: &str,
) -> ApiResult<(Uuid, String)> {
    let display_name = format!("agent-{client_id}");
    let handle = generate_profile_handle_conn(&mut *conn, &display_name).await?;
    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, NULL, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
    )
    .execute(&mut *conn)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
        VALUES ($1, $2, $3, $4, NULL, false, true, now())
        "#,
        auth_link_id,
        profile_id,
        crate::auth::MACHINE_PROVIDER_TAG,
        client_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok((profile_id, handle))
}
```

`generate_profile_handle` at `:32` currently takes `&PgPool`. Add a connection-taking twin next to it and make the pool version delegate, so the human path is untouched:

```rust
/// Connection-taking twin of `generate_profile_handle`, for use inside a transaction.
pub(crate) async fn generate_profile_handle_conn(
    conn: &mut sqlx::PgConnection,
    display_name: &str,
) -> ApiResult<String> {
    let base: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive dashes (matches SQL backfill regex [^a-zA-Z0-9]+)
    let base: String = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };

    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
        &base,
    )
    .fetch_one(&mut *conn)
    .await?;

    if !exists {
        return Ok(base);
    }

    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
            &candidate,
        )
        .fetch_one(&mut *conn)
        .await?;

        if !exists {
            return Ok(candidate);
        }
        suffix += 1;
    }
}
```

Then replace the existing `generate_profile_handle` (`:32`) body entirely with a delegate, so the handle-generation logic exists exactly once:

```rust
/// Generate a unique profile handle from a display name.
pub async fn generate_profile_handle(pool: &PgPool, display_name: &str) -> ApiResult<String> {
    let mut conn = pool.acquire().await?;
    generate_profile_handle_conn(&mut conn, display_name).await
}
```

Now change `provision_profile_entities` at `:340` to take a connection. Its body is unchanged except that every `.execute(pool)` becomes `.execute(&mut *conn)`:

```rust
pub(crate) async fn provision_profile_entities(
    conn: &mut sqlx::PgConnection,
    profile_id: Uuid,
    handle: &str,
) -> ApiResult<()> {
```

Its two existing callers must adapt. In `resolve_human_from_claims` at `:94` and wherever else it is called, replace `provision_profile_entities(pool, profile_id, &handle).await?` with:

```rust
    let mut conn = pool.acquire().await?;
    provision_profile_entities(&mut conn, profile_id, &handle).await?;
```

`create_agent_profile_and_link`'s old call site inside `resolve_machine_from_claims` is gone — the function you replaced in Step 3 no longer references it. Confirm with `rg -n 'create_agent_profile_and_link' crates/` that the only remaining callers are the registration service (Task 4) and tests.

- [ ] **Step 5: Fix the two existing profile_service tests that assert JIT**

`machine_first_sight_provisions_agent_profile` (`:710`) and `machine_resolution_is_idempotent` (`:732`) both assert the deleted behavior. `machine_first_sight_provisions_agent_profile` is **superseded** by `unregistered_machine_is_rejected_and_creates_no_profile` — delete it. Rewrite `machine_resolution_is_idempotent` to seed a registration first:

```rust
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn machine_resolution_is_idempotent(pool: PgPool) {
        let profile_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, 'agent-idem', 'agent-idem', NULL, '{}')",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed profile");
        sqlx::query!(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES ('agent-idem', 'test', $1, $1)",
            profile_id,
        )
        .execute(&pool)
        .await
        .expect("seed client");

        let c = machine_claims("agent-idem");
        let first = resolve_from_claims(&pool, &c).await.expect("first");
        let second = resolve_from_claims(&pool, &c).await.expect("second");
        assert_eq!(first.id, second.id, "resolution is stable across calls");
    }
```

- [ ] **Step 6: Invert the e2e test that asserts JIT**

`tests/e2e/tests/auth_seam_m2m_e2e.rs` asserts, through the real MCP gate, that a machine token provisions an agent profile. That is now false. Replace the test body (keep `build_mcp_service` and `machine_parts` exactly as they are):

```rust
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unregistered_machine_token_is_rejected_by_the_mcp_gate(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    let err = svc
        .ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect_err("an unregistered machine must be rejected at the mcp gate");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("not registered"),
        "the mcp surface inherits the services-layer gate (D4): {rendered}"
    );

    let links = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'auth0-m2m'",
    )
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(links, Some(0), "rejection creates no auth link");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn registered_machine_token_is_admitted_by_the_mcp_gate(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    let profile_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-steward', 'agent-steward', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('steward-client-1', 'test', $1, $1)",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed registration");

    svc.ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect("mcp gate must admit a registered machine");
}
```

Also update the module doc comment at the top of the file — it currently describes the JIT behavior:

```rust
//! Stage 4b: a machine (`client_credentials`) token, driven through the real mcp
//! gate `ensure_profile_from_parts`. Since G3 Phase A, registration is fail-closed:
//! an unregistered client is rejected and creates nothing; a registered one resolves
//! to its pre-created agent profile. temper-mcp inherits the gate from
//! `temper-services` — it has no gate of its own (D4).
```

- [ ] **Step 7: Run every affected tier**

```bash
cargo nextest run -p temper-services --features test-db
cargo build -p temper-cli --bin temper   # e2e spawns the binary; nextest will NOT rebuild it
cargo make test-e2e
```

Expected: all PASS. If `test-e2e` hangs at test-list enumeration on macOS, run the single target with `cargo test --test auth_seam_m2m_e2e --features test-db` instead.

- [ ] **Step 8: Regenerate caches, check, commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-e2e
cargo fmt --all
cargo make check
git add -u
git add $(git status --porcelain .sqlx crates/temper-services/.sqlx tests/e2e/.sqlx | awk '{print $2}')
git status --short
git commit -m "G3 Phase A: the gate — resolve_machine_from_claims is lookup-or-reject

Authentication no longer creates a profile. temper-mcp inherits the gate
from temper-services, so the two surfaces cannot drift. Inverts the e2e
test that asserted the JIT behavior."
```

Run `cargo fmt --all` before committing — the pre-commit hook checks formatting, and an incremental clippy pass can succeed locally where CI's clean build fails.

---

## Task 4: The registration service

**Files:**
- Create: `crates/temper-services/src/services/machine_registration_service.rs`
- Modify: `crates/temper-services/src/services/access_service.rs` (extract `insert_grant`)
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: `profile_service::{create_agent_profile_and_link, provision_profile_entities}` (Task 3); `temper_core::types::machine::{ProvisionMachineRequest, RebindMachineRequest, MachineClient}` (Task 2).
- Produces:
  - `machine_registration_service::provision(&PgPool, ProfileId, &ProvisionMachineRequest) -> ApiResult<MachineClient>`
  - `machine_registration_service::rebind(&PgPool, ProfileId, &RebindMachineRequest) -> ApiResult<MachineClient>`
  - `access_service::insert_grant(conn: &mut PgConnection, params: &InsertGrantParams) -> ApiResult<bool>`

Authorization (`is_system_admin`) is enforced by the **handler** (Task 5), matching how `promote_admin` documents its contract. These functions assume an authorized caller and record them as `registered_by_profile_id`.

- [ ] **Step 1: Extract a transaction-capable grant insert**

In `crates/temper-services/src/services/access_service.rs`, add above `grant_capability` (`:113`):

```rust
/// The columns of one `kb_access_grants` upsert. A params struct because the
/// insert takes seven domain values (repo rule: >5 ⇒ struct).
#[derive(Debug, Clone)]
pub struct InsertGrantParams {
    pub subject_table: String,
    pub subject_id: Uuid,
    pub principal_table: String,
    pub principal_id: Uuid,
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_grant: bool,
    pub granted_by_profile_id: Uuid,
}

/// Raw upsert of one access grant, on a connection so it can join a transaction.
/// **Performs no authorization** — every caller must gate first (auth before writes).
/// Returns whether the row was freshly inserted (`xmax = 0`) rather than updated.
pub async fn insert_grant(conn: &mut sqlx::PgConnection, p: &InsertGrantParams) -> ApiResult<bool> {
    let inserted = sqlx::query_scalar!(
        r#"INSERT INTO kb_access_grants
               (subject_table, subject_id, principal_table, principal_id,
                can_read, can_write, can_delete, can_grant, granted_by_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
           DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                         can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                         granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
           RETURNING (xmax = 0) AS "inserted!""#,
        p.subject_table,
        p.subject_id,
        p.principal_table,
        p.principal_id,
        p.can_read,
        p.can_write,
        p.can_delete,
        p.can_grant,
        p.granted_by_profile_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(inserted)
}
```

Then rewrite `grant_capability`'s body to keep its auth check and delegate the SQL — so the statement exists once:

```rust
pub async fn grant_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &GrantCapabilityRequest,
) -> ApiResult<GrantOutcome> {
    if !can_administer_grant(pool, caller, &req.subject_table, req.subject_id).await? {
        return Err(ApiError::Forbidden);
    }
    let mut conn = pool.acquire().await?;
    let granted = insert_grant(
        &mut conn,
        &InsertGrantParams {
            subject_table: req.subject_table.clone(),
            subject_id: req.subject_id,
            principal_table: req.principal_table.clone(),
            principal_id: req.principal_id,
            can_read: req.can_read,
            can_write: req.can_write,
            can_delete: req.can_delete,
            can_grant: req.can_grant,
            granted_by_profile_id: *caller,
        },
    )
    .await?;
    Ok(GrantOutcome { granted })
}
```

- [ ] **Step 2: Write the failing registration tests**

Create `crates/temper-services/src/services/machine_registration_service.rs` with only its test module:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    use temper_core::types::ids::ProfileId;
    use temper_core::types::machine::{GrantSpec, ProvisionMachineRequest, RebindMachineRequest, TeamSpec};

    use crate::services::machine_registration_service as svc;

    async fn seed_admin(pool: &PgPool) -> ProfileId {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, 'admin', 'Admin', 'admin@example.test', '{}')",
            id,
        )
        .execute(pool)
        .await
        .expect("seed admin");
        ProfileId::from(id)
    }

    fn req(client_id: &str) -> ProvisionMachineRequest {
        ProvisionMachineRequest {
            client_id: client_id.to_string(),
            label: "steward".to_string(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_creates_profile_link_emitters_and_registration(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let client = svc::provision(&pool, admin, &req("acme-agent")).await.expect("provision");

        assert_eq!(client.client_id, "acme-agent");
        assert_eq!(client.issuer, "auth0-m2m");
        assert_eq!(client.registered_by_profile_id, *admin);
        assert!(client.revoked_at.is_none());

        let link = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links \
              WHERE auth_provider = 'auth0-m2m' AND auth_provider_user_id = 'acme-agent'",
        )
        .fetch_one(&pool)
        .await
        .expect("count link");
        assert_eq!(link, Some(1));

        let emitters = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_entities WHERE profile_id = $1",
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count emitters");
        assert_eq!(emitters, Some(4), "one emitter per Surface::ALL variant");
    }

    /// D14: the trigger auto-joins only while access_mode='open'. provision must not
    /// depend on it, or every machine 403s the day the instance flips to invite_only.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_enrolls_the_agent_in_the_gating_team(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        sqlx::query!("UPDATE kb_system_settings SET access_mode = 'invite_only'")
            .execute(&pool)
            .await
            .expect("flip to invite_only");

        let client = svc::provision(&pool, admin, &req("gated-agent")).await.expect("provision");

        let has_access = sqlx::query_scalar!("SELECT has_system_access($1)", client.profile_id)
            .fetch_one(&pool)
            .await
            .expect("has_system_access");
        assert_eq!(
            has_access,
            Some(true),
            "a provisioned machine must pass the system gate under invite_only (D14)"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_applies_explicit_team_and_cogmap_reach(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let team_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'acme', 'Acme')",
            team_id,
        )
        .execute(&pool)
        .await
        .expect("seed team");

        let cogmap_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_cogmaps (id, name) VALUES ($1, 'Acme Map')",
            cogmap_id,
        )
        .execute(&pool)
        .await
        .expect("seed cogmap");

        let request = ProvisionMachineRequest {
            client_id: "reach-agent".to_string(),
            label: "steward".to_string(),
            owner_team_id: Some(team_id),
            teams: vec![TeamSpec { team_id, role: "member".to_string() }],
            grants: vec![GrantSpec { cogmap_id, can_write: true }],
        };
        let client = svc::provision(&pool, admin, &request).await.expect("provision");

        assert_eq!(client.team_id, Some(team_id), "owner is recorded");

        let member = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            team_id,
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count membership");
        assert_eq!(member, Some(1));

        let grant = sqlx::query!(
            "SELECT can_read, can_write FROM kb_access_grants \
              WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 \
                AND principal_table = 'kb_profiles' AND principal_id = $2",
            cogmap_id,
            client.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("grant row");
        assert!(grant.can_read && grant.can_write, "write implies read (DB coherence CHECK)");
    }

    /// The regression test for the silent identity fork (D8).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_preserves_the_agent_profile_and_revokes_the_old_client(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(&pool, admin, &req("old-client")).await.expect("provision");

        let new = svc::rebind(
            &pool,
            admin,
            &RebindMachineRequest {
                client_id: "new-client".to_string(),
                from_machine_client_id: old.id,
                label: "steward (rotated)".to_string(),
                keep_old_active: false,
            },
        )
        .await
        .expect("rebind");

        assert_eq!(
            new.profile_id, old.profile_id,
            "a rotated application must not fork the machine's identity"
        );

        let old_row = crate::services::machine_client_service::get(&pool, old.id)
            .await
            .expect("old row");
        assert!(old_row.revoked_at.is_some(), "the old client is revoked in the same transaction");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rebind_with_keep_old_active_leaves_an_overlap_window(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let old = svc::provision(&pool, admin, &req("overlap-old")).await.expect("provision");

        svc::rebind(
            &pool,
            admin,
            &RebindMachineRequest {
                client_id: "overlap-new".to_string(),
                from_machine_client_id: old.id,
                label: "steward".to_string(),
                keep_old_active: true,
            },
        )
        .await
        .expect("rebind");

        let old_row = crate::services::machine_client_service::get(&pool, old.id)
            .await
            .expect("old row");
        assert!(old_row.revoked_at.is_none(), "--no-revoke-old keeps both credentials live");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provisioning_a_duplicate_client_id_is_a_conflict(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        svc::provision(&pool, admin, &req("dupe")).await.expect("first");
        let err = svc::provision(&pool, admin, &req("dupe")).await.expect_err("second must fail");
        assert!(matches!(err, crate::error::ApiError::Conflict(_)), "got {err:?}");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db machine_registration_service
```

Expected: FAIL to compile — `svc::provision` / `svc::rebind` do not exist. Register the module first (`pub mod machine_registration_service;` in `services/mod.rs`) so you get *that* error rather than an unresolved-module error.

- [ ] **Step 4: Implement the registration service**

Prepend to `crates/temper-services/src/services/machine_registration_service.rs`:

```rust
//! Transactional registration of machine principals.
//!
//! `provision` is the inversion (D3): it creates the agent profile, its auth link, its
//! emitter entities, its gating-team membership, its explicit reach, and the
//! `kb_machine_clients` row — all in ONE transaction, ahead of the machine's first call.
//!
//! Authorization is the caller's job. Handlers gate on `is_system_admin` before calling
//! (auth before writes); these functions record the authorized caller as
//! `registered_by_profile_id`.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};

use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{insert_grant, InsertGrantParams};
use crate::services::machine_client_service;
use crate::services::profile_service;

/// Enroll `profile_id` in the configured gating team as `watcher`.
///
/// D14: `trg_sync_system_membership` auto-joins new profiles ONLY while
/// `access_mode = 'open'`, because `has_system_access` short-circuits true under that
/// mode. Under `invite_only` it enrolls nothing, and an unenrolled machine authenticates
/// and then 403s at `require_system_access`. So we enroll explicitly, exactly as
/// `access_service::review_request` does for an approved human. Never depend on the
/// trigger: its behavior is a function of a setting that is about to change.
async fn enroll_in_gating_team(conn: &mut sqlx::PgConnection, profile_id: Uuid) -> ApiResult<()> {
    let slug = sqlx::query_scalar!("SELECT gating_team_slug FROM kb_system_settings LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?
        .flatten();

    let Some(slug) = slug else {
        // No gating team configured ⇒ nothing to enroll into. `update_system_settings`
        // already rejects `invite_only` with an empty slug, so this is the open-mode case.
        return Ok(());
    };

    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
           SELECT t.id, $2, 'watcher'::team_role FROM kb_teams t WHERE t.slug = $1
           ON CONFLICT (team_id, profile_id) DO NOTHING"#,
        slug,
        profile_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Apply the explicit reach: team memberships and cogmap grants. Reach is plural and
/// never inferred from `owner_team_id` (D10, D6).
async fn apply_reach(
    conn: &mut sqlx::PgConnection,
    caller: ProfileId,
    profile_id: Uuid,
    req: &ProvisionMachineRequest,
) -> ApiResult<()> {
    for team in &req.teams {
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, $3::text::team_role)
               ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role"#,
            team.team_id,
            profile_id,
            team.role,
        )
        .execute(&mut *conn)
        .await?;
    }

    for grant in &req.grants {
        insert_grant(
            &mut *conn,
            &InsertGrantParams {
                subject_table: "kb_cogmaps".to_string(),
                subject_id: grant.cogmap_id,
                principal_table: "kb_profiles".to_string(),
                principal_id: profile_id,
                // Write implies read — the DB's coherence CHECK enforces it anyway.
                can_read: true,
                can_write: grant.can_write,
                can_delete: false,
                can_grant: false,
                granted_by_profile_id: *caller,
            },
        )
        .await?;
    }

    Ok(())
}

/// Both unique constraints a duplicate `client_id` can trip. The auth-link one fires
/// first, because `create_agent_profile_and_link` inserts before the registration row.
const DUPLICATE_CONSTRAINTS: [&str; 2] = [
    "kb_machine_clients_client_id_key",
    "kb_profile_auth_links_auth_provider_auth_provider_user_id_key",
];

/// Name the client id in a duplicate-registration conflict.
///
/// `From<sqlx::Error> for ApiError` already maps SQLSTATE 23505 to
/// `Conflict("Resource already exists")`, so this is purely about the message: an operator
/// registering a client that already exists should be told *which* one. Any other error
/// falls through to the standard mapping.
fn map_duplicate(err: sqlx::Error, client_id: &str) -> ApiError {
    if let sqlx::Error::Database(ref db) = err {
        if db.constraint().is_some_and(|c| DUPLICATE_CONSTRAINTS.contains(&c)) {
            return ApiError::Conflict(format!("machine client '{client_id}' is already registered"));
        }
    }
    ApiError::from(err)
}

/// Register a new machine principal, creating its agent profile. One transaction.
pub async fn provision(
    pool: &PgPool,
    caller: ProfileId,
    req: &ProvisionMachineRequest,
) -> ApiResult<MachineClient> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // Auth before writes is the handler's job; this is the friendly-conflict check. It is
    // NOT the race guard — two concurrent provisions both pass it. The unique constraints
    // are the guard, and `map_duplicate` turns either one into a 409 naming the client id.
    if machine_client_service::lookup_by_client_id(pool, &req.client_id).await?.is_some() {
        return Err(ApiError::Conflict(format!(
            "machine client '{}' is already registered",
            req.client_id
        )));
    }

    let (profile_id, handle) =
        profile_service::create_agent_profile_and_link(&mut tx, &req.client_id)
            .await
            .map_err(|e| match e {
                // The auth-link unique constraint fires before the registration row's.
                ApiError::Conflict(_) => ApiError::Conflict(format!(
                    "machine client '{}' is already registered",
                    req.client_id
                )),
                other => other,
            })?;

    profile_service::provision_profile_entities(&mut tx, profile_id, &handle).await?;
    enroll_in_gating_team(&mut tx, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, req).await?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
           VALUES ($1, 'auth0-m2m', $2, $3, $4, $5)
           RETURNING id"#,
        req.client_id,
        req.label,
        profile_id,
        req.owner_team_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    machine_client_service::get(pool, id).await
}

/// Point a fresh `client_id` at an EXISTING agent profile, revoking the old row in the
/// same transaction unless an overlap window was requested (D8).
///
/// Binding is only ever to an agent profile already reached through a machine auth link —
/// never to a human's profile. That narrow case is the whole reason `rebind` is safe; see
/// the spec's Rejected section.
pub async fn rebind(
    pool: &PgPool,
    caller: ProfileId,
    req: &RebindMachineRequest,
) -> ApiResult<MachineClient> {
    let old = machine_client_service::get(pool, req.from_machine_client_id).await?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // A second auth link for the same profile, under the new client id.
    sqlx::query!(
        r#"INSERT INTO kb_profile_auth_links
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
           VALUES ($1, $2, $3, $4, NULL, false, false, now())"#,
        Uuid::now_v7(),
        old.profile_id,
        crate::auth::MACHINE_PROVIDER_TAG,
        req.client_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
           VALUES ($1, 'auth0-m2m', $2, $3, $4, $5)
           RETURNING id"#,
        req.client_id,
        req.label,
        old.profile_id,
        old.team_id,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &req.client_id))?;

    if !req.keep_old_active {
        sqlx::query!(
            r#"UPDATE kb_machine_clients
                  SET revoked_at = now(), revoked_by_profile_id = $2
                WHERE id = $1 AND revoked_at IS NULL"#,
            old.id,
            *caller,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    machine_client_service::get(pool, id).await
}
```

> **Implementer note on the in-flight race.** `rebind` revokes the old row while a
> concurrent authentication may be resolving it. Postgres' read-committed default means
> that authentication sees either the pre-revocation row (admitted) or the post-revocation
> one (401, and the caller retries with the new credential). Both are safe; no lock is
> needed. Do not add `SELECT ... FOR UPDATE`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db machine_registration_service
```

Expected: PASS, 6 tests. If `provision_enrolls_the_agent_in_the_gating_team` fails with `has_system_access = false`, the `enroll_in_gating_team` slug lookup found no gating team — check that the canonical migration seeds `kb_system_settings.gating_team_slug = 'temper-system'` and that `temper-system` exists (migration `20260625000001` creates it).

- [ ] **Step 6: Regenerate caches, check, commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo fmt --all
cargo make check
git add crates/temper-services/src/services/machine_registration_service.rs \
        crates/temper-services/src/services/access_service.rs \
        crates/temper-services/src/services/mod.rs
git add $(git status --porcelain .sqlx crates/temper-services/.sqlx | awk '{print $2}')
git status --short
git commit -m "G3 Phase A: transactional provision + rebind, and gating-team enrollment (D14)"
```

---

## Task 5: The API surface

**Files:**
- Create: `crates/temper-api/src/handlers/machine_clients.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: `machine_registration_service::{provision, rebind}`, `machine_client_service::{list, get, revoke}`.
- Produces: `POST/GET /api/machine-clients`, `GET/DELETE /api/machine-clients/{id}`, `POST /api/machine-clients/{id}/rebind`.

Mounted with plain `.route()` in the **gated** router, matching `/api/access/admin/*` — operator-only, out of the public OpenAPI contract. No `#[utoipa::path]`, so no `operation_id` obligation from P5.

- [ ] **Step 1: Write the handlers**

Create `crates/temper-api/src/handlers/machine_clients.rs`:

```rust
//! Operator-only machine-client registration. Out of the OpenAPI contract (plain
//! `.route()` mounting), like `/api/access/admin/*`.
//!
//! **The `is_system_admin` check here is load-bearing, not defense-in-depth.** Production
//! runs `access_mode = 'open'`, under which `has_system_access` is true for every profile,
//! so `require_system_access` on the gated router admits everyone. This check is the only
//! thing protecting these endpoints (D12).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};
use temper_core::types::AuthenticatedProfile;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::{access_service, machine_client_service, machine_registration_service};
use temper_services::state::AppState;

/// Query flags for `GET /api/machine-clients`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_revoked: bool,
}

/// Auth before writes: reject a non-admin before any mutation runs.
async fn require_admin(state: &AppState, authed: &AuthenticatedProfile) -> ApiResult<ProfileId> {
    let caller = ProfileId::from(authed.profile.id);
    if !access_service::is_system_admin(&state.pool, caller).await? {
        return Err(ApiError::Forbidden);
    }
    Ok(caller)
}

pub async fn provision(
    State(state): State<AppState>,
    axum::Extension(authed): axum::Extension<AuthenticatedProfile>,
    Json(body): Json<ProvisionMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &authed).await?;
    let client = machine_registration_service::provision(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn rebind(
    State(state): State<AppState>,
    axum::Extension(authed): axum::Extension<AuthenticatedProfile>,
    Path(id): Path<Uuid>,
    Json(mut body): Json<RebindMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &authed).await?;
    // The path segment is authoritative for which client is being rotated away from.
    body.from_machine_client_id = id;
    let client = machine_registration_service::rebind(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn list(
    State(state): State<AppState>,
    axum::Extension(authed): axum::Extension<AuthenticatedProfile>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<MachineClient>>> {
    require_admin(&state, &authed).await?;
    Ok(Json(machine_client_service::list(&state.pool, q.include_revoked).await?))
}

pub async fn get(
    State(state): State<AppState>,
    axum::Extension(authed): axum::Extension<AuthenticatedProfile>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    require_admin(&state, &authed).await?;
    Ok(Json(machine_client_service::get(&state.pool, id).await?))
}

pub async fn revoke(
    State(state): State<AppState>,
    axum::Extension(authed): axum::Extension<AuthenticatedProfile>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &authed).await?;
    Ok(Json(machine_client_service::revoke(&state.pool, id, caller).await?))
}
```

Add to `crates/temper-api/src/handlers/mod.rs`:

```rust
pub mod machine_clients;
```

- [ ] **Step 2: Mount the routes**

In `crates/temper-api/src/routes.rs`, inside `gated_routes()`, append after the `/api/access/admin/promote` route:

```rust
        .route(
            "/api/machine-clients",
            get(handlers::machine_clients::list).post(handlers::machine_clients::provision),
        )
        .route(
            "/api/machine-clients/{id}",
            get(handlers::machine_clients::get).delete(handlers::machine_clients::revoke),
        )
        .route(
            "/api/machine-clients/{id}/rebind",
            post(handlers::machine_clients::rebind),
        )
```

The `use axum::routing::{get, patch, post};` line already at the top of `gated_routes` covers `get`/`post`. `delete` must be added to that import list.

- [ ] **Step 3: Verify the OpenAPI contract is unchanged**

```bash
cargo make check-openapi-spec
```

Expected: PASS. Plain `.route()` mounting keeps these paths out of the emitted spec, so the committed `openapi.json` must not change. **If it changed, you used `routes!()` — revert to `.route()`.**

- [ ] **Step 4: Build, check, commit**

```bash
cargo fmt --all
cargo make check
git add crates/temper-api/src/handlers/machine_clients.rs \
        crates/temper-api/src/handlers/mod.rs crates/temper-api/src/routes.rs
git commit -m "G3 Phase A: /api/machine-clients, is_system_admin-gated"
```

---

## Task 6: The client

**Files:**
- Create: `crates/temper-client/src/machine.rs`
- Modify: `crates/temper-client/src/lib.rs`

**Interfaces:**
- Consumes: the routes from Task 5; `temper_core::types::machine::*`.
- Produces: `TemperClient::machine_clients() -> MachineClientsClient`, with `provision`, `rebind`, `list`, `get`, `revoke`.

- [ ] **Step 1: Write the sub-client**

Create `crates/temper-client/src/machine.rs`, following `access.rs` exactly:

```rust
//! Typed sub-client for the operator-only `/api/machine-clients` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::machine::{MachineClient, ProvisionMachineRequest, RebindMachineRequest};

/// Sub-client for machine-principal registration.
pub struct MachineClientsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for MachineClientsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineClientsClient").finish_non_exhaustive()
    }
}

impl<'a> MachineClientsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Register a new machine principal, creating its agent profile.
    pub async fn provision(&self, body: &ProvisionMachineRequest) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/machine-clients").json(body);
        self.http
            .send_json(&Method::POST, "/api/machine-clients", req, Some(&token))
            .await
    }

    /// Point a fresh client id at an existing agent profile.
    pub async fn rebind(&self, id: Uuid, body: &RebindMachineRequest) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/machine-clients/{id}/rebind");
        let req = self.http.post(&path).json(body);
        self.http.send_json(&Method::POST, &path, req, Some(&token)).await
    }

    /// Enumerate registered clients.
    pub async fn list(&self, include_revoked: bool) -> Result<Vec<MachineClient>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/machine-clients?include_revoked={include_revoked}");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }

    /// Load one registered client.
    pub async fn get(&self, id: Uuid) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/machine-clients/{id}");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }

    /// Revoke a client. Denies authentication; grants and memberships survive (D11).
    pub async fn revoke(&self, id: Uuid) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/machine-clients/{id}");
        let req = self.http.delete(&path);
        self.http.send_json(&Method::DELETE, &path, req, Some(&token)).await
    }
}
```

> **Implementer:** confirm `HttpClient` exposes `get`, `post`, and `delete` with these
> shapes by reading `crates/temper-client/src/http.rs`. `access.rs` uses
> `self.http.delete("/api/access/requests/me")`, so `delete` exists.

- [ ] **Step 2: Register the sub-client**

In `crates/temper-client/src/lib.rs`, add `pub mod machine;` beside the other module declarations, and next to `pub fn admin` (`:161`):

```rust
    /// Operator-only machine-principal registration.
    pub fn machine_clients(&self) -> machine::MachineClientsClient<'_> {
        machine::MachineClientsClient::new(&self.http)
    }
```

- [ ] **Step 3: Build and commit**

```bash
cargo fmt --all
cargo make check
git add crates/temper-client/src/machine.rs crates/temper-client/src/lib.rs
git commit -m "G3 Phase A: temper-client machine_clients sub-client"
```

---

## Task 7: The CLI

**Files:**
- Create: `crates/temper-cli/src/commands/admin_machine.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`, `crates/temper-cli/src/cli.rs`

**Interfaces:**
- Consumes: `TemperClient::machine_clients()` (Task 6); `crate::actions::cogmap::resolve_team_id` (existing, used by `promote_remote`).
- Produces: `temper admin machine provision | rebind | list | show | revoke`.

Commands parse args and call into the client; no business logic here (thin commands, fat actions).

- [ ] **Step 1: Add the clap surface**

In `crates/temper-cli/src/cli.rs`, add to `AdminAction` (`:815`), after the `Saml` variant:

```rust
    /// Register and rotate machine (client_credentials) principals
    Machine {
        #[command(subcommand)]
        action: AdminMachineAction,
    },
```

And add the new enum next to `AdminSamlAction` (`:859`):

```rust
#[derive(Debug, clap::Subcommand)]
pub enum AdminMachineAction {
    /// Register a machine principal: creates its agent profile, emitters, gating-team
    /// membership, and the reach you specify. Run this BEFORE the machine's first call.
    Provision {
        /// The IdP client id (Auth0 M2M application client id)
        #[arg(long = "client-id")]
        client_id: String,
        /// Human-facing label
        #[arg(long)]
        label: String,
        /// Team recorded as this machine's OWNER. Not its reach.
        #[arg(long = "owner-team")]
        owner_team: Option<String>,
        /// Team to enroll in, as `<ref>` or `<ref>:<role>` (role defaults to `member`).
        /// Repeatable. Reach is plural and never inferred from --owner-team.
        #[arg(long = "team")]
        teams: Vec<String>,
        /// Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable.
        #[arg(long = "cogmap")]
        cogmaps: Vec<String>,
    },
    /// Point a fresh client id at an existing agent profile, preserving its authorship
    /// history. Revokes the old client unless --no-revoke-old.
    Rebind {
        /// The machine client being rotated away from (its `id`, from `list`)
        from: String,
        /// The new IdP client id
        #[arg(long = "client-id")]
        client_id: String,
        /// Label for the new registration
        #[arg(long)]
        label: String,
        /// Leave both credentials live for an overlap window
        #[arg(long = "no-revoke-old")]
        no_revoke_old: bool,
    },
    /// List registered machine clients
    List {
        /// Include revoked clients
        #[arg(long = "include-revoked")]
        include_revoked: bool,
    },
    /// Show one machine client
    Show { id: String },
    /// Revoke a machine client. Denies authentication; grants and memberships survive.
    Revoke { id: String },
}
```

- [ ] **Step 2: Write the failing spec parser test**

The `--team acme:member` / `--cogmap map-uuid:ro` suffix parsing is the only logic in this file, so it gets a unit test. Create `crates/temper-cli/src/commands/admin_machine.rs`:

```rust
//! `temper admin machine` — operator-only machine-principal registration.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_spec_defaults_to_member() {
        assert_eq!(split_spec("acme"), ("acme".to_string(), None));
        assert_eq!(split_spec("acme:owner"), ("acme".to_string(), Some("owner".to_string())));
    }

    /// A decorated ref is `sluggify(title)-<uuid>` — it contains hyphens but no colon,
    /// so splitting on the LAST colon is safe. A UUID contains no colon either.
    #[test]
    fn split_spec_does_not_mangle_decorated_refs() {
        let r = "temper-self-cognition-019f2391-e001-7933-b88a-28fb92e56ac1";
        assert_eq!(split_spec(r), (r.to_string(), None));
        assert_eq!(
            split_spec(&format!("{r}:ro")),
            (r.to_string(), Some("ro".to_string()))
        );
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

```bash
cargo nextest run -p temper-cli split_spec
```

Expected: FAIL to compile — `split_spec` is undefined.

- [ ] **Step 4: Implement the command module**

Prepend to `crates/temper-cli/src/commands/admin_machine.rs`:

```rust
//! `temper admin machine` — operator-only machine-principal registration.
//!
//! Thin commands: parse, resolve refs to ids, call the client, render. Reach
//! (`--team`, `--cogmap`) is explicit and repeatable and is never inferred from
//! `--owner-team`, which records only the machine's owner.

use temper_core::types::machine::{GrantSpec, ProvisionMachineRequest, RebindMachineRequest, TeamSpec};
use temper_core::TemperError;

use crate::error::Result;
use crate::format::OutputFormat;

/// Split `"<ref>"` or `"<ref>:<suffix>"` on the LAST colon. Neither a UUID nor a
/// decorated `slug-<uuid>` ref contains a colon, so this cannot mangle a ref.
fn split_spec(raw: &str) -> (String, Option<String>) {
    match raw.rsplit_once(':') {
        Some((head, tail)) => (head.to_string(), Some(tail.to_string())),
        None => (raw.to_string(), None),
    }
}

fn parse_uuid(what: &str, raw: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw)
        .map_err(|e| TemperError::Api(format!("invalid {what} '{raw}': {e}")).into())
}

/// Register a machine principal.
pub async fn provision_remote(
    client: &temper_client::TemperClient,
    client_id: &str,
    label: &str,
    owner_team: Option<&str>,
    teams: &[String],
    cogmaps: &[String],
    fmt: OutputFormat,
) -> Result<()> {
    let owner_team_id = match owner_team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let mut team_specs = Vec::with_capacity(teams.len());
    for raw in teams {
        let (team_ref, role) = split_spec(raw);
        team_specs.push(TeamSpec {
            team_id: crate::actions::cogmap::resolve_team_id(client, &team_ref).await?,
            role: role.unwrap_or_else(|| "member".to_string()),
        });
    }

    let mut grant_specs = Vec::with_capacity(cogmaps.len());
    for raw in cogmaps {
        let (cogmap_ref, mode) = split_spec(raw);
        // `parse_ref` returns `Result<ResourceId, TemperError>`; `ResourceId` is a newtype
        // over `Uuid`. Resolution is trailing-UUID-only, so a stale slug half is harmless.
        let cogmap_id = temper_workflow::operations::parse_ref(&cogmap_ref)
            .map_err(|e| TemperError::Api(format!("invalid cogmap ref '{cogmap_ref}': {e}")))?
            .0;
        grant_specs.push(GrantSpec {
            cogmap_id,
            can_write: mode.as_deref() != Some("ro"),
        });
    }

    let req = ProvisionMachineRequest {
        client_id: client_id.to_string(),
        label: label.to_string(),
        owner_team_id,
        teams: team_specs,
        grants: grant_specs,
    };
    let row = client
        .machine_clients()
        .provision(&req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Rotate an application: bind a new client id to the existing agent profile.
pub async fn rebind_remote(
    client: &temper_client::TemperClient,
    from: &str,
    client_id: &str,
    label: &str,
    no_revoke_old: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let from_id = parse_uuid("machine client id", from)?;
    let req = RebindMachineRequest {
        client_id: client_id.to_string(),
        from_machine_client_id: from_id,
        label: label.to_string(),
        keep_old_active: no_revoke_old,
    };
    let row = client
        .machine_clients()
        .rebind(from_id, &req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Enumerate registered clients.
pub async fn list_remote(
    client: &temper_client::TemperClient,
    include_revoked: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let rows = client
        .machine_clients()
        .list(include_revoked)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&rows, fmt)?);
    Ok(())
}

/// Show one registered client.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .machine_clients()
        .get(parse_uuid("machine client id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Revoke a client. Denies authentication; grants and memberships survive (D11) —
/// which is exactly what lets `rebind` inherit reach.
pub async fn revoke_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .machine_clients()
        .revoke(parse_uuid("machine client id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    crate::output::hint("Grants and team memberships were NOT removed — revocation denies authentication only.");
    Ok(())
}
```

Add `pub mod admin_machine;` to `crates/temper-cli/src/commands/mod.rs`.

- [ ] **Step 5: Wire the dispatch**

Find where `AdminAction::Saml` is dispatched (search `AdminAction::` in the CLI's command dispatch, alongside `Commands::Admin`) and add a sibling arm. Every action needs a client, so use the existing `runtime::with_client` helper — the same one `promote_remote`'s caller uses:

```rust
        AdminAction::Machine { action } => match action {
            AdminMachineAction::Provision { client_id, label, owner_team, teams, cogmaps } => {
                runtime::with_client(|client| {
                    Box::pin(async move {
                        commands::admin_machine::provision_remote(
                            client, &client_id, &label, owner_team.as_deref(), &teams, &cogmaps, fmt,
                        )
                        .await
                    })
                })
            }
            AdminMachineAction::Rebind { from, client_id, label, no_revoke_old } => {
                runtime::with_client(|client| {
                    Box::pin(async move {
                        commands::admin_machine::rebind_remote(
                            client, &from, &client_id, &label, no_revoke_old, fmt,
                        )
                        .await
                    })
                })
            }
            AdminMachineAction::List { include_revoked } => runtime::with_client(|client| {
                Box::pin(async move { commands::admin_machine::list_remote(client, include_revoked, fmt).await })
            }),
            AdminMachineAction::Show { id } => runtime::with_client(|client| {
                Box::pin(async move { commands::admin_machine::show_remote(client, &id, fmt).await })
            }),
            AdminMachineAction::Revoke { id } => runtime::with_client(|client| {
                Box::pin(async move { commands::admin_machine::revoke_remote(client, &id, fmt).await })
            }),
        },
```

> **Implementer:** match the surrounding arms' exact shape — read how `AdminAction::Promote`
> is dispatched and mirror it, including how `fmt` is obtained. `with_client` is the
> required helper for client-dependent async; never construct a raw `Runtime::new()`.

- [ ] **Step 6: Run the tests, then verify the binary really exposes the commands**

```bash
cargo nextest run -p temper-cli split_spec
cargo build -p temper-cli --bin temper
./target/debug/temper admin machine --help
./target/debug/temper admin machine provision --help
```

Expected: PASS, then both `--help` invocations print. A command surface that only exists in a plan is not a command surface — run the real binary.

- [ ] **Step 7: Check and commit**

```bash
cargo fmt --all
cargo make check
git add crates/temper-cli/src/commands/admin_machine.rs \
        crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/cli.rs
git commit -m "G3 Phase A: temper admin machine provision/rebind/list/show/revoke"
```

---

## Task 8: End-to-end coverage and documentation

A `test-db`-green result is a false signal for a change to authentication semantics. This task proves the gate through a real Axum server and a real MCP service.

**Files:**
- Create: `tests/e2e/tests/machine_gate_e2e.rs`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing further.

- [ ] **Step 1: Add a machine-token signer to the e2e harness**

`tests/e2e/tests/common/mod.rs` has `generate_test_jwt(sub, email)` (`:129`), whose `TestClaims`
struct carries no `gty`/`azp` — so it cannot mint a machine token. Add a sibling. Place it directly
after `generate_test_jwt`:

```rust
/// JWT claims for a machine (`client_credentials`) test token. `gty` is the definitive
/// machine signal `normalize_machine` keys on; `azp` carries the client id. No email:
/// a machine has none.
#[derive(Debug, Serialize, Deserialize)]
struct MachineTestClaims {
    sub: String,
    azp: String,
    gty: String,
    iss: String,
    iat: i64,
    exp: i64,
}

/// Sign a machine JWT with the test RSA private key. Valid for 1 hour. The claim shape
/// mirrors the real Auth0 `client_credentials` token pinned by `normalize.rs`'s
/// known-answer test.
pub fn generate_machine_jwt(client_id: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = MachineTestClaims {
        sub: format!("{client_id}@clients"),
        azp: client_id.to_string(),
        gty: "client-credentials".to_string(),
        iss: "test-issuer".to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign machine JWT")
}
```

- [ ] **Step 2: Write the e2e test**

Create `tests/e2e/tests/machine_gate_e2e.rs`. The harness exposes `common::setup(pool) -> E2eTestApp`
and `app.url(path)`; requests go through a plain `reqwest::Client`, as in `auth_seam_parity_e2e.rs`.

```rust
#![cfg(feature = "test-db")]
//! G3 Phase A: the machine-principal registration gate, proven end to end.
//!
//! `test-db` alone is a false signal for a change to authentication semantics, so this
//! drives a real Axum server. The MCP side of the same gate is covered by
//! `auth_seam_m2m_e2e.rs` — both surfaces resolve machines through the one
//! `temper-services` function, which is the point (D4).

mod common;

use uuid::Uuid;

/// Register a machine client against a freshly created agent profile.
async fn register(pool: &sqlx::PgPool, client_id: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'e2e', $2, $2)",
        client_id,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed registration");
    profile_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unregistered_machine_is_rejected_by_the_http_surface(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_machine_jwt("ghost-client");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        401,
        "an unregistered machine must not reach the data plane"
    );
    let body = response.text().await.expect("body");
    assert!(body.contains("not registered"), "the rejection names the reason: {body}");
    assert!(body.contains("ghost-client"), "the rejection names the client id: {body}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn registered_machine_reaches_the_data_plane(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register(&pool, "live-client").await;
    let token = common::generate_machine_jwt("live-client");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        200,
        "a registered machine authenticates and passes the system gate"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn revoked_machine_is_rejected_immediately(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register(&pool, "doomed-client").await;
    let token = common::generate_machine_jwt("doomed-client");
    let http = reqwest::Client::new();

    let before = http
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(before.status(), 200);

    sqlx::query!("UPDATE kb_machine_clients SET revoked_at = now() WHERE client_id = 'doomed-client'")
        .execute(&pool)
        .await
        .expect("revoke");

    // The SAME token — still cryptographically valid, still unexpired — is now dead.
    // Revocation does not wait for the token to expire, and does not need Auth0.
    let after = http
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(after.status(), 401, "revocation takes effect on the next call");
    assert!(after.text().await.expect("body").contains("revoked"));
}
```

> **Implementer:** `registered_machine_reaches_the_data_plane` expects 200, which requires the
> seeded agent profile to pass `require_system_access`. In a test database `access_mode` is `open`
> (the migration default), so it does. Do **not** "fix" a 403 here by loosening the gate — if you
> see one, the migration default changed, and the correct fix is to enroll the seeded profile in
> the gating team exactly as `machine_registration_service::enroll_in_gating_team` does.

- [ ] **Step 3: Run the e2e tier**

```bash
cargo build -p temper-cli --bin temper   # nextest will NOT rebuild the spawned binary
cargo make test-e2e
```

Expected: PASS, including the two rewritten tests in `auth_seam_m2m_e2e.rs`. On macOS, a freshly built e2e binary can hang at nextest's `--list`; if so run `cargo test --test machine_gate_e2e --features test-db`.

- [ ] **Step 4: Regenerate the e2e sqlx cache**

```bash
cargo make prepare-e2e
```

- [ ] **Step 5: Document the gate in CLAUDE.md**

In `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md`, add to the **Key Patterns** list, after the `Cloud operations` bullet:

```markdown
- **Machine principals are registered, not discovered** — a `client_credentials` token
  authenticates only if its `client_id` appears in `kb_machine_clients` and is not revoked.
  `resolve_machine_from_claims` is lookup-or-401; there is no JIT create branch. The gate lives in
  `temper-services` (not middleware) so temper-api and temper-mcp cannot drift. Register with
  `temper admin machine provision --client-id <id> --label <l> [--team <ref>[:role]]... [--cogmap <ref>[:ro]]...`
  — reach is plural and never inferred from `--owner-team`, which records the machine's *owner* and
  is never consulted for authorization. Rotating the IdP *secret* needs no temper action (the
  `client_id` is unchanged, so authorship history stays continuous); rotating the IdP *application*
  needs `temper admin machine rebind`, which binds the new `client_id` to the existing agent profile.
  `revoke` denies authentication and nothing else — grants and memberships hang off the profile.
  No secret is ever stored. See
  [docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md](docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md).
```

- [ ] **Step 6: Full check and commit**

```bash
cargo fmt --all
cargo make check
cargo make test
cargo make test-db
git add tests/e2e/tests/machine_gate_e2e.rs CLAUDE.md
git add $(git status --porcelain tests/e2e/.sqlx | awk '{print $2}')
git status --short
git commit -m "G3 Phase A: end-to-end gate coverage on both surfaces, and docs"
```

---

## Pre-merge checklist

- [ ] `cargo make check` passes from clean (an incremental pre-commit clippy can pass where CI's clean build fails).
- [ ] `cargo make test-e2e` passes with a freshly built `temper` binary.
- [ ] `cargo make check-openapi-spec` passes and `openapi.json` is **unchanged** — these routes are deliberately out of the contract.
- [ ] `git status --short` shows no stray `.sqlx` files staged, and no orphaned ones left behind.
- [ ] `./target/debug/temper admin machine provision --help` prints.
- [ ] The spec's D1–D14 each have a home in the code. D14's home is `enroll_in_gating_team`.

## Before the migration reaches production

- [ ] **Re-verify the backfill set.** It was exactly the steward on 2026-07-10; re-run before migrating:
  ```bash
  psql "$(neonctl connection-string main --project-id crimson-fog-23541670 \
    --org-id org-wild-snow-32921543 --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)" \
    -X -A -c "SELECT auth_provider_user_id FROM kb_profile_auth_links WHERE auth_provider = 'auth0-m2m';"
  ```
  Expect one row: `y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu`. **More than one row means a machine authenticated that nobody authorized** — stop and investigate before a backfill legitimizes it. Never print the connection string; pipe it.
- [ ] **Snapshot Neon** before migrating (`neonctl branches create`).
- [ ] **Migrate before deploying.** Migrate-ahead-of-deploy is inert (the table exists, nothing reads it). Deploy-ahead-of-migrate 500s every machine call. Accepted blast radius: one missed steward tick.
- [ ] After the deploy, confirm the steward's next hourly tick succeeds and `last_seen_at` advances.
