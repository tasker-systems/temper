# Graph Atlas C3.1 Beat 2a — Shell, Wayfinding & Legibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize the Atlas shell (collapsible vault rail, top bar + bottom bar, no dock), add a depth-aware clickable breadcrumb with single-level ascend, relocate filters to a popover, and fix three legibility gaps (empty territories, aggregate bridges, Tier-2 labels) — all in `packages/temper-ui` plus one thin backend field.

**Architecture:** The Atlas route (`/graph/[owner]`) keeps its URL-as-state model (`nav.ts`). We retire the 232px left dock, move search+breadcrumb to a top bar and the legend to a collapsible bottom bar, and collapse the shared vault sidebar to an icon rail. `focus` becomes a comma-joined path so the crumb is depth-aware and `↑` pops one level. The territory hop is named by extending the existing, already-visibility-gated R3 region-slice read with the region `label` (no migration, no new endpoint/e2e surface).

**Tech Stack:** SvelteKit (Svelte 5 runes), TypeScript, Tailwind v4, Vitest (pure-logic units), Rust (temper-core types + temper-services read), ts-rs codegen.

## Global Constraints

- **Frontend lives in `packages/temper-ui`; Rust in `crates/temper-core` + `crates/temper-services`.** One PR.
- **Test pattern = pure-logic units only.** No `.svelte` component tests exist; do not add a component-test harness. Each new/changed pure function gets a colocated `*.test.ts` (mirrors `nav.test.ts`, `legend.test.ts`, `layout/*.test.ts`). `.svelte` wiring is verified by `bun run check` + build, not unit tests.
- **URL is the state model.** No new Svelte stores for Atlas view state; scope/focus/filters stay in `$page.url` via `nav.ts` builders. (The one new store is the *shared vault sidebar* collapse bit — app-shell chrome, not Atlas state.)
- **History policy (from Beat 1):** scope/drill transitions PUSH; ephemeral view state (filters, `?sel`, panel close) REPLACE. Preserve this at every call site.
- **No migration.** `kb_cogmap_regions.label` already exists. The R3 read uses runtime `sqlx::query_scalar`/`query_as` (not the `query!` macro), so **no `.sqlx` regen**.
- **After any Rust type change:** run `cargo make generate-ts-types` and commit the regenerated `packages/temper-ui/src/lib/types/generated/graph_territory.ts`.
- **Dark-only**, palette-sourced. Never re-declare hex that `palette.ts` already owns.
- **Verification is prod-only** (Vercel previews lack Auth0): browser-verify on temperkb.io/graph/@me after merge.
- **Gates per task:** `cd packages/temper-ui && bun run check` (svelte-check) + `bun run test` (vitest) green; for Rust tasks `cargo make check` green. Frequent commits (one per task).

---

## File Structure

**Create:**
- `packages/temper-ui/src/lib/graph/atlas/crumbModel.ts` — pure crumb-segment derivation (+ `.test.ts`)
- `packages/temper-ui/src/lib/graph/atlas/labels.ts` — pure `labelAnchors` + `truncateLabel` (+ `.test.ts`)
- `packages/temper-ui/src/lib/graph/atlas/layout/bridges.ts` — pure `bridgeGeometry` (+ `.test.ts`)
- `packages/temper-ui/src/lib/graph/atlas/territory.ts` — pure `isEmptyTerritory` (+ `.test.ts`)
- `packages/temper-ui/src/lib/stores/sidebar.svelte.ts` — shared vault-rail collapse state + pure `defaultCollapsed(pathname)` (+ `.test.ts`)
- `packages/temper-ui/src/lib/components/graph/atlas/AtlasCrumb.svelte` — depth-aware breadcrumb + `↑`
- `packages/temper-ui/src/lib/components/graph/atlas/FilterPopover.svelte` — filters popover + badge
- `packages/temper-ui/src/lib/components/graph/atlas/marks/BridgeRibbon.svelte` — aggregate bridge mark

**Modify:**
- `crates/temper-core/src/types/graph_territory.rs` — `TerritorySlice.label`
- `crates/temper-services/src/services/graph_service.rs` — `territory_slice` returns label
- `packages/temper-ui/src/lib/types/generated/graph_territory.ts` — regenerated
- `packages/temper-ui/src/lib/graph/atlas/nav.ts` (+ `nav.test.ts`) — focus-as-path
- `packages/temper-ui/src/lib/components/Sidebar.svelte` — icon-rail mode
- `packages/temper-ui/src/routes/(app)/+layout.svelte` — collapse wiring
- `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.svelte` — retire dock, top/bottom bars
- `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` — `crumbTerritory` threading
- `packages/temper-ui/src/lib/components/graph/atlas/AtlasLegend.svelte` — bottom bar, default collapsed
- `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte` — ghost empties + bridges
- `packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte` — real region label
- `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte` — anchor+hover labels
- `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeChip.svelte` — persistent-vs-hover label

**Retire (deleted in Task 8):**
- `packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte`
- `packages/temper-ui/src/lib/components/graph/atlas/CogmapCrumb.svelte`

---

## Task 1: Backend — region label on the R3 region-slice read

**Files:**
- Modify: `crates/temper-core/src/types/graph_territory.rs:115-119`
- Modify: `crates/temper-services/src/services/graph_service.rs:610-663`
- Regen: `packages/temper-ui/src/lib/types/generated/graph_territory.ts`

**Interfaces:**
- Produces: `TerritorySlice.label: Option<String>` (Rust) → `label: string | null` (TS), populated by `territory_slice`. Consumed by the loader (Task 8) and `TierTerritory` (Task 9).

- [ ] **Step 1: Add `label` to the Rust type**

In `crates/temper-core/src/types/graph_territory.rs`, change `TerritorySlice` (line 115):

```rust
/// R3 territory drill-in: region label, components + top-N members (visibility-scoped).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TerritorySlice {
    pub region_id: Uuid,
    /// The region's human label (`kb_cogmap_regions.label`); may be null.
    pub label: Option<String>,
    pub components: Vec<Component>,
    pub members: Vec<RegionMember>,
}
```

- [ ] **Step 2: Fold the label into the existing readability query**

In `crates/temper-services/src/services/graph_service.rs`, in `territory_slice` (line 610), replace the `readable: bool` gate (lines 615-626) with a label-returning `fetch_optional`, and add `label` to the returned struct (line 658):

```rust
    // The readability gate and the label come from one query: deny-as-absence
    // (region must exist, be unfolded, and be cogmap-readable). Selecting the
    // label here adds no new visibility surface — it is strictly less sensitive
    // than the member titles returned below.
    let label: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT reg.label FROM kb_cogmap_regions reg \
         WHERE reg.id = $1 AND NOT reg.is_folded \
           AND cogmap_readable_by_profile($2, reg.cogmap_id)",
    )
    .bind(region_id)
    .bind(profile_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
```

Then update the return (line 658):

```rust
    Ok(TerritorySlice {
        region_id,
        label,
        components,
        members,
    })
```

> Note: `fetch_optional` returns `Option<Option<String>>` — outer `None` = no matching readable row (→ `NotFound`); inner value is the nullable label. `ok_or(ApiError::NotFound)?` collapses the outer option, leaving `label: Option<String>`.

- [ ] **Step 3: Build the Rust workspace**

Run: `cargo make check`
Expected: green (fmt + clippy + docs). If clippy flags the `query_scalar` turbofish as redundant, keep the explicit `::<_, Option<String>>` only if inference fails; otherwise drop it.

