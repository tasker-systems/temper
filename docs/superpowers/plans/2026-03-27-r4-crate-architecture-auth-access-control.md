# R4: Crate Architecture, Auth & Access Control — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Update the unified R2 migration with R4 schema changes (profiles, auth links, teams, access control functions) and produce Rust type stubs in a new `temper-core` types module — all as a research deliverable, no runtime code.

**Architecture:** Postgres-as-authority. Schema DDL and Rust types co-evolve as thinking tools. The migration is the source of truth; Rust types mirror it with `sqlx::FromRow`/`sqlx::Type` derives. Access control lives in SQL functions that compose into CTEs, vector search, and graph traversal. No crate split yet — Rust types land in the existing `temper-cli` crate under a `cloud/` module namespace that will become `temper-core` during I-phase.

**Tech Stack:** PostgreSQL 18, pgvector 0.8.2, sqlx 0.8 (FromRow/Type derives only), Rust (type definitions only — no runtime implementation)

**Design Spec:** `docs/superpowers/specs/2026-03-27-r4-crate-architecture-auth-access-control-design.md`

---

## File Structure

```
migrations/
  20260326000001_r2_schema.sql      # MODIFY — evolve kb_profiles, add auth links, teams, access control
  20260326000002_r2_seed.sql         # MODIFY — update seed profiles for new schema
src/
  cloud/
    mod.rs                           # CREATE — module root, re-exports
    types/
      mod.rs                         # CREATE — type module root
      auth.rs                        # CREATE — AuthProvider, AuthClaims, AuthenticatedProfile
      profile.rs                     # CREATE — Profile, ProfileAuthLink, DeactivationCheck
      team.rs                        # CREATE — Team, TeamMember, TeamRole
      access.rs                      # CREATE — AccessLevel, TeamResource, AccessScoped trait
      invitation.rs                  # CREATE — TeamInvitation, InvitationStatus
      ownership.rs                   # CREATE — ResourceOwnership
  lib.rs                             # MODIFY — add `pub mod cloud;`
```

The `src/cloud/types/` module is the future `temper-core`. During I-phase crate split, this module lifts out wholesale. Placing it under `cloud/` keeps it visually separate from existing vault/CLI code while staying in the same compilation unit for now.

---

