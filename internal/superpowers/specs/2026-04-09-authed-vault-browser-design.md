# Authed Vault Browser Design

**Date**: 2026-04-09
**Task**: 2026-04-09-build-out-authed-ui-contents-contexts-resources-search
**Branch**: jct/temper-authed-dashboard-ui (continuing the existing branch)

## Overview

Build the authenticated vault browsing experience for temperkb.io: a SvelteKit
shell with a context-list sidebar, an SVAR DataGrid for sortable/filterable
resource browsing, a ⌘K command palette for global search, and full-route
resource detail pages whose URLs mirror `kb://` URIs 1:1. Backed by extended
`temper-api` endpoints that add pagination, sort, filter, search, facets, and
URI-shaped resource lookup.

The vault browser replaces the current `(app)/dashboard` route as the
post-login landing — the vault *is* the dashboard. Read-only (L0) for v1; no
content editing or metadata mutation from the UI in this scope.

This spec follows directly from the dashboard PR (`jct/temper-authed-dashboard-ui`,
3 commits ahead of origin), which established the auth+session+access-gate
plumbing, the dark editorial design language, and the Auth0 + system-access-gate
flow. Nothing in that work changes; this spec adds new routes and components
on top of the existing shell and extends the API surface its `+page.server.ts`
files consume.

## Locked Design Decisions

Five foundational decisions came out of brainstorming. Each is locked; the
implementation plan should not relitigate them.

| # | Decision | Choice |
|---|---|---|
| 1 | Data access architecture | Extend HTTP API endpoints. SvelteKit `+page.server.ts` files call `apiGet` (existing pass-through helper) — no direct Postgres access from SvelteKit. |
| 2 | Shell / nav structure | Context list as left sidebar, sidebar footer for non-vault chrome (Teams / Admin / Settings / user). Contexts grouped by owner — `@me` first, then any `+team-*`. |
| 3 | Search interaction | ⌘K opens a floating overlay. Search is global (no context scope). Overlay shows preview results (top ~10). "See all results" link navigates to `/vault/search?q=...` — a transient grid view, not persisted in the sidebar. |
| 4 | Read/write scope | L0 (read-only). Browse + search + view markdown content. No create, no edit, no delete from the UI. CLI/MCP remain the write surface. L1 (lightweight metadata edits — stage transitions, etc.) is explicit follow-up scope. |
| 5 | Detail view UX | Full route push: clicking a row navigates to `/vault/[owner]/[context]/[doc_type]/[ident]`. Dedicated page, full reading width, URL is shareable, browser back returns to grid. |

## 1. Architecture

The vault browser is a thin SvelteKit shell over an extended HTTP API. The
shell does no SQL, owns no auth logic, and holds no business rules — every
meaningful read goes through `temper-api` and reuses its `require_auth` +
`require_system_access` middleware. The browser is a *view* of the vault; the
vault is the system of record.

**Three layers:**

1. **`temper-core` (shared types).** Extend `ResourceListParams`, extend
   `ResourceRow` with joined display fields and managed-meta projections, add
   `ResourceListResponse` (rows + total), add `ResourceFacets`, add
   `ContextRowWithCounts`. All ts-rs derived. The contract — Rust services
   and SvelteKit `+page.server.ts` files consume the same generated types.
2. **`temper-api` (extended endpoints).** `GET /api/resources` gains query
   params and a wrapped response shape. `GET /api/contexts` always returns
   the count-enriched shape (no flag). Two new endpoints:
   `GET /api/resources/facets` and `GET /api/resources/by-uri`. Service layer
   (`resource_service`, `context_service`) holds the SQL — handlers stay thin.
3. **`temper-ui` (SvelteKit shell).** New `(app)/vault/...` route subtree
   replaces `(app)/dashboard` as the post-login destination. Server loads call
   `apiGet` (existing helper that attaches the bearer token from `locals`).
   No direct DB access from SvelteKit. SVAR Svelte DataGrid handles tables;
   everything else is hand-rolled with the existing dark editorial Tailwind
   styles.

**Single boundary, no parallel paths.** MCP tools call
`resource_service::list_visible(...)` directly. CLI commands call it. The HTTP
handler calls it. The SvelteKit `+page.server.ts` calls the HTTP handler. Same
query, same access checks, same behavior. The ts-rs cascade catches any type
drift at compile time on every consumer.

**Notable non-decisions** that fall out of this:

- No new database tables (only one new migration for indexes — see §8)
- No new SQL functions (just extended `WHERE` clauses, JSONB projections, and `COUNT(*)` aggregates)
- No client-side data store beyond what SvelteKit's load functions return
- No changes to the auth/session/access-gate plumbing — it already works

## 2. URL Structure and Route Map

SvelteKit routes mirror `kb://<owner>/<context>/<doc_type>/<ident>` 1:1, with
`/vault/` substituted for the URI scheme. The `@` character in URLs is allowed
(RFC 3986) and unambiguous.