- [ ] **Step 4: Regenerate TS types**

Run: `cargo make generate-ts-types`
Expected: `packages/temper-ui/src/lib/types/generated/graph_territory.ts` now shows
`export type TerritorySlice = { region_id: string, label: string | null, components: Array<Component>, members: Array<RegionMember>, };`

- [ ] **Step 5: Verify the UI typechecks against the new field**

Run: `cd packages/temper-ui && bun run check`
Expected: no new errors (existing `readRegionSlice` consumers don't read `label` yet).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/graph_territory.rs crates/temper-services/src/services/graph_service.rs packages/temper-ui/src/lib/types/generated/graph_territory.ts
git commit -m "feat(atlas): R3 region-slice read returns region label (W1 crumb prereq)"
```

---

## Task 2: nav.ts — focus becomes a path

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts:41-47,146-156`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces:
  - `parseFocusPath(url: URL): Focus[]` — full drill path, `[]` when no focus.
  - `parseFocus(params): Focus` — unchanged signature, returns the **leaf** (last path segment) or `{kind:'none'}`.
  - `buildDrillNodeUrl(base, nodeId)` — **appends** `node:<id>` when the current focus leaf is a territory, else sets `node:<id>`.
  - `buildDrillTerritoryUrl(base, territoryId)` — sets `focus=territory:<id>` (unchanged behavior; always the first hop).
  - `buildAscendUrl(base)` — **pops the last path segment**.
- Consumes: existing `withParams`, `Focus`.

- [ ] **Step 1: Write the failing tests**

Add to `packages/temper-ui/src/lib/graph/atlas/nav.test.ts` (import `parseFocusPath` alongside existing imports):

```ts
import { parseFocusPath, buildDrillNodeUrl, buildDrillTerritoryUrl, buildAscendUrl, parseFocus } from './nav';

const u = (search: string) => new URL(`https://x/graph/@me${search}`);

describe('focus-as-path', () => {
	it('parses an empty path', () => {
		expect(parseFocusPath(u(''))).toEqual([]);
	});
	it('parses a territory→node path', () => {
		expect(parseFocusPath(u('?focus=territory:R,node:N'))).toEqual([
			{ kind: 'territory', id: 'R' },
			{ kind: 'node', id: 'N' }
		]);
	});
	it('parseFocus returns the leaf segment', () => {
		expect(parseFocus(u('?focus=territory:R,node:N').searchParams)).toEqual({ kind: 'node', id: 'N' });
		expect(parseFocus(u('?focus=territory:R').searchParams)).toEqual({ kind: 'territory', id: 'R' });
	});
	it('drillNode appends when a territory leaf is present', () => {
		expect(buildDrillNodeUrl(u('?team=T&focus=territory:R'), 'N')).toBe('/graph/@me?team=T&focus=territory%3AR%2Cnode%3AN');
	});
	it('drillNode sets directly when drilled from panorama', () => {
		expect(buildDrillNodeUrl(u('?team=T'), 'N')).toBe('/graph/@me?team=T&focus=node%3AN');
	});
	it('drillTerritory sets the first hop', () => {
		expect(buildDrillTerritoryUrl(u('?team=T'), 'R')).toBe('/graph/@me?team=T&focus=territory%3AR');
	});
	it('ascend pops one segment', () => {
		expect(buildAscendUrl(u('?team=T&focus=territory:R,node:N'))).toBe('/graph/@me?team=T&focus=territory%3AR');
		expect(buildAscendUrl(u('?team=T&focus=territory:R'))).toBe('/graph/@me?team=T');
	});
});
```

Also review the existing `nav.test.ts` cases for `buildDrillNodeUrl`/`buildAscendUrl` and update any that assumed the single-element `set`/`delete` behavior to match the new append/pop semantics.

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- nav`
Expected: FAIL — `parseFocusPath` not exported; append/pop assertions fail.

- [ ] **Step 3: Implement focus-as-path in `nav.ts`**

Add `parseFocusPath` and rework `parseFocus` (replace lines 41-47):

```ts
/** Parse one `kind:id` focus token; null if malformed. */
function parseFocusToken(raw: string): Focus | null {
	const [kind, id] = raw.split(':', 2);
	if (id && (kind === 'territory' || kind === 'node')) return { kind, id };
	return null;
}

/** The full drill path (`?focus=territory:X,node:Y` → [territory, node]). */
export function parseFocusPath(url: URL): Focus[] {
	const raw = url.searchParams.get('focus');
	if (!raw) return [];
	return raw.split(',').map(parseFocusToken).filter((f): f is Focus => f !== null);
}

/** The active focus = the leaf (last) segment; drives tier + neighborhood seeding. */
export function parseFocus(params: URLSearchParams): Focus {
	const raw = params.get('focus');
	if (!raw) return { kind: 'none' };
	const segs = raw.split(',').map(parseFocusToken).filter((f): f is Focus => f !== null);
	return segs.length ? segs[segs.length - 1] : { kind: 'none' };
}
```

Replace the drill/ascend builders (lines 146-156):

```ts
export function buildDrillTerritoryUrl(base: URL, territoryId: string): string {
	// A territory is always the first drill hop from the panorama.
	return withParams(base, (p) => p.set('focus', `territory:${territoryId}`));
}

export function buildDrillNodeUrl(base: URL, nodeId: string): string {
	// Append the node to the existing path when we drilled in via a territory
	// (so the crumb + ascend keep the territory hop); otherwise drill straight.
	return withParams(base, (p) => {
		const path = (p.get('focus') ?? '').split(',').filter(Boolean);
		const leaf = path[path.length - 1] ?? '';
		if (leaf.startsWith('territory:')) p.set('focus', `${path.join(',')},node:${nodeId}`);
		else p.set('focus', `node:${nodeId}`);
	});
}

export function buildAscendUrl(base: URL): string {
	// Pop exactly one drill level (node → its territory → panorama).
	return withParams(base, (p) => {
		const path = (p.get('focus') ?? '').split(',').filter(Boolean);
		path.pop();
		if (path.length) p.set('focus', path.join(','));
		else p.delete('focus');
	});
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd packages/temper-ui && bun run test -- nav`
Expected: PASS (all focus-path cases + updated existing cases).

- [ ] **Step 5: Typecheck (call sites unchanged)**

Run: `cd packages/temper-ui && bun run check`
Expected: green. `parseFocus` keeps its signature, so `+page.server.ts`/`deriveTier` need no change; drill call sites pass `(url, id)` unchanged.

- [ ] **Step 6: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): focus becomes a drill path (W1 depth-aware crumb + single-level ascend)"
```

---

## Task 3: crumbModel — pure breadcrumb segment derivation

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/crumbModel.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/crumbModel.test.ts`

**Interfaces:**
- Consumes: `Focus` from `nav.ts`; `TeamScopeView` from `graph_scope`.
- Produces:
  ```ts
  export interface CrumbSegment { label: string; kind: 'home' | 'ancestor' | 'team' | 'cogmap' | 'territory' | 'node'; focusPath: string | null; }
  export interface CrumbInput {
    scope: TeamScopeView | null;
    cogmapName: string | null;
    focusPath: Focus[];
    crumbTerritory: { id: string; label: string | null } | null;
    seedTitle: string | null;
  }
  export function crumbModel(input: CrumbInput): CrumbSegment[];
  ```
  Each segment's `focusPath` is the `?focus=` value to navigate to (null for home/ancestor/team/cogmap, which use scope builders). `AtlasCrumb` (Task 4) maps segments to hrefs.

