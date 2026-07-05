// +page.server.ts
import type { PageServerLoad } from './$types';
import type { GraphFilters } from '$lib/graph/atlas/nav';
import {
	deriveTier,
	parseCogmap,
	parseFilters,
	parseFocus,
	parseFocusPath,
	parseTeam,
	selectedElement
} from '$lib/graph/atlas/nav';
import type { EdgeKind } from '$lib/types/generated/graph';
import {
	readAtlasHome,
	readCogmapPanorama,
	readNeighborhood,
	readRegionSlice,
	readResourceRow,
	readTeamScope,
	readTerritories,
	readTrail
} from '$lib/server/graph-reads';

const NEIGHBORHOOD_DEPTH = 2;

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
		const [territories, slice, home] = await Promise.all([
			tier === 0 ? readCogmapPanorama(token, cogmapId) : Promise.resolve(null),
			tier === 1 && focus.kind === 'territory' ? readRegionSlice(token, focus.id) : Promise.resolve(null),
			readAtlasHome(token)
		]);
		const cogmapName = home.cogmaps.find((c) => c.id === cogmapId)?.name ?? 'Cognitive map';
		// Name the territory hop in the crumb — mirrors the team branch below (reuses
		// the already-loaded slice at Tier 1; fetches the path territory's slice for
		// its label at Tier 2).
		const crumbTerritory = territorySeg
			? slice && slice.region_id === territorySeg.id
				? { id: territorySeg.id, label: slice.label }
				: { id: territorySeg.id, label: (await readRegionSlice(token, territorySeg.id)).label }
			: null;
		return {
			owner: params.owner,
			teamId: null,
			cogmapId,
			cogmapName,
			scope: null,
			tier,
			focus,
			teams: null,
			cogmaps: null,
			territories,
			slice,
			neighborhood: null,
			selection: { kind: 'none' as const },
			trail: null,
			resourceRow: null,
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
			teams: home.teams,
			cogmaps: home.cogmaps,
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
	const slice = tier === 1 && focus.kind === 'territory' ? await readRegionSlice(token, focus.id) : null;
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
	// gated R3 read — over-fetches members, acceptable for one label).
	const crumbTerritory = territorySeg
		? slice && slice.region_id === territorySeg.id
			? { id: territorySeg.id, label: slice.label }
			: { id: territorySeg.id, label: (await readRegionSlice(token, territorySeg.id)).label }
		: null;

	return {
		owner: params.owner,
		teamId,
		cogmapId: null,
		cogmapName: null,
		scope,
		tier,
		focus,
		teams: null,
		cogmaps: null,
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
