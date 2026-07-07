// +page.server.ts
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { GraphFilters } from '$lib/graph/atlas/nav';
import {
	buildPanoramaUrl,
	deriveTier,
	parseCogmap,
	parseFilters,
	parseFocus,
	parseFocusPath,
	parseHomeLens,
	parseTeam,
	selectedElement
} from '$lib/graph/atlas/nav';
import type { EdgeKind } from '$lib/types/generated/graph';
import { ApiError } from '$lib/server/api';
import {
	readAtlasHome,
	readCogmapNeighborhood,
	readCogmapPanorama,
	readNeighborhood,
	readRegionSlice,
	readResourceRow,
	readTeamScope,
	readTerritories,
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
	const teamId = parseTeam(url.searchParams);
	const cogmapId = parseCogmap(url);
	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);
	const focusPath = parseFocusPath(url);
	const territorySeg = focusPath.find((f) => f.kind === 'territory') ?? null;

	// Default filter bag for the two branches below (no team scope, so no real
	// edge-kind/lens filtering happens yet — deferred, see Task 8 self-review notes).
	const defaultFilters: GraphFilters = { lensId: null, edgeKinds: [], docTypes: [] };

	// A cogmap door is a distinct scope from a team (spec Task 5): it reads the
	// cogmap's own panorama directly, no team-scope fetch involved. Checked before
	// the `!teamId` home branch, since `buildCogmapUrl` always clears `team`.
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
		// Name the territory hop in the crumb — mirrors the team branch below (reuses
		// the already-loaded slice at Tier 1; fetches the path territory's slice for
		// its label at Tier 2, tolerating a stale region id).
		const crumbTerritory = territorySeg
			? await crumbTerritoryLabel(token, territorySeg.id, slice)
			: null;

		// TrailRail parity: R5 trail + resource row are profile-scoped (not team-scoped),
		// so the same selection block as the team branch works inside a cogmap door.
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
			teamId: null,
			cogmapId,
			cogmapName,
			scope: null,
			tier,
			focus,
			home: null,
			homeLens: null,
			territories,
			slice,
			neighborhood,
			selection,
			trail,
			resourceRow,
			filters: defaultFilters,
			focusPath,
			crumbTerritory
		};
	}

	// No team scoped → the canonical @me membership home (you → teams → cogmaps).
	if (!teamId) {
		const home = await readAtlasHome(token);
		return {
			owner: params.owner,
			teamId: null,
			cogmapId: null,
			cogmapName: null,
			scope: null,
			tier,
			focus,
			home,
			homeLens: parseHomeLens(url),
			territories: null,
			slice: null,
			neighborhood: null,
			selection: { kind: 'none' as const },
			trail: null,
			resourceRow: null,
			filters: defaultFilters,
			focusPath,
			crumbTerritory: null
		};
	}

	const filters = parseFilters(url.searchParams);
	const scope = await readTeamScope(token, teamId);

	const territories = tier === 0 ? await readTerritories(token, teamId, filters.lensId) : null;
	const slice =
		tier === 1 && focus.kind === 'territory' ? await sliceOrPanorama(token, focus.id, url) : null;
	const neighborhood =
		tier === 2 && focus.kind === 'node'
			? await readNeighborhood(token, teamId, {
					seeds: [focus.id],
					depth: NEIGHBORHOOD_DEPTH,
					edge_kinds: filters.edgeKinds as EdgeKind[]
				})
			: null;

	const selection = selectedElement(focus, url);
	const trail =
		selection.kind === 'edge'
			? await readTrail(token, 'edge', selection.id)
			: selection.kind === 'node'
				? await readTrail(token, 'node', selection.id)
				: null;
	const resourceRow = selection.kind === 'node' ? await readResourceRow(token, selection.id) : null;

	// Name the territory hop in the crumb. Tier 1 already loaded the slice (carries
	// label); at Tier 2 fetch the path territory's slice for its label (reuses the
	// gated R3 read — over-fetches members, acceptable for one label), tolerating a
	// stale region id.
	const crumbTerritory = territorySeg ? await crumbTerritoryLabel(token, territorySeg.id, slice) : null;

	return {
		owner: params.owner,
		teamId,
		cogmapId: null,
		cogmapName: null,
		scope,
		tier,
		focus,
		home: null,
		homeLens: null,
		territories,
		slice,
		neighborhood,
		selection,
		trail,
		resourceRow,
		filters,
		focusPath,
		crumbTerritory
	};
};