- [ ] **Step 1: Write the failing test**

Create `packages/temper-ui/src/lib/graph/atlas/crumbModel.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { crumbModel } from './crumbModel';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';

const scope: TeamScopeView = {
	team: { id: 'T', slug: 't', name: 'Engineering' },
	ancestors: [{ id: 'A', slug: 'a', name: 'Acme' }],
	zones: []
};

describe('crumbModel', () => {
	it('team scope, no focus → Atlas / Acme / Engineering', () => {
		const segs = crumbModel({ scope, cogmapName: null, focusPath: [], crumbTerritory: null, seedTitle: null });
		expect(segs.map((s) => [s.kind, s.label])).toEqual([
			['home', '⌂ Atlas'],
			['ancestor', 'Acme'],
			['team', 'Engineering']
		]);
	});
	it('territory focus adds the labeled territory hop', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: 'Runbooks' }, seedTitle: null
		});
		expect(segs.at(-1)).toEqual({ kind: 'territory', label: 'Runbooks', focusPath: 'territory:R' });
	});
	it('node reached via a territory shows both hops', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }, { kind: 'node', id: 'N' }],
			crumbTerritory: { id: 'R', label: 'Runbooks' }, seedTitle: 'Deploy pipeline'
		});
		expect(segs.slice(-2).map((s) => [s.kind, s.label, s.focusPath])).toEqual([
			['territory', 'Runbooks', 'territory:R'],
			['node', 'Deploy pipeline', 'territory:R,node:N']
		]);
	});
	it('node drilled straight from panorama has no territory hop', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'node', id: 'N' }],
			crumbTerritory: null, seedTitle: 'Orphan doc'
		});
		expect(segs.map((s) => s.kind)).toEqual(['home', 'ancestor', 'team', 'node']);
		expect(segs.at(-1)).toEqual({ kind: 'node', label: 'Orphan doc', focusPath: 'node:N' });
	});
	it('cogmap scope → Atlas / <cogmap name>', () => {
		const segs = crumbModel({ scope: null, cogmapName: 'Team self-model', focusPath: [], crumbTerritory: null, seedTitle: null });
		expect(segs.map((s) => [s.kind, s.label])).toEqual([['home', '⌂ Atlas'], ['cogmap', 'Team self-model']]);
	});
	it('territory label falls back to a generic when unresolved', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: null }, seedTitle: null
		});
		expect(segs.at(-1)?.label).toBe('Region');
	});
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- crumbModel`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `crumbModel.ts`**

```ts
import type { Focus } from './nav';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';

export interface CrumbSegment {
	label: string;
	kind: 'home' | 'ancestor' | 'team' | 'cogmap' | 'territory' | 'node';
	/** The `?focus=` value this segment navigates to; null for home/scope segments. */
	focusPath: string | null;
}

export interface CrumbInput {
	scope: TeamScopeView | null;
	cogmapName: string | null;
	focusPath: Focus[];
	crumbTerritory: { id: string; label: string | null } | null;
	seedTitle: string | null;
}

const encode = (path: Focus[]): string => path.map((f) => `${f.kind}:${f.id}`).join(',');

/** Derive the ordered breadcrumb segments from URL/loaded state. Pure. */
export function crumbModel(input: CrumbInput): CrumbSegment[] {
	const segs: CrumbSegment[] = [{ label: '⌂ Atlas', kind: 'home', focusPath: null }];

	if (input.scope) {
		for (const a of input.scope.ancestors) segs.push({ label: a.name, kind: 'ancestor', focusPath: null });
		segs.push({ label: input.scope.team.name, kind: 'team', focusPath: null });
	} else if (input.cogmapName) {
		segs.push({ label: input.cogmapName, kind: 'cogmap', focusPath: null });
	}

	// Build cumulative focus paths so each drill segment links to its own depth.
	const walked: Focus[] = [];
	for (const f of input.focusPath) {
		walked.push(f);
		if (f.kind === 'territory') {
			const label = input.crumbTerritory?.id === f.id ? input.crumbTerritory.label : null;
			segs.push({ label: label ?? 'Region', kind: 'territory', focusPath: encode(walked) });
		} else {
			segs.push({ label: input.seedTitle ?? 'Node', kind: 'node', focusPath: encode(walked) });
		}
	}
	return segs;
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd packages/temper-ui && bun run test -- crumbModel`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/crumbModel.ts packages/temper-ui/src/lib/graph/atlas/crumbModel.test.ts
git commit -m "feat(atlas): pure crumbModel for the depth-aware breadcrumb"
```

---

## Task 4: AtlasCrumb.svelte — the breadcrumb component + ascend

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/AtlasCrumb.svelte`

**Interfaces:**
- Consumes: `crumbModel` (Task 3); `nav.ts` builders `buildHomeUrl`, `buildScopeUrl`, `buildCogmapUrl`, `buildAscendUrl`, and a small helper to navigate to a `focusPath`. Props: `{ scope, cogmapName, focusPath, crumbTerritory, seedTitle, teamId, cogmapId }`.
- Produces: a mounted breadcrumb; Task 8 mounts it in the top bar.

- [ ] **Step 1: Implement the component**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TeamScopeView } from '$lib/types/generated/graph_scope';
	import type { Focus } from '$lib/graph/atlas/nav';
	import { buildHomeUrl, buildScopeUrl, buildCogmapUrl, buildAscendUrl } from '$lib/graph/atlas/nav';
	import { crumbModel, type CrumbSegment } from '$lib/graph/atlas/crumbModel';

	interface Props {
		scope: TeamScopeView | null;
		cogmapName: string | null;
		focusPath: Focus[];
		crumbTerritory: { id: string; label: string | null } | null;
		seedTitle: string | null;
		teamId: string | null;
		cogmapId: string | null;
	}
	let { scope, cogmapName, focusPath, crumbTerritory, seedTitle, teamId, cogmapId }: Props = $props();

	const segments = $derived(
		crumbModel({ scope, cogmapName, focusPath, crumbTerritory, seedTitle })
	);
	const canAscend = $derived(focusPath.length > 0);

	// Navigate to a specific `?focus=` path (drill segment click). PUSH — this is a
	// scope/drill transition (Beat 1 history policy).
	function gotoFocus(focusValue: string) {
		const u = new URL($page.url);
		u.searchParams.set('focus', focusValue);
		goto(`${u.pathname}${u.search}`);
	}

	function onSegment(seg: CrumbSegment) {
		if (seg.kind === 'home') return goto(buildHomeUrl($page.url));
		if (seg.kind === 'team' && teamId) return goto(buildScopeUrl($page.url, teamId));
		if (seg.kind === 'cogmap' && cogmapId) return goto(buildCogmapUrl($page.url, cogmapId));
		if (seg.focusPath) return gotoFocus(seg.focusPath);
		// ancestors are a de-emphasized set with no drill target
	}
</script>

