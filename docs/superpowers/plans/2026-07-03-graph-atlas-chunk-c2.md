# Graph Atlas — Chunk C2 (Tier 1 & 2 renderers + drill + sparse-state) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the temper-ui Atlas interactive: relocate the route to `/graph/[owner]`, add a canonical `@me` membership home (you → teams), render the Tier-1 region interior (R3) and Tier-2 force-directed neighborhood (R4), wire click-to-drill across every element, turn the sparse / all-orphan Tier-0 state into a bounded interactive cogmap-as-territory view, and resolve the four C1-deferred items (I2/M4/M5/M6).

**Architecture:** d3 supplies math (layout, force, hull, zoom) as pure functions unit-tested under vitest/node; Svelte renders SVG reactively (`{#each}`) — components are verified by running the app, not render-tested. Navigation stays in the URL (`?team`, `?focus`); the `+page.server.ts` load derives the tier from `focus` and fetches the matching read (R2/R3/R4) server-side, or lists teams for the home when no team is scoped. This extends the C1 Atlas stack (PR #256); the old context-scoped graph is untouched (Chunk D deletes it).

**Tech Stack:** SvelteKit 2, Svelte 5 (runes), TypeScript 5, Tailwind v4, vitest 3 (node env), d3 submodules (`d3-hierarchy`, `d3-zoom`, `d3-selection`, **+ `d3-force`, `d3-polygon`**).

## Global Constraints

- **Svelte 5 runes only.** Props via `interface Props` + `let { … }: Props = $props()`; state via `$state`; derived via `$derived`/`$derived.by`; lifecycle via `onMount`/`onDestroy`. **No `$effect`.**
- **vitest:** node environment, `globals: false` — every test file imports `{ describe, it, expect }` from `vitest`. Test files are `src/**/*.test.ts`. Only pure functions are unit-tested; components are verified by running the app.
- **Generated types are read-only.** Import wire types from `$lib/types/generated/*`; never hand-model or edit them.
- **Server-only reads.** `apiGet`/`apiPost` and the `$lib/server/graph-reads.ts` wrappers may be imported **only** from `.server.ts` / `+server.ts`.
- **URL is the source of truth for navigation.** Change scope/focus with `goto(url, { replaceState: true })`; the load re-runs and returns the new tier's data.
- **Palette single source of truth:** all Atlas hues + edge styles come from `$lib/graph/atlas/palette.ts`. No color literal for graph semantics outside it.
- **Force runs ONLY on Tier 2** (parent spec D1). Tiers 0/1 use `d3-hierarchy` packing; never `d3-force`.
- **Indentation:** tabs. **d3:** import only the named submodules used; never the `d3` meta-package.

---

### Task 0: Add d3-force + d3-polygon dependencies

**Files:**
- Modify: `packages/temper-ui/package.json`, `packages/temper-ui/bun.lock`

- [ ] **Step 1: Add the runtime + type deps**

Run (from `packages/temper-ui/`):

```bash
cd packages/temper-ui
bun add d3-force@^3 d3-polygon@^3
bun add -d @types/d3-force@^3 @types/d3-polygon@^3
```

- [ ] **Step 2: Verify they resolve**

Run: `cd packages/temper-ui && bun run check`
Expected: svelte-check completes with 0 errors (current baseline).

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/package.json packages/temper-ui/bun.lock
git commit -m "chore(temper-ui): add d3-force/d3-polygon for the Atlas Tier-2 + hulls"
```

---

### Task 1: Relocate the route to `/graph/[owner]`

**Files:**
- Move: `packages/temper-ui/src/routes/(app)/vault/[owner]/graph/` → `packages/temper-ui/src/routes/(app)/graph/[owner]/`
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts:13,66`

The `nav.ts` URL builders are pathname-relative (`${u.pathname}${u.search}`), so their logic is unchanged — only the pathname they run at moves. No internal link points at the new Atlas route yet (the sole `graph` href in `ContextNavGroup.svelte` targets the *old* `/vault/[owner]/[context]/graph` stack, which Chunk D deletes — leave it). The old context graph route is untouched.

- [ ] **Step 1: Move the route directory**

```bash
cd packages/temper-ui
mkdir -p "src/routes/(app)/graph"
git mv "src/routes/(app)/vault/[owner]/graph" "src/routes/(app)/graph/[owner]"
```

- [ ] **Step 2: Update the nav test base URL**

In `src/lib/graph/atlas/nav.test.ts`, change line 13:

```ts
const url = (qs: string) => new URL(`https://x/graph/@me${qs}`);
```

and line 66:

```ts
		expect(buildScopeUrl(url('?team=old'), 'new').startsWith('/graph/@me?')).toBe(true);
```

- [ ] **Step 3: Run the nav tests + typecheck**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/nav.test.ts && bun run check`
Expected: nav tests PASS; svelte-check 0 errors (the route just moved; `$types` regenerate on check).

- [ ] **Step 4: Commit**

```bash
git add -A "packages/temper-ui/src/routes" packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): relocate route to /graph/[owner] (out of the vault tree)"
```

---

### Task 2: `nav.ts` — `buildHomeUrl` + null-team home state

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces: `buildHomeUrl(base: URL): string` — clears both `team` and `focus` (returns to the membership home). `parseTeam` (existing) returning `null` is the home signal.
- Consumed by: Task 15 (AtlasCanvas ascend), Task 11 (TierHome is shown when `parseTeam` is null).

- [ ] **Step 1: Write the failing test**

Add to `src/lib/graph/atlas/nav.test.ts` inside the `describe('URL builders', …)` block:

```ts
	it('buildHomeUrl clears BOTH team and focus (back to membership home)', () => {
		const p = new URL(buildHomeUrl(url('?team=t1&focus=node:n5')), 'https://x').searchParams;
		expect(p.get('team')).toBeNull();
		expect(p.get('focus')).toBeNull();
	});
```

and add `buildHomeUrl` to the import list at the top of the file.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/nav.test.ts`
Expected: FAIL — `buildHomeUrl` is not exported.

- [ ] **Step 3: Add the builder**

Append to `src/lib/graph/atlas/nav.ts`:

```ts
/** Return to the membership home: clear both team and focus. */
export function buildHomeUrl(base: URL): string {
	return withParams(base, (p) => {
		p.delete('team');
		p.delete('focus');
	});
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/nav.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): nav buildHomeUrl — clear team+focus for the membership home"
```

---

### Task 3: `palette.ts` — `edgeStyle()` (consume the edge grammar)

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/palette.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/palette.test.ts`

**Interfaces:**
- Consumes: `AtlasEdge` from `$lib/types/generated/graph_atlas`; the existing `EDGE_COLORS`.
- Produces: `interface EdgeStyle { color: string; width: number; dash: string | null; markerStart: boolean; markerEnd: boolean }`, `edgeStyle(edge: AtlasEdge): EdgeStyle`.
- Consumed by: Task 10 (`Edge.svelte`).

Grammar (spec C2-D6): `edge_kind` → line style (`contains`=solid, `leads_to`=dashed `7 4`, `express`=dotted `1 4`, `near`=short-dash `4 4`); `label==="derived_from"` → provenance color + dashed; `label==="contradicts"` → contradicts-red; else structural gray. `weight` → thickness clamped `[1,5]`. `polarity` → arrowhead: `near` is symmetric (no marker); else `forward`→end marker, `inverse`→start marker.

- [ ] **Step 1: Write the failing test**

Add to `src/lib/graph/atlas/palette.test.ts`:

```ts
import { EDGE_COLORS, edgeStyle } from './palette';
import type { AtlasEdge } from '$lib/types/generated/graph_atlas';

const edge = (o: Partial<AtlasEdge>): AtlasEdge => ({
	source: 's',
	target: 't',
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1,
	...o
});

describe('edgeStyle', () => {
	it('maps edge_kind to line style', () => {
		expect(edgeStyle(edge({ edge_kind: 'contains' })).dash).toBeNull();
		expect(edgeStyle(edge({ edge_kind: 'leads_to' })).dash).toBe('7 4');
		expect(edgeStyle(edge({ edge_kind: 'express' })).dash).toBe('1 4');
		expect(edgeStyle(edge({ edge_kind: 'near' })).dash).toBe('4 4');
	});
	it('derived_from label → provenance color + dashed regardless of kind', () => {
		const s = edgeStyle(edge({ edge_kind: 'contains', label: 'derived_from' }));
		expect(s.color).toBe(EDGE_COLORS.derived);
		expect(s.dash).toBe('7 4');
	});
	it('contradicts label → warning red', () => {
		expect(edgeStyle(edge({ label: 'contradicts' })).color).toBe(EDGE_COLORS.contradicts);
	});
	it('default color is structural gray', () => {
		expect(edgeStyle(edge({})).color).toBe(EDGE_COLORS.structural);
	});
	it('weight → thickness clamped to [1,5]', () => {
		expect(edgeStyle(edge({ weight: 0.2 })).width).toBe(1);
		expect(edgeStyle(edge({ weight: 3 })).width).toBe(3);
		expect(edgeStyle(edge({ weight: 99 })).width).toBe(5);
	});
	it('polarity → arrowhead; near is symmetric (no marker)', () => {
		expect(edgeStyle(edge({ polarity: 'forward' }))).toMatchObject({ markerEnd: true, markerStart: false });
		expect(edgeStyle(edge({ polarity: 'inverse' }))).toMatchObject({ markerEnd: false, markerStart: true });
		const n = edgeStyle(edge({ edge_kind: 'near', polarity: 'forward' }));
		expect(n.markerStart).toBe(false);
		expect(n.markerEnd).toBe(false);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/palette.test.ts`
Expected: FAIL — `edgeStyle` not exported.

- [ ] **Step 3: Add the implementation**

Append to `src/lib/graph/atlas/palette.ts` (add the `AtlasEdge` import to the existing import from `graph_atlas`):

```ts
export interface EdgeStyle {
	color: string;
	width: number;
	dash: string | null;
	markerStart: boolean;
	markerEnd: boolean;
}

const KIND_DASH: Record<AtlasEdge['edge_kind'], string | null> = {
	contains: null,
	leads_to: '7 4',
	express: '1 4',
	near: '4 4'
};

/** Map an Atlas edge to its SVG style per the encoding grammar (spec C2-D6). */
export function edgeStyle(edge: AtlasEdge): EdgeStyle {
	const color =
		edge.label === 'derived_from'
			? EDGE_COLORS.derived
			: edge.label === 'contradicts'
				? EDGE_COLORS.contradicts
				: EDGE_COLORS.structural;
	const dash = edge.label === 'derived_from' ? '7 4' : KIND_DASH[edge.edge_kind];
	const width = Math.max(1, Math.min(5, edge.weight));
	const symmetric = edge.edge_kind === 'near';
	return {
		color,
		width,
		dash,
		markerStart: !symmetric && edge.polarity === 'inverse',
		markerEnd: !symmetric && edge.polarity === 'forward'
	};
}
```

Change the existing import line `import type { NodeHome } from '$lib/types/generated/graph_atlas';` to also import `AtlasEdge`:

```ts
import type { AtlasEdge, NodeHome } from '$lib/types/generated/graph_atlas';
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/palette.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/palette.ts packages/temper-ui/src/lib/graph/atlas/palette.test.ts
git commit -m "feat(atlas): palette edgeStyle — consume the Tier-2 edge grammar"
```

---

### Task 4: I2 — size regions by salience, contexts by member_count

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.test.ts`

**Interfaces:**
- Unchanged signature: `packTerritories(territories: Territory[], size): PositionedTerritory[]`.
- Change: the pack **weight** is now `region → salience`, `context/cogmap → member_count`, each on a common scale. `PositionedTerritory` fields are unchanged.

Rationale (spec C2-D7 · I2): the wire contract says `salience` sizes regions (`member_count` is `None`-relevant only for contexts). Regions carry `salience ∈ [0,1]`; contexts carry integer `member_count`. Normalize to a shared weight so one pack call mixes them: region weight = `max(1, round(salience * 100))`; context/cogmap weight = `max(1, member_count)`.

- [ ] **Step 1: Update the failing test**

In `packTerritories.test.ts`, replace the `'sizes radius monotonically with member_count'` test with:

```ts
	it('sizes regions by salience and contexts by member_count', () => {
		const region = (id: string, salience: number): Territory => ({
			id,
			kind: 'region',
			label: id,
			member_count: 1,
			salience,
			anchor_id: `anchor-${id}`
		});
		const context = (id: string, member_count: number): Territory => ({
			id,
			kind: 'context',
			label: id,
			member_count,
			salience: null,
			anchor_id: `anchor-${id}`
		});
		const out = packTerritories(
			[region('hi', 0.9), region('lo', 0.1), context('big', 80), context('small', 2)],
			{ width: 500, height: 500 }
		);
		const r = (id: string) => out.find((p) => p.id === id)!;
		expect(r('hi').r).toBeGreaterThan(r('lo').r); // salience sizes regions
		expect(r('big').r).toBeGreaterThan(r('small').r); // member_count sizes contexts
	});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/packTerritories.test.ts`
Expected: FAIL — regions currently all size by `member_count` (=1 here), so `hi` and `lo` tie.

- [ ] **Step 3: Update the weight accessor**

In `packTerritories.ts`, replace the `.sum(...)` line:

```ts
		.sum((d) => {
			if (!d.territory) return 0;
			const t = d.territory;
			return t.kind === 'region'
				? Math.max(1, Math.round((t.salience ?? 0) * 100))
				: Math.max(1, t.member_count);
		});
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/packTerritories.test.ts`
Expected: PASS (and the other packTerritories cases still pass).

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.test.ts
git commit -m "feat(atlas): I2 — regions sized by salience, contexts by member_count"
```

---

### Task 5: `cogmapTerritories` layout (sparse cogmap-as-territory)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/cogmapTerritories.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/cogmapTerritories.test.ts`

**Interfaces:**
- Consumes: `OrphanNode` from `$lib/types/generated/graph_territory`; `pack`, `hierarchy` from `d3-hierarchy`.
- Produces:
  ```ts
  interface PositionedFacet { id: string; title: string; docType: string | null; x: number; y: number; r: number }
  interface CogmapTerritory { cogmapId: string; label: string; facetCount: number; x: number; y: number; r: number; facets: PositionedFacet[] }
  packCogmapTerritories(orphans: OrphanNode[], size: { width: number; height: number }): CogmapTerritory[]
  ```
- Consumed by: Task 14 (TierPanorama sparse rendering).

Group orphans by `anchor_id` (their cogmap) → one `CogmapTerritory` per cogmap, packed among each other by facet count; each cogmap's facets are packed *inside* its circle by degree (`d3-pack` again, in the cogmap's local box). `OrphanNode` carries no cogmap name, so `label` is a generic `"cogmap · <n> facets"` (followup: R2 could carry the cogmap name for orphan anchors — track as an Atlas Home / backend followup).

