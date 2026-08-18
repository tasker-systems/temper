import { describe, expect, it } from 'vitest';
import type { ResourceListResponse } from '$lib/types';
import { searchFailureMessage, toVaultFacets, toVaultList } from './vault-list';

const response = (over: Partial<ResourceListResponse> = {}): ResourceListResponse =>
	({
		rows: [],
		total: BigInt(42),
		returned: BigInt(2),
		truncated: true,
		limit: 50,
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
		const list = toVaultList(response(), new URLSearchParams('limit=50'));
		expect(list.total).toBe(42);
		expect(list.returned).toBe(2);
		expect(list.truncated).toBe(true);
	});

	it('takes limit and offset from the query actually sent, defaulting offset to 0', () => {
		expect(toVaultList(response(), new URLSearchParams('limit=25&offset=75')).offset).toBe(75);
		expect(toVaultList(response(), new URLSearchParams('limit=25')).offset).toBe(0);
		expect(toVaultList(response(), new URLSearchParams('limit=25')).limit).toBe(25);
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