### Task 1: Evolve `kb_profiles` — Remove Auth Provider Fields, Add R4 Columns

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql:32-41`

- [ ] **Step 1: Replace the `kb_profiles` table definition**

Replace the existing R2 placeholder `kb_profiles` (lines 32-41) with the R4 version that removes auth provider fields and adds profile-domain columns:

```sql
CREATE TABLE kb_profiles (
    id              UUID PRIMARY KEY,              -- UUIDv7
    display_name    VARCHAR(128) NOT NULL,
    email           VARCHAR(256),                  -- cached from default auth provider
    avatar_url      TEXT,
    preferences     JSONB NOT NULL DEFAULT '{}',   -- theme, default project, notifications
    vault_config    JSONB NOT NULL DEFAULT '{}',   -- local vault path, sync preferences
    is_active       BOOLEAN NOT NULL DEFAULT true,
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated         TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Verify the migration file parses correctly**

Run: `grep -c 'CREATE TABLE kb_profiles' migrations/20260326000001_r2_schema.sql`
Expected: `1`

- [ ] **Step 3: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "schema: evolve kb_profiles — remove auth provider fields, add preferences and vault_config"
```

---

### Task 2: Add `kb_profile_auth_links` Table

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql` (insert after `kb_profiles` table)

- [ ] **Step 1: Add the auth links table immediately after `kb_profiles`**

Insert after the `kb_profiles` CREATE TABLE block:

```sql
CREATE TABLE kb_profile_auth_links (
    id                        UUID PRIMARY KEY,              -- UUIDv7
    profile_id                UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    auth_provider             VARCHAR(32) NOT NULL,          -- "neon_auth", "auth0", "okta", etc.
    auth_provider_user_id     VARCHAR(128) NOT NULL,         -- external identity ID from this provider
    email                     VARCHAR(256),                  -- email from this provider at link time
    is_default                BOOLEAN NOT NULL DEFAULT false, -- which link is the primary identity
    linked_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(auth_provider, auth_provider_user_id)
);

CREATE INDEX idx_auth_links_profile ON kb_profile_auth_links(profile_id);
CREATE INDEX idx_auth_links_email ON kb_profile_auth_links(email);
```

- [ ] **Step 2: Verify both tables exist in the migration**

Run: `grep 'CREATE TABLE kb_profile' migrations/20260326000001_r2_schema.sql`
Expected output includes both `kb_profiles` and `kb_profile_auth_links`.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "schema: add kb_profile_auth_links — provider identity reconciliation via email"
```

---

### Task 3: Add Ownership and Soft-Delete Columns to `resources`

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql:43-56` (resources table)

- [ ] **Step 1: Add three columns to the `resources` table definition**

Add these columns to the `resources` CREATE TABLE block, after the `mimetype` column and before `created`:

```sql
    originator_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    owner_profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    is_active           BOOLEAN NOT NULL DEFAULT true,
```

- [ ] **Step 2: Add an index for owner lookups**

Insert after the existing `idx_resources_updated` index:

```sql
CREATE INDEX idx_resources_owner ON resources(owner_profile_id);
CREATE INDEX idx_resources_originator ON resources(originator_profile_id);
```

- [ ] **Step 3: Verify the resources table has the new columns**

Run: `grep -c 'originator_profile_id\|owner_profile_id\|is_active' migrations/20260326000001_r2_schema.sql`
Expected: `3` (one for each column)

- [ ] **Step 4: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "schema: add resource ownership (originator/owner) and soft-delete to resources"
```

---

### Task 4: Add Postgres Enums and Team Tables

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql` (append before `kb_events`)

- [ ] **Step 1: Add the Postgres enum types**

Insert before the `kb_events` table definition:

```sql
-- R4: Team and access control enums
CREATE TYPE team_role AS ENUM ('owner', 'maintainer', 'member', 'watcher');
CREATE TYPE access_level AS ENUM ('vault', 'mutable', 'immutable');
CREATE TYPE invitation_status AS ENUM ('pending', 'accepted', 'declined', 'expired');
```

- [ ] **Step 2: Add the `kb_teams` table**

Insert after the enum definitions:

```sql
CREATE TABLE kb_teams (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    name                    VARCHAR(128) NOT NULL,
    slug                    VARCHAR(128) NOT NULL UNIQUE,
    description             VARCHAR(512),
    metadata                JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
    is_active               BOOLEAN NOT NULL DEFAULT true,
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                 TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 3: Add the `kb_team_members` table**

```sql
CREATE TABLE kb_team_members (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    profile_id              UUID NOT NULL REFERENCES kb_profiles(id),
    role                    team_role NOT NULL,
    joined_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    invited_by_profile_id   UUID REFERENCES kb_profiles(id),
    UNIQUE(team_id, profile_id)
);

CREATE INDEX idx_team_members_profile ON kb_team_members(profile_id);
CREATE INDEX idx_team_members_team ON kb_team_members(team_id);
```

- [ ] **Step 4: Add the `kb_team_resources` table**

```sql
CREATE TABLE kb_team_resources (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    resource_id             UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    access_level            access_level NOT NULL,
    added_by_profile_id     UUID NOT NULL REFERENCES kb_profiles(id),
    added_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, resource_id)
);

CREATE INDEX idx_team_resources_resource ON kb_team_resources(resource_id);
CREATE INDEX idx_team_resources_team ON kb_team_resources(team_id);
```

- [ ] **Step 5: Add the `kb_team_invitations` table**

```sql
CREATE TABLE kb_team_invitations (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    invited_email           VARCHAR(256) NOT NULL,
    invited_by_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
    role                    team_role NOT NULL,
    token                   VARCHAR(128) NOT NULL UNIQUE,
    status                  invitation_status NOT NULL DEFAULT 'pending',
    expires_at              TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '7 days',
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, invited_email)
);

CREATE INDEX idx_invitations_token ON kb_team_invitations(token);
CREATE INDEX idx_invitations_email ON kb_team_invitations(invited_email);
```

- [ ] **Step 6: Verify all four team tables exist**

Run: `grep 'CREATE TABLE kb_team' migrations/20260326000001_r2_schema.sql`
Expected: four lines — `kb_teams`, `kb_team_members`, `kb_team_resources`, `kb_team_invitations`.

- [ ] **Step 7: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "schema: add team tables — kb_teams, kb_team_members, kb_team_resources, kb_team_invitations"
```

---

### Task 5: Add Composable SQL Access Control Functions

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql` (append after team tables, before `kb_events`)

- [ ] **Step 1: Add the `resources_visible_to` function**

```sql
-- R4: Composable access control functions
-- These are STABLE (no side effects) so the query planner can inline them.
-- They compose into CTEs, subqueries, and joins for vector search and graph traversal.

