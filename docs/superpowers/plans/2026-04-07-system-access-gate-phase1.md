# System Access Gate — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the system access gate — database tables, SQL functions, API middleware (default-deny router split), access service, handlers, profile entitlements, and full e2e lifecycle tests.

**Architecture:** A `kb_system_settings` singleton controls whether the instance requires team membership (`invite_only`) or is open. Users request access via `kb_join_requests`; admins approve, which atomically grants `watcher` membership in the gating team. A `require_system_access` middleware layer enforces the gate on all routes except profile and access endpoints. The router is split into auth-only (exempt) and gated (default-deny) groups.

**Tech Stack:** Rust (Axum, sqlx, serde), PostgreSQL 18, cargo-nextest for testing

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `migrations/20260407000001_system_access_gate.sql` | Migration: enum, tables, functions, bootstrap data |
| `migrations/templates/system_initialization.sql` | Seed template for self-hosters and tests |
| `crates/temper-core/src/types/access_gate.rs` | `JoinRequestStatus`, `JoinRequest`, `SystemSettings`, `Entitlements` types |
| `crates/temper-api/src/services/access_service.rs` | System access checks, join request lifecycle, entitlements |
| `crates/temper-api/src/handlers/access.rs` | `/api/access/*` endpoint handlers |
| `crates/temper-api/src/middleware/system_access.rs` | `require_system_access` middleware |
| `tests/e2e/tests/access_gate_test.rs` | Full lifecycle e2e tests |

### Modified files
| File | Change |
|------|--------|
| `crates/temper-core/src/types/mod.rs` | Add `pub mod access_gate;` and re-exports |
| `crates/temper-api/src/error.rs` | Add `SystemAccessRequired` variant |
| `crates/temper-api/src/routes.rs` | Split into auth_only + gated routers |
| `crates/temper-api/src/services/mod.rs` | Add `pub mod access_service;` |
| `crates/temper-api/src/handlers/mod.rs` | Add `pub mod access;` |
| `crates/temper-api/src/handlers/profiles.rs` | Add entitlements to GET response |
| `crates/temper-api/src/middleware/mod.rs` | Add `pub mod system_access;` |
| `tests/e2e/tests/common/mod.rs` | Add cleanup for new tables, helper for second user |

---

## Task 1: Database Migration

**Files:**
- Create: `migrations/20260407000001_system_access_gate.sql`
- Create: `migrations/templates/system_initialization.sql`

- [ ] **Step 1: Write the migration file**

```sql
-- migrations/20260407000001_system_access_gate.sql
-- System access gate: join_request_status enum, kb_system_settings singleton,
-- kb_join_requests table, has_system_access/is_system_admin SQL functions,
-- and temper-system team bootstrap.

-- 1. Join request status enum
CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');

-- 2. System settings singleton
CREATE TABLE kb_system_settings (
    id                  INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    access_mode         VARCHAR(16) NOT NULL DEFAULT 'open'
        CHECK (access_mode IN ('open', 'invite_only')),
    gating_team_slug    VARCHAR(128),
    terms_version       VARCHAR(32),
    terms_resource_uri  TEXT,
    instance_name       VARCHAR(128),
    updated             TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO kb_system_settings (id, access_mode) VALUES (1, 'open');

-- 3. Join requests table
CREATE TABLE kb_join_requests (
    id                       UUID PRIMARY KEY,
    team_id                  UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    requesting_profile_id    UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    status                   join_request_status NOT NULL DEFAULT 'pending',
    message                  TEXT,
    source                   VARCHAR(16) NOT NULL,
    accepted_terms_version   VARCHAR(32),
    accepted_terms_at        TIMESTAMPTZ,
    reviewed_by_profile_id   UUID REFERENCES kb_profiles(id),
    reviewed_at              TIMESTAMPTZ,
    decision_note            TEXT,
    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One pending request per profile per team
CREATE UNIQUE INDEX idx_join_requests_one_pending
    ON kb_join_requests (team_id, requesting_profile_id)
    WHERE status = 'pending';

-- Admin queue ordering
CREATE INDEX idx_join_requests_status_created
    ON kb_join_requests (status, created DESC);

-- 4. System access check function
CREATE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (
        SELECT access_mode, gating_team_slug
          FROM kb_system_settings
         LIMIT 1
    )
    SELECT CASE
        WHEN settings.access_mode = 'open' THEN true
        WHEN settings.access_mode = 'invite_only' THEN EXISTS (
            SELECT 1
              FROM kb_team_members tm
              JOIN kb_teams t ON t.id = tm.team_id
             WHERE tm.profile_id = p_profile_id
               AND t.slug = settings.gating_team_slug
               AND t.is_active = true
        )
        ELSE false
    END
      FROM settings
$$;

-- 5. System admin check function
CREATE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (
        SELECT gating_team_slug
          FROM kb_system_settings
         LIMIT 1
    )
    SELECT EXISTS (
        SELECT 1
          FROM kb_team_members tm
          JOIN kb_teams t ON t.id = tm.team_id
         WHERE tm.profile_id = p_profile_id
           AND t.slug = settings.gating_team_slug
           AND t.is_active = true
           AND tm.role = 'owner'
    )
      FROM settings
$$;

-- 6. Bootstrap: temper-system team + general context
-- System profile (00000000-0000-0000-0004-000000000001) exists from seed migration.
INSERT INTO kb_teams (id, name, slug, description, created_by_profile_id, is_active, created, updated)
VALUES (
    '00000000-0000-0000-0000-000000000002',
    'temper-system',
    'temper-system',
    'System team for instance-wide access control and shared content',
    '00000000-0000-0000-0004-000000000001',
    true,
    now(),
    now()
);

INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created)
VALUES (
    '00000000-0000-0000-0000-000000000003',
    'general',
    'kb_teams',
    '00000000-0000-0000-0000-000000000002',
    now()
);
```

