// +page.server.ts
import type { PageServerLoad } from './$types';
import { deriveTier, parseCogmap, parseFilters, parseFocus, parseTeam } from '$lib/graph/atlas/nav';
import {
	readAtlasHome,
	readCogmapPanorama,
	readNeighborhood,
	readRegionSlice,
	readTeamScope,
	readTerritories
} from '$lib/server/graph-reads';

const NEIGHBORHOOD_DEPTH = 2;

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;
	const teamId = parseTeam(url.searchParams);
	const cogmapId = parseCogmap(url);
	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);

	// A cogmap door is a distinct scope from a team (spec Task 5): it reads the
	// cogmap's own panorama directly, no team-scope fetch involved. Checked before
	// the `!teamId` home branch, since `buildCogmapUrl` always clears `team`.
	if (cogmapId) {
		const territories = tier === 0 ? await readCogmapPanorama(token, cogmapId) : null;
		const slice = tier === 1 && focus.kind === 'territory' ? await readRegionSlice(token, focus.id) : null;
		return {
			owner: params.owner,
			teamId: null,
			cogmapId,
			scope: null,
			tier,
			focus,
			teams: null,
			cogmaps: null,
			territories,
			slice,
			neighborhood: null
		};
	}

	// No team scoped → the canonical @me membership home (you → teams → cogmaps).
	if (!teamId) {
		const home = await readAtlasHome(token);
		return {
			owner: params.owner,
			teamId: null,
			cogmapId: null,
			scope: null,
			tier,
			focus,
			teams: home.teams,
			cogmaps: home.cogmaps,
			territories: null,
			slice: null,
			neighborhood: null
		};
	}

	const filters = parseFilters(url.searchParams);
	const scope = await readTeamScope(token, teamId);

	const territories = tier === 0 ? await readTerritories(token, teamId, filters.lensId) : null;
	const slice = tier === 1 && focus.kind === 'territory' ? await readRegionSlice(token, focus.id) : null;
	const neighborhood =
		tier === 2 && focus.kind === 'node'
			? await readNeighborhood(token, teamId, { seeds: [focus.id], depth: NEIGHBORHOOD_DEPTH, edge_kinds: [] })
			: null;

	return {
		owner: params.owner,
		teamId,
		cogmapId: null,
		scope,
		tier,
		focus,
		teams: null,
		cogmaps: null,
		territories,
		slice,
		neighborhood
	};
};