| Path | Renders | Purpose |
|---|---|---|
| `/vault` | redirect → `/vault/all` | Default post-login landing |
| `/vault/all` | All-resources grid | No owner/context scope. Shows everything `resources_visible_to(profile)` returns. |
| `/vault/search?q=…` | Search-results grid | Transient. Sidebar context highlight cleared. URL-shareable. |
| `/vault/[owner]/[context]` | Context grid | e.g. `/vault/@me/temper`. Primary browsing surface. |
| `/vault/[owner]/[context]/[doc_type]/[ident]` | Resource detail | e.g. `/vault/@me/temper/task/my-task`. Full-route detail page. |
| `/admin/access` | (existing) | Reachable via sidebar footer. Implemented in current branch. |
| `/teams` | placeholder stub | Sidebar footer link. "Coming soon" until teams ship. |
| `/settings` | placeholder stub | Sidebar footer link. Stub for v1. |

**Cross-owner browsing.** The API already returns resources across every owner
the profile can see (`resources_visible_to`). The sidebar groups contexts by
owner: `@me` first, then any `+team-*` the profile belongs to. For v1 most
users have only `@me`, so the grouping degenerates to a flat list — but the
structure is in place from day 1, which means when team-context membership
ships, the sidebar lights up automatically with no UI work.

**Removed.** The existing `(app)/dashboard` route is deleted. There's no longer
a "dashboard" — the vault is the dashboard. The dashboard's current "recent
resources" panels are subsumed by the grid (sorted by `updated_at desc` by
default).

**Default `+page.server.ts` parameter handling.** Each grid route passes its
path params + URL query string straight through to a single `apiGet` call:

```ts
// /vault/[owner]/[context]/+page.server.ts
export const load: PageServerLoad = async ({ params, url, locals }) => {
  const qs = new URLSearchParams({
    owner: params.owner,
    context_name: params.context,
    ...Object.fromEntries(url.searchParams),  // doc_type, sort, order, limit, offset, q
  });
  const [resources, facets] = await Promise.all([
    apiGet(`/api/resources?${qs}`, locals.accessToken),
    apiGet(`/api/resources/facets?${qs}`, locals.accessToken),
  ]);
  return { resources, facets };
};
```

Two parallel requests per page load (resources + facets). Sidebar context list
is fetched once in the layout-level load (`/api/contexts`).

## 3. API Extensions

Three changes to `temper-api`, all driven by extended types in `temper-core`.
Every change cascades through ts-rs generation.

### 3.1 `temper-core` type changes

**Extended `ResourceListParams`** (`crates/temper-core/src/types/resource.rs`):

```rust
pub struct ResourceListParams {
    // existing UUID filters (kept for back-compat with internal callers)
    pub kb_context_id: Option<Uuid>,
    pub kb_doc_type_id: Option<Uuid>,
    // NEW: name-based filters (preferred from the UI, which works in URL slugs)
    pub context_name: Option<String>,
    pub doc_type_name: Option<String>,
    pub owner: Option<String>,           // "@me" | "+team-x" — resolved server-side
    pub q: Option<String>,                // FTS query, narrows the visible set
    pub sort: Option<ResourceSortField>,  // enum: Updated | Created | Title | Stage | Seq
    pub order: Option<SortOrder>,         // enum: Asc | Desc
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub enum ResourceSortField { Updated, Created, Title, Stage, Seq }
pub enum SortOrder { Asc, Desc }
```

**Extended `ResourceRow`** with the joined-in friendly fields the grid needs:

```rust
pub struct ResourceRow {
    // existing
    pub id: ResourceId,
    pub kb_context_id: ContextId,
    pub kb_doc_type_id: DocTypeId,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub originator_profile_id: ProfileId,
    pub owner_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // NEW: joined-in display fields
    pub context_name: String,             // from kb_contexts.name
    pub doc_type_name: String,            // from kb_doc_types.name
    pub owner_handle: String,             // "@me" | "+team-x" from kb_profiles
    // NEW: managed_meta projections (nullable — not all docs have these)
    pub stage: Option<String>,            // managed_meta->>'temper-stage'
    pub seq: Option<i64>,                 // (managed_meta->>'temper-seq')::bigint
    pub mode: Option<String>,             // managed_meta->>'temper-mode'
    pub effort: Option<String>,           // managed_meta->>'temper-effort'
}
```

**New wrapper response** for list endpoints:

```rust
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
}
```

**New types for facets and contexts-with-counts:**

```rust
pub struct ResourceFacets {
    pub doc_type: HashMap<String, i64>,  // {"task": 80, "session": 220, ...}
    pub stage: HashMap<String, i64>,     // {"in-progress": 5, "done": 240, ...}
}

pub struct ContextRowWithCounts {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub resource_count: i64,             // NEW
}
```

