import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';
import type { PageServerLoad } from './$types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ url, locals }) => {
	const q = url.searchParams.get('q') ?? '';
	const params = new URLSearchParams(url.searchParams);
	if (!params.has('limit')) params.set('limit', String(DEFAULT_LIMIT));

	const resources = await apiGet<ResourceListResponse>(
		`/api/resources?${params}`,
		locals.accessToken!,
		// A synthetic empty envelope for a FAILED fetch, not an empty result set. Nothing on this
		// page reads `returned`/`truncated` — limit and offset come from `params` below — so the
		// paging state here is inert. Were a consumer to start reading it, `truncated: false` would
		// be the lie that field exists to prevent: the fetch failed, so the caller emphatically may
		// not conclude a resource is absent. The fix then is to surface the error instead of
		// synthesizing a success shape, not to pick a different boolean.
	).catch(
		() =>
			({
				rows: [],
				total: BigInt(0),
				facets: { doc_type: {} },
				returned: BigInt(0),
				truncated: false,
				limit: null,
				offset: BigInt(0),
			}) as ResourceListResponse,
	);

	return {
		query: q,
		rows: resources.rows,
		total: Number(resources.total),
		limit: Number(params.get('limit')),
		offset: Number(params.get('offset') ?? 0),
		facets: Object.fromEntries(
			Object.entries(resources.facets.doc_type).map(([k, v]) => [k, Number(v ?? 0)]),
		) as Record<string, number>,
	};
};
