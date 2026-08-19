# Atlas Interstitial — Task 1: Addressing Spine + Nav Integrity + Bridge — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a canonical vault-URL builder and route every nav/back link and the new Atlas rail "View full resource →" button through it — fixing team-view 500s, the resource back-link error, and the personal-context/active-state defects, while pruning dead nav.

**Architecture:** All `/vault/...` URLs currently get hand-rolled at each call site from whatever fields the caller holds, and several pick the wrong ones. This plan adds one typed module (`src/lib/vault-url.ts`) that is the single authority for vault route URLs, unit-tested in isolation, then rewires every call site to it. The URL-building *logic* is fully tested; component edits are verified via `svelte-check` + manual/harness checks (this repo has no Svelte component-test harness — only pure-function vitest specs).

**Tech Stack:** SvelteKit 2 / Svelte 5 (runes), TypeScript, Vitest, Tailwind v4, `ts-rs`-generated wire types.

## Global Constraints

- **Package dir:** all commands run from `packages/temper-ui`. Test runner: `bun run test` (vitest). Typecheck: `bun run check` (svelte-check). Lint/format: run repo pre-commit (biome) via the normal commit path.
- **`ownerRef` is already sigil'd** (`@<handle>` / `+<team-slug>`) and MUST NOT be percent-encoded — the `@`/`+` sigils are valid path chars the `[owner]` route matches literally (`VaultGrid.svelte:80` interpolates them raw today). Slugs and doc-types are defensively encoded.
- **`ResourceRow` carries no resource slug** — the full-resource path's final segment is the bare `id` (the route resolves trailing-UUID-only; this matches the current `VaultGrid.svelte:80` behavior). Do not attempt to build a decorated `<slug>-<id>` ref for resources.
- **`context_owner_ref` / `context_slug` are nullable** on `ResourceRow` (null for cogmap-homed resources). `resourceHref` returns `null` in that case; callers gate the affordance.
- **Do not touch** the legacy `src/lib/graph/navigation.ts:resourceHref(owner, context, node: GraphNode)` — it serves the old KnowledgeGraph/`ResourcePeek` surface and is out of scope. `vault-url.ts` is a separate authority for the vault routes.
- ID types (`ResourceId`/`ContextId`/`ProfileId`) are plain `string` aliases — test fixtures may use string literals.

## File Structure

- **Create** `src/lib/vault-url.ts` — the URL builder authority (`contextHref`, `contextGraphHref`, `resourceHref`, `searchHref`).
- **Create** `src/lib/vault-url.test.ts` — vitest unit spec for the builder.
- **Modify** `src/lib/components/ContextNavGroup.svelte` — build links via the builder off `ctx.owner_ref`/`ctx.slug`; fix active-state; drop `ownerPrefix`.
- **Modify** `src/lib/components/Sidebar.svelte` — drop `ownerPrefix` pass-through; remove Teams/Admin/Settings links; drop `isAdmin` prop.
- **Modify** `src/routes/(app)/+layout.svelte` — stop passing `isAdmin` to Sidebar.
- **Delete** `src/routes/(app)/teams/` and `src/routes/(app)/settings/` route directories.
- **Modify** `src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte` — back-link via `contextHref`, guarded for null context.
- **Modify** `src/lib/components/VaultGrid.svelte` and `src/lib/components/CommandPalette.svelte` — dedup the inline `/vault/...` strings through `resourceHref`/`searchHref`.
- **Modify** `src/lib/components/graph/atlas/TrailRail.svelte` — add the "View full resource →" button in the existing `.actions` section.

---

### Task 1: The `vault-url.ts` builder (pure, TDD)

**Files:**
- Create: `src/lib/vault-url.ts`
- Test: `src/lib/vault-url.test.ts`

**Interfaces:**
- Consumes: `ResourceRow` from `$lib/types/generated/resource`.
- Produces (relied on by all later tasks):
  - `contextHref(ownerRef: string, slug: string): string`
  - `contextGraphHref(ownerRef: string, slug: string): string`
  - `resourceHref(row: ResourceRow): string | null`
  - `searchHref(query: string): string`

- [ ] **Step 1: Write the failing test**

