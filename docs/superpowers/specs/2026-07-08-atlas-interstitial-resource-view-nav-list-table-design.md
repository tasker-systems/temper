# Atlas Interstitial (Beat D↔E) — resource-view bridge, navigation integrity, list/table/search rebuild

**Task:** `019f420c-cf01-7bc1-87c9-09684b0fa69e` (plan/large, context `@me/temper`)
**Goal:** `019f28a1-…` — Graph Atlas visualization
**Date:** 2026-07-08
**Status:** design approved, pending spec review

## Context

This is the bridge between the Atlas (spatial graph, Beat D shipped + prod-verified) and the
reading/working surfaces. It was surfaced during the Beat D prod review as a named body of work
sitting between Beat D and the future Beat E (context-surface re-imagining).

Discovery reframed the task. The original framing ("reach the full resource", "rebuild the
placeholder list/table views") turned out to be partly already true:

- The **resource route** `/vault/[owner]/[context]/[doc_type]/[ident]` already exists and renders a
  real reader view (`ResourceMetaHeader` + `MarkdownRenderer`); `[ident]` resolves trailing-UUID-only.
- The **list/table views** (`/vault/[owner]/[context]`, `/vault/all`, `/vault/search`) already render
  a real `VaultGrid` (SVAR data grid: Title/Context/Type/Stage/Updated, URL-driven sort, pagination,
  doc-type facet chips, row-click → resource). They are **not** stubs — the "scaffold feel" is in the
  thin page wrappers, not the grid.
- The **build-circle → context list** landing already works (Beat D).

So the real work is: (1) a small **bridge** from an Atlas node to its full resource, (2) a
**navigation-integrity** pass (several nav/back links error out; team views throw; dead nav to
prune), and (3) a **focused rebuild** of the list/table/search surfaces (filtering, density,
coherence, and real excerpt-bearing search).

### Boundary with Beat E

Confirmed with the user: this task is **"bridge + focused polish."** The large **context-surface
re-imagining** (cross-view IA rethink, saved views, etc.) stays in Beat E. This task delivers the
bridge, correct navigation, and a genuinely usable — not re-imagined-from-scratch — list/table/search
treatment.

## The spine: a canonical vault-URL builder

Every navigation bug reported and the new bridge button share **one root cause**: there is no
canonical URL builder. Each call site hand-rolls `/vault/...` from whatever fields it happens to
hold, and several pick the *wrong* fields:

| Symptom | Site | Defect |
|---|---|---|
| Team views 500 | `ContextNavGroup.svelte:27` via `Sidebar.svelte:72` | builds `/vault/+team/{ctx.name}` from a **literal `+team`** + display **name**; API can't resolve `+team/<name>` and the load has no `.catch()` → 500 |
| Team graph link 500 | `ContextNavGroup.svelte:41` | same defect on the per-context graph sublink |
| Personal links fragile | `ContextNavGroup.svelte:27` via `Sidebar.svelte:68` | `@me` + `ctx.name` instead of `ctx.owner_ref` + `ctx.slug` |
| Active-state wrong | `ContextNavGroup.svelte:14` | compares `$page.params.context` (a **slug**) against `ctx.name` |
| Resource back-link errors | `…/[ident]/+page.svelte:16` | uses `owner_handle`/`context_name` instead of `context_owner_ref`/`context_slug`; mis-shaped and wrong for team-owned resources |
| Duplicated string (3×) | `VaultGrid.svelte:80`, `CommandPalette.svelte:61,103` | correct fields, but the `/vault/.../{id}` string is hand-built and copy-pasted |

The correct addressing fields already exist on the types:

- `ContextRowWithCounts` (`types/generated/context.ts`) carries `owner_ref` (already sigil'd:
  `@<handle>` / `+<team-slug>`) and `slug`.
- `ResourceRow` (`types/generated/resource.ts`) carries `context_owner_ref`, `context_slug`,
  `doc_type_name`, `id` (and `cogmap_id`/`cogmap_name` for cogmap-homed resources).

**Fix by construction.** Introduce a single typed module `src/lib/vault-url.ts`:

```ts
// Build /vault/... paths from canonical, already-sigil'd addressing fields.
// The one place that knows the route shape. All call sites route through here.

export function contextHref(ownerRef: string, slug: string): string;
// → `/vault/${ownerRef}/${encodeURIComponent(slug)}`

export function contextGraphHref(ownerRef: string, slug: string): string;
// → `${contextHref(ownerRef, slug)}/graph`

export function resourceHref(row: ResourceRow): string | null;
// context-homed → `/vault/${context_owner_ref}/${context_slug}/${doc_type_name}/${id}`
// cogmap-homed (context_* null) → null   (see Open Questions — gated, no full-resource route yet)

export function searchHref(query: string): string;
// → `/vault/search?q=${encodeURIComponent(query)}`
```

`resourceHref` returns `null` for cogmap-homed resources so callers can gate the affordance rather
than emit a broken URL. The existing legacy `src/lib/graph/navigation.ts:resourceHref(owner, context,
node: GraphNode)` stays put — it serves the old `KnowledgeGraph`/`ResourcePeek` surface at
`/vault/[owner]/[context]/graph` and is typed for the legacy `GraphNode`; it is out of scope here.

This spine is why WS1 (bridge) and WS2 (nav integrity) are one chunk: they are the same fix applied
at every call site.

## WS1 — The node → resource bridge *(small)*

From a selected Atlas node's rail, add a **"View full resource →"** action landing on the resource's
own page.

- The `TrailRail` already receives `resourceRow: ResourceRow | null` (loaded server-side in
  `graph/[owner]/+page.server.ts` on node selection) and already renders CONTEXT/COGMAP/STAGE from it.
  **No new fetch.**
- Add a button in the existing `.actions` section (`TrailRail.svelte:73–77`, beside "Drill into
  neighborhood →"). `href = resourceHref(resourceRow)`.
- **Gate:** render the button only when `resourceRow` is present and `resourceHref` returns non-null
  (context-homed). Cogmap-homed nodes keep the excerpt/neighbors rail without the button (see Open
  Questions).
- Style to match the existing `.drill-in` button.

## WS2 — Navigation integrity *(small–medium, bug-fix)*

All fixes route the offending links through `vault-url.ts`.

1. **`ContextNavGroup.svelte`** — build the context link with `contextHref(ctx.owner_ref, ctx.slug)`
   (drop the `ownerPrefix` literal + `ctx.name`); build the graph sublink with
   `contextGraphHref(...)`; fix the active-state test to compare against `ctx.slug`. This fixes team
   views, team graph links, and personal-context links in one edit.
2. **`Sidebar.svelte`** — remove the now-unused `ownerPrefix` plumbing (68, 72) once
   `ContextNavGroup` reads `owner_ref`/`slug` off the row directly.
3. **Resource back-link** — `…/[ident]/+page.svelte:16` → `contextHref(resource.context_owner_ref,
   resource.context_slug)`.
4. **Dedup** — `VaultGrid.svelte:80` and `CommandPalette.svelte:61,103` → `resourceHref(row)`;
   `CommandPalette` search links → `searchHref(q)`. (Belt-and-suspenders: catches the same class of
   bug if these types ever drift.)
5. **Prune the left nav** — remove the **Teams**, **Admin**, and **Settings** links from
   `Sidebar.svelte:76–96`; drop the now-unused `isAdmin` prop (`Sidebar.svelte:9,16` and the pass at
   `+layout.svelte:35`). Keep the Sign-out link and its bordered wrapper.
6. **Route files** (per user decision "delete placeholders, keep admin"):
   - **Delete** `src/routes/(app)/teams/` and `src/routes/(app)/settings/` (both are "coming soon"
     placeholders).
   - **Keep** `src/routes/(app)/admin/access/` — a real, functional console; now reachable by direct
     URL only (unlinked from nav).

### WS2 validation

- Team context in the sidebar → renders the team's resource list (no 500).
- Team graph sublink → renders (no 500).
- Personal context link + active-highlight correct.
- Resource detail "← back to context" → lands on the context list for both personal- and team-owned
  resources.
- Nav shows no Teams/Admin/Settings; Sign-out intact; `/admin/access` still loads by URL.
- `bun run check` clean (no unused `isAdmin`).

## WS3 — List / table / search rebuild *(medium–large)*

### Table views (`/vault/[owner]/[context]`, `/vault/all`)

- **Filtering:** add a **stage** filter and **multi-select** doc-type facets (today `FacetChips` is
  single-active, doc-type only); add an **inline text/title filter** box in the list header. All
  URL-param-driven, matching the existing `?sort`/`?order`/`?offset`/`?doc_type_name` pattern so
  state stays shareable/back-button-safe.
- **Columns & density:** type-aware columns — surface `seq`/`mode`/`effort` when the filtered set is
  tasks, `owner` for team contexts; add a compact/dense row toggle. Keep `VaultGrid` as the engine;
  make its `columns` a function of the active doc-type filter.
- **View coherence / chrome:** a shared header treatment (title + count + active-filter summary bar +
  clear-all) so `context` / `all` / `search` read as one deliberate surface instead of three
  near-identical thin wrappers. Extract the shared chrome into one component consumed by all three.

### Search (`/vault/search`) — real search + excerpt list *(per user decision)*

The current page is not search — it filters `/api/resources` (browse) by title and renders a bare
table with no excerpt. Rewire it:

- **Endpoint:** `POST /api/search` (exists — `routes.rs:223`, `handlers/search.rs`), returning
  unified FTS+vector results. Add a typed client call in `src/lib/server/`.
- **Wire type:** consume `UnifiedSearchResultRow` (`types/generated/search.ts`) — carries
  `title`, `doc_type`, `snippet`, scores, `context_slug`, `context_owner_ref`. (`SearchResultRow`
  also carries `snippet`; use whichever the `/api/search` handler returns — confirm during planning.)
- **Render:** a **TrailRail-style excerpt list** (not the dense grid): per result, title + **doc-type
  badge** + **snippet** + a light score/context line, each linking via `resourceHref`-equivalent
  built from the search row's `context_owner_ref`/`context_slug`/`doc_type`/`resource_id`. This is the
  direct answer to "search isn't clear about what kind of doc you're getting back."
- Keep the `EmptyState` for no-results; keep `q` URL-param driven.

> **Note:** the search row and `ResourceRow` are different shapes. Either add a
> `resourceHref` overload that accepts the search row's fields, or map the search row into the same
> addressing fields before calling the builder. Decide during planning — keep one builder authority.

### WS3 validation

- Table: stage + multi-facet + text filters compose and survive reload/back-button; type-aware
  columns switch with the doc-type filter; density toggle works; three views share one chrome.
- Search: a query returns real FTS+vector hits with visible doc-type + snippet; clicking a result
  lands on the correct resource (personal and team-owned); empty query / no results handled.
- Both light/dark themes; `bun run check` + biome clean.

## Sequencing (roadmap)

1. **Task 1 — Addressing spine + nav integrity + bridge (WS1 + WS2).** `build` / `medium`. Delivers
   `vault-url.ts`, all nav/back-link fixes, nav pruning + route deletions, and the rail button. High
   value, ships fast, makes navigation correct everywhere. Unblocks WS3 (which reuses the builder).
2. **Task 2 — List/table/search rebuild (WS3).** `build` / `large`. May warrant a short visual
   brainstorm for the search excerpt-list and the shared chrome before implementation.

Dependency: Task 1 → Task 2 (Task 2 consumes `vault-url.ts` and the corrected addressing).

## Verification approach

- Local: `cd packages/temper-ui && bun run dev`, browser-drive the flows; `bun run check`
  (svelte-check) + biome.
- Atlas rail button (WS1): exercise via the `/dev/atlas` render harness (fixtures from `__data.json`)
  rather than merge→prod→eyeball.
- Auth-gated routing (WS2 team views) cannot be verified on Vercel PR previews (Auth0 callback
  allowlist is prod-only) — browser-verify authed nav in prod post-merge, as with prior Atlas beats.

## Out of scope

### Rejected (load-bearing — deliberately not doing)

- **Context-surface re-imagining** (cross-view IA rethink, saved/named views, layout re-design).
  That is Beat E; conflating it here would balloon the task and blur the boundary the user set.
- **A second URL-builder authority.** The legacy `graph/navigation.ts:resourceHref` stays for the old
  KnowledgeGraph surface but is not extended or reused; `vault-url.ts` is the single authority for the
  vault routes. Do not fork addressing logic.

### Deferred (later, not rejected)

- **Cogmap-homed resource addressing** — whether a cogmap node has (or should have) a full-resource
  vault page. Until resolved, the bridge button is gated to context-homed resources.
- **Backend search tuning** (ranking weights, graph-score exposure) — WS3 consumes `/api/search` as
  is; tuning is a separate concern.

## Open questions (resolve during planning)

1. **Cogmap-homed bridge:** does `/vault/[owner]/[context]/[doc_type]/[ident]` (context-shaped) have
   any landing for a cogmap-homed resource? If not, confirm the gate (button hidden) is acceptable, or
   design a cogmap-scoped resource view (likely Beat E).
2. **Search wire type:** which exact shape does `POST /api/search` return to the client
   (`UnifiedSearchResultRow` vs `SearchResultRow`)? Confirm against `handlers/search.rs` and keep the
   TS consumer on the generated type.
3. **`resourceHref` for search rows:** overload vs. field-mapping — pick the one that keeps a single
   builder authority.
4. **Density/columns budget:** final column set per doc-type and the dense-row line height (a small
   visual-brainstorm item at the top of Task 2).
