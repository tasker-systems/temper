/**
 * The fixture → view adapter the dev harness renders through.
 *
 * **Why this is a `$lib` module and not a helper inside the route.** The harness exists to show a
 * person the real `GraphPage` against real-shaped data, and it is only worth looking at if the
 * `GraphViewData` it renders is assembled the way `+page.server.ts` assembles one. So this calls
 * the same builders the load calls — `buildGraph`, `buildEntryGraph`, `buildTraversal`,
 * `declareBounds`, `declareEntryBounds`, `declareTraversalBounds`, `buildReadout` — and does no
 * shaping of its own. Living here rather than in `routes/dev/` also lets the fixture guard import
 * it, so what the guard checks and what the harness draws cannot diverge.
 *
 * What it deliberately does NOT reproduce is the load's *reading*: no auth, no network, no
 * refusals decided from an address. A fixture is an answer that already came back.
 *
 * @see src/routes/dev/graph/README.md
 */
import type {
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
	ShapeEmptiness,
} from '$lib/types/generated/cognitive_maps';
import type { AtlasEntry, AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import type { QueryResponse } from '$lib/types/generated/query';
import { analyseShape } from './analysis';
import { declareBounds, declareEntryBounds, declareTraversalBounds } from './bound';
import type { Anchor, GraphPlan } from './composition';
import { buildEntryGraph, buildGraph, buildTraversal } from './model';
import { buildReadout } from './readout';
import type { AnalysisViewData, GraphViewData } from './view';

/** An anchor as the capture stores it — the list-read row plus the display name its home carries. */
export interface HarnessAnchor extends Anchor {
	name: string;
}

/** A scenario that answered a composition: `POST /api/query` plus the regions naming its groupings. */
export interface CompositionScenario {
	_why: string;
	question: string | null;
	borrowedFrom: GraphViewData['borrowedFrom'];
	anchorsAsked: Anchor[];
	anchorsAvailable: number;
	surveyStages: string[];
	walkStage: string;
	response: QueryResponse;
	/**
	 * The region ROWS, not the shape envelope. The door answers an `AnchorShape`;
	 * `capture-graph-fixtures.ts` takes `.regions` at capture time, exactly where
	 * `readAnchorRegions` takes it at load time, so this field feeds `RegionLookup.rows` unchanged.
	 */
	shape_rows: CogmapRegionRow[];
}

/** A scenario that answered the entry read — no question, no seeds. */
export interface EntryScenario {
	_why: string;
	/** What this fixture cannot witness, declared beside the data rather than in a note. */
	_does_not_witness?: string | null;
	entry: AtlasEntry;
}

/** A scenario that answered the `?from=` walk. */
export interface TraversalScenario {
	_why: string;
	seeds: string[];
	depth: number;
	subgraph: AtlasSubgraph;
}

export interface HarnessBundle {
	_captured: Record<string, unknown>;
	_sanitized?: Record<string, unknown>;
	_anchors: HarnessAnchor[];
	[scenario: string]:
		| CompositionScenario
		| EntryScenario
		| TraversalScenario
		| HarnessAnchor[]
		| Record<string, unknown>
		| undefined;
}

/** The keys that are the bundle's own bookkeeping rather than a scenario. */
const META_KEYS = new Set([
	'_captured',
	'_sanitized',
	'_anchors',
	// Provenance for the `authored_*` scenarios: WRITTEN, not observed. A fixed set, so a new
	// bookkeeping key that is not added here becomes a SCENARIO and fails the key-set guard loudly
	// rather than quietly rendering as one.
	'_authored',
]);

export const scenarioNames = (bundle: HarnessBundle): string[] =>
	Object.keys(bundle).filter((k) => !META_KEYS.has(k));

const isEntry = (s: object): s is EntryScenario => 'entry' in s;
const isTraversal = (s: object): s is TraversalScenario => 'subgraph' in s;

/**
 * The anchor-id → home-label map.
 *
 * The load builds this from its two list reads (`homesOf`); the capture stores the same pairs so
 * the harness does not carry a second copy of that mapping free to disagree with the one that ships.
 */
const homesOf = (anchors: HarnessAnchor[]): Map<string, string> =>
	new Map(anchors.map((a) => [a.id, a.name]));

/** Everything in a `GraphViewData` is a promise here; a fixture is an answer that already landed. */
const settled = <T>(v: T): Promise<T> => Promise.resolve(v);

/**
 * Build the view for one scenario.
 *
 * `owner` is `@me` because a capture is always taken AS the authenticated profile, and `@me` is the
 * canonical self-addressed form every real page load carries.
 */
export function viewFor(bundle: HarnessBundle, name: string): GraphViewData {
	const scenario = bundle[name] as object | undefined;
	if (!scenario) throw new Error(`no such scenario: ${name}`);
	const homes = homesOf(bundle._anchors);

	const base = {
		owner: '@me',
		borrowedFrom: null,
		// The harness renders answers, never addresses, so no branch here can produce an
		// address-decided refusal. Spelled as a constant rather than plumbed, so that nothing in
		// this file looks like it is deciding one.
		refusal: null,
		selected: settled<string | null>(null),
		selectedExcerpt: null,
		selectedTrail: null,
	} satisfies Partial<GraphViewData>;

	if (isEntry(scenario)) {
		const e = scenario.entry;
		const bounds = {
			drawn: e.bounds.drawn,
			eligible: e.bounds.eligible,
			inScope: e.bounds.in_scope,
			truncated: e.bounds.truncated,
		};
		return {
			...base,
			question: null,
			model: settled(buildEntryGraph(e, homes)),
			// The entry read ranks across every readable anchor when unaddressed, which is what the
			// captures do; `asked`/`available` therefore both count the whole readable set.
			bound: settled(
				declareEntryBounds(bounds, {
					asked: bundle._anchors.length,
					available: bundle._anchors.length,
				}),
			),
			// No composition ran, so there is no reasoning to report — absent, never an empty readout.
			readout: settled(null),
			// Rung 2, on the same predicate the load uses: `eligible === 0` is the read's own verdict
			// about the answer it just produced, and it is the only branch that can reach it.
			tooLittleStructure: settled(
				bounds.eligible === 0
					? { kind: 'too-little-structure' as const, inScope: bounds.inScope }
					: null,
			),
			placesAsked: [],
		};
	}

	if (isTraversal(scenario)) {
		const model = buildTraversal(scenario.subgraph, scenario.seeds, homes);
		return {
			...base,
			question: null,
			model: settled(model),
			bound: settled(
				declareTraversalBounds({
					drawn: model.nodes.length,
					from: model.nodes.filter((n) => scenario.seeds.includes(n.id)).length,
					depth: scenario.depth,
				}),
			),
			readout: settled(null),
			// A walk ranks and cuts nothing, so it has no axis on which to reach rung 2.
			tooLittleStructure: settled(null),
			placesAsked: [],
		};
	}

	const c = scenario as CompositionScenario;
	const plan = {
		composition: { outcome: { returns: [] }, stages: [] },
		anchorsAsked: c.anchorsAsked,
		anchorsAvailable: c.anchorsAvailable,
		surveyStages: c.surveyStages,
		walkStage: c.walkStage,
	} as unknown as GraphPlan;

	return {
		...base,
		question: c.question,
		borrowedFrom: c.borrowedFrom,
		model: settled(buildGraph({ response: c.response, plan, seeds: [] })),
		bound: settled(declareBounds(c.response, plan)),
		readout: settled(buildReadout(c.response, { rows: c.shape_rows, complete: true })),
		// A composition has no connection floor and no `eligible` axis, so it can never reach rung 2.
		tooLittleStructure: settled(null),
		placesAsked: c.anchorsAsked.map((a) => ({
			kind: a.kind,
			ref: a.ref,
			title: homes.get(a.id) ?? a.ref,
		})),
	};
}

// ── The analysis door ───────────────────────────────────────────────────────────────────────────

/** One anchor as `graph-analysis-anchors.json` stores it — the three reads the door makes. */
export interface AnalysisScenario {
	name?: string;
	ref?: string;
	/** The region ROWS — `AnchorShape.regions`, as `readAnchorAnalysis` hands them to the door. */
	shape: CogmapRegionRow[];
	/**
	 * `AnchorShape.emptiness` — why `shape` is empty, when it is.
	 *
	 * **Optional, and absent from the committed bundle**, which was captured on 2026-08-20 against
	 * the deployed API — before the shape read carried an envelope at all. So the one zero-row
	 * scenario in it (`cogmap_never_materialized`) resolves to `null` here and `/dev/analysis`
	 * renders the no-cause-given wording for it.
	 *
	 * That is deliberate rather than an omission waiting to be tidied. The obvious tidy is to write
	 * `'never_clustered'` into the fixture — its `analytics.staleness.materialized_at` is `null`, so
	 * the value would even be *right* — but the capture never observed an `emptiness`, and a fixture
	 * that states what a read said when the read did not say it is a synthesized guarantee. The
	 * other tidy, deriving the arm here from the captured fields, would put a second copy of
	 * `anchor_shape`'s arm cascade in TypeScript, free to drift from the SQL that owns it. The
	 * remainder is named instead: this bundle needs a re-capture to exercise the caused wordings on
	 * `/dev/analysis`, and until it gets one the four sentences are covered by their own tests.
	 */
	emptiness?: ShapeEmptiness | null;
	region_metrics: CogmapRegionMetricsRow[] | null;
	analytics?: CogmapAnalyticsRow | null;
}

export interface AnalysisBundle {
	_captured: Record<string, unknown>;
	_sanitized?: Record<string, unknown>;
	[anchor: string]: AnalysisScenario | Record<string, unknown> | undefined;
}

export const analysisScenarioNames = (bundle: AnalysisBundle): string[] =>
	Object.keys(bundle).filter((k) => !META_KEYS.has(k));

/**
 * Build the analysis view for one captured anchor.
 *
 * The same subtraction the graph half makes: `analyseShape` is the load's own builder, and the
 * `map` field is assembled exactly as `readAnchorAnalysis` assembles it.
 *
 * **`[2026-08-25]` A context has an anchor-level readout now** — `/api/contexts/{id}/analytics`
 * answers the staleness half — and the committed bundle carries none for its context, because the
 * capture was taken on 2026-08-20, before that door existed. So a context here resolves to `null`
 * and `/dev/analysis` renders the declined branch for it. That is a property of the CAPTURE, not of
 * the world, and it is left standing rather than filled in for the same reason `emptiness` is: a
 * fixture that states what a read said when the read was never made is a synthesized guarantee.
 * The remainder is named — this bundle needs a re-capture to exercise a context's clock on
 * `/dev/analysis`, and until it gets one that path is covered by the component tests.
 *
 * The anchor `kind` is likewise derived from whether the scenario carries an `analytics` row, which
 * holds only because of that capture date. A re-capture must carry the kind rather than infer it.
 */
export function analysisViewFor(bundle: AnalysisBundle, name: string): AnalysisViewData {
	const s = bundle[name] as AnalysisScenario | undefined;
	if (!s) throw new Error(`no such analysis anchor: ${name}`);
	const kind: 'context' | 'cogmap' = s.analytics ? 'cogmap' : 'context';
	const title = s.name ?? s.ref ?? name;
	const shape = analyseShape(s.shape, s.region_metrics ?? null);

	const choices = analysisScenarioNames(bundle).map((n) => {
		const other = bundle[n] as AnalysisScenario;
		return {
			kind: (other.analytics ? 'cogmap' : 'context') as 'context' | 'cogmap',
			ref: other.ref ?? n,
			title: other.name ?? other.ref ?? n,
		};
	});

	return {
		owner: '@me',
		place: { kind, ref: s.ref ?? name, title },
		alsoNamed: [],
		choices,
		refusal: null,
		regions: settled(shape.regions),
		metricsAvailable: settled(shape.metricsAvailable),
		emptiness: settled(s.emptiness ?? null),
		map: settled(
			s.analytics
				? {
						kind: 'cogmap' as const,
						telos: { id: s.analytics.telos_resource_id, title },
						staleness: s.analytics.staleness,
						regulation: s.analytics.regulation ?? [],
					}
				: null,
		),
	};
}