CREATE FUNCTION resources_visible_to(
    p_profile_id UUID,
    p_team_id UUID DEFAULT NULL
) RETURNS TABLE(resource_id UUID, access_level VARCHAR(32), via VARCHAR(256))
LANGUAGE SQL STABLE AS $$
    -- Resources I own (always visible, full control)
    SELECT id, 'owner'::VARCHAR(32), 'ownership'::VARCHAR(256)
    FROM resources
    WHERE owner_profile_id = p_profile_id
      AND is_active = true

    UNION

    -- Resources shared with teams I belong to
    SELECT tr.resource_id, tr.access_level::VARCHAR(32), ('team:' || t.slug)::VARCHAR(256)
    FROM kb_team_resources tr
    JOIN kb_teams t ON t.id = tr.team_id
    JOIN kb_team_members tm ON tm.team_id = tr.team_id
    WHERE tm.profile_id = p_profile_id
      AND t.is_active = true
      AND (p_team_id IS NULL OR tr.team_id = p_team_id)
$$;
```

- [ ] **Step 2: Add the `can_modify_resource` function**

```sql
CREATE FUNCTION can_modify_resource(
    p_profile_id UUID,
    p_resource_id UUID
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        -- I own it
        SELECT 1 FROM resources
        WHERE id = p_resource_id AND owner_profile_id = p_profile_id
    ) OR EXISTS (
        -- It's vault or mutable in a team I belong to, and I'm not a watcher
        SELECT 1
        FROM kb_team_resources tr
        JOIN kb_team_members tm ON tm.team_id = tr.team_id
        WHERE tr.resource_id = p_resource_id
          AND tm.profile_id = p_profile_id
          AND tr.access_level IN ('vault', 'mutable')
          AND tm.role != 'watcher'
    )
$$;
```

- [ ] **Step 3: Add the `can_manage_team` function**

```sql
CREATE FUNCTION can_manage_team(
    p_profile_id UUID,
    p_team_id UUID,
    p_action VARCHAR(32)  -- 'invite', 'remove', 'change_role', 'delete'
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_team_members
        WHERE team_id = p_team_id
          AND profile_id = p_profile_id
          AND (
            (p_action = 'delete' AND role = 'owner')
            OR (p_action IN ('invite', 'remove', 'change_role')
                AND role IN ('owner', 'maintainer'))
          )
    )
$$;
```

- [ ] **Step 4: Verify all three functions exist**

Run: `grep 'CREATE FUNCTION' migrations/20260326000001_r2_schema.sql`
Expected: three lines — `resources_visible_to`, `can_modify_resource`, `can_manage_team`.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "schema: add composable SQL access control functions — visibility, modification, team management"
```

---

### Task 6: Update Seed Data for R4 Profile Schema

**Files:**
- Modify: `migrations/20260326000002_r2_seed.sql:59-61`

- [ ] **Step 1: Update the seed profile inserts**

Replace the existing profile inserts (lines 59-61) with inserts matching the new schema. Profiles no longer have `provider`/`external_id` — those move to `kb_profile_auth_links`:

```sql
INSERT INTO kb_profiles (id, display_name) VALUES
    ('00000000-0000-0000-0004-000000000001', 'System'),
    ('00000000-0000-0000-0004-000000000002', 'Anonymous');

INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, is_default) VALUES
    ('00000000-0000-0000-0005-000000000001', '00000000-0000-0000-0004-000000000001', 'system', 'system', true),
    ('00000000-0000-0000-0005-000000000002', '00000000-0000-0000-0004-000000000002', 'anonymous', 'anonymous', true);
```

- [ ] **Step 2: Verify the seed file no longer references `provider` or `external_id`**

Run: `grep -c 'provider\|external_id' migrations/20260326000002_r2_seed.sql`
Expected: `2` (only the `auth_provider` and `auth_provider_user_id` references in the auth links insert)

- [ ] **Step 3: Commit**

```bash
git add migrations/20260326000002_r2_seed.sql
git commit -m "schema: update R2 seed data for R4 profile and auth link schema"
```

---

### Task 7: Create `cloud` Module Root and Type Module Structure

**Files:**
- Create: `src/cloud/mod.rs`
- Create: `src/cloud/types/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/cloud/mod.rs`**

```rust
//! Temper Cloud types and abstractions.
//!
//! This module contains the domain types that will become `temper-core`
//! during the I-phase crate split. Placed under `cloud/` to keep it
//! visually separate from existing vault/CLI code.

pub mod types;
```

