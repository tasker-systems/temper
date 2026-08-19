import { apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';
import { doorParams } from '$lib/vault-filters';
import { toVaultList } from '$lib/vault-list';
import type { PageServerLoad } from './$types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ locals, url, params: routeParams }) => {
	// The whole query string rides through to the door, except `context_ref`, which this route
	// pins from its own path — the browser mounts with the Context select suppressed to match.
	// `doorParams`: an empty/whitespace-only filter param is dropped rather than forwarded, so
	// the door and `parseFilters` agree about what is narrowing the set.
	const params = doorParams(url.searchParams);
	params.set('context_ref', `${routeParams.owner}/${routeParams.context}`);
	if (!params.has('limit')) params.set('limit', String(DEFAULT_LIMIT));
	const resources = await apiGet<ResourceListResponse>(
		`/api/resources?${params}`,
		locals.accessToken!,
	);

	return {
		owner: routeParams.owner,
		context: routeParams.context,
		list: toVaultList(resources),
	};
};
