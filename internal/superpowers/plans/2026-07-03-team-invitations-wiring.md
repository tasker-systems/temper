# Team Invitations Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the inert `kb_team_invitations` substrate into a working invite → accept/decline flow, and reconcile the mis-scoped system-gate CLI verbs by moving them under `temper auth` so `team join` correctly means "accept an invitation."

**Architecture:** New `invitation_service` (service-direct, no Backend trait, no events — same precedent as `team_service`) over `kb_team_invitations`; thin HTTP handlers dispatch one service call each; a token is a 128-bit CSPRNG value (not a UUID); accept/decline routes live in the un-gated router tier so gating-team invitees can redeem before they have system access. CLI splits entitlement (`auth`) from collaboration (`team`).

**Tech Stack:** Rust (axum, sqlx macros, clap), PostgreSQL 18 + pgvector, ts-rs for wire types, cargo-nextest, e2e crate driving CLI ↔ API ↔ DB.

**Design spec:** `docs/superpowers/specs/2026-07-03-team-invitations-wiring-design.md` — read it before starting; every task's requirements implicitly include it.

## Global Constraints

- **Additive-only on `main`:** the migration only relaxes a uniqueness constraint on an inert (zero-row) table — safe. No destructive/big-bang SQL.
- **Typed structs over inline JSON:** no `serde_json::json!()` for structured payloads.
- **Auth before writes:** every mutating service fn checks authorization before any INSERT/UPDATE.
- **Token = CSPRNG, never `Uuid::now_v7()`:** 128 bits from `rand::rngs::OsRng`, hex-encoded (32 chars, fits `VARCHAR(128)`).
- **Wire types in `temper-core`** with ts-rs derives (`#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]` + `ts(export, export_to = "invitation.ts")`).
- **`#[sqlx::test]` files need `#![cfg(feature = "test-db")]`** at the top or the unit-tests CI job fails.
- **Role `Owner` is never invitable** — reject with `BadRequest` ("ownership is transferred, not invited").
- **sqlx cache after any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` / `prepare-api` / `prepare-e2e` for test-target queries.
- **Run `cargo make check` before every commit.**

---

## File Structure

- Create `migrations/20260703NNNNNN_invitation_partial_unique.sql` — relax the invitation uniqueness constraint.
- Modify `crates/temper-core/src/types/invitation.rs` — add `CreateInvitationRequest`, `AcceptInvitationResponse`.
- Modify `crates/temper-services/Cargo.toml` — add `rand = "0.8"`.
- Create `crates/temper-services/src/services/invitation_service.rs` — the four operations + params struct + token minting.
- Modify `crates/temper-services/src/services/mod.rs` — register the module.
- Create `crates/temper-api/src/handlers/invitations.rs` — four thin handlers + request-body types.
- Modify `crates/temper-api/src/handlers/mod.rs` — register the module.
- Modify `crates/temper-api/src/routes.rs` — 2 routes into `gated`, 2 into `auth_only`.
- Modify `crates/temper-client/src/teams.rs` — 4 client methods.
- Modify `crates/temper-cli/src/cli.rs` — `AuthAction` gains `RequestAccess`/`WithdrawRequest`; `TeamAction` gains `Invite`/`Decline`/`Invitations` and repurposes `Join`; loses `Status`/`WithdrawRequest` and old `Join`.
- Modify `crates/temper-cli/src/commands/auth.rs` — `request_access`, `withdraw_request`, and fold system-access into `status`.
- Modify `crates/temper-cli/src/commands/team.rs` — `invite_remote`, `accept_invitation`, `decline_invitation`, `list_invitations_remote`; remove `join`/`status`/`withdraw_request`.
- Modify `crates/temper-cli/src/main.rs` — rewire dispatch.
- Create `tests/e2e/tests/team_invitations_test.rs` — full invite → accept flow.

---

## Task 1: Migration — partial-unique invitation constraint

**Files:**
- Create: `migrations/20260703NNNNNN_invitation_partial_unique.sql` (replace `NNNNNN` with a real timestamp, e.g. `20260703120000`)

**Interfaces:**
- Produces: `kb_team_invitations` with a partial unique index `(team_id, invited_email) WHERE status='pending'` instead of a full `UNIQUE(team_id, invited_email)`.

- [ ] **Step 1: Confirm the existing constraint name**

Run:
```bash
cargo make docker-up
psql postgresql://temper:temper@localhost:5437/temper_development -c '\d kb_team_invitations'
```
Expected: an index line `"kb_team_invitations_team_id_invited_email_key" UNIQUE CONSTRAINT, btree (team_id, invited_email)`. If the auto-name differs, use the actual name in Step 2.

- [ ] **Step 2: Write the migration**

Create `migrations/20260703120000_invitation_partial_unique.sql`:
```sql
-- Relax kb_team_invitations uniqueness from a full UNIQUE(team_id, invited_email)
-- to a PARTIAL unique index scoped to pending rows, mirroring
-- idx_join_requests_one_pending. This lets declined/expired/accepted history
-- rows coexist while still enforcing "one pending invite per email per team".
-- Safe: the table is inert (zero rows in every environment), and relaxing a
-- uniqueness constraint cannot break existing data.

