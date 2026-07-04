// +page.server.ts
import type { PageServerLoad } from './$types';
import { deriveTier, parseFilters, parseFocus, parseTeam } from '$lib/graph/atlas/nav';
import {
	listTeams,
	readNeighborhood,
	readRegionSlice,
	readTeamScope,
	readTerritories
} from '$lib/server/graph-reads';

const NEIGHBORHOOD_DEPTH = 2;

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;
	const teamId = parseTeam(url.searchParams);
	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);

	// No team scoped → the canonical @me membership home (you → teams).
	if (!teamId) {
		const teams = await listTeams(token);
		return {
			owner: params.owner,
			teamId: null,
			scope: null,
			tier,
			focus,
			teams,
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
		scope,
		tier,
		focus,
		teams: null,
		territories,
		slice,
		neighborhood
	};
};