- [ ] **Step 1: Write the failing test**

```ts
// cogmapTerritories.test.ts
import { describe, expect, it } from 'vitest';
import { packCogmapTerritories } from './cogmapTerritories';
import type { OrphanNode } from '$lib/types/generated/graph_territory';

const orphan = (id: string, anchor: string, degree = 1): OrphanNode => ({
	id,
	title: id,
	doc_type: 'concept',
	degree,
	anchor_id: anchor
});

describe('packCogmapTerritories', () => {
	it('groups orphans by anchor_id into one territory per cogmap', () => {
		const out = packCogmapTerritories(
			[orphan('a', 'cm1'), orphan('b', 'cm1'), orphan('c', 'cm2')],
			{ width: 800, height: 500 }
		);
		expect(out).toHaveLength(2);
		const cm1 = out.find((t) => t.cogmapId === 'cm1')!;
		expect(cm1.facetCount).toBe(2);
		expect(cm1.facets.map((f) => f.id).sort()).toEqual(['a', 'b']);
	});
	it('packs each territory inside the box and its facets inside the territory', () => {
		const out = packCogmapTerritories(
			[orphan('a', 'cm1'), orphan('b', 'cm1'), orphan('c', 'cm1')],
			{ width: 400, height: 400 }
		);
		const t = out[0];
		expect(t.x - t.r).toBeGreaterThanOrEqual(0);
		expect(t.x + t.r).toBeLessThanOrEqual(400);
		for (const f of t.facets) {
			// facet centre lies within the territory circle
			const d = Math.hypot(f.x - t.x, f.y - t.y);
			expect(d).toBeLessThanOrEqual(t.r);
		}
	});
	it('labels generically (no cogmap name in the wire) and returns [] for no orphans', () => {
		expect(packCogmapTerritories([orphan('a', 'cm1')], { width: 200, height: 200 })[0].label).toContain('cogmap');
		expect(packCogmapTerritories([], { width: 10, height: 10 })).toEqual([]);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/cogmapTerritories.test.ts`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```ts
// cogmapTerritories.ts
/**
 * Sparse-state layout (spec C2-D4): orphan nodes carry the cogmap they're homed
 * in (anchor_id). Group them into synthetic cogmap territories and pack each
 * cogmap's facets INSIDE its hull — the same cartographic language as dense
 * territories, but for region-less cogmaps. Pure; no force (Tiers 0/1 pack).
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { OrphanNode } from '$lib/types/generated/graph_territory';

export interface PositionedFacet {
	id: string;
	title: string;
	docType: string | null;
	x: number;
	y: number;
	r: number;
}

export interface CogmapTerritory {
	cogmapId: string;
	label: string;
	facetCount: number;
	x: number;
	y: number;
	r: number;
	facets: PositionedFacet[];
}

interface Node {
	orphan?: OrphanNode;
	cogmapId?: string;
	children?: Node[];
}

export function packCogmapTerritories(
	orphans: OrphanNode[],
	size: { width: number; height: number }
): CogmapTerritory[] {
	if (orphans.length === 0) return [];

	// group by cogmap (anchor_id), stable insertion order
	const groups = new Map<string, OrphanNode[]>();
	for (const o of orphans) {
		const g = groups.get(o.anchor_id);
		if (g) g.push(o);
		else groups.set(o.anchor_id, [o]);
	}

	// outer pack: one leaf per cogmap, sized by facet count
	const root = hierarchy<Node>({
		children: [...groups.entries()].map(([cogmapId, facets]) => ({
			cogmapId,
			children: facets.map((orphan) => ({ orphan }))
		}))
	}).sum((d) => (d.orphan ? 1 : 0));

	const packed = pack<Node>().size([size.width, size.height]).padding(14)(root);

	return (packed.children ?? []).map((group) => {
		const cogmapId = group.data.cogmapId!;
		const facets: PositionedFacet[] = (group.children ?? []).map((leaf) => ({
			id: leaf.data.orphan!.id,
			title: leaf.data.orphan!.title,
			docType: leaf.data.orphan!.doc_type,
			x: leaf.x,
			y: leaf.y,
			r: Math.max(4, leaf.r)
		}));
		return {
			cogmapId,
			label: `cogmap · ${facets.length} facets`,
			facetCount: facets.length,
			x: group.x,
			y: group.y,
			r: group.r,
			facets
		};
	});
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/cogmapTerritories.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/cogmapTerritories.ts packages/temper-ui/src/lib/graph/atlas/layout/cogmapTerritories.test.ts
git commit -m "feat(atlas): cogmapTerritories — sparse cogmap-as-territory packing (M5)"
```

---

### Task 6: `regionInterior` layout (Tier 1 members by affinity)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/regionInterior.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/regionInterior.test.ts`

**Interfaces:**
- Consumes: `RegionMember` from `$lib/types/generated/graph_territory`; `pack`, `hierarchy` from `d3-hierarchy`.
- Produces:
  ```ts
  interface PositionedMember { id: string; title: string; docType: string | null; affinity: number | null; x: number; y: number; r: number }
  packRegionMembers(members: RegionMember[], size: { width: number; height: number }): PositionedMember[]
  ```
- Consumed by: Task 12 (TierTerritory).

Members packed by `affinity` (higher = larger). `affinity` is nullable → floor weight 1. Weight = `max(1, round((affinity ?? 0) * 100))`.

- [ ] **Step 1: Write the failing test**

```ts
// regionInterior.test.ts
import { describe, expect, it } from 'vitest';
import { packRegionMembers } from './regionInterior';
import type { RegionMember } from '$lib/types/generated/graph_territory';

const member = (id: string, affinity: number | null): RegionMember => ({
	id,
	title: id,
	doc_type: 'concept',
	affinity
});

describe('packRegionMembers', () => {
	it('returns one positioned circle per member, inside the box', () => {
		const out = packRegionMembers([member('a', 0.9), member('b', 0.3), member('c', null)], {
			width: 400,
			height: 300
		});
		expect(out).toHaveLength(3);
		for (const p of out) {
			expect(p.x - p.r).toBeGreaterThanOrEqual(0);
			expect(p.x + p.r).toBeLessThanOrEqual(400);
			expect(p.r).toBeGreaterThan(0);
		}
	});
	it('sizes radius monotonically with affinity; null floors', () => {
		const out = packRegionMembers([member('hi', 0.9), member('lo', 0.05), member('none', null)], {
			width: 400,
			height: 400
		});
		const r = (id: string) => out.find((p) => p.id === id)!.r;
		expect(r('hi')).toBeGreaterThan(r('lo'));
		expect(r('none')).toBeLessThanOrEqual(r('lo'));
	});
	it('returns [] for no members', () => {
		expect(packRegionMembers([], { width: 10, height: 10 })).toEqual([]);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/regionInterior.test.ts`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```ts
// regionInterior.ts
/**
 * Tier-1 region interior layout (spec C2-D5): pack the region's members by
 * affinity inside the hull. Members are the payload; components are surfaced as
 * a badge by the component, not spatially (R3 gives no member→component map).
 * Pure; no force.
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { RegionMember } from '$lib/types/generated/graph_territory';

export interface PositionedMember {
	id: string;
	title: string;
	docType: string | null;
	affinity: number | null;
	x: number;
	y: number;
	r: number;
}

interface Node {
	member?: RegionMember;
	children?: Node[];
}

export function packRegionMembers(
	members: RegionMember[],
	size: { width: number; height: number }
): PositionedMember[] {
	if (members.length === 0) return [];

	const root = hierarchy<Node>({
		children: members.map((member) => ({ member }))
	}).sum((d) => (d.member ? Math.max(1, Math.round((d.member.affinity ?? 0) * 100)) : 0));

	const packed = pack<Node>().size([size.width, size.height]).padding(8)(root);

	return packed.leaves().map((leaf) => {
		const m = leaf.data.member!;
		return {
			id: m.id,
			title: m.title,
			docType: m.doc_type,
			affinity: m.affinity,
			x: leaf.x,
			y: leaf.y,
			r: Math.max(4, leaf.r)
		};
	});
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/regionInterior.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/regionInterior.ts packages/temper-ui/src/lib/graph/atlas/layout/regionInterior.test.ts
git commit -m "feat(atlas): regionInterior — Tier-1 members packed by affinity"
```

---

### Task 7: `hull` layout (d3-polygon convex hull → SVG path)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/hull.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/hull.test.ts`

**Interfaces:**
- Consumes: `polygonHull` from `d3-polygon`.
- Produces: `hullPath(points: [number, number][], padding?: number): string | null` — a closed SVG path around the convex hull, or `null` when fewer than 3 points (a hull is undefined). `padding` expands each hull vertex outward from the centroid.
- Consumed by: Task 12 (TierTerritory), Task 13 (TierNeighborhood).

- [ ] **Step 1: Write the failing test**

```ts
// hull.test.ts
import { describe, expect, it } from 'vitest';
import { hullPath } from './hull';

describe('hullPath', () => {
	it('returns null for fewer than 3 points', () => {
		expect(hullPath([])).toBeNull();
		expect(hullPath([[0, 0]])).toBeNull();
		expect(hullPath([[0, 0], [1, 1]])).toBeNull();
	});
	it('returns a closed path string for a triangle', () => {
		const p = hullPath([[0, 0], [10, 0], [5, 10]]);
		expect(p).toMatch(/^M/);
		expect(p).toMatch(/Z$/);
	});
	it('padding pushes vertices outward (path differs from unpadded)', () => {
		const pts: [number, number][] = [[0, 0], [10, 0], [10, 10], [0, 10]];
		expect(hullPath(pts, 8)).not.toBe(hullPath(pts, 0));
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/hull.test.ts`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```ts
// hull.ts
/**
 * Convex-hull outline for a region/neighborhood (spec C2-D5/D6). d3-polygon
 * computes the hull; we emit a padded closed SVG path. Pure.
 */
import { polygonHull } from 'd3-polygon';

export function hullPath(points: [number, number][], padding = 0): string | null {
	if (points.length < 3) return null;
	const hull = polygonHull(points);
	if (!hull) return null;

	const cx = hull.reduce((s, p) => s + p[0], 0) / hull.length;
	const cy = hull.reduce((s, p) => s + p[1], 0) / hull.length;

	const expanded = hull.map(([x, y]) => {
		if (padding === 0) return [x, y];
		const dx = x - cx;
		const dy = y - cy;
		const len = Math.hypot(dx, dy) || 1;
		return [x + (dx / len) * padding, y + (dy / len) * padding];
	});

	return `M${expanded.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join('L')}Z`;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/hull.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/hull.ts packages/temper-ui/src/lib/graph/atlas/layout/hull.test.ts
git commit -m "feat(atlas): hull — d3-polygon convex-hull path for region/neighborhood"
```

---

### Task 8: `forceNeighborhood` layout (Tier 2, d3-force)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.test.ts`

**Interfaces:**
- Consumes: `AtlasSubgraph`, `AtlasNode`, `AtlasEdge` from `$lib/types/generated/graph_atlas`; `forceSimulation`, `forceLink`, `forceManyBody`, `forceCenter`, `forceCollide` from `d3-force`.
- Produces:
  ```ts
  interface ForceNode { id: string; title: string; docType: string | null; home: AtlasNode['home']; degree: number; isSeed: boolean; x: number; y: number }
  interface ForceEdge { edge: AtlasEdge; source: ForceNode; target: ForceNode }
  interface ForceGraph { nodes: ForceNode[]; edges: ForceEdge[] }
  forceNeighborhood(subgraph: AtlasSubgraph, seeds: string[], size: { width: number; height: number }): ForceGraph
  ```
- Consumed by: Task 13 (TierNeighborhood).

The simulation runs synchronously (`.stop()` then fixed `.tick()` loop) so the component gets final positions without an animation loop. Only the deterministic wiring is unit-tested (node/edge count, seed flag, endpoint resolution, degree carry) — never exact coordinates. d3-force placement is deterministic (phyllotaxis init; no `Math.random`), but positions stay untested to avoid brittleness. Edges whose endpoints aren't both present are dropped (defensive; R4 guarantees they are).

- [ ] **Step 1: Write the failing test**

```ts
// forceNeighborhood.test.ts
import { describe, expect, it } from 'vitest';
import { forceNeighborhood } from './forceNeighborhood';
import type { AtlasEdge, AtlasNode, AtlasSubgraph } from '$lib/types/generated/graph_atlas';

const node = (id: string, degree = 1): AtlasNode => ({
	id,
	title: id,
	doc_type: 'concept',
	home: 'cogmap',
	degree,
	salience: null
});
const edge = (source: string, target: string): AtlasEdge => ({
	source,
	target,
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1
});
const graph = (nodes: AtlasNode[], edges: AtlasEdge[]): AtlasSubgraph => ({ nodes, edges });

describe('forceNeighborhood', () => {
	it('positions every node and flags the seed(s)', () => {
		const out = forceNeighborhood(graph([node('s'), node('n1'), node('n2')], [edge('s', 'n1'), edge('s', 'n2')]), ['s'], {
			width: 600,
			height: 400
		});
		expect(out.nodes).toHaveLength(3);
		expect(out.nodes.find((n) => n.id === 's')!.isSeed).toBe(true);
		expect(out.nodes.find((n) => n.id === 'n1')!.isSeed).toBe(false);
		for (const n of out.nodes) {
			expect(Number.isFinite(n.x)).toBe(true);
			expect(Number.isFinite(n.y)).toBe(true);
		}
	});
	it('resolves edge endpoints to node objects and carries degree', () => {
		const out = forceNeighborhood(graph([node('s', 5), node('n1', 2)], [edge('s', 'n1')]), ['s'], {
			width: 600,
			height: 400
		});
		expect(out.edges).toHaveLength(1);
		expect(out.edges[0].source.id).toBe('s');
		expect(out.edges[0].target.id).toBe('n1');
		expect(out.nodes.find((n) => n.id === 's')!.degree).toBe(5);
	});
	it('drops edges whose endpoints are missing', () => {
		const out = forceNeighborhood(graph([node('s')], [edge('s', 'ghost')]), ['s'], { width: 100, height: 100 });
		expect(out.edges).toHaveLength(0);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/forceNeighborhood.test.ts`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```ts
// forceNeighborhood.ts
/**
 * Tier-2 neighborhood layout (spec C2-D6): the ONLY place d3-force runs. Builds
 * a force graph from an R4 AtlasSubgraph, runs the simulation synchronously to a
 * settled state, and returns final node/edge positions. Pure w.r.t. inputs; the
 * simulation is deterministic (phyllotaxis init, no Math.random).
 */
import {
	forceCenter,
	forceCollide,
	forceLink,
	forceManyBody,
	forceSimulation,
	type SimulationNodeDatum
} from 'd3-force';
import type { AtlasEdge, AtlasNode, AtlasSubgraph } from '$lib/types/generated/graph_atlas';

export interface ForceNode extends SimulationNodeDatum {
	id: string;
	title: string;
	docType: string | null;
	home: AtlasNode['home'];
	degree: number;
	isSeed: boolean;
	x: number;
	y: number;
}

export interface ForceEdge {
	edge: AtlasEdge;
	source: ForceNode;
	target: ForceNode;
}

export interface ForceGraph {
	nodes: ForceNode[];
	edges: ForceEdge[];
}

const TICKS = 300;

export function forceNeighborhood(
	subgraph: AtlasSubgraph,
	seeds: string[],
	size: { width: number; height: number }
): ForceGraph {
	const seedSet = new Set(seeds);
	const nodes: ForceNode[] = subgraph.nodes.map((n) => ({
		id: n.id,
		title: n.title,
		docType: n.doc_type,
		home: n.home,
		degree: n.degree,
		isSeed: seedSet.has(n.id),
		x: 0,
		y: 0
	}));
	const byId = new Map(nodes.map((n) => [n.id, n]));

	const links = subgraph.edges
		.map((edge) => {
			const source = byId.get(edge.source);
			const target = byId.get(edge.target);
			return source && target ? { edge, source, target } : null;
		})
		.filter((l): l is ForceEdge => l !== null);

	const sim = forceSimulation(nodes)
		.force(
			'link',
			forceLink(links.map((l) => ({ source: l.source, target: l.target }))).distance(90).strength(0.6)
		)
		.force('charge', forceManyBody().strength(-260))
		.force('center', forceCenter(size.width / 2, size.height / 2))
		.force('collide', forceCollide<ForceNode>().radius((n) => 12 + Math.min(10, n.degree)))
		.stop();

	for (let i = 0; i < TICKS; i++) sim.tick();

	return { nodes, edges: links };
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/forceNeighborhood.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.ts packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.test.ts
git commit -m "feat(atlas): forceNeighborhood — Tier-2 d3-force layout (synchronous settle)"
```

---

### Task 9: `homeLayout` (you → teams)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.test.ts`

**Interfaces:**
- Consumes: `TeamRow` from `$lib/types/generated/team`.
- Produces:
  ```ts
  interface HomeNode { id: string; name: string; x: number; y: number; kind: 'you' | 'team' }
  interface HomeEdge { fromX: number; fromY: number; toX: number; toY: number }
  interface HomeGraph { you: HomeNode; teams: HomeNode[]; edges: HomeEdge[] }
  layoutHome(teams: TeamRow[], size: { width: number; height: number }): HomeGraph
  ```
- Consumed by: Task 11 (TierHome). Deterministic two-column layout — the seed of the Atlas Home membership graph (which later adds a cogmap column).

- [ ] **Step 1: Write the failing test**

```ts
// homeLayout.test.ts
import { describe, expect, it } from 'vitest';
import { layoutHome } from './homeLayout';
import type { TeamRow } from '$lib/types/generated/team';

const team = (id: string, name: string): TeamRow =>
	({ id, slug: name, name, description: null }) as unknown as TeamRow;

describe('layoutHome', () => {
	it('places you at left, teams in a right column, one edge each', () => {
		const g = layoutHome([team('t1', 'A'), team('t2', 'B'), team('t3', 'C')], { width: 800, height: 400 });
		expect(g.you.kind).toBe('you');
		expect(g.teams).toHaveLength(3);
		expect(g.edges).toHaveLength(3);
		for (const t of g.teams) {
			expect(t.x).toBeGreaterThan(g.you.x); // teams are to the right of you
			expect(t.x).toBeLessThanOrEqual(800);
			expect(t.y).toBeGreaterThanOrEqual(0);
			expect(t.y).toBeLessThanOrEqual(400);
		}
	});
	it('handles zero teams (you alone, no edges)', () => {
		const g = layoutHome([], { width: 400, height: 300 });
		expect(g.teams).toEqual([]);
		expect(g.edges).toEqual([]);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/homeLayout.test.ts`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```ts
// homeLayout.ts
/**
 * Canonical @me home layout (spec C2-D2): you → teams, the membership-graph half
 * C2 ships. Deterministic two-column layout; the Atlas Home chunk grows a third
 * (cogmap) column onto this same shape. Pure.
 */
import type { TeamRow } from '$lib/types/generated/team';

export interface HomeNode {
	id: string;
	name: string;
	x: number;
	y: number;
	kind: 'you' | 'team';
}

export interface HomeEdge {
	fromX: number;
	fromY: number;
	toX: number;
	toY: number;
}

export interface HomeGraph {
	you: HomeNode;
	teams: HomeNode[];
	edges: HomeEdge[];
}

export function layoutHome(teams: TeamRow[], size: { width: number; height: number }): HomeGraph {
	const you: HomeNode = { id: 'you', name: 'you', x: size.width * 0.16, y: size.height / 2, kind: 'you' };
	const teamX = size.width * 0.58;
	const top = size.height * 0.12;
	const span = size.height * 0.76;
	const step = teams.length > 1 ? span / (teams.length - 1) : 0;

	const teamNodes: HomeNode[] = teams.map((t, i) => ({
		id: t.id,
		name: t.name,
		x: teamX,
		y: teams.length === 1 ? size.height / 2 : top + i * step,
		kind: 'team'
	}));

	const edges: HomeEdge[] = teamNodes.map((t) => ({ fromX: you.x, fromY: you.y, toX: t.x, toY: t.y }));

	return { you, teams: teamNodes, edges };
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/homeLayout.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.ts packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.test.ts
git commit -m "feat(atlas): homeLayout — you → teams membership half (C2 slice)"
```

---

### Task 10: SVG marks — NodeChip, Edge, MemberChip, RegionHull + OrphanNode facet upgrade

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeChip.svelte`
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/Edge.svelte`
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/MemberChip.svelte`
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/RegionHull.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/OrphanNodeMark.svelte`

**Interfaces:**
- `NodeChip` props: `{ x; y; r; title; docType: string | null; home: 'context' | 'cogmap'; seed?: boolean; onEnter?: () => void }`.
- `Edge` props: `{ x1; y1; x2; y2; edge: AtlasEdge; label?: boolean }`.
- `MemberChip` props: `{ x; y; r; title; docType: string | null; onEnter: () => void }`.
- `RegionHull` props: `{ d: string; label: string | null; tint: string }`.
- `OrphanNodeMark` (upgraded) props: `{ x; y; r?; title; docType: string | null; onEnter?: () => void }` — now clickable + label-on-hover.
- Consumed by: Tasks 12/13/14.

These are presentational SVG fragments, verified visually in Task 17.

- [ ] **Step 1: Write `NodeChip.svelte`**

```svelte
<script lang="ts">
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		home: 'context' | 'cogmap';
		seed?: boolean;
		onEnter?: () => void;
	}
	let { x, y, r, title, docType, home, seed = false, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(home === 'cogmap');
</script>

<g
	class="node-chip"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	{#if seed}
		<circle cx={x} cy={y} r={r + 6} fill="none" stroke="#cfd6e2" stroke-width="1.5" />
	{/if}
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill="#1b1e26" stroke={color} stroke-width="2.5" />
	{/if}
	<text x={x} y={y + r + 13} text-anchor="middle" fill="#c7d0da" font-size="10">{title}</text>
</g>
```

- [ ] **Step 2: Write `Edge.svelte`**

```svelte
<script lang="ts">
	import type { AtlasEdge } from '$lib/types/generated/graph_atlas';
	import { edgeStyle } from '$lib/graph/atlas/palette';

	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		edge: AtlasEdge;
		label?: boolean;
	}
	let { x1, y1, x2, y2, edge, label = false }: Props = $props();

	const s = $derived(edgeStyle(edge));
	const midX = $derived((x1 + x2) / 2);
	const midY = $derived((y1 + y2) / 2);
</script>

<g class="edge">
	<line
		{x1}
		{y1}
		{x2}
		{y2}
		stroke={s.color}
		stroke-width={s.width}
		stroke-dasharray={s.dash ?? undefined}
		marker-end={s.markerEnd ? 'url(#arrow-end)' : undefined}
		marker-start={s.markerStart ? 'url(#arrow-start)' : undefined}
	/>
	{#if label && edge.label}
		<text x={midX} y={midY - 3} text-anchor="middle" fill="#c9b183" font-size="9">{edge.label}</text>
	{/if}
</g>
```

- [ ] **Step 3: Write `MemberChip.svelte`**

```svelte
<script lang="ts">
	import { docTypeHue, isAuthored } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		onEnter: () => void;
	}
	let { x, y, r, title, docType, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(isAuthored(docType));
</script>

<g
	class="member-chip"
	role="button"
	tabindex="0"
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter()}
	style="cursor:pointer"
>
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill="#1b1e26" stroke={color} stroke-width="2.2" />
	{/if}
	<text x={x} y={y + r + 12} text-anchor="middle" fill="#c7d0da" font-size="10">{title}</text>
</g>
```

- [ ] **Step 4: Write `RegionHull.svelte`**

```svelte
<script lang="ts">
	interface Props {
		d: string;
		label: string | null;
		tint: string;
	}
	let { d, label, tint }: Props = $props();
</script>

<g class="region-hull">
	<path {d} fill={tint} fill-opacity="0.05" stroke={tint} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="7 5" />
	{#if label}
		<text fill={tint} font-size="11" font-weight="600" letter-spacing="1" style="text-transform:uppercase" dx="6" dy="-6">
			<textPath href="">{label}</textPath>
		</text>
	{/if}
</g>
```

> Note: the `<textPath>` here is a placeholder anchor; the consuming component (Task 12) positions the label explicitly and passes `label={null}` if it renders its own. Keep `RegionHull` focused on the path; the label prop is a convenience the caller may ignore.

- [ ] **Step 5: Upgrade `OrphanNodeMark.svelte` (clickable facet + hover label)**

Replace the file contents with:

```svelte
<script lang="ts">
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r?: number;
		title: string;
		docType: string | null;
		onEnter?: () => void;
	}
	let { x, y, r = 5, title, docType, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	let hovered = $state(false);
</script>

<g
	class="orphan"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	onmouseenter={() => (hovered = true)}
	onmouseleave={() => (hovered = false)}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	<circle cx={x} cy={y} {r} fill={color} />
	{#if hovered}
		<text x={x + r + 4} y={y + 3} fill="#e6edf5" font-size="10">{title}</text>
	{/if}
</g>
```

- [ ] **Step 6: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 7: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/marks/"
git commit -m "feat(atlas): marks — NodeChip, Edge, MemberChip, RegionHull + clickable orphan facet"
```

---

### Task 11: `TierHome` component (you → teams)

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte`

**Interfaces:**
- Consumes: `TeamRow` (`$lib/types/generated/team`); `layoutHome` (Task 9); `buildScopeUrl` (`nav.ts`); `goto` (`$app/navigation`), `page` (`$app/stores`).
- Props: `{ teams: TeamRow[]; width: number; height: number }`.
- Consumed by: Task 15 (AtlasCanvas).

Renders the `you` node + team doors + edges. Clicking a team enters its Tier-0 via `goto(buildScopeUrl(...))`.

- [ ] **Step 1: Write the component**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TeamRow } from '$lib/types/generated/team';
	import { layoutHome } from '$lib/graph/atlas/layout/homeLayout';
	import { buildScopeUrl } from '$lib/graph/atlas/nav';

	interface Props {
		teams: TeamRow[];
		width: number;
		height: number;
	}
	let { teams, width, height }: Props = $props();

	const g = $derived(layoutHome(teams, { width, height }));

	function enterTeam(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
</script>

<text x={width / 2} y="28" text-anchor="middle" fill="#5f7686" font-size="11" letter-spacing="1">YOUR TEAMS</text>

{#each g.edges as e, i (i)}
	<line x1={e.fromX} y1={e.fromY} x2={e.toX} y2={e.toY} stroke="#8b93a5" stroke-opacity="0.5" />
{/each}

<circle cx={g.you.x} cy={g.you.y} r="22" fill="#cfd6e2" fill-opacity="0.14" stroke="#cfd6e2" stroke-width="1.5" />
<text x={g.you.x} y={g.you.y + 4} text-anchor="middle" fill="#cfd6e2" font-size="11">you</text>

{#each g.teams as t (t.id)}
	<g role="button" tabindex="0" onclick={() => enterTeam(t.id)} onkeydown={(e) => e.key === 'Enter' && enterTeam(t.id)} style="cursor:pointer">
		<rect x={t.x - 90} y={t.y - 19} width="180" height="38" rx="8" fill="#3a8ae8" fill-opacity="0.12" stroke="#6fa8c7" stroke-opacity="0.6" />
		<text x={t.x} y={t.y + 4} text-anchor="middle" fill="#9fc4d6" font-size="11" font-weight="600">{t.name} ↵</text>
	</g>
{/each}
```

- [ ] **Step 2: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte"
git commit -m "feat(atlas): TierHome — canonical @me you → teams membership home"
```

---

### Task 12: `TierTerritory` component (Tier 1 region interior)

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte`

**Interfaces:**
- Consumes: `TerritorySlice` (`$lib/types/generated/graph_territory`); `packRegionMembers` (Task 6); `hullPath` (Task 7); `MemberChip` (Task 10); `buildDrillNodeUrl` (`nav.ts`); `TERRITORY_TINTS` (`palette.ts`); `goto`, `page`.
- Props: `{ slice: TerritorySlice; width: number; height: number }`.
- Consumed by: Task 15 (AtlasCanvas).

Members packed by affinity inside a hull; a "N sub-clusters" badge for the components count; clicking a member → Tier 2.

- [ ] **Step 1: Write the component**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritorySlice } from '$lib/types/generated/graph_territory';
	import { packRegionMembers } from '$lib/graph/atlas/layout/regionInterior';
	import { hullPath } from '$lib/graph/atlas/layout/hull';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import MemberChip from './marks/MemberChip.svelte';

	interface Props {
		slice: TerritorySlice;
		width: number;
		height: number;
	}
	let { slice, width, height }: Props = $props();

	const members = $derived(packRegionMembers(slice.members, { width, height: Math.max(1, height - 60) }));
	const hull = $derived(hullPath(members.map((m) => [m.x, m.y] as [number, number]), 26));

	function drill(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId), { replaceState: true });
	}
</script>

<g transform="translate(0, 40)">
	{#if hull}
		<path d={hull} fill={TERRITORY_TINTS.region} fill-opacity="0.05" stroke={TERRITORY_TINTS.region} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="7 5" />
	{/if}
	{#each members as m (m.id)}
		<MemberChip x={m.x} y={m.y} r={m.r} title={m.title} docType={m.docType} onEnter={() => drill(m.id)} />
	{/each}
</g>

<text x="24" y="28" fill="#e0b060" font-size="12" font-weight="600" letter-spacing="1">REGION · interior</text>
<g transform={`translate(${width - 190}, 14)`}>
	<rect width="168" height="24" rx="12" fill="#e0b060" fill-opacity="0.08" stroke="#e0b060" stroke-opacity="0.25" />
	<text x="84" y="16" text-anchor="middle" fill="#c9b183" font-size="10">◵ {slice.components.length} sub-clusters</text>
</g>
```

- [ ] **Step 2: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte"
git commit -m "feat(atlas): TierTerritory — Tier-1 region interior (members + hull + sub-cluster badge)"
```

---

### Task 13: `TierNeighborhood` component (Tier 2 force graph)

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte`

**Interfaces:**
- Consumes: `AtlasSubgraph` (`$lib/types/generated/graph_atlas`); `forceNeighborhood` (Task 8); `NodeChip`, `Edge` (Task 10); `buildDrillNodeUrl` (`nav.ts`); `goto`, `page`.
- Props: `{ subgraph: AtlasSubgraph; seedId: string; width: number; height: number }`.
- Consumed by: Task 15 (AtlasCanvas).

Runs the force layout once (derived from props), renders edges then node chips, hover reveals an edge label, click a node re-seeds. Arrowhead markers are defined once here.

- [ ] **Step 1: Write the component**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import { forceNeighborhood } from '$lib/graph/atlas/layout/forceNeighborhood';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { EDGE_COLORS } from '$lib/graph/atlas/palette';
	import NodeChip from './marks/NodeChip.svelte';
	import Edge from './marks/Edge.svelte';

	interface Props {
		subgraph: AtlasSubgraph;
		seedId: string;
		width: number;
		height: number;
	}
	let { subgraph, seedId, width, height }: Props = $props();

	const graph = $derived(forceNeighborhood(subgraph, [seedId], { width, height }));
	let hoveredEdge = $state<number | null>(null);

	function drill(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId), { replaceState: true });
	}

	function nodeRadius(degree: number): number {
		return 8 + Math.min(10, degree);
	}
</script>

<defs>
	<marker id="arrow-end" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
		<path d="M0,0 L10,5 L0,10 z" fill={EDGE_COLORS.structural} />
	</marker>
	<marker id="arrow-start" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="7" markerHeight="7" orient="auto">
		<path d="M10,0 L0,5 L10,10 z" fill={EDGE_COLORS.structural} />
	</marker>
</defs>

{#each graph.edges as e, i (i)}
	<g role="presentation" onmouseenter={() => (hoveredEdge = i)} onmouseleave={() => (hoveredEdge = null)}>
		<Edge x1={e.source.x} y1={e.source.y} x2={e.target.x} y2={e.target.y} edge={e.edge} label={hoveredEdge === i} />
	</g>
{/each}

{#each graph.nodes as n (n.id)}
	<NodeChip x={n.x} y={n.y} r={nodeRadius(n.degree)} title={n.title} docType={n.docType} home={n.home} seed={n.isSeed} onEnter={() => drill(n.id)} />
{/each}
```

- [ ] **Step 2: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte"
git commit -m "feat(atlas): TierNeighborhood — Tier-2 force graph with the edge grammar"
```

---

### Task 14: `TierPanorama` — sparse cogmap-as-territory + drill wiring

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`

**Interfaces:**
- `TerritoryCircle` gains: `onEnter?: () => void` (clickable only when provided — region territories only).
- `TierPanorama` now: packs real territories (region drillable → Tier 1, context inert), and renders `packCogmapTerritories(overview.orphan_nodes)` as cogmap hulls with clickable facet dots (→ Tier 2). Zones unchanged.
- Consumes additionally: `packCogmapTerritories` (Task 5), `buildDrillTerritoryUrl`, `buildDrillNodeUrl` (`nav.ts`), `TERRITORY_TINTS`.

- [ ] **Step 1: Add `onEnter` to `TerritoryCircle.svelte`**

In the `<script>` `interface Props`, add `onEnter?: () => void;` and destructure it. Wrap the `<g class="territory">` opening tag with the click affordance:

```svelte
<g
	class="territory"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}
>
```

(Leave the inner `<circle>`/`<text>` unchanged.)

- [ ] **Step 2: Rewrite `TierPanorama.svelte`**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { packTerritories } from '$lib/graph/atlas/layout/packTerritories';
	import { packCogmapTerritories } from '$lib/graph/atlas/layout/cogmapTerritories';
	import { buildScopeUrl, buildDrillTerritoryUrl, buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import TerritoryCircle from './marks/TerritoryCircle.svelte';
	import TeamZoneMark from './marks/TeamZoneMark.svelte';
	import OrphanNodeMark from './marks/OrphanNodeMark.svelte';

	interface Props {
		overview: TerritoryOverview;
		zones: TeamZone[];
		width: number;
		height: number;
	}
	let { overview, zones, width, height }: Props = $props();

	const ZONE_BAND = 120;
	const ZONE_W = 170;
	const ZONE_H = 90;

	const bodyHeight = $derived(Math.max(1, height - ZONE_BAND));
	const packed = $derived(packTerritories(overview.territories, { width, height: bodyHeight }));
	const cogmaps = $derived(packCogmapTerritories(overview.orphan_nodes, { width, height: bodyHeight }));

	function enterZone(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
	function drillTerritory(regionId: string) {
		goto(buildDrillTerritoryUrl($page.url, regionId), { replaceState: true });
	}
	function drillNode(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId), { replaceState: true });
	}
</script>

<!-- team-DAG zones (enterable, membership-gated) -->
{#each zones as zone, i (zone.id)}
	<TeamZoneMark
		x={10 + i * (ZONE_W + 14)}
		y={16}
		width={ZONE_W}
		height={ZONE_H}
		name={zone.name}
		resourceCount={zone.resource_count}
		onEnter={() => enterZone(zone.id)}
	/>
{/each}

<g transform={`translate(0, ${ZONE_BAND})`}>
	<!-- dense territories: regions drill to Tier 1, contexts are inert -->
	{#each packed as t (t.id)}
		<TerritoryCircle
			x={t.x}
			y={t.y}
			r={t.r}
			kind={t.kind}
			label={t.label}
			onEnter={t.kind === 'region' ? () => drillTerritory(t.id) : undefined}
		/>
	{/each}

	<!-- sparse state: region-less cogmaps drawn as territories with clickable facet dots -->
	{#each cogmaps as cm (cm.cogmapId)}
		<g class="cogmap-territory">
			<circle cx={cm.x} cy={cm.y} r={cm.r} fill={TERRITORY_TINTS.cogmap} fill-opacity="0.06" stroke={TERRITORY_TINTS.cogmap} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="6 4" />
			<text x={cm.x} y={cm.y - cm.r - 6} text-anchor="middle" fill={TERRITORY_TINTS.cogmap} font-size="11" font-weight="600" letter-spacing="1" style="text-transform:uppercase">{cm.label}</text>
			{#each cm.facets as f (f.id)}
				<OrphanNodeMark x={f.x} y={f.y} r={Math.min(7, f.r)} title={f.title} docType={f.docType} onEnter={() => drillNode(f.id)} />
			{/each}
		</g>
	{/each}
</g>
```

- [ ] **Step 3: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte" "packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte"
git commit -m "feat(atlas): TierPanorama — sparse cogmap-as-territory + region/facet drill wiring"
```

---

### Task 15: `AtlasCanvas` — home/1/2 dispatch + M6 camera reset

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte`

**Interfaces:**
- New props: `{ teamId: string | null; tier: number; focus: Focus; territories; slice; neighborhood; teams; zones }` — the load's PageData (Task 16). `Focus` from `nav.ts`.
- Renders `TierHome` (no team), `TierPanorama` (tier 0), `TierTerritory` (tier 1), `TierNeighborhood` (tier 2). Camera resets on `teamId + focus` change (M6).

M6: rather than an internal `{#key}` (which would re-create the `bind:this` SVG without re-firing `onMount`, and we can't use `$effect` to react to the rebind), the **parent** keys the whole `AtlasCanvas` on a `teamId|focus` string (Task 17). The component therefore genuinely remounts on re-scope, so its own `onMount` re-attaches a fresh camera at the identity transform — no internal keying needed here.

- [ ] **Step 1: Rewrite the component**

```svelte
<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import type { TeamRow } from '$lib/types/generated/team';
	import type { Focus } from '$lib/graph/atlas/nav';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierHome from './TierHome.svelte';
	import TierPanorama from './TierPanorama.svelte';
	import TierTerritory from './TierTerritory.svelte';
	import TierNeighborhood from './TierNeighborhood.svelte';

	interface Props {
		teamId: string | null;
		tier: number;
		focus: Focus;
		territories: TerritoryOverview | null;
		slice: TerritorySlice | null;
		neighborhood: AtlasSubgraph | null;
		teams: TeamRow[] | null;
		zones: TeamZone[];
	}
	let { teamId, tier, focus, territories, slice, neighborhood, teams, zones }: Props = $props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	const seedId = $derived(focus.kind === 'node' ? focus.id : '');

	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	// The parent keys this whole component on teamId|focus (Task 17), so onMount
	// re-fires on every re-scope and the d3-zoom transform resets (M6).
	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="atlas-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Team graph atlas">
		<rect x="0" y="0" width={W} height={H} fill={CANVAS_BG} />
		<g bind:this={viewportEl}>
			{#if !teamId && teams}
				<TierHome {teams} width={W} height={H} />
			{:else if tier === 0 && territories}
				<TierPanorama overview={territories} {zones} width={W} height={H} />
			{:else if tier === 1 && slice}
				<TierTerritory {slice} width={W} height={H} />
			{:else if tier === 2 && neighborhood}
				<TierNeighborhood subgraph={neighborhood} {seedId} width={W} height={H} />
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">No data for this view.</text>
			{/if}
		</g>
	</svg>
</div>

<style>
	.atlas-canvas {
		width: 100%;
		height: 100%;
	}
	.atlas-canvas svg {
		display: block;
		width: 100%;
		height: auto;
	}
</style>
```

- [ ] **Step 2: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte"
git commit -m "feat(atlas): AtlasCanvas — home/tier-1/tier-2 dispatch + M6 camera reset on re-scope"
```

---

### Task 16: Route load — branch home / Tier 0 / 1 / 2

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts`

**Interfaces:**
- Produces the `PageData` shape: `{ owner: string; teamId: string | null; scope: TeamScopeView | null; tier: Tier; focus: Focus; teams: TeamRow[] | null; territories: TerritoryOverview | null; slice: TerritorySlice | null; neighborhood: AtlasSubgraph | null }`.
- Consumed by: Task 17 (`+page.svelte`) → AtlasCanvas (Task 15).

No team → home (list teams, everything else null). With a team → scope + the one read the tier needs.

- [ ] **Step 1: Rewrite the load**

```ts
// +page.server.ts
import type { PageServerLoad } from './$types';
import { deriveTier, parseFilters, parseFocus, parseTeam } from '$lib/graph/atlas/nav';
import {
	listTeams,
	readNeighborhood,
	readRegionSlice,
	readTeamScope,
	readTerritories
} from '$lib/server/graph-reads';

const NEIGHBORHOOD_DEPTH = 2;

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;
	const teamId = parseTeam(url.searchParams);
	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);

	// No team scoped → the canonical @me membership home (you → teams).
	if (!teamId) {
		const teams = await listTeams(token);
		return {
			owner: params.owner,
			teamId: null,
			scope: null,
			tier,
			focus,
			teams,
			territories: null,
			slice: null,
			neighborhood: null
		};
	}

	const filters = parseFilters(url.searchParams);
	const scope = await readTeamScope(token, teamId);

	const territories = tier === 0 ? await readTerritories(token, teamId, filters.lensId) : null;
	const slice = tier === 1 && focus.kind === 'territory' ? await readRegionSlice(token, focus.id) : null;
	const neighborhood =
		tier === 2 && focus.kind === 'node'
			? await readNeighborhood(token, teamId, { seeds: [focus.id], depth: NEIGHBORHOOD_DEPTH, edge_kinds: [] })
			: null;

	return {
		owner: params.owner,
		teamId,
		scope,
		tier,
		focus,
		teams: null,
		territories,
		slice,
		neighborhood
	};
};
```

- [ ] **Step 2: Verify it type-checks**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts"
git commit -m "feat(atlas): load — branch home / Tier 0-1-2 reads (R2/R3/R4)"
```

---

### Task 17: Page shell (home vs team) + end-to-end verification

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.svelte`

**Interfaces:**
- `+page.svelte`: `let { data }: { data: PageData } = $props()`. Shows the `ScopeBar` only when scoped to a team; the home shows a minimal header.
- Passes the full load payload into `AtlasCanvas`.

- [ ] **Step 1: Rewrite `+page.svelte`**

```svelte
<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';

	let { data }: { data: PageData } = $props();

	// M6: keying AtlasCanvas on the scoped view remounts it on re-scope, resetting the camera.
	const viewKey = $derived(
		`${data.teamId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
</script>

<div class="atlas-page">
	{#if data.scope}
		<ScopeBar scope={data.scope} />
	{:else}
		<nav class="scope-bar home">Atlas · your teams</nav>
	{/if}
	{#key viewKey}
		<AtlasCanvas
			teamId={data.teamId}
			tier={data.tier}
			focus={data.focus}
			territories={data.territories}
			slice={data.slice}
			neighborhood={data.neighborhood}
			teams={data.teams}
			zones={data.scope?.zones ?? []}
		/>
	{/key}
</div>

<style>
	.atlas-page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	.scope-bar.home {
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
</style>
```

- [ ] **Step 2: Type-check + full unit suite**

Run: `cd packages/temper-ui && bun run check && bunx vitest run`
Expected: svelte-check 0 errors; all `atlas/*` unit tests pass (nav, palette, packTerritories, cogmapTerritories, regionInterior, hull, forceNeighborhood, homeLayout).

- [ ] **Step 3: End-to-end manual verification (the gate CI can't run)**

Prereq: API running + authed session. Run: `cd packages/temper-ui && bun run dev`. In the browser:

1. **Home:** open `/graph/@me` (no `?team`). The membership home renders: `you` node + your team doors + edges. Clicking a team navigates to `?team=<id>` and paints that team's Tier-0. The header reads "Atlas · your teams" (no ScopeBar).
2. **Sparse Tier-0 (real prod shape):** for the L0 team, the region-less cogmap renders as a **bounded dashed circle with clickable facet dots** (not an off-canvas column). Clicking a facet dot → `?focus=node:<id>` → a Tier-2 neighborhood.
3. **Tier-2:** the neighborhood shows the seed enlarged + ringed, node chips (filled cogmap-home / outlined context-home), typed edges (line style per kind, arrowheads per polarity, thickness per weight); hovering an edge reveals its relation label. Clicking another node re-seeds.
4. **Tier-1 (if a dense team/region is available):** clicking a region territory → an affinity-packed member interior with a hull + "N sub-clusters" badge; clicking a member → Tier-2.
5. **Drill/ascend + camera:** the browser back button ascends one level; zoom/pan works within a tier and **resets after a drill/re-scope** (M6). Switching teams resets the camera.

Expected: all hold. If no dense region seed exists, (4) is logic-verified via the unit tests; (1)(2)(3)(5) must all pass against the real L0 data.

- [ ] **Step 4: Commit**

```bash
git add "packages/temper-ui/src/routes/(app)/graph/[owner]/+page.svelte"
git commit -m "feat(atlas): page shell — home header vs team ScopeBar; C2 end-to-end"
```

---

## Self-Review

**1. Spec coverage (C2 spec → tasks):**
- C2-D1 route relocation → Task 1. ✓
- C2-D2 canonical @me home (you → teams, buildHomeUrl, no arbitrary teams[0]) → Tasks 2, 9, 11, 16, 17. ✓
- C2-D3 drill semantics (region→T1, context inert, orphan/member/node→T2, ascend+back) → Tasks 12, 13, 14 (wiring), 2 (buildHomeUrl), 16 (load). ✓
- C2-D4 sparse cogmap-as-territory → Tasks 5, 14. ✓
- C2-D5 Tier-1 members-focused + sub-cluster badge + no-home fill fallback → Tasks 6, 10 (MemberChip), 12. ✓
- C2-D6 Tier-2 force + edge grammar + labels-on-touch + depth-2 + degree sizing → Tasks 3 (edgeStyle), 8 (forceNeighborhood, degree), 10 (NodeChip/Edge), 13, 16 (depth 2). ✓
- C2-D7 deferred items: I2 → Task 4; M4 → Tasks 1/16 (@me-canonical, no owner→team); M5 → Tasks 5/14; M6 → Task 15. ✓
- Palette single-source, edge grammar consumed → Tasks 3, 10, 13. ✓
- New d3 deps → Task 0. ✓
- **Out of scope (recorded in spec):** Atlas Home rich chunk (cogmap column + counts + enter-a-cogmap read), C3 chrome, Chunk D deletion. Not gaps.

**2. Placeholder scan:** No TBD/TODO. Every step has complete code. The `RegionHull` `label` prop is intentionally caller-optional (Task 12 renders its own header and passes no label); `RegionHull` is used for its path. If a reviewer prefers, drop the unused label branch — it does not affect behavior.

**3. Type consistency:** `Focus` (nav.ts) flows through the load (Task 16) → AtlasCanvas (Task 15) → `seedId`/tier dispatch. `PositionedFacet`/`CogmapTerritory` (Task 5) match TierPanorama usage (Task 14). `ForceNode`/`ForceEdge` (Task 8) match TierNeighborhood (Task 13). `PositionedMember` (Task 6) matches TierTerritory (Task 12). `EdgeStyle`/`edgeStyle` (Task 3) match `Edge.svelte` (Task 10). `HomeGraph` (Task 9) matches TierHome (Task 11). Read wrapper return types (`readRegionSlice`→`TerritorySlice`, `readNeighborhood`→`AtlasSubgraph`, `listTeams`→`TeamRow[]`) match the load's fields (Task 16) and the component props. `buildDrillTerritoryUrl`/`buildDrillNodeUrl`/`buildScopeUrl`/`buildHomeUrl` (nav.ts) are used consistently in Tasks 11/12/13/14.

**4. M6 camera reset:** resolved correctly by construction — Task 17 keys the whole `AtlasCanvas` on `teamId|focus`, so the component remounts on re-scope and its `onMount` re-attaches a fresh camera at the identity transform (no `$effect`, no internal `{#key}` on a `bind:this` node). Task 17 Step 3(5) still verifies zoom-resets-after-drill in the browser.
