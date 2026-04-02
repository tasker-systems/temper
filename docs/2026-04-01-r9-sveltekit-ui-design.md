# R9: SvelteKit UI for temperkb.io — Research & Design

**Date**: 2026-04-01
**Context**: temper
**Goal**: temper-cloud-ui-sveltekit-on-vercel
**Depends On**: R2 (Data Model), R4 (Auth & Access Control), R7 (Knowledge Graph), I5e (Auth Compatibility), I5f (Context CRUD)
**Status**: draft

---

## 1. Executive Summary

This document proposes the architecture and implementation plan for **temper-ui**, a SvelteKit application that serves as the web surface for temperkb.io. The UI provides a public-facing landing site describing temper's capabilities, documentation pages for users and enterprise operators, and authenticated pages for knowledge base management, search, graph traversal, team administration, and permission management.

The design addresses a core tension: **which queries should flow through the existing Rust API versus hitting Neon directly from SvelteKit server-side code**. The guiding principle is that domain actions (mutations, access-controlled reads, search) always use the Rust API with JWT forwarding, while read-only navigation queries that simply populate the UI chrome (context lists, doc-type enumerations, resource counts for dashboards) may query Neon directly from SvelteKit server routes — provided they respect the same access boundaries.

The application lives at `packages/temper-ui` in the temper monorepo, follows the same SvelteKit-on-Vercel + Neon pattern established in `storyteller-site`, and shares Auth0 credentials with the existing `temper-web` SPA application configured in the Auth0 tenant.

---

## 2. Prior Art: storyteller-site Pattern

The sibling repo `~/projects/tasker-systems/storyteller-site` establishes the deployment pattern we will follow:

### 2.1 Architecture

```
storyteller-site/
├── packages/
│   ├── api/              # (optional API package)
│   └── web/              # SvelteKit app
│       ├── src/
│       │   ├── lib/
│       │   │   ├── server/db.ts      # postgres.js → Neon
│       │   │   ├── api.ts            # Client-side types and fetch helpers
│       │   │   ├── components/       # Svelte 5 components (runes mode)
│       │   │   └── styles/global.css
│       │   ├── routes/
│       │   │   ├── +layout.svelte    # Global nav, CSS vars
│       │   │   ├── +page.svelte      # Landing page
│       │   │   ├── explore/          # Data-driven pages
│       │   │   └── api/              # SvelteKit API routes
│       │   └── app.html
│       ├── svelte.config.js          # adapter-vercel, runes mode
│       ├── vite.config.ts
│       └── package.json
```

### 2.2 Key Patterns

| Pattern | storyteller-site Implementation | temper-ui Adaptation |
|---------|-------------------------------|---------------------|
| **DB access** | `postgres` (postgres.js) in `$lib/server/db.ts` with `DATABASE_URL` | Same — Neon serverless driver for read-only navigation queries |
| **Server-side data** | `+page.server.ts` with raw SQL via `sql` tagged template | Same for navigation/chrome; API proxy for domain operations |
| **Svelte version** | Svelte 5 with runes mode (`$props()`, `$state()`, `$derived()`) | Same |
| **Adapter** | `@sveltejs/adapter-vercel` with `nodejs22.x` runtime | Same |
| **Styling** | CSS custom properties, no framework | Tailwind CSS for faster iteration on marketing + app pages |
| **Visualization** | d3.js (v7) for radar charts, genre landscapes | d3.js (v7) for knowledge graph visualization |
| **Auth** | None (public data) | Auth0 SPA SDK via `@auth0/auth0-sveltekit` or custom PKCE |

### 2.3 What Differs

temper-ui has requirements storyteller-site does not:

1. **Authentication** — Auth0 login/logout with JWT-bearing API calls
2. **Mutations** — Creating, updating, deleting resources; managing teams and permissions
3. **Hybrid data sources** — Some reads from Neon directly (navigation), some through Rust API (domain)
4. **Two audience segments** — Public marketing/docs pages + authenticated app pages

---

## 3. Vercel Routing Strategy

### 3.1 Current State

Today, `vercel.json` routes everything through two layers:

```json
{
  "routes": [
    { "handle": "filesystem" },
    { "src": "/(.*)", "dest": "/api/axum" }
  ]
}
```

The filesystem handler matches TypeScript serverless functions in `api/`. The catch-all sends everything else to the Rust Axum binary. This means **there is no room for SvelteKit** in the current routing — the Axum catch-all would swallow all SvelteKit routes.

### 3.2 Proposed Routing

SvelteKit with `adapter-vercel` produces its own serverless functions and static assets. We need to carve out path space so the three concerns (SvelteKit, TypeScript API, Rust API) coexist.

**Option A — Path-prefix partitioning (Recommended)**

```json
{
  "routes": [
    { "handle": "filesystem" },
    { "src": "/api/(.*)", "dest": "/api/axum" },
    { "src": "/(.*)", "dest": "/packages/temper-ui/.vercel/output" }
  ]
}
```

All `/api/*` routes go to Axum. Everything else goes to the SvelteKit output. TypeScript endpoints that currently live at `/api/ingest`, `/api/upload`, etc. would either:
- Be migrated to Axum (I5g already moves ingest to Rust), or
- Be explicitly routed before the Axum catch-all

**Option B — Subdomain separation**

- `temperkb.io` → SvelteKit
- `api.temperkb.io` → Axum

Cleaner separation but requires Vercel multi-project or custom domain configuration. Deferred as a future optimization.

**Option C — Monorepo with Vercel project per package**

Each package (`temper-ui`, `temper-cloud`) is its own Vercel project. SvelteKit calls the API via `fetch('https://api.temperkb.io/...')`. Clean but introduces CORS complexity and deployment coupling.

### 3.3 Recommendation

**Option A** for the initial implementation. The `/api/*` prefix is already the convention for all Rust and TypeScript endpoints. SvelteKit owns everything outside `/api/`. This matches how Vercel's framework detection works with SvelteKit's adapter.

The `vercel.json` evolves to:

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "buildCommand": "cd packages/temper-ui && bun run build",
  "installCommand": "bun install",
  "framework": "sveltekit",
  "outputDirectory": "packages/temper-ui/.svelte-kit",
  "routes": [
    { "src": "/api/ingest", "dest": "/api/ingest.ts" },
    { "src": "/api/upload", "dest": "/api/upload.ts" },
    { "src": "/api/(.*)", "dest": "/api/axum" },
    { "handle": "filesystem" },
    { "src": "/(.*)", "dest": "packages/temper-ui/$1" }
  ]
}
```

> **Open question**: Vercel may require a monorepo configuration with `vercel.json` at the root pointing to the SvelteKit package as the "root" framework, with Rust functions alongside. This needs a spike during implementation. The storyteller-site pattern (SvelteKit as the primary framework with other functions alongside) is the model.

### 3.4 Build Pipeline

The Vercel build must produce both:
1. The SvelteKit app (Node.js serverless functions + static assets)
2. The Rust Axum binary (compiled via `cargo build --release`)

Today, Vercel detects the Rust binary from `api/axum.rs` and the `[package]` in the root `Cargo.toml`. With SvelteKit as the framework, we may need to use Vercel's `functions` configuration or a custom build script that runs both `bun run build` (SvelteKit) and `cargo build` (Rust).

```json
{
  "functions": {
    "api/axum.rs": {
      "runtime": "vercel-rust@latest",
      "maxDuration": 30
    }
  }
}
```

---

## 4. Authentication Architecture

### 4.1 Auth0 Configuration (Existing)

From the Auth0 integration design (2026-03-30), the tenant already has:

| App | Type | Client ID | Purpose |
|-----|------|-----------|---------|
| `temper-cli` | Native | `mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF` | CLI PKCE |
| `temper-web` | SPA | `CJsNv3MerZSZKqi14eaqrK7sAy6Eg7fM` | SvelteKit web UI |

API resource:
- **Identifier (audience)**: `https://temperkb.io/api`
- **Signing algorithm**: RS256
- **Token lifetime**: 86400s (24h)

### 4.2 SvelteKit Auth Flow

For a server-rendered SvelteKit app, the recommended Auth0 integration uses the **Authorization Code Flow with PKCE** (not the SPA implicit flow), managed server-side via SvelteKit hooks.

#### Option A — `@auth0/auth0-sveltekit` SDK (if available)

