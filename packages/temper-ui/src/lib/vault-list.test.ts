import { describe, expect, it } from 'vitest';
import type { ResourceListResponse } from '$lib/types';
import { pageState, searchFailureMessage, toVaultFacets, toVaultList } from './vault-list';

const response = (over: Partial<ResourceListResponse> = {}): ResourceListResponse =>
	({
		rows: [],
		total: BigInt(42),
		returned: BigInt(2),
		truncated: true,
		limit: BigInt(50),
		offset: BigInt(0),
		facets: { doc_type: {}, stage: {}, status: {} },
		...over,
	}) as ResourceListResponse;

describe('toVaultFacets', () => {
	it('converts all three histograms, not just doc_type', () => {
		const facets = toVaultFacets({
			doc_type: { task: BigInt(3) },
			stage: { 'in-progress': BigInt(2) },
			status: { active: BigInt(1) },
		});
		expect(facets).toEqual({
			doc_type: { task: 3 },
			stage: { 'in-progress': 2 },
			status: { active: 1 },
		});
	});

	it('reads a missing count as zero rather than NaN', () => {
		expect(toVaultFacets({ doc_type: { task: undefined }, stage: {}, status: {} })).toEqual({
			doc_type: { task: 0 },
			stage: {},
			status: {},
		});
	});

	it('keeps an empty histogram empty', () => {
		expect(toVaultFacets({ doc_type: {}, stage: {}, status: {} })).toEqual({
			doc_type: {},
			stage: {},
			status: {},
		});
	});
});

describe('toVaultList', () => {
	it('carries returned and truncated through as the server reported them', () => {
		const list = toVaultList(response());
		expect(list.total).toBe(42);
		expect(list.returned).toBe(2);
		expect(list.truncated).toBe(true);
	});

	// The whole point of FIX 1: the server normalizes before it cuts the page and echoes what
	// it applied. Deriving limit/offset from the request URL instead is the second derivation
	// `substrate_read.rs:70-74` warns about, and it is the one that is not the truth.
	it('takes limit and offset from the envelope, not from the request', () => {
		const list = toVaultList(response({ limit: BigInt(25), offset: BigInt(75) }));
		expect(list.limit).toBe(25);
		expect(list.offset).toBe(75);
	});

	it('reports the floored offset the server applied, not the negative one asked for', () => {
		// `?offset=-10` — the door floors it at 0 and says so. Believing the URL rendered
		// `-9–40 of 100` and a Next that jumped to offset 40, skipping rows 40–49.
		expect(toVaultList(response({ offset: BigInt(0) })).offset).toBe(0);
	});

	it('keeps an uncapped limit as null rather than coercing it to a number', () => {
		// `Number(null)` is 0, and "page size 0" is a different claim from "no page size".
		expect(toVaultList(response({ limit: null })).limit).toBeNull();
	});
});

describe('pageState', () => {
	const page = (over: Partial<Parameters<typeof pageState>[0]> = {}) =>
		pageState({ total: 100, returned: 50, truncated: true, limit: 50, offset: 0, ...over });

	it('numbers the visible range from the applied offset', () => {
		const s = page({ offset: 50, returned: 50, truncated: false });
		expect(s.rangeStart).toBe(51);
		expect(s.rangeEnd).toBe(100);
	});

	it('reports an empty page as a zero range rather than a phantom first row', () => {
		const s = page({ returned: 0, truncated: false });
		expect(s.rangeStart).toBe(0);
		expect(s.rangeEnd).toBe(0);
	});

	it('counts pages from the applied limit', () => {
		expect(page().currentPage).toBe(1);
		expect(page().totalPages).toBe(2);
		expect(page({ offset: 50 }).currentPage).toBe(2);
	});

	// The floored-offset repro, end to end: with offset 0 the walk starts at row 1 and Next
	// goes to 50 — not the 40 the URL's `-10` would have produced.
	it('walks forward by the applied limit from the applied offset', () => {
		expect(page().nextOffset).toBe(50);
		expect(page({ offset: 50 }).prevOffset).toBe(0);
	});

	it('never offers a negative previous offset', () => {
		expect(page({ offset: 20 }).prevOffset).toBe(0);
	});

	it('takes hasNext from truncated, the answer the server itself computed', () => {
		expect(page({ truncated: false }).hasNext).toBe(false);
		expect(page({ total: 100, offset: 50, returned: 50, truncated: false }).hasPrev).toBe(true);
	});

	it('treats an uncapped page as exactly one page', () => {
		const s = page({ limit: null, returned: 100, truncated: false });
		expect(s.currentPage).toBe(1);
		expect(s.totalPages).toBe(1);
		expect(s.paged).toBe(false);
	});

	// `Math.floor(0/0)` is NaN and `Math.ceil(100/0)` is Infinity — a page counter that is
	// not a number is worse than no page counter.
	it('renders no NaN or Infinity counter for a zero limit', () => {
		const s = page({ limit: 0, returned: 0, truncated: true });
		expect(Number.isFinite(s.currentPage)).toBe(true);
		expect(Number.isFinite(s.totalPages)).toBe(true);
		expect(s.paged).toBe(false);
	});

	// An offset past the end is a real place to be; "3/1" would be a counter that cannot be true.
	it('never shows a current page beyond the total page count', () => {
		const s = page({ total: 10, offset: 100, returned: 0, truncated: false });
		expect(s.totalPages).toBeGreaterThanOrEqual(s.currentPage);
		expect(s.hasPrev).toBe(true);
		expect(s.paged).toBe(true);
	});
});

describe('searchFailureMessage', () => {
	// The point of every arm: it must read as "the search did not happen", never as a result.
	// "no results" / "not found" / "0" would be the failure this function exists to prevent.
	const arms = [null, 401, 403, 404, 429, 500, 503];

	it('never reads as an empty result set', () => {
		for (const status of arms) {
			const message = searchFailureMessage(status);
			expect(message).toContain('could not be performed');
			expect(message).not.toMatch(/no results|not found|\b0\b/i);
		}
	});

	it('names an unreachable vault when there was no response at all', () => {
		expect(searchFailureMessage(null)).toContain('could not be reached');
		expect(searchFailureMessage(null)).not.toContain('HTTP');
	});

	it('distinguishes authorization, rejection, and service failure', () => {
		expect(searchFailureMessage(403)).toContain('not authorized');
		expect(searchFailureMessage(400)).toContain('rejected the request (HTTP 400)');
		expect(searchFailureMessage(503)).toContain('vault service failed (HTTP 503)');
	});
});