- [ ] **Step 2: Write the seed template**

```sql
-- migrations/templates/system_initialization.sql
-- Template for self-hosted operators to enable invite-only mode.
-- Copy this file, fill in your values, and run against your database.
-- Also used verbatim by integration tests.

-- Step 1: Add yourself as owner of the gating team.
-- Replace the profile_id with your own (from kb_profiles).
INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
VALUES (
    gen_random_uuid(),
    '00000000-0000-0000-0000-000000000002',  -- temper-system team
    '00000000-0000-0000-0004-000000000001',  -- REPLACE with your profile ID
    'owner',
    now()
);

-- Step 2: Enable invite-only mode.
UPDATE kb_system_settings
   SET access_mode = 'invite_only',
       gating_team_slug = 'temper-system',
       instance_name = 'temper',              -- REPLACE with your instance name
       updated = now();

-- Optional: Set terms version and URI.
-- UPDATE kb_system_settings
--    SET terms_version = '1.0',
--        terms_resource_uri = 'kb://+temper-system/general/concept/terms',
--        updated = now();
```

- [ ] **Step 3: Run the migration**

Run: `sqlx migrate run --database-url postgresql://temper:temper@localhost:5437/temper_development`
Expected: `Applied 20260407000001/migrate system access gate`

- [ ] **Step 4: Verify tables and functions exist**

Run: `psql postgresql://temper:temper@localhost:5437/temper_development -c "\dt kb_system_settings; \dt kb_join_requests; SELECT has_system_access('00000000-0000-0000-0004-000000000001'); SELECT is_system_admin('00000000-0000-0000-0004-000000000001');"`
Expected: Both tables exist. `has_system_access` returns `true` (open mode). `is_system_admin` returns `false` (no team members yet).

- [ ] **Step 5: Regenerate sqlx cache**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo sqlx prepare --workspace -- --all-features`
Expected: Updated `.sqlx/` cache files

- [ ] **Step 6: Commit**

```bash
git add migrations/20260407000001_system_access_gate.sql migrations/templates/system_initialization.sql .sqlx/
git commit -m "feat: add system access gate migration and seed template

Creates kb_system_settings, kb_join_requests, has_system_access() and
is_system_admin() SQL functions, and bootstraps the temper-system team."
```

---

## Task 2: Core Types

**Files:**
- Create: `crates/temper-core/src/types/access_gate.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Create the access_gate types module**

```rust
// crates/temper-core/src/types/access_gate.rs
//! Types for the system access gate: join requests, system settings, and entitlements.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a join request in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "join_request_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JoinRequestStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}

/// A user-initiated request to join a team (typically the gating team).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JoinRequest {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A join request with the requesting profile's display info (for admin queue).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct JoinRequestWithProfile {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined from kb_profiles
    pub display_name: String,
    pub email: Option<String>,
}

/// Instance-wide system settings (singleton row).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SystemSettings {
    pub id: i32,
    pub access_mode: String,
    pub gating_team_slug: Option<String>,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
    pub updated: DateTime<Utc>,
}

/// Public-facing system settings (no gating_team_slug — prevents info leakage).
#[derive(Debug, Clone, Serialize)]
pub struct PublicSystemSettings {
    pub access_mode: String,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
}

impl From<SystemSettings> for PublicSystemSettings {
    fn from(s: SystemSettings) -> Self {
        Self {
            access_mode: s.access_mode,
            terms_version: s.terms_version,
            terms_resource_uri: s.terms_resource_uri,
            instance_name: s.instance_name,
        }
    }
}

/// Entitlements included in the profile response — tells the client
/// what this profile is allowed to do at the system level.
#[derive(Debug, Clone, Serialize)]
pub struct Entitlements {
    pub system_access: bool,
    pub is_admin: bool,
    pub join_request_status: Option<JoinRequestStatus>,
}
```

- [ ] **Step 2: Register the module and add re-exports**

Add to `crates/temper-core/src/types/mod.rs`:

After the line `pub mod access;`, add:
```rust
pub mod access_gate;
```

In the re-export section at the bottom, add:
```rust
pub use access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-core`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/access_gate.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(core): add access gate types — JoinRequest, SystemSettings, Entitlements"
```

---

## Task 3: ApiError Extension

**Files:**
- Modify: `crates/temper-api/src/error.rs`

- [ ] **Step 1: Add the SystemAccessRequired variant**

In `crates/temper-api/src/error.rs`, add after the `Forbidden` variant:

```rust
    #[error("System access required")]
    SystemAccessRequired,
```

- [ ] **Step 2: Add the match arm in IntoResponse**

In the `into_response` method, update the `(status, code)` match to include:

```rust
            ApiError::SystemAccessRequired => (StatusCode::FORBIDDEN, "SYSTEM_ACCESS_REQUIRED"),
```

Add a logging arm in the logging match block:

```rust
            ApiError::SystemAccessRequired => {
                tracing::info!(status_code, error_code = code, "system access required");
            }
