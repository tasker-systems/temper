# R4: Crate Architecture, Auth & Access Control — Design Spec

## Overview

R4 connects the platform decisions from R3 (Neon + Vercel + R2 + Neon Auth) to the domain model: how identity flows through the system, who owns what, who can see what, and how access control composes into every query. The deliverable is a research spec with unified migration DDL and Rust type stubs — progressively more concrete than R2/R3, so that R5 (indexing, sync, resource lifecycle) has the complete picture to build from.

### What Prior Research Established

| Phase | Key Decisions |
|-------|--------------|
| R1 | Workflow & lifecycle vision — resource behaviors, lifecycle stages, event-driven patterns |
| R2 | Postgres as single source of truth — flat resources, versioned chunks, pgvector, `kb://` URIs. `kb_profiles` as placeholder. |
| R3 | Neon + Vercel + Cloudflare R2. Neon Auth for identity (Option B — identity only, temper owns domain). Five-crate architecture. Graph via recursive CTEs, not AGE. |

### Why R4 Before R5

Access control must be a precondition to queries, not a filter after them. If we build indexing and search strategies (R5) without profile/team/role/access scope, we have to retrofit it — and retrofitting access control into vector search and graph traversal is error-prone and architecturally ugly. By establishing the access boundary now, vector search and graph traversal only ever operate within the visible set. No post-hoc filtering means no information leakage through result counts, ordering artifacts, or timing differences.

## 1. Crate Architecture

### Workspace Layout

```
temper/
├── Cargo.toml              # workspace root
├── crates/
│   ├── temper-core/        # types, traits, sqlx FromRow models — the shared vocabulary
│   ├── temper-client/      # auth-aware API wrapper (JWT lifecycle, token refresh, HTTP)
│   ├── temper-cli/         # lightweight CLI — vault ops, uses temper-client for cloud
│   ├── temper-api/         # axum routes, auth middleware, handlers
│   ├── temper-cloud/       # Vercel runtime adapter — thin deployment glue
│   ├── temper-mcp/         # MCP server — vault+permission-aware tool provider
│   └── temper-embed/       # kreuzberg/ONNX — separate binary, not bundled in CLI
├── migrations/             # shared, at workspace root
└── docs/
```

### Dependency Graph

```
temper-cloud ──→ temper-api ──→ temper-core
temper-cli ──→ temper-client ──→ temper-core
temper-mcp ──→ temper-client ──→ temper-core
temper-embed ──→ temper-core
temper-api ──→ temper-core
```

### What Lives Where

**temper-core** — The shared vocabulary. No runtime, no IO, no framework dependencies beyond sqlx's compile-time `FromRow` derive (Postgres-as-authority commitment makes DB-agnostic core unnecessary).

Contains:
- Domain types: `Profile`, `ProfileAuthLink`, `Team`, `TeamMember`, `Resource`, `Chunk`, `Context`, `DocType`
- Behavior state types: `WorkflowableState`, `SequenceableState`, `AssignableState`, `TaggableState`
- Team/access types: `TeamRole`, `AccessLevel`, `TeamInvitation`, `TeamResource`
- Ownership types: `ResourceOwnership` (originator + owner)
- Auth types: `AuthProvider`, `AuthClaims`, `AuthenticatedProfile`
- Traits: `Ownable`, `TeamScopable`, `AccessScoped`
- Enums mapped to Postgres enums: `TeamRole`, `AccessLevel`, `InvitationStatus`
- `kb://` URI types from R2

**temper-client** — Auth-aware API wrapper. The shared consumer layer that CLI, MCP, and any future consumer use to interact with temper-api. Handles JWT lifecycle, token refresh, HTTP client operations. Never talks to the database directly.

Contains:
- HTTP client wrapping temper-api endpoints
- Token storage and refresh logic (`~/.temper/auth.json`)
- CLI auth callback server (ephemeral port, browser redirect)
- Request/response types for API interactions

**temper-cli** — Lightweight CLI. Everything that exists today minus the TUI (which is being retired in favor of SvelteKit web UI) and minus embedding (which moves to temper-embed as a separate binary). Uses temper-client for cloud operations.