All new and changed types get
`#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]`. TS bindings
regenerate via `cargo make generate-ts-types`.

### 3.2 Endpoint changes

**`GET /api/resources`** — extended params, new response shape:

| Before | After |
|---|---|
| Returns `Vec<ResourceRow>` | Returns `ResourceListResponse { rows, total }` |
| Query params: `kb_context_id`, `kb_doc_type_id`, `limit`, `offset` | Adds: `context_name`, `doc_type_name`, `owner`, `q`, `sort`, `order` |
| `ResourceRow` with bare DB columns | `ResourceRow` with joined names + frontmatter projections |

The service-layer SQL changes from a simple `SELECT FROM kb_resources` to a
`LEFT JOIN kb_resource_manifests`, `JOIN kb_contexts`, `JOIN kb_doc_types`,
`JOIN kb_profiles`, with parameterized `WHERE` and
`ORDER BY (managed_meta->>'temper-seq')::bigint` for sortable frontmatter
fields. All inside `resource_service::list_visible`. The
`resources_visible_to(profile_id)` CTE stays exactly where it is — we're
augmenting the projection, not changing the access predicate.

**`GET /api/contexts`** — always returns the count-enriched shape:

```
GET /api/contexts → Vec<ContextRowWithCounts>
```

The existing `Vec<ContextRow>` shape is replaced by `Vec<ContextRowWithCounts>`,
which adds a single `resource_count: i64` field. This is a breaking change
identical in nature to the `ResourceListResponse` change — caught at compile
time by the ts-rs cascade through every consumer (CLI, MCP, UI, client). The
extra `COUNT(*)` per row is negligible (contexts are typically <20 rows;
SQL groups them in a single query). Avoiding polymorphic response types
keeps the OpenAPI spec clean and removes a class of utoipa derive friction.

**`GET /api/resources/facets` (new)** — returns aggregated counts for the
current filter set, for the chip UI above the grid:

```
GET /api/resources/facets?context_name=temper&owner=@me
→ ResourceFacets {
    doc_type: { "task": 80, "session": 220, "goal": 12, ... },
    stage:    { "backlog": 8, "in-progress": 5, "done": 240, ... }
  }
```

Same access predicate (`resources_visible_to`), same `WHERE` filters as the
list endpoint, but groups by doc_type and managed_meta stage. New service
function `resource_service::compute_facets`, new handler
`handlers::resources::facets`, new route entry under the gated router.

**`GET /api/resources/by-uri` (new)** — resolves a kb-URI-shaped path to a
resource:

```
GET /api/resources/by-uri?owner=@me&context=temper&doc_type=task&ident=my-task
→ ResourceRow
```

Single-purpose lookup that mirrors `Vault::parse_uri` from
`temper-core::vault`. The `ident` parameter accepts either a slug or a UUID.
Returns 404 if no matching resource exists or if it's not visible to the
profile. New service function `resource_service::resolve_by_uri`, new handler,
new route under the gated router.

### 3.3 Frontmatter projection from `kb_resource_manifests`

`stage`, `seq`, `mode`, `effort` are not columns on `kb_resources`. They live
in `kb_resource_manifests.managed_meta` as JSONB, written by the meta service
on create/update. The grid query `LEFT JOIN`s this table and projects the
JSONB keys:

```sql
SELECT
  r.id, r.title, r.slug, r.created, r.updated,
  c.name AS context_name,
  dt.name AS doc_type_name,
  CASE p.kind WHEN 'profile' THEN '@' || p.handle ELSE '+' || p.handle END AS owner_handle,
  m.managed_meta->>'temper-stage' AS stage,
  (m.managed_meta->>'temper-seq')::bigint AS seq,
  m.managed_meta->>'temper-mode' AS mode,
  m.managed_meta->>'temper-effort' AS effort
FROM resources_visible_to($1) v
JOIN kb_resources r ON r.id = v.resource_id
JOIN kb_contexts c ON c.id = r.kb_context_id
JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
JOIN kb_profiles p ON p.id = r.owner_profile_id
LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
WHERE ...
ORDER BY ...
LIMIT $N OFFSET $M;
```

Sorting and filtering by these projected fields works via the indexes added in
§8. Nullability: not all doc types have all fields; the grid renders `—` for
null cells.

### 3.4 Breaking changes — known and accepted

- **`Vec<ResourceRow>` → `ResourceListResponse { rows, total }`** is a breaking
  change to the JSON shape of `GET /api/resources`. Cascades through
  `temper-core::types::resource::ResourceRow` and `ResourceListResponse`:
  - `temper-client::resources::list()` won't compile until updated to return
    `ResourceListResponse`
  - `temper-cli` commands that print the list need a one-line update to read
    `.rows`
  - `temper-mcp` tools that delegate to the service get the new shape
    automatically (they call `list_visible` directly)
  - `unified_search` and other endpoints are unaffected
