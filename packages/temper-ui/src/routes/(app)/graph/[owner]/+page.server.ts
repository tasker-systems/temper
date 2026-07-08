// +page.server.ts
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { GraphFilters } from '$lib/graph/atlas/nav';
import {
	buildPanoramaUrl,
	deriveTier,
	parseCogmap,
	parseFocus,
	parseFocusPath,
	parseScopeFilter,
	selectedElement,
	territoryIds
} from '$lib/graph/atlas/nav';
import type { EdgeKind } from '$lib/types/generated/graph';
import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import { ApiError } from '$lib/server/api';
import {
	readAtlasHome,
	readCogmapNeighborhood,
	readCogmapPanorama,
	readRegionComposition,
	readResourceRow,
	readTrail
} from '$lib/server/graph-reads';

const NEIGHBORHOOD_DEPTH = 2;

const isNotFound = (e: unknown): boolean => e instanceof ApiError && e.status === 404;

/**
 * Beat D — read a focused territory's COMPOSITION drill (facets + the
 * context-resources they were derived_from), degrading gracefully when the region
 * has been re-materialized out from under the URL. Region ids are ephemeral (the
 * steward re-sweeps cogmap clusters), so a bookmarked / back-navigated / long-open
 * territory URL can 404. On 404 we redirect to the current scope's panorama rather
 * than 500 the whole page; a genuine 5xx still surfaces. Drives one region or a
 * shift-selected union (the `~`-split id list).
 */
async function compositionOrPanorama(
	token: string,
	ids: string[],
	url: URL
): Promise<AtlasSubgraph> {
	try {
		return await readRegionComposition(token, ids, 1);
	} catch (e) {
		if (isNotFound(e)) throw redirect(303, buildPanoramaUrl(url));
		throw e;
	}
}

/**
 * Read-free crumb for a territory hop. The composition read carries no region
 * label (region ids are ephemeral anyway), so a single region shows the generic
 * label (null → crumbModel renders "Region") and a union shows its count.
 */
function crumbTerritoryFor(segId: string, unionSize: number): { id: string; label: string | null } {
	return { id: segId, label: unionSize > 1 ? `${unionSize} regions` : null };
}

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;
	const cogmapId = parseCogmap(url);
	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);
	const focusPath = parseFocusPath(url);
	const territorySeg = focusPath.find((f) => f.kind === 'territory') ?? null;
	// Beat C: the committed Home `?scope` narrow. Only meaningful on the Home branch
	// below (crumbModel suppresses the segment once a cogmap is set), but computed
	// once here so both branches carry it for AtlasViewData shape uniformity.
	const scopeFilter = parseScopeFilter(url);

	// Default filter bag for both branches below (no team scope anymore, so no real
	// edge-kind/lens filtering happens yet — deferred, see Task 8 self-review notes).
	const defaultFilters: GraphFilters = { lensId: null, edgeKinds: [], docTypes: [] };

	// A cogmap door reads the cogmap's own panorama directly (spec Task 5). The team
	// scope has been retired entirely (Beat C) — Home already surfaces every reachable
	// context, so anything that isn't a cogmap door falls through to Home below.
	if (cogmapId) {
		// Resolve the cogmap's display name for the breadcrumb (B2). The panorama read
		// carries no self-name, so look it up in the membership home (the same list the
		// door was entered from). Falls back to a generic label if not visible there
		// (e.g. a public/system cogmap outside your membership — refined in Beat 2).
		// The home read is independent of the tier read, so run them concurrently.
		// Beat D: a territory focus (one region or a `~`-union) loads the composition
		// force-graph into `neighborhood`; a node focus loads the cogmap neighborhood.
		// The R3 members-hull `slice` is retired — territory drill is now the two-axis
		// composition (facets + the context-resources they were derived_from).
		const [territories, home, neighborhood] = await Promise.all([
			tier === 0 ? readCogmapPanorama(token, cogmapId) : Promise.resolve(null),
			readAtlasHome(token),
			tier === 1 && focus.kind === 'territory'
				? compositionOrPanorama(token, territoryIds(focus), url)
				: tier === 2 && focus.kind === 'node'
					? readCogmapNeighborhood(token, cogmapId, {
							seeds: [focus.id],
							depth: NEIGHBORHOOD_DEPTH,
							edge_kinds: [] as EdgeKind[]
						})
					: Promise.resolve(null)
		]);
		const cogmapName = home.research.find((c) => c.id === cogmapId)?.name ?? 'Cognitive map';
		const crumbTerritory = territorySeg
			? crumbTerritoryFor(territorySeg.id, territoryIds(focus).length)
			: null;

		// R5 trail + resource row are profile-scoped, not scope-gated.
		const selection = selectedElement(focus, url);
		const trail =
			selection.kind === 'edge'
				? await readTrail(token, 'edge', selection.id)
				: selection.kind === 'node'
					? await readTrail(token, 'node', selection.id)
					: null;
		const resourceRow = selection.kind === 'node' ? await readResourceRow(token, selection.id) : null;

		return {
			owner: params.owner,
			cogmapId,
			cogmapName,
			tier,
			focus,
			home: null,
			territories,
			slice: null,
			neighborhood,
			selection,
			trail,
			resourceRow,
			filters: defaultFilters,
			focusPath,
			crumbTerritory,
			scopeFilter
		};
	}

	// Not a cogmap door → the canonical @me membership home (you → teams → cogmaps).
	const home = await readAtlasHome(token);
	return {
		owner: params.owner,
		cogmapId: null,
		cogmapName: null,
		tier,
		focus,
		home,
		territories: null,
		slice: null,
		neighborhood: null,
		selection: { kind: 'none' as const },
		trail: null,
		resourceRow: null,
		filters: defaultFilters,
		focusPath,
		crumbTerritory: null,
		scopeFilter
	};
};
