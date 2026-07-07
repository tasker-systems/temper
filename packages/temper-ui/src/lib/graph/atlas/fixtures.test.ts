// fixtures.test.ts — guards the committed `/dev/atlas` fixture bundle.
//
// The harness renders the real `AtlasPage` against `static/dev/atlas-fixtures.json`
// (see src/routes/dev/atlas/README.md). Two failure modes this locks down:
//   1. Shape drift — the real `/graph/[owner]` page-load output (an `AtlasViewData`)
//      gains/loses a field but the committed fixtures don't, so the harness renders a
//      stale shape and only a human notices. The `REQUIRED_KEYS` map below is pinned to
//      the type via `satisfies Record<keyof AtlasViewData, true>`, so adding/removing an
//      `AtlasViewData` field fails `bun run check` at compile time; the runtime assertion
//      then requires every committed scenario to carry exactly those keys.
//   2. Personal-data leak — a raw capture (which holds real titles/handles/ids) is
//      committed by mistake instead of the sanitized bundle.
//
// Scope: this gate covers the top-level `AtlasViewData` composition (where the
// harness/page contract lives). Nested payload types (TerritorySlice, AtlasSubgraph, …)
// are checked where components consume them via svelte-check.
import { describe, expect, it } from 'vitest';
import type { AtlasViewData } from './viewData';
import type { AtlasFixtureBundle } from '../../../routes/dev/atlas/+page';
import bundleJson from '../../../../static/dev/atlas-fixtures.json';

// Pinned to AtlasViewData: if the type gains/loses a field, this object stops
// satisfying `Record<keyof AtlasViewData, true>` and `bun run check` fails, forcing
// the fixtures (and this list) back into lockstep with the type.
const REQUIRED_KEYS = {
	owner: true,
	cogmapId: true,
	cogmapName: true,
	tier: true,
	focus: true,
	home: true,
	territories: true,
	slice: true,
	neighborhood: true,
	selection: true,
	trail: true,
	resourceRow: true,
	filters: true,
	focusPath: true,
	crumbTerritory: true,
	scopeFilter: true
} satisfies Record<keyof AtlasViewData, true>;

const EXPECTED_KEYS = Object.keys(REQUIRED_KEYS).sort();

// Every scenario the harness offers must be present so a fresh checkout can drive it.
const EXPECTED_SCENARIOS = [
	'home',
	'teamPanorama',
	'regionSlice',
	'nodeNeighborhood',
	'nodeSelected',
	'cogmapPanorama',
	'leafBare'
];

const bundle = bundleJson as AtlasFixtureBundle;
const scenarioNames = Object.keys(bundle).filter((k) => k !== '_meta');
const scenario = (k: string) => bundle[k] as AtlasViewData;

describe('committed atlas fixtures', () => {
	it('carries the synthetic (personal-data-free) provenance stamp', () => {
		expect(bundle._meta?.synthetic).toBe(true);
	});

	it('includes every harness scenario, incl. the neighbor-less leaf case', () => {
		for (const name of EXPECTED_SCENARIOS) {
			expect(scenarioNames, `missing scenario "${name}"`).toContain(name);
		}
		expect(scenarioNames).toContain('leafBare');
	});

	it.each(EXPECTED_SCENARIOS)('scenario "%s" has the full AtlasViewData key set', (name) => {
		const view = scenario(name);
		expect(view, `scenario "${name}" is missing`).toBeDefined();
		expect(Object.keys(view).sort()).toEqual(EXPECTED_KEYS);
	});

	it('leafBare is a Tier-2 node view whose neighborhood has no mapped neighbors', () => {
		// The case #276 hardened: a leaf resource selected with an empty subgraph still
		// opens a working TrailRail off its resourceRow. The fixture must exercise that.
		const leaf = scenario('leafBare');
		expect(leaf.tier).toBe(2);
		expect(leaf.focus.kind).toBe('node');
		expect(leaf.resourceRow).not.toBeNull();
		const mappedNeighbors = leaf.neighborhood?.nodes.length ?? 0;
		expect(mappedNeighbors).toBeLessThanOrEqual(1); // only the focus node, or none
	});

	it('nodeSelected selects a node backed by a resourceRow (TrailRail has data)', () => {
		const view = scenario('nodeSelected');
		expect(view.selection?.kind).toBe('node');
		expect(view.resourceRow).not.toBeNull();
	});

	it('nodeSelected neighborhood nodes carry an excerpt field', () => {
		const view = scenario('nodeSelected');
		const withExcerpt = view.neighborhood?.nodes.filter((n) => typeof n.excerpt === 'string') ?? [];
		expect(withExcerpt.length).toBeGreaterThan(0);
	});

	it('nodeSelected trail events carry payload + actor_name', () => {
		const events = scenario('nodeSelected').trail?.events ?? [];
		expect(events.length).toBeGreaterThan(0);
		for (const e of events) {
			expect(e).toHaveProperty('payload');
			expect(typeof e.actor_name).toBe('string');
		}
	});

	it('leaks no personal data from the original capture', () => {
		// Denylist of real strings/ids that appeared in the personal capture this bundle
		// was sanitized from. A raw capture committed by mistake would trip this.
		const DENY = [
			'cole',
			'taylor',
			'self-cognition',
			'019eea5e', // real personal-team id prefix
			'agent-y23aqxuvzjysb5n8laueuigixoftcwyu'
		];
		// Serialize scenarios only (skip _meta, whose note legitimately says "personal-data-free").
		const haystack = JSON.stringify(scenarioNames.map(scenario)).toLowerCase();
		for (const needle of DENY) {
			expect(haystack, `personal-data leak: "${needle}"`).not.toContain(needle.toLowerCase());
		}
	});
});
