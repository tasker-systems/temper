# Graph Atlas — Chunk C1 (Foundations + Tier-0 Panorama) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the temper-ui Atlas UI foundation and a working Tier-0 team panorama: a new `/vault/[owner]/graph` route that SSR-loads a team's scope (R1) + territory overview (R2), renders territories / team-zones / orphan salient nodes on a d3+SVG canvas with the "Vivid Cartographer" palette, supports pan/zoom and team-zone re-scope, and carries scope in the URL.

**Architecture:** d3 supplies math (layout, zoom) as pure functions; Svelte renders SVG reactively (`{#each}`). Navigation state lives in the URL (`?team`, `?focus`, filters); the `+page.server.ts` load derives the tier from the URL and fetches the matching read server-side (authed via `apiGet`). Everything testable-in-isolation (palette, nav, layout, read-paths) is a pure function unit-tested under vitest/node; Svelte components are thin reactive-SVG over that math and are verified by running the app. This is a **new** Atlas stack built alongside the old context-scoped graph, which Chunk D deletes.

**Tech Stack:** SvelteKit 2, Svelte 5 (runes), TypeScript 5, Tailwind v4, vitest 3 (node env), d3 submodules (`d3-hierarchy`, `d3-zoom`, `d3-selection`).

## Global Constraints

- **Svelte 5 runes only.** Props via `interface Props` + `let { … }: Props = $props()`; state via `$state`; derived via `$derived`/`$derived.by`; lifecycle via `onMount`/`onDestroy`. **No `$effect`** — the repo has no precedent; use `onMount` + `$derived` instead.
- **vitest:** node environment, `globals: false` — every test file imports `{ describe, it, expect }` from `vitest`. Test files are `src/**/*.test.ts`. Only pure functions are unit-tested (no jsdom; components are not render-tested — verified by running the app).
- **Generated types are read-only.** Import wire types from `$lib/types/generated/*`; never hand-model or edit them. Regenerate via `cargo make generate-ts-types` (not needed here — all types already exist).
- **Server-only reads.** `apiGet`/`apiPost` (`$lib/server/api.ts`) use `$env/dynamic/private` + `locals.accessToken`; they may be imported **only** from `.server.ts` / `+server.ts`. The Atlas read wrappers live in `$lib/server/graph-reads.ts` for this reason.
- **URL is the source of truth for navigation.** Change scope/focus/filters with `goto(url.toString(), { replaceState: true })` (the FacetChips/VaultGrid idiom); `+page.server.ts load` re-runs and returns the new tier's data.
- **Palette single source of truth:** all Atlas hues come from `$lib/graph/atlas/palette.ts`. Do **not** read the legacy `--graph-*` / `--color-graph-*` CSS vars or `styling.ts` `NODE_COLORS` (those belong to the old stack and are removed in Chunk D).
- **Indentation:** tabs (match existing `.ts`/`.svelte` files).
- **d3:** import only the named submodules used; never the `d3` meta-package.

---

### Task 0: Add d3 dependencies

**Files:**
- Modify: `packages/temper-ui/package.json`

- [ ] **Step 1: Add the runtime + type deps**

Run (from `packages/temper-ui/`):

```bash
cd packages/temper-ui
bun add d3-hierarchy@^3 d3-zoom@^3 d3-selection@^3
bun add -d @types/d3-hierarchy@^3 @types/d3-zoom@^3 @types/d3-selection@^3
```

- [ ] **Step 2: Verify they resolve**

Run: `bun run check`
Expected: svelte-check completes with no new errors (0 errors is the current baseline).

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/package.json packages/temper-ui/bun.lock
git commit -m "chore(temper-ui): add d3-hierarchy/zoom/selection for the Atlas graph"
```

---

### Task 1: Palette module (single source of truth)

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/palette.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/palette.test.ts`

**Interfaces:**
- Consumes: `NodeHome` from `$lib/types/generated/graph_atlas`.
- Produces: `type AtlasDocType`, `DOC_TYPE_HUES: Record<AtlasDocType,string>`, `AUTHORED_DOC_TYPES: ReadonlySet<AtlasDocType>`, `FALLBACK_HUE: string`, `docTypeHue(docType: string | null): string`, `isAuthored(docType: string | null): boolean`, `nodeMark(docType: string | null, home: NodeHome): { color: string; filled: boolean }`, `EDGE_COLORS`, `LIGHT_MODE_RING: string`, `salienceOpacity(salience: number | null): number`, `paletteStyleVars(): string`.

- [ ] **Step 1: Write the failing test**

```ts
// palette.test.ts
import { describe, expect, it } from 'vitest';
import {
	AUTHORED_DOC_TYPES,
	DOC_TYPE_HUES,
	FALLBACK_HUE,
	docTypeHue,
	isAuthored,
	nodeMark,
	paletteStyleVars,
	salienceOpacity
} from './palette';

describe('DOC_TYPE_HUES', () => {
	it('defines all 14 doc-types with the locked Vivid Cartographer hexes', () => {
		expect(DOC_TYPE_HUES.concept).toBe('#e8942e');
		expect(DOC_TYPE_HUES.fact).toBe('#f7c62b');
		expect(DOC_TYPE_HUES.domain).toBe('#d3d84e');
		expect(DOC_TYPE_HUES.goal).toBe('#3a8ae8'); // goal is cool (legacy gold retired)
		expect(Object.keys(DOC_TYPE_HUES)).toHaveLength(14);
	});
});

describe('docTypeHue', () => {
	it('returns the hue for a known type', () => {
		expect(docTypeHue('question')).toBe('#a95cf0');
	});
	it('falls back for unknown or null', () => {
		expect(docTypeHue('nonsense')).toBe(FALLBACK_HUE);
		expect(docTypeHue(null)).toBe(FALLBACK_HUE);
	});
});

describe('isAuthored', () => {
	it('classifies authored vs workflow types', () => {
		expect(isAuthored('concept')).toBe(true);
		expect(isAuthored('goal')).toBe(false);
		expect(isAuthored(null)).toBe(false);
	});
	it('keeps the two families disjoint and covering', () => {
		const workflow = ['research', 'task', 'session', 'goal', 'decision', 'memory'];
		for (const t of AUTHORED_DOC_TYPES) expect(workflow).not.toContain(t);
		expect(AUTHORED_DOC_TYPES.size + workflow.length).toBe(14);
	});
});

describe('nodeMark', () => {
	it('fills cogmap-homed nodes and outlines context-homed ones', () => {
		expect(nodeMark('concept', 'cogmap')).toEqual({ color: '#e8942e', filled: true });
		expect(nodeMark('research', 'context')).toEqual({ color: '#33b0e2', filled: false });
	});
});

describe('salienceOpacity', () => {
	it('ramps within [0.35, 1] and clamps', () => {
		expect(salienceOpacity(0)).toBeCloseTo(0.35);
		expect(salienceOpacity(1)).toBeCloseTo(1);
		expect(salienceOpacity(2)).toBeCloseTo(1); // clamp high
		expect(salienceOpacity(null)).toBeCloseTo(0.35); // null → floor
	});
});

describe('paletteStyleVars', () => {
	it('emits a CSS custom-property string for every doc-type', () => {
		const s = paletteStyleVars();
		expect(s).toContain('--dt-concept:#e8942e');
		expect(s).toContain('--dt-goal:#3a8ae8');
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/palette.test.ts`
Expected: FAIL — cannot find module `./palette`.

- [ ] **Step 3: Write the implementation**

```ts
// palette.ts
/**
 * Vivid Cartographer — the single source of truth for the Atlas graph palette.
 *
 * Warm semicircle = authored/knowledge doc-types (cogmap-homed, rendered filled);
 * cool semicircle = workflow doc-types (context-homed, rendered outline). Home is
 * carried by fill-vs-outline, so hue is free to mean doc-type. See
 * docs/superpowers/specs/2026-07-03-graph-atlas-chunk-c-ui-engine-design.md (D3–D5).
 *
 * This module is the ONLY place Atlas hues are defined. The legacy `--graph-*` /
 * `--color-graph-*` CSS vars and styling.ts NODE_COLORS belong to the old graph
 * stack and are removed in Chunk D.
 */
import type { NodeHome } from '$lib/types/generated/graph_atlas';

export type AtlasDocType =
	| 'concept' | 'fact' | 'domain' | 'principle' | 'commitment' | 'concern' | 'theme' | 'question'
	| 'research' | 'task' | 'session' | 'goal' | 'decision' | 'memory';

/** Warm/authored — rendered filled. */
export const AUTHORED_DOC_TYPES: ReadonlySet<AtlasDocType> = new Set([
	'concept', 'fact', 'domain', 'principle', 'commitment', 'concern', 'theme', 'question'
]);

/** Locked dark-canvas hues (light mode adds a contrast ring, not a hue fork). */
export const DOC_TYPE_HUES: Record<AtlasDocType, string> = {
	// warm · authored
	concept: '#e8942e',
	fact: '#f7c62b',
	domain: '#d3d84e',
	principle: '#f2743a',
	commitment: '#f0533f',
	concern: '#ef5090',
	theme: '#e24fc0',
	question: '#a95cf0',
	// cool · workflow
	research: '#33b0e2',
	task: '#34cf7e',
	session: '#7ed24a',
	goal: '#3a8ae8',
	decision: '#6a6ee8',
	memory: '#2ec9b0'
};

/** Neutral for unknown/absent doc-types. */
export const FALLBACK_HUE = '#9aa5b5';

/** Structural edge gray, contradicts-red, derived_from bridge. */
export const EDGE_COLORS = {
	structural: '#8b93a5',
	contradicts: '#d98a8a',
	derived: '#5f6b86'
} as const;

/** Dark contrast ring applied to dots in light mode so pale hues read. */
export const LIGHT_MODE_RING = '#2a2f38';

const SALIENCE_FLOOR = 0.35;

export function docTypeHue(docType: string | null): string {
	if (docType && docType in DOC_TYPE_HUES) return DOC_TYPE_HUES[docType as AtlasDocType];
	return FALLBACK_HUE;
}

export function isAuthored(docType: string | null): boolean {
	return docType !== null && AUTHORED_DOC_TYPES.has(docType as AtlasDocType);
}

/** A node's dot mark: hue by doc-type, filled vs outline by home. */
export function nodeMark(docType: string | null, home: NodeHome): { color: string; filled: boolean } {
	return { color: docTypeHue(docType), filled: home === 'cogmap' };
}

/** Salience → opacity ramp in [0.35, 1]; null/low → floor, clamps high. */
export function salienceOpacity(salience: number | null): number {
	if (salience === null || Number.isNaN(salience)) return SALIENCE_FLOOR;
	const clamped = Math.min(1, Math.max(0, salience));
	return SALIENCE_FLOOR + (1 - SALIENCE_FLOOR) * clamped;
}

/** CSS custom-property string (`--dt-<type>:<hex>;…`) for scoping onto the canvas root. */
export function paletteStyleVars(): string {
	return (Object.entries(DOC_TYPE_HUES) as [AtlasDocType, string][])
		.map(([type, hex]) => `--dt-${type}:${hex};`)
		.join('');
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/palette.test.ts`
Expected: PASS (all cases).

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/palette.ts packages/temper-ui/src/lib/graph/atlas/palette.test.ts
git commit -m "feat(atlas): palette.ts — single-source Vivid Cartographer palette"
```

---

### Task 2: Navigation URL logic

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces: `type Tier = 0 | 1 | 2`, `type Focus`, `parseFocus(params: URLSearchParams): Focus`, `deriveTier(focus: Focus): Tier`, `parseTeam(params: URLSearchParams): string | null`, `type GraphFilters`, `parseFilters(params: URLSearchParams): GraphFilters`, `buildScopeUrl(base: URL, teamId: string): string`, `buildDrillTerritoryUrl(base: URL, territoryId: string): string`, `buildDrillNodeUrl(base: URL, nodeId: string): string`, `buildAscendUrl(base: URL): string`.
- Consumed by: Task 6 (load), Task 8/10 (components) — the URL builders are passed to `goto`.

- [ ] **Step 1: Write the failing test**

```ts
// nav.test.ts
import { describe, expect, it } from 'vitest';
import {
	buildAscendUrl,
	buildDrillNodeUrl,
	buildDrillTerritoryUrl,
	buildScopeUrl,
	deriveTier,
	parseFocus,
	parseTeam
} from './nav';

const url = (qs: string) => new URL(`https://x/vault/@me/graph${qs}`);

describe('parseFocus + deriveTier', () => {
	it('no focus param → none → tier 0', () => {
		const f = parseFocus(url('').searchParams);
		expect(f).toEqual({ kind: 'none' });
		expect(deriveTier(f)).toBe(0);
	});
	it('territory focus → tier 1', () => {
		const f = parseFocus(url('?focus=territory:abc').searchParams);
		expect(f).toEqual({ kind: 'territory', id: 'abc' });
		expect(deriveTier(f)).toBe(1);
	});
	it('node focus → tier 2', () => {
		const f = parseFocus(url('?focus=node:n1').searchParams);
		expect(f).toEqual({ kind: 'node', id: 'n1' });
		expect(deriveTier(f)).toBe(2);
	});
	it('malformed focus → none → tier 0', () => {
		expect(deriveTier(parseFocus(url('?focus=garbage').searchParams))).toBe(0);
	});
});

describe('parseTeam', () => {
	it('reads ?team, else null', () => {
		expect(parseTeam(url('?team=t1').searchParams)).toBe('t1');
		expect(parseTeam(url('').searchParams)).toBeNull();
	});
});

describe('URL builders', () => {
	it('buildScopeUrl sets team and CLEARS focus (re-scope resets to tier 0)', () => {
		const out = buildScopeUrl(url('?team=old&focus=node:n1'), 'new');
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('team')).toBe('new');
		expect(p.get('focus')).toBeNull();
	});
	it('buildDrillTerritoryUrl sets focus=territory:<id>, keeps team', () => {
		const out = buildDrillTerritoryUrl(url('?team=t1'), 'r9');
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('team')).toBe('t1');
		expect(p.get('focus')).toBe('territory:r9');
	});
	it('buildDrillNodeUrl sets focus=node:<id>', () => {
		const p = new URL(buildDrillNodeUrl(url('?team=t1'), 'n5'), 'https://x').searchParams;
		expect(p.get('focus')).toBe('node:n5');
	});
	it('buildAscendUrl clears focus', () => {
		const p = new URL(buildAscendUrl(url('?team=t1&focus=node:n5')), 'https://x').searchParams;
		expect(p.get('focus')).toBeNull();
		expect(p.get('team')).toBe('t1');
	});
	it('builders return path+query only (relative), preserving the graph pathname', () => {
		expect(buildScopeUrl(url('?team=old'), 'new').startsWith('/vault/@me/graph?')).toBe(true);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/nav.test.ts`
Expected: FAIL — cannot find module `./nav`.

- [ ] **Step 3: Write the implementation**

```ts
// nav.ts
/**
 * Atlas navigation is expressed entirely in the URL — the "URI frame". The route
 * `/vault/[owner]/graph` carries `?team`, `?focus`, and filter params; the tier is
 * DERIVED from focus, never stored. Zoom/pan is ephemeral client state and is NOT
 * in the URL. See spec D2. Builders return a relative `path?query` string ready to
 * hand to `goto(…, { replaceState: true })`.
 */
export type Tier = 0 | 1 | 2;

export type Focus =
	| { kind: 'none' }
	| { kind: 'territory'; id: string }
	| { kind: 'node'; id: string };

export interface GraphFilters {
	/** Optional lens id driving Tier-0 salience sizing (R2 `?lens_id`). */
	lensId: string | null;
}

export function parseTeam(params: URLSearchParams): string | null {
	return params.get('team');
}

export function parseFocus(params: URLSearchParams): Focus {
	const raw = params.get('focus');
	if (!raw) return { kind: 'none' };
	const [kind, id] = raw.split(':', 2);
	if (id && (kind === 'territory' || kind === 'node')) return { kind, id };
	return { kind: 'none' };
}

export function deriveTier(focus: Focus): Tier {
	switch (focus.kind) {
		case 'territory':
			return 1;
		case 'node':
			return 2;
		default:
			return 0;
	}
}

export function parseFilters(params: URLSearchParams): GraphFilters {
	return { lensId: params.get('lens_id') };
}

function withParams(base: URL, mutate: (p: URLSearchParams) => void): string {
	const u = new URL(base);
	mutate(u.searchParams);
	return `${u.pathname}${u.search}`;
}

/** Enter a team zone / switch team: set team, clear focus (re-scope resets to Tier 0). */
export function buildScopeUrl(base: URL, teamId: string): string {
	return withParams(base, (p) => {
		p.set('team', teamId);
		p.delete('focus');
	});
}

export function buildDrillTerritoryUrl(base: URL, territoryId: string): string {
	return withParams(base, (p) => p.set('focus', `territory:${territoryId}`));
}

export function buildDrillNodeUrl(base: URL, nodeId: string): string {
	return withParams(base, (p) => p.set('focus', `node:${nodeId}`));
}

export function buildAscendUrl(base: URL): string {
	return withParams(base, (p) => p.delete('focus'));
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/nav.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): nav.ts — URL-frame navigation logic (tier derived from focus)"
```

---

### Task 3: Server read wrappers + path builders

**Files:**
- Create: `packages/temper-ui/src/lib/server/graph-reads.ts`
- Test: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`

**Interfaces:**
- Consumes: `apiGet`, `apiPost` from `$lib/server/api`; wire types `TeamScopeView`, `TerritoryOverview`, `TerritorySlice`, `AtlasSubgraph`, `SliceRequest`, `EventTrail` from `$lib/types/generated/*`; `TeamRow` from `$lib/types/generated/team`.
- Produces (pure): `teamScopePath`, `territoriesPath`, `regionSlicePath`, `neighborhoodSlicePath`, `trailPath`, `teamsListPath`. Produces (thin async): `readTeamScope`, `readTerritories`, `readRegionSlice`, `readNeighborhood`, `readTrail`, `listTeams`.
- Consumed by: Task 6 (load) and (in C3) the trail `+server.ts`.

Note: only the pure path builders are unit-tested; the async wrappers are thin `apiGet`/`apiPost` pass-throughs (the same untested shape as `api.ts` itself).

- [ ] **Step 1: Write the failing test (pure path builders)**

```ts
// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import {
	neighborhoodSlicePath,
	regionSlicePath,
	teamScopePath,
	teamsListPath,
	territoriesPath,
	trailPath
} from './graph-reads';

describe('graph API path builders', () => {
	it('R1 team scope', () => {
		expect(teamScopePath('t1')).toBe('/api/teams/t1/graph-scope');
	});
	it('R2 territories, optional lens', () => {
		expect(territoriesPath('t1', null)).toBe('/api/teams/t1/graph/territories');
		expect(territoriesPath('t1', 'lens9')).toBe('/api/teams/t1/graph/territories?lens_id=lens9');
	});
	it('R3 region slice', () => {
		expect(regionSlicePath('r5')).toBe('/api/graph/regions/r5/slice');
	});
	it('R4 neighborhood slice (POST target)', () => {
		expect(neighborhoodSlicePath('t1')).toBe('/api/teams/t1/graph/slice');
	});
	it('R5 element trail', () => {
		expect(trailPath('node', 'n1')).toBe('/api/graph/elements/node/n1/trail');
		expect(trailPath('edge', 'e1')).toBe('/api/graph/elements/edge/e1/trail');
	});
	it('teams list', () => {
		expect(teamsListPath()).toBe('/api/teams');
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/server/graph-reads.paths.test.ts`
Expected: FAIL — cannot find module `./graph-reads`.

- [ ] **Step 3: Write the implementation**

```ts
// graph-reads.ts
/**
 * Server-only wrappers for the Atlas reads R1–R5 (+ teams list). These use apiGet/
 * apiPost, which read the encrypted session token — so this module may be imported
 * ONLY from `.server.ts` / `+server.ts`. Path builders are pure and unit-tested;
 * the async wrappers are thin pass-throughs.
 */
import { apiGet, apiPost } from '$lib/server/api';
import type { AtlasSubgraph, SliceRequest } from '$lib/types/generated/graph_atlas';
import type { EventTrail, ElementKind } from '$lib/types/generated/element_trail';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';
import type { TeamRow } from '$lib/types/generated/team';
import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';

export const teamScopePath = (teamId: string): string => `/api/teams/${teamId}/graph-scope`;

export const territoriesPath = (teamId: string, lensId: string | null): string =>
	lensId
		? `/api/teams/${teamId}/graph/territories?lens_id=${encodeURIComponent(lensId)}`
		: `/api/teams/${teamId}/graph/territories`;

export const regionSlicePath = (regionId: string): string => `/api/graph/regions/${regionId}/slice`;

export const neighborhoodSlicePath = (teamId: string): string => `/api/teams/${teamId}/graph/slice`;

export const trailPath = (kind: ElementKind, id: string): string =>
	`/api/graph/elements/${kind}/${id}/trail`;

export const teamsListPath = (): string => `/api/teams`;

export const readTeamScope = (token: string, teamId: string): Promise<TeamScopeView> =>
	apiGet<TeamScopeView>(teamScopePath(teamId), token);

export const readTerritories = (
	token: string,
	teamId: string,
	lensId: string | null
): Promise<TerritoryOverview> => apiGet<TerritoryOverview>(territoriesPath(teamId, lensId), token);

export const readRegionSlice = (token: string, regionId: string): Promise<TerritorySlice> =>
	apiGet<TerritorySlice>(regionSlicePath(regionId), token);

export const readNeighborhood = (
	token: string,
	teamId: string,
	req: SliceRequest
): Promise<AtlasSubgraph> => apiPost<AtlasSubgraph>(neighborhoodSlicePath(teamId), token, req);

export const readTrail = (token: string, kind: ElementKind, id: string): Promise<EventTrail> =>
	apiGet<EventTrail>(trailPath(kind, id), token);

export const listTeams = (token: string): Promise<TeamRow[]> =>
	apiGet<TeamRow[]>(teamsListPath(), token);
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/server/graph-reads.paths.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/server/graph-reads.ts packages/temper-ui/src/lib/server/graph-reads.paths.test.ts
git commit -m "feat(atlas): server read wrappers for R1–R5 + teams list"
```

---

### Task 4: Territory packing layout

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.test.ts`

**Interfaces:**
- Consumes: `Territory` from `$lib/types/generated/graph_territory`; `pack`, `hierarchy` from `d3-hierarchy`.
- Produces: `interface PositionedTerritory { id: string; kind: Territory['kind']; label: string | null; anchorId: string; x: number; y: number; r: number; salience: number | null }`, `packTerritories(territories: Territory[], size: { width: number; height: number }): PositionedTerritory[]`.
- Consumed by: Task 8 (TierPanorama).

- [ ] **Step 1: Write the failing test**

```ts
// packTerritories.test.ts
import { describe, expect, it } from 'vitest';
import { packTerritories } from './packTerritories';
import type { Territory } from '$lib/types/generated/graph_territory';

const terr = (id: string, member_count: number): Territory => ({
	id,
	kind: 'region',
	label: id,
	member_count,
	salience: 0.5,
	anchor_id: `anchor-${id}`
});

describe('packTerritories', () => {
	it('returns one positioned circle per territory, inside the box', () => {
		const out = packTerritories([terr('a', 10), terr('b', 4), terr('c', 1)], {
			width: 400,
			height: 300
		});
		expect(out).toHaveLength(3);
		for (const p of out) {
			expect(p.x - p.r).toBeGreaterThanOrEqual(0);
			expect(p.x + p.r).toBeLessThanOrEqual(400);
			expect(p.y - p.r).toBeGreaterThanOrEqual(0);
			expect(p.y + p.r).toBeLessThanOrEqual(300);
			expect(p.r).toBeGreaterThan(0);
		}
	});

	it('sizes radius monotonically with member_count', () => {
		const out = packTerritories([terr('big', 100), terr('small', 1)], {
			width: 400,
			height: 400
		});
		const big = out.find((p) => p.id === 'big')!;
		const small = out.find((p) => p.id === 'small')!;
		expect(big.r).toBeGreaterThan(small.r);
	});

	it('carries kind/label/anchor through and floors member_count at 1', () => {
		const out = packTerritories([{ ...terr('z', 0), kind: 'context', label: 'ctx' }], {
			width: 200,
			height: 200
		});
		expect(out[0]).toMatchObject({ id: 'z', kind: 'context', label: 'ctx', anchorId: 'anchor-z' });
		expect(out[0].r).toBeGreaterThan(0); // member_count 0 floored so it still gets a circle
	});

	it('returns [] for no territories', () => {
		expect(packTerritories([], { width: 10, height: 10 })).toEqual([]);
	});
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/packTerritories.test.ts`
Expected: FAIL — cannot find module `./packTerritories`.

- [ ] **Step 3: Write the implementation**

```ts
// packTerritories.ts
/**
 * Tier-0 / Tier-1 cartographic layout: pack territories into circles sized by
 * member_count using d3-hierarchy. Pure — takes data + a box, returns positions.
 * No force simulation (force runs only on Tier-2 neighborhoods, per spec D1).
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { Territory } from '$lib/types/generated/graph_territory';

export interface PositionedTerritory {
	id: string;
	kind: Territory['kind'];
	label: string | null;
	anchorId: string;
	x: number;
	y: number;
	r: number;
	salience: number | null;
}

interface PackDatum {
	territory?: Territory;
	children?: PackDatum[];
}

export function packTerritories(
	territories: Territory[],
	size: { width: number; height: number }
): PositionedTerritory[] {
	if (territories.length === 0) return [];

	const root = hierarchy<PackDatum>({
		children: territories.map((t) => ({ territory: t }))
	})
		.sum((d) => (d.territory ? Math.max(1, d.territory.member_count) : 0));

	const layout = pack<PackDatum>().size([size.width, size.height]).padding(6);
	const packed = layout(root);

	return packed.leaves().map((leaf) => {
		const t = leaf.data.territory!;
		return {
			id: t.id,
			kind: t.kind,
			label: t.label,
			anchorId: t.anchor_id,
			x: leaf.x,
			y: leaf.y,
			r: leaf.r,
			salience: t.salience
		};
	});
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/layout/packTerritories.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.test.ts
git commit -m "feat(atlas): packTerritories — d3-hierarchy Tier-0 territory layout"
```

---

### Task 5: Zoom camera

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/camera.ts`

**Interfaces:**
- Consumes: `zoom` from `d3-zoom`, `select` from `d3-selection`.
- Produces: `interface Camera { destroy(): void }`, `attachCamera(svgEl: SVGSVGElement, viewportEl: SVGGElement, opts: { min: number; max: number }): Camera`.
- Consumed by: Task 9 (AtlasCanvas).

This is a thin DOM wrapper (not unit-tested — no jsdom, matching the repo's untested-render boundary). It is verified when AtlasCanvas runs (Task 9). Zoom is pan/zoom of the viewport `<g>` only — it never touches tier (spec D2).

- [ ] **Step 1: Write the implementation**

```ts
// camera.ts
/**
 * d3-zoom camera: applies pan/zoom transforms to the viewport <g> inside the SVG.
 * Decoupled from tier — zoom is within-tier observability only (spec D2). Returns a
 * handle with destroy() to unwire on component teardown.
 */
import { select } from 'd3-selection';
import { zoom, type ZoomBehavior } from 'd3-zoom';

export interface Camera {
	destroy(): void;
}

export function attachCamera(
	svgEl: SVGSVGElement,
	viewportEl: SVGGElement,
	opts: { min: number; max: number }
): Camera {
	const svg = select(svgEl);
	const viewport = select(viewportEl);

	const behavior: ZoomBehavior<SVGSVGElement, unknown> = zoom<SVGSVGElement, unknown>()
		.scaleExtent([opts.min, opts.max])
		.on('zoom', (event) => {
			viewport.attr('transform', event.transform.toString());
		});

	svg.call(behavior);

	return {
		destroy() {
			svg.on('.zoom', null);
		}
	};
}
```

- [ ] **Step 2: Verify it type-checks**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors (camera is imported nowhere yet; this just confirms d3 types resolve).

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/camera.ts
git commit -m "feat(atlas): camera.ts — d3-zoom viewport camera (decoupled from tier)"
```

---

### Task 6: Route load — SSR team scope + Tier-0 territories

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.server.ts`

**Interfaces:**
- Consumes: `parseTeam`, `parseFocus`, `deriveTier`, `parseFilters` (Task 2); `readTeamScope`, `readTerritories`, `listTeams` (Task 3); `error` from `@sveltejs/kit`.
- Produces: the `PageData` shape returned by `load` — `{ owner: string; teamId: string; scope: TeamScopeView; tier: Tier; focus: Focus; territories: TerritoryOverview | null }`. Consumed by Task 10 (`+page.svelte`) and Task 9/8 via props.

Route note: the static segment `graph` in `/vault/[owner]/graph` takes routing priority over the sibling dynamic `/vault/[owner]/[context]`, so no context may be literally named `graph`. Acceptable; the old `[context]/graph` route is deleted in Chunk D.

- [ ] **Step 1: Write the load implementation**

```ts
// +page.server.ts
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { deriveTier, parseFilters, parseFocus, parseTeam } from '$lib/graph/atlas/nav';
import { listTeams, readTeamScope, readTerritories } from '$lib/server/graph-reads';

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;

	// Resolve scope team: ?team wins; else the profile's first accessible team.
	let teamId = parseTeam(url.searchParams);
	if (!teamId) {
		const teams = await listTeams(token);
		if (teams.length === 0) throw error(404, 'No accessible teams to graph.');
		teamId = teams[0].id;
	}

	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);
	const filters = parseFilters(url.searchParams);

	const scope = await readTeamScope(token, teamId);

	// C1 renders Tier 0 fully; Tier 1/2 payloads land in C2. Only fetch what we draw.
	const territories = tier === 0 ? await readTerritories(token, teamId, filters.lensId) : null;

	return { owner: params.owner, teamId, scope, tier, focus, territories };
};
```

- [ ] **Step 2: Verify it type-checks**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors. (Runtime verification happens in Task 10 once the page exists.)

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.server.ts"
git commit -m "feat(atlas): /vault/[owner]/graph load — SSR team scope + Tier-0 territories"
```

---

### Task 7: SVG mark components

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/TeamZoneMark.svelte`
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/OrphanNodeMark.svelte`

**Interfaces:**
- `TerritoryCircle` props: `{ x: number; y: number; r: number; kind: 'region' | 'context' | 'cogmap'; label: string | null }`.
- `TeamZoneMark` props: `{ x: number; y: number; width: number; height: number; name: string; resourceCount: number; onEnter: () => void }`.
- `OrphanNodeMark` props: `{ x: number; y: number; title: string; docType: string | null }`.
- Consumed by: Task 8 (TierPanorama).

These are tiny presentational SVG fragments. Verified visually in Task 10.

- [ ] **Step 1: Write `TerritoryCircle.svelte`**

```svelte
<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';

	interface Props {
		x: number;
		y: number;
		r: number;
		kind: Territory['kind'];
		label: string | null;
	}
	let { x, y, r, kind, label }: Props = $props();

	// Region = warm-neutral tint; context = cool tint; cogmap = warm tint. Low-opacity
	// washes with a dashed hull outline, cartographic style.
	const TINTS: Record<Territory['kind'], string> = {
		region: '#e0b060',
		context: '#6fa8c7',
		cogmap: '#e8942e'
	};
	const tint = $derived(TINTS[kind]);
</script>

<g class="territory">
	<circle
		cx={x}
		cy={y}
		{r}
		fill={tint}
		fill-opacity="0.09"
		stroke={tint}
		stroke-opacity="0.4"
		stroke-width="1.5"
		stroke-dasharray="6 4"
	/>
	{#if label}
		<text
			x={x}
			y={y}
			text-anchor="middle"
			fill={tint}
			font-size="11"
			font-weight="600"
			letter-spacing="1"
			style="text-transform:uppercase"
		>
			{label}
		</text>
	{/if}
</g>
```

- [ ] **Step 2: Write `TeamZoneMark.svelte`**

```svelte
<script lang="ts">
	interface Props {
		x: number;
		y: number;
		width: number;
		height: number;
		name: string;
		resourceCount: number;
		onEnter: () => void;
	}
	let { x, y, width, height, name, resourceCount, onEnter }: Props = $props();
</script>

<g class="team-zone" role="button" tabindex="0" onclick={onEnter} onkeydown={(e) => e.key === 'Enter' && onEnter()} style="cursor:pointer">
	<rect
		{x}
		{y}
		{width}
		{height}
		rx="10"
		fill="#6fa8c7"
		fill-opacity="0.07"
		stroke="#6fa8c7"
		stroke-opacity="0.5"
		stroke-width="1.5"
		stroke-dasharray="6 4"
	/>
	<text x={x + 10} y={y + 18} fill="#9fc4d6" font-size="11" font-weight="600">▸ {name} ⏎</text>
	<text x={x + 10} y={y + 32} fill="#5f7686" font-size="9">{resourceCount} nodes</text>
</g>
```

- [ ] **Step 3: Write `OrphanNodeMark.svelte`**

```svelte
<script lang="ts">
	import { docTypeHue, isAuthored } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		title: string;
		docType: string | null;
	}
	let { x, y, title, docType }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(isAuthored(docType));
</script>

<g class="orphan">
	{#if filled}
		<circle cx={x} cy={y} r="5" fill={color} />
	{:else}
		<circle cx={x} cy={y} r="5" fill="#1b1e26" stroke={color} stroke-width="2.5" />
	{/if}
	<text x={x + 10} y={y + 3} fill="#c7d0da" font-size="11">{title}</text>
</g>
```

- [ ] **Step 4: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/marks/"
git commit -m "feat(atlas): SVG mark components — territory circle, team zone, orphan node"
```

---

### Task 8: TierPanorama component

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`

**Interfaces:**
- Consumes: `TerritoryOverview`, `TeamZone` (`$lib/types/generated/*`); `packTerritories` (Task 4); the three marks (Task 7); `buildScopeUrl` (Task 2); `goto` from `$app/navigation`, `page` from `$app/stores`.
- Props: `{ overview: TerritoryOverview; zones: TeamZone[]; width: number; height: number }`.
- Consumed by: Task 9 (AtlasCanvas).

Zones are laid out in a simple top row (a full DAG-zone layout is a later refinement); territories are packed in the lower area; orphan nodes render at packed-orphan-territory positions if present, else skipped. Clicking a zone re-scopes via `goto(buildScopeUrl(...))`.

- [ ] **Step 1: Write the component**

```svelte
<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { packTerritories } from '$lib/graph/atlas/layout/packTerritories';
	import { buildScopeUrl } from '$lib/graph/atlas/nav';
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

	// Zones occupy a top band; territories pack the rest.
	const ZONE_BAND = 120;
	const ZONE_W = 170;
	const ZONE_H = 90;

	const packed = $derived(
		packTerritories(overview.territories, { width, height: Math.max(1, height - ZONE_BAND) })
	);

	function enterZone(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
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

<!-- this scope's own territories -->
<g transform={`translate(0, ${ZONE_BAND})`}>
	{#each packed as t (t.id)}
		<TerritoryCircle x={t.x} y={t.y} r={t.r} kind={t.kind} label={t.label} />
	{/each}

	<!-- sparsity fallback: orphan salient nodes drawn directly -->
	{#each overview.orphan_nodes as o, i (o.id)}
		<OrphanNodeMark x={40} y={20 + i * 22} title={o.title} docType={o.doc_type} />
	{/each}
</g>
```

- [ ] **Step 2: Verify type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 3: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte"
git commit -m "feat(atlas): TierPanorama — Tier-0 zones + packed territories + orphan nodes"
```

---

### Task 9: AtlasCanvas component

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte`

**Interfaces:**
- Consumes: `attachCamera` (Task 5); `paletteStyleVars` (Task 1); `TierPanorama` (Task 8); the load `PageData` fields (`tier`, `scope`, `territories`); `onMount`/`onDestroy`.
- Props: `{ tier: number; territories: import('$lib/types/generated/graph_territory').TerritoryOverview | null; zones: import('$lib/types/generated/graph_scope').TeamZone[] }`.
- Consumed by: Task 10 (`+page.svelte`).

Owns the SVG root, the viewport `<g>`, and the d3-zoom camera. Renders `TierPanorama` when `tier === 0` and territories are present; a placeholder for Tier 1/2 (their renderers land in C2). Applies the palette CSS vars on the root so descendant CSS can reference `--dt-*`.

- [ ] **Step 1: Write the component**

```svelte
<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierPanorama from './TierPanorama.svelte';

	interface Props {
		tier: number;
		territories: TerritoryOverview | null;
		zones: TeamZone[];
	}
	let { tier, territories, zones }: Props = $props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="atlas-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Team graph atlas">
		<rect x="0" y="0" width={W} height={H} fill="#1b1e26" />
		<g bind:this={viewportEl}>
			{#if tier === 0 && territories}
				<TierPanorama overview={territories} {zones} width={W} height={H} />
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">
					Tier {tier} view lands in Chunk C2
				</text>
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
git commit -m "feat(atlas): AtlasCanvas — SVG root + d3-zoom camera + tier dispatch"
```

---

### Task 10: Page shell + ScopeBar, end-to-end verification

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte`
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.svelte`

**Interfaces:**
- `ScopeBar` props: `{ scope: import('$lib/types/generated/graph_scope').TeamScopeView }` — for C1 it shows the current team name + the ancestor breadcrumb (`scope.ancestors`). A team *switcher* dropdown is a small C3 follow-up.
- `+page.svelte`: `let { data }: { data: PageData } = $props()`.

- [ ] **Step 1: Write `ScopeBar.svelte`**

```svelte
<script lang="ts">
	import type { TeamScopeView } from '$lib/types/generated/graph_scope';

	interface Props {
		scope: TeamScopeView;
	}
	let { scope }: Props = $props();
</script>

<nav class="scope-bar">
	{#each scope.ancestors as ancestor (ancestor.id)}
		<span class="crumb">{ancestor.name}</span>
		<span class="sep">/</span>
	{/each}
	<span class="crumb current">{scope.team.name}</span>
</nav>

<style>
	.scope-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
	.crumb.current {
		font-weight: 600;
	}
	.sep {
		opacity: 0.4;
	}
</style>
```

- [ ] **Step 2: Write `+page.svelte`**

```svelte
<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';

	let { data }: { data: PageData } = $props();
</script>

<div class="atlas-page">
	<ScopeBar scope={data.scope} />
	<AtlasCanvas tier={data.tier} territories={data.territories} zones={data.scope.zones} />
</div>

<style>
	.atlas-page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
</style>
```

- [ ] **Step 3: Type-check**

Run: `cd packages/temper-ui && bun run check`
Expected: 0 errors.

- [ ] **Step 4: End-to-end manual verification**

Prereq: the API must be running with a seeded team that has territories (the deployed steward's corpus, or a local seed). Then:

Run: `cd packages/temper-ui && bun run dev`

In the browser, open `/vault/@me/graph` (or `/vault/@me/graph?team=<known-team-uuid>`). Verify:
1. The page loads with no error; the ScopeBar shows the team name (and any ancestors).
2. The slate canvas renders territory circles (sized by member_count) and, if the team has child zones, enterable zone rectangles across the top.
3. Mouse-wheel / trackpad zoom and drag-pan move the whole scene (camera works).
4. Clicking a team zone changes the URL to `?team=<childId>` and the canvas re-loads scoped to that child (breadcrumb updates). The browser back button returns to the parent scope.
5. If a context/cogmap has no region, its salient orphan nodes render directly (sparsity fallback).

Expected: all five hold. If territories are absent for the seed, at minimum (1), (3), (4) hold and the canvas shows the empty slate with zones.

- [ ] **Step 5: Run the full check + unit suite**

Run: `cd packages/temper-ui && bun run check && bunx vitest run`
Expected: svelte-check 0 errors; all `atlas/*` unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add "packages/temper-ui/src/lib/components/graph/atlas/ScopeBar.svelte" "packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.svelte"
git commit -m "feat(atlas): /vault/[owner]/graph page shell + ScopeBar — Tier-0 panorama end-to-end"
```

---

## Self-Review

**1. Spec coverage (C1 slice of the Chunk-C spec):**
- D1 renderer (d3 math + Svelte SVG): Tasks 4 (packTerritories/d3-hierarchy), 5 (camera/d3-zoom), 7–9 (Svelte-SVG rendering). ✓
- D2 navigation (URL frame, tier from focus, zoom decoupled): Task 2 (nav), Task 6 (load derives tier), Task 8 (zone re-scope via goto), Task 5 (camera never touches tier). ✓
- D3/D4/D5 palette (Vivid Cartographer, goal cool, one set + ring, single source): Task 1. Hull tints/edge colors/salience ramp are defined in palette.ts; full edge rendering + light-mode ring application land with the Tier-2 marks in C2 (noted). ✓ for C1 scope.
- Component decomposition + SSR-initial load: Tasks 6–10 follow the spec's file tree; `reads` relocated to `$lib/server/graph-reads.ts` for the SvelteKit server-only boundary (documented deviation). ✓
- R1+R2 consumed (Task 3/6); R3/R4/R5 wrappers written (Task 3) but consumed in C2/C3 — intentional (C1 is Tier-0 only). ✓
- **Out of C1 scope (→ C2/C3, by design):** Tier-1/2 renderers, `forceNeighborhood`, `hull`, full `NodeChip`/`Edge` marks, TrailRail, SearchAccelerator, AtlasLegend, filter UI. Not gaps — the plan's stated phase boundary.

**2. Placeholder scan:** No TBD/TODO. The "Tier N lands in C2" canvas text is an intentional runtime placeholder for out-of-phase tiers, not a plan placeholder — every C1 step has complete code.

**3. Type consistency:** `PositionedTerritory` fields (Task 4) match TierPanorama usage (Task 8). `Focus`/`Tier` (Task 2) match the load return (Task 6) and AtlasCanvas prop (`tier`). `nodeMark`/`docTypeHue`/`isAuthored` signatures (Task 1) match OrphanNodeMark usage (Task 7). Read wrapper return types (Task 3) match the load's `scope`/`territories` (Task 6). `TeamZone.resource_count` / `TeamScopeView.zones` / `.ancestors` (generated) match Tasks 8/10 usage.
