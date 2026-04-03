# SvelteKit UI Foundations — Design Spec

**Task**: `2026-04-02-sveltekit-ui-for-temperkb-io-foundations`
**Branch**: `jcoletaylor/sveltekit-ui-for-temperkb-io-foundations`
**Date**: 2026-04-02
**Status**: Approved

---

## 1. Architecture: Two Vercel Projects, One Repo, Single Origin

The temper monorepo hosts two Vercel projects:

| Project | Root Directory | Framework | Domain |
|---------|---------------|-----------|--------|
| `temper-cloud` (existing) | `/` (repo root) | `null` | Internal deployment URL only |
| `temper-ui` (new) | `packages/temper-ui` | `sveltekit` | `temperkb.io` |

**Routing via rewrites**: temper-ui owns `temperkb.io` and serves all user-facing routes. `/api/*` requests are rewritten to the temper-cloud deployment URL, preserving a single-origin experience. No CORS. The CLI and all API consumers continue hitting `temperkb.io/api/*` transparently.

```
temperkb.io (temper-ui project)
├── /              → SvelteKit (landing, docs, app pages)
├── /auth/*        → SvelteKit server routes (Auth0 OIDC)
├── /dashboard/*   → SvelteKit (authenticated app)
├── /api/*         → REWRITTEN to temper-cloud deployment
│   ├── /api/health, /api/resources, /api/search → Rust Axum
│   ├── /api/upload → TypeScript function
│   └── /api/auth/cli-callback → TypeScript function
└── static assets  → SvelteKit adapter-vercel CDN
```

**Why two projects instead of one**: Vercel does not support two frameworks in a single project. Setting SvelteKit as the framework hides the Rust `api/axum.rs` function. The Build Output API alternative would require us to own the full output assembly including Rust cross-compilation — equivalent to building our own framework adapter. Two projects is Vercel's documented monorepo pattern.

**Known trade-off**: Neon database branch auto-creation (preview environments) only works for the project with the Neon integration (temper-cloud). UI preview deployments will use the main Neon database unless manually configured. This is acceptable for now.

---

## 2. Project Structure

### New package: `packages/temper-ui/`

```
packages/temper-ui/
├── src/
│   ├── app.d.ts                 # SvelteKit type declarations
│   ├── app.html                 # HTML shell
│   ├── hooks.server.ts          # Auth middleware (stub in session 1)
│   ├── lib/
│   │   ├── server/
│   │   │   ├── db.ts            # postgres.js → Neon connection
│   │   │   └── api.ts           # Typed API proxy helpers (apiGet, apiPost)
│   │   ├── types/
│   │   │   ├── generated/       # ts-rs output from temper-core structs
│   │   │   └── index.ts         # Re-exports generated + UI-only types
│   │   ├── components/          # Svelte 5 components (runes mode)
│   │   └── styles/
│   │       └── app.css          # Tailwind v4 directives
│   └── routes/
│       ├── +layout.svelte       # Root layout: global styles, nav
│       ├── +page.svelte         # Landing page
│       └── (app)/               # Authenticated layout group
│           ├── +layout.svelte
│           └── dashboard/
│               └── +page.svelte
├── static/
│   └── robots.txt
├── svelte.config.js             # adapter-vercel, runes mode, nodejs22.x
├── vite.config.ts               # Minimal: sveltekit() plugin only
├── tsconfig.json                # Strict, extends .svelte-kit/tsconfig.json
├── package.json                 # @temper/ui
└── vercel.json                  # /api/* rewrite to temper-cloud
```

### Root changes

- `package.json`: add `"packages/temper-ui"` to workspaces
- Root `vercel.json`: **unchanged** — temper-cloud project continues to use it as-is

### Vercel project settings (dashboard configuration)

| Setting | Value |
|---------|-------|
| Root Directory | `packages/temper-ui` |
| Framework | SvelteKit (auto-detected) |
| Node.js Version | 22.x |

---

## 3. Technology Choices

### Matching storyteller-site

| Concern | Choice |
|---------|--------|
| Svelte | v5 runes mode (`$props()`, `$state()`, `$derived()`) |
| Adapter | `@sveltejs/adapter-vercel` with `nodejs22.x` |
| DB client | `postgres` (postgres.js) in `$lib/server/db.ts` |
| Vite | Minimal — `sveltekit()` plugin only |
| TypeScript | Strict mode, extends `.svelte-kit/tsconfig.json` |

### Diverging from storyteller-site

| Concern | Choice | Rationale |
|---------|--------|-----------|
| Styling | Tailwind CSS v4 (CSS-first config, `@theme` in CSS) | Faster iteration on 30+ pages |
| Auth | Server-side Auth0 OIDC via hooks.server.ts | Not needed in storyteller-site |
| API proxy | `$lib/server/api.ts` typed fetch helpers | Split data access: Neon-direct for nav chrome, API proxy for domain ops |
| Types | `ts-rs` codegen from Rust structs | Single source of truth, no drift |

