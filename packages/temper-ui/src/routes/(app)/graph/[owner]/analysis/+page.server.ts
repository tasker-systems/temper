import { analyseShape } from '$lib/graph/analysis';
import { describeAnchor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import type { AnalysisRefusal, AnalysisViewData } from '$lib/graph/view';
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
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §4 (Beat C)
 */
export const load: PageServerLoad = async ({ locals, params, url }): Promise<AnalysisViewData> => {
	const token = locals.accessToken!;
	const address = parseGraphAddress(url);

	const [contexts, cogmaps] = await readAnchorSources(token);
	const readable = readableAnchors({ contexts, cogmaps });

	const base = {
		owner: params.owner,
		place: null,
		alsoNamed: [],
		choices: readable.map((a) => describeAnchor(a, cogmaps)),
		refusal: null as AnalysisRefusal | null,
		regions: [],
		metricsAvailable: true,
		map: null,
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
	const { shape, metrics, analytics, telos } = await readAnchorAnalysis(token, subject);
	const { regions, metricsAvailable } = analyseShape(shape, metrics);

	return {
		...base,
		place: describeAnchor(subject, cogmaps),
		alsoNamed: rest.map((a) => describeAnchor(a, cogmaps)),
		regions,
		metricsAvailable,
		// Null covers two situations the page tells apart: a context, which genuinely has neither a
		// charter nor a regulation set, and a map whose analytics read was declined.
		map: analytics
			? {
					telos: { id: analytics.telos_resource_id, title: telos?.title ?? null },
					staleness: analytics.staleness,
					regulation: analytics.regulation,
				}
			: null,
	};
};
