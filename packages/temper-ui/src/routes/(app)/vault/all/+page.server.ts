import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';

export const load: PageServerLoad = async ({ locals, url }) => {
	const params = new URLSearchParams(url.searchParams);
	const resources = await apiGet<ResourceListResponse>(
		`/api/resources${params.toString() ? `?${params}` : ''}`,
		locals.accessToken!
	);

	return {
		rows: resources.rows,
		total: Number(resources.total),
		facets: Object.fromEntries(
			Object.entries(resources.facets.doc_type).map(([k, v]) => [k, Number(v ?? 0)])
		) as Record<string, number>
	};
};
