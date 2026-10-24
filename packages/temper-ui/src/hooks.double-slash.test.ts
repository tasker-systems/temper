import { describe, expect, it, vi } from 'vitest';

/**
 * A pathname opening with `//` is refused by the real `handle` hook before routing or proxying.
 *
 * GHSA-pfhg-q3qv-9q6r: on SvelteKit 2, an enhanced form on a route that matched `//host/…` derives
 * a protocol-relative action and navigates off-origin. The fix exists only in kit 3; the advisory's
 * own interim workaround is this refusal. No route here can match such a path today, so the test
 * pins the guard, not a live exploit.
 *
 * The mocks below stand in for module-load side effects, as in `hooks.security-headers.test.ts`.
 */
vi.mock('$lib/server/oidc', () => ({
	REFRESH_THRESHOLD_SECONDS: 60,
	refreshAccessToken: () => Promise.reject(new Error('not used here')),
}));
vi.mock('$lib/server/session', () => ({
	readSession: () => Promise.resolve(null),
	writeSession: () => Promise.resolve(),
	clearSession: () => {},
}));
vi.mock('$lib/server/telemetry/register', () => ({}));
vi.mock('$lib/server/telemetry/request-span', () => ({
	traceRequest: (_event: unknown, run: () => unknown) => run(),
}));

const proxyRequest = vi.fn(() => Promise.resolve(new Response('upstream')));
vi.mock('$lib/server/proxy', () => ({
	// Deliberately broad: a guard placed after the proxy short-circuit would lose to this.
	isProxiedPath: () => true,
	proxyRequest: () => proxyRequest(),
}));

import { handle } from './hooks.server';

const run = (url: string) => {
	const resolve = vi.fn(() => Promise.resolve(new Response('page')));
	const response = handle({
		event: {
			url: new URL(url),
			request: new Request(url),
			cookies: { get: () => undefined },
			locals: {},
			route: { id: null },
		},
		resolve,
	} as never) as Promise<Response>;
	return { response, resolve };
};

describe('a path opening with // is refused before anything routes it', () => {
	it('answers 404 with the baseline headers, and neither renders nor proxies', async () => {
		proxyRequest.mockClear();
		const { response, resolve } = run('https://temper.invalid//evil.example/request-access');
		const refused = await response;

		expect(refused.status).toBe(404);
		expect(refused.headers.get('x-content-type-options')).toBe('nosniff');
		expect(resolve).not.toHaveBeenCalled();
		expect(proxyRequest).not.toHaveBeenCalled();
	});

	it('leaves a doubled slash later in the path to the ordinary handling', async () => {
		proxyRequest.mockClear();
		const { response } = run('https://temper.invalid/api//v1/resources');

		expect((await response).status).toBe(200);
		expect(proxyRequest).toHaveBeenCalledOnce();
	});
});