### Type generation via ts-rs

- Add `ts-rs` dependency to `temper-core`
- Derive `TS` on domain structs: `Profile`, `Resource`, `Context`, `SearchResult`, `Team`, `TeamMember`, `GraphNode`, `GraphEdge`
- Generated `.ts` files output to `packages/temper-ui/src/lib/types/generated/`
- `cargo make generate-ts-types` task runs the export
- `$lib/types/index.ts` re-exports generated types alongside any UI-only type definitions

---

## 4. Vercel Configuration

### `packages/temper-ui/vercel.json`

```json
{
  "rewrites": [
    { "source": "/api/:path*", "destination": "${API_ORIGIN}/api/:path*" }
  ]
}
```

`API_ORIGIN` is an environment variable set in the temper-ui Vercel project, pointing to the temper-cloud deployment URL. Empty string or omitted for same-origin if Vercel supports that; otherwise the full `https://temper-cloud-*.vercel.app` URL.

### Environment variables (temper-ui project)

| Variable | Scope | Description |
|----------|-------|-------------|
| `DATABASE_URL` | Server | Neon connection string (same database as temper-cloud) |
| `API_BASE_URL` | Server | Rust API base for `$lib/server/api.ts` fetch calls from SvelteKit server loaders. Empty string in production (same-origin via rewrite), `http://localhost:3000` for local dev against a running Axum server. |
| `API_ORIGIN` | Build/Server | temper-cloud deployment URL used in vercel.json rewrites. Stable production URL: `https://temper-cloud.vercel.app`. This is the external URL of the API project — distinct from `API_BASE_URL` which is what server-side code uses for fetch. |
| `PUBLIC_APP_URL` | Client | `https://temperkb.io` |

Auth0 variables (`AUTH0_DOMAIN`, `AUTH0_CLIENT_ID`, `AUTH0_CLIENT_SECRET`, `SESSION_SECRET`) are added in session 2 when auth is implemented.

---

## 5. Data Access Strategy

Two paths for server-side data:

**Via Rust API** (through `/api/*` rewrite or `API_BASE_URL` in server loaders):
- All mutations (create, update, delete resources/contexts/teams)
- Domain logic (search with embeddings, profile resolution, access control)
- Anything using SQL functions with business rules (`resources_visible_to`, `can_modify_resource`)

**Direct Neon** (from `$lib/server/db.ts` in `+page.server.ts` loaders):
- Read-only navigation chrome (context list for sidebar, doc-type enumeration)
- Dashboard aggregation (resource counts, team counts)
- Always scoped by `profile_id` from the authenticated session

---

## 6. Session 1 Scope (This Session)

Delivers a working SvelteKit scaffold that can be deployed to Vercel alongside the existing API.

1. **SvelteKit scaffold** at `packages/temper-ui/` — Svelte 5, adapter-vercel, Tailwind v4, TypeScript strict
2. **ts-rs integration** — derive `TS` on key temper-core structs, cargo-make task, generated output in `$lib/types/generated/`
3. **Placeholder routes** — root layout with minimal nav, landing page, `(app)` group with placeholder dashboard
4. **Server-side stubs** — `$lib/server/db.ts` (Neon connection pattern), `$lib/server/api.ts` (typed proxy)
5. **`packages/temper-ui/vercel.json`** — `/api/*` rewrite configuration
6. **Root `package.json`** — add temper-ui to workspaces
7. **Local dev verification** — `bun run dev` starts, placeholder page renders at `localhost:5173`

---

## 7. Future Sessions

Ordered by dependency. Each session is a coherent deliverable that can be committed and deployed independently.

### Session 2: Auth0 Server-Side Integration

Implement the full OIDC Authorization Code + PKCE flow in SvelteKit server routes.

- **Auth routes**: `/auth/login` (redirect to Auth0 `/authorize`), `/auth/callback` (exchange code for tokens, set encrypted session cookie), `/auth/logout` (clear session, redirect to Auth0 `/v2/logout`)
- **`hooks.server.ts`**: Parse session cookie on every request, refresh expired access tokens via Auth0 `/oauth/token`, populate `event.locals.user` and `event.locals.accessToken`, redirect protected routes to login
- **Session storage**: Encrypted HTTP-only cookie using `jose` (JWE). Stores access token, refresh token, profile ID, expiry timestamp
- **Profile resolution**: On first login, call `GET /api/profile` with the Auth0 access token to auto-provision the profile (Rust API handles this). Store `profile_id` in session for Neon-direct query scoping
- **Auth0 dashboard changes**: Change `temper-web` app type from SPA to Regular Web Application. Verify callback URLs include `https://temperkb.io/auth/callback` and `http://localhost:5173/auth/callback`
- **Route protection**: `(app)` layout group's `+layout.server.ts` checks session, redirects to login if missing
- **Depends on**: Session 1 scaffold, Auth0 dashboard access

### Session 3: Public Marketing & Docs Pages

