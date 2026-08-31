import { apiGet } from '$lib/server/api';
import { bounded } from '$lib/server/bounded';
import type { ResourceListResponse } from '$lib/types';
import type { ShapeView } from '$lib/types/generated/data_artifact_shape';
import { doorParams } from '$lib/vault-filters';
import { toVaultList } from '$lib/vault-list';
import type { PageServerLoad } from './$types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ locals, url, params: routeParams, parent }) => {
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

	// Governance is context-scoped, and this route addresses the context by ref while the shapes
	// door addresses it by id. The layout's `/n` read already carries every visible context's row,
	// so resolution costs nothing — and when that read failed (`null`) or this context is not
	// among the visible rows, `null` is the honest answer: nobody resolved, so nothing is claimed.
	// The `[owner]` param matches the already-sigil'd `owner_ref` literally — that is how
	// `contextHref` builds these very URLs.
	const { contexts } = await parent();
	const context = contexts?.find(
		(c) => c.owner_ref === routeParams.owner && c.slug === routeParams.context,
	);
	const shapes = context
		? bounded(
				apiGet<ShapeView[]>(`/api/contexts/${context.id}/shapes`, locals.accessToken!),
				'governed families',
			)
		: null;

	return {
		owner: routeParams.owner,
		context: routeParams.context,
		list: toVaultList(resources),
		shapes,
	};
};