Create `src/lib/vault-url.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { contextHref, contextGraphHref, resourceHref, searchHref } from './vault-url';
import type { ResourceRow } from './types/generated/resource';

const ID = '019f420c-cf01-7bc1-87c9-09684b0fa69e';

function makeRow(partial: Partial<ResourceRow>): ResourceRow {
	return {
		id: ID,
		kb_context_id: '00000000-0000-0000-0003-000000000001',
		origin_uri: '',
		title: 'T',
		originator_profile_id: '00000000-0000-0000-0000-000000000001',
		owner_profile_id: '00000000-0000-0000-0000-000000000001',
		is_active: true,
		created: '2026-07-08T00:00:00Z',
		updated: '2026-07-08T00:00:00Z',
		context_name: 'Temper',
		doc_type_name: 'task',
		owner_handle: 'j-cole-taylor',
		context_slug: 'temper',
		context_owner_ref: '@j-cole-taylor',
		cogmap_id: null,
		cogmap_name: null,
		stage: null,
		seq: null,
		mode: null,
		effort: null,
		body_hash: null,
		...partial
	};
}

describe('contextHref', () => {
	it('builds /vault/{ownerRef}/{slug} without encoding the sigil', () => {
		expect(contextHref('@j-cole-taylor', 'temper')).toBe('/vault/@j-cole-taylor/temper');
		expect(contextHref('+acme-team', 'ops')).toBe('/vault/+acme-team/ops');
	});

	it('encodes the slug defensively', () => {
		expect(contextHref('@me', 'my context')).toBe('/vault/@me/my%20context');
	});
});

describe('contextGraphHref', () => {
	it('appends /graph to the context path', () => {
		expect(contextGraphHref('+acme-team', 'ops')).toBe('/vault/+acme-team/ops/graph');
	});
});

describe('resourceHref', () => {
	it('builds the full resource path for a context-homed resource', () => {
		expect(resourceHref(makeRow({}))).toBe(`/vault/@j-cole-taylor/temper/task/${ID}`);
	});

	it('uses the exact doc_type and the bare id (no decorated ref)', () => {
		expect(resourceHref(makeRow({ doc_type_name: 'session' }))).toBe(
			`/vault/@j-cole-taylor/temper/session/${ID}`
		);
	});

	it('returns null for a cogmap-homed resource (null context fields)', () => {
		expect(
			resourceHref(
				makeRow({ context_owner_ref: null, context_slug: null, cogmap_id: 'x', cogmap_name: 'Map' })
			)
		).toBe(null);
	});
});

describe('searchHref', () => {
	it('encodes the query', () => {
		expect(searchHref('auth flow')).toBe('/vault/search?q=auth%20flow');
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun run test -- vault-url`
Expected: FAIL — cannot resolve `./vault-url` (module does not exist yet).

- [ ] **Step 3: Write the minimal implementation**

Create `src/lib/vault-url.ts`:

```ts
import type { ResourceRow } from '$lib/types/generated/resource';

/**
 * Single authority for `/vault/...` route URLs. Every nav link, back link,
 * row-click, and the Atlas rail's "View full resource" button routes through
 * these builders so addressing can never drift per call site.
 *
 * `ownerRef` is already sigil'd (`@<handle>` / `+<team-slug>`) and is NOT
 * percent-encoded — the sigils are valid path chars the `[owner]` route matches
 * literally. `slug` and `docType` are encoded defensively.
 */

export function contextHref(ownerRef: string, slug: string): string {
	return `/vault/${ownerRef}/${encodeURIComponent(slug)}`;
}

export function contextGraphHref(ownerRef: string, slug: string): string {
	return `${contextHref(ownerRef, slug)}/graph`;
}

/**
 * Full-resource path for a context-homed resource. Returns `null` for a
 * cogmap-homed resource (its `context_*` fields are null) so callers can gate
 * the affordance rather than emit a broken URL. The final segment is the bare
 * `id`: the route resolves trailing-UUID-only, and `ResourceRow` carries no
 * resource slug to decorate with.
 */
export function resourceHref(row: ResourceRow): string | null {
	if (!row.context_owner_ref || !row.context_slug) return null;
	return `/vault/${row.context_owner_ref}/${encodeURIComponent(row.context_slug)}/${encodeURIComponent(row.doc_type_name)}/${row.id}`;
}

export function searchHref(query: string): string {
	return `/vault/search?q=${encodeURIComponent(query)}`;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `bun run test -- vault-url`
Expected: PASS (4 describe blocks, 6 tests green).

- [ ] **Step 5: Typecheck**

Run: `bun run check`
Expected: no new errors.

- [ ] **Step 6: Commit**

```bash
git add src/lib/vault-url.ts src/lib/vault-url.test.ts
git commit -m "feat(temper-ui): canonical vault-url builder (contextHref/resourceHref/searchHref)"
```

---

### Task 2: Nav integrity — route context links through the builder

**Files:**
- Modify: `src/lib/components/ContextNavGroup.svelte`
- Modify: `src/lib/components/Sidebar.svelte:67-73` (the two `ContextNavGroup` usages)

**Interfaces:**
- Consumes: `contextHref`, `contextGraphHref` from `$lib/vault-url` (Task 1).
- Produces: nothing new; corrects existing markup.

This fixes team-view 500s (was `/vault/+team/{name}`), the team-graph 500, personal-context links (was `@me/{name}`), and the active-state name/slug mismatch — all by using `ctx.owner_ref`/`ctx.slug` instead of a literal `ownerPrefix` + `ctx.name`.

- [ ] **Step 1: Rewrite `ContextNavGroup.svelte`**

Replace the entire file contents with:

```svelte
<script lang="ts">
	import { page } from '$app/stores';
	import { contextHref, contextGraphHref } from '$lib/vault-url';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		label: string;
		contexts: ContextRowWithCounts[];
	}

	let { label, contexts }: Props = $props();

	function isActive(ctx: ContextRowWithCounts): boolean {
		return $page.params.owner === ctx.owner_ref && $page.params.context === ctx.slug;
	}

	function isGraphActive(ctx: ContextRowWithCounts): boolean {
		return isActive(ctx) && $page.url.pathname.endsWith('/graph');
	}
</script>

<div class="px-3 pt-4 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">
	{label}