Build the public-facing landing page and documentation section.

- **Landing page** (`/`): Hero section, value proposition, "how it works" workflow visualization, key concepts cards, architecture diagram (SVG), CTA buttons
- **Docs layout** (`/docs/+layout.svelte`): Sidebar navigation component, docs-specific typography
- **Initial docs pages**: Getting Started, CLI Reference (adapted from README), Concepts (resources, contexts, doc types, search, sync)
- **SEO**: Meta tags, Open Graph images, `sitemap.xml`, `robots.txt`
- **Styling**: First real use of the Tailwind v4 design system — establish color palette (temper blue), typography scale (Inter + JetBrains Mono), dark mode toggle
- **Depends on**: Session 1 scaffold. Does NOT depend on auth.

### Session 4: Dashboard & Resource Browsing

First authenticated pages — users can log in and see their knowledge base.

- **Dashboard** (`/dashboard`): Stats cards (resource count, context count, team count), recent activity timeline via `GET /api/events`, quick action buttons, context overview cards
- **Resource list** (`/resources`): Paginated list via `GET /api/resources`, filters (context, doc type, resource mode), sort (updated, created, title)
- **Resource detail** (`/resources/[id]`): Metadata panel, rendered markdown content via `GET /api/resources/{id}/content`, sharing panel stub
- **Context pages** (`/contexts`, `/contexts/[id]`): Context list, context detail with resource list
- **Markdown rendering**: `marked` or similar for rendering resource content as HTML
- **Depends on**: Session 2 auth (all pages require authenticated session), session 1 scaffold

### Session 5: Search & Text Search Endpoint

Enable semantic search from the web UI.

- **New Rust API endpoint**: `POST /api/search/text` — accepts `{ query, context_name?, doc_type?, limit? }`, generates embedding server-side, calls existing `search_service::search()`. Start with HuggingFace Inference API for embedding generation.
- **Search page** (`/search`): Text input with context and doc-type filter dropdowns, results list with title/context/doc-type/snippet/score, result click navigates to resource detail
- **This endpoint also benefits the CLI** — `temper search` could switch to server-side embedding via this endpoint
- **Depends on**: Session 4 (search page lives in authenticated app), HuggingFace API key configuration

### Session 6: Teams, Invitations & Transfers

Team management and collaboration features.

- **New Rust API endpoints**: Teams CRUD, member management, resource sharing, invitations, transfers (as specified in task doc sections 6.1–6.3)
- **Team pages** (`/teams`, `/teams/[id]`, `/teams/[id]/members`, `/teams/[id]/resources`): Team list, detail, member management, shared resource management
- **Invitation pages** (`/invitations`): Accept/decline incoming invitations
- **Transfer pages** (`/transfers`): Accept/decline/cancel resource ownership transfers
- **Depends on**: Session 4 (authenticated app infrastructure), new API endpoints

### Session 7: Knowledge Graph Visualization

The showcase feature — d3.js force-directed graph of the user's knowledge base.

- **New Rust API endpoints**: `POST /api/graph/neighbors`, `POST /api/graph/traverse`, `POST /api/graph/subgraph`, resource edge CRUD
- **Graph page** (`/graph`): d3.js v7 force simulation, nodes colored by context, shaped by doc type, sized by edge count. Edge styling by type. Progressive loading with expand-on-double-click.
- **Interactions**: Click → navigate, double-click → expand neighbors, filter panel for edge types/contexts/doc-types, zoom/pan
- **Performance**: Cap at ~500 nodes, server-side `max_depth`/`max_nodes` params, incremental loading
- **Depends on**: Session 4, R7 knowledge graph schema in place

### Session 8: Settings, Polish & First Deploy

Final integration and production readiness.

- **Settings pages** (`/settings`): Profile editing, preferences, auth links
- **Error handling**: Toast notifications, error boundaries, loading states across all pages
- **Dark mode**: Tailwind `darkMode: 'class'` with user preference toggle, synced to profile preferences
- **Production deploy**: Domain transfer — point `temperkb.io` to temper-ui project, verify API rewrites work end-to-end, verify CLI still functions
- **Depends on**: All previous sessions

---

## 8. Open Questions (for future sessions)

- **Vercel rewrite destination**: Does `vercel.json` `rewrites` support environment variable interpolation, or do we need to hardcode the temper-cloud deployment URL? The stable production URL is `temper-cloud.vercel.app` if we need a fixed destination.
- **Neon branch coherence**: If preview environment drift between UI and API becomes a problem, consider configuring the UI project's `DATABASE_URL` to point to a Neon branch matching the API project's preview.
- **ts-rs output stability**: Verify ts-rs generates types compatible with SvelteKit's module resolution. May need a post-generation step to add proper ESM exports.
- **Local dev DX**: The two-server setup (Axum on 3000, SvelteKit on 5173) needs a smooth developer experience. Consider a root-level `cargo make dev` that starts both, or document the two-terminal workflow.