- [ ] **Step 2: Create `src/cloud/types/mod.rs`**

```rust
//! Domain types for temper-cloud — profiles, teams, access control, auth.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow`.
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`.
//! Postgres enums map directly via `sqlx::Type` with `type_name` attributes.

pub mod access;
pub mod auth;
pub mod invitation;
pub mod ownership;
pub mod profile;
pub mod team;

pub use access::{AccessLevel, AccessScoped, TeamResource};
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile};
pub use invitation::{InvitationStatus, TeamInvitation};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use team::{Team, TeamMember, TeamRole};
```

- [ ] **Step 3: Add `pub mod cloud;` to `src/lib.rs`**

Add at the end of the existing module declarations in `src/lib.rs`:

```rust
pub mod cloud;
```

- [ ] **Step 4: Verify the module structure compiles**

Run: `cargo check 2>&1 | head -5`
Expected: Errors about missing files (auth.rs, profile.rs, etc.) — that's expected, we create those next.

- [ ] **Step 5: Commit**

```bash
git add src/cloud/mod.rs src/cloud/types/mod.rs src/lib.rs
git commit -m "feat: add cloud module structure — future temper-core types namespace"
```

---

### Task 8: Create Auth Types

**Files:**
- Create: `src/cloud/types/auth.rs`

- [ ] **Step 1: Create `src/cloud/types/auth.rs`**

```rust
use chrono::{DateTime, Utc};

use super::profile::Profile;

/// Identity provider configuration — Neon Auth default, swappable for enterprise.
///
/// The provider is configuration, not code. The JWT verification middleware
/// is parameterized by `AuthProvider`, not specialized per provider.
/// Neon Auth uses EdDSA (Ed25519) with `sub`. Auth0/Okta use RS256 with `sub`.
#[derive(Debug, Clone)]
pub struct AuthProvider {
    /// Provider identifier: "neon_auth", "auth0", "okta", etc.
    pub name: String,
    /// JWKS endpoint for key discovery (e.g., `{base_url}/.well-known/jwks.json`)
    pub jwks_url: String,
    /// Expected `iss` claim in JWTs
    pub issuer: String,
    /// Expected `aud` claim, if the provider uses it
    pub audience: Option<String>,
    /// Which JWT claim holds the external user ID (usually "sub")
    pub user_id_claim: String,
}

/// JWT claims extracted from any supported identity provider.
///
/// Parsed during middleware verification. The `external_user_id` is the value
/// of the configured `user_id_claim` from the JWT, used to look up the
/// corresponding `ProfileAuthLink`.
#[derive(Debug, Clone)]
pub struct AuthClaims {
    /// Which provider issued this token
    pub provider: String,
    /// External user ID (value of the configured `user_id_claim`)
    pub external_user_id: String,
    /// User's email from token claims
    pub email: String,
    /// Token expiry (Unix timestamp)
    pub exp: i64,
    /// Token issued-at (Unix timestamp)
    pub iat: i64,
}

/// The authenticated identity for the current request.
///
/// Extracted by axum middleware via JWT verification → auth link lookup → profile load.
/// Available to all route handlers as an axum extractor.
#[derive(Debug, Clone)]
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
}
```

- [ ] **Step 2: Verify the file compiles in isolation**