Auth0 has been expanding framework-specific SDKs. If an official SvelteKit adapter exists at implementation time, use it. It would handle:
- Server-side session management via encrypted cookies
- Token acquisition and refresh
- Route protection via `handle` hooks

#### Option B — Custom server-side OIDC (Recommended)

Build a thin OIDC integration using standard libraries. This is the approach that gives us the most control and is consistent with the provider-agnostic auth story.

```
Browser                    SvelteKit Server              Auth0
  |                              |                          |
  |-- GET /auth/login ---------->|                          |
  |                              |-- 302 → /authorize ----->|
  |                              |   (PKCE, state, nonce)   |
  |<---- 302 follow redirect ----|                          |
  |                              |                          |
  |<-- Universal Login -------->|                          |
  |   (Google sign-in)          |                          |
  |                              |                          |
  |-- GET /auth/callback ------->|                          |
  |   ?code=...&state=...        |-- POST /oauth/token ---->|
  |                              |   (code + code_verifier) |
  |                              |<-- tokens ---------------|
  |                              |                          |
  |                              |-- verify id_token        |
  |                              |-- set encrypted cookie   |
  |<-- 302 → /dashboard --------|                          |
```

**Server-side session**: The SvelteKit server stores the Auth0 tokens in an encrypted HTTP-only cookie (or a server-side session store backed by Neon). The `access_token` is never exposed to client-side JavaScript. When making API calls, the SvelteKit server-side code forwards the JWT in `Authorization: Bearer` headers.

**SvelteKit hooks** (`hooks.server.ts`):
- Parse the session cookie on every request
- If the access token is expired, attempt refresh via Auth0's `/oauth/token` with the refresh token
- Populate `event.locals.user` and `event.locals.accessToken` for downstream loaders/actions
- For protected routes, redirect to `/auth/login` if no valid session

```typescript
// src/hooks.server.ts (conceptual)
import type { Handle } from '@sveltejs/kit';
import { redirect } from '@sveltejs/kit';
import { decryptSession, refreshTokens } from '$lib/server/auth';

const PROTECTED_PREFIXES = ['/dashboard', '/resources', '/teams', '/graph', '/settings'];

export const handle: Handle = async ({ event, resolve }) => {
    const session = decryptSession(event.cookies.get('temper_session'));

    if (session) {
        if (session.expiresAt < Date.now() / 1000 - 300) {
            // Refresh the access token
            const refreshed = await refreshTokens(session.refreshToken);
            if (refreshed) {
                event.locals.accessToken = refreshed.accessToken;
                event.locals.user = refreshed.user;
                // Update cookie with new tokens
            }
        } else {
            event.locals.accessToken = session.accessToken;
            event.locals.user = session.user;
        }
    }

    const isProtected = PROTECTED_PREFIXES.some(p => event.url.pathname.startsWith(p));
    if (isProtected && !event.locals.user) {
        throw redirect(302, `/auth/login?returnTo=${event.url.pathname}`);
    }

    return resolve(event);
};
```

### 4.3 Profile Resolution Consistency

The Rust API's `require_auth` middleware already handles:
1. JWT verification via JWKS
2. Email extraction (from token claims or `/userinfo` fallback)
3. Profile auto-provisioning via `profile_service::resolve_from_claims()`

When SvelteKit server-side code calls `/api/profile` with the Auth0 access token, the profile is auto-provisioned if it doesn't exist. The web UI session should call `GET /api/profile` on first login to ensure the profile exists before any other API calls.

**Auth-to-profile flow for web login:**

```
1. User completes Auth0 login → SvelteKit receives tokens
2. SvelteKit server calls GET /api/profile with access_token
   → Axum middleware auto-provisions profile if needed
   → Returns Profile { id, display_name, email, ... }
3. SvelteKit stores profile_id in session alongside tokens
4. All subsequent API calls include Authorization: Bearer header
5. All subsequent Neon-direct queries use profile_id for scoping
```

### 4.4 Auth0 Application Settings

The `temper-web` Auth0 application needs these callback URLs configured:

| Setting | Value |
|---------|-------|
| Allowed Callback URLs | `https://temperkb.io/auth/callback`, `http://localhost:5173/auth/callback` |
| Allowed Logout URLs | `https://temperkb.io`, `http://localhost:5173` |
| Allowed Web Origins | `https://temperkb.io`, `http://localhost:5173` |
| Token Endpoint Auth Method | None (PKCE) |
| Application Type | Regular Web Application |

> **Note**: Change the app type from "SPA" to "Regular Web Application" in Auth0. Server-side SvelteKit uses the Authorization Code flow, not the implicit/SPA flow. This also enables use of a `client_secret` for the token exchange, adding a layer of security. The `temper-web` client ID remains the same.

---

## 5. Data Access Strategy — API vs. Direct Neon

### 5.1 The Decision Framework

The core question: when should SvelteKit query Neon directly vs. calling the Rust API?

**Use the Rust API when:**
- The operation is a **mutation** (create, update, delete)
- The operation involves **domain logic** (access control checks, profile resolution, content reconstitution from chunks, search with embeddings)
- The operation uses **SQL functions** that encode business rules (`resources_visible_to`, `can_modify_resource`, `can_manage_team`, `contexts_visible_to`)
- **Consistency** matters — the same operation is available via CLI and should behave identically

**Query Neon directly when:**
- The query is **read-only navigation chrome** (listing contexts for a sidebar, counting resources for a dashboard badge)
- The query is **simple enough** that it doesn't need the Rust service layer
- The data is **non-sensitive** or the query correctly applies the same access boundaries
- **Performance** benefits from eliminating the API round-trip (server-side SvelteKit → Neon is a direct connection, vs. SvelteKit → Vercel function → Neon)

### 5.2 Concrete Categorization

#### Always via Rust API

| Operation | Endpoint | Rationale |
|-----------|----------|-----------|
| Profile get/update | `GET/PATCH /api/profile` | Auto-provisioning, email reconciliation |
| Auth links | `GET /api/profile/auth-links` | Identity-sensitive |
| Resource CRUD | `GET/POST/PATCH/DELETE /api/resources` | `resources_visible_to()`, `can_modify_resource()` |
| Resource content | `GET /api/resources/{id}/content` | Chunk reconstitution logic |
| Search | `POST /api/search` | Embedding vector handling, visibility scoping |
| Ingest | `POST /api/ingest` | Extract → chunk → embed pipeline |
| Context CRUD | `GET/POST /api/contexts` | Ownership, visibility scoping |
| Events | `GET /api/events` | Profile-scoped |
| Team management | (future) `POST/PATCH/DELETE /api/teams/*` | `can_manage_team()` |
| Transfers | (future) `POST /api/transfers/*` | Ownership transfer logic |
| Invitations | (future) `POST /api/invitations/*` | Token generation, expiry |
| Graph traversal | (future) `POST /api/graph/traverse` | `resources_visible_to()` + recursive CTEs |

#### Acceptable via Direct Neon

| Query | SQL Pattern | Rationale |
|-------|-------------|-----------|
| Context list for sidebar | `SELECT * FROM contexts_visible_to($1)` | Uses the same SQL function; read-only; populates nav chrome |
| Doc-type enumeration | `SELECT id, name FROM kb_doc_types ORDER BY name` | Public taxonomy; no access control needed |
| Resource count per context | `SELECT kb_context_id, count(*) FROM kb_resources WHERE owner_profile_id = $1 AND is_active GROUP BY 1` | Dashboard badge; owner-scoped |
| Team list for user | `SELECT t.* FROM kb_teams t JOIN kb_team_members tm ON ... WHERE tm.profile_id = $1 AND t.is_active` | Read-only; member-scoped |
| Recent activity summary | `SELECT event_type, count(*) FROM kb_events WHERE profile_id = $1 AND created > now() - interval '7 days' GROUP BY 1` | Dashboard widget; profile-scoped |

> **Critical constraint**: Every direct Neon query from SvelteKit MUST scope by `profile_id` from the authenticated session. The `profile_id` is obtained from the Rust API during login (see §4.3) and stored in the encrypted session cookie.

### 5.3 SvelteKit API Proxy Helper

For routes that call the Rust API, create a typed proxy helper:

```typescript
// src/lib/server/api.ts
import { API_BASE_URL } from '$env/static/private';

export async function apiGet<T>(path: string, accessToken: string): Promise<T> {
    const res = await fetch(`${API_BASE_URL}${path}`, {
        headers: { Authorization: `Bearer ${accessToken}` }
    });
    if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new ApiError(res.status, body.error?.message ?? `HTTP ${res.status}`);
    }
    return res.json() as Promise<T>;
}

export async function apiPost<T>(path: string, accessToken: string, body: unknown): Promise<T> {
    const res = await fetch(`${API_BASE_URL}${path}`, {
        method: 'POST',
        headers: {
            Authorization: `Bearer ${accessToken}`,
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(body)
    });
    if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new ApiError(res.status, body.error?.message ?? `HTTP ${res.status}`);
    }
    return res.json() as Promise<T>;
}
```

For same-origin deployment (SvelteKit and Axum on the same Vercel project), `API_BASE_URL` can be empty string or the Vercel project URL. For local development, it points to the local Axum server (`http://localhost:3000`).

### 5.4 Direct Neon Connection

Following the storyteller-site pattern:

```typescript
// src/lib/server/db.ts
import postgres from 'postgres';
import { DATABASE_URL } from '$env/static/private';

export const sql = postgres(DATABASE_URL, {
    max: 10,
    idle_timeout: 20,
    connect_timeout: 10
});
```

Alternatively, use `@neondatabase/serverless` for better Vercel Edge compatibility:

```typescript
// src/lib/server/db.ts
import { neon } from '@neondatabase/serverless';
import { DATABASE_URL } from '$env/static/private';

export const sql = neon(DATABASE_URL);
```

The `postgres` (postgres.js) package is preferred for its tagged template ergonomics and connection pooling, consistent with storyteller-site.

---

## 6. New API Endpoints Required

The current Rust API surface is minimal. The UI needs additional endpoints to support its features. These should be added to `temper-api` as part of the UI implementation.

### 6.1 Teams API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `GET` | `/api/teams` | `handlers::teams::list` | List teams the user is a member of |
| `POST` | `/api/teams` | `handlers::teams::create` | Create a new team |
| `GET` | `/api/teams/{id}` | `handlers::teams::get` | Get team details |
| `PATCH` | `/api/teams/{id}` | `handlers::teams::update` | Update team name/description |
| `DELETE` | `/api/teams/{id}` | `handlers::teams::delete` | Soft-delete a team |
| `GET` | `/api/teams/{id}/members` | `handlers::teams::list_members` | List team members with roles |
| `POST` | `/api/teams/{id}/members` | `handlers::teams::add_member` | Add member (by email or profile_id) |
| `PATCH` | `/api/teams/{id}/members/{pid}` | `handlers::teams::update_member` | Change role |
| `DELETE` | `/api/teams/{id}/members/{pid}` | `handlers::teams::remove_member` | Remove from team |
| `GET` | `/api/teams/{id}/resources` | `handlers::teams::list_resources` | Resources shared with team |
| `POST` | `/api/teams/{id}/resources` | `handlers::teams::share_resource` | Share a resource |
| `DELETE` | `/api/teams/{id}/resources/{rid}` | `handlers::teams::unshare_resource` | Revoke sharing |

### 6.2 Invitations API

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/teams/{id}/invitations` | Send invitation (email, role) |
| `GET` | `/api/invitations` | List pending invitations for current user (by email) |
| `POST` | `/api/invitations/{id}/accept` | Accept invitation |
| `POST` | `/api/invitations/{id}/decline` | Decline invitation |

### 6.3 Transfers API

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/transfers` | Initiate ownership transfer |
| `GET` | `/api/transfers` | List pending transfers (incoming/outgoing) |
| `POST` | `/api/transfers/{id}/accept` | Accept transfer |
| `POST` | `/api/transfers/{id}/decline` | Decline transfer |
| `POST` | `/api/transfers/{id}/cancel` | Cancel outgoing transfer |

### 6.4 Graph Traversal API (R7 dependent)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/graph/neighbors` | Get N-hop neighbors of a resource |
| `POST` | `/api/graph/traverse` | Multi-hop traversal with typed edge filtering |
| `POST` | `/api/graph/subgraph` | Extract a subgraph rooted at a resource |
| `GET` | `/api/resources/{id}/edges` | List edges for a resource |
| `POST` | `/api/resources/{id}/edges` | Create an edge (manual linking) |
| `DELETE` | `/api/resources/{id}/edges/{eid}` | Remove an edge |

### 6.5 Dashboard / Aggregation API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/dashboard/stats` | Resource counts, team counts, recent activity |
| `GET` | `/api/dashboard/activity` | Recent events timeline |

---

## 7. Page Architecture

### 7.1 Route Map

```
src/routes/
├── +layout.svelte                    # Root layout: nav, footer, CSS
├── +layout.server.ts                 # Session resolution
├── +page.svelte                      # Landing page (public)
├── +error.svelte                     # Error boundary
│
├── docs/                             # Documentation (public)
│   ├── +layout.svelte                # Docs sidebar layout
│   ├── +page.svelte                  # Docs index
│   ├── getting-started/
│   │   └── +page.svelte              # Installation, first vault, first search
│   ├── cli-reference/
│   │   └── +page.svelte              # Command reference
│   ├── concepts/
│   │   ├── +page.svelte              # Concepts overview
│   │   ├── resources/+page.svelte    # Two-tier resource model
│   │   ├── contexts/+page.svelte     # Context scoping
│   │   ├── doc-types/+page.svelte    # Doc types and behaviors
│   │   ├── search/+page.svelte       # Semantic search
│   │   └── sync/+page.svelte         # Sync protocol
│   ├── workflows/
│   │   ├── +page.svelte              # Workflow overview
│   │   ├── tasks/+page.svelte        # Task lifecycle
│   │   ├── sessions/+page.svelte     # Session-based development
│   │   └── research/+page.svelte     # Research workflow
│   ├── teams/
│   │   └── +page.svelte              # Team collaboration docs
│   ├── self-hosting/
│   │   └── +page.svelte              # Enterprise / self-hosted guide
│   └── api-reference/
│       └── +page.svelte              # OpenAPI reference (embedded Swagger)
│
├── auth/                             # Auth routes (public)
│   ├── login/+server.ts              # Redirect to Auth0 /authorize
│   ├── callback/+server.ts           # Handle Auth0 callback, set session
│   └── logout/+server.ts             # Clear session, redirect to Auth0 /logout
│
├── (app)/                            # Authenticated layout group
│   ├── +layout.svelte                # App chrome: sidebar, breadcrumbs
│   ├── +layout.server.ts             # Load user profile, contexts, teams
│   │
│   ├── dashboard/
│   │   ├── +page.svelte              # Overview: stats, recent activity, quick actions
│   │   └── +page.server.ts           # Aggregate stats from API + Neon
│   │
│   ├── resources/
│   │   ├── +page.svelte              # Resource list with filters
│   │   ├── +page.server.ts           # GET /api/resources with query params
│   │   └── [id]/
│   │       ├── +page.svelte          # Resource detail: metadata, content, edges
│   │       ├── +page.server.ts       # GET /api/resources/{id} + /content + /edges
│   │       └── edit/
│   │           ├── +page.svelte      # Edit resource metadata
│   │           └── +page.server.ts
│   │
│   ├── contexts/
│   │   ├── +page.svelte              # Context list
│   │   ├── +page.server.ts
│   │   └── [id]/
│   │       ├── +page.svelte          # Context detail: resources, doc types
│   │       └── +page.server.ts
│   │
│   ├── search/
│   │   ├── +page.svelte              # Full search interface
│   │   └── +page.server.ts           # POST /api/search (with client-side embedding)
│   │
│   ├── graph/
│   │   ├── +page.svelte              # Knowledge graph explorer (d3.js)
│   │   └── +page.server.ts           # POST /api/graph/subgraph
│   │
│   ├── teams/
│   │   ├── +page.svelte              # Team list
│   │   ├── +page.server.ts
│   │   ├── new/
│   │   │   └── +page.svelte          # Create team form
│   │   └── [id]/
│   │       ├── +page.svelte          # Team detail: members, resources, settings
│   │       ├── +page.server.ts
│   │       ├── members/
│   │       │   └── +page.svelte      # Member management
│   │       ├── resources/
│   │       │   └── +page.svelte      # Shared resource management
│   │       └── settings/
│   │           └── +page.svelte      # Team settings, danger zone
│   │
│   ├── invitations/
│   │   ├── +page.svelte              # Pending invitations
│   │   └── +page.server.ts
│   │
│   ├── transfers/
│   │   ├── +page.svelte              # Pending transfers
│   │   └── +page.server.ts
│   │
│   └── settings/
│       ├── +page.svelte              # User settings: profile, preferences
│       ├── +page.server.ts
│       └── auth-links/
│           └── +page.svelte          # Linked auth providers
```

