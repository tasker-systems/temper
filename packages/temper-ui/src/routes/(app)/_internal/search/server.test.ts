import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();

// `vi.mock` over `$lib/server/*` follows the idiom established by
// `src/routes/(app)/vault/r/[ident]/page.server.test.ts` — module-scope `vi.fn()`, `vi.mock`
// forwarding to it, then a dynamic `import` of the module under test so the mock is installed
// before it is evaluated.
vi.mock('$lib/server/api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
}));

const { GET } = await import('./+server');

const run = (q: string) =>
	(GET as unknown as (e: unknown) => Promise<Response>)({
		url: new URL(`http://localhost/_internal/search?q=${encodeURIComponent(q)}`),
		locals: { accessToken: 'tok' },
	});

beforeEach(() => {
	vi.clearAllMocks();
	apiGet.mockResolvedValue({ rows: [], total: 0 });
});

describe('the search proxy', () => {
	it('hands the upstream answer through unchanged', async () => {
		apiGet.mockResolvedValue({ rows: [{ id: 'r1', title: 'Ledger design' }], total: 1 });
		const resp = await run('ledger');

		expect(resp.status).toBe(200);
		expect(await resp.json()).toEqual({ rows: [{ id: 'r1', title: 'Ledger design' }], total: 1 });
	});

	it('does not launder an upstream failure into an empty answer', async () => {
		const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
		try {
			apiGet.mockRejectedValue(new Error('503 from the api'));
			const resp = await run('ledger');

			expect(resp.status).toBe(503);
			// A failure keeps a body no client can read as a result: not rows-shaped, so a
			// caller checking the status never mistakes this for a successful empty answer.
			expect(await resp.json()).toEqual({ error: 'search unavailable' });
			expect(errSpy).toHaveBeenCalledTimes(1);
		} finally {
			errSpy.mockRestore();
		}
	});

	it('answers an empty query with an empty answer and no upstream read', async () => {
		const resp = await run('   ');

		expect(resp.status).toBe(200);
		expect(await resp.json()).toEqual({ rows: [], total: 0 });
		expect(apiGet).not.toHaveBeenCalled();
	});
});
