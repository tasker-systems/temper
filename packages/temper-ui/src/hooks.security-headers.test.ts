import { describe, expect, it, vi } from 'vitest';

/**
 * The baseline headers, witnessed through the real `handle` hook rather than through the pure
 * function `security-headers.test.ts` covers.
 *
 * A unit test of `applySecurityHeaders` proves the header set is right and proves nothing about
 * whether anything calls it. This file is the wiring: it asserts a rendered response comes out
 * carrying the baseline, and — the half that is easy to get wrong — that a **proxied** response does
 * not have the UI's headers written over the upstream's.
 *
 * The mocks below stand in for module-load side effects, not for anything under test; see the same
 * pattern and its history in `hooks.server.test.ts`.
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

const proxied = new Response('upstream said so', {
	headers: {
		'content-security-policy': "default-src 'none'; style-src 'unsafe-inline'",
		'x-content-type-options': 'nosniff',
	},
});

vi.mock('$lib/server/proxy', () => ({
	isProxiedPath: (pathname: string) => pathname.startsWith('/api/'),
	proxyRequest: () => Promise.resolve(proxied),
}));

import { handle } from './hooks.server';

const eventFor = (url: string) =>
	({
		url: new URL(url),
		request: new Request(url),
		cookies: {},
		locals: {},
	}) as never;

const run = (url: string, rendered = new Response('page')) =>
	handle({
		event: eventFor(url),
		resolve: () => Promise.resolve(rendered),
	} as never) as Promise<Response>;

describe('the baseline reaches real responses', () => {
	it('a rendered page carries every baseline header', async () => {
		const response = await run('https://temper.invalid/goals');

		expect(response.headers.get('x-content-type-options')).toBe('nosniff');
		expect(response.headers.get('x-frame-options')).toBe('DENY');
		expect(response.headers.get('referrer-policy')).toBe('no-referrer');
		expect(response.headers.get('strict-transport-security')).toBe(
			'max-age=63072000; includeSubDomains',
		);
	});

	/**
	 * A proxied response belongs to the upstream, which sets its own headers — including, for the
	 * Slack callback page, a deliberately relaxed content-security policy that lets its inline
	 * `<style>` block render.
	 *
	 * Had the hook applied the baseline after the proxy short-circuit instead of before it, this
	 * app would be a second writer on those names. The failure would not have been loud: the page
	 * would simply have rendered unstyled, on a route nobody visits except once, at the end of
	 * linking a Slack account.
	 */
	it('a proxied response keeps the upstream policy untouched', async () => {
		const response = await run('https://temper.invalid/api/auth/slack/callback?error=denied');

		expect(response.headers.get('content-security-policy')).toBe(
			"default-src 'none'; style-src 'unsafe-inline'",
		);
	});
});