### 7.2 Layout Groups

SvelteKit's parenthesized layout groups let us share layout logic without affecting URL paths:

- **Root layout** (`+layout.svelte`): Minimal — global styles, meta tags
- **Public pages** (landing, docs): Use the root layout directly with marketing nav
- **`(app)` group**: Authenticated layout with sidebar nav, user menu, breadcrumbs. The `+layout.server.ts` here loads user profile, context list, and team list for the sidebar.

### 7.3 Public Pages Detail

#### Landing Page (`/`)

The landing page serves as the primary marketing surface for temper. Structure:

1. **Hero section**: "Your knowledge base, with structure" — tagline, value prop, CTA buttons (Get Started, View on GitHub)
2. **What is temper**: Brief description — CLI-first knowledge base with semantic search, frontmatter-driven structure, and cloud sync
3. **How it works**: Visual workflow showing `temper init` → `temper add`/`temper import` → `temper search` → `temper sync`
4. **Key concepts**: Cards for Contexts, Doc Types, Resources, Search, Sync
5. **Architecture diagram**: SVG/image showing CLI ↔ Rust API ↔ Postgres ↔ pgvector, with MCP and Web UI as consumers
6. **For teams**: Brief section on team features — sharing, permissions, collaboration
7. **Self-host or use temperkb.io**: Two-track pitch — managed service vs. enterprise self-hosting
8. **Footer**: GitHub link, docs link, license (MIT)

#### Documentation Pages (`/docs/*`)

Documentation is authored as Svelte components (not fetched from the KB), allowing us to include interactive examples, code blocks with syntax highlighting, and diagrams. A docs sidebar layout provides navigation.

Key documentation pages:

- **Getting Started**: Installation via `cargo install`, first vault, first search, Auth0 login
- **CLI Reference**: Full command reference (adapted from current README)
- **Concepts**: Two-tier resource model, contexts, doc types, behaviors, kb:// URI scheme
- **Workflows**: Task lifecycle, session-based development, research workflow, adaptive scope
- **Teams**: Creating teams, inviting members, sharing resources, access levels
- **Self-Hosting**: Docker Compose setup, Postgres requirements (pgvector), environment variables, Auth0/Keycloak/Okta configuration, Vercel vs. bare metal deployment
- **API Reference**: Embedded Swagger UI (already available at `/api-docs/ui` when `ENABLE_SWAGGER=true`)

#### Self-Hosting Guide (`/docs/self-hosting`)

This is a critical page for enterprise adoption. It should cover:

1. **Infrastructure requirements**: Postgres 15+ with pgvector 0.5+, any OAuth2/OIDC provider, Node.js 22+ (for Vercel) or direct Rust binary
2. **Docker Compose quick-start**: Pre-built images for temper-api + Postgres
3. **Auth provider configuration**: Step-by-step for Auth0, Keycloak, Okta, Azure AD — showing how `config.toml` and environment variables map to each provider
4. **Environment variables reference**: Full table of `DATABASE_URL`, `JWKS_URL`, `AUTH_ISSUER`, `AUTH_AUDIENCE`, etc.
5. **Neon vs. self-managed Postgres**: Tradeoffs, pgvector installation, connection pooling
6. **Deployment targets**: Vercel (recommended), Railway, Fly.io, bare EC2/VPS
7. **Migration guide**: Running `sqlx migrate run` against your database

---

## 8. Authenticated App Pages Detail

### 8.1 Dashboard (`/dashboard`)

The authenticated landing page. Shows at a glance:

- **Stats cards**: Total resources, contexts, teams, pending invitations/transfers
- **Recent activity**: Timeline of the last 20 events (from `GET /api/events`)
- **Quick actions**: "Add Resource", "Create Context", "Create Team", "Search"
- **Context overview**: Cards for each context showing resource count and last updated

Data sources:
- Stats → `GET /api/dashboard/stats` (API) or direct Neon aggregation query
- Activity → `GET /api/events?limit=20` (API)
- Contexts → `contexts_visible_to()` via Neon (sidebar data already loaded in layout)

### 8.2 Resources (`/resources`)

Paginated, filterable list of resources visible to the user.

**Filters**: Context, doc type, resource mode (added/imported), search text (title match)
**Sort**: Updated (default), created, title
**Actions**: View, edit metadata, delete (with confirmation)

The list page calls `GET /api/resources?kb_context_id=...&limit=50&offset=0` from `+page.server.ts`.

#### Resource Detail (`/resources/[id]`)

Full view of a single resource:

- **Metadata panel**: Title, context, doc type, origin URI, kb:// URI, owner, created/updated timestamps, resource mode
- **Content panel**: Reconstituted markdown rendered as HTML (from `GET /api/resources/{id}/content`). Use a markdown renderer like `marked` or `mdsvex`.
- **Edges panel** (R7): List of graph edges (relates_to, extends, depends_on, etc.) with links to connected resources
- **Sharing panel**: Which teams this resource is shared with, at what access level. Actions to share/unshare.
- **Transfer panel**: Initiate ownership transfer to another user
- **Danger zone**: Delete resource (soft-delete)

### 8.3 Search (`/search`)

The search page needs to handle the embedding generation challenge. The Rust API's `POST /api/search` expects a 768-dimensional embedding vector. Options:

**Option A — Server-side embedding in SvelteKit**
Run the embedding model (kreuzberg balanced / bge-base-en-v1.5) in the SvelteKit server function. This requires `onnxruntime-node` or `@huggingface/transformers` as a dependency.

**Option B — Add a text-search endpoint to the Rust API (Recommended)**
Add `POST /api/search/text` that accepts `{ query: string, context_name?, doc_type?, limit? }` and handles embedding generation server-side in Rust (via `temper-embed` or the existing TypeScript pipeline). This is the cleanest separation — the UI never needs to know about embeddings.

**Option C — Client-side embedding via WASM**
Run the model in the browser via ONNX WASM runtime. Heavy download (~30MB model), slow first inference. Not recommended for primary search.

**Recommendation**: Option B. The text-search endpoint also benefits the CLI (`temper search "query"` currently does client-side embedding; a server-side endpoint would simplify the CLI too).

#### Search UI Components

- **Search bar**: Text input with context and doc-type filter dropdowns
- **Results list**: Cards showing title, context, doc type, snippet (with match highlighting), similarity score
- **Result detail drawer**: Click a result to see full chunk content and link to resource detail page

### 8.4 Knowledge Graph Explorer (`/graph`)

This is the showcase feature — a d3.js force-directed graph visualization of the user's knowledge base.

**Data source**: `POST /api/graph/subgraph` returns nodes (resources) and edges with types and weights.

**Visualization approach**:

```typescript
// d3.js force simulation
const simulation = d3.forceSimulation(nodes)
    .force('link', d3.forceLink(edges).id(d => d.id).distance(d => 100 / d.weight))
    .force('charge', d3.forceManyBody().strength(-200))
    .force('center', d3.forceCenter(width / 2, height / 2))
    .force('collision', d3.forceCollide().radius(30));
```

**Node styling**:
- Color by context (consistent palette)
- Shape by doc type (circle for task, square for research, diamond for session, etc.)
- Size by edge count (more connected = larger)
- Hover shows resource title and metadata tooltip

**Edge styling**:
- Color/dash pattern by edge type (solid for `depends_on`, dashed for `relates_to`, dotted for `references`)
- Thickness by weight
- Arrow direction for directional edges

**Interactions**:
- Click node → navigate to `/resources/[id]`
- Right-click node → context menu (view, edit, traverse from here)
- Double-click node → expand (load neighbors and add to graph)
- Drag to reposition
- Scroll to zoom
- Filter panel: toggle edge types, filter by context/doc-type, set max depth

**Progressive loading**: Start with the user's most-connected resources (top-N by edge count), allow "expand" to load neighbors on demand. Never load the entire graph at once.

