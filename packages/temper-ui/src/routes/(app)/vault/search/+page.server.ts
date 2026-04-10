import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';

export const load: PageServerLoad = async ({ url, locals }) => {
	const q = url.searchParams.get('q') ?? '';
	const params = new URLSearchParams(url.searchParams);

	const resources = await apiGet<ResourceListResponse>(
		`/api/resources?${params}`,
		locals.accessToken!
	).catch(() => ({ rows: [], total: BigInt(0), facets: { doc_type: {} } } as ResourceListResponse));

	return {
		query: q,
		rows: resources.rows,
		total: Number(resources.total),
		facets: Object.fromEntries(
			Object.entries(resources.facets.doc_type).map(([k, v]) => [k, Number(v ?? 0)])
		) as Record<string, number>
	};
};