Contains:
- CLI commands (vault ops, ticket management, session notes, search)
- Local vault operations (file materialization, sync)
- Local HNSW index as optional offline cache

**temper-api** — The portable axum application. Could run anywhere — Vercel, Docker, bare metal.

Contains:
- JWT verification middleware (JWKS fetch, Ed25519 validation, profile lookup)
- Route handlers: resources, profiles, teams, search, upload tokens
- Access control enforcement via composable SQL functions
- Cloudflare R2 presigned URL generation
- sqlx connection pool management

**temper-cloud** — Vercel deployment adapter. Wires temper-api into Vercel's runtime, maps environment variables, handles preview deployment concerns. Approximately 100 lines. The separation exists because temper-api is portable — an on-prem or enterprise deployment path uses a different adapter, not a different application.

**temper-mcp** — MCP server for agent-assisted workflows. Uses temper-client for the same vault and permission-aware operations as CLI and web. Same access control, same auth, different interface.

**temper-embed** — Isolated ML runtime. Ships as a separate binary (`temper-embed` or `temper-embed-cli`), not bundled into temper-cli. kreuzberg balanced (bge-base-en-v1.5, 768-dim) for embedding, document extraction for PDF/DOCX/etc., chunking logic. Heavy ONNX runtime dependencies stay isolated from the lightweight CLI.

### Design Rationale: temper-client as Shared Consumer Layer

Multiple consumers (CLI, MCP, future tools) need authenticated API access. Extracting temper-client means the auth flow and API interaction logic is written once. Each consumer wires it into their runtime:

- **CLI**: `temper-client` handles auth, CLI provides the command surface
- **MCP**: `temper-client` handles auth, MCP server provides the tool surface
- **Web (SvelteKit)**: Uses Neon Auth JS SDK directly — doesn't need temper-client (which is Rust)

### Reference Architecture

The crate separation pattern mirrors `tasker-core/crates/{tasker-client,tasker-mcp,tasker-ctl}` — same mental model with one key distinction: tasker supports both REST and gRPC transport, while temper is REST-only (Vercel constraint, no current need for dual transport).

### TUI Removal

The ratatui-based TUI is being retired. The project vision has grown to include teams, permissions, and collaborative workflows — managing these in a TUI is not the right fit. A SvelteKit web UI on Vercel (similar to storyteller-site, tracked under the temper-cloud-ui-sveltekit-on-vercel milestone) is the replacement. TUI code does not migrate during the crate split.

### Migration Path

The crate split is an I-phase concern, not R4. R4 produces the target architecture and type definitions. The current monolithic `temper-cli` crate continues working until the I-phase implementation creates the workspace structure. All I-phase work happens as feature branches off the `jcoletaylor/temper-cloud` branch.

## 2. Identity Layer — Provider-Agnostic Auth

### Design Principle

Authentication is provider-agnostic. Neon Auth is the default identity provider, but the system is designed so that swapping to Auth0, Okta, or any JWT-issuing provider requires only configuration changes — the entire domain model (profiles, teams, resources, access control) survives untouched.

### The Identity Seam

```
auth_provider.user_id  →  kb_profile_auth_links.auth_provider_user_id  →  kb_profiles.id
```

Auth provider identities are linked to profiles via `kb_profile_auth_links`, not stored on the profile itself. A profile can have multiple linked providers (e.g., Google and GitHub with the same email). The profile is a pure temper-domain entity with no auth provider contamination. Everything downstream works with `kb_profiles.id`.

### Identity Reconciliation Policy

If a user authenticates from a legitimate provider with the same email address as an existing profile's linked identity, we treat them as the same person and auto-link the new provider. This is documented as public policy. The reconciliation flow:

1. JWT arrives → extract provider + user_id
2. Check `kb_profile_auth_links` for matching `(auth_provider, auth_provider_user_id)`
3. If found → load that profile
4. If not found → check if email matches an existing link's email → auto-link new provider to existing profile
5. If no match at all → create new profile + first auth link (marked as default)