### 8.5 Teams (`/teams`)

Team management pages:

- **Team list**: Cards for each team showing name, member count, role badge
- **Team detail**: Members table (avatar, name, email, role, joined date), shared resources table, settings
- **Member management**: Invite by email (sends Auth0 invitation), change roles (owner/maintainer only), remove members
- **Resource sharing**: Browse your resources, select one, choose access level (vault/mutable/immutable), share. List shared resources with unshare action.
- **Settings**: Rename team, update description, transfer ownership, delete team (owner only)

### 8.6 Invitations & Transfers (`/invitations`, `/transfers`)

Simple list + action pages:

- **Invitations**: Pending invitations received (by email match). Accept or decline. Shows team name, invited by, role offered, expiry.
- **Transfers**: Incoming and outgoing transfers. Accept/decline incoming. Cancel outgoing. Shows resource title, from/to user, status.

### 8.7 Settings (`/settings`)

User profile management:

- **Profile**: Display name (editable), email (read-only from auth), avatar
- **Preferences**: JSON editor or structured form for user preferences
- **Auth links**: List of linked auth providers. Shows provider name, email, linked date. (Future: link additional providers)
- **Vault config**: Manage cloud vault configuration

---

## 9. Component Architecture

### 9.1 Shared Components

```
src/lib/components/
├── ui/                    # Generic UI primitives
│   ├── Button.svelte
│   ├── Input.svelte
│   ├── Select.svelte
│   ├── Modal.svelte
│   ├── Toast.svelte
│   ├── Card.svelte
│   ├── Badge.svelte
│   ├── Table.svelte
│   ├── Pagination.svelte
│   ├── Breadcrumbs.svelte
│   ├── Spinner.svelte
│   └── EmptyState.svelte
│
├── layout/                # Layout components
│   ├── AppSidebar.svelte
│   ├── AppHeader.svelte
│   ├── UserMenu.svelte
│   ├── DocsNav.svelte
│   └── Footer.svelte
│
├── resources/             # Resource-specific
│   ├── ResourceCard.svelte
│   ├── ResourceList.svelte
│   ├── ResourceDetail.svelte
│   ├── ResourceFilters.svelte
│   ├── MarkdownRenderer.svelte
│   └── EdgeList.svelte
│
├── search/                # Search-specific
│   ├── SearchBar.svelte
│   ├── SearchResults.svelte
│   └── SearchResultCard.svelte
│
├── graph/                 # Knowledge graph
│   ├── GraphCanvas.svelte
│   ├── GraphControls.svelte
│   ├── GraphTooltip.svelte
│   └── GraphLegend.svelte
│
├── teams/                 # Team management
│   ├── TeamCard.svelte
│   ├── MemberTable.svelte
│   ├── InviteForm.svelte
│   ├── ShareResourceModal.svelte
│   └── RoleBadge.svelte
│
└── landing/               # Public marketing
    ├── Hero.svelte
    ├── FeatureCard.svelte
    ├── WorkflowDiagram.svelte
    └── ArchitectureDiagram.svelte
```

### 9.2 Svelte 5 Patterns

All components use Svelte 5 runes mode (consistent with storyteller-site):

```svelte
<script lang="ts">
    import type { Resource } from '$lib/types';

    let { resource, onDelete }: {
        resource: Resource;
        onDelete: (id: string) => void;
    } = $props();

    let isExpanded = $state(false);
    let formattedDate = $derived(
        new Intl.DateTimeFormat('en-US').format(new Date(resource.updated))
    );
</script>
```

### 9.3 Type Definitions

```typescript
// src/lib/types.ts — mirrors temper-core Rust types

export interface Profile {
    id: string;
    display_name: string;
    email: string | null;
    avatar_url: string | null;
    preferences: Record<string, unknown>;
    vault_config: Record<string, unknown>;
    is_active: boolean;
    created: string;
    updated: string;
}

export interface Context {
    id: string;
    name: string;
    kb_owner_table: string;
    kb_owner_id: string;
    created: string;
    updated: string;
}

export interface Resource {
    id: string;
    kb_context_id: string;
    kb_doc_type_id: string;
    origin_uri: string;
    title: string;
    slug: string | null;
    content_hash: string | null;
    mimetype: string | null;
    originator_profile_id: string;
    owner_profile_id: string;
    is_active: boolean;
    created: string;
    updated: string;
}

export interface SearchResult {
    resource_id: string;
    title: string;
    kb_uri: string;
    origin_uri: string;
    context: string;
    doc_type: string;
    snippet: string;
    header_path: string;
    score: number;
}

export interface Team {
    id: string;
    name: string;
    slug: string;
    description: string | null;
    metadata: Record<string, unknown>;
    created_by_profile_id: string;
    is_active: boolean;
    created: string;
    updated: string;
}

export interface TeamMember {
    id: string;
    team_id: string;
    profile_id: string;
    role: 'owner' | 'maintainer' | 'member' | 'watcher';
    joined_at: string;
    invited_by_profile_id: string | null;
    // Denormalized from profile join
    display_name?: string;
    email?: string;
    avatar_url?: string;
}

export interface GraphNode {
    id: string;
    title: string;
    context: string;
    doc_type: string;
    edge_count: number;
}

export interface GraphEdge {
    id: string;
    source_id: string;
    target_id: string;
    edge_type: string;
    weight: number;
    metadata: Record<string, unknown>;
}

export interface TeamInvitation {
    id: string;
    team_id: string;
    team_name: string;
    invited_email: string;
    invited_by_profile_id: string;
    invited_by_name: string;
    role: 'owner' | 'maintainer' | 'member' | 'watcher';
    status: 'pending' | 'accepted' | 'declined' | 'expired';
    expires_at: string;
    created: string;
}

export interface ResourceTransfer {
    id: string;
    resource_id: string;
    resource_title: string;
    from_profile_id: string;
    from_display_name: string;
    to_profile_id: string;
    to_display_name: string;
    status: 'pending' | 'accepted' | 'declined' | 'cancelled';
    created: string;
    resolved_at: string | null;
}
```

---

## 10. Styling Strategy

### 10.1 Tailwind CSS

Unlike storyteller-site (which uses hand-crafted CSS custom properties for a bespoke aesthetic), temper-ui benefits from Tailwind for:

- Faster iteration on a larger page count (~30+ pages)
- Consistent spacing, typography, and color systems
- Dark mode support via `dark:` variants
- Responsive design utilities

### 10.2 Design Tokens

```css
/* tailwind.config.ts theme extension */
{
    colors: {
        temper: {
            50:  '#f0f7ff',
            100: '#e0effe',
            200: '#bae0fd',
            300: '#7ccbfc',
            400: '#36b2f8',
            500: '#0c99e9',  /* primary */
            600: '#0079c7',
            700: '#0060a1',
            800: '#045185',
            900: '#09446e',
            950: '#062b49',
        },
        ink:    '#1a1a2e',
        chalk:  '#f8f9fa',
    },
    fontFamily: {
        sans: ['Inter', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
    }
}
```

### 10.3 Dark Mode

The app should support dark mode from day one. Tailwind's `darkMode: 'class'` strategy with a user preference toggle stored in `localStorage` (and synced to the profile `preferences` JSON for cross-device persistence).

---

## 11. Search Architecture — Embedding Challenge

### 11.1 The Problem

The current `POST /api/search` endpoint requires the caller to provide a 768-dimensional embedding vector. This works for the CLI (which uses `temper-embed` or calls the embedding locally) but is impractical for a web UI where users type text queries.

### 11.2 Proposed Solution: Text Search Endpoint

Add a new endpoint to `temper-api`:

```
POST /api/search/text
Content-Type: application/json
Authorization: Bearer <token>

{
    "query": "error handling patterns in authentication",
    "context_name": "temper",
    "doc_type": "research",
    "limit": 10
}
```

The Axum handler:
1. Receives the text query
2. Generates the embedding server-side (via `temper-embed` crate or an HTTP call to a model endpoint)
3. Calls the existing `search_service::search()` with the generated embedding
4. Returns the same `SearchResultRow` response

**Embedding generation options for the server:**

