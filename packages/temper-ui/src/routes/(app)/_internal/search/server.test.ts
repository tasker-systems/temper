import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiPost = vi.fn();

// `vi.mock` over `$lib/server/*` follows the idiom established by
// `src/routes/(app)/vault/r/[ident]/page.server.test.ts` — module-scope `vi.fn()`, `vi.mock`
// forwarding to it, then a dynamic `import` of the module under test so the mock is installed
// before it is evaluated.
vi.mock('$lib/server/api', () => ({
	apiPost: (...a: unknown[]) => apiPost(...a),
}));

const { POST } = await import('./+server');

const run = (q: string) =>
	(POST as unknown as (e: unknown) => Promise<Response>)({
		request: new Request('http://localhost/_internal/search', {
			method: 'POST',
			body: JSON.stringify({ q }),
		}),
		locals: { accessToken: 'tok' },
	});

/** The wire shape `POST /api/search` answers with on main: both arms, each keyed, never combined. */
const armsAnswer = {
	exact: { hits: [], reason: 'ok', hint: null },
	wide: { hits: [], reason: 'no_match', hint: 'try different words', degraded: false },
	scope: { kind: 'global', size: null },
};

beforeEach(() => {
	vi.clearAllMocks();
	apiPost.mockResolvedValue(armsAnswer);
});

describe('the search proxy', () => {
	it('hands the upstream answer through unchanged', async () => {
		const answered = {
			...armsAnswer,
			wide: {
				hits: [{ resource: { id: 'r1', title: 'Ledger design' }, vec_norm: 0.9 }],
				reason: 'ok',
				hint: null,
				degraded: false,
			},
		};
		apiPost.mockResolvedValue(answered);
		const resp = await run('ledger');

		expect(resp.status).toBe(200);
		expect(await resp.json()).toEqual(answered);
	});

	it('sends the generic-client request body — query and limit, nothing else', async () => {
		await run('ledger');

		// The exact-arg form is the witness: an `arms` key, an embedding, or any other extra
		// would make this call not match. The palette's requests stay byte-identical to any
		// current client's — that is what makes the arm selection display-side only.
		expect(apiPost).toHaveBeenCalledWith('/api/search', 'tok', { query: 'ledger', limit: 10 });
	});

	it('does not launder an upstream failure into an empty answer', async () => {
		const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
		try {
			apiPost.mockRejectedValue(new Error('503 from the api'));
			const resp = await run('ledger');

			expect(resp.status).toBe(503);
			// A failure keeps a body no client can read as a result: not arms-shaped, so a
			// caller checking the status never mistakes this for a successful empty answer.
			expect(await resp.json()).toEqual({ error: 'search unavailable' });
			expect(errSpy).toHaveBeenCalledTimes(1);
		} finally {
			errSpy.mockRestore();
		}
	});

	it('declines an empty query with no upstream read', async () => {
		const resp = await run('   ');

		expect(resp.status).toBe(400);
		expect(apiPost).not.toHaveBeenCalled();
	});
});