### Auth Provider Configuration

```rust
/// Identity provider configuration — Neon Auth default, swappable for enterprise
pub struct AuthProvider {
    /// Provider identifier: "neon_auth", "auth0", "okta", etc.
    pub name: String,
    /// JWKS endpoint for key discovery
    pub jwks_url: String,
    /// Expected `iss` claim in JWTs
    pub issuer: String,
    /// Expected `aud` claim, if the provider uses it
    pub audience: Option<String>,
    /// Which JWT claim holds the external user ID (usually "sub")
    pub user_id_claim: String,
}
```

The provider is configuration, not code. Neon Auth uses EdDSA (Ed25519) JWTs with `sub` as the user ID claim. Auth0 uses RS256 with `sub`. Okta uses RS256 with `sub` or `uid`. The verification middleware is parameterized by `AuthProvider`, not specialized per provider.

### JWT Verification Middleware (Axum)

Flow for every authenticated request:

1. Extract `Authorization: Bearer <jwt>` header
2. Decode JWT header to get key ID (`kid`)
3. Validate signature against cached JWKS keys
4. Validate `exp`, `iss`, `aud` claims against `AuthProvider` config
5. Extract user ID from the configured claim (`user_id_claim`)
6. Lookup `kb_profile_auth_links` by `(auth_provider, auth_provider_user_id)` → get `profile_id`
7. If link found → load profile by `profile_id`
8. If link not found → check email reconciliation → auto-link or create new profile + link
9. Inject `AuthenticatedProfile` into axum handler extractors

**JWKS caching**: Fetch on startup, store in `Arc<RwLock<JwksCache>>` in axum state. Refresh when verification fails with unknown key ID (key rotation), rate-limited to prevent abuse. No background polling.

**Profile lookup**: Per-request, not globally cached. Single-row indexed lookup on `kb_profile_auth_links(auth_provider, auth_provider_user_id)` then a PK lookup on `kb_profiles` — two sub-millisecond queries. A shared cache introduces invalidation complexity that isn't justified for the throughput profile of this application.

### Auth Types (temper-core)

```rust
/// JWT claims extracted from any supported provider
pub struct AuthClaims {
    /// Which provider issued this token
    pub provider: String,
    /// External user ID (value of the configured user_id_claim)
    pub external_user_id: String,
    /// User's email from token claims
    pub email: String,
    /// Token expiry (Unix timestamp)
    pub exp: i64,
    /// Token issued-at (Unix timestamp)
    pub iat: i64,
}

/// The authenticated identity for the current request.
/// Extracted by axum middleware, available to all handlers.
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
}
```

### CLI Auth Flow

Modeled after `gh auth login`:

1. `temper auth login` starts a local HTTP server on an ephemeral port
2. Opens browser to provider's login URL with `redirect_uri=http://localhost:{port}/callback`
3. User authenticates with any supported provider (Google, GitHub, email, etc.)
4. Provider redirects to localhost with auth code
5. CLI exchanges code for access token + refresh token
6. Stores credentials at `~/.temper/auth.json`:

```json
{
    "provider": "neon_auth",
    "access_token": "eyJ...",
    "refresh_token": "...",
    "profile_id": "019...",
    "expires_at": "2026-03-27T08:00:00Z"
}
```

7. Subsequent CLI calls: temper-client reads token, checks expiry, refreshes if needed, sends `Authorization: Bearer` header

**SSH/headless fallback**: Display the auth URL for manual copy-paste when no browser is available. The localhost callback still works — user navigates manually.

**Token refresh**: temper-client checks `expires_at` before each request. If expired or within a refresh window, call the provider's token endpoint with the refresh token to get a new access token. Transparent to the caller.

### Web Auth Flow

SvelteKit web UI uses Neon Auth's JS SDK directly for cookie-based session management. This is the path Neon Auth is designed for — no custom work needed. The web UI never touches temper-client (which is Rust).

## 3. Profile Layer — "Who I Am in Temper"

### Design Principle

Profile is the temper-domain identity. It bridges the external auth identity to everything temper cares about — team membership, resource ownership, preferences, vault configuration. A profile is "who I am in temper" regardless of which provider I authenticated through.

