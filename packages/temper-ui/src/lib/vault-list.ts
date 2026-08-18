/**
 * What a vault list page needs from one `/api/resources` read — and what it needs when that
 * read FAILS.
 *
 * The three list routes (`vault/all`, `vault/[owner]/[context]`, `vault/search`) each did the
 * same wire→page conversion inline, so it lives here once instead of three times.
 *
 * `null` is the whole point of the failure half. A failed read has no rows, no total and no
 * `truncated`, so it must not be given any: a synthesized `{ rows: [], total: 0, truncated:
 * false }` envelope claims "your query ran and matched nothing", which is the opposite of what
 * happened. `VaultList | null` makes "we have a page" and "we could not read one" different
 * shapes, so no consumer can read absence out of a failure.
 */

import type { ResourceFacets, ResourceListResponse, ResourceView } from '$lib/types';

/** The three facet histograms, converted from the wire's `bigint` counts. */
export interface VaultFacets {
	doc_type: Record<string, number>;
	stage: Record<string, number>;
	status: Record<string, number>;
}

/** One successfully read page of the vault list, in the shape `VaultBrowser` mounts. */
export interface VaultList {
	rows: ResourceView[];
	total: number;
	returned: number;
	truncated: boolean;
	limit: number;
	offset: number;
	facets: VaultFacets;
}

/**
 * The wire carries each histogram as `{ [key in string]?: bigint }`; the UI wants plain
 * numbers. `?? 0` covers the optional-value type, not a real absent count.
 */
function counts(histogram: { [key in string]?: bigint }): Record<string, number> {
	return Object.fromEntries(Object.entries(histogram).map(([k, v]) => [k, Number(v ?? 0)]));
}

export function toVaultFacets(facets: ResourceFacets): VaultFacets {
	return {
		doc_type: counts(facets.doc_type),
		stage: counts(facets.stage),
		status: counts(facets.status),
	};
}

/**
 * Convert one list response into page data. `params` is the query the server actually sent,
 * so `limit`/`offset` describe the page that came back rather than what the URL asked for.
 */
export function toVaultList(response: ResourceListResponse, params: URLSearchParams): VaultList {
	return {
		rows: response.rows,
		total: Number(response.total),
		returned: Number(response.returned),
		truncated: response.truncated,
		limit: Number(params.get('limit')),
		offset: Number(params.get('offset') ?? 0),
		facets: toVaultFacets(response.facets),
	};
}

/**
 * The user-facing sentence for a search read that never completed. Every arm says the search
 * could not be PERFORMED — none of them may read as a result, because a failed read is
 * evidence of nothing about what the vault holds.
 *
 * `status` is the `ApiError` status when the door answered, or `null` when the fetch itself
 * failed (no response at all). Kept status-in / string-out so it is a pure function the
 * `.test.ts` beside it can pin without a server environment — the load function does the
 * `err instanceof ApiError` narrowing.
 */
export function searchFailureMessage(status: number | null): string {
	if (status === null) {
		return 'The search could not be performed — the vault could not be reached.';
	}
	if (status === 401 || status === 403) {
		return 'The search could not be performed — this session is not authorized to search the vault.';
	}
	if (status >= 500) {
		return `The search could not be performed — the vault service failed (HTTP ${status}).`;
	}
	return `The search could not be performed — the vault rejected the request (HTTP ${status}).`;
}