| Option | Pros | Cons |
|--------|------|------|
| `temper-embed` crate (ONNX in Rust) | No network calls, fast, consistent with CLI | Binary size, ONNX runtime in serverless |
| HTTP call to HuggingFace Inference API | No binary dependency | Latency, rate limits, API key |
| HTTP call to a self-hosted model endpoint | Full control | Infrastructure to manage |
| Pre-compute in TypeScript (existing `@huggingface/transformers`) | Already deployed | Cross-runtime complexity |

**Recommendation**: Start with the HuggingFace Inference API for the text-search endpoint (simplest to deploy), with a plan to migrate to `temper-embed` in the Axum binary once binary size and cold start are validated.

### 11.3 Hybrid Search (Future)

With the R7 knowledge graph in place, search can combine:
1. **Vector similarity** (semantic match)
2. **Graph proximity** (resources connected to high-scoring results)
3. **Full-text search** (Postgres `tsvector` for exact keyword matching)

The `POST /api/search/text` endpoint can evolve to support a `mode` parameter:

```json
{
    "query": "authentication patterns",
    "mode": "hybrid",    // "semantic" | "graph" | "fulltext" | "hybrid"
    "context_name": "temper",
    "limit": 10
}
```

---

## 12. Knowledge Graph Visualization (R7 Integration)

### 12.1 Data Model Recap

From R7, the graph surface consists of:

- **Nodes**: `kb_resources` (any resource is a vertex)
- **Edges**: `kb_resource_edges` (typed, weighted, directional)
- **Edge types**: `relates_to`, `extends`, `depends_on`, `references`, `parent_of`, `tagged_with`, `preceded_by`, `derived_from`

### 12.2 Visualization Component

The `GraphCanvas.svelte` component uses d3.js v7's force simulation:

```svelte
<script lang="ts">
    import * as d3 from 'd3';
    import type { GraphNode, GraphEdge } from '$lib/types';

    let { nodes, edges, onNodeClick }: {
        nodes: GraphNode[];
        edges: GraphEdge[];
        onNodeClick: (id: string) => void;
    } = $props();

    let svgElement: SVGSVGElement;
    let width = $state(800);
    let height = $state(600);

    $effect(() => {
        if (!svgElement || nodes.length === 0) return;
        renderGraph(svgElement, nodes, edges, width, height, onNodeClick);
    });
</script>

<div class="graph-container" bind:clientWidth={width} bind:clientHeight={height}>
    <svg bind:this={svgElement} {width} {height}></svg>
</div>
```

### 12.3 Graph Interaction Patterns

| Interaction | Behavior |
|-------------|----------|
| Page load | Load user's top-20 most-connected resources as seed nodes |
| Click node | Navigate to resource detail page |
| Double-click node | Expand: fetch 1-hop neighbors, add to graph |
| Shift+click edge | Show edge metadata tooltip |
| Right-click node | Context menu: expand, collapse, hide, open in new tab |
| Filter panel | Toggle edge types, filter by context/doc-type, slider for max depth |
| Search integration | Search results highlight matching nodes in the graph |
| Minimap | Small overview showing current viewport position |

### 12.4 Performance Considerations

- **Max nodes**: Cap at ~500 nodes in the viewport. Beyond that, use level-of-detail: collapse clusters into single super-nodes.
- **WebGL fallback**: For very large graphs (1000+ nodes), consider `d3-force` with a Canvas renderer instead of SVG, or a WebGL library like `sigma.js`.
- **Server-side filtering**: The API should support `max_depth` and `max_nodes` parameters to limit the response size.
- **Incremental loading**: Use `expand` semantics (load neighbors on demand) rather than loading the full graph.

---

## 13. Package Structure

### 13.1 Directory Layout

```
packages/temper-ui/
├── src/
│   ├── app.d.ts               # SvelteKit type declarations
│   ├── app.html               # HTML shell
│   ├── hooks.server.ts        # Auth middleware
│   ├── lib/
│   │   ├── server/
│   │   │   ├── db.ts          # Neon connection (postgres.js)
│   │   │   ├── auth.ts        # Session encrypt/decrypt, token refresh
│   │   │   └── api.ts         # Typed API proxy (apiGet, apiPost, etc.)
│   │   ├── types.ts           # TypeScript type definitions
│   │   ├── stores.ts          # Svelte stores (user, theme, etc.)
│   │   ├── utils.ts           # Formatting, date helpers
│   │   ├── components/        # (see §9.1)
│   │   ├── styles/
│   │   │   └── app.css        # Tailwind directives + custom styles
│   │   └── assets/
│   │       ├── favicon.svg
│   │       ├── logo.svg
│   │       └── images/        # Landing page illustrations
│   └── routes/                # (see §7.1)
├── static/
│   ├── robots.txt
│   ├── sitemap.xml
│   └── og-image.png           # OpenGraph preview image
├── svelte.config.js
├── vite.config.ts
├── tailwind.config.ts
├── postcss.config.js
├── tsconfig.json
├── package.json
└── README.md
```

### 13.2 Dependencies

```json
{
    "name": "@temper/ui",
    "private": true,
    "version": "0.1.0",
    "type": "module",
    "scripts": {
        "dev": "vite dev",
        "build": "vite build",
        "preview": "vite preview",
        "check": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json"
    },
    "devDependencies": {
        "@sveltejs/adapter-vercel": "^5",
        "@sveltejs/kit": "^2",
        "@sveltejs/vite-plugin-svelte": "^6",
        "@tailwindcss/typography": "^0.5",
        "@types/d3": "^7",
        "autoprefixer": "^10",
        "postcss": "^8",
        "svelte": "^5",
        "svelte-check": "^4",
        "tailwindcss": "^4",
        "typescript": "^5",
        "vite": "^7"
    },
    "dependencies": {
        "d3": "^7",
        "jose": "^6",
        "marked": "^15",
        "postgres": "^3"
    }
}
```

### 13.3 SvelteKit Config

```javascript
// svelte.config.js
import adapter from '@sveltejs/adapter-vercel';
import { relative, sep } from 'node:path';

/** @type {import('@sveltejs/kit').Config} */
const config = {
    compilerOptions: {
        runes: ({ filename }) => {
            const relativePath = relative(import.meta.dirname, filename);
            const pathSegments = relativePath.toLowerCase().split(sep);
            const isExternalLibrary = pathSegments.includes('node_modules');
            return isExternalLibrary ? undefined : true;
        }
    },
    kit: {
        adapter: adapter({
            runtime: 'nodejs22.x'
        }),
        alias: {
            '$components': 'src/lib/components'
        }
    }
};

export default config;
```

---

## 14. Environment Variables

### 14.1 SvelteKit Server-Side (Private)

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | Neon connection string | `postgresql://user:pass@host/db?sslmode=require` |
| `AUTH0_DOMAIN` | Auth0 tenant domain | `temperkb.us.auth0.com` |
| `AUTH0_CLIENT_ID` | temper-web app client ID | `CJsNv3MerZSZKqi14eaqrK7sAy6Eg7fM` |
| `AUTH0_CLIENT_SECRET` | temper-web app secret (Regular Web App) | `...` |
| `AUTH0_AUDIENCE` | API identifier | `https://temperkb.io/api` |
| `SESSION_SECRET` | 32-byte secret for cookie encryption | `...` |
| `API_BASE_URL` | Rust API base (same origin for prod) | `` (empty for same-origin) or `http://localhost:3000` |

### 14.2 SvelteKit Client-Side (Public)

| Variable | Description | Example |
|----------|-------------|---------|
| `PUBLIC_APP_URL` | Application URL | `https://temperkb.io` |
| `PUBLIC_GITHUB_URL` | GitHub repo URL | `https://github.com/tasker-systems/temper` |

### 14.3 Shared with Rust API (Already Configured)

| Variable | Current Value |
|----------|---------------|
| `JWKS_URL` | `https://temperkb.us.auth0.com/.well-known/jwks.json` |
| `AUTH_ISSUER` | `https://temperkb.us.auth0.com/` |
| `AUTH_AUDIENCE` | `https://temperkb.io/api` |
| `CORS_ORIGINS` | Must include `https://temperkb.io` |

---

## 15. Local Development

### 15.1 Dev Server Setup

```bash
# Terminal 1: Rust API (local)
cd crates/temper-api
cargo run
# Listens on http://localhost:3000

# Terminal 2: SvelteKit dev server
cd packages/temper-ui
bun run dev
# Listens on http://localhost:5173
# API_BASE_URL=http://localhost:3000
```