</div>
{#each contexts as ctx}
	<a
		href={contextHref(ctx.owner_ref, ctx.slug)}
		class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
		       {isActive(ctx)
			? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
			: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
	>
		<span
			class="w-1.5 h-1.5 rounded-sm {isActive(ctx) ? 'bg-quiet-accent' : 'bg-zinc-600'}"
		></span>
		<span class="flex-1 truncate">{ctx.name}</span>
		<span class="text-xs text-zinc-600">{ctx.resource_count}</span>
	</a>
	{#if isActive(ctx)}
		<a
			href={contextGraphHref(ctx.owner_ref, ctx.slug)}
			class="flex items-center gap-2 pl-8 pr-3 py-1.5 text-sm transition-colors
			       {isGraphActive(ctx)
				? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(2rem-2px)]'
				: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
		>
			<span
				class="w-1.5 h-1.5 rounded-sm {isGraphActive(ctx) ? 'bg-quiet-accent' : 'bg-zinc-600'}"
			></span>
			<span class="flex-1 truncate">Graph</span>
		</a>
	{/if}
{/each}
```

(Note: display text stays `{ctx.name}` — the human-readable name is correct for the label; only the *hrefs* and *active test* move to `owner_ref`/`slug`.)

- [ ] **Step 2: Update the two `ContextNavGroup` usages in `Sidebar.svelte`**

At `Sidebar.svelte:67-73`, drop the `ownerPrefix` attribute from both:

```svelte
			{#if myContexts.length > 0}
				<ContextNavGroup label="Contexts" contexts={myContexts} />
			{/if}

			{#if teamContexts.length > 0}
				<ContextNavGroup label="Teams" contexts={teamContexts} />
			{/if}
```

- [ ] **Step 3: Typecheck**

Run: `bun run check`
Expected: no errors (the removed `ownerPrefix` prop is no longer required or passed).

- [ ] **Step 4: Manual verification (dev server)**

Run: `bun run dev`, then in the browser (authed):
- Expand a **team** context in the sidebar → it navigates to `/vault/+<team-slug>/<slug>` and the resource list renders (no 500).
- The team's **Graph** sublink → renders (no 500).
- A **personal** context → navigates to `/vault/@<handle>/<slug>`, list renders, and the item shows the active highlight.

Expected: all three succeed; previously team links 500'd.

- [ ] **Step 5: Commit**

```bash
git add src/lib/components/ContextNavGroup.svelte src/lib/components/Sidebar.svelte
git commit -m "fix(temper-ui): build sidebar context links from owner_ref/slug (fixes team-view 500s + active-state)"
```

---

### Task 3: Prune the left nav — remove Teams/Admin/Settings; delete placeholder routes

**Files:**
- Modify: `src/lib/components/Sidebar.svelte` (bottom group `76-96`; `isAdmin` prop at `9,16`)
- Modify: `src/routes/(app)/+layout.svelte:35` (stop passing `isAdmin`)
- Delete: `src/routes/(app)/teams/` and `src/routes/(app)/settings/`

**Interfaces:**
- Consumes: nothing from prior tasks.
- Produces: `Sidebar` no longer accepts an `isAdmin` prop.

Per the approved decision "delete placeholders, keep admin": remove the three nav links and the two placeholder routes; keep `/admin/access` reachable by direct URL (unlinked).

- [ ] **Step 1: Remove the three links in `Sidebar.svelte`**

In the bottom `<div class="border-t border-zinc-800 py-2">` block, delete the **Teams** (`/teams`), **Admin** (`{#if isAdmin} … /admin/access …`), and **Settings** (`/settings`) anchors. The block should retain only Sign out + the user display:

```svelte
		<div class="border-t border-zinc-800 py-2">
			<a
				href="/auth/logout"
				class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200"
			>
				<span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Sign out
			</a>
			{#if user}
				<div class="flex items-center gap-2 px-3 py-2 text-xs text-zinc-500">
					<div class="w-5 h-5 rounded-full bg-zinc-700 flex-shrink-0"></div>
					{user.display_name}
				</div>
			{/if}
		</div>
```

- [ ] **Step 2: Drop the now-unused `isAdmin` prop from `Sidebar.svelte`**

In the `<script>` block, remove `isAdmin` from the `Props` interface and from the `$props()` destructure:

```svelte
	interface Props {
		contexts: ContextRowWithCounts[];
		user: { display_name: string; email: string } | null;
		/** Operator-configured instance brand ("temper @ acme"); null → default. */
		instanceName?: string | null;
		collapsed: boolean;
		onToggle: () => void;
	}

	let { contexts, user, instanceName = null, collapsed, onToggle }: Props = $props();
```

- [ ] **Step 3: Stop passing `isAdmin` in `+layout.svelte`**

Delete the `isAdmin={data.entitlements?.is_admin ?? false}` line (currently `+layout.svelte:35`) from the `<Sidebar … />` invocation. (Leave `data.entitlements` loading untouched — other code may read it; only the prop pass is removed.)

- [ ] **Step 4: Delete the placeholder routes**

```bash
git rm -r 'src/routes/(app)/teams' 'src/routes/(app)/settings'
```

- [ ] **Step 5: Typecheck**

Run: `bun run check`
Expected: no errors; no "unused prop" or "missing prop" diagnostics for `isAdmin`.

- [ ] **Step 6: Manual verification (dev server)**

Run: `bun run dev` (if not already running) and confirm in the browser: the sidebar bottom group shows only **Sign out** + the user; navigating to `/teams` or `/settings` now 404s; `/admin/access` still loads by direct URL.

- [ ] **Step 7: Commit**

```bash
git add src/lib/components/Sidebar.svelte 'src/routes/(app)/+layout.svelte'
git commit -m "chore(temper-ui): remove Teams/Admin/Settings from left nav; delete placeholder routes"
```

---

### Task 4: Fix the resource detail back-link

**Files:**
- Modify: `src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte:12-19`

**Interfaces:**
- Consumes: `contextHref` from `$lib/vault-url` (Task 1).

The current back-link uses `owner_handle`/`context_name` (a bare handle + display name) → mis-shaped and wrong for team-owned resources. Route it through `contextHref` off `context_owner_ref`/`context_slug`, guarded for the (rare here) null-context case.

- [ ] **Step 1: Update the page**

Replace the file contents with:

```svelte
<script lang="ts">
	import type { PageData } from './$types';
	import ResourceMetaHeader from '$lib/components/ResourceMetaHeader.svelte';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import { contextHref } from '$lib/vault-url';

	let { data }: { data: PageData } = $props();

	// context_* are null only for a cogmap-homed resource; this context-shaped
	// route is reached for context-homed resources, but guard defensively.
	let backHref = $derived(
		data.resource.context_owner_ref && data.resource.context_slug
			? contextHref(data.resource.context_owner_ref, data.resource.context_slug)
			: null
	);
</script>

<svelte:head>
	<title>{data.resource.title} — temper</title>
</svelte:head>

<div class="p-6 max-w-4xl">
	{#if backHref}
		<div class="mb-6">
			<a
				href={backHref}
				class="text-xs font-mono tracking-wide text-zinc-500 hover:text-zinc-300 transition-colors"
			>
				&larr; {data.resource.context_name ?? data.resource.context_slug}
			</a>
		</div>
	{/if}

	<div class="mb-8">
		<ResourceMetaHeader resource={data.resource} />
	</div>

	<MarkdownRenderer markdown={data.content} />
</div>
```

- [ ] **Step 2: Typecheck**

Run: `bun run check`
Expected: no errors.

- [ ] **Step 3: Manual verification (dev server)**

In the browser, open a resource in a **personal** context and click "← back" → lands on that context's list. Repeat for a **team-owned** resource (open one via `/vault/all` row-click, then back) → lands on the team context list (previously errored).

- [ ] **Step 4: Commit**

```bash
git add 'src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte'
git commit -m "fix(temper-ui): resource back-link via context_owner_ref/context_slug (fixes team-owned + mis-shaped links)"
```

---

### Task 5: Dedup inline `/vault/...` strings through the builder

**Files:**
- Modify: `src/lib/components/VaultGrid.svelte:80`
- Modify: `src/lib/components/CommandPalette.svelte` (row nav `~59` and `~103`; search links `~65` and `~120`)

**Interfaces:**
- Consumes: `resourceHref`, `searchHref` from `$lib/vault-url` (Task 1).

Belt-and-suspenders: the same class of bug (wrong/duplicated fields) is prevented at the two remaining correct-but-duplicated sites. `resourceHref` returns `string | null`; guard before navigating.

- [ ] **Step 1: `VaultGrid.svelte` — import + use the builder**

Add the import to the `<script>`:

```ts
	import { resourceHref } from '$lib/vault-url';
```

Replace the `goto(...)` in `handleFocusCell` (line 80):

```ts
	function handleFocusCell(ev: {
		row?: string | number;
		column?: string | number;
		eventSource?: string;
	}) {
		if (ev.eventSource !== 'click' || !ev.row) return;
		const row = rowLookup.get(String(ev.row));
		if (!row) return;
		const href = resourceHref(row);
		if (href) goto(href);
	}
```

- [ ] **Step 2: `CommandPalette.svelte` — import + use the builder**

Add to the `<script>`:

```ts
	import { resourceHref, searchHref } from '$lib/vault-url';
```

In `onKeydown`'s Enter branch, replace the two inline builds:

```ts
		} else if (e.key === 'Enter') {
			e.preventDefault();
			if (focused < results.length) {
				const href = resourceHref(results[focused]);
				if (href) goto(href);
				open = false;
			} else if (query.trim()) {
				goto(searchHref(query));
				open = false;
			}
		}
```

In the result-row `onclick` (the `{#each results as row, i}` button):

```svelte
						onclick={() => {
							const href = resourceHref(row);
							if (href) goto(href);
							open = false;
						}}
```

In the "See all {total} results" button `onclick`:

```svelte
					onclick={() => {
						goto(searchHref(query));
						open = false;
					}}
```

- [ ] **Step 3: Typecheck**

Run: `bun run check`
Expected: no errors.

- [ ] **Step 4: Manual verification (dev server)**

In the browser: row-click in `/vault/all` still opens the resource; `⌘K` command palette → Enter on a result opens the resource, Enter on empty-selection (with a query) goes to `/vault/search?q=…`, and "See all results" navigates to search. Behavior unchanged from before.

- [ ] **Step 5: Commit**

```bash
git add src/lib/components/VaultGrid.svelte src/lib/components/CommandPalette.svelte
git commit -m "refactor(temper-ui): route VaultGrid + CommandPalette nav through vault-url builder"
```

---

### Task 6: The Atlas rail "View full resource →" bridge button

**Files:**
- Modify: `src/lib/components/graph/atlas/TrailRail.svelte` (script `1-64`; `.actions` section `73-77`; styles near `255-275`)

**Interfaces:**
- Consumes: `resourceHref` from `$lib/vault-url` (Task 1); the already-loaded `resourceRow` prop.

Adds the bridge from a selected node to its full resource. Renders when `resourceRow` is present and context-homed (`resourceHref` non-null). A cogmap-homed node keeps the drill button but no view button.

- [ ] **Step 1: Add the import and a derived href in `TrailRail.svelte`**

Add to the imports:

```ts
	import { resourceHref } from '$lib/vault-url';
```

Add a derived value alongside `canDrill` (near line 59):

```ts
	// Bridge to the full resource page. null for cogmap-homed nodes (no context
	// route) and for edges — gate the button on it.
	const viewHref = $derived(isNode && resourceRow ? resourceHref(resourceRow) : null);
```

- [ ] **Step 2: Render the button in the existing `.actions` section**

Replace the `{#if canDrill}` actions block (lines 73-77) with a block that shows either/both actions:

```svelte
		{#if canDrill || viewHref}
			<section class="actions">
				{#if canDrill}
					<button class="drill-in" onclick={drillIn}>Drill into neighborhood →</button>
				{/if}
				{#if viewHref}
					<a class="view-resource" href={viewHref} data-testid="view-full-resource"
						>View full resource →</a
					>
				{/if}
			</section>
		{/if}
```

- [ ] **Step 3: Add `.view-resource` styling**

After the `.drill-in:hover { … }` rule (near line 275), add:

```css
	.view-resource {
		display: block;
		width: 100%;
		margin-top: 6px;
		text-align: left;
		background: transparent;
		border: 1px solid color-mix(in srgb, var(--hue) 35%, transparent);
		border-radius: 6px;
		padding: 7px 10px;
		color: color-mix(in srgb, var(--hue) 80%, white);
		font-size: 12px;
		letter-spacing: 0.02em;
		text-decoration: none;
		cursor: pointer;
	}
	.view-resource:hover {
		background: color-mix(in srgb, var(--hue) 12%, transparent);
		border-color: color-mix(in srgb, var(--hue) 55%, transparent);
	}
```

(When `canDrill` is false the `margin-top` still reads fine as the only action; when both render, the view button sits under the drill button.)

- [ ] **Step 4: Typecheck**

Run: `bun run check`
Expected: no errors.

- [ ] **Step 5: Verify in the `/dev/atlas` render harness**

Run: `bun run dev`, open `/dev/atlas` (the fixture-driven render harness — bypasses the loader, per the team's UI verification convention). Select a **context-homed** node → the rail shows "View full resource →" pointing at `/vault/<owner_ref>/<slug>/<doc_type>/<id>`; clicking it lands on the resource reader. Select a **cogmap-homed facet** node → only "Drill into neighborhood →" shows (no view button). Check both light and dark themes.

If `/dev/atlas` lacks a fixture with a context-homed `resourceRow`, verify against prod post-merge instead (Auth0-gated flows can't run on Vercel previews) and note it in the session.

- [ ] **Step 6: Commit**

```bash
git add src/lib/components/graph/atlas/TrailRail.svelte
git commit -m "feat(temper-ui): 'View full resource' bridge button in Atlas rail"
```

---

## Final verification (after all tasks)

- [ ] `bun run test` — all vitest specs green (including `vault-url`).
- [ ] `bun run check` — svelte-check clean.
- [ ] Full pre-commit passes on the last commit (biome + the repo's Rust gates are untouched by this UI-only change).
- [ ] Sidebar: personal + team contexts navigate correctly with active highlight; no Teams/Admin/Settings; Sign out intact.
- [ ] Resource back-link works for personal and team-owned resources.
- [ ] Atlas rail shows "View full resource →" for context-homed nodes and lands on the reader.

## Self-Review Notes (author)

- **Spec coverage:** WS1 (bridge) → Task 6; WS2 (nav integrity: team-view fix → Task 2; back-link → Task 4; prune nav + delete routes → Task 3; dedup → Task 5); the spine (`vault-url.ts`) → Task 1. WS3 is a separate future plan (Task 2 of the roadmap), out of scope here.
- **Type consistency:** builder signatures in Task 1 (`contextHref`, `contextGraphHref`, `resourceHref → string | null`, `searchHref`) are used verbatim in Tasks 2, 4, 5, 6. Null-return of `resourceHref` is guarded at every consumer.
- **No component-test fabrication:** only `vault-url.ts` gets vitest TDD (it is pure); component edits are verified via `svelte-check` + dev/harness, matching this repo's actual testing surface.
