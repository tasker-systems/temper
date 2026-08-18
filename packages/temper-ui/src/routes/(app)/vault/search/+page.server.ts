import { ApiError, apiGet } from '$lib/server/api';
import type { ResourceListResponse } from '$lib/types';
import { doorParams } from '$lib/vault-filters';
import { searchFailureMessage, toVaultList, type VaultList } from '$lib/vault-list';
import type { PageServerLoad } from './$types';

const DEFAULT_LIMIT = 50;

export const load: PageServerLoad = async ({ url, locals }) => {
	const q = url.searchParams.get('q') ?? '';
	// `doorParams`: an empty/whitespace-only filter param is dropped rather than forwarded, so
	// the door and `parseFilters` agree about what is narrowing the set.
	const params = doorParams(url.searchParams);
	if (!params.has('limit')) params.set('limit', String(DEFAULT_LIMIT));

	// A failed read reaches the page AS A FAILURE. This load used to answer a failed fetch with
	// a synthetic success envelope — empty rows, `total: 0`, `truncated: false` — which said
	// "your search ran and matched nothing" about a search that never ran. `truncated: false`
	// is precisely the claim the caller may not make from a failed read, and the grid now reads
	// that field, so the lie was live rather than latent.
	//
	// `error()` is the repo's idiom for a load that cannot produce its page (`vault/r/[ident]`),
	// but it takes the whole route to the error boundary and the filter bar goes with it, so a
	// user cannot adjust the query that failed. A search page can still be a useful page without
	// its results, so the failure travels in `data` and `VaultBrowser` renders an error state in
	// place of the grid. `list: null` means there is no envelope to misread.
	let list: VaultList | null = null;
	let loadError: string | null = null;
	try {
		const resources = await apiGet<ResourceListResponse>(
			`/api/resources?${params}`,
			locals.accessToken!,
		);
		list = toVaultList(resources);
	} catch (err) {
		loadError = searchFailureMessage(err instanceof ApiError ? err.status : null);
	}

	return { query: q, list, loadError };
};