ALTER TABLE kb_team_invitations
    DROP CONSTRAINT kb_team_invitations_team_id_invited_email_key;

CREATE UNIQUE INDEX idx_invitations_one_pending
    ON kb_team_invitations (team_id, invited_email)
    WHERE status = 'pending';
```

- [ ] **Step 3: Apply and verify**

Run:
```bash
cargo sqlx migrate run --source migrations --database-url postgresql://temper:temper@localhost:5437/temper_development
psql postgresql://temper:temper@localhost:5437/temper_development -c '\d kb_team_invitations'
```
Expected: the `_key` UNIQUE CONSTRAINT is gone; `idx_invitations_one_pending` appears as a partial unique index with `WHERE (status = 'pending')`.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260703120000_invitation_partial_unique.sql
git commit -m "feat(invitations): relax uniqueness to partial-on-pending

Mirrors idx_join_requests_one_pending so declined/expired/accepted invitation
history rows coexist while one-pending-per-email-per-team is still enforced.
Safe: kb_team_invitations is inert."
```

---

## Task 2: Core wire types

**Files:**
- Modify: `crates/temper-core/src/types/invitation.rs`

**Interfaces:**
- Produces: `CreateInvitationRequest { invited_email: String, role: TeamRole }`, `AcceptInvitationResponse { team_id: Uuid, team_slug: String, role: TeamRole }`.

- [ ] **Step 1: Add the request/response types**

Append to `crates/temper-core/src/types/invitation.rs` (the file already imports `TeamRole`, `Uuid`; add `serde::{Deserialize, Serialize}` to the imports if not present):
```rust
/// Request body for `POST /api/teams/{id}/invite`.
///
/// `role` cannot be `Owner` — the service rejects it (ownership is transferred,
/// not invited).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateInvitationRequest {
    pub invited_email: String,
    pub role: TeamRole,
}

/// Response from `POST /api/invitations/{token}/accept` — the team the caller
/// just joined and at what role.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcceptInvitationResponse {
    pub team_id: Uuid,
    pub team_slug: String,
    pub role: TeamRole,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-core`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/invitation.rs
git commit -m "feat(invitations): add CreateInvitationRequest + AcceptInvitationResponse wire types"
```

---

## Task 3: `invitation_service::create_invitation`

**Files:**
- Modify: `crates/temper-services/Cargo.toml`
- Create: `crates/temper-services/src/services/invitation_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: `team_service::role_on_team`, `team_service::can_manage` (both `pub(crate)`), `TeamInvitation`, `CreateInvitationRequest`.
- Produces:
  - `pub struct CreateInvitationParams { pub invited_email: String, pub role: TeamRole }`
  - `pub async fn create_invitation(pool: &PgPool, caller: ProfileId, team_id: Uuid, params: CreateInvitationParams) -> ApiResult<TeamInvitation>`
  - `fn mint_token() -> String` (module-private; 128-bit CSPRNG hex).

- [ ] **Step 1: Add the `rand` dependency**

In `crates/temper-services/Cargo.toml`, under `[dependencies]`, add:
```toml
rand = "0.8"
```

- [ ] **Step 2: Register the module**

In `crates/temper-services/src/services/mod.rs`, add (alphabetical with siblings):
```rust
pub mod invitation_service;
```

- [ ] **Step 3: Write the failing test**