Run: `cargo check 2>&1 | grep 'auth.rs'`
Expected: No errors referencing `auth.rs` (may have errors from other missing files).

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/auth.rs
git commit -m "feat: add auth types — AuthProvider, AuthClaims, AuthenticatedProfile"
```

---

### Task 9: Create Profile Types

**Files:**
- Create: `src/cloud/types/profile.rs`

- [ ] **Step 1: Create `src/cloud/types/profile.rs`**

```rust
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Profile — the temper-domain identity.
///
/// Bridges external auth identity to everything temper cares about:
/// team membership, resource ownership, preferences, vault configuration.
/// A profile is "who I am in temper" regardless of which provider I
/// authenticated through. No auth provider fields — those live in
/// `ProfileAuthLink`.
///
/// Auto-provisioned on first authenticated request. Soft-deleted via
/// `is_active = false` for referential integrity and GDPR compliance.
#[derive(Debug, Clone, FromRow)]
pub struct Profile {
    pub id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub preferences: serde_json::Value,
    pub vault_config: serde_json::Value,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Links an external auth provider identity to a temper profile.
///
/// A profile can have multiple auth links (e.g., Google and GitHub with the
/// same email). Identity reconciliation: when a new provider identity arrives
/// with an email matching an existing link, it auto-links to the same profile.
/// One link is marked `is_default` as the primary identity.
#[derive(Debug, Clone, FromRow)]
pub struct ProfileAuthLink {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub auth_provider: String,
    pub auth_provider_user_id: String,
    pub email: Option<String>,
    pub is_default: bool,
    pub linked_at: DateTime<Utc>,
}

/// Result of validating whether a profile can be deactivated.
///
/// Deactivation is blocked if the profile is the sole owner of any active team
/// (must transfer ownership first) or owns resources with no other access path
/// (must transfer or share first).
#[derive(Debug, Clone)]
pub enum DeactivationCheck {
    /// Safe to deactivate
    Ready,
    /// Must resolve these issues first
    Blocked {
        /// Teams where this profile is the only owner
        sole_owner_teams: Vec<Uuid>,
        /// Count of resources owned by this profile not in any team
        orphaned_resource_count: u32,
    },
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `cargo check 2>&1 | grep 'profile.rs'`
Expected: No errors referencing `profile.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/profile.rs
git commit -m "feat: add profile types — Profile, ProfileAuthLink, DeactivationCheck"
```

---

### Task 10: Create Team Types

**Files:**
- Create: `src/cloud/types/team.rs`

- [ ] **Step 1: Create `src/cloud/types/team.rs`**

```rust
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Team role — strict hierarchy: Owner > Maintainer > Member > Watcher.
///
/// Maps directly to the `team_role` Postgres enum. Four roles is small enough
/// that explicit matching in SQL functions and Rust logic is clearer than a
/// join-table permission model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

/// A team — the unit of collaboration in temper.
///
/// Teams are fully owned by temper, not delegated to the auth provider.
/// This means the team model survives auth provider swaps. A team must
/// always have exactly one owner. Soft-deleted via `is_active = false`.
#[derive(Debug, Clone, FromRow)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub metadata: serde_json::Value,
    pub created_by_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A profile's membership in a team with a specific role.
#[derive(Debug, Clone, FromRow)]
pub struct TeamMember {
    pub id: Uuid,
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by_profile_id: Option<Uuid>,
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `cargo check 2>&1 | grep 'team.rs'`
Expected: No errors referencing `team.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/team.rs
git commit -m "feat: add team types — TeamRole, Team, TeamMember"
```

---

### Task 11: Create Access Control Types

**Files:**
- Create: `src/cloud/types/access.rs`

- [ ] **Step 1: Create `src/cloud/types/access.rs`**

```rust
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::auth::AuthenticatedProfile;

/// Access level for a resource within a team scope.
///
/// Maps directly to the `access_level` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "access_level", rename_all = "snake_case")]
pub enum AccessLevel {
    /// Collaborative ownership — any team member (member role or above) can
    /// modify or delete. Deletion means full removal from the temper system.
    /// Essential for shared tickets, milestones, research notes, session notes.
    Vault,

    /// Team members can read and edit content, but only the resource owner can
    /// remove it from the team or delete it entirely. Useful for shared specs,
    /// plans, reference documents.
    Mutable,

    /// Read-only for all team members. The owner controls all mutations,
    /// sharing decisions, and removal. Useful for published research,
    /// finalized decisions, reference material.
    Immutable,
}

/// A resource's scoped presence in a team with an explicit access level.
///
/// A resource can belong to multiple teams simultaneously with different
/// access levels per team.
#[derive(Debug, Clone, FromRow)]
pub struct TeamResource {
    pub id: Uuid,
    pub team_id: Uuid,
    pub resource_id: Uuid,
    pub access_level: AccessLevel,
    pub added_by_profile_id: Uuid,
    pub added_at: DateTime<Utc>,
}

/// Marker trait for types that participate in access-scoped queries.
///
/// The actual enforcement is in SQL via `resources_visible_to()`,
/// `can_modify_resource()`, and `can_manage_team()`. This trait provides
/// the Rust-side interface for constructing scoped query parameters.
/// The database is the authority; Rust is the caller.
pub trait AccessScoped {
    /// The profile ID to scope visibility to
    fn profile_id(&self) -> Uuid;
    /// Optional team scope narrowing (None = all teams the profile belongs to)
    fn team_id(&self) -> Option<Uuid>;
}

impl AccessScoped for AuthenticatedProfile {
    fn profile_id(&self) -> Uuid {
        self.profile.id
    }