<nav class="crumb-bar" aria-label="Atlas breadcrumb">
	<button
		class="ascend"
		type="button"
		disabled={!canAscend}
		title="Up one level"
		aria-label="Up one level"
		onclick={() => goto(buildAscendUrl($page.url))}>↑</button
	>
	{#each segments as seg, i (i)}
		{#if i > 0}<span class="sep">›</span>{/if}
		{#if seg.kind === 'ancestor'}
			<span class="seg ancestor">{seg.label}</span>
		{:else}
			<button
				class="seg {seg.kind} {i === segments.length - 1 ? 'current' : ''}"
				type="button"
				onclick={() => onSegment(seg)}>{seg.label}</button
			>
		{/if}
	{/each}
</nav>

<style>
	.crumb-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
		min-width: 0;
		flex-wrap: wrap;
	}
	.ascend {
		background: none;
		border: 1px solid #4a5162;
		border-radius: 6px;
		color: inherit;
		cursor: pointer;
		padding: 0 7px;
		line-height: 1.6;
	}
	.ascend:disabled {
		opacity: 0.3;
		cursor: default;
	}
	.seg {
		background: none;
		border: 0;
		padding: 2px 4px;
		font: inherit;
		color: inherit;
		cursor: pointer;
		border-radius: 5px;
	}
	.seg:not(.ancestor):hover {
		background: rgba(255, 255, 255, 0.06);
	}
	.seg.current {
		font-weight: 600;
		cursor: default;
	}
	.ancestor {
		opacity: 0.6;
	}
	.sep {
		opacity: 0.4;
	}
</style>
```

- [ ] **Step 2: Typecheck**

Run: `cd packages/temper-ui && bun run check`
Expected: green (component is standalone; not yet mounted).

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/graph/atlas/AtlasCrumb.svelte
git commit -m "feat(atlas): AtlasCrumb — depth-aware breadcrumb + single-level ascend"
```

---

## Task 5: FilterPopover.svelte — filters relocated to a popover

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/FilterPopover.svelte`
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts` (add `activeFilterCount`)
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts` (add cases)

**Interfaces:**
- Produces: `activeFilterCount(url: URL): number` in `nav.ts` (count of non-default filter dimensions). `FilterPopover` props: `{ filters: GraphFilters }`.
- Consumes: existing `buildFiltersUrl`, `GraphFilters`, `DOC_TYPE_HUES`, the `EDGE_KIND_OPTIONS` moved from `ScopeBar`.

- [ ] **Step 1: Write the failing test for `activeFilterCount`**

Add to `nav.test.ts`:

```ts
import { activeFilterCount } from './nav';

describe('activeFilterCount', () => {
	it('is 0 with no filters', () => expect(activeFilterCount(u(''))).toBe(0));
	it('counts each active dimension', () =>
		expect(activeFilterCount(u('?lens_id=L&edge_kinds=contains,near&doc_types=note'))).toBe(3));
	it('empty CSV params do not count', () =>
		expect(activeFilterCount(u('?edge_kinds='))).toBe(0));
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- nav`
Expected: FAIL — `activeFilterCount` not exported.

- [ ] **Step 3: Implement `activeFilterCount` in `nav.ts`**

Add after `parseFilters`:

```ts
/** Count of active (non-default) filter dimensions — drives the popover badge. */
export function activeFilterCount(url: URL): number {
	const f = parseFilters(url.searchParams);
	return (f.lensId ? 1 : 0) + (f.edgeKinds.length ? 1 : 0) + (f.docTypes.length ? 1 : 0);
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test -- nav`
Expected: PASS.

- [ ] **Step 5: Implement `FilterPopover.svelte`**

Port the filter controls from `ScopeBar.svelte` (lines 15-90) into a popover. Behavior/params unchanged (REPLACE history):

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { EdgeKind } from '$lib/types/generated/graph';
	import { buildFiltersUrl, activeFilterCount, type GraphFilters } from '$lib/graph/atlas/nav';
	import { DOC_TYPE_HUES, type AtlasDocType } from '$lib/graph/atlas/palette';

	interface Props {
		filters: GraphFilters;
	}
	let { filters }: Props = $props();

	const EDGE_KIND_OPTIONS: EdgeKind[] = ['contains', 'leads_to', 'express', 'near'];
	const DOC_TYPE_OPTIONS = Object.keys(DOC_TYPE_HUES) as AtlasDocType[];
	const count = $derived(activeFilterCount($page.url));
	let open = $state(false);

	function toggleEdgeKind(k: EdgeKind) {
		const next = filters.edgeKinds.includes(k)
			? filters.edgeKinds.filter((x) => x !== k)
			: [...filters.edgeKinds, k];
		goto(buildFiltersUrl($page.url, { edgeKinds: next }), { replaceState: true });
	}
	function toggleDocType(dt: AtlasDocType) {
		const next = filters.docTypes.includes(dt)
			? filters.docTypes.filter((x) => x !== dt)
			: [...filters.docTypes, dt];
		goto(buildFiltersUrl($page.url, { docTypes: next }), { replaceState: true });
	}
	function setLens(raw: string) {
		goto(buildFiltersUrl($page.url, { lensId: raw.trim() || null }), { replaceState: true });
	}
</script>

<div class="filter-popover">
	<button type="button" class="trigger" class:active={count > 0} onclick={() => (open = !open)}>
		⚑ Filters{#if count > 0}<span class="badge">{count}</span>{/if}
	</button>
	{#if open}
		<div class="panel">
			<div class="filter-group">
				<span class="filter-label">EDGE</span>
				{#each EDGE_KIND_OPTIONS as k (k)}
					<button type="button" class="chip" class:active={filters.edgeKinds.includes(k)} onclick={() => toggleEdgeKind(k)}>{k}</button>
				{/each}
			</div>
			<div class="filter-group">
				<span class="filter-label">TYPE</span>
				{#each DOC_TYPE_OPTIONS as dt (dt)}
					<button type="button" class="chip" class:active={filters.docTypes.includes(dt)} style="--chip-color: {DOC_TYPE_HUES[dt]}" onclick={() => toggleDocType(dt)}>{dt}</button>
				{/each}
			</div>
			<div class="filter-group">
				<span class="filter-label">LENS</span>
				<input class="lens-input" type="text" placeholder="lens id" value={filters.lensId ?? ''} onchange={(e) => setLens((e.currentTarget as HTMLInputElement).value)} />
			</div>
		</div>
	{/if}
</div>

<style>
	.filter-popover { position: relative; }
	.trigger {
		background: #1c1c1c; border: 1px solid #4a5162; border-radius: 6px;
		color: var(--color-quiet-ink, #c9ced9); cursor: pointer; font-size: 12px; padding: 3px 10px;
		display: inline-flex; align-items: center; gap: 6px;
	}
	.trigger.active { border-color: #6e5a2a; color: #e8cf8f; }
	.badge {
		background: #6e5a2a; color: #1a1206; border-radius: 8px; font-size: 10px;
		padding: 0 5px; line-height: 1.5;
	}
	.panel {
		position: absolute; right: 0; top: calc(100% + 6px); z-index: 10;
		background: #14171d; border: 1px solid rgba(255, 255, 255, 0.1); border-radius: 8px;
		padding: 8px 12px; display: flex; flex-direction: column; gap: 8px; min-width: 220px;
	}
	.filter-group { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
	.filter-label { font: 8.5px monospace; letter-spacing: 0.2em; color: #6a727e; margin-right: 2px; }
	.chip {
		background: none; border: 1px solid var(--chip-color, #4a5162); color: var(--color-quiet-ink, #c9ced9);
		border-radius: 10px; padding: 1px 8px; font-size: 10.5px; cursor: pointer; opacity: 0.55;
	}
	.chip.active { opacity: 1; background: color-mix(in srgb, var(--chip-color, #4a5162) 22%, transparent); }
	.lens-input {
		background: rgba(255, 255, 255, 0.04); border: 1px solid #4a5162; border-radius: 6px;
		color: var(--color-quiet-ink, #c9ced9); font-size: 11px; padding: 2px 6px; width: 96px;
	}
</style>
```

- [ ] **Step 6: Typecheck**

Run: `cd packages/temper-ui && bun run check`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts packages/temper-ui/src/lib/components/graph/atlas/FilterPopover.svelte
git commit -m "feat(atlas): FilterPopover + activeFilterCount badge (filters relocated from dock)"
```

---

## Task 6: AtlasLegend — collapsible bottom bar, collapsed by default

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasLegend.svelte:5,8-89,91-136`

**Interfaces:**
- Produces: a horizontal legend that lays sections in a row when expanded, collapsed by default. Task 8 mounts it in the bottom bar. `legendModel()` unchanged.

- [ ] **Step 1: Flip default + horizontal layout**

Change line 5:

```ts
	let open = $state(false);
```

Add a bridge note row after the WEIGHT section (before the closing `{/if}` at line 88):

```svelte
			<div class="sec">
				<div class="lbl">BRIDGE</div>
				<div class="row"><span class="line thick" style="background:#e8cf8f"></span>shared-edge count · thicker = stronger</div>
			</div>
```

Update `.legend` styles so that, when open, sections flow horizontally. Replace the `.legend` and `.sec` rules:

```css
	.legend {
		padding: 6px 12px;
		font-size: 12px;
		color: var(--color-quiet-ink, #c9d1d9);
		display: flex;
		align-items: flex-start;
		gap: 18px;
		flex-wrap: wrap;
	}
	.sec {
		padding: 2px 0;
	}
```

Add a `.line.thick { height: 4px; }` rule alongside the existing `.line` rule.

- [ ] **Step 2: Typecheck + visual sanity via build**

Run: `cd packages/temper-ui && bun run check`
Expected: green.

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/graph/atlas/AtlasLegend.svelte
git commit -m "feat(atlas): legend as horizontal bottom bar, collapsed by default (+ bridge key)"
```

---

## Task 7: Collapsible vault sidebar (shared shell)

**Files:**
- Create: `packages/temper-ui/src/lib/stores/sidebar.svelte.ts` (+ `.test.ts`)
- Modify: `packages/temper-ui/src/lib/components/Sidebar.svelte`
- Modify: `packages/temper-ui/src/routes/(app)/+layout.svelte`

**Interfaces:**
- Produces:
  - `defaultCollapsed(pathname: string): boolean` — pure; true on `/graph` routes.
  - `sidebarCollapsed` — a runes-based, localStorage-persisted boolean holder: `{ get value(): boolean; set(v: boolean): void; toggle(): void; initFor(pathname: string): void }`.
- Consumes: `browser` from `$app/environment`.

- [ ] **Step 1: Write the failing test for `defaultCollapsed`**

Create `packages/temper-ui/src/lib/stores/sidebar.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { defaultCollapsed } from './sidebar.svelte';

describe('defaultCollapsed', () => {
	it('collapses on graph routes', () => {
		expect(defaultCollapsed('/graph/@me')).toBe(true);
		expect(defaultCollapsed('/graph/@me?team=T')).toBe(true);
	});
	it('stays expanded elsewhere', () => {
		expect(defaultCollapsed('/vault/all')).toBe(false);
		expect(defaultCollapsed('/teams')).toBe(false);
	});
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- sidebar`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `sidebar.svelte.ts`**

```ts
import { browser } from '$app/environment';

const KEY = 'temper.sidebar.collapsed';

/** Graph routes default to a collapsed rail (they want the width). Pure. */
export function defaultCollapsed(pathname: string): boolean {
	return pathname.startsWith('/graph');
}

function load(): boolean | null {
	if (!browser) return null;
	const v = localStorage.getItem(KEY);
	return v === null ? null : v === '1';
}

let collapsed = $state(false);

export const sidebarCollapsed = {
	get value() {
		return collapsed;
	},
	set(v: boolean) {
		collapsed = v;
		if (browser) localStorage.setItem(KEY, v ? '1' : '0');
	},
	toggle() {
		this.set(!collapsed);
	},
	/** Seed from stored preference, else the route default. Explicit user choice wins. */
	initFor(pathname: string) {
		const stored = load();
		collapsed = stored === null ? defaultCollapsed(pathname) : stored;
	}
};
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test -- sidebar`
Expected: PASS.

- [ ] **Step 5: Add icon-rail mode to `Sidebar.svelte`**

Add a `collapsed` prop and a toggle button; render an icon rail when collapsed. Change the `Props` interface (line 6) and destructure (line 14):

```ts
	interface Props {
		contexts: ContextRowWithCounts[];
		user: { display_name: string; email: string } | null;
		isAdmin: boolean;
		instanceName?: string | null;
		collapsed: boolean;
		onToggle: () => void;
	}
	let { contexts, user, isAdmin, instanceName = null, collapsed, onToggle }: Props = $props();
```

Wrap the `<aside>` width and gate the full nav on `!collapsed`. Replace the `<aside>` opening tag (line 25) and add a collapsed rail. Minimal approach — swap the width class and, when collapsed, render only the brand initial + a expand button + context dots:

```svelte
<aside class="flex flex-col {collapsed ? 'w-12' : 'w-52'} bg-zinc-900/50 border-r border-zinc-800 overflow-hidden transition-[width] duration-150">
	<button
		type="button"
		onclick={onToggle}
		class="block px-3 pt-3 pb-2 border-b border-zinc-800 font-mono text-xs tracking-[0.15em] text-zinc-300 hover:text-zinc-100 text-left truncate"
		title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
	>
		{collapsed ? '≡' : brand}
	</button>
	{#if !collapsed}
		<!-- existing <nav> … </nav> and footer <div> … </div> unchanged (lines 33-89) -->
	{/if}
</aside>
```

Move the existing `<nav>` (lines 33-54) and footer `<div>` (lines 56-89) inside the `{#if !collapsed}` block verbatim. (The collapsed rail intentionally shows just the toggle for 2a; per-context icons can come later.)

- [ ] **Step 6: Wire collapse state in `+layout.svelte`**

In `packages/temper-ui/src/routes/(app)/+layout.svelte`, import the store + page, init on navigation, and pass props. Add to the `<script>` (after line 5):

```ts
	import { page } from '$app/stores';
	import { sidebarCollapsed } from '$lib/stores/sidebar.svelte';

	// Seed collapse from stored preference or the route default on each navigation
	// (explicit user toggles persist and win). $effect re-runs when the path changes.
	$effect(() => {
		sidebarCollapsed.initFor($page.url.pathname);
	});
```

Update the `<Sidebar … />` mount (lines 22-29) to pass the two new props:

```svelte
	<Sidebar
		contexts={data.contexts ?? []}
		user={data.profile
			? { display_name: data.profile.display_name, email: data.profile.email ?? '' }
			: null}
		isAdmin={data.entitlements?.is_admin ?? false}
		instanceName={data.instanceName ?? null}
		collapsed={sidebarCollapsed.value}
		onToggle={() => sidebarCollapsed.toggle()}
	/>
```

> Note: `initFor` on every navigation means a user's explicit toggle persists to localStorage and is honored across routes (stored value wins over the route default). This is the intended "general, persisted affordance."

- [ ] **Step 7: Typecheck**

Run: `cd packages/temper-ui && bun run check`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add packages/temper-ui/src/lib/stores/sidebar.svelte.ts packages/temper-ui/src/lib/stores/sidebar.test.ts packages/temper-ui/src/lib/components/Sidebar.svelte "packages/temper-ui/src/routes/(app)/+layout.svelte"
git commit -m "feat(shell): collapsible vault sidebar (icon rail, persisted, default-collapsed on /graph)"
```

---

## Task 8: Atlas page restructure — retire dock, top bar + bottom bar, loader threading

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts`
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.svelte`
- Delete: `packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte`, `.../CogmapCrumb.svelte`

**Interfaces:**
- Consumes: `AtlasCrumb` (Task 4), `FilterPopover` (Task 5), `AtlasLegend` (Task 6), `parseFocusPath` (Task 2), `readRegionSlice` w/ `label` (Task 1).
- Produces: page data adds `focusPath: Focus[]` and `crumbTerritory: { id: string; label: string | null } | null`.

- [ ] **Step 1: Thread `focusPath` + `crumbTerritory` in the loader**

In `+page.server.ts`, import `parseFocusPath`:

```ts
import { deriveTier, parseCogmap, parseFilters, parseFocus, parseFocusPath, parseTeam, selectedElement } from '$lib/graph/atlas/nav';
```

Compute the path + the territory in the drill path (a territory segment can appear at Tier 1 or as the ancestor of a Tier-2 node):

```ts
	const focusPath = parseFocusPath(url);
	const territorySeg = focusPath.find((f) => f.kind === 'territory') ?? null;
```

In the **team branch** (after `slice`/`neighborhood`, before the return at line 111), resolve the territory label for the crumb, reusing the now-labeled R3 read (skip the fetch if the Tier-1 `slice` already carries it):

```ts
	// Name the territory hop in the crumb. Tier 1 already loaded the slice (carries
	// label); at Tier 2 fetch the path territory's slice for its label (reuses the
	// gated R3 read — over-fetches members, acceptable for one label).
	const crumbTerritory = territorySeg
		? slice && slice.region_id === territorySeg.id
			? { id: territorySeg.id, label: slice.label }
			: { id: territorySeg.id, label: (await readRegionSlice(token, territorySeg.id)).label }
		: null;
```

Add `focusPath` and `crumbTerritory` to the team-branch return object. For the **cogmap** and **home** branches, add `focusPath` and `crumbTerritory: null` (cogmap drill paths don't carry team territories in 2a). Every returned object gains these two keys.

- [ ] **Step 2: Restructure `+page.svelte`**

Replace the template + styles. Remove the `dock`; add a top bar (crumb + filters + search placeholder) and a bottom bar (legend). Retire `ScopeBar`/`CogmapCrumb` imports; add `AtlasCrumb`/`FilterPopover`; import `parseFocusPath`:

```svelte
<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import AtlasLegend from '$lib/components/graph/atlas/AtlasLegend.svelte';
	import AtlasCrumb from '$lib/components/graph/atlas/AtlasCrumb.svelte';
	import FilterPopover from '$lib/components/graph/atlas/FilterPopover.svelte';
	import SearchAccelerator from '$lib/components/graph/atlas/SearchAccelerator.svelte';
	import TrailRail from '$lib/components/graph/atlas/TrailRail.svelte';
	import { selectedElement } from '$lib/graph/atlas/nav';
	import { navigating, page } from '$app/stores';

	let { data }: { data: PageData } = $props();

	const viewKey = $derived(
		`${data.teamId ?? data.cogmapId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
	const selection = $derived(selectedElement(data.focus, $page.url));
	const subgraph = $derived(data.neighborhood ?? null);
	const hasPanelData = $derived(subgraph !== null && subgraph.nodes.length > 0);
	const seedTitle = $derived(
		data.focus.kind === 'node' && subgraph
			? (subgraph.nodes.find((n) => n.id === (data.focus as { id: string }).id)?.title ?? null)
			: null
	);
	const scopeKey = (u: URL) =>
		`${u.searchParams.get('team') ?? u.searchParams.get('cogmap') ?? 'home'}|${u.searchParams.get('focus') ?? ''}`;
	const isViewLoad = $derived(!!$navigating?.to && scopeKey($navigating.to.url) !== scopeKey($page.url));
</script>

<div class="atlas-page">
	<div class="top-bar">
		<AtlasCrumb
			scope={data.scope}
			cogmapName={data.cogmapName}
			focusPath={data.focusPath}
			crumbTerritory={data.crumbTerritory}
			{seedTitle}
			teamId={data.teamId}
			cogmapId={data.cogmapId}
		/>
		<div class="top-right">
			{#if data.scope}
				<FilterPopover filters={data.filters} />
			{/if}
			{#if data.teamId}
				<SearchAccelerator teamId={data.teamId} />
			{/if}
		</div>
	</div>

	<div class="canvas-wrap">
		{#if isViewLoad}
			<div class="loading-veil" role="status" aria-live="polite">Loading…</div>
		{/if}
		{#key viewKey}
			<AtlasCanvas
				teamId={data.teamId}
				cogmapId={data.cogmapId}
				tier={data.tier}
				focus={data.focus}
				territories={data.territories}
				slice={data.slice}
				neighborhood={data.neighborhood}
				teams={data.teams}
				cogmaps={data.cogmaps}
				zones={data.scope?.zones ?? []}
				filters={data.filters}
			/>
		{/key}
		{#if selection.kind !== 'none' && hasPanelData}
			<TrailRail {selection} {subgraph} trail={data.trail} resourceRow={data.resourceRow} />
		{/if}
	</div>

	<div class="bottom-bar"><AtlasLegend /></div>
</div>

<style>
	.atlas-page {
		display: grid;
		grid-template-rows: auto 1fr auto;
		height: 100%;
		min-height: 0;
	}
	.top-bar {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 8px 14px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
		min-width: 0;
	}
	.top-right {
		margin-left: auto;
		display: flex;
		align-items: center;
		gap: 10px;
	}
	.canvas-wrap {
		position: relative;
		min-width: 0;
		min-height: 0;
	}
	.bottom-bar {
		border-top: 1px solid rgba(255, 255, 255, 0.06);
		overflow-x: auto;
	}
	.loading-veil {
		position: absolute;
		top: 12px;
		left: 50%;
		transform: translateX(-50%);
		z-index: 2;
		padding: 4px 14px;
		border-radius: 12px;
		background: rgba(20, 23, 29, 0.85);
		border: 1px solid rgba(255, 255, 255, 0.08);
		color: var(--color-quiet-ink, #c9ced9);
		font-size: 12px;
		letter-spacing: 0.04em;
		pointer-events: none;
	}
</style>
```

> Note: `TrailRail` moves inside `.canvas-wrap` as an absolutely-positioned right rail. Confirm `TrailRail` already positions itself as a right panel (it does — it slides in from the right); if it relied on the old grid `auto` column, add `position:absolute; right:0; top:0; bottom:0;` to `.canvas-wrap > :global(...)` or a wrapper. Verify visually in the build.

- [ ] **Step 3: Delete the retired components**

```bash
git rm packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte packages/temper-ui/src/lib/components/graph/atlas/CogmapCrumb.svelte
```

- [ ] **Step 4: Typecheck**

Run: `cd packages/temper-ui && bun run check`
Expected: green. If `PageData` complains about `focusPath`/`crumbTerritory`, confirm every loader return branch includes both keys.

- [ ] **Step 5: Build to confirm the route compiles**

Run: `cd packages/temper-ui && bun run build`
Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
git add -A packages/temper-ui/src/routes/(app)/graph packages/temper-ui/src/lib/components/graph/atlas
git commit -m "feat(atlas): retire dock — top-bar (crumb+filters+search) + bottom-bar (legend) shell"
```

---

## Task 9: L3 — empty-territory ghost + real region label at Tier 1

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/territory.ts` (+ `.test.ts`)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte:36`

**Interfaces:**
- Produces: `isEmptyTerritory(t: Territory): boolean` (member_count === 0).
- Consumes: `Territory` from `graph_territory`.

- [ ] **Step 1: Write the failing test**

Create `packages/temper-ui/src/lib/graph/atlas/territory.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { isEmptyTerritory } from './territory';
import type { Territory } from '$lib/types/generated/graph_territory';

const t = (over: Partial<Territory>): Territory => ({
	id: 'x', kind: 'context', label: 'X', member_count: 3, salience: null, anchor_id: 'a', ...over
});

describe('isEmptyTerritory', () => {
	it('true when no members', () => expect(isEmptyTerritory(t({ member_count: 0 }))).toBe(true));
	it('false when populated', () => expect(isEmptyTerritory(t({ member_count: 1 }))).toBe(false));
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- territory`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `territory.ts`**

```ts
import type { Territory } from '$lib/types/generated/graph_territory';

/** A territory with no members — rendered as a de-emphasized ghost (L3). */
export function isEmptyTerritory(t: Territory): boolean {
	return t.member_count === 0;
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test -- territory`
Expected: PASS.

- [ ] **Step 5: Add a `ghost` mode to `TerritoryCircle.svelte`**

Read the current props; add a `ghost?: boolean` prop. When `ghost`, render a smaller, dimmed, dashed circle with an "empty" suffix on the label. (Grounding: `TerritoryCircle` takes `x,y,r,kind,label,onEnter`.) Add to its `<script>` props `ghost = false`, and in the template reduce opacity / add `stroke-dasharray` and an `· empty` label suffix when `ghost` is true. Keep it drillable (leave `onEnter` intact).

- [ ] **Step 6: Thread `member_count` through `packTerritories`, pass `ghost`**

`PositionedTerritory` (`packTerritories.ts:10-19`) currently carries `id/kind/label/x/y/r/salience` but **not** `member_count`, so add it. In `packTerritories.ts`, add `member_count: number;` to the `PositionedTerritory` interface, and in the `.map` return (line 48) add `member_count: t.member_count,`. Update `packTerritories.test.ts` to expect the new field on returned items.

Then in `TierPanorama.svelte`, at the `TerritoryCircle` mount (line 66), add:

```svelte
			ghost={t.member_count === 0}
```

> Regions size by salience and contexts by member_count, but the empty signal is uniformly `member_count === 0` (an empty context is the case from the spec). Ghosting a zero-salience-but-populated region is not intended, so gate on `member_count`, not `salience`.

- [ ] **Step 7: Show the real region label at Tier 1**

In `TierTerritory.svelte`, the props already include `slice` (now carrying `label`). Change the heading (line 36) from the generic label to the real one:

```svelte
<text x="24" y="28" fill={TERRITORY_TINTS.region} font-size="12" font-weight="600" letter-spacing="1">{(slice.label ?? 'REGION').toUpperCase()} · interior</text>
```

- [ ] **Step 8: Typecheck + test**

Run: `cd packages/temper-ui && bun run check && bun run test -- territory packTerritories`
Expected: green.

- [ ] **Step 9: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/territory.ts packages/temper-ui/src/lib/graph/atlas/territory.test.ts packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.test.ts
git commit -m "feat(atlas): empty-territory ghost state + real region label at Tier 1 (L3)"
```

---

## Task 10: G2 — Tier-2 label anchor + hover

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/labels.ts` (+ `.test.ts`)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeChip.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte`

**Interfaces:**
- Produces:
  - `labelAnchors(nodes: { id: string; degree: number }[], seedId: string, k: number): Set<string>` — the ids whose labels are always shown (seed + top-K by degree).
  - `truncateLabel(title: string, max: number): string`.
- Consumes: nothing external.

- [ ] **Step 1: Write the failing tests**

Create `packages/temper-ui/src/lib/graph/atlas/labels.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { labelAnchors, truncateLabel } from './labels';

describe('labelAnchors', () => {
	const nodes = [
		{ id: 'seed', degree: 1 },
		{ id: 'a', degree: 9 },
		{ id: 'b', degree: 7 },
		{ id: 'c', degree: 3 },
		{ id: 'd', degree: 2 }
	];
	it('always includes the seed plus the top-K by degree', () => {
		const set = labelAnchors(nodes, 'seed', 2);
		expect(set.has('seed')).toBe(true);
		expect(set.has('a')).toBe(true);
		expect(set.has('b')).toBe(true);
		expect(set.has('c')).toBe(false);
	});
	it('does not double-count the seed if it is high-degree', () => {
		const set = labelAnchors([{ id: 'seed', degree: 99 }, { id: 'a', degree: 5 }, { id: 'b', degree: 4 }], 'seed', 2);
		expect(set).toEqual(new Set(['seed', 'a', 'b']));
	});
});

describe('truncateLabel', () => {
	it('leaves short titles', () => expect(truncateLabel('Short', 20)).toBe('Short'));
	it('truncates with an ellipsis', () => expect(truncateLabel('A very long node title here', 10)).toBe('A very lo…'));
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- labels`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `labels.ts`**

```ts
/** Ids whose labels are always drawn at Tier 2: the seed plus the top-K by degree. */
export function labelAnchors(
	nodes: { id: string; degree: number }[],
	seedId: string,
	k: number
): Set<string> {
	const ranked = nodes
		.filter((n) => n.id !== seedId)
		.sort((a, b) => b.degree - a.degree)
		.slice(0, k)
		.map((n) => n.id);
	return new Set([seedId, ...ranked]);
}

/** Truncate a title to `max` chars with a trailing ellipsis. */
export function truncateLabel(title: string, max: number): string {
	return title.length <= max ? title : `${title.slice(0, max - 1)}…`;
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test -- labels`
Expected: PASS.

- [ ] **Step 5: NodeChip — persistent vs hover label**

In `NodeChip.svelte`, add an `anchored?: boolean` prop (default false) and a hover state; draw the label only when `anchored` OR `hovered`, truncated. Update props (line 4-16):

```ts
	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		home: 'context' | 'cogmap';
		seed?: boolean;
		anchored?: boolean;
		dim?: boolean;
		onEnter?: () => void;
	}
	let { x, y, r, title, docType, home, seed = false, anchored = false, dim = false, onEnter }: Props = $props();
```

Add to the `<script>`:

```ts
	import { truncateLabel } from '$lib/graph/atlas/palette'; // NOTE: import from labels
	let hovered = $state(false);
	const showLabel = $derived(anchored || hovered);
```

> Correction: import `truncateLabel` from `'$lib/graph/atlas/labels'`, not palette.

Add `onmouseenter`/`onmouseleave` to the `<g>` (mirroring `OrphanNodeMark` lines 29-30), and replace the label `<text>` (line 41):

```svelte
	{#if showLabel}
		<text x={x} y={y + r + 13} text-anchor="middle" fill="#c7d0da" font-size="10">{truncateLabel(title, 22)}</text>
	{/if}
```

- [ ] **Step 6: TierNeighborhood — compute anchors, pass `anchored`**

In `TierNeighborhood.svelte`, import `labelAnchors` and compute the set from the force graph nodes (they carry `id` + `degree`), then pass `anchored` to `NodeChip`:

```ts
	import { labelAnchors } from '$lib/graph/atlas/labels';
	const anchors = $derived(labelAnchors(graph.nodes, seedId, 5));
```

At the `NodeChip` mount (line 64), add:

```svelte
		anchored={anchors.has(n.id)}
```

- [ ] **Step 7: Typecheck + test**

Run: `cd packages/temper-ui && bun run check && bun run test -- labels`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/labels.ts packages/temper-ui/src/lib/graph/atlas/labels.test.ts packages/temper-ui/src/lib/components/graph/atlas/marks/NodeChip.svelte packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte
git commit -m "feat(atlas): Tier-2 labels — anchor seed+top-degree, others on hover, truncated (G2)"
```

---

## Task 11: G1 — draw aggregate bridges

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/bridges.ts` (+ `.test.ts`)
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/BridgeRibbon.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`

**Interfaces:**
- Produces:
  - `bridgeGeometry(bridges: Bridge[], positions: Map<string, { x: number; y: number }>): BridgeLine[]` where `BridgeLine = { x1; y1; x2; y2; edgeCount: number }`. Force-ready: takes a position map so a future force layout feeds computed positions into the same function.
- Consumes: `Bridge` from `graph_territory`.

- [ ] **Step 1: Write the failing test**

Create `packages/temper-ui/src/lib/graph/atlas/layout/bridges.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { bridgeGeometry } from './bridges';
import type { Bridge } from '$lib/types/generated/graph_territory';

const bridges: Bridge[] = [
	{ source_territory: 'A', target_territory: 'B', edge_count: 5 },
	{ source_territory: 'A', target_territory: 'Z', edge_count: 2 } // Z has no position
];

describe('bridgeGeometry', () => {
	it('maps positioned territory pairs to line segments', () => {
		const pos = new Map([
			['A', { x: 0, y: 0 }],
			['B', { x: 10, y: 20 }]
		]);
		expect(bridgeGeometry(bridges, pos)).toEqual([{ x1: 0, y1: 0, x2: 10, y2: 20, edgeCount: 5 }]);
	});
	it('drops bridges whose endpoints are not both positioned', () => {
		const pos = new Map([['A', { x: 0, y: 0 }]]);
		expect(bridgeGeometry(bridges, pos)).toEqual([]);
	});
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test -- bridges`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `bridges.ts`**

```ts
import type { Bridge } from '$lib/types/generated/graph_territory';

export interface BridgeLine {
	x1: number;
	y1: number;
	x2: number;
	y2: number;
	edgeCount: number;
}

/**
 * Map aggregate bridges to line segments between territory centers. Position-
 * agnostic: pass any `Map<territoryId, {x,y}>` — packing today, force later.
 * Bridges with an unpositioned endpoint are dropped.
 */
export function bridgeGeometry(
	bridges: Bridge[],
	positions: Map<string, { x: number; y: number }>
): BridgeLine[] {
	const lines: BridgeLine[] = [];
	for (const b of bridges) {
		const s = positions.get(b.source_territory);
		const t = positions.get(b.target_territory);
		if (!s || !t) continue;
		lines.push({ x1: s.x, y1: s.y, x2: t.x, y2: t.y, edgeCount: b.edge_count });
	}
	return lines;
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test -- bridges`
Expected: PASS.

- [ ] **Step 5: Implement `BridgeRibbon.svelte`**

```svelte
<script lang="ts">
	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		edgeCount: number;
	}
	let { x1, y1, x2, y2, edgeCount }: Props = $props();
	// Thickness ∝ shared-edge count, clamped so a busy bridge stays readable.
	const width = $derived(Math.min(6, 1 + edgeCount * 0.6));
</script>

<line {x1} {y1} {x2} {y2} stroke="#e8cf8f" stroke-opacity="0.22" stroke-width={width} stroke-linecap="round" />
```

- [ ] **Step 6: Wire bridges into `TierPanorama.svelte`**

Build a position map from the packed territories and render ribbons *beneath* the circles. Add imports + a derived map + geometry:

```ts
	import { bridgeGeometry } from '$lib/graph/atlas/layout/bridges';
	import BridgeRibbon from './marks/BridgeRibbon.svelte';

	const territoryPos = $derived(new Map(packed.map((t) => [t.id, { x: t.x, y: t.y }])));
	const bridgeLines = $derived(bridgeGeometry(overview.bridges, territoryPos));
```

Render the ribbons at the very start of the `<g transform={`translate(0, ${ZONE_BAND})`}>` block (line 63), before the territory circles so they sit underneath:

```svelte
	{#each bridgeLines as bl, i (i)}
		<BridgeRibbon x1={bl.x1} y1={bl.y1} x2={bl.x2} y2={bl.y2} edgeCount={bl.edgeCount} />
	{/each}
```

> Positions from `packTerritories` are relative to the `terrBox`; the ribbons render inside the same translated `<g>`, so they align with the circles. Verify visually.

- [ ] **Step 7: Typecheck + test**

Run: `cd packages/temper-ui && bun run check && bun run test -- bridges`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/bridges.ts packages/temper-ui/src/lib/graph/atlas/layout/bridges.test.ts packages/temper-ui/src/lib/components/graph/atlas/marks/BridgeRibbon.svelte packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte
git commit -m "feat(atlas): draw aggregate bridges as strength-weighted ribbons (G1/B, force-ready)"
```

---

## Final: full-suite gate + PR

- [ ] **Step 1: Full gates**

```bash
cd packages/temper-ui && bun run check && bun run test && bun run build
cd ../.. && cargo make check
```
Expected: all green.

- [ ] **Step 2: Push + open PR**

```bash
git push -u origin jct/atlas-c31-beat2a-shell-wayfinding
gh pr create --title "Graph Atlas C3.1 Beat 2a — shell, wayfinding & legibility" --body "…"
```

- [ ] **Step 3: Prod browser-verify (post-merge)** — collapsible rail; depth-aware crumb + `↑` ascend across team/cogmap/territory/node; filters popover badge; bottom-bar legend collapsed; ghost empty territories + real region label; aggregate bridges; Tier-2 label legibility.

---

## Self-Review

**Spec coverage:**
- L1 (collapsible sidebar + retire dock) → Tasks 7, 8 ✓
- W1 (depth-aware crumb + ascend + territory label) → Tasks 1, 2, 3, 4, 8 ✓
- Filters popover → Task 5, mounted Task 8 ✓
- L2 (legend bottom bar, collapsed) → Task 6, mounted Task 8 ✓
- L3 (empty ghost + region label) → Task 9 ✓
- G1 (bridges) → Task 11 ✓
- G2 (anchor+hover labels) → Task 10 ✓
- Crumb dedup (retire ScopeBar/CogmapCrumb) → Task 8 ✓
- Backend region label + ts-rs → Task 1 ✓

**Type consistency:** `TerritorySlice.label` (Task 1) consumed in loader + TierTerritory (Tasks 8, 9); `crumbTerritory`/`focusPath` produced in Task 8 loader, consumed by `crumbModel` (Task 3) via `AtlasCrumb` (Task 4); `parseFocusPath` (Task 2) used in loader (Task 8); `labelAnchors`/`truncateLabel` (Task 10) names match NodeChip/TierNeighborhood usage; `bridgeGeometry`/`BridgeLine` (Task 11) match BridgeRibbon props.

**Placeholder scan:** No TBD/TODO. `.svelte` edits that can't be fully literal (TerritoryCircle ghost styling, TrailRail positioning) are flagged with explicit "verify visually" notes and grounded line refs, not left vague.

**Known dependency to verify during build:** `packTerritories` output must carry `member_count` for Task 9's ghost flag and `{id,x,y}` for Task 11's position map — Task 9 Step 6 handles the passthrough if missing.