### Schema

```sql
CREATE TABLE kb_profiles (
    id                        UUID PRIMARY KEY,              -- UUIDv7
    display_name              VARCHAR(128) NOT NULL,
    email                     VARCHAR(256),                  -- cached from default provider for display
    avatar_url                TEXT,
    preferences               JSONB NOT NULL DEFAULT '{}',   -- theme, default project, notifications
    vault_config              JSONB NOT NULL DEFAULT '{}',   -- local vault path, sync preferences
    is_active                 BOOLEAN NOT NULL DEFAULT true,
    created                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                   TIMESTAMPTZ NOT NULL DEFAULT now()
);

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

This evolves the R2 placeholder (`provider`/`external_id`) by separating auth provider linkage from the profile entirely. The profile is a pure temper-domain entity — no auth provider fields. All provider identities are tracked in `kb_profile_auth_links`, where each row maps one provider identity to one profile. A profile can have multiple linked providers (Google, GitHub, email, etc.) and one is marked as default. Identity reconciliation is temper-owned: when a new provider identity arrives with an email matching an existing link, it auto-links to the same profile.

### Rust Types (temper-core)

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProfileAuthLink {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub auth_provider: String,
    pub auth_provider_user_id: String,
    pub email: Option<String>,
    pub is_default: bool,
    pub linked_at: DateTime<Utc>,
}
```

### Resource Ownership

Two new columns on `resources` (folded into the R2 migration):

```sql
-- On the resources table:
originator_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
owner_profile_id        UUID NOT NULL REFERENCES kb_profiles(id),
is_active               BOOLEAN NOT NULL DEFAULT true
```

- **originator_profile_id** — Immutable provenance. The profile that created this resource. Never changes, even on ownership transfer. Part of the permanent audit trail.
- **owner_profile_id** — Mutable control. The profile that currently manages this resource. Defaults to originator at creation. Can be transferred (e.g., when someone leaves a team and hands off their work).
- **is_active** — Soft delete flag. When false, resource is excluded from all queries including vector search and graph traversal (enforced by SQL access control functions).

```rust
/// Resource ownership — present on every resource
pub struct ResourceOwnership {
    /// Immutable provenance — who created this resource
    pub originator_profile_id: Uuid,
    /// Mutable control — who currently manages this resource
    pub owner_profile_id: Uuid,
}
```

### Profile Lifecycle

- **Auto-provisioned** on first authenticated request: JWT arrives with no matching auth link → create profile from claims (email, display name from provider) + first auth link marked as default
- **Preferences**: JSONB for CLI behavior — default project, sync frequency, embedding mode, theme. Schema-less to evolve without migrations.
- **Vault config**: JSONB for local vault path and sync preferences. Each profile can have different local materialization settings.

## 4. Team Model — Collaboration Boundaries

### Design Principle

Teams are the unit of collaboration in temper. A team is a named scope with role-based membership that controls access to a shared set of resources. Teams are fully owned by temper — not delegated to the auth provider. This means the team model survives auth provider swaps, and we control the role hierarchy, permission semantics, and resource scoping entirely.

### Why Not Neon Auth Organizations

Neon Auth's org plugin was evaluated and rejected:
- Beta status, teams sub-feature not enabled
- Only three fixed roles (owner/admin/member) — temper needs four (owner/maintainer/member/watcher)
- No custom roles or custom permissions
- No resource-level access control
- Building on their org model means fighting it rather than leveraging it

By owning teams, temper can swap auth providers without touching the domain model.

### Schema

```sql
CREATE TYPE team_role AS ENUM ('owner', 'maintainer', 'member', 'watcher');

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

### Rust Types (temper-core)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

#[derive(Debug, Clone, sqlx::FromRow)]
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamMember {
    pub id: Uuid,
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by_profile_id: Option<Uuid>,
}
```

### Role Hierarchy and Permissions

`TeamRole` encodes a strict hierarchy: `Owner > Maintainer > Member > Watcher`. Four roles is small enough that explicit matching in SQL functions and Rust logic is clearer than a join-table permission model.

