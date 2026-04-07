# System Access Gate — Phase 1: Data Layer & API

**Date:** 2026-04-07
**Task:** 2026-04-07-implement-temper-system-access-gate
**Branch:** jct/temper-system-access-gate
**Research:** R11 — System Access Gate and Owner-Scoped URIs
**Scope:** Database migration, Rust types, API middleware, service layer, handlers, audit events
**Out of scope:** CLI, MCP, SvelteKit UI, Phase 2 (owner-scoped URIs)

---

## 1. Overview

This spec covers the backend implementation of the system access gate — the layer that
bridges authentication ("Auth0 says this is a real person") to authorization ("temper says
this person is allowed to use the system"). After this work, every API route is either
explicitly exempt from the gate or requires the caller to be an approved member of the
gating team.

The gate is modeled as membership in a well-known team (`temper-system` on temperkb.io,
configurable for self-hosted). A `kb_system_settings` singleton controls whether the gate
is active (`invite_only`) or bypassed (`open`). Users request access via a
`kb_join_requests` table; admins approve or reject. Approval atomically inserts a
`watcher`-role team membership.

---

## 2. Database Migration

A single migration file creates the new enum, tables, functions, and bootstrap data.

### 2.1 Enum

```sql
CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');
```

### 2.2 kb_system_settings

Singleton table controlling instance-wide access behavior.

```sql
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
```

The `id = 1` check enforces singleton-ness. Default `open` means self-hosted instances
work immediately without a gate — the operator enables it after confirming their own
membership.

### 2.3 kb_join_requests

User-initiated access requests, distinct from admin-initiated `kb_team_invitations`.

```sql
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

CREATE UNIQUE INDEX idx_join_requests_one_pending
    ON kb_join_requests (team_id, requesting_profile_id)
    WHERE status = 'pending';

CREATE INDEX idx_join_requests_status_created
    ON kb_join_requests (status, created DESC);
```

The partial unique index ensures one pending request per profile per team. After rejection
or withdrawal, a new request can be submitted.

### 2.4 SQL Functions

#### has_system_access

```sql
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
```

#### is_system_admin

```sql
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
```

### 2.5 Bootstrap Data

The migration seeds the `temper-system` team and a `general` context owned by it. Both
use deterministic UUIDs for reproducibility across environments.

```sql
-- System profile already exists from the consolidated schema seed.
-- Insert the temper-system team.
INSERT INTO kb_teams (id, name, slug, description, created_by_profile_id, is_active, created, updated)
VALUES (
    '00000000-0000-0000-0000-000000000002',
    'temper-system',
    'temper-system',
    'System team for instance-wide access control and shared content',
    '00000000-0000-0000-0000-000000000001',  -- system profile
    true,
    now(),
    now()
);

-- Insert general context owned by temper-system team.
INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created)
VALUES (
    '00000000-0000-0000-0000-000000000003',
    'general',
    'kb_teams',
    '00000000-0000-0000-0000-000000000002',  -- temper-system team
    now()
);
```

The system profile UUID (`00000000-...-000000000001`) must match the existing seed. The
team and context UUIDs are chosen to be deterministic and non-colliding with UUIDv7
production IDs.

---

## 3. Seed Template

A template file at `migrations/templates/system_initialization.sql` provides a
copy-and-customize SQL script for self-hosted operators. It inserts a gating team, adds
the operator as owner, and flips `kb_system_settings` to `invite_only`. This file is also
used verbatim in integration tests.

Contents:
- INSERT a gating team (operator fills in name, slug, their profile ID)
- INSERT team member with `owner` role
- UPDATE `kb_system_settings` to `invite_only` with the team slug
- Optionally set `terms_version` and `terms_resource_uri`

---

## 4. Rust Types

### 4.1 New types in temper-core

File: `crates/temper-core/src/types/access_gate.rs`

```rust
// JoinRequestStatus — maps to join_request_status Postgres enum
pub enum JoinRequestStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}

// JoinRequest — mirrors kb_join_requests table
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

// SystemSettings — mirrors kb_system_settings table (internal use)
pub struct SystemSettings {
    pub id: i32,
    pub access_mode: String,
    pub gating_team_slug: Option<String>,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
    pub updated: DateTime<Utc>,
}

// Entitlements — public-facing shape in profile response
pub struct Entitlements {
    pub system_access: bool,
    pub is_admin: bool,
    pub join_request_status: Option<JoinRequestStatus>,
}
```

All types derive `sqlx::FromRow` (where applicable), `Serialize`, `Deserialize`.
`JoinRequestStatus` derives `sqlx::Type` with `rename_all = "snake_case"`.

### 4.2 JoinRequestWithProfile

A view struct for the admin queue that joins profile metadata:

```rust
pub struct JoinRequestWithProfile {
    // All JoinRequest fields, plus:
    pub display_name: String,
    pub email: Option<String>,
}
```

### 4.3 ApiError extension

File: `crates/temper-api/src/error.rs`

New variant:

```rust
pub enum ApiError {
    // ... existing variants ...
    SystemAccessRequired,
}
```

Maps to 403 with structured body:

```json
{
  "error": {
    "code": "SYSTEM_ACCESS_REQUIRED",
    "message": "This system requires team membership. Contact your administrator for access."
  }
}
```

No team slug, access mode, or other internal details are leaked. The authenticated user
can call `/api/access/requests/me` and `/api/access/settings` to learn more through
proper channels.

---

## 5. Router Structure

### 5.1 Split design (default-deny)

The existing single protected router is split into two nested routers, both behind
`require_auth`. The gated router adds a second middleware layer (`require_system_access`).
New routes default to the gated router unless explicitly placed in the auth-only router.

```
public_routes (no auth)
  GET /api/health

auth_only_routes (require_auth only)
  GET  /api/profile
  PATCH /api/profile
  GET  /api/profile/auth-links
  POST /api/access/requests
  GET  /api/access/requests/me
  DELETE /api/access/requests/me
  GET  /api/access/settings

gated_routes (require_auth + require_system_access)
  GET/POST    /api/resources
  GET/PATCH/DELETE /api/resources/:id
  GET  /api/resources/:id/content
  PUT  /api/resources/:id/meta
  GET/POST /api/contexts
  GET  /api/contexts/:id
  POST /api/search
  POST /api/sync/status
  POST /api/sync/complete
  GET  /api/sync/manifest
  POST /api/ingest
  PUT  /api/ingest/:id
  GET  /api/events
  GET  /api/access/admin/requests
  PATCH /api/access/admin/requests/:id
```

### 5.2 require_system_access middleware

File: `crates/temper-api/src/middleware/system_access.rs`

- Extracts `AuthenticatedProfile` from request extensions (set by `require_auth`)
- Calls `access_service::has_system_access(pool, profile_id)`
- If false: returns `ApiError::SystemAccessRequired`
- If true: passes through to handler

---

## 6. Service Layer

File: `crates/temper-api/src/services/access_service.rs`

### 6.1 System access checks

```rust
pub async fn has_system_access(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool>
pub async fn is_system_admin(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool>
```

Both call their respective SQL functions via `sqlx::query_scalar!()`.

### 6.2 System settings

```rust
pub async fn get_system_settings(pool: &PgPool) -> ApiResult<SystemSettings>
```

Reads the singleton row.

### 6.3 Join request lifecycle

```rust
pub struct CreateJoinRequestParams {
    pub profile_id: Uuid,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
}

pub async fn create_join_request(
    pool: &PgPool,
    params: CreateJoinRequestParams,
) -> ApiResult<JoinRequest>
```

- Reads `kb_system_settings` to find the gating team slug
- Returns `BadRequest` if system is in `open` mode (no request needed)
- Resolves team ID from slug
- Inserts `kb_join_requests` row with `pending` status
- Writes `join_request.submitted` event to `kb_events`

```rust
pub async fn get_own_request(
    pool: &PgPool,
    profile_id: Uuid,
) -> ApiResult<Option<JoinRequest>>
```

Returns the most recent join request for this profile against the gating team.

```rust
pub async fn withdraw_request(pool: &PgPool, profile_id: Uuid) -> ApiResult<()>
```

Sets status to `withdrawn` on the pending request. Writes `join_request.withdrawn` event.

```rust
pub async fn list_pending_requests(
    pool: &PgPool,
) -> ApiResult<Vec<JoinRequestWithProfile>>
```

Admin view: pending requests joined with profile display_name and email, ordered by
created DESC.

```rust
pub struct ReviewRequestParams {
    pub request_id: Uuid,
    pub reviewer_profile_id: Uuid,
    pub decision: JoinRequestStatus,  // Approved or Rejected only
    pub decision_note: Option<String>,
}

pub async fn review_request(
    pool: &PgPool,
    params: ReviewRequestParams,
) -> ApiResult<JoinRequest>
```

- Validates decision is `Approved` or `Rejected`
- In a transaction:
  - Updates the join request status, `reviewed_by_profile_id`, `reviewed_at`, `decision_note`
  - On approval: inserts `kb_team_members` row with `watcher` role
- Writes `join_request.approved` or `join_request.rejected` event

### 6.4 Entitlements

```rust
pub async fn get_entitlements(
    pool: &PgPool,
    profile_id: Uuid,
) -> ApiResult<Entitlements>
```

Combines `has_system_access()`, `is_system_admin()`, and `get_own_request()` into one
`Entitlements` struct. Called by the profile handler.

---

## 7. Handler Layer

File: `crates/temper-api/src/handlers/access.rs`

### 7.1 Public-facing endpoints (auth_only router)

#### POST /api/access/requests

```rust
pub async fn create_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequestBody>,
) -> ApiResult<(StatusCode, Json<JoinRequest>)>
```

Body:
```json
{
  "message": "I'd like to use temper for my team's knowledge base",
  "source": "web",
  "accepted_terms_version": "1.0"
}
```

Returns 201 with the created `JoinRequest`.

#### GET /api/access/requests/me

```rust
pub async fn get_own_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Option<JoinRequest>>>
```

Returns the most recent join request, or `null` if none.

#### DELETE /api/access/requests/me

```rust
pub async fn withdraw_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<StatusCode>
```

Returns 204 on success. Returns 404 if no pending request exists.

#### GET /api/access/settings

```rust
pub async fn get_settings(
    State(state): State<AppState>,
) -> ApiResult<Json<PublicSystemSettings>>
```

Returns a filtered view — no `gating_team_slug`:
```json
{
  "access_mode": "invite_only",
  "terms_version": "1.0",
  "terms_resource_uri": "kb://...",
  "instance_name": "temperkb.io"
}
```

### 7.2 Admin endpoints (gated router, handler-level admin check)

#### GET /api/access/admin/requests

```rust
pub async fn list_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<JoinRequestWithProfile>>>
```

Checks `is_system_admin(profile_id)`, returns `Forbidden` if not. Returns pending
requests with profile metadata.

#### PATCH /api/access/admin/requests/:id

```rust
pub async fn review_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(request_id): Path<Uuid>,
    Json(body): Json<ReviewRequestBody>,
) -> ApiResult<Json<JoinRequest>>
```

Body:
```json
{
  "status": "approved",
  "decision_note": "Welcome aboard"
}
```

Checks `is_system_admin`, calls `access_service::review_request`.

---

## 8. Profile Handler Extension

The existing `GET /api/profile` handler calls `access_service::get_entitlements()` and
includes the result in the response. This is a non-breaking addition — existing consumers
that don't read `entitlements` are unaffected.

Response shape gains:
```json
{
  "id": "...",
  "display_name": "...",
  "entitlements": {
    "system_access": true,
    "is_admin": false,
    "join_request_status": null
  }
}
```

---

## 9. Audit Events

Join request lifecycle events are written to `kb_events` with `resource_id = NULL`:

| event_type | profile_id | payload |
|------------|------------|---------|
| `join_request.submitted` | requesting profile | `{ join_request_id }` |
| `join_request.approved` | requesting profile | `{ join_request_id, reviewed_by }` |
| `join_request.rejected` | requesting profile | `{ join_request_id, reviewed_by, decision_note }` |
| `join_request.withdrawn` | requesting profile | `{ join_request_id }` |

---

## 10. Testing Strategy

### 10.1 SQL function tests

Integration tests (behind `test-db` feature flag) that verify:
- `has_system_access` returns true in `open` mode for any profile
- `has_system_access` returns false in `invite_only` mode for non-member
- `has_system_access` returns true in `invite_only` mode for gating team member
- `is_system_admin` returns true only for `owner` role in gating team
- `is_system_admin` returns false for `watcher`, `member`, `maintainer` roles

### 10.2 Service tests

Integration tests that verify:
- Join request creation succeeds in `invite_only` mode
- Join request creation returns `BadRequest` in `open` mode
- Partial unique index prevents duplicate pending requests
- Approval atomically creates team membership
- Rejection allows a new request to be submitted
- Withdrawal sets status correctly
- Entitlements reflect current state accurately

### 10.3 Handler / route tests

Integration tests that verify:
- Gated routes return `SYSTEM_ACCESS_REQUIRED` for non-members in `invite_only` mode
- Auth-only routes (`/api/profile`, `/api/access/*`) remain accessible
- Admin endpoints return `Forbidden` for non-admins
- The full lifecycle: create request -> admin approves -> gated routes now accessible

### 10.4 Seed template

The `migrations/templates/system_initialization.sql` is used in integration tests to
set up the `invite_only` scenario. Tests run the template verbatim against the test
database.

---

## 11. Files Changed

### New files
- `migrations/2026MMDD_______system_access_gate.sql` — migration
- `migrations/templates/system_initialization.sql` — seed template
- `crates/temper-core/src/types/access_gate.rs` — new types
- `crates/temper-api/src/services/access_service.rs` — new service
- `crates/temper-api/src/handlers/access.rs` — new handlers
- `crates/temper-api/src/middleware/system_access.rs` — new middleware

### Modified files
- `crates/temper-core/src/types/mod.rs` — export `access_gate` module
- `crates/temper-api/src/error.rs` — add `SystemAccessRequired` variant
- `crates/temper-api/src/routes.rs` — split into auth_only + gated routers
- `crates/temper-api/src/services/mod.rs` — export `access_service`
- `crates/temper-api/src/handlers/mod.rs` — export `access`
- `crates/temper-api/src/handlers/profile.rs` — add entitlements to GET response
- `crates/temper-api/src/middleware/mod.rs` — export `system_access`