### 15.2 Database

Local development uses the same Neon database (or a Neon branch for isolation). The `DATABASE_URL` in `.env.local` for the SvelteKit package points to Neon.

For fully offline development, a local Docker Postgres with pgvector can be used (from the existing `docker-compose.yml`).

### 15.3 Auth0 Callbacks

The `temper-web` Auth0 application has `http://localhost:5173/auth/callback` as an allowed callback URL, enabling local development with real Auth0 login.

---

## 16. Implementation Phases

### Phase 1 — Foundation (1-2 weeks)

**Goal**: SvelteKit project scaffold, Auth0 integration, Vercel deployment working alongside Rust API.

- [ ] Create `packages/temper-ui` with SvelteKit scaffold
- [ ] Configure `adapter-vercel`, Tailwind CSS, TypeScript
- [ ] Implement Auth0 server-side flow (`/auth/login`, `/auth/callback`, `/auth/logout`)
- [ ] Implement `hooks.server.ts` with session management and route protection
- [ ] Create `$lib/server/db.ts` (Neon connection) and `$lib/server/api.ts` (API proxy)
- [ ] Build root layout with public nav and `(app)` layout group with authenticated sidebar
- [ ] Update `vercel.json` routing to support both SvelteKit and Axum
- [ ] Deploy to Vercel and verify SvelteKit + Axum coexistence
- [ ] Create TypeScript types mirroring Rust domain types

### Phase 2 — Public Pages (1-2 weeks)

**Goal**: Landing page and documentation live on temperkb.io.

- [ ] Landing page with hero, feature cards, architecture diagram, CTAs
- [ ] Docs layout with sidebar navigation
- [ ] Getting Started guide
- [ ] CLI Reference (adapted from README)
- [ ] Concepts documentation (resources, contexts, doc types, search, sync)
- [ ] Workflow documentation (tasks, sessions, research)
- [ ] Self-Hosting guide
- [ ] API Reference page (embedded Swagger or static OpenAPI rendering)
- [ ] SEO meta tags, Open Graph images, sitemap

### Phase 3 — Core App (2-3 weeks)

**Goal**: Authenticated users can browse, search, and manage their knowledge base.

- [ ] Dashboard page with stats and activity
- [ ] Resource list with filters and pagination
- [ ] Resource detail with markdown rendering and metadata
- [ ] Context list and detail pages
- [ ] `POST /api/search/text` endpoint in Rust API
- [ ] Search page with text input and results
- [ ] Settings page (profile, preferences, auth links)
- [ ] Toast notifications and error handling

### Phase 4 — Teams & Permissions (1-2 weeks)

**Goal**: Full team management, sharing, invitations, and transfers.

- [ ] Teams API endpoints in Rust (§6.1)
- [ ] Invitations API endpoints (§6.2)
- [ ] Transfers API endpoints (§6.3)
- [ ] Team list, detail, creation pages
- [ ] Member management UI
- [ ] Resource sharing modal
- [ ] Invitations page (receive, accept, decline)
- [ ] Transfers page (initiate, accept, decline, cancel)

### Phase 5 — Knowledge Graph (2-3 weeks, R7 dependent)

**Goal**: Interactive graph visualization of the knowledge base.

- [ ] Graph traversal API endpoints in Rust (§6.4)
- [ ] `GraphCanvas.svelte` with d3.js force simulation
- [ ] Graph controls: zoom, pan, filter, expand
- [ ] Graph tooltips and context menus
- [ ] Node styling by context and doc type
- [ ] Edge styling by type and weight
- [ ] Progressive loading with expand-on-demand
- [ ] Search → graph integration (highlight results)

### Phase 6 — Polish & Performance (1 week)

**Goal**: Production-ready quality.

- [ ] Dark mode toggle with preference persistence
- [ ] Responsive design (mobile-friendly app layout)
- [ ] Loading states, skeleton screens
- [ ] Error boundaries and friendly error pages
- [ ] Accessibility audit (ARIA labels, keyboard navigation, contrast)
- [ ] Performance audit (Core Web Vitals, lazy loading, code splitting)
- [ ] Analytics integration (privacy-respecting, e.g. Plausible)

---

## 17. Open Questions

### 17.1 Vercel Build Integration

How do SvelteKit and Rust coexist in a single Vercel project? The current project uses `"framework": null` and relies on filesystem routing for TypeScript + a Rust catch-all. Switching to `"framework": "sveltekit"` changes Vercel's build detection. This needs a spike.

**Possible solutions:**
- Use `"framework": "sveltekit"` with a custom `buildCommand` that also runs `cargo build`
- Use Vercel's monorepo support to treat `packages/temper-ui` as the framework root
- Keep `"framework": null` and use SvelteKit's `adapter-node` output, served by the Rust Axum binary

### 17.2 Same-Origin vs. Cross-Origin API

If SvelteKit and Axum are on the same Vercel project, API calls from SvelteKit server functions are same-origin and don't need CORS. If they're separate projects, CORS must be configured. The routing strategy (§3) determines this. Same-origin is strongly preferred.

### 17.3 Embedding Generation for Search

Where does the text→embedding conversion happen for web search? See §11 for options. The recommended path (server-side text endpoint) requires changes to the Rust API.

### 17.4 Real-Time Updates

Should the app use WebSockets or SSE for real-time updates (e.g., a teammate sharing a resource with you)? Not in initial scope, but the architecture should not preclude it. SvelteKit's streaming responses and Vercel's serverless WebSocket support can be explored later.

### 17.5 Markdown Editing

Should the resource detail page allow inline markdown editing (like a mini Obsidian)? This would require a rich markdown editor component (e.g., Milkdown, TipTap, or CodeMirror with markdown mode). Deferred to a future phase — initial implementation is read-only with edit-metadata-only forms.

### 17.6 Content Security Policy

SvelteKit generates inline scripts for hydration. A strict CSP with nonce-based script allowlisting is recommended. Vercel supports CSP headers via `vercel.json`. The d3.js visualization may require `unsafe-eval` for dynamic layouts — investigate alternatives.

---

## 18. Security Considerations

### 18.1 Session Security

- **Encrypted cookies**: Session tokens stored in HTTP-only, Secure, SameSite=Lax cookies
- **Cookie encryption**: AES-256-GCM with `SESSION_SECRET`, rotatable
- **CSRF protection**: SvelteKit's built-in CSRF protection (Origin header checking) for form actions
- **Token storage**: Access tokens never sent to the client; all API calls happen server-side

### 18.2 Access Control

- **Server-side enforcement**: All access control is enforced by the Rust API via `resources_visible_to()`, `can_modify_resource()`, `can_manage_team()`
- **Direct Neon queries**: Must use `profile_id` from the verified session; never trust client-provided IDs
- **No client-side routing for security**: Protected routes redirect server-side in `hooks.server.ts`, not via client-side guards that can be bypassed

### 18.3 Input Validation

- **API proxy**: The SvelteKit server validates and sanitizes inputs before forwarding to the Rust API
- **Markdown rendering**: Use `marked` with `sanitize: true` or DOMPurify to prevent XSS in user-authored content
- **Query parameters**: Validate and type-check all URL parameters in `+page.server.ts` loaders

---

## 19. Dependency on Other Workstreams

| Dependency | Status | Impact on temper-ui |
|------------|--------|---------------------|
| **I5f** — Context CRUD in Axum | In progress | Unblocks context management pages |
| **I5g** — CLI-native ingest | In progress | Moves ingest to Rust; simplifies API surface |
| **R7** — Knowledge graph model | Proposed | Unblocks graph visualization (Phase 5) |
| **R8** — Local search / PageIndex | Proposed | No direct UI dependency; local-only feature |
| **Teams API** | Not started | Must be built as part of Phase 4 |
| **Text search endpoint** | Not started | Must be built as part of Phase 3 |
| **Graph traversal API** | Not started | Must be built as part of Phase 5 |

---

## 20. Success Criteria