| Permission | Owner | Maintainer | Member | Watcher |
|-----------|-------|------------|--------|---------|
| Delete team | yes | — | — | — |
| Transfer ownership | yes | — | — | — |
| Manage roles (except owner) | yes | yes | — | — |
| Add/remove members (except owner) | yes | yes | — | — |
| Add own resources to team | yes | yes | yes | — |
| Remove own resources from team | yes | yes | yes | — |
| Add others' resources to team | yes | yes | — | — |
| Read all team resources | yes | yes | yes | yes |
| Modify vault/mutable resources | yes | yes | yes | — |
| Modify immutable resources | — | — | — | — |

### Constraints

- A team must always have exactly one owner
- Ownership transfer is explicit — owner assigns new owner, old owner becomes maintainer
- Team creator is automatically the owner
- Maintainers can modify roles and membership for everyone except the owner
- Soft delete via `is_active = false` — preserves team history and resource associations for audit

## 5. Resource-Team Scoping

### Design Principle

Resources enter and exit teams with explicit access levels. A resource can belong to multiple teams simultaneously with different access levels per team. The access level controls what team members can do with the resource, independent of their team role (though watchers are always read-only regardless of access level).

### Schema

```sql
CREATE TYPE access_level AS ENUM ('vault', 'mutable', 'immutable');

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

### Rust Types (temper-core)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "access_level", rename_all = "snake_case")]
pub enum AccessLevel {
    /// Collaborative ownership — any team member (member role or above) can modify or delete.
    /// Deletion of a vault resource means full removal from the temper system.
    /// The originator retains provenance credit but not ownership.
    /// Essential for shared tickets, milestones, research notes, session notes.
    Vault,

    /// Team members can read and edit content, but only the resource owner can
    /// remove it from the team or delete it entirely.
    /// Useful for shared specs, plans, reference documents — things the team
    /// works on but one person stewards.
    Mutable,

    /// Read-only for all team members. The owner controls all mutations,
    /// sharing decisions, and removal.
    /// Useful for published research, finalized decisions, reference material.
    Immutable,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamResource {
    pub id: Uuid,
    pub team_id: Uuid,
    pub resource_id: Uuid,
    pub access_level: AccessLevel,
    pub added_by_profile_id: Uuid,
    pub added_at: DateTime<Utc>,
}
```

### Lifecycle Semantics

**Adding a resource to a team**: The resource owner (or a maintainer/owner in the target team) adds the resource with an explicit access level. The `added_by_profile_id` records who performed the action.

**Removing a non-vault resource from a team**: The resource disappears from the team's view. The owner retains it in their personal vault. Other teams' associations with the same resource are unaffected.

**Deleting a vault resource**: Full removal from the temper system. The `resources` row is soft-deleted (`is_active = false`) for audit trail, chunks are marked inactive, the resource disappears from all team and personal views. The originator retains provenance credit but not ownership.

**Modification rights by access level and relationship:**

| | Resource Owner | Team Owner/Maintainer/Member | Team Watcher |
|---|---|---|---|
| **vault** | modify, delete, remove from team | modify, delete | read |
| **mutable** | modify, delete, remove, reshare | modify | read |
| **immutable** | modify, delete, remove, reshare | read | read |

### Information Security Boundary

Temper's responsibility for information security stops at the data design and access control layer. We do not attempt to prevent a user who has read access to a resource from syncing it locally, making an untracked copy, and re-uploading it elsewhere. We assume good faith among team members. The EULA informs that sharing data to a team constitutes opt-in to team access, and synced resources cannot be retroactively removed from remote systems.

## 6. Access Control as Composable SQL Functions

### Design Principle

This is the architectural centerpiece. Access control is a data-layer concern, not an application-layer middleware concern. Because temper uses Postgres for everything — search, graph traversal, CRUD, events — access scoping must work inside SQL. The functions are `STABLE` (no side effects, same inputs produce same outputs within a transaction), which means the query planner can inline them.