```

- [ ] **Step 3: Override the message for SystemAccessRequired**

The default `self.to_string()` would produce "System access required" from the `#[error]` attribute. We want a more helpful message. Update the `let message = ...` line to:

```rust
        let message = match &self {
            ApiError::SystemAccessRequired => "This system requires team membership. Contact your administrator for access.".to_string(),
            other => other.to_string(),
        };
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: Compiles with no errors

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/error.rs
git commit -m "feat(api): add SystemAccessRequired error variant"
```

---

## Task 4: Access Service

**Files:**
- Create: `crates/temper-api/src/services/access_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Create the access service module**

```rust
// crates/temper-api/src/services/access_service.rs
//! Access gate service — system access checks, join request lifecycle, entitlements.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
};
use temper_core::types::ids::EventId;

use crate::error::{ApiError, ApiResult};

// ---------------------------------------------------------------------------
// System access checks (called by middleware)
// ---------------------------------------------------------------------------

/// Check if a profile has system-level access.
/// In `open` mode this always returns true.
/// In `invite_only` mode the profile must be a member of the gating team.
pub async fn has_system_access(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query_scalar!(
        "SELECT has_system_access($1)",
        profile_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(result.unwrap_or(false))
}

/// Check if a profile is a system admin (owner of the gating team).
pub async fn is_system_admin(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query_scalar!(
        "SELECT is_system_admin($1)",
        profile_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(result.unwrap_or(false))
}

// ---------------------------------------------------------------------------
// System settings
// ---------------------------------------------------------------------------

/// Read the singleton system settings row.
pub async fn get_system_settings(pool: &PgPool) -> ApiResult<SystemSettings> {
    let row = sqlx::query_as!(
        SystemSettings,
        "SELECT id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated FROM kb_system_settings LIMIT 1",
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Return the public-safe subset of system settings (no gating_team_slug).
pub async fn get_public_settings(pool: &PgPool) -> ApiResult<PublicSystemSettings> {
    get_system_settings(pool).await.map(PublicSystemSettings::from)
}

// ---------------------------------------------------------------------------
// Join request lifecycle
// ---------------------------------------------------------------------------

/// Parameters for creating a join request.
pub struct CreateJoinRequestParams {
    pub profile_id: Uuid,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
}

/// Submit a join request for the gating team.
///
/// Returns `BadRequest` if the system is in `open` mode (no request needed).
/// The partial unique index on `kb_join_requests` prevents duplicate pending requests.
pub async fn create_join_request(
    pool: &PgPool,
    params: CreateJoinRequestParams,
) -> ApiResult<JoinRequest> {
    let settings = get_system_settings(pool).await?;

    if settings.access_mode == "open" {
        return Err(ApiError::BadRequest(
            "System is in open mode — no access request needed".to_string(),
        ));
    }

    let gating_slug = settings.gating_team_slug.ok_or_else(|| {
        ApiError::Internal("System is invite_only but no gating team configured".to_string())
    })?;

    // Resolve team ID from slug
    let team_id = sqlx::query_scalar!(
        "SELECT id FROM kb_teams WHERE slug = $1 AND is_active = true",
        gating_slug,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ApiError::Internal(format!("Gating team '{gating_slug}' not found or inactive"))
    })?;

    let request_id = Uuid::now_v7();
    let accepted_terms_at = params.accepted_terms_version.as_ref().map(|_| chrono::Utc::now());

    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        INSERT INTO kb_join_requests
            (id, team_id, requesting_profile_id, status, message, source,
             accepted_terms_version, accepted_terms_at, created, updated)
        VALUES ($1, $2, $3, 'pending', $4, $5, $6, $7, now(), now())
        RETURNING id, team_id, requesting_profile_id,
                  status as "status: JoinRequestStatus",
                  message, source, accepted_terms_version, accepted_terms_at,
                  reviewed_by_profile_id, reviewed_at, decision_note,
                  created, updated
        "#,
        request_id,
        team_id,
        params.profile_id,
        params.message,
        params.source,
        params.accepted_terms_version,
        accepted_terms_at,
    )
    .fetch_one(pool)
    .await?;

    // Emit audit event
    emit_join_request_event(pool, params.profile_id, row.id, "join_request.submitted").await;

    Ok(row)
}

/// Get the most recent join request for this profile against the gating team.
pub async fn get_own_request(pool: &PgPool, profile_id: Uuid) -> ApiResult<Option<JoinRequest>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(None);
    };

    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        SELECT jr.id, jr.team_id, jr.requesting_profile_id,
               jr.status as "status: JoinRequestStatus",
               jr.message, jr.source, jr.accepted_terms_version, jr.accepted_terms_at,
               jr.reviewed_by_profile_id, jr.reviewed_at, jr.decision_note,
               jr.created, jr.updated
          FROM kb_join_requests jr
          JOIN kb_teams t ON t.id = jr.team_id
         WHERE jr.requesting_profile_id = $1
           AND t.slug = $2
         ORDER BY jr.created DESC
         LIMIT 1
        "#,
        profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Withdraw the pending join request for this profile.
pub async fn withdraw_request(pool: &PgPool, profile_id: Uuid) -> ApiResult<()> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Err(ApiError::NotFound);
    };

    let result = sqlx::query_scalar!(
        r#"
        UPDATE kb_join_requests jr
           SET status = 'withdrawn', updated = now()
          FROM kb_teams t
         WHERE jr.team_id = t.id
           AND jr.requesting_profile_id = $1
           AND t.slug = $2
           AND jr.status = 'pending'
        RETURNING jr.id
        "#,
        profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    match result {
        Some(request_id) => {
            emit_join_request_event(pool, profile_id, request_id, "join_request.withdrawn").await;
            Ok(())
        }
        None => Err(ApiError::NotFound),
    }
}