Create `crates/temper-services/src/services/invitation_service.rs` starting with the module cfg and the first test. (Test helpers: reuse the pattern in `team_service.rs`'s `#[cfg(test)]` block — look there for how a team + owner profile are seeded; replicate that seeding helper at the bottom of this file.)
```rust
#![cfg(feature = "test-db")]
//! Team invitation service over `kb_team_invitations`.
//!
//! Service-direct: no Backend-trait command, no event emission — invitations are
//! provisioning/infra, same precedent as `team_service` / `context_service`.
//! Authorization precedes every write, reusing `team_service::role_on_team` +
//! `can_manage`. Tokens are 128-bit CSPRNG values, never UUIDs.

// (Implementation added in Step 5; test block below drives it.)

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::team::TeamRole;

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_by_owner_succeeds(pool: sqlx::PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let inv = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams {
                invited_email: "alice@example.com".into(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner can invite");
        assert_eq!(inv.invited_email, "alice@example.com");
        assert_eq!(inv.role, TeamRole::Member);
        assert_eq!(inv.status, InvitationStatus::Pending);
        assert_eq!(inv.token.len(), 32); // 16 bytes hex
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_rejects_owner_role(pool: sqlx::PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let err = create_invitation(
            &pool,
            owner,
            team_id,
            CreateInvitationParams { invited_email: "a@e.com".into(), role: TeamRole::Owner },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_non_manager_forbidden(pool: sqlx::PgPool) {
        let (team_id, _owner) = seed_team_with_owner(&pool).await;
        let stranger = seed_profile(&pool, "stranger").await;
        let err = create_invitation(
            &pool,
            stranger,
            team_id,
            CreateInvitationParams { invited_email: "a@e.com".into(), role: TeamRole::Member },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_invitation_duplicate_pending_conflicts(pool: sqlx::PgPool) {
        let (team_id, owner) = seed_team_with_owner(&pool).await;
        let p = || CreateInvitationParams { invited_email: "dup@e.com".into(), role: TeamRole::Member };
        create_invitation(&pool, owner, team_id, p()).await.unwrap();
        let err = create_invitation(&pool, owner, team_id, p()).await.unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)));
    }
}
```

⚠️ Plan/reality gap: `team_service.rs`'s test block has the real seeding helpers (`seed_team_with_owner`, `seed_profile` may be named differently). Open `crates/temper-services/src/services/team_service.rs` `#[cfg(test)]` block, copy the actual seeding helpers into this file's test module, and adjust the helper names/signatures used above to match. Do not invent seeding SQL — reuse theirs.

- [ ] **Step 4: Run the test to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db create_invitation_by_owner_succeeds`
Expected: FAIL — `create_invitation` / `CreateInvitationParams` not defined.

- [ ] **Step 5: Write the implementation**

Insert above the `#[cfg(test)]` block:
```rust
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{can_manage, role_on_team};
use temper_core::types::ids::ProfileId;
use temper_core::types::invitation::{InvitationStatus, TeamInvitation};
use temper_core::types::team::TeamRole;

/// Parameters for creating an invitation.
pub struct CreateInvitationParams {
    pub invited_email: String,
    pub role: TeamRole,
}

/// Mint a 128-bit capability token, hex-encoded (32 chars). CSPRNG-backed —
/// NOT a UUID (which is time-sortable and guessable).
fn mint_token() -> String {
    let mut rng = rand::rngs::OsRng;
    format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64())
}

/// Create a pending invitation. Auth: caller must own/maintain the team.
/// `Owner` role is rejected. A second pending invite for the same
/// `(team, email)` conflicts (partial unique index).
pub async fn create_invitation(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    params: CreateInvitationParams,
) -> ApiResult<TeamInvitation> {
    // Auth before writes.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    if params.role == TeamRole::Owner {
        return Err(ApiError::BadRequest(
            "ownership is transferred, not invited".to_string(),
        ));
    }

    let id = Uuid::now_v7();
    let token = mint_token();
    let row = sqlx::query_as!(
        TeamInvitation,
        r#"
        INSERT INTO kb_team_invitations
            (id, team_id, invited_email, invited_by_profile_id, role, token, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')
        RETURNING id, team_id, invited_email, invited_by_profile_id,
                  role AS "role: TeamRole", token,
                  status AS "status: InvitationStatus", expires_at, created
        "#,
        id,
        team_id,
        params.invited_email,
        *caller,
        params.role as TeamRole,
        token,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            ApiError::Conflict("a pending invitation already exists for this email".to_string())
        }
        _ => ApiError::from(e),
    })?;

    Ok(row)
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db invitation_service`
Expected: all four `create_invitation_*` tests PASS.

- [ ] **Step 7: Regenerate the services test-target sqlx cache and check**

Run:
```bash
cargo make prepare-services
cargo make check
```
Expected: cache updated; check passes.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-services/Cargo.toml crates/temper-services/src/services/invitation_service.rs crates/temper-services/src/services/mod.rs crates/temper-services/.sqlx
git commit -m "feat(invitations): create_invitation service (auth-gated, CSPRNG token, pending-conflict)"
```

---

## Task 4: `invitation_service::accept_invitation`

**Files:**
- Modify: `crates/temper-services/src/services/invitation_service.rs`

**Interfaces:**
- Consumes: `AcceptInvitationResponse`.
- Produces: `pub async fn accept_invitation(pool: &PgPool, caller: ProfileId, token: &str) -> ApiResult<AcceptInvitationResponse>`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block:
```rust
#[sqlx::test(migrations = "../../migrations")]
async fn accept_creates_membership(pool: sqlx::PgPool) {
    let (team_id, owner) = seed_team_with_owner(&pool).await;
    let invitee = seed_profile(&pool, "invitee").await;
    let inv = create_invitation(&pool, owner, team_id,
        CreateInvitationParams { invited_email: "i@e.com".into(), role: TeamRole::Member }).await.unwrap();

    let resp = accept_invitation(&pool, invitee, &inv.token).await.expect("accept");
    assert_eq!(resp.team_id, team_id);
    assert_eq!(resp.role, TeamRole::Member);

    let role: Option<TeamRole> = role_on_team(&pool, team_id, invitee).await.unwrap();
    assert_eq!(role, Some(TeamRole::Member));
}

#[sqlx::test(migrations = "../../migrations")]
async fn accept_is_idempotent(pool: sqlx::PgPool) {
    let (team_id, owner) = seed_team_with_owner(&pool).await;
    let invitee = seed_profile(&pool, "invitee").await;
    let inv = create_invitation(&pool, owner, team_id,
        CreateInvitationParams { invited_email: "i@e.com".into(), role: TeamRole::Member }).await.unwrap();
    accept_invitation(&pool, invitee, &inv.token).await.unwrap();
    // Second accept by the same profile succeeds (idempotent).
    let resp = accept_invitation(&pool, invitee, &inv.token).await.expect("idempotent");
    assert_eq!(resp.team_id, team_id);
}

#[sqlx::test(migrations = "../../migrations")]
async fn accept_unknown_token_not_found(pool: sqlx::PgPool) {
    let invitee = seed_profile(&pool, "invitee").await;
    let err = accept_invitation(&pool, invitee, "deadbeef").await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

#[sqlx::test(migrations = "../../migrations")]
async fn accept_expired_errors_and_marks_expired(pool: sqlx::PgPool) {
    let (team_id, owner) = seed_team_with_owner(&pool).await;
    let invitee = seed_profile(&pool, "invitee").await;
    let inv = create_invitation(&pool, owner, team_id,
        CreateInvitationParams { invited_email: "i@e.com".into(), role: TeamRole::Member }).await.unwrap();
    // Force expiry.
    sqlx::query!("UPDATE kb_team_invitations SET expires_at = now() - interval '1 day' WHERE id = $1", inv.id)
        .execute(&pool).await.unwrap();

    let err = accept_invitation(&pool, invitee, &inv.token).await.unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
    let status: InvitationStatus = sqlx::query_scalar!(
        r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE id = $1"#, inv.id)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(status, InvitationStatus::Expired);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `DATABASE_URL=... cargo nextest run -p temper-services --features test-db accept_`
Expected: FAIL — `accept_invitation` not defined.

- [ ] **Step 3: Write the implementation**

Add `AcceptInvitationResponse` to the `use temper_core::types::invitation::...` import, then add:
```rust
/// Redeem an invitation token (bearer authority — the token IS the authority;
/// membership is created for `caller`). Idempotent. Expiry is checked lazily
/// here and flips the row to `expired`.
pub async fn accept_invitation(
    pool: &PgPool,
    caller: ProfileId,
    token: &str,
) -> ApiResult<AcceptInvitationResponse> {
    let inv = sqlx::query_as!(
        TeamInvitation,
        r#"
        SELECT id, team_id, invited_email, invited_by_profile_id,
               role AS "role: TeamRole", token,
               status AS "status: InvitationStatus", expires_at, created
          FROM kb_team_invitations
         WHERE token = $1
        "#,
        token,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    let team_slug = sqlx::query_scalar!("SELECT slug FROM kb_teams WHERE id = $1", inv.team_id)
        .fetch_one(pool)
        .await?;

    match inv.status {
        InvitationStatus::Accepted => {
            // Idempotent iff caller is already the member.
            match role_on_team(pool, inv.team_id, caller).await? {
                Some(role) => Ok(AcceptInvitationResponse { team_id: inv.team_id, team_slug, role }),
                None => Err(ApiError::Conflict("invitation already redeemed".to_string())),
            }
        }
        InvitationStatus::Declined => {
            Err(ApiError::BadRequest("invitation was declined".to_string()))
        }
        InvitationStatus::Expired => {
            Err(ApiError::BadRequest("invitation has expired".to_string()))
        }
        InvitationStatus::Pending => {
            if inv.expires_at < chrono::Utc::now() {
                sqlx::query!(
                    "UPDATE kb_team_invitations SET status = 'expired' WHERE id = $1",
                    inv.id,
                )
                .execute(pool)
                .await?;
                return Err(ApiError::BadRequest("invitation has expired".to_string()));
            }

            let mut tx = pool.begin().await?;
            sqlx::query!(
                r#"
                INSERT INTO kb_team_members (team_id, profile_id, role)
                VALUES ($1, $2, $3)
                ON CONFLICT (team_id, profile_id) DO NOTHING
                "#,
                inv.team_id,
                *caller,
                inv.role as TeamRole,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE kb_team_invitations SET status = 'accepted' WHERE id = $1",
                inv.id,
            )
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(AcceptInvitationResponse { team_id: inv.team_id, team_slug, role: inv.role })
        }
    }
}
```

⚠️ Plan/reality gap: confirm `ApiError` has `NotFound`, `Conflict(String)`, `BadRequest(String)` variants (they are used across `access_service`/`team_service`, so they exist — but verify the exact spellings) and that `pool.begin()` errors map cleanly via `?` (they do in `review_request`; if a manual `map_err(|e| ApiError::Internal(...))` is the house style there, match it).

- [ ] **Step 4: Run to verify pass**

Run: `DATABASE_URL=... cargo nextest run -p temper-services --features test-db accept_`
Expected: all `accept_*` tests PASS.

- [ ] **Step 5: Regenerate cache + check**

Run: `cargo make prepare-services && cargo make check`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/invitation_service.rs crates/temper-services/.sqlx
git commit -m "feat(invitations): accept_invitation (bearer, idempotent, lazy-expiry, member tx)"
```

---

## Task 5: `invitation_service::decline_invitation` + `list_invitations`

**Files:**
- Modify: `crates/temper-services/src/services/invitation_service.rs`

**Interfaces:**
- Produces:
  - `pub async fn decline_invitation(pool: &PgPool, caller: ProfileId, token: &str) -> ApiResult<()>`
  - `pub async fn list_invitations(pool: &PgPool, caller: ProfileId, team_id: Uuid) -> ApiResult<Vec<TeamInvitation>>`

- [ ] **Step 1: Write the failing tests**

Add to the test block:
```rust
#[sqlx::test(migrations = "../../migrations")]
async fn decline_marks_declined_and_is_idempotent(pool: sqlx::PgPool) {
    let (team_id, owner) = seed_team_with_owner(&pool).await;
    let invitee = seed_profile(&pool, "invitee").await;
    let inv = create_invitation(&pool, owner, team_id,
        CreateInvitationParams { invited_email: "i@e.com".into(), role: TeamRole::Member }).await.unwrap();

    decline_invitation(&pool, invitee, &inv.token).await.expect("decline");
    let status: InvitationStatus = sqlx::query_scalar!(
        r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE id = $1"#, inv.id)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(status, InvitationStatus::Declined);
    // Idempotent.
    decline_invitation(&pool, invitee, &inv.token).await.expect("idempotent decline");
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_returns_pending_for_manager(pool: sqlx::PgPool) {
    let (team_id, owner) = seed_team_with_owner(&pool).await;
    create_invitation(&pool, owner, team_id,
        CreateInvitationParams { invited_email: "a@e.com".into(), role: TeamRole::Member }).await.unwrap();
    let list = list_invitations(&pool, owner, team_id).await.expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].invited_email, "a@e.com");
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_forbidden_for_non_manager(pool: sqlx::PgPool) {
    let (team_id, _owner) = seed_team_with_owner(&pool).await;
    let stranger = seed_profile(&pool, "stranger").await;
    let err = list_invitations(&pool, stranger, team_id).await.unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `DATABASE_URL=... cargo nextest run -p temper-services --features test-db 'decline_marks|list_returns|list_forbidden'`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Write the implementation**

```rust
/// Decline an invitation (bearer authority). Idempotent if already declined;
/// declining an accepted invitation is a BadRequest.
pub async fn decline_invitation(pool: &PgPool, _caller: ProfileId, token: &str) -> ApiResult<()> {
    let status = sqlx::query_scalar!(
        r#"SELECT status AS "status: InvitationStatus" FROM kb_team_invitations WHERE token = $1"#,
        token,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    match status {
        InvitationStatus::Declined => Ok(()),
        InvitationStatus::Accepted => {
            Err(ApiError::BadRequest("invitation was already accepted".to_string()))
        }
        InvitationStatus::Pending | InvitationStatus::Expired => {
            sqlx::query!(
                "UPDATE kb_team_invitations SET status = 'declined' WHERE token = $1",
                token,
            )
            .execute(pool)
            .await?;
            Ok(())
        }
    }
}

/// List pending, non-expired invitations for a team. Auth: owner/maintainer.
pub async fn list_invitations(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
) -> ApiResult<Vec<TeamInvitation>> {
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    let rows = sqlx::query_as!(
        TeamInvitation,
        r#"
        SELECT id, team_id, invited_email, invited_by_profile_id,
               role AS "role: TeamRole", token,
               status AS "status: InvitationStatus", expires_at, created
          FROM kb_team_invitations
         WHERE team_id = $1 AND status = 'pending' AND expires_at > now()
         ORDER BY created DESC
        "#,
        team_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `DATABASE_URL=... cargo nextest run -p temper-services --features test-db invitation_service`
Expected: all invitation_service tests PASS.

- [ ] **Step 5: Regenerate cache + check**

Run: `cargo make prepare-services && cargo make check`

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/invitation_service.rs crates/temper-services/.sqlx
git commit -m "feat(invitations): decline_invitation + list_invitations"
```

---

## Task 6: HTTP handlers + routes

**Files:**
- Create: `crates/temper-api/src/handlers/invitations.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: the four `invitation_service` fns; `CreateInvitationRequest`, `AcceptInvitationResponse`, `TeamInvitation`.
- Produces routes: `POST /api/teams/{id}/invite` and `GET /api/teams/{id}/invitations` (gated); `POST /api/invitations/{token}/accept` and `POST /api/invitations/{token}/decline` (auth_only).

- [ ] **Step 1: Write the handlers**

Create `crates/temper-api/src/handlers/invitations.rs`:
```rust
//! Team invitation handlers — thin: extract `AuthUser`, dispatch one
//! `invitation_service` call, return the typed row. Service-direct.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, TeamInvitation,
};
use temper_services::error::ApiResult;
use temper_services::services::invitation_service;
use temper_services::state::AppState;

/// POST /api/teams/{id}/invite — create a pending invitation (owner/maintainer).
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<CreateInvitationRequest>,
) -> ApiResult<(StatusCode, Json<TeamInvitation>)> {
    let params = invitation_service::CreateInvitationParams {
        invited_email: body.invited_email,
        role: body.role,
    };
    let inv = invitation_service::create_invitation(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        params,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(inv)))
}

/// GET /api/teams/{id}/invitations — list pending invitations (owner/maintainer).
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<Vec<TeamInvitation>>> {
    invitation_service::list_invitations(&state.pool, ProfileId::from(auth.0.profile.id), team_id)
        .await
        .map(Json)
}

/// POST /api/invitations/{token}/accept — redeem a token (bearer).
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(token): Path<String>,
) -> ApiResult<Json<AcceptInvitationResponse>> {
    invitation_service::accept_invitation(&state.pool, ProfileId::from(auth.0.profile.id), &token)
        .await
        .map(Json)
}

/// POST /api/invitations/{token}/decline — decline a token (bearer).
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(token): Path<String>,
) -> ApiResult<StatusCode> {
    invitation_service::decline_invitation(&state.pool, ProfileId::from(auth.0.profile.id), &token)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
```

⚠️ Plan/reality gap: check whether sibling handlers carry `#[utoipa::path(...)]` OpenAPI annotations (teams.rs does). If the crate registers handlers in an OpenAPI derive (`ApiDoc`), add matching `#[utoipa::path]` blocks and register the four new paths + `TeamInvitation`/`CreateInvitationRequest`/`AcceptInvitationResponse` schemas there, mirroring how `teams::*` are registered. Grep `rg -n "teams::add_member|components\(schemas" crates/temper-api/src` to find the registration site.

- [ ] **Step 2: Register the module**

In `crates/temper-api/src/handlers/mod.rs`, add:
```rust
pub mod invitations;
```

- [ ] **Step 3: Add the routes**

In `crates/temper-api/src/routes.rs`, in the `auth_only` router (after the `/api/access/settings` route), add:
```rust
        .route(
            "/api/invitations/{token}/accept",
            post(handlers::invitations::accept),
        )
        .route(
            "/api/invitations/{token}/decline",
            post(handlers::invitations::decline),
        )
```
And in the `gated` router (next to the `/api/teams/{id}/members` route), add:
```rust
        .route(
            "/api/teams/{id}/invite",
            post(handlers::invitations::create),
        )
        .route(
            "/api/teams/{id}/invitations",
            get(handlers::invitations::list),
        )
```

- [ ] **Step 4: Verify it compiles + check**

Run: `cargo check -p temper-api --features test-db && cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/invitations.rs crates/temper-api/src/handlers/mod.rs crates/temper-api/src/routes.rs
git commit -m "feat(invitations): API handlers + routes (invite/list gated, accept/decline auth-only)"
```

---

## Task 7: Client methods

**Files:**
- Modify: `crates/temper-client/src/teams.rs`

**Interfaces:**
- Produces on `TeamsClient`: `invite`, `list_invitations`, `accept_invitation`, `decline_invitation`.

- [ ] **Step 1: Add the methods**

Extend the `use temper_core::types::...` imports with `invitation::{AcceptInvitationResponse, CreateInvitationRequest, TeamInvitation}`, then add inside `impl<'a> TeamsClient<'a>`:
```rust
    /// POST /api/teams/{id}/invite — create a pending invitation.
    pub async fn invite(
        &self,
        team_id: Uuid,
        body: &CreateInvitationRequest,
    ) -> Result<TeamInvitation> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/invite");
        let req = self.http.post(&path).json(body);
        self.http.send_json(&Method::POST, &path, req, Some(&token)).await
    }

    /// GET /api/teams/{id}/invitations — list pending invitations.
    pub async fn list_invitations(&self, team_id: Uuid) -> Result<Vec<TeamInvitation>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/invitations");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }

    /// POST /api/invitations/{token}/accept — redeem an invitation token.
    pub async fn accept_invitation(&self, invite_token: &str) -> Result<AcceptInvitationResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invitations/{invite_token}/accept");
        let req = self.http.post(&path);
        self.http.send_json(&Method::POST, &path, req, Some(&token)).await
    }

    /// POST /api/invitations/{token}/decline — decline an invitation token.
    pub async fn decline_invitation(&self, invite_token: &str) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invitations/{invite_token}/decline");
        let req = self.http.post(&path);
        self.http.send_no_content(&Method::POST, &path, req, Some(&token)).await
    }
```

⚠️ Plan/reality gap: `decline` returns 204 No Content. Check how `access().withdraw_request` (also a no-content call) sends — it may use `send_no_content`, `send_empty`, or `send_json::<()>`. Grep `crates/temper-client/src/access.rs:63` and match that exact helper name for `decline_invitation`. `accept` returns a body, so `send_json` is correct there.

- [ ] **Step 2: Verify + check**

Run: `cargo check -p temper-client && cargo make check`

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/teams.rs
git commit -m "feat(invitations): client methods (invite/list/accept/decline)"
```

---

## Task 8: CLI — team invitation verbs

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/commands/team.rs`
- Modify: `crates/temper-cli/src/main.rs`

**Interfaces:**
- Produces `TeamAction` variants `Invite`, `Decline`, `Invitations`, and repurposed `Join { token }`; command fns `invite_remote`, `accept_invitation`, `decline_invitation`, `list_invitations_remote`.

- [ ] **Step 1: Repurpose `Join` and add invitation variants in `cli.rs`**

In `enum TeamAction`, replace the existing `Join { team, message }` with:
```rust
    /// Accept a team invitation by its token.
    Join {
        /// Invitation token (from `temper team invite`).
        token: String,
    },
    /// Invite an email to a team (owner/maintainer).
    Invite {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
        /// Email address to invite.
        email: String,
        /// Role to grant on acceptance: maintainer | member | watcher.
        #[arg(long)]
        role: String,
    },
    /// Decline a team invitation by its token.
    Decline {
        /// Invitation token.
        token: String,
    },
    /// List pending invitations for a team (owner/maintainer).
    Invitations {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
    },
```
Leave `Status` and `WithdrawRequest` in place for now — Task 9 removes them.

- [ ] **Step 2: Add command fns in `commands/team.rs`**

Add (the module already imports `TeamRole`, `resolve_team_id`, `client_err`, `render`):
```rust
/// Invite an email to a team.
pub async fn invite_remote(
    client: &temper_client::TemperClient,
    team: &str,
    email: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let req = temper_core::types::invitation::CreateInvitationRequest {
        invited_email: email.to_owned(),
        role: parse_role(role)?,
    };
    let inv = client
        .teams()
        .invite(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&inv, fmt)?);
    Ok(())
}

/// Accept an invitation by token.
pub async fn accept_invitation(
    client: &temper_client::TemperClient,
    token: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resp = client
        .teams()
        .accept_invitation(token)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&resp, fmt)?);
    Ok(())
}

/// Decline an invitation by token.
pub async fn decline_invitation(
    client: &temper_client::TemperClient,
    token: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    client
        .teams()
        .decline_invitation(token)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Invitation declined.");
    Ok(())
}

/// List pending invitations for a team.
pub async fn list_invitations_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let invitations = client
        .teams()
        .list_invitations(team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&invitations, fmt)?);
    Ok(())
}
```

- [ ] **Step 3: Rewire dispatch in `main.rs`**

Replace the `TeamAction::Join { team: _, message }` arm and add the new arms:
```rust
            TeamAction::Join { token } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::accept_invitation(client, &token, output_format).await
                })
            }),
            TeamAction::Invite { team, email, role } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::team::invite_remote(
                            client, &team, &email, &role, output_format,
                        )
                        .await
                    })
                })
            }
            TeamAction::Decline { token } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::decline_invitation(client, &token, output_format).await
                })
            }),
            TeamAction::Invitations { team } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    temper_cli::commands::team::list_invitations_remote(client, &team, output_format).await
                })
            }),
```

- [ ] **Step 4: Build the CLI**

Run: `cargo build -p temper-cli --bin temper`
Expected: clean (Task 9 handles the now-orphaned `join`/`status` fns; if the old `team::join` fn is unused it will warn — it's removed in Task 9, so a temporary `#[allow(dead_code)]` is acceptable, or sequence Task 9 immediately after).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/team.rs crates/temper-cli/src/main.rs
git commit -m "feat(invitations): CLI team invite/join/decline/invitations verbs"
```

---

## Task 9: CLI — `auth` reframe + retire mis-scoped team verbs

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/commands/auth.rs`
- Modify: `crates/temper-cli/src/commands/team.rs`
- Modify: `crates/temper-cli/src/main.rs`

**Interfaces:**
- Produces `AuthAction::RequestAccess { message }` and `AuthAction::WithdrawRequest`; extends `auth status` output with a system-access line. Removes `TeamAction::Status`, `TeamAction::WithdrawRequest`, and `team::{join, status, withdraw_request}`.

- [ ] **Step 1: Extend `AuthAction` in `cli.rs`**

Add to `enum AuthAction`:
```rust
    /// Request system access (the invite_only gate). Reviewed by an admin.
    RequestAccess {
        /// Message for the admin reviewing your request.
        #[arg(long)]
        message: Option<String>,
    },
    /// Withdraw your pending system-access request.
    WithdrawRequest,
```
Remove `Status { team }` and `WithdrawRequest` from `enum TeamAction`.

- [ ] **Step 2: Move the command fns into `commands/auth.rs`**

Move `join` (rename to `request_access`), `status` (rename to `request_access_status` — used to extend `auth status`), and `withdraw_request` from `commands/team.rs` into `commands/auth.rs` verbatim (they only use `client.access()` + `output`, both available there). Delete them from `team.rs`.

- [ ] **Step 3: Fold system-access state into `auth status`**

In `commands/auth.rs`, find the existing `status()` fn (renders login/token state) and append a system-access section. After the existing status output, add:
```rust
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let entitlements = client
                .access()
                .get_entitlements()
                .await
                .map_err(crate::commands::client_err)?;
            if entitlements.system_access {
                output::success("System access: granted");
            } else {
                match client.access().get_own_request().await.map_err(crate::commands::client_err)? {
                    Some(req) if req.status == JoinRequestStatus::Pending => {
                        output::plain(format!(
                            "System access: pending (requested {})",
                            req.created.format("%Y-%m-%d")
                        ));
                    }
                    _ => {
                        output::plain("System access: none");
                        output::hint("  Run `temper auth request-access` to request it.");
                    }
                }
            }
            Ok(())
        })
    })?;
```

⚠️ Plan/reality gap: (a) confirm `client.access().get_entitlements()` exists — grep `crates/temper-client/src/access.rs`; if not, use `get_own_request` alone and treat `Approved` as granted. (b) The existing `status()` may already be sync and wrap `with_client` itself; integrate this block into that existing async closure rather than nesting a second `with_client`. Read the current `auth::status` fn and merge, don't bolt on.

- [ ] **Step 4: Rewire dispatch in `main.rs`**

Remove the `TeamAction::Status`/`TeamAction::WithdrawRequest` arms. Add under the `AuthAction` match:
```rust
            AuthAction::RequestAccess { message } => {
                temper_cli::commands::auth::request_access(message.as_deref())
            }
            AuthAction::WithdrawRequest => temper_cli::commands::auth::withdraw_request(),
```
Ensure the `AuthAction::Status =>` arm still calls `auth::status()` (now extended).

- [ ] **Step 5: Build + manual smoke**

Run:
```bash
cargo build -p temper-cli --bin temper
./target/debug/temper team --help
./target/debug/temper auth --help
```
Expected: `team` shows `invite`, `join <token>`, `decline`, `invitations` and NO `status`/`withdraw-request`; `auth` shows `request-access` + `withdraw-request` + `status`.

- [ ] **Step 6: `cargo make check`**

Run: `cargo make check`
Expected: no dead-code warnings (old fns removed), clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/auth.rs crates/temper-cli/src/commands/team.rs crates/temper-cli/src/main.rs
git commit -m "refactor(cli): move system-gate verbs to \`temper auth\`, fold access state into \`auth status\`"
```

---

## Task 10: e2e — full invite → accept flow

**Files:**
- Create: `tests/e2e/tests/team_invitations_test.rs`

**Interfaces:**
- Consumes: the e2e harness in `tests/e2e/tests/common/` (server spawn, JWT minting, `temper` binary driver). Read an existing test (e.g. a `team_*` or `access_*` e2e) first to learn the harness API — do NOT invent harness calls.

- [ ] **Step 1: Read a sibling e2e test**

Run: `ls tests/e2e/tests && rg -l "teams|team_member|add_member|create_team" tests/e2e/tests`
Read the closest team/access e2e in full to learn: how the harness spawns the server, how it authenticates two distinct profiles, and how it invokes the `temper` CLI (or the client directly).

- [ ] **Step 2: Write the e2e test**

Create `tests/e2e/tests/team_invitations_test.rs` following the harness patterns from Step 1. The test must:
```
1. Spin up the harness; create/authenticate an INVITER profile.
2. Create a team owned by the inviter (client.teams().create or the CLI).
3. Authenticate a second INVITEE profile.
4. As inviter: POST invite (email=invitee's email, role=member) → capture token.
5. As invitee: accept the token.
6. Assert: GET team detail shows the invitee as a member with role=member.
7. Negative: a second accept of the same token by the invitee still succeeds
   (idempotent); accepting an unknown token returns an error.
```
Use the real harness helpers and the real `client.teams().invite(...)` / `accept_invitation(...)` methods. Mark the file `#![cfg(feature = "test-db")]` (match sibling e2e files).

⚠️ Plan/reality gap: if the harness drives the CLI binary rather than the client, remember the local `temper` bin is stale until rebuilt (`cargo build -p temper-cli --bin temper`) — rebuild before running (see the e2e-stale-bin gotcha in CLAUDE.md). Prefer driving through the client if sibling tests do.

- [ ] **Step 3: Run the e2e test**

Run: `cargo make test-e2e 2>&1 | tee /tmp/e2e.log; rg 'team_invitations|FAIL|error: test run failed' /tmp/e2e.log`
Expected: the new test passes; no `FAIL [`.

- [ ] **Step 4: Regenerate e2e sqlx cache if the test uses `query!` macros**

Run: `cargo make prepare-e2e` (only if the test added compile-checked macro queries).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/team_invitations_test.rs tests/e2e/.sqlx
git commit -m "test(invitations): e2e invite -> accept -> membership flow"
```

---

## Task 11: Full verification + sqlx cache ritual

**Files:** none (verification only).

- [ ] **Step 1: Full sqlx cache regeneration**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
git status --porcelain crates/*/.sqlx tests/e2e/.sqlx
```
Expected: any drift is staged; no orphaned entries.

- [ ] **Step 2: Full check + test suites**

Run:
```bash
cargo make check
cargo make test-db
cargo make test-e2e
```
Expected: all green. If the Embed CI job's features matter for touched code (they don't here — no push-body/ingest changes), skip `test-e2e-embed`.

- [ ] **Step 3: Regenerate TS types (wire types changed)**

Run: `cargo make generate-ts-types && git status --porcelain packages/`
Expected: `invitation.ts` gains `CreateInvitationRequest` / `AcceptInvitationResponse`; stage any generated changes.

- [ ] **Step 4: Commit any cache/type drift**

```bash
git add -A
git commit -m "chore(invitations): sqlx cache + generated TS types"
```

- [ ] **Step 5: Push + open PR**

```bash
git push -u origin jct/team-invitations-wiring
gh pr create --title "Team invitations wiring + system-gate CLI reconciliation (teams-in-temper #2)" --body "..."
```
PR body: summarize the model decision (link the spec), the four endpoints, the CLI reframe, and the test coverage. End with the Claude Code attribution footer.

---

## Self-Review Notes (coverage against spec)

- Spec §1 (migration) → Task 1. §2 (service) → Tasks 3–5. §3 (HTTP/routes) → Task 6. §4 (wire types) → Task 2. §5 (client) → Task 7. §6 (CLI) → Tasks 8–9. §7 (tests) → service tests in 3–5, e2e in 10, ritual in 11.
- Spec risks: CSPRNG dep → Task 3 Step 1 (`rand` added, `OsRng`, not UUID). Decline pure-bearer → Task 5 (`_caller` unused, documented). `auth status` open vs invite_only → Task 9 Step 3 (entitlements bool + pending branch). Non-gating-team-invite gap → design-only, no code.
- Owner-role rejection: Task 3 (service `BadRequest`) — the load-bearing guard, tested.
