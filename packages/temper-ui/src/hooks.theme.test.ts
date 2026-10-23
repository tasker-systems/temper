import { describe, expect, it, vi } from 'vitest';

/**
 * The served theme, witnessed through the real `handle` hook rather than through the pure
 * functions `$lib/theme`'s tests cover: a page comes out written in the reader's theme, whichever
 * of `renderRequest`'s exits resolved it, and a marketing page comes out dark regardless.
 *
 * The mocks stand in for module-load side effects, as in `hooks.security-headers.test.ts`.
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

import { handle } from './hooks.server';

const SHELL = '<html lang="en" data-theme="quiet-instrument"><head></head></html>';

type Opts = {
	transformPageChunk?: (c: { html: string; done: boolean }) => Promise<string | undefined>;
};

/** The page `handle` serves for a route, given the theme cookie (or none). */
async function served(routeId: string, cookie?: string): Promise<string> {
	const event = {
		url: new URL('https://temper.invalid/somewhere'),
		request: new Request('https://temper.invalid/somewhere'),
		cookies: { get: (name: string) => (name === 'temper.theme' ? cookie : undefined) },
		locals: {},
		route: { id: routeId },
	};
	const resolve = async (_e: unknown, opts?: Opts) =>
		new Response((await opts?.transformPageChunk?.({ html: SHELL, done: true })) ?? SHELL);
	const response = (await handle({ event, resolve } as never)) as Response;
	return response.text();
}

describe('a page is served in the reader’s theme', () => {
	it('a fixed preference is written onto the root', async () => {
		expect(await served('/(app)/vault/all', 'fixed:quiet-instrument-paper')).toContain(
			'data-theme="quiet-instrument-paper"',
		);
	});

	it('with no cookie, the page carries the light candidate for the inline script', async () => {
		const html = await served('/(app)/vault/all');
		expect(html).toContain('data-theme="quiet-instrument"');
		expect(html).toContain('data-theme-light="quiet-instrument-paper"');
	});

	it('a marketing page is dark whatever the reader prefers', async () => {
		const html = await served('/(public)/manifesto', 'fixed:quiet-instrument-paper');
		expect(html).toContain('data-theme="quiet-instrument"');
		expect(html).not.toContain('paper');
	});
});
