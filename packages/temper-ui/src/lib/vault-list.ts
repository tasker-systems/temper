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
	/** The page size the server actually applied, or `null` for an uncapped page. */
	limit: number | null;
	/** The offset the page actually starts at — already floored at 0 by the server. */
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
 * Convert one list response into page data.
 *
 * `limit`/`offset` come from the ENVELOPE, never from the request URL. The door normalizes
 * before it cuts the page — a negative offset is floored at 0, a negative limit means
 * uncapped — and echoes what it applied precisely so callers do not have to guess
 * (`substrate_read.rs:70-74`: "two derivations of 'the effective page' would drift, and the
 * reported one would be the one that is not the truth"). Re-deriving them from the query is
 * that second derivation: `?offset=-10` over 100 rows had the grid render `-9–40 of 100`,
 * page `0/2`, and a Next button that jumped to `offset=40` — silently skipping rows 40–49.
 *
 * `limit` stays `number | null` rather than being coerced: `null` is "uncapped", which is a
 * different statement from any page size, and `Number(null)` is `0`, which is a third.
 */
export function toVaultList(response: ResourceListResponse): VaultList {
	return {
		rows: response.rows,
		total: Number(response.total),
		returned: Number(response.returned),
		truncated: response.truncated,
		limit: response.limit === null ? null : Number(response.limit),
		offset: Number(response.offset),
		facets: toVaultFacets(response.facets),
	};
}

/** What the grid's paging chrome renders, derived once from the envelope's own numbers. */
export interface PageState {
	/** 1-based index of the first row on this page; `0` when the page is empty. */
	rangeStart: number;
	/** 1-based index of the last row on this page; `0` when the page is empty. */
	rangeEnd: number;
	currentPage: number;
	totalPages: number;
	hasPrev: boolean;
	hasNext: boolean;
	/** The offset the Previous button navigates to — never negative. */
	prevOffset: number;
	/** The offset the Next button navigates to. */
	nextOffset: number;
	/** Is there more than one page to move between at all? */
	paged: boolean;
}

/**
 * Paging chrome from the page the server actually returned.
 *
 * An uncapped page (`limit === null`, i.e. `--all`) is one page by definition, and so is a
 * `limit <= 0` echo — `Math.floor(0 / 0)` is `NaN` and `Math.ceil(n / 0)` is `Infinity`, so
 * both would otherwise render as a page counter that is not a number.
 *
 * `hasNext` is `truncated`, the server's own answer to "are there matching rows beyond this
 * page" (`offset + returned < total`), rather than a page-count comparison the UI computes.
 */
export function pageState(list: {
	total: number;
	returned: number;
	truncated: boolean;
	limit: number | null;
	offset: number;
}): PageState {
	const { total, returned, truncated, limit, offset } = list;
	const capped = limit !== null && limit > 0;
	const currentPage = capped ? Math.floor(offset / limit) + 1 : 1;
	return {
		rangeStart: returned === 0 ? 0 : offset + 1,
		rangeEnd: returned === 0 ? 0 : offset + returned,
		currentPage,
		// `currentPage` is a floor, never a ceiling: an offset past the end of the filtered set
		// is a real page the user can be sitting on, and "3/1" would be a counter that cannot
		// be true. `hasPrev` is still true there, but nothing renders it: `VaultGrid` branches
		// on `rows.length === 0` BEFORE the paging chrome, so an over-offset page shows the
		// empty state and no Previous button. The way back is the "Clear filters" link (which
		// deletes `offset` along with the filters) when a filter is active, and the browser's
		// own Back otherwise.
		totalPages: capped ? Math.max(1, currentPage, Math.ceil(total / limit)) : 1,
		hasPrev: offset > 0,
		hasNext: truncated,
		prevOffset: capped ? Math.max(0, offset - limit) : 0,
		nextOffset: capped ? offset + limit : offset,
		paged: capped && (offset > 0 || truncated),
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
