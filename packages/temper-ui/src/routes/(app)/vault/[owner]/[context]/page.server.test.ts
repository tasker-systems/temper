import { isHttpError } from '@sveltejs/kit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();

/** The SAME class the load does `instanceof` against, so a rejection minted here is recognized. */
class ApiErrorStub extends Error {
	status: number;
	constructor(status: number) {
		super(`HTTP ${status}`);
		this.status = status;
	}
}

vi.mock('$lib/server/api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
	ApiError: ApiErrorStub,
}));

const { load } = await import('./+page.server');
const { PLACE_NOT_AVAILABLE } = await import('$lib/vault-list');

const run = () =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: { accessToken: 'tok' },
		params: { owner: '+acme', context: 'platform' },
		url: new URL('/vault/+acme/platform', 'http://ui.invalid'),
		parent: async () => ({ contexts: [] }),
	});

beforeEach(() => {
	apiGet.mockReset();
});

/**
 * A place the reader cannot read, from the reader's side: told it is not available to them, never
 * whether it exists, and never that temper failed.
 */
describe('a context the reader cannot read', () => {
	it.each([404, 403])('is a 404 saying the place is not available to them (API %i)', async (s) => {
		apiGet.mockImplementation(() => Promise.reject(new ApiErrorStub(s)));
		const thrown = await run().catch((e: unknown) => e);
		expect(isHttpError(thrown)).toBe(true);
		expect(thrown).toMatchObject({ status: 404, body: { message: PLACE_NOT_AVAILABLE } });
	});

	it('does not pass off a failed read as one: anything else still fails', async () => {
		const failure = new ApiErrorStub(503);
		apiGet.mockImplementation(() => Promise.reject(failure));
		await expect(run()).rejects.toBe(failure);
	});
});