The critical insight: these functions compose into CTEs, subqueries, and joins. They don't wrap queries from outside. Access control is always present in the query plan, never an afterthought. Records that a profile doesn't have access to are never part of the vector search or graph traversal in the first place.

### Core Functions

#### Resource Visibility

```sql
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

The `access_level` column returns `'owner'` for owned resources (distinct from the `access_level` enum) and the enum value cast to varchar for team-scoped resources. This avoids conflating ownership with vault access — an owner always has full control regardless of how the resource is scoped in teams. The `via` column provides audit context — when reviewing access, you can see *how* a resource became visible (direct ownership vs. which team shared it).

#### Resource Modification Check

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

#### Team Management Check

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

### Composition Patterns

#### With Vector Search

```sql
WITH visible AS (
    SELECT resource_id, access_level FROM resources_visible_to($1)
)
SELECT r.*, c.content, c.embedding <=> $2::vector AS distance
FROM kb_current_chunks c
JOIN resources r ON r.id = c.resource_id
JOIN visible v ON v.resource_id = r.id
ORDER BY c.embedding <=> $2::vector
LIMIT 20;
```

The `visible` CTE establishes the access boundary. The vector search operates only within that boundary. Resources outside the visible set are never scored, never ranked, never returned.

#### With Graph Traversal

```sql
WITH RECURSIVE
  visible AS (SELECT resource_id FROM resources_visible_to($1)),
  seeds AS (
    SELECT c.resource_id, c.embedding <=> $2::vector AS sim
    FROM kb_current_chunks c
    JOIN visible v ON v.resource_id = c.resource_id
    ORDER BY c.embedding <=> $2::vector LIMIT 10
  ),
  expanded AS (
    SELECT s.resource_id, 0 AS hop FROM seeds s
    UNION ALL
    SELECT e.target_id, ex.hop + 1
    FROM expanded ex
    JOIN kb_resource_edges e ON e.source_id = ex.resource_id
    JOIN visible v ON v.resource_id = e.target_id  -- access control in the traversal
    WHERE ex.hop < 2
  )
SELECT DISTINCT r.* FROM expanded ex
JOIN resources r ON r.id = ex.resource_id;
```

Access control is enforced at every hop of the graph traversal, not just at the seed selection. A resource two hops away that the profile can't see is never traversed through, preventing information leakage via graph structure.

### Rust Trait (temper-core)

```rust
/// Marker trait for types that participate in access-scoped queries.
/// The actual enforcement is in SQL — this trait provides the Rust-side
/// interface for constructing scoped query parameters.
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

The trait doesn't re-implement SQL logic. It provides the parameters that get passed into the SQL functions. The database is the authority; Rust is the caller.

## 7. Team Invitations

### Design Principle

Invitations are temper-owned. Without Neon Auth organizations, temper manages the full invitation lifecycle: creation, delivery (via shareable link), acceptance, and expiry. The invitation system is intentionally simple — link-based as the primary flow, with CLI convenience commands for common patterns.

### Schema

