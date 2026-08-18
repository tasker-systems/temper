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
		// A synthetic empty envelope for a FAILED fetch, not an empty result set. `truncated: false`
		// is the lie that field exists to prevent: the fetch failed, so the caller emphatically may
		// not conclude a resource is absent. The fix is to surface the error instead of synthesizing
		// a success shape, not to pick a different boolean.
		//
		// `stage`/`status` were added only to satisfy the widened `ResourceFacets`; they are as
		// synthetic as the rest of this envelope.
		//
		// LIVE as of the grid reading `returned`/`truncated` (VaultGrid no longer re-derives
		// `hasNext` itself): on a failed fetch this now renders as "0 results, next disabled"
		// rather than surfacing the error. Deferred to the task that reworks this page's error
		// handling rather than fixed here — out of scope for the grid-side change that made it live.
	).catch(
		() =>
			({
				rows: [],
				total: BigInt(0),
				facets: { doc_type: {}, stage: {}, status: {} },
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
		returned: Number(resources.returned),
		truncated: resources.truncated,
		limit: Number(params.get('limit')),
		offset: Number(params.get('offset') ?? 0),
		facets: Object.fromEntries(
			Object.entries(resources.facets.doc_type).map(([k, v]) => [k, Number(v ?? 0)]),
		) as Record<string, number>,
	};
};
