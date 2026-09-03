import type { RequestEvent } from '@sveltejs/kit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const startSpan = vi.fn();

vi.mock('temper-telemetry-ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('temper-telemetry-ts')>();
	return {
		...actual,
		isTelemetryEnabled: () => true,
		getTracer: () => ({ startSpan }),
		forceFlush: () => Promise.resolve(),
	};
});

const { traceRequest } = await import('./request-span');

/**
 * A minimal RequestEvent: only what `traceRequest` reads. `route` is the part
 * SvelteKit fills in after routing — null exactly for proxied paths (which
 * short-circuit in hooks before routing) and for requests matching no route.
 */
function makeEvent(pathname: string, routeId?: string): RequestEvent {
	const url = new URL(`https://temperkb.io${pathname}`);
	return {
		url,
		request: new Request(url, { method: 'GET' }),
		route: routeId === undefined ? null : { id: routeId },
	} as unknown as RequestEvent;
}

/**
 * The span is the only place a request's identity exports from this hop, so the
 * property under test is absolute: **no raw pathname fragment may survive into
 * the span name or its attributes.** Page paths carry titles, slugs, handles,
 * and resource ids; proxied paths carry resource UUIDs. The door (route pattern
 * or proxy group) replaces the path in both the name and `url.path`.
 */
describe('traceRequest span shape', () => {
	beforeEach(() => {
		startSpan.mockClear();
		startSpan.mockImplementation(() => ({
			setAttribute: vi.fn(),
			setStatus: vi.fn(),
			recordException: vi.fn(),
			end: vi.fn(),
		}));
	});

	it('a matched page route exports its pattern, not the requested slug or handle', async () => {
		await traceRequest(
			makeEvent('/vault/alices-handle/notes', '/(app)/vault/[owner]/[context]'),
			() => Promise.resolve(new Response('ok')),
		);
		const [name, options] = startSpan.mock.calls[0];
		expect(name).toBe('GET /(app)/vault/[owner]/[context]');
		expect(options.attributes['url.path']).toBe('/(app)/vault/[owner]/[context]');
	});

	it('the graph route exports the [owner] pattern, not the handle', async () => {
		await traceRequest(makeEvent('/graph/j-cole-taylor', '/(app)/graph/[owner]'), () =>
			Promise.resolve(new Response('ok')),
		);
		const [name, options] = startSpan.mock.calls[0];
		expect(name).toBe('GET /(app)/graph/[owner]');
		expect(options.attributes['url.path']).toBe('/(app)/graph/[owner]');
	});

	it('a proxied API path exports the proxy group, not the resource id', async () => {
		await traceRequest(makeEvent('/api/resources/019f97a7-ad61-7e40-b325-73028060ac06'), () =>
			Promise.resolve(new Response('ok')),
		);
		const [name, options] = startSpan.mock.calls[0];
		expect(name).toBe('GET /api/*');
		expect(options.attributes['url.path']).toBe('/api/*');
	});

	it('each proxied root groups under itself', async () => {
		for (const [path, group] of [
			['/mcp', '/mcp/*'],
			['/oauth/token', '/oauth/*'],
			['/.well-known/openid-configuration', '/.well-known/*'],
		] as const) {
			startSpan.mockClear();
			await traceRequest(makeEvent(path), () => Promise.resolve(new Response('ok')));
			const [name, options] = startSpan.mock.calls[0];
			expect(name).toBe(`GET ${group}`);
			expect(options.attributes['url.path']).toBe(group);
		}
	});

	it('a request matching no route and no proxy root is `unmatched`', async () => {
		await traceRequest(makeEvent('/no-such-page'), () => Promise.resolve(new Response('ok')));
		const [name, options] = startSpan.mock.calls[0];
		expect(name).toBe('GET unmatched');
		expect(options.attributes['url.path']).toBe('unmatched');
	});

	it('the raw pathname appears nowhere in the span name or attributes', async () => {
		// The identifying segment(s) of each path — the parts a person or a resource
		// could be located by. Static route words (`vault`, `graph`, `api`, `resources`)
		// legitimately survive inside route patterns and proxy groups.
		for (const [path, routeId, forbidden] of [
			['/vault/alices-handle/notes', '/(app)/vault/[owner]/[context]', ['alices-handle', 'notes']],
			['/graph/j-cole-taylor', '/(app)/graph/[owner]', ['j-cole-taylor']],
			[
				'/api/resources/019f97a7-ad61-7e40-b325-73028060ac06',
				undefined,
				['019f97a7-ad61-7e40-b325-73028060ac06'],
			],
			['/no-such-page', undefined, ['no-such-page']],
		] as const) {
			startSpan.mockClear();
			await traceRequest(makeEvent(path, routeId), () => Promise.resolve(new Response('ok')));
			const [name, options] = startSpan.mock.calls[0];
			const exported = [String(name), ...Object.values(options.attributes ?? {}).map(String)];
			for (const secret of forbidden) {
				for (const value of exported) {
					expect(value).not.toContain(secret);
				}
			}
		}
	});
});
