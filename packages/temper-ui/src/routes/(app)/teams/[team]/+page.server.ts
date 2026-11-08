import { error } from '@sveltejs/kit';
import { ApiError, apiGet } from '$lib/server/api';
import { toTeamView } from '$lib/team-view';
import type { TeamDetail } from '$lib/types';
import type { PageServerLoad } from './$types';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export const load: PageServerLoad = async ({ locals, params, parent }) => {
	// A slug names one of the reader's own teams (the layout's membership read); any other team is
	// addressed by its id. The API decides what the reader sees of it either way.
	let id: string | undefined;
	if (UUID.test(params.team)) {
		id = params.team;
	} else {
		const { teams } = await parent();
		id = teams?.find((t) => t.slug === params.team)?.id;
	}
	if (!id) throw error(404, 'Team not found');

	try {
		const detail = await apiGet<TeamDetail>(`/api/teams/${id}`, locals.accessToken!);
		return { team: toTeamView(detail) };
	} catch (err) {
		if (err instanceof ApiError && (err.status === 404 || err.status === 403)) {
			throw error(404, 'Team not found');
		}
		throw err;
	}
};
