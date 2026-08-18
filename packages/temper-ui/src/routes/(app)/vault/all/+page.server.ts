import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';
import { doorParams } from '$lib/vault-filters';
import { toVaultList } from '$lib/vault-list';
import type { PageServerLoad } from './$types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ locals, url }) => {
	// The whole query string rides through to the door: every UI filter is a URL param and
	// `ResourceListParams` ignores what it does not know, so no param needs listing here.
	// `doorParams` drops empty/whitespace-only FILTER params first, so the door applies exactly
	// the filters `parseFilters` reports to the user — see its doc for why the parse alone is
	// not enough.
	const params = doorParams(url.searchParams);
	if (!params.has('limit')) params.set('limit', String(DEFAULT_LIMIT));
	const resources = await apiGet<ResourceListResponse>(
		`/api/resources?${params}`,
		locals.accessToken!,
	);

	return { list: toVaultList(resources) };
};
