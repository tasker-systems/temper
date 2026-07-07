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
	selectedElement
} from '$lib/graph/atlas/nav';
import type { EdgeKind } from '$lib/types/generated/graph';
import { ApiError } from '$lib/server/api';
import {
	readAtlasHome,
	readCogmapNeighborhood,
	readCogmapPanorama,
	readRegionSlice,
	readResourceRow,
	readTrail
} from '$lib/server/graph-reads';
import type { TerritorySlice } from '$lib/types/generated/graph_territory';

const NEIGHBORHOOD_DEPTH = 2;

const isNotFound = (e: unknown): boolean => e instanceof ApiError && e.status === 404;

/**
 * Read a focused territory's slice, degrading gracefully when it has been
 * re-materialized out from under the URL. Region ids are ephemeral (the steward
 * re-sweeps cogmap clusters), so a bookmarked / back-navigated / long-open
 * territory URL can 404. On 404 we redirect to the current scope's panorama
 * rather than 500 the whole page; a genuine 5xx still surfaces.
 */
async function sliceOrPanorama(token: string, regionId: string, url: URL): Promise<TerritorySlice> {
	try {
		return await readRegionSlice(token, regionId);
	} catch (e) {
		if (isNotFound(e)) throw redirect(303, buildPanoramaUrl(url));
		throw e;
	}
}

/**
 * Resolve the breadcrumb label for a path territory. At Tier 1 the primary slice
 * is already loaded and reused; at Tier 2 (a node leaf under a territory) we fetch
 * the ancestor's slice just for its label. That ancestor can be a stale region id —
 * but the node view itself is still valid (node ids are stable), so on 404 we keep
 * the view and let the crumb fall back to its generic label rather than 500 or bounce
 * the user off a working page.
 */
async function crumbTerritoryLabel(
	token: string,
	segId: string,
	slice: TerritorySlice | null
): Promise<{ id: string; label: string | null }> {
	if (slice && slice.region_id === segId) return { id: segId, label: slice.label };
	try {
		return { id: segId, label: (await readRegionSlice(token, segId)).label };
	} catch (e) {
		if (isNotFound(e)) return { id: segId, label: null };
		throw e;
	}
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
		const [territories, slice, home, neighborhood] = await Promise.all([
			tier === 0 ? readCogmapPanorama(token, cogmapId) : Promise.resolve(null),
			tier === 1 && focus.kind === 'territory' ? sliceOrPanorama(token, focus.id, url) : Promise.resolve(null),
			readAtlasHome(token),
			tier === 2 && focus.kind === 'node'
				? readCogmapNeighborhood(token, cogmapId, {
						seeds: [focus.id],
						depth: NEIGHBORHOOD_DEPTH,
						edge_kinds: [] as EdgeKind[]
					})
				: Promise.resolve(null)
		]);
		const cogmapName = home.research.find((c) => c.id === cogmapId)?.name ?? 'Cognitive map';
		// Name the territory hop in the crumb (reuses the already-loaded slice at Tier 1;
		// fetches the path territory's slice for its label at Tier 2, tolerating a stale
		// region id).
		const crumbTerritory = territorySeg
			? await crumbTerritoryLabel(token, territorySeg.id, slice)
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
			slice,
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