1. **Public surface**: temperkb.io serves a compelling landing page and comprehensive documentation that drives GitHub stars and user adoption
2. **Auth flow**: Auth0 login/logout works seamlessly; users see their KB within seconds of first login
3. **Resource management**: Users can browse, search, and view all resources visible to them, with full fidelity to CLI behavior
4. **Team collaboration**: Users can create teams, invite members, share resources, and manage permissions — all operations that were designed in R4 but had no UI surface
5. **Knowledge graph**: Users can visually explore the connections between their resources — the "aha moment" for the value of structured knowledge
6. **Self-hosting story**: Enterprise users can follow the self-hosting guide and deploy their own temper instance with their own auth provider
7. **Performance**: All pages load in <2s on 3G. Core Web Vitals in green. No layout shift on authenticated pages.
8. **Consistency**: Every domain operation (create, update, delete, search, share) produces identical results whether initiated from CLI, MCP, or web UI — because they all flow through the same Rust API

---

## Appendix A: Comparison with Alternative Approaches

### A.1 Why Not a Separate Frontend Repository?

A separate repo (like `storyteller-site`) would be simpler in some ways but creates:
- **Deployment coupling**: Two Vercel projects, CORS configuration, separate CI/CD
- **Type drift**: TypeScript types diverge from Rust types over time
- **API versioning**: Need formal API versioning sooner
- **Auth complexity**: Cross-origin token forwarding

The monorepo approach (`packages/temper-ui`) keeps everything in one deployment, one CI pipeline, and one source of truth for types.

### A.2 Why SvelteKit Over Next.js or Remix?

- **Established pattern**: storyteller-site already validates SvelteKit + Vercel + Neon
- **Svelte 5 performance**: Compiled reactivity, smaller bundles, faster hydration
- **Server-side simplicity**: `+page.server.ts` loaders are cleaner than Next.js's data fetching patterns
- **d3.js compatibility**: Svelte's direct DOM access works better with d3 than React's virtual DOM
- **Team familiarity**: The team already has Svelte 5 (runes mode) experience

### A.3 Why Tailwind Over Hand-Crafted CSS?

storyteller-site uses bespoke CSS custom properties for a literary aesthetic. temper-ui has 3-4x the page count and a more conventional SaaS layout. Tailwind provides:
- Utility-first rapid prototyping
- Built-in responsive, dark mode, and animation utilities
- Consistent design system without a designer
- Broad ecosystem of Tailwind-compatible component libraries if needed later

---

## Appendix B: API Surface Summary (Current + Proposed)

### Current Endpoints (Rust Axum)

| Method | Path | Auth | Status |
|--------|------|------|--------|
| `GET` | `/api/health` | No | ✅ Live |
| `GET` | `/api/resources` | Yes | ✅ Live |
| `GET` | `/api/resources/{id}` | Yes | ✅ Live |
| `GET` | `/api/resources/{id}/content` | Yes | ✅ Live |
| `POST` | `/api/resources` | Yes | ✅ Live |
| `PATCH` | `/api/resources/{id}` | Yes | ✅ Live |
| `DELETE` | `/api/resources/{id}` | Yes | ✅ Live |
| `GET` | `/api/profile` | Yes | ✅ Live |
| `PATCH` | `/api/profile` | Yes | ✅ Live |
| `GET` | `/api/profile/auth-links` | Yes | ✅ Live |
| `GET` | `/api/contexts` | Yes | ✅ Live |
| `POST` | `/api/contexts` | Yes | ✅ Live |
| `GET` | `/api/contexts/{id}` | Yes | ✅ Live |
| `POST` | `/api/ingest` | Yes | ✅ Live |
| `PUT` | `/api/ingest/{id}` | Yes | ✅ Live |
| `GET` | `/api/events` | Yes | ✅ Live |
| `POST` | `/api/search` | Yes | ✅ Live |

### Proposed New Endpoints

| Method | Path | Phase | Priority |
|--------|------|-------|----------|
| `POST` | `/api/search/text` | 3 | High |
| `GET` | `/api/dashboard/stats` | 3 | Medium |
| `GET` | `/api/dashboard/activity` | 3 | Medium |
| `GET` | `/api/teams` | 4 | High |
| `POST` | `/api/teams` | 4 | High |
| `GET` | `/api/teams/{id}` | 4 | High |
| `PATCH` | `/api/teams/{id}` | 4 | Medium |
| `DELETE` | `/api/teams/{id}` | 4 | Medium |
| `GET` | `/api/teams/{id}/members` | 4 | High |
| `POST` | `/api/teams/{id}/members` | 4 | High |
| `PATCH` | `/api/teams/{id}/members/{pid}` | 4 | Medium |
| `DELETE` | `/api/teams/{id}/members/{pid}` | 4 | Medium |
| `GET` | `/api/teams/{id}/resources` | 4 | High |
| `POST` | `/api/teams/{id}/resources` | 4 | High |
| `DELETE` | `/api/teams/{id}/resources/{rid}` | 4 | Medium |
| `POST` | `/api/teams/{id}/invitations` | 4 | High |
| `GET` | `/api/invitations` | 4 | High |
| `POST` | `/api/invitations/{id}/accept` | 4 | High |
| `POST` | `/api/invitations/{id}/decline` | 4 | Medium |
| `POST` | `/api/transfers` | 4 | Medium |
| `GET` | `/api/transfers` | 4 | Medium |
| `POST` | `/api/transfers/{id}/accept` | 4 | Medium |
| `POST` | `/api/transfers/{id}/decline` | 4 | Low |
| `POST` | `/api/transfers/{id}/cancel` | 4 | Low |
| `POST` | `/api/graph/neighbors` | 5 | High |
| `POST` | `/api/graph/traverse` | 5 | High |
| `POST` | `/api/graph/subgraph` | 5 | High |
| `GET` | `/api/resources/{id}/edges` | 5 | High |
| `POST` | `/api/resources/{id}/edges` | 5 | Medium |
| `DELETE` | `/api/resources/{id}/edges/{eid}` | 5 | Low |

---

## Appendix C: Related Documents

| Document | Relevance |
|----------|-----------|
| `goals/temper/temper-cloud-ui-sveltekit-on-vercel.md` | Parent goal |
| `goals/temper/temper-cloud.md` | Umbrella cloud goal, key decisions |
| `goals/temper/temper-cloud-cli-api-usability.md` | CLI/API design decisions |
| `research/temper/R2 Data Model and Schema Design.md` | Schema that the UI queries |
| `research/temper/R4 Crate Architecture, Auth & Access Control.md` | Auth model, access control functions |
| `docs/superpowers/specs/2026-03-30-auth0-integration-design.md` | Auth0 tenant setup, PKCE flow |
| `docs/2026-03-31-user-workflow-analysis.md` | Auth-to-profile gap analysis |
| `docs/2026-04-01-i5e-handoff.md` | Config unification, vault layout, current state |
| `migrations/20260330000001_consolidated_schema.sql` | Full DDL including teams, transfers, invitations |
| R7 (proposed) | Vertex-edge graph model |
| R8 (proposed) | Local search / PageIndex |

---

## Appendix D: Tickets to Create

| Ticket | Title | Phase | Depends On |
|--------|-------|-------|------------|
| UI-1 | SvelteKit scaffold + Vercel routing spike | 1 | — |
| UI-2 | Auth0 server-side OIDC integration | 1 | UI-1 |
| UI-3 | Neon + API proxy helpers | 1 | UI-1 |
| UI-4 | Root layout, public nav, app layout group | 1 | UI-1 |
| UI-5 | Landing page | 2 | UI-4 |
| UI-6 | Documentation pages + docs layout | 2 | UI-4 |
| UI-7 | Self-hosting guide | 2 | UI-6 |
| UI-8 | Dashboard page | 3 | UI-2, UI-3 |
| UI-9 | Resource list + detail pages | 3 | UI-3 |
| UI-10 | Text search API endpoint (Rust) | 3 | — |
| UI-11 | Search page | 3 | UI-10 |
| UI-12 | Settings + profile pages | 3 | UI-2 |
| UI-13 | Teams API endpoints (Rust) | 4 | — |
| UI-14 | Team management pages | 4 | UI-13 |
| UI-15 | Invitations + transfers API (Rust) | 4 | UI-13 |
| UI-16 | Invitations + transfers pages | 4 | UI-15 |
| UI-17 | Graph traversal API (Rust) | 5 | R7 |
| UI-18 | Knowledge graph visualization (d3.js) | 5 | UI-17 |
| UI-19 | Dark mode + responsive polish | 6 | UI-8 |
| UI-20 | Accessibility + performance audit | 6 | All |