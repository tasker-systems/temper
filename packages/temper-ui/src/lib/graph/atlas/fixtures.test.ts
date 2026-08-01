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
// harness/page contract lives). Nested payload types (AtlasSubgraph, TerritoryOverview, …)
// are checked where components consume them via svelte-check.
import { describe, expect, it } from 'vitest';
import type { HomeContext } from '$lib/types/generated/graph_home';
import { resourceHref } from '$lib/vault-url';
import { WORDS } from '../../../../scripts/atlas-synthetic-vocabulary.mjs';
import bundleJson from '../../../../static/dev/atlas-fixtures.json';
import type { AtlasFixtureBundle } from '../../../routes/dev/atlas/+page';
import type { AtlasViewData } from './viewData';

// Pinned to AtlasViewData: if the type gains/loses a field, this object stops
// satisfying `Record<keyof AtlasViewData, true>` and `bun run check` fails, forcing
// the fixtures (and this list) back into lockstep with the type.
const REQUIRED_KEYS = {
	owner: true,
	cogmapId: true,
	cogmapName: true,
	contextSlug: true,
	panorama: true,
	tier: true,
	focus: true,
	home: true,
	territories: true,
	neighborhood: true,
	selection: true,
	trail: true,
	resourceRow: true,
	filters: true,
	focusPath: true,
	crumbTerritory: true,
	scopeFilter: true,
} satisfies Record<keyof AtlasViewData, true>;

const EXPECTED_KEYS = Object.keys(REQUIRED_KEYS).sort();

// Every scenario the harness offers must be present so a fresh checkout can drive it.
const EXPECTED_SCENARIOS = [
	'home',
	'nodeNeighborhood',
	'nodeSelected',
	'nodeSelectedContext',
	'cogmapPanorama',
	'leafBare',
	'regionDrill',
	'regionDrillUnion',
	'contextPanorama',
	'contextDrill',
	// The measured-substrate refresh (2026-08-01). Each name below is a corpus shape that
	// the pre-refresh bundle could not express, so a design validated in `/dev/atlas`
	// against the old fixtures was validated against a corpus that no longer exists.
	'coldStartCogmap',
	'sparseAnchor',
	'residualDrill',
	'nearEmptyAnchor',
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

	// The TrailRail "View full resource →" button used to be gated on
	// `resourceHref(resourceRow)` being non-null, and these two scenarios asserted each
	// side of that gate. The gate is gone: the resource route is home-agnostic, so both
	// homes resolve. The fixtures still carry one of each home, which is what makes them
	// worth asserting — this is now the live example that homing does NOT decide
	// addressability.
	it('nodeSelected is cogmap-homed → resourceHref still resolves (the 533-resource fix)', () => {
		const rr = scenario('nodeSelected').resourceRow;
		expect(rr).not.toBeNull();
		expect(rr!.context_owner_ref).toBeNull();
		expect(rr!.cogmap_id).not.toBeNull();
		expect(resourceHref(rr!)).toBe(`/vault/r/${rr!.id}`);
	});

	it('nodeSelectedContext is context-homed → resourceHref resolves to the same shape', () => {
		const view = scenario('nodeSelectedContext');
		expect(view.selection?.kind).toBe('node');
		const rr = view.resourceRow;
		expect(rr).not.toBeNull();
		expect(rr!.context_owner_ref).not.toBeNull();
		expect(rr!.cogmap_id).toBeNull();
		expect(resourceHref(rr!)).toBe(`/vault/r/${rr!.id}`);
	});

	it('home build entries carry a recency field and no leaked PII (Beat C)', () => {
		const home = scenario('home').home;
		for (const c of home?.build ?? []) {
			expect(c).toHaveProperty('last_active_at'); // string | null
			// key-set pinned to the wire type:
			const _pin = c satisfies HomeContext;
			void _pin;
		}
	});

	// ── The measured corpus shapes ─────────────────────────────────────────────────────
	// Each assertion below pins ONE shape measured on prod 2026-08-01 that the previous
	// fixture set could not express. They are deliberately weak on magnitude and strict on
	// KIND: the corpus keeps moving, so an assertion tuned to "355 resources" would break on
	// every re-capture, while "this anchor has material and no derived structure at all"
	// stays true for as long as the shape does — and fails loudly when a re-capture no
	// longer finds it, which is the signal worth having.

	it('coldStartCogmap: a cogmap with material and ZERO live regions', () => {
		// 6 of 11 non-empty anchors are in this state, incl. the L0 kernel `system-default`.
		// This is `entry-does-not-presume-organization` in its literal form — the single most
		// important case the old bundle omitted.
		const view = scenario('coldStartCogmap');
		expect(view.territories).not.toBeNull();
		expect(view.territories!.territories).toHaveLength(0);
		// With no region to draw, the sparsity rule is the only thing standing between the
		// reader and an empty screen.
		expect(view.territories!.orphan_nodes.length).toBeGreaterThan(0);
	});

	it('cogmapPanorama: singleton-dominated, the real shape of the dominant map', () => {
		// Measured 64.5% singleton. A cogmap region is a connected clump of the DECLARED
		// graph (`w_cos = 0`), so a node with no declared edge and no shared facet is in no
		// candidate pair at all — it is off the tree, not below the cut.
		const territories = scenario('cogmapPanorama').territories?.territories ?? [];
		expect(territories.length).toBeGreaterThan(0);
		const singletons = territories.filter((t) => t.member_count === 1).length;
		expect(singletons / territories.length).toBeGreaterThan(0.5);
	});

	/** Resources in a panorama's residual tray — the material reaching no container. */
	const residualTotal = (p: { residual: { buckets: { count: number }[] } }) =>
		p.residual.buckets.reduce((a, b) => a + b.count, 0);

	it('contextPanorama: genuinely clustered — containers with real multi-member spread', () => {
		// The contrast with the singleton-dominated cogmap above IS the two-kinds-of-region
		// finding. NOTE what this can and cannot show: no endpoint returns context REGIONS
		// (`graph_cogmap_territories` filters `WHERE reg.cogmap_id = p_cogmap`, and a context
		// region carries `cogmap_id IS NULL`), so the clustered-context case is expressible
		// only through the container panorama. See the README's "known gap".
		const panorama = scenario('contextPanorama').panorama;
		expect(panorama).not.toBeNull();
		expect(panorama!.containers.length).toBeGreaterThan(0);
		const multi = panorama!.containers.filter((c) => c.member_count > 1).length;
		expect(multi).toBeGreaterThan(0);
		// Being the MOST organized anchor does not make it organized: measured 2026-08-01, 898
		// of this context's 1,462 resources (61%) reach no container at all. An "organized"
		// panorama that showed only its containers would present a 39% view as a complete one —
		// which is why the tray is part of the Tier-0 payload and part of this fixture.
		expect(residualTotal(panorama!)).toBeGreaterThan(0);
	});

	it('sparseAnchor: real volume, overwhelmingly unorganized', () => {
		// `entry-does-not-presume-organization` with weight behind it. Selected by residual
		// FRACTION above a volume floor, NOT by `containers.length === 0` — on the measured
		// corpus no context with real volume has zero containers, so a zero-container rule
		// silently degrades this scenario into a one-resource context and it stops being
		// about volume at all.
		const panorama = scenario('sparseAnchor').panorama;
		expect(panorama).not.toBeNull();
		const contained = panorama!.containers.reduce((a, c) => a + c.member_count, 0);
		expect(residualTotal(panorama!)).toBeGreaterThan(contained);
	});

	it('residualDrill: region-less material is reachable, at scale', () => {
		// Drilled on the DENSEST context, where the residue is a scale question rather than a
		// handful of stragglers. Also the one scenario that exercises an asymmetry worth
		// knowing: `context_composition` caps SEEDS (250) but has no node cap, where the cogmap
		// door's `region_composition_slice` truncates at 120 nodes.
		const view = scenario('residualDrill');
		expect(view.focus.kind).toBe('bucket');
		expect(view.tier).toBe(1);
		expect(view.neighborhood?.nodes.length ?? 0).toBeGreaterThan(0);
	});

	it('nearEmptyAnchor: an anchor with nothing to organize by still renders', () => {
		// NEAR-empty, not empty. `graph_home_contexts` applies no resource-count filter, so a
		// wholly empty context would appear at the home door if one were readable — on the
		// measured corpus none is. That shape is declared uncovered in the README rather than
		// fabricated here.
		const panorama = scenario('nearEmptyAnchor').panorama;
		expect(panorama).not.toBeNull();
		expect(panorama!.containers).toHaveLength(0);
		expect(residualTotal(panorama!)).toBeLessThanOrEqual(5);
	});

	it('home: concentration, not average — one enormous anchor and a near-empty tail', () => {
		// A fixture of evenly-sized anchors tunes the design on a case that barely exists.
		// Asserted as a SPREAD, not as magnitudes, so a re-capture does not churn this.
		const home = scenario('home').home;
		expect(home).not.toBeNull();
		const counts = home!.build.map((c) => c.resource_count).sort((a, b) => b - a);
		expect(counts.length).toBeGreaterThan(2);
		expect(counts[0]).toBeGreaterThan(100);
		expect(counts[counts.length - 1]).toBeLessThanOrEqual(5);
	});

	it('coldStartCogmap and sparseAnchor are cold in DIFFERENT senses', () => {
		// Not redundancy, and not sloppiness — the two doors read different things. A cogmap
		// panorama draws REGIONS, so its cold case is "zero live regions". A context panorama
		// draws goal-rooted CONTAINERS and cannot see regions at all, so its cold case is
		// "material that reaches no container". Collapsing them would lose one of the two.
		expect(scenario('coldStartCogmap').territories!.territories).toHaveLength(0);
		expect(scenario('coldStartCogmap').panorama).toBeNull();
		expect(scenario('sparseAnchor').territories).toBeNull();
		expect(scenario('sparseAnchor').panorama).not.toBeNull();
	});

	it('leaks no personal data from the original capture', () => {
		// Denylist of real strings/ids that appeared in the personal capture this bundle
		// was sanitized from. A raw capture committed by mistake would trip this.
		const DENY = [
			'cole',
			'taylor',
			'self-cognition',
			'019eea5e', // real personal-team id prefix
			'agent-y23aqxuvzjysb5n8laueuigixoftcwyu',
		];
		// Serialize scenarios only (skip _meta, whose note legitimately says "personal-data-free").
		const haystack = JSON.stringify(scenarioNames.map(scenario)).toLowerCase();
		for (const needle of DENY) {
			expect(haystack, `personal-data leak: "${needle}"`).not.toContain(needle.toLowerCase());
		}
	});

	// A denylist can only catch strings someone thought to list. It cannot catch a NEW leak
	// surface — and the Rust capture path opened three the browser capture never had: real
	// first-paragraph `excerpt` prose, real `actor_name` display names, and territory
	// `label`s (which fall back to a member's resource title). So this is the positive form
	// of the same guard: every free-text value the sanitizer is responsible for must be
	// built from its own word bank. Anything else means a value reached the committed bundle
	// without passing through `synthText`.
	it('free text is drawn only from the sanitizer word bank', () => {
		const bank = new Set([...WORDS, 'system']); // `system` is the synthetic actor name
		const offenders: string[] = [];

		const check = (label: string, value: unknown) => {
			if (typeof value !== 'string' || value.length === 0) return;
			for (const token of value
				.toLowerCase()
				.split(/[^a-z]+/)
				.filter(Boolean)) {
				if (!bank.has(token)) offenders.push(`${label}: ${JSON.stringify(value)}`);
			}
		};

		for (const name of scenarioNames) {
			const view = scenario(name);
			for (const n of view.neighborhood?.nodes ?? []) {
				check(`${name}.neighborhood.node.title`, n.title);
				check(`${name}.neighborhood.node.excerpt`, n.excerpt);
			}
			for (const e of view.trail?.events ?? []) check(`${name}.trail.actor_name`, e.actor_name);
			for (const t of view.territories?.territories ?? [])
				check(`${name}.territories.label`, t.label);
			for (const o of view.territories?.orphan_nodes ?? []) {
				check(`${name}.orphan.title`, o.title);
				check(`${name}.orphan.anchor_label`, o.anchor_label);
			}
			for (const c of view.panorama?.containers ?? []) check(`${name}.containers.label`, c.label);
			check(`${name}.cogmapName`, view.cogmapName);
			check(`${name}.resourceRow.title`, view.resourceRow?.title);
			for (const c of view.home?.build ?? []) check(`${name}.home.build.name`, c.name);
			for (const c of view.home?.research ?? []) check(`${name}.home.research.name`, c.name);
		}

		// Report every offender at once — fixing them one round-trip at a time is the slow way
		// to discover that a whole field was missed.
		expect(offenders.slice(0, 20), `un-sanitized free text (${offenders.length} total)`).toEqual(
			[],
		);
	});
});