- **`ResourceRow` adds new non-`Option` fields** (`context_name`, `doc_type_name`,
  `owner_handle`). Same cascade — every consumer gets a compile error until
  they handle them.
- **`Vec<ContextRow>` → `Vec<ContextRowWithCounts>`** for `GET /api/contexts`.
  Adds `resource_count: i64` to every returned row. Cascades through
  `temper-client::contexts`, any CLI command that lists contexts, and the MCP
  tool that exposes contexts. Each consumer either uses the new field or
  ignores it; either way the compile passes.

These are exactly the kind of "wider knock-on" change the ts-rs cascade is
designed to make safe. The test suite (`crates/temper-api/tests/resources_test.rs`,
`tests/contexts_test.rs`, plus the e2e suite) catches behavioral regressions.

## 4. Data Flow

### 4.1 Loading the context grid (`/vault/@me/temper`)

1. Browser navigates to `/vault/@me/temper`. SvelteKit matches
   `(app)/vault/[owner]/[context]`.
2. `(app)/+layout.server.ts` runs (layout load). Validates session +
   entitlements via `locals`. Fetches sidebar contexts:
   `apiGet('/api/contexts', locals.accessToken)` →
   `Vec<ContextRowWithCounts>`. Returned as `data.contexts`.
3. `(app)/vault/[owner]/[context]/+page.server.ts` runs in parallel for
   page-level data. Builds `URLSearchParams` from path params + query string,
   makes two parallel requests:
   - `apiGet('/api/resources?...')` → `ResourceListResponse { rows, total }`
   - `apiGet('/api/resources/facets?...')` → `ResourceFacets`
4. `apiGet` (existing) attaches `Authorization: Bearer ${locals.accessToken}`
   and forwards to `temper-api`.
5. `temper-api` routes through `require_auth` → `require_system_access` →
   `handlers::resources::list`. Handler calls `resource_service::list_visible`.
6. `resource_service::list_visible` runs the extended SQL from §3.3. A second
   `SELECT count(*)` over the same predicates produces `total`. Returns
   `ResourceListResponse`.
7. `+page.svelte` receives `data.contexts`, `data.resources`, `data.facets`.
   Renders sidebar from contexts, chips from facets, feeds rows + total into
   SVAR DataGrid. Sort/page interactions update `url.searchParams` via
   `goto()`, re-running the page load.
8. Total round-trips per page load: 3 (contexts in layout, resources + facets
   in parallel in page). Layout contexts fetch is cached for the session
   unless invalidated.

### 4.2 ⌘K command palette → "see all results"

1. User presses ⌘K anywhere in `(app)`. Global keydown listener in
   `(app)/+layout.svelte` opens `<CommandPalette />`.
2. User types. Each keystroke (debounced 150ms) calls a SvelteKit `+server.ts`
   proxy at `(app)/_internal/search/+server.ts` that re-uses the server-side
   `locals.accessToken` and forwards to `temper-api`. The proxy keeps the
   bearer token off the client.
3. `temper-api` responds with `ResourceListResponse { rows, total }` where
   `rows` is up to ~10 preview hits.
4. Overlay renders rows with title · context · doc_type. Arrow keys navigate,
   Enter opens the focused row.
5. User clicks "See all N results". `goto('/vault/search?q=...')`.
6. `(app)/vault/search/+page.server.ts` runs the same `apiGet` but with the
   full grid limit. Sidebar context highlight cleared. Main area renders the
   grid with a heading rule "Search: <query> · N results · ×" where × goes
   back to `/vault/all`.
7. SVAR's column filters work as normal — user can narrow within the result
   set client-side. No new server round-trip needed for in-grid narrowing.

### 4.3 Opening a resource detail (`/vault/@me/temper/task/my-task`)

1. User clicks a row. SVAR row click handler navigates:
   `goto('/vault/' + row.owner_handle + '/' + row.context_name + '/' + row.doc_type_name + '/' + (row.slug ?? row.id))`.
2. `(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts` runs.
   Resolves the URI and fetches content in two calls:
   - `apiGet('/api/resources/by-uri?owner=...&context=...&doc_type=...&ident=...', locals.accessToken)` → `ResourceRow`
   - `apiGet(`/api/resources/${row.id}/content`, locals.accessToken)` → `ContentResponse { resource_id, markdown }`
3. `+page.svelte` renders the detail view: `<ResourceMetaHeader />` + breadcrumb
   + `<MarkdownRenderer />`. The renderer uses `marked` + `dompurify` to
   produce sanitized HTML targeting the existing dark editorial prose styles.
4. Browser back works naturally; SvelteKit restores scroll position. Explicit
   "← {context}" link in the detail header.

### 4.4 Sidebar context switching

Trivial: clicking a sidebar context entry is an `<a href="/vault/@me/{name}">`
link. The (app) layout doesn't re-run (same parent layout), only the
page-level load runs. Layout-level data (`contexts`, `user`, `entitlements`)
stays cached. Active highlight derives from `$page.params`.

