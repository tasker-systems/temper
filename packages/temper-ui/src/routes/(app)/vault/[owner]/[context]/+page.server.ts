import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ locals, url, params: routeParams }) => {
	const params = new URLSearchParams(url.searchParams);
	params.set('owner', routeParams.owner);
	params.set('context_name', routeParams.context);
	if (!params.has('limit')) params.set('limit', String(DEFAULT_LIMIT));
	const resources = await apiGet<ResourceListResponse>(
		`/api/resources?${params}`,
		locals.accessToken!
	);

	return {
		owner: routeParams.owner,
		context: routeParams.context,
		rows: resources.rows,
		total: Number(resources.total),
		limit: Number(params.get('limit')),
		offset: Number(params.get('offset') ?? 0),
		facets: Object.fromEntries(
			Object.entries(resources.facets.doc_type).map(([k, v]) => [k, Number(v ?? 0)])
		) as Record<string, number>
	};
};