    fn team_id(&self) -> Option<Uuid> {
        None // default: visible across all teams
    }
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `cargo check 2>&1 | grep 'access.rs'`
Expected: No errors referencing `access.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/access.rs
git commit -m "feat: add access control types — AccessLevel, TeamResource, AccessScoped trait"
```

---

### Task 12: Create Invitation Types

**Files:**
- Create: `src/cloud/types/invitation.rs`

- [ ] **Step 1: Create `src/cloud/types/invitation.rs`**

```rust
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::team::TeamRole;

/// Invitation status — lifecycle of a team invitation.
///
/// Maps directly to the `invitation_status` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "invitation_status", rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Declined,
    Expired,
}

/// A pending or resolved invitation to join a team.
///
/// Primary flow is link-based: invite generates a token-bearing URL,
/// recipient clicks, authenticates, profile auto-created if needed,
/// joins team. CLI commands: `temper team invite`, `temper team join`,
/// `temper team request-join`.
///
/// Constraints:
/// - `role` cannot be `Owner` — ownership is only transferred, never invited
/// - One pending invite per email per team
/// - 7-day default expiry, checked at acceptance time
/// - Acceptance is idempotent
#[derive(Debug, Clone, FromRow)]
pub struct TeamInvitation {
    pub id: Uuid,
    pub team_id: Uuid,
    pub invited_email: String,
    pub invited_by_profile_id: Uuid,
    pub role: TeamRole,
    pub token: String,
    pub status: InvitationStatus,
    pub expires_at: DateTime<Utc>,
    pub created: DateTime<Utc>,
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `cargo check 2>&1 | grep 'invitation.rs'`
Expected: No errors referencing `invitation.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/invitation.rs
git commit -m "feat: add invitation types — InvitationStatus, TeamInvitation"
```

---

### Task 13: Create Ownership Types

**Files:**
- Create: `src/cloud/types/ownership.rs`

- [ ] **Step 1: Create `src/cloud/types/ownership.rs`**

```rust
use uuid::Uuid;

/// Resource ownership — present on every resource.
///
/// Two distinct concepts:
/// - `originator_profile_id`: Immutable provenance — who created this resource.
///   Never changes, even on ownership transfer. Part of the permanent audit trail.
/// - `owner_profile_id`: Mutable control — who currently manages this resource.
///   Defaults to originator at creation. Can be transferred (e.g., when someone
///   leaves a team and hands off their work).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceOwnership {
    /// Immutable provenance — who created this resource
    pub originator_profile_id: Uuid,
    /// Mutable control — who currently manages this resource
    pub owner_profile_id: Uuid,
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `cargo check 2>&1 | grep 'ownership.rs'`
Expected: No errors referencing `ownership.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/cloud/types/ownership.rs
git commit -m "feat: add ownership types — ResourceOwnership with originator/owner split"
```

---

### Task 14: Full Compilation Check and Final Commit

**Files:**
- All files from Tasks 1-13

- [ ] **Step 1: Run full cargo check**

Run: `cargo check --all-features 2>&1`
Expected: Compiles with zero errors. Warnings about unused code are acceptable (these are type stubs).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-features 2>&1`
Expected: No errors. Warnings about unused code are acceptable for type stubs.

- [ ] **Step 3: Verify migration file coherence**

Run: `wc -l migrations/20260326000001_r2_schema.sql`
Expected: Approximately 250-280 lines (original ~157 lines + ~100 lines of R4 additions).

- [ ] **Step 4: Review the complete migration for table ordering**

Verify that tables are created in dependency order:
1. `kb_contexts`, `kb_doc_types`, `kb_behaviors` (no FK dependencies)
2. `kb_doc_type_behaviors` (depends on doc_types, behaviors)
3. `kb_profiles` (no FK dependencies)
4. `kb_profile_auth_links` (depends on profiles)
5. `resources` (depends on contexts, doc_types, profiles)
6. Behavior state tables (depend on resources)
7. `kb_chunks`, `kb_ingestion_records` (depend on resources)
8. Enum types (no dependencies)
9. `kb_teams` (depends on profiles)
10. `kb_team_members` (depends on teams, profiles)
11. `kb_team_resources` (depends on teams, resources)
12. `kb_team_invitations` (depends on teams, profiles)
13. SQL functions (depend on resources, teams, team_members, team_resources)
14. `kb_events` (depends on profiles, contexts, resources)

- [ ] **Step 5: Commit if any fixups were needed**

```bash
git add -A
git commit -m "chore: R4 migration and type stubs — final coherence check"
```