### 4.5 Token refresh and stale data

The existing `hooks.server.ts` already refreshes tokens near expiry on every
request — no change needed. Contexts list fetched at layout load is fresh for
the session. If a new context is created via CLI/MCP while the user is
browsing, they see it on next navigation. No SSE/websocket subscription in v1.

## 5. Components and File Structure

### 5.1 New components (`packages/temper-ui/src/lib/components/`)

| Component | Responsibility |
|---|---|
| `Sidebar.svelte` | Two-section shell: scrollable contexts list + bottom footer with Teams / Admin / Settings / user row. Active state derives from `$page.params`. |
| `ContextNavGroup.svelte` | One owner's contexts as a labeled list. Used inside `Sidebar`. Renders the count badge from `ContextRowWithCounts.resource_count`. |
| `VaultGrid.svelte` | Wraps SVAR DataGrid. Defines columns (Title, Type, Stage, Updated, Seq, Owner). Binds sort/page/filter state to `url.searchParams` via `goto()`. Row click navigates to the detail route. |
| `FacetChips.svelte` | Doc-type chips above the grid. Reads `data.facets`, renders one chip per doc type with count, click toggles `?doc_type=` in the URL. |
| `RuleHeading.svelte` | Editorial left-rule heading pattern (yellow `border-left`, title + caption). Verify whether this exists from the dashboard build before creating new. |
| `MarkdownRenderer.svelte` | Renders markdown to HTML using `marked` + `dompurify`. Sanitized output. Targets the existing dark prose CSS. |
| `ResourceMetaHeader.svelte` | Detail-view header: `RuleHeading` + meta-row (seq, stage, author, context, updated). |
| `CommandPalette.svelte` | The ⌘K overlay. Listens to global keydown in `(app)/+layout.svelte`. Debounced 150ms input → fetch via `_internal/search` proxy → preview list with arrow keys + Enter. "See all results" link at bottom navigates to `/vault/search?q=...`. |
| `EmptyState.svelte` | Generic empty-state for "no resources in this context", "no search results", etc. |

### 5.2 Modified existing files

| File | Change |
|---|---|
| `(app)/+layout.svelte` | Replace shell with `<Sidebar />` + main slot. Mount global ⌘K listener. |
| `(app)/+layout.server.ts` | Add `apiGet('/api/contexts', locals.accessToken)`. Return `contexts` in `data`. Existing user/entitlements/profile work unchanged. |

### 5.3 Routes — full file tree

```
packages/temper-ui/src/routes/
├── (app)/
│   ├── +layout.svelte                                       MODIFIED
│   ├── +layout.server.ts                                    MODIFIED
│   ├── dashboard/                                           DELETED (replaced by /vault/all)
│   ├── vault/
│   │   ├── +page.server.ts                                  NEW — redirect to /vault/all
│   │   ├── all/
│   │   │   ├── +page.server.ts                              NEW
│   │   │   └── +page.svelte                                 NEW
│   │   ├── search/
│   │   │   ├── +page.server.ts                              NEW
│   │   │   └── +page.svelte                                 NEW
│   │   └── [owner]/
│   │       └── [context]/
│   │           ├── +page.server.ts                          NEW
│   │           ├── +page.svelte                             NEW
│   │           └── [doc_type]/
│   │               └── [ident]/
│   │                   ├── +page.server.ts                  NEW
│   │                   └── +page.svelte                     NEW
│   ├── _internal/
│   │   └── search/
│   │       └── +server.ts                                   NEW — proxy for ⌘K overlay
│   ├── teams/
│   │   └── +page.svelte                                     NEW — placeholder stub
│   ├── settings/
│   │   └── +page.svelte                                     NEW — placeholder stub
│   ├── admin/                                               existing — no changes
│   └── +error.svelte                                        NEW — in-shell error page
└── (public)/                                                existing — no changes
```

The `_internal/` prefix is used (instead of `/api/`) because Vercel rewrites
`/api/*` to `temper-cloud`/`temper-api`. Implementation may pick a different
prefix during plan writing if a better convention exists in the codebase.

### 5.4 Library additions (`packages/temper-ui` deps)

- **`@svar-ui/svelte-grid`** (or whatever the actual SVAR Svelte DataGrid
  package name is — verify exact name and version during plan writing). MIT
  license. Svelte 5 runes confirmed compatible.
- **`marked`** for markdown rendering. MIT, small.
- **`dompurify`** for HTML sanitization. Critical for L0 read-only — we're
  rendering arbitrary user-authored markdown, even though all authors are
  trusted.

No new dev deps. Existing Svelte/Tailwind/TypeScript toolchain handles the
rest.

### 5.5 What's hand-rolled vs library

- **SVAR DataGrid handles:** column rendering, sort UI, column filter UI,
  virtualized scrolling for long lists, row selection. Hand-rolled binding to
  URL params is one wrapper component (`VaultGrid.svelte`).
- **Hand-rolled:** sidebar, command palette, facet chips, markdown renderer
  wrapper, empty states, editorial heading patterns.
- **Reused:** the existing `apiGet` helper, the existing dark editorial
  Tailwind config, the existing auth/session/access-gate plumbing.

### 5.6 Documented fallback

If SVAR Svelte DataGrid hits a hard incompatibility during implementation
(Svelte 5 runes regression, dealbreaker bug, license surprise), the documented
fallback is **TanStack Table v9 (Svelte 5 native adapter)** with hand-rolled
UI (~300-400 LoC). This is recorded as a contingency, not a planned path.

## 6. Error Handling

Read-only L0 simplifies the surface area — no failed-write rollbacks. Error
story is dominated by API call failures during page loads and ⌘K overlay
calls.

### 6.1 SvelteKit error conventions

Standard SvelteKit error handling: `error(status, message)` from
`@sveltejs/kit` thrown inside `+page.server.ts` triggers the nearest
`+error.svelte`. Two error pages added:

- `(app)/+error.svelte` — generic in-shell error page that keeps the sidebar
  visible. Renders error code, friendly message, "Back to vault" link.
- (Optional) a route-specific override under
  `[owner]/[context]/[doc_type]/[ident]` if a more specific 404 message is
  needed for resource lookups.

### 6.2 Per-call failure modes

| Failure | Where | Behavior |
|---|---|---|
| Network/timeout on `/api/resources` | grid `+page.server.ts` | `error(503, 'Vault temporarily unavailable')` → in-shell error page. Sidebar still works. |
| Network/timeout on `/api/resources/facets` | grid `+page.server.ts` | **Graceful degradation:** try/catch, return `data.facets = null`. Grid renders without chips. |
| Network/timeout on `/api/contexts` | `(app)/+layout.server.ts` | Catch and return `data.contexts = []`. Sidebar renders an `<EmptyState>` with retry. Don't `error()` — sidebar must never take down the app. |
| 401 Unauthorized | any `apiGet` | Existing `hooks.server.ts` refresh handles routine cases. If refresh fails, existing pattern redirects to `/auth/login?returnTo=<current>`. |
| 403 SYSTEM_ACCESS_REQUIRED | any `apiGet` | Already handled by `(app)/+layout.server.ts` entitlements check. Mid-session loss-of-access redirects on next navigation. |
| 404 on by-uri lookup | detail `+page.server.ts` | `error(404, 'Resource not found')` → in-shell error page with the offending kb-URI shown. "Back to {context}" link. |
| 404 on context lookup | grid `+page.server.ts` | `error(404, 'Context "<name>" not found')` — silent redirects hide bugs and broken bookmarks. |
| 500 from any endpoint | any `apiGet` | `error(500, 'Server error')` → in-shell error page. Console-logs the response body. |
| Empty result set (no rows) | grid load | **Not an error.** Render the grid with an `<EmptyState>` overlay: "No resources match these filters." Reset-filters button if any chip filters are active. |

### 6.3 ⌘K overlay error handling

| Failure | Behavior |
|---|---|
| Network/timeout on the proxy | Inline error row in the overlay: "Search unavailable. Try again." Pressing Enter retries; escape closes. |
| 401 from temper-api | Overlay shows "Please sign in again" with a `/auth/login` link. |
| Empty results | Not an error. "No results" inside the overlay. |

### 6.4 Markdown render failures

`MarkdownRenderer.svelte` uses `marked` + `dompurify`:

- Malformed markdown — `marked` is permissive, no error path needed.
- DOMPurify removes everything — render an inline `<EmptyState>`: "This resource appears to be empty or contains unsupported content."
- `marked` itself throws — wrap in try/catch; on throw, render a minimal `<pre>` with raw markdown text and console.error.

### 6.5 Deliberately not in scope

- No retry-with-backoff logic
- No global toast system
- No client-side error tracking (Sentry, etc.)
- No write-conflict handling (no writes)

## 7. Testing Strategy

Testing pyramid leans on Rust integration tests with real Postgres,
compile-time type cascades, and minimal-but-targeted e2e tests. Frontend gets
manual smoke testing — building UI test infrastructure is out of scope.

### 7.1 Rust unit tests (`temper-api/src/services/`)

Service-layer logic gets unit tests where the logic is non-trivial:

- `resource_service::list_visible` filter resolution — each filter
  (context_name, doc_type_name, owner, q, sort) in isolation and combined
- `resource_service::list_visible` sort order — by `temper-seq` (numeric,
  JSONB cast), `updated_at`, `title`, `stage`. Both desc and asc.
- `resource_service::list_visible` pagination — `limit`/`offset` boundary
  cases: 0, max, mid-page, beyond-end
- `resource_service::compute_facets` — counts match the visible rows for the
  same filter set; empty filter returns global counts
- `resource_service::resolve_by_uri` — slug + UUID lookups both work; 404 on
  miss; respects owner/context/type composition; reuses `Vault::parse_uri`
- `context_service::list_visible_with_counts` — counts match `kb_resources`
  row counts grouped by context; zero-resource contexts still appear with
  `resource_count: 0`

Run via `cargo nextest run -p temper-api --features test-db` against the
Docker Postgres on port 5437.

### 7.2 Rust integration tests (`temper-api/tests/`)

Existing `tests/resources_test.rs` extended for the new endpoint shapes. New
files:

- `tests/resources_facets_test.rs` for the facets endpoint
- `tests/resources_by_uri_test.rs` for the by-uri endpoint

Coverage targets:

- `GET /api/resources` with new query params returns `ResourceListResponse`
  with correct rows and total
- `GET /api/resources` with `q=` does FTS narrowing and returns relevant rows
- `GET /api/resources` honors `resources_visible_to` — a profile not in a
  context can't see its rows
- `GET /api/resources/facets` returns aggregates matching the filtered list
- `GET /api/contexts` returns `Vec<ContextRowWithCounts>` with accurate
  per-context resource counts
- `GET /api/resources/by-uri` resolves slug, resolves UUID, returns 404 on
  miss, returns 403 on inaccessible
- 401 for unauthenticated requests, 403 for system-access-gated unauthenticated
  requests

### 7.3 e2e tests (`crates/temper-e2e/`)

Add `tests/vault_browse_test.rs` that spins up the full stack, creates two
contexts with several resources each, makes authenticated calls to:
- `/api/contexts`
- `/api/resources?context_name=...`
- `/api/resources/facets?...`
- `/api/resources/by-uri?...`

Verifies the responses chain together as the UI would consume them. Doesn't
test SvelteKit — tests the API contract end-to-end.

### 7.4 Compile-time guarantees (the ts-rs cascade)

These don't need explicit tests — the compiler enforces them:

- Every `ResourceRow` consumer in the workspace gets the new fields. If
  `temper-cli`'s table renderer doesn't handle them, build fails.
- `ResourceListResponse` replacing `Vec<ResourceRow>` cascades through
  `temper-client::resources::list()`. If a CLI command unpacks the old shape,
  build fails.
- `cargo make generate-ts-types` regenerates `packages/temper-core-types/`. If
  SvelteKit `+page.server.ts` files import a stale shape, `bun run check`
  catches it.
- `cargo sqlx prepare --workspace -- --all-features` regenerates the offline
  cache. CI's `SQLX_OFFLINE=true` build catches drift.

These gates run as part of `cargo make check` and via CI in
`code-quality.yml` and `test-rust.yml`.

### 7.5 SvelteKit testing — explicitly skipped

- No component tests. temper-ui has no Vitest/Playwright setup today; adding
  it would double scope. Deferred to a follow-up "stand up frontend test infra"
  task.
- No visual regression tests.
- No e2e through the SvelteKit app.

What we get for free on the frontend:
- `bun run check` (svelte-check) catches type errors against regenerated TS types
- `bun run build` catches SvelteKit-level build errors

### 7.6 Manual smoke test ritual

Before considering this PR complete, manually verify in the browser against a
local dev stack (`cargo make docker-up`, `cargo make run`,
`cd packages/temper-ui && bun run dev`):

1. Sign in with Auth0 → land on `/vault/all` → see all-resources grid populated
2. Click a context in the sidebar → grid filters to that context, URL updates
3. Click a doc-type chip → grid narrows, URL updates with `?doc_type=`
4. Sort a column → grid resorts, URL updates with `?sort=&order=`
5. Click a row → land on `/vault/@me/<context>/<doc_type>/<slug>`, see rendered markdown
6. Use the back button → return to grid, scroll position preserved
7. Press ⌘K → overlay opens, type a query, see preview results
8. Click "see all results" → land on `/vault/search?q=...`, grid shows full results
9. Click sidebar footer "Admin" → land on `/admin/access` (existing route)
10. Click sidebar footer "Teams" → land on placeholder stub
11. Force a 404: navigate to `/vault/@me/nonexistent/task/x` → see in-shell 404
12. Permission test: have a second profile request access, sign in as them, verify they only see their visible vault contents

### 7.7 Verification gate before commit

```
cargo make check        # rust fmt + clippy + machete + ts typecheck + biome
cargo make test         # unit tests
cargo make test-db      # integration tests with Docker postgres
cd packages/temper-ui && bun run check && bun run build
```

All four must pass before commit. CI re-runs the same gates on push.

## 8. Migrations

One new migration to add indexes on `kb_resource_manifests.managed_meta` for
the JSONB projections used by the grid query.

**File:** `migrations/20260410000001_index_resource_manifests_managed_meta.sql`