```sql
CREATE TYPE invitation_status AS ENUM ('pending', 'accepted', 'declined', 'expired');

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

### Rust Types (temper-core)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "invitation_status", rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Declined,
    Expired,
}

#[derive(Debug, Clone, sqlx::FromRow)]
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

### Invitation Flows

**Link-based (primary)**: Maintainer/owner generates invite → token-bearing URL created → recipient clicks link → authenticates via any provider → profile auto-created if needed → joins team with assigned role. No email delivery built into temper — the inviter shares the link through whatever channel they prefer (Slack, email, etc.).

**CLI-initiated**: `temper team invite <email> --role member --team <slug>` → generates invite → displays URL for sharing. Convenience wrapper around link-based flow.

**CLI acceptance**: `temper team join <token>` → validates token, checks expiry, creates membership. The receiving end of a link-based invite from the command line.

**Request-join (Discord-style)**: `temper team request-join <team-slug>` → creates a pending request that team maintainers/owners can approve or deny. Useful for discoverable teams where membership is open but moderated.

**Direct add**: If the invitee already has a temper profile, owner/maintainer can add them directly by email. Skips the invitation dance. Only works for known, active profiles.

### Constraints

- `role` on invitation cannot be `owner` — ownership is only transferred, never invited
- `UNIQUE(team_id, invited_email)` — one pending invite per email per team
- Expired invitations are checked at acceptance time, not auto-cleaned
- Invitation acceptance is idempotent — accepting an already-accepted invite is a no-op
- 7-day default expiry — configurable per team via `kb_teams.metadata` if needed

## 8. Profile Deactivation

### Design Principle

Deactivation is a soft operation that preserves referential integrity, audit trails, and provenance. It is designed for both "I changed my mind" reversibility and GDPR-style right-to-erasure compliance.

### Pre-Deactivation Checks

Block deactivation if:
- Profile is sole owner of any active team → must transfer ownership first
- Profile owns resources with no other access path (not in any team) → must transfer or share first

```rust
/// Result of validating whether a profile can be deactivated
pub enum DeactivationCheck {
    /// Safe to deactivate
    Ready,
    /// Must resolve these issues first
    Blocked {
        sole_owner_teams: Vec<Uuid>,
        orphaned_resource_count: u32,
    },
}
```

### On Deactivation (`is_active = false`)

1. Remove from all `kb_team_members` rows
2. Pending invitations *to* this profile's email → marked expired
3. Invitations *sent by* this profile → remain valid (invited_by is provenance, not authority)
4. Resources where `owner_profile_id = deactivated` and in a team with vault access → remain accessible via team scope (no owner mutation needed, the resource is collaboratively owned)
5. Resources where `owner_profile_id = deactivated` and not in any team → become orphaned (retained in database but inaccessible until an admin reassigns ownership)
6. `originator_profile_id` **never changes** — audit trail is permanent

### Reactivation

Flip `is_active = true`. Profile can then be re-invited to teams. Resources they still own become accessible again. Simple and reversible.

### GDPR Right-to-Erasure

For permanent deletion requests, scrub PII from the profile row:
- `display_name` → "Deleted User"
- `email` → null
- `avatar_url` → null
- `preferences` → `{}`
- `vault_config` → `{}`

The UUID persists for referential integrity. `originator_profile_id` and `owner_profile_id` references remain structurally valid but resolve to an anonymized record. The provenance chain stays intact without exposing personal information.

## 9. Unified R2 Migration — Schema Addendum

All R4 changes are folded into the existing R2 migration (`20260326000001_r2_schema.sql`) rather than creating a separate migration file. Migrations reflect intent, not planning-level schema evolution. Since no database has been deployed yet, the migration represents the full target schema.

### Changes to Existing R2 Tables

**`kb_profiles`** — replace the placeholder, extract auth linkage:
```sql
-- R2 placeholder:
--   provider VARCHAR(32), external_id VARCHAR(128), display_name, email, created, updated
-- R4 replacement:
--   display_name, email, avatar_url, preferences JSONB, vault_config JSONB, is_active, created, updated
--   Auth provider fields moved to new kb_profile_auth_links table
```

**`resources`** — add ownership and soft-delete columns:
```sql
-- Add to resources table:
originator_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
owner_profile_id        UUID NOT NULL REFERENCES kb_profiles(id),
is_active               BOOLEAN NOT NULL DEFAULT true
```

### New Postgres Enums

```sql
CREATE TYPE team_role AS ENUM ('owner', 'maintainer', 'member', 'watcher');
CREATE TYPE access_level AS ENUM ('vault', 'mutable', 'immutable');
CREATE TYPE invitation_status AS ENUM ('pending', 'accepted', 'declined', 'expired');
```

### New Tables

- `kb_profile_auth_links` — provider identity linkage with default flag, email-based reconciliation
- `kb_teams` — team definitions with slug, metadata, soft-delete
- `kb_team_members` — membership with `team_role` enum, invited-by provenance
- `kb_team_resources` — resource scoping with `access_level` enum, added-by provenance
- `kb_team_invitations` — link-based invitations with `invitation_status` enum, token, expiry

### New SQL Functions

- `resources_visible_to(profile_id, team_id?)` — composable resource visibility
- `can_modify_resource(profile_id, resource_id)` — modification permission check
- `can_manage_team(profile_id, team_id, action)` — team management permission check

### New Indexes

- `idx_auth_links_profile`, `idx_auth_links_email` — auth link lookups and email reconciliation
- `idx_team_members_profile`, `idx_team_members_team` — membership lookups
- `idx_team_resources_resource`, `idx_team_resources_team` — resource scoping lookups
- `idx_invitations_token`, `idx_invitations_email` — invitation resolution
- `idx_resources_owner` on `resources(owner_profile_id)` — ownership lookups

### VARCHAR Discipline

All string columns use scoped `VARCHAR(N)` rather than unbounded `TEXT`, except where content is genuinely unbounded (resource URIs, avatar URLs). Fixed vocabularies use Postgres enums. This provides implicit validation at the database level and makes column constraints visible in the schema.

## 10. Rust Type Stubs — Complete Inventory

All types live in `temper-core`. All struct types derive `Debug, Clone, sqlx::FromRow`. All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`.

