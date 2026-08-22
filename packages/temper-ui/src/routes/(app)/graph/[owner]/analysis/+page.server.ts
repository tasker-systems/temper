import { analyseShape } from '$lib/graph/analysis';
import { describeAnchor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import type { AnalysisRefusal, AnalysisViewData } from '$lib/graph/view';
import { bounded, derive } from '$lib/server/bounded';
import { readAnchorAnalysis, readAnchorSources } from '$lib/server/graph-query';
import { parseGraphAddress } from '$lib/vault-url';
import type { PageServerLoad } from './$types';

/**
 * The analysis door — the machine's own measurements of ONE place, declared as measurements.
 *
 * **This is the receiver Beat C exists to build.** The successor canvas draws the reader's own
 * resources and the edges between them and nothing else; the per-region metrics
 * `RegionHoverCard.svelte:17-19` (at `87ccd211`, the last commit before Beat D deleted it)
 * used to render on the navigational surface — member count,
 * salience, coherence — land here, alongside the map-level picture that has had no reader in the UI
 * since it was built. `displaced-structure-remains-reachable` asks that displaced structure remain
 * *"available somewhere that declares itself as analysis rather than as the reader's material"*,
 * and that declaration is this route's whole reason to be a separate place rather than a panel.
 *
 * **One place at a time**, and that is a measurement rather than a simplification: on the deployed
 * substrate `centrality` maxes at 2342.2 on the self-cognition map and 276 on `@me/temper`,
 * `salience` at 497.65 against 69.54. A single ranked list across two places would be arithmetic on
 * incommensurable quantities, and the order it produced would look exactly as authoritative as a
 * real one. Places the reader also named are **linked**, never merged and never dropped.
 *
 * **No backend change.** Four reads that already existed, through the wholesale `/api` proxy, and
 * the two per-region ones are the same two-doors-onto-one-act pairing as `shape`.
 *
 * **The `await` on the left is the scaffold's, never the measurement's.** `[2026-08-21, spec §2]`
 * Exactly one read is awaited — {@link readAnchorSources}, because the plan it feeds decides *which
 * read runs at all*, and whether there is an index, a refusal or a subject to measure. The
 * measurements stream, so the page can say **which place it is measuring** before any measurement
 * arrives.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §4 (Beat C)
 * @see internal/superpowers/specs/2026-08-21-the-rendering-approach-design.md §3, §5
 */
export const load: PageServerLoad = async ({ locals, params, url }): Promise<AnalysisViewData> => {
	const token = locals.accessToken!;
	const address = parseGraphAddress(url);

	const [contexts, cogmaps] = await readAnchorSources(token);
	const readable = readableAnchors({ contexts, cogmaps });

	// **Nothing here is read and nothing here waits.** The index and both refusals are decided from
	// the address and from what the reader can see, so they render with the page chrome — a refusal
	// is the answer, not a delay. The three measured fields are already-resolved promises rather
	// than nulls: no read ran, which is a fact about the BRANCH, and an outer null would be a fourth
	// state on fields that already carry their own.
	const base = {
		owner: params.owner,
		place: null,
		alsoNamed: [],
		choices: readable.map((a) => describeAnchor(a, cogmaps)),
		refusal: null as AnalysisRefusal | null,
		regions: Promise.resolve([]),
		metricsAvailable: Promise.resolve(true),
		map: Promise.resolve(null),
	} satisfies AnalysisViewData;

	// No place named: the index. `entry-does-not-presume-organization` reaches this door too — a
	// reader arriving with no address is offered what they can read, not refused for not knowing
	// the grammar. An empty list is a different statement and gets its own refusal.
	if (address.anchors.length === 0) {
		return readable.length === 0 ? { ...base, refusal: { kind: 'nothing-to-analyse' } } : base;
	}

	// A named place that no longer resolves is a REFUSAL. Falling back to "measure everything you
	// can read" would answer a question nobody asked, about a place that is not the one in the
	// link — the same silent widening the graph door refuses one route over.
	const resolution = resolveAnchors(readable, address.anchors);
	if (resolution.entry === 'none-resolved') {
		return { ...base, refusal: { kind: 'no-place-resolved', named: resolution.named } };
	}

	const [subject, ...rest] = resolution.anchors;

	// **One read, one arriving region.** The whole measured payload comes from `readAnchorAnalysis`,
	// so the groupings table, its unknown-metrics caption and the map-level section are three views
	// of a single read — derived from ONE `bounded(...)` rather than started three times. Three
	// arriving markers for one read would tell the reader those regions could disagree about whether
	// it answered, and they cannot.
	const measured = bounded(readAnchorAnalysis(token, subject), 'measurements');
	// Joined once rather than in each of the two fields below: the join is the same arithmetic over
	// 907 groupings, and running it twice invites the two to be given different inputs later.
	const analysed = derive(measured, ({ shape, metrics }) => analyseShape(shape, metrics));

	return {
		...base,
		place: describeAnchor(subject, cogmaps),
		alsoNamed: rest.map((a) => describeAnchor(a, cogmaps)),
		regions: derive(analysed, (a) => a.regions),
		metricsAvailable: derive(analysed, (a) => a.metricsAvailable),
		// The inner null still covers the two situations the page tells apart: a context, which
		// genuinely has neither a charter nor a regulation set, and a map whose analytics read was
		// declined. The read FAILING is the third state, and it is a rejection — never spelled as
		// either of those absences (spec §5.1).
		map: derive(measured, ({ analytics, telos }) =>
			analytics
				? {
						telos: { id: analytics.telos_resource_id, title: telos?.title ?? null },
						staleness: analytics.staleness,
						regulation: analytics.regulation,
					}
				: null,
		),
	};
};
