// +page.server.ts
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { GraphFilters } from '$lib/graph/atlas/nav';
import {
	buildPanoramaUrl,
	deriveTier,
	parseCogmap,
	parseContextScope,
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
	readContextComposition,
	readContextPanorama,
	readRegionComposition,
	readResourceRow,
	readTrail,
	type CompositionTarget
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

/**
 * Beat E — read a context drill's COMPOSITION, degrading to the panorama on 404 the same
 * way the region drill does. A container is a stable resource uuid, but a residual bucket
 * can vanish (well-edged data absorbs it) and a container can be deleted, so a bookmarked
 * or back-navigated drill URL can 404 — land the user on the current panorama rather than
 * 500 the page. A genuine 5xx still surfaces.
 */
async function contextCompositionOrPanorama(
	token: string,
	ref: string,
	target: CompositionTarget,
	url: URL
): Promise<AtlasSubgraph> {
	try {
		return await readContextComposition(token, ref, target);
	} catch (e) {
		if (isNotFound(e)) throw redirect(303, buildPanoramaUrl(url));
		throw e;
	}
}

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;
	const cogmapId = parseCogmap(url);
	const contextSlug = parseContextScope(url);
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

	// Beat E — the context door (builder axis). Mutually exclusive with the cogmap door,
	// so it precedes it. Tier 0 is the container panorama + residual tray; a container /
	// bucket focus drills into the composition (the canvas inverts the radial via
	// coreHome). Node detail inside a composition opens via `?sel=node:` (orthogonal), so
	// the focus path stays at the container/bucket level and the selection rail is shared
	// with the cogmap branch.
	if (contextSlug) {
		const contextRef = `${params.owner}/${contextSlug}`;
		const drillTarget: CompositionTarget | null =
			focus.kind === 'container' || focus.kind === 'bucket' ? focus : null;

		const [panorama, neighborhood] = await Promise.all([
			tier === 0 ? readContextPanorama(token, contextRef) : Promise.resolve(null),
			drillTarget
				? contextCompositionOrPanorama(token, contextRef, drillTarget, url)
				: Promise.resolve(null)
		]);

		// R5 trail + resource row are profile-scoped, not scope-gated (same as the cogmap
		// branch): an explicit `?sel` node/edge, else the focused node.
		const selection = selectedElement(focus, url);
		const trail =
			selection.kind === 'edge'
				? await readTrail(token, 'edge', selection.id)
				: selection.kind === 'node'
					? await readTrail(token, 'node', selection.id)
					: null;
		const resourceRow =
			selection.kind === 'node' ? await readResourceRow(token, selection.id) : null;

		return {
			owner: params.owner,
			cogmapId: null,
			cogmapName: null,
			contextSlug,
			panorama,
			tier,
			focus,
			home: null,
			territories: null,
			neighborhood,
			selection,
			trail,
			resourceRow,
			filters: defaultFilters,
			focusPath,
			crumbTerritory: null,
			scopeFilter
		};
	}

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
		// Union size comes from the territory SEGMENT, not the leaf focus — at tier 2
		// (a node under a territory) the leaf is the node, so counting `focus` would
		// lose the "N regions" label.
		const crumbTerritory = territorySeg
			? crumbTerritoryFor(territorySeg.id, territoryIds(territorySeg).length)
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
			contextSlug: null,
			panorama: null,
			tier,
			focus,
			home: null,
			territories,
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
		contextSlug: null,
		panorama: null,
		tier,
		focus,
		home,
		territories: null,
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