```sql
-- B-tree expression indexes on the specific managed_meta keys we sort and filter by.
-- These accelerate the `->>'temper-*'` extractions used in the vault grid query.
CREATE INDEX idx_manifests_managed_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'));
CREATE INDEX idx_manifests_managed_seq
    ON kb_resource_manifests (((managed_meta->>'temper-seq')::bigint));
CREATE INDEX idx_manifests_managed_mode
    ON kb_resource_manifests ((managed_meta->>'temper-mode'));
CREATE INDEX idx_manifests_managed_effort
    ON kb_resource_manifests ((managed_meta->>'temper-effort'));
CREATE INDEX idx_manifests_managed_doc_type
    ON kb_resource_manifests ((managed_meta->>'temper-type'));

-- GIN index with jsonb_path_ops for future ad-hoc containment queries.
-- Smaller and faster to maintain than the default jsonb_ops; supports `@>`.
CREATE INDEX idx_manifests_managed_meta_gin
    ON kb_resource_manifests USING gin (managed_meta jsonb_path_ops);
```

After applying, regenerate the sqlx offline cache:

```bash
cargo sqlx prepare --workspace -- --all-features
```

Commit both the migration and the regenerated `.sqlx/` cache as part of the
same change.

## 9. Out of Scope / Future Work

Captured here so the implementation plan doesn't accidentally drift into
adjacent territory:

- **L1 metadata edits** — stage transitions, tag edits, renaming via the UI.
  The natural follow-up after L0 ships and we know which writes people reach
  for. Hooks into the existing `PUT /api/resources/{id}/meta` endpoint.
- **L2/L3 content editing** — full markdown editor in the detail view. Out of
  scope, possibly never. Temper is not a web markdown editor.
- **CLI extension to use new pagination/sort/filter machinery** — the new API
  capabilities (paging, sort, filter, facets) enable richer
  `temper resource list` commands. Hold for a future task; this PR only
  extends the API and the UI.
- **Teams browsing** — placeholder stub only. The full teams UI (members,
  invitations, ownership transfers) is its own future session.
- **Admin-initiated team invitations and ownership transfers** — backend
  tables exist but no UI. Future session.
- **Knowledge graph d3 visualization** — depends on R7 graph schema. Future.
- **Vitest / Playwright for SvelteKit** — frontend test infra is its own
  setup task.
- **Visual regression testing** — high-churn editorial design makes this
  expensive. Defer until design stabilizes.
- **Saved searches** — the `/vault/search?q=...` state is transient. No
  persistence in v1.
- **Real-time updates** (SSE / websocket) for vault changes from CLI/MCP —
  v1 refreshes on next navigation only.
- **Sentry / client-side error tracking** — separate concern, separate task.
- **Global toast system** — not needed for v1 error UX.
- **Resizable columns** in the grid — nice-to-have, depends on SVAR's coverage.
- **A new `/api/search/text` endpoint with HuggingFace embeddings** — the
  prior session's planning notes mentioned this; verified unnecessary because
  the existing `POST /api/search` already accepts text-only queries
  (`compute_weights` returns `(1.0, 0.0)` for FTS-only mode).

## 10. Open Verification Items

Items that don't change the design but need verification during plan writing
or implementation:

1. **SVAR Svelte DataGrid exact package name + version.** Research confirmed
   v2.6 (March 2026), MIT, Svelte 5 compatible. Verify the npm package name
   when adding the dep.
2. **SVAR bundle size.** Not published as a precise gzip number. Measure when
   added to the build; if surprisingly heavy (>200KB), revisit. TanStack Table
   v9 + custom UI is the documented fallback.
3. **SVAR resizable columns support.** Nice-to-have only; verify presence
   during plan writing. Doesn't block implementation.
4. **`RuleHeading.svelte` existence in the dashboard branch.** May already
   exist; if so, reuse. If inlined, extract during this work.
5. **`_internal/` prefix convention.** Check whether the SvelteKit app already
   uses a different prefix for non-API server endpoints; align if so.
6. **Vercel `/api/*` rewrite scope.** Confirm exactly which paths are
   rewritten so we can guarantee `_internal/search/+server.ts` doesn't
   collide.

## 11. Acceptance Criteria

This PR is complete when:

- [ ] All Rust unit tests pass: `cargo nextest run -p temper-api --features test-db`
- [ ] All Rust integration tests pass for the new endpoints
- [ ] e2e test `vault_browse_test.rs` passes
- [ ] `cargo make check` passes (fmt, clippy, machete, ts typecheck, biome)
- [ ] `cargo make test-all` passes
- [ ] `bun run check && bun run build` pass for `packages/temper-ui`
- [ ] sqlx offline cache regenerated and committed
- [ ] All 12 manual smoke tests in §7.6 pass against a local dev stack
- [ ] No regressions in the existing `(app)/admin/access` flow
- [ ] No regressions in CLI/MCP resource operations (caught by ts-rs cascade
      + existing test suites)
- [ ] PR description references this spec doc and the original task
