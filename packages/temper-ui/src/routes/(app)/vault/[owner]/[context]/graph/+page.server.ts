import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/server/api';
import type { SubgraphResponse } from '$lib/types/generated/graph';

export const load: PageServerLoad = async ({ locals, params }) => {
	const query = new URLSearchParams({
		owner: params.owner,
		context: params.context
	});
	const subgraph = await apiGet<SubgraphResponse>(
		`/api/graph/subgraph?${query}`,
		locals.accessToken!
	);

	return {
		owner: params.owner,
		context: params.context,
		subgraph
	};
};