/// List pending join requests with profile info (admin view).
pub async fn list_pending_requests(pool: &PgPool) -> ApiResult<Vec<JoinRequestWithProfile>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(vec![]);
    };

    let rows = sqlx::query_as!(
        JoinRequestWithProfile,
        r#"
        SELECT jr.id, jr.team_id, jr.requesting_profile_id,
               jr.status as "status: JoinRequestStatus",
               jr.message, jr.source, jr.accepted_terms_version, jr.accepted_terms_at,
               jr.reviewed_by_profile_id, jr.reviewed_at, jr.decision_note,
               jr.created, jr.updated,
               p.display_name, p.email
          FROM kb_join_requests jr
          JOIN kb_teams t ON t.id = jr.team_id
          JOIN kb_profiles p ON p.id = jr.requesting_profile_id
         WHERE t.slug = $1
           AND jr.status = 'pending'
         ORDER BY jr.created DESC
        "#,
        gating_slug,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Parameters for reviewing (approving/rejecting) a join request.
pub struct ReviewRequestParams {
    pub request_id: Uuid,
    pub reviewer_profile_id: Uuid,
    pub decision: JoinRequestStatus,
    pub decision_note: Option<String>,
}

/// Approve or reject a join request. On approval, atomically insert team membership.
pub async fn review_request(
    pool: &PgPool,
    params: ReviewRequestParams,
) -> ApiResult<JoinRequest> {
    if params.decision != JoinRequestStatus::Approved
        && params.decision != JoinRequestStatus::Rejected
    {
        return Err(ApiError::BadRequest(
            "Decision must be 'approved' or 'rejected'".to_string(),
        ));
    }

    let mut tx = pool.begin().await.map_err(|e| {
        ApiError::Internal(format!("Failed to begin transaction: {e}"))
    })?;

    // Update the join request
    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        UPDATE kb_join_requests
           SET status = $2,
               reviewed_by_profile_id = $3,
               reviewed_at = now(),
               decision_note = $4,
               updated = now()
         WHERE id = $1
           AND status = 'pending'
        RETURNING id, team_id, requesting_profile_id,
                  status as "status: JoinRequestStatus",
                  message, source, accepted_terms_version, accepted_terms_at,
                  reviewed_by_profile_id, reviewed_at, decision_note,
                  created, updated
        "#,
        params.request_id,
        params.decision as JoinRequestStatus,
        params.reviewer_profile_id,
        params.decision_note,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    // On approval, insert team membership
    if params.decision == JoinRequestStatus::Approved {
        let member_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at, invited_by_profile_id)
            VALUES ($1, $2, $3, 'watcher', now(), $4)
            ON CONFLICT (team_id, profile_id) DO NOTHING
            "#,
            member_id,
            row.team_id,
            row.requesting_profile_id,
            params.reviewer_profile_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await.map_err(|e| {
        ApiError::Internal(format!("Failed to commit transaction: {e}"))
    })?;

    // Emit audit event (outside transaction — best-effort)
    let event_type = if params.decision == JoinRequestStatus::Approved {
        "join_request.approved"
    } else {
        "join_request.rejected"
    };
    emit_join_request_event(pool, row.requesting_profile_id, row.id, event_type).await;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Entitlements
// ---------------------------------------------------------------------------

/// Build the entitlements object for a profile.
pub async fn get_entitlements(pool: &PgPool, profile_id: Uuid) -> ApiResult<Entitlements> {
    let system_access = has_system_access(pool, profile_id).await?;
    let is_admin = is_system_admin(pool, profile_id).await?;
    let request = get_own_request(pool, profile_id).await?;

    Ok(Entitlements {
        system_access,
        is_admin,
        join_request_status: request.map(|r| r.status),
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Emit a join request lifecycle event to kb_events (best-effort, no error propagation).
async fn emit_join_request_event(
    pool: &PgPool,
    profile_id: Uuid,
    join_request_id: Uuid,
    event_type: &str,
) {
    let event_id = EventId::new();
    let payload = serde_json::json!({ "join_request_id": join_request_id });

    let _ = sqlx::query(
        "INSERT INTO kb_events (id, profile_id, device_id, event_type, payload, created)
         VALUES ($1, $2, 'system', $3, $4, now())",
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(event_type)
    .bind(payload)
    .execute(pool)
    .await;
}
```

- [ ] **Step 2: Register in services/mod.rs**

Add to `crates/temper-api/src/services/mod.rs`:
```rust
pub mod access_service;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/access_service.rs crates/temper-api/src/services/mod.rs
git commit -m "feat(api): add access_service — system access checks and join request lifecycle"
```

---

## Task 5: System Access Middleware

**Files:**
- Create: `crates/temper-api/src/middleware/system_access.rs`
- Modify: `crates/temper-api/src/middleware/mod.rs`

- [ ] **Step 1: Create the system access middleware**

```rust
// crates/temper-api/src/middleware/system_access.rs
//! Middleware that enforces system-level access.
//!
//! Applied to the gated router — all routes that require the caller to be
//! an approved member of the gating team. Routes in the auth-only router
//! (profile, access endpoints) bypass this middleware entirely via the
//! router split in routes.rs.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::types::AuthenticatedProfile;

use crate::error::ApiError;
use crate::services::access_service;
use crate::state::AppState;

/// Axum middleware that checks system-level access after authentication.
///
/// Reads `AuthenticatedProfile` from request extensions (set by `require_auth`)
/// and calls `has_system_access`. Returns `SystemAccessRequired` if the profile
/// is not an approved member of the gating team.
pub async fn require_system_access(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let profile = request
        .extensions()
        .get::<AuthenticatedProfile>()
        .ok_or_else(|| {
            // This should never happen — require_auth runs first.
            ApiError::Internal("AuthenticatedProfile not found in request extensions".to_string())
        })?;

    let has_access = access_service::has_system_access(&state.pool, profile.profile.id).await?;

    if !has_access {
        return Err(ApiError::SystemAccessRequired);
    }

    Ok(next.run(request).await)
}
```

- [ ] **Step 2: Register in middleware/mod.rs**

Update `crates/temper-api/src/middleware/mod.rs`:
```rust
pub mod auth;
pub mod system_access;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/middleware/system_access.rs crates/temper-api/src/middleware/mod.rs
git commit -m "feat(api): add require_system_access middleware"
```

---

## Task 6: Access Handlers

**Files:**
- Create: `crates/temper-api/src/handlers/access.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`

- [ ] **Step 1: Create the access handlers module**

```rust
// crates/temper-api/src/handlers/access.rs
//! Handlers for the system access gate endpoints.
//!
//! Public endpoints (auth_only router):
//! - POST /api/access/requests — submit a join request
//! - GET  /api/access/requests/me — check own request status
//! - DELETE /api/access/requests/me — withdraw pending request
//! - GET  /api/access/settings — read public system settings
//!
//! Admin endpoints (gated router, handler-level admin check):
//! - GET   /api/access/admin/requests — list pending requests
//! - PATCH /api/access/admin/requests/:id — approve or reject

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::access_gate::{
    JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
};

use crate::error::{ApiError, ApiResult};
use crate::middleware::auth::AuthUser;
use crate::services::access_service;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRequestBody {
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRequestBody {
    pub status: JoinRequestStatus,
    pub decision_note: Option<String>,
}

// ---------------------------------------------------------------------------
// Public endpoints (auth_only router)
// ---------------------------------------------------------------------------

/// POST /api/access/requests — submit a join request for the gating team.
pub async fn create_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequestBody>,
) -> ApiResult<(StatusCode, Json<JoinRequest>)> {
    let params = access_service::CreateJoinRequestParams {
        profile_id: auth.0.profile.id,
        message: body.message,
        source: body.source,
        accepted_terms_version: body.accepted_terms_version,
    };

    let request = access_service::create_join_request(&state.pool, params).await?;
    Ok((StatusCode::CREATED, Json(request)))
}

/// GET /api/access/requests/me — check own join request status.
pub async fn get_own_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Option<JoinRequest>>> {
    let request = access_service::get_own_request(&state.pool, auth.0.profile.id).await?;
    Ok(Json(request))
}

/// DELETE /api/access/requests/me — withdraw a pending join request.
pub async fn withdraw_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<StatusCode> {
    access_service::withdraw_request(&state.pool, auth.0.profile.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/access/settings — read public system settings.
pub async fn get_settings(
    State(state): State<AppState>,
) -> ApiResult<Json<PublicSystemSettings>> {
    access_service::get_public_settings(&state.pool).await.map(Json)
}

// ---------------------------------------------------------------------------
// Admin endpoints (gated router, handler-level admin check)
// ---------------------------------------------------------------------------

/// GET /api/access/admin/requests — list pending join requests (admin only).
pub async fn list_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<JoinRequestWithProfile>>> {
    let is_admin = access_service::is_system_admin(&state.pool, auth.0.profile.id).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }

    access_service::list_pending_requests(&state.pool).await.map(Json)
}

/// PATCH /api/access/admin/requests/:id — approve or reject a join request (admin only).
pub async fn review_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(request_id): Path<Uuid>,
    Json(body): Json<ReviewRequestBody>,
) -> ApiResult<Json<JoinRequest>> {
    let is_admin = access_service::is_system_admin(&state.pool, auth.0.profile.id).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }

    let params = access_service::ReviewRequestParams {
        request_id,
        reviewer_profile_id: auth.0.profile.id,
        decision: body.status,
        decision_note: body.decision_note,
    };

    access_service::review_request(&state.pool, params).await.map(Json)
}
```

- [ ] **Step 2: Register in handlers/mod.rs**

Add to `crates/temper-api/src/handlers/mod.rs`:
```rust
pub mod access;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/handlers/access.rs crates/temper-api/src/handlers/mod.rs
git commit -m "feat(api): add access gate handlers — join requests and admin review"
```

---

## Task 7: Router Split & Profile Entitlements

**Files:**
- Modify: `crates/temper-api/src/routes.rs`
- Modify: `crates/temper-api/src/handlers/profiles.rs`

- [ ] **Step 1: Split the router in routes.rs**

Replace the current `protected` router block in `crates/temper-api/src/routes.rs` with two routers — `auth_only` and `gated`:

```rust
pub fn create_app(state: AppState) -> Router {
    use axum::routing::{delete, get, patch, post, put};

    let public = Router::new().route("/api/health", get(handlers::health::health_check));

    // Auth-only routes: authenticated but NOT gated by system access.
    // Profile and access endpoints must be reachable before approval.
    let auth_only = Router::new()
        .route(
            "/api/profile",
            get(handlers::profiles::get).patch(handlers::profiles::update),
        )
        .route(
            "/api/profile/auth-links",
            get(handlers::profiles::list_auth_links),
        )
        .route("/api/access/requests", post(handlers::access::create_request))
        .route(
            "/api/access/requests/me",
            get(handlers::access::get_own_request).delete(handlers::access::withdraw_request),
        )
        .route("/api/access/settings", get(handlers::access::get_settings))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    // Gated routes: require both authentication AND system access.
    // Default-deny — new routes go here unless explicitly exempt.
    let gated = Router::new()
        .route(
            "/api/resources",
            get(handlers::resources::list).post(handlers::resources::create),
        )
        .route(
            "/api/resources/{id}",
            get(handlers::resources::get)
                .patch(handlers::resources::update)
                .delete(handlers::resources::delete),
        )
        .route(
            "/api/resources/{id}/content",
            get(handlers::resources::get_content),
        )
        .route("/api/resources/{id}/meta", put(handlers::meta::update_meta))
        .route(
            "/api/contexts",
            get(handlers::contexts::list).post(handlers::contexts::create),
        )
        .route("/api/contexts/{id}", get(handlers::contexts::get))
        .route("/api/ingest", post(handlers::ingest::create))
        .route("/api/ingest/{id}", put(handlers::ingest::update))
        .route("/api/events", get(handlers::events::list))
        .route("/api/search", post(handlers::search::search))
        .route("/api/sync/status", post(handlers::sync::status))
        .route("/api/sync/complete", post(handlers::sync::complete))
        .route("/api/sync/manifest", get(handlers::sync::manifest))
        // Admin access endpoints (system access required first, then admin check in handler)
        .route(
            "/api/access/admin/requests",
            get(handlers::access::list_pending),
        )
        .route(
            "/api/access/admin/requests/{id}",
            patch(handlers::access::review_request),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::system_access::require_system_access,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let cors = cors_layer(&state);

    let mut app = Router::new().merge(public).merge(auth_only).merge(gated);

    // ... rest of the function unchanged (swagger, fallback, layers) ...
```

**Important:** The middleware layers on `gated` are applied in reverse order — `require_auth` runs first (outermost layer), then `require_system_access` runs second.

- [ ] **Step 2: Check for doc_types handler import**

The existing `routes.rs` may or may not already reference `handlers::doc_types`. Search the handlers module for a `doc_types` module. If it doesn't exist, remove the `/api/doc-types` route from the gated block above. Check the current routes.rs for any routes not accounted for in the split.

Run: `grep -r "doc_types" crates/temper-api/src/handlers/`

If it doesn't exist, remove the `.route("/api/doc-types", ...)` line.

- [ ] **Step 3: Add entitlements to profile GET response**

Create a wrapper response type and update the profile handler in `crates/temper-api/src/handlers/profiles.rs`:

Add at the top of the file:
```rust
use temper_core::types::access_gate::Entitlements;
use crate::services::access_service;
```

Add a response struct:
```rust
#[derive(serde::Serialize)]
pub struct ProfileWithEntitlements {
    #[serde(flatten)]
    pub profile: Profile,
    pub entitlements: Entitlements,
}
```

Update the `get` handler:
```rust
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<ProfileWithEntitlements>> {
    let profile = profile_service::get_by_id(&state.pool, auth.0.profile.id).await?;
    let entitlements = access_service::get_entitlements(&state.pool, auth.0.profile.id).await?;

    Ok(Json(ProfileWithEntitlements {
        profile,
        entitlements,
    }))
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: Compiles with no errors

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/routes.rs crates/temper-api/src/handlers/profiles.rs
git commit -m "feat(api): split router into auth-only and gated, add profile entitlements

Default-deny: new routes go to gated router unless explicitly exempt.
Profile GET now includes entitlements (system_access, is_admin, join_request_status)."
```

---

## Task 8: E2E Test Setup Updates

**Files:**
- Modify: `tests/e2e/tests/common/mod.rs`

- [ ] **Step 1: Add cleanup for new tables**

In the `clean_and_seed` function, add cleanup for `kb_join_requests` before the existing `kb_team_members` cleanup:

```rust
    sqlx::query("DELETE FROM kb_join_requests")
        .execute(pool)
        .await
        .expect("clean kb_join_requests");
```

- [ ] **Step 2: Add well-known UUID constants for new entities**

Add to the constants section at the top of `common/mod.rs`:

```rust
pub const TEMPER_SYSTEM_TEAM_ID: &str = "00000000-0000-0000-0000-000000000002";
pub const TEMPER_SYSTEM_GENERAL_CONTEXT_ID: &str = "00000000-0000-0000-0000-000000000003";
```

- [ ] **Step 3: Add helper to generate a second authenticated user**

Add a new function after `generate_expired_jwt`:

```rust
/// Generate a JWT for a second test user (distinct from the primary e2e user).
pub fn generate_second_user_jwt() -> String {
    generate_test_jwt("e2e-second-user", "second@test.example.com")
}
```

- [ ] **Step 4: Add helper to enable invite-only mode**

```rust
/// Enable invite-only mode in tests by running the seed template steps.
pub async fn enable_invite_only(pool: &PgPool, admin_profile_id: uuid::Uuid) {
    // Add admin as owner of temper-system team
    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
         VALUES (gen_random_uuid(), $1::uuid, $2, 'owner', now())
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(uuid::Uuid::parse_str(TEMPER_SYSTEM_TEAM_ID).unwrap())
    .bind(admin_profile_id)
    .execute(pool)
    .await
    .expect("add admin to temper-system team");

    // Flip to invite_only
    sqlx::query(
        "UPDATE kb_system_settings SET access_mode = 'invite_only', gating_team_slug = 'temper-system', updated = now()",
    )
    .execute(pool)
    .await
    .expect("enable invite_only mode");
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p temper-e2e --features test-db`
Expected: Compiles with no errors (or warnings about dead code, which is fine)

- [ ] **Step 6: Commit**

```bash
git add tests/e2e/tests/common/mod.rs
git commit -m "test: add e2e helpers for access gate — cleanup, invite-only mode, second user"
```

---

## Task 9: E2E Lifecycle Tests

**Files:**
- Create: `tests/e2e/tests/access_gate_test.rs`

- [ ] **Step 1: Create the test file with open mode tests**

```rust
// tests/e2e/tests/access_gate_test.rs
//! E2E tests for the system access gate.
//!
//! Verifies the full lifecycle: open mode bypass, invite-only gating,
//! join request creation/withdrawal/approval/rejection, entitlements,
//! and admin endpoints.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use serde_json::Value;

/// In open mode (default), all authenticated users can access gated routes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_mode_allows_all_authenticated_users(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Pre-flight: auto-provision profile
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile request");
    assert_eq!(resp.status(), StatusCode::OK);

    // Gated route should work in open mode
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("resources request");
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Entitlements show system_access=true in open mode.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn entitlements_in_open_mode(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile request");
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = resp.json().await.expect("parse json");
    assert_eq!(body["entitlements"]["system_access"], true);
    assert_eq!(body["entitlements"]["is_admin"], false);
    assert!(body["entitlements"]["join_request_status"].is_null());
}

/// System settings endpoint returns public settings without gating_team_slug.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn system_settings_no_slug_leak(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Pre-flight
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile");

    let resp = app
        .reqwest_client
        .get(app.url("/api/access/settings"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("settings request");
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = resp.json().await.expect("parse json");
    assert_eq!(body["access_mode"], "open");
    // gating_team_slug must NOT appear in the response
    assert!(body.get("gating_team_slug").is_none());
}
```

- [ ] **Step 2: Add invite-only mode gating tests**

Append to the same file:

```rust
/// In invite-only mode, non-members get SYSTEM_ACCESS_REQUIRED on gated routes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invite_only_blocks_non_members(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Pre-flight: auto-provision profile
    let profile_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile request");
    assert_eq!(profile_resp.status(), StatusCode::OK);
    let profile: Value = profile_resp.json().await.expect("parse");
    let profile_id: uuid::Uuid = profile["id"].as_str().unwrap().parse().unwrap();

    // Enable invite-only (the test user is the admin)
    common::enable_invite_only(&pool, profile_id).await;

    // Now create a SECOND user who is NOT a member
    let second_token = common::generate_second_user_jwt();

    // Pre-flight for second user
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // Second user should be blocked on gated routes
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("resources request");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body: Value = resp.json().await.expect("parse json");
    assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED");
}

/// Auth-only routes remain accessible even when blocked by the gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn auth_only_routes_bypass_gate(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Pre-flight as admin
    let profile_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile");
    let profile: Value = profile_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = profile["id"].as_str().unwrap().parse().unwrap();

    common::enable_invite_only(&pool, admin_id).await;

    // Second user (not a member)
    let second_token = common::generate_second_user_jwt();

    // Profile endpoint should still work (auth-only, not gated)
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("profile request");
    assert_eq!(resp.status(), StatusCode::OK);

    // Access settings should still work
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/settings"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("settings request");
    assert_eq!(resp.status(), StatusCode::OK);

    // Own request endpoint should still work
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("own request");
    assert_eq!(resp.status(), StatusCode::OK);
}
```

- [ ] **Step 3: Add full join request lifecycle test**

Append:

```rust
/// Full lifecycle: submit request -> check status -> admin approves -> gated routes unlock.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_approval_lifecycle(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Setup: admin provisions and enables invite-only
    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();

    common::enable_invite_only(&pool, admin_id).await;

    // Second user submits a join request
    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile pre-flight");

    let create_resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({
            "message": "I'd like to use temper",
            "source": "cli",
            "accepted_terms_version": "1.0"
        }))
        .send()
        .await
        .expect("create request");
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let request: Value = create_resp.json().await.expect("parse");
    assert_eq!(request["status"], "pending");
    let request_id = request["id"].as_str().unwrap();

    // Second user checks their own request
    let own_resp = app
        .reqwest_client
        .get(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("own request");
    assert_eq!(own_resp.status(), StatusCode::OK);
    let own: Value = own_resp.json().await.expect("parse");
    assert_eq!(own["status"], "pending");

    // Second user's entitlements show pending
    let profile_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("profile");
    let profile: Value = profile_resp.json().await.expect("parse");
    assert_eq!(profile["entitlements"]["system_access"], false);
    assert_eq!(profile["entitlements"]["join_request_status"], "pending");

    // Second user is still blocked on gated routes
    let blocked = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("blocked");
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    // Admin lists pending requests
    let pending_resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/requests"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("list pending");
    assert_eq!(pending_resp.status(), StatusCode::OK);
    let pending: Vec<Value> = pending_resp.json().await.expect("parse");
    assert!(!pending.is_empty());
    assert!(pending.iter().any(|r| r["id"].as_str() == Some(request_id)));

    // Admin approves the request
    let approve_resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({
            "status": "approved",
            "decision_note": "Welcome!"
        }))
        .send()
        .await
        .expect("approve");
    assert_eq!(approve_resp.status(), StatusCode::OK);
    let approved: Value = approve_resp.json().await.expect("parse");
    assert_eq!(approved["status"], "approved");

    // Second user can now access gated routes
    let unblocked = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("unblocked");
    assert_eq!(unblocked.status(), StatusCode::OK);

    // Second user's entitlements now show system_access=true
    let profile_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("profile after approval");
    let profile: Value = profile_resp.json().await.expect("parse");
    assert_eq!(profile["entitlements"]["system_access"], true);
    assert_eq!(profile["entitlements"]["join_request_status"], "approved");
}
```

- [ ] **Step 4: Add rejection and withdrawal tests**

Append:

```rust
/// Rejection lifecycle: submit -> admin rejects -> can submit again.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_rejection_allows_resubmit(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Setup admin + invite-only
    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();
    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // Submit first request
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("create");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let first: Value = resp.json().await.expect("parse");
    let first_id = first["id"].as_str().unwrap();

    // Admin rejects
    let resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{first_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "status": "rejected", "decision_note": "Not yet" }))
        .send()
        .await
        .expect("reject");
    assert_eq!(resp.status(), StatusCode::OK);

    // User can submit a new request (partial unique index allows it after rejection)
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web", "message": "Please reconsider" }))
        .send()
        .await
        .expect("resubmit");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

/// Withdrawal lifecycle: submit -> withdraw -> can submit again.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_withdraw_allows_resubmit(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();
    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // Submit
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("create");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Withdraw
    let resp = app
        .reqwest_client
        .delete(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("withdraw");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Can submit again
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "cli", "message": "Changed my mind" }))
        .send()
        .await
        .expect("resubmit");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

/// Non-admin cannot access admin endpoints (gets Forbidden, not SystemAccessRequired).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_blocked_from_admin_endpoints(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Setup: admin + invite-only, then approve the second user
    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();
    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // Submit and approve second user so they have system access but are NOT admin
    let create_resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("create");
    let request: Value = create_resp.json().await.expect("parse");
    let request_id = request["id"].as_str().unwrap();

    app.reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "status": "approved" }))
        .send()
        .await
        .expect("approve");

    // Second user (watcher, not admin) tries admin endpoint — should get Forbidden
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("admin list");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body: Value = resp.json().await.expect("parse");
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

