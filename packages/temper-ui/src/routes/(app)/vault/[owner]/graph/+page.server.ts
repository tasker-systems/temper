// +page.server.ts
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { deriveTier, parseFilters, parseFocus, parseTeam } from '$lib/graph/atlas/nav';
import { listTeams, readTeamScope, readTerritories } from '$lib/server/graph-reads';

export const load: PageServerLoad = async ({ locals, params, url }) => {
	const token = locals.accessToken!;

	// Resolve scope team: ?team wins; else the profile's first accessible team.
	let teamId = parseTeam(url.searchParams);
	if (!teamId) {
		const teams = await listTeams(token);
		if (teams.length === 0) throw error(404, 'No accessible teams to graph.');
		teamId = teams[0].id;
	}

	const focus = parseFocus(url.searchParams);
	const tier = deriveTier(focus);
	const filters = parseFilters(url.searchParams);

	const scope = await readTeamScope(token, teamId);

	// C1 renders Tier 0 fully; Tier 1/2 payloads land in C2. Only fetch what we draw.
	const territories = tier === 0 ? await readTerritories(token, teamId, filters.lensId) : null;

	return { owner: params.owner, teamId, scope, tier, focus, territories };
};