### Enums

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "access_level", rename_all = "snake_case")]
pub enum AccessLevel {
    Vault,
    Mutable,
    Immutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "invitation_status", rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Declined,
    Expired,
}
```

### Auth Types

```rust
/// Identity provider configuration
pub struct AuthProvider {
    pub name: String,
    pub jwks_url: String,
    pub issuer: String,
    pub audience: Option<String>,
    pub user_id_claim: String,
}

/// JWT claims extracted from any supported provider
pub struct AuthClaims {
    pub provider: String,
    pub external_user_id: String,
    pub email: String,
    pub exp: i64,
    pub iat: i64,
}

/// Authenticated identity for the current request
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
}
```

### Domain Types

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProfileAuthLink {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub auth_provider: String,
    pub auth_provider_user_id: String,
    pub email: Option<String>,
    pub is_default: bool,
    pub linked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamMember {
    pub id: Uuid,
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by_profile_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamResource {
    pub id: Uuid,
    pub team_id: Uuid,
    pub resource_id: Uuid,
    pub access_level: AccessLevel,
    pub added_by_profile_id: Uuid,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
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

/// Resource ownership — present on every resource
pub struct ResourceOwnership {
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
}

/// Result of validating whether a profile can be deactivated
pub enum DeactivationCheck {
    Ready,
    Blocked {
        sole_owner_teams: Vec<Uuid>,
        orphaned_resource_count: u32,
    },
}
```

### Traits

```rust
/// Marker trait for types that participate in access-scoped queries.
pub trait AccessScoped {
    fn profile_id(&self) -> Uuid;
    fn team_id(&self) -> Option<Uuid>;
}

impl AccessScoped for AuthenticatedProfile {
    fn profile_id(&self) -> Uuid {
        self.profile.id
    }

    fn team_id(&self) -> Option<Uuid> {
        None
    }
}
```

## 11. Open Questions for R5

These questions are intentionally deferred. R4 establishes the access control boundary; R5 designs the operations that work within it.

1. **Sync queue design**: How does `temper sync pull/push` work when scoped to a profile's visible resources across multiple teams?
2. **Chunk version retention**: When a resource is modified, how many chunk versions are retained? Is this per-resource or global policy?
3. **Event scoping**: `kb_events` references `profile_id` — should event visibility follow the same `resources_visible_to` pattern, or are events always visible to the actor who generated them?
4. **Resource transfer mechanics**: What's the UX for ownership transfer? Is it a two-step (offer → accept) or single-step (owner assigns)?
5. **Offline index scope**: When temper-cli builds a local HNSW index for offline search, which resources are included? Everything the profile can see? Only resources in a specific context?
6. **Embedding pipeline integration**: How does temper-embed's output flow into `kb_chunks`? Direct database write? Via temper-api? Via a queue?
7. **Access control performance validation**: Do the SQL scope functions compose efficiently inside recursive CTEs and vector search? Need benchmarks at representative scale (10K resources, 100K chunks, 50 teams).