/// Duplicate pending request returns conflict.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn duplicate_pending_request_returns_conflict(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();
    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // First request succeeds
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("first request");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Second request with same user + same team + still pending -> conflict
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("duplicate request");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// Creating a request in open mode returns bad request.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_in_open_mode_returns_bad_request(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Pre-flight
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("profile");

    // System is in open mode — requesting access makes no sense
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("request in open mode");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Audit events are written for join request lifecycle.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_events_written_for_lifecycle(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Setup admin + invite-only
    let admin_resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("admin profile");
    let admin: Value = admin_resp.json().await.expect("parse");
    let admin_id: uuid::Uuid = admin["id"].as_str().unwrap().parse().unwrap();
    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    app.reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", second_token))
        .send()
        .await
        .expect("second profile");

    // Submit request
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", second_token))
        .json(&serde_json::json!({ "source": "web" }))
        .send()
        .await
        .expect("create");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let request: Value = resp.json().await.expect("parse");
    let request_id = request["id"].as_str().unwrap();

    // Approve
    app.reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "status": "approved" }))
        .send()
        .await
        .expect("approve");

    // Verify audit events exist
    let submitted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_events WHERE event_type = 'join_request.submitted'",
    )
    .fetch_one(&pool)
    .await
    .expect("count submitted");
    assert!(submitted > 0, "expected join_request.submitted event");

    let approved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_events WHERE event_type = 'join_request.approved'",
    )
    .fetch_one(&pool)
    .await
    .expect("count approved");
    assert!(approved > 0, "expected join_request.approved event");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-e2e --features test-db access_gate`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/access_gate_test.rs
git commit -m "test: add e2e lifecycle tests for system access gate

Tests open mode, invite-only gating, join request submit/approve/reject/withdraw,
entitlements, admin endpoint authorization, duplicate prevention, and audit events."
```

---

## Task 10: Final Verification & sqlx Cache

**Files:**
- Modify: `.sqlx/` (regenerate cache)

- [ ] **Step 1: Run full check suite**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo make check`
Expected: All formatting, clippy, docs, typecheck, and biome checks pass

- [ ] **Step 2: Run all tests**

Run: `cargo make test-all`
Expected: All unit tests, integration tests, and e2e tests pass (including new access gate tests)

- [ ] **Step 3: Regenerate sqlx cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: Updated `.sqlx/` cache files reflecting new queries

- [ ] **Step 4: Commit the updated cache**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx query cache for access gate queries"
```

- [ ] **Step 5: Run check one more time**

Run: `cargo make check`
Expected: Still passes (no regressions from cache update)
