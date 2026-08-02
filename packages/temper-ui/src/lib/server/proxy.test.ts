import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { gzipSync } from 'node:zlib';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import {
	buildUpstreamUrl,
	forwardRequest,
	isProxiedPath,
	isSelfReferentialUpstream,
} from './proxy';

describe('isProxiedPath', () => {
	it('matches the MCP entrypoint (exact and subpaths)', () => {
		expect(isProxiedPath('/mcp')).toBe(true);
		expect(isProxiedPath('/mcp/messages')).toBe(true);
	});

	it('matches OAuth, discovery, and API prefixes', () => {
		expect(isProxiedPath('/oauth/token')).toBe(true);
		expect(isProxiedPath('/.well-known/openid-configuration')).toBe(true);
		expect(isProxiedPath('/.well-known/oauth-authorization-server')).toBe(true);
		expect(isProxiedPath('/api/profile')).toBe(true);
		expect(isProxiedPath('/api/resources?q=x')).toBe(true);
	});

	it('does NOT match the app, auth, or marketing routes the UI owns', () => {
		expect(isProxiedPath('/')).toBe(false);
		expect(isProxiedPath('/vault/all')).toBe(false);
		expect(isProxiedPath('/auth/login')).toBe(false);
		expect(isProxiedPath('/manifesto')).toBe(false);
	});

	it('does not over-match prefixes that merely share a leading substring', () => {
		expect(isProxiedPath('/mcpfoo')).toBe(false);
		expect(isProxiedPath('/apilike')).toBe(false);
		expect(isProxiedPath('/oauthish')).toBe(false);
	});
});

describe('buildUpstreamUrl', () => {
	it('joins the upstream base with the request path and query', () => {
		expect(buildUpstreamUrl('https://api.example.com', '/api/profile', '?q=x')).toBe(
			'https://api.example.com/api/profile?q=x',
		);
	});

	it('preserves an empty query string', () => {
		expect(buildUpstreamUrl('https://api.example.com', '/mcp', '')).toBe(
			'https://api.example.com/mcp',
		);
	});

	it('tolerates a trailing slash on the upstream base', () => {
		expect(buildUpstreamUrl('https://api.example.com/', '/oauth/token', '')).toBe(
			'https://api.example.com/oauth/token',
		);
	});
});

describe('isSelfReferentialUpstream', () => {
	it('flags an upstream whose host equals the UI origin (the self-proxy loop)', () => {
		expect(isSelfReferentialUpstream('https://temperkb.io', 'temperkb.io')).toBe(true);
		// host comparison ignores path / trailing slash on the base
		expect(isSelfReferentialUpstream('https://temperkb.io/', 'temperkb.io')).toBe(true);
	});

	it('allows an upstream on a different host (the correct config)', () => {
		expect(isSelfReferentialUpstream('https://temper-cloud.vercel.app', 'temperkb.io')).toBe(false);
	});

	it('does not throw on a malformed upstream base', () => {
		expect(isSelfReferentialUpstream('not a url', 'temperkb.io')).toBe(false);
	});
});

describe('forwardRequest (passthrough)', () => {
	// A minimal upstream that records what it received and exercises the two
	// behaviors the platform rewrite used to handle: compression and redirects.
	let server: Server;
	let base: string;
	let lastRequest: {
		method: string;
		url: string;
		body: string;
		auth: string | undefined;
		traceparent: string | undefined;
	};

	beforeAll(async () => {
		server = createServer((req: IncomingMessage, res: ServerResponse) => {
			const chunks: Buffer[] = [];
			req.on('data', (c) => chunks.push(c as Buffer));
			req.on('end', () => {
				lastRequest = {
					method: req.method ?? '',
					url: req.url ?? '',
					body: Buffer.concat(chunks).toString('utf-8'),
					auth: req.headers.authorization,
					traceparent: req.headers.traceparent as string | undefined,
				};

				if (req.url?.startsWith('/redirect')) {
					res.writeHead(302, { location: '/landed' });
					res.end();
					return;
				}
				if (req.url?.startsWith('/gzip')) {
					const payload = gzipSync(Buffer.from(JSON.stringify({ hello: 'world' })));
					res.writeHead(200, {
						'content-type': 'application/json',
						'content-encoding': 'gzip',
						'content-length': String(payload.byteLength),
					});
					res.end(payload);
					return;
				}
				res.writeHead(200, { 'content-type': 'application/json' });
				res.end(JSON.stringify({ ok: true, echoedBody: lastRequest.body }));
			});
		});
		await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
		const { port } = server.address() as AddressInfo;
		base = `http://127.0.0.1:${port}`;
	});

	afterAll(() => {
		server.close();
	});

	it('forwards method, body, and authorization to the upstream, relaying the response', async () => {
		const res = await forwardRequest(
			base,
			'/api/resources',
			'?q=x',
			new Request('http://ui.local/api/resources?q=x', {
				method: 'POST',
				headers: { authorization: 'Bearer tok', 'content-type': 'application/json' },
				body: JSON.stringify({ a: 1 }),
			}),
		);
		expect(res.status).toBe(200);
		expect(lastRequest.method).toBe('POST');
		expect(lastRequest.url).toBe('/api/resources?q=x');
		expect(lastRequest.auth).toBe('Bearer tok');
		expect(JSON.parse(lastRequest.body)).toEqual({ a: 1 });
		expect(await res.json()).toMatchObject({ ok: true, echoedBody: JSON.stringify({ a: 1 }) });
	});

	it('relays a compressed response without leaving stale content-encoding/length', async () => {
		const res = await forwardRequest(
			base,
			'/gzip',
			'',
			new Request('http://ui.local/gzip', {
				headers: { 'accept-encoding': 'gzip, br' },
			}),
		);
		// undici already decoded the body; the relayed response must not still
		// claim gzip (or carry the now-wrong compressed length) or the browser
		// fails with ERR_CONTENT_DECODING_FAILED.
		expect(res.headers.get('content-encoding')).toBeNull();
		expect(res.headers.get('content-length')).toBeNull();
		expect(await res.json()).toEqual({ hello: 'world' });
	});

	it('relays an upstream redirect to the caller instead of following it', async () => {
		const res = await forwardRequest(
			base,
			'/redirect',
			'',
			new Request('http://ui.local/redirect'),
		);
		expect(res.status).toBe(302);
		expect(res.headers.get('location')).toBe('/landed');
	});

	it('forwards an inbound traceparent to the upstream unchanged', async () => {
		const tp = '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01';
		await forwardRequest(
			base,
			'/api/x',
			'',
			new Request('http://ui.local/api/x', { headers: { traceparent: tp } }),
		);
		expect(lastRequest.traceparent).toBe(tp);
	});

	it('generates a well-formed W3C traceparent when the caller sent none', async () => {
		await forwardRequest(base, '/api/x', '', new Request('http://ui.local/api/x'));
		// version 00, 32-hex trace-id, 16-hex span-id, sampled.
		expect(lastRequest.traceparent).toMatch(/^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/);
	});
});

describe('forwardRequest (upstream failure handling)', () => {
	// An address that refuses connections, so `fetch` rejects with the
	// connection-level `TypeError: fetch failed` this proxy has to absorb.
	const unreachable = 'http://127.0.0.1:1'; // port 1 is not listenable in practice

	it('returns 502 (not a naked 500) when the upstream is unreachable', async () => {
		const res = await forwardRequest(
			unreachable,
			'/api/profile',
			'',
			new Request('http://ui.local/api/profile'),
		);
		expect(res.status).toBe(502);
		expect(await res.json()).toMatchObject({ message: expect.stringContaining('unreachable') });
	});

	it('returns 504 when the upstream does not respond within the connect timeout', async () => {
		// A server that accepts the connection but never writes a response header,
		// so only the connect timeout can end the wait.
		const hung = createServer(() => {
			/* intentionally never responds */
		});
		await new Promise<void>((resolve) => hung.listen(0, '127.0.0.1', resolve));
		const { port } = hung.address() as AddressInfo;
		try {
			const res = await forwardRequest(
				`http://127.0.0.1:${port}`,
				'/api/profile',
				'',
				new Request('http://ui.local/api/profile'),
				{ connectTimeoutMs: 50 },
			);
			expect(res.status).toBe(504);
			expect(await res.json()).toMatchObject({
				message: expect.stringContaining('did not respond'),
			});
		} finally {
			hung.close();
		}
	});

	it('retries an idempotent GET once after a dropped connection, then succeeds', async () => {
		let hits = 0;
		const flaky = createServer((req: IncomingMessage, res: ServerResponse) => {
			hits += 1;
			if (hits === 1) {
				req.socket.destroy(); // ECONNRESET → undici `fetch failed`
				return;
			}
			res.writeHead(200, { 'content-type': 'application/json' });
			res.end(JSON.stringify({ ok: true }));
		});
		await new Promise<void>((resolve) => flaky.listen(0, '127.0.0.1', resolve));
		const { port } = flaky.address() as AddressInfo;
		try {
			const res = await forwardRequest(
				`http://127.0.0.1:${port}`,
				'/api/profile',
				'',
				new Request('http://ui.local/api/profile'),
			);
			expect(res.status).toBe(200);
			expect(hits).toBe(2); // failed once, retried once
		} finally {
			flaky.close();
		}
	});

	it('does NOT retry a non-idempotent POST — a dropped write surfaces as 502', async () => {
		let hits = 0;
		const flaky = createServer((req: IncomingMessage, res: ServerResponse) => {
			hits += 1;
			req.socket.destroy();
		});
		await new Promise<void>((resolve) => flaky.listen(0, '127.0.0.1', resolve));
		const { port } = flaky.address() as AddressInfo;
		try {
			const res = await forwardRequest(
				`http://127.0.0.1:${port}`,
				'/api/resources/abc',
				'',
				new Request('http://ui.local/api/resources/abc', {
					method: 'POST',
					body: JSON.stringify({ a: 1 }),
				}),
			);
			expect(res.status).toBe(502);
			expect(hits).toBe(1); // exactly one attempt — the write was not replayed
		} finally {
			flaky.close();
		}
	});

	it('retries a keyed write (Idempotency-Key header) after a dropped connection, replaying the same body', async () => {
		// A write carrying an idempotency key is replay-safe: the API dedups on (owner, key), so the
		// proxy retries it exactly like a GET (issue #581, spike rung 3-C). The replayed attempt must
		// carry the identical body, which requires the proxy to buffer it (a Request body is a
		// single-use stream).
		let hits = 0;
		const bodies: string[] = [];
		const flaky = createServer((req: IncomingMessage, res: ServerResponse) => {
			hits += 1;
			let raw = '';
			req.on('data', (c) => {
				raw += c;
			});
			req.on('end', () => {
				bodies.push(raw);
				if (hits === 1) {
					req.socket.destroy(); // drop the first attempt at the connection layer
					return;
				}
				res.writeHead(200, { 'content-type': 'application/json' });
				res.end(JSON.stringify({ id: 'r-1' }));
			});
		});
		await new Promise<void>((resolve) => flaky.listen(0, '127.0.0.1', resolve));
		const { port } = flaky.address() as AddressInfo;
		try {
			const res = await forwardRequest(
				`http://127.0.0.1:${port}`,
				'/api/ingest',
				'',
				new Request('http://ui.local/api/ingest', {
					method: 'POST',
					headers: { 'idempotency-key': '018f-key', 'content-type': 'application/json' },
					body: JSON.stringify({ title: 'doc', idempotency_key: '018f-key' }),
				}),
			);
			expect(res.status).toBe(200);
			expect(hits).toBe(2); // failed once, retried once
			// Both attempts saw the same body — the buffer was replayed, not lost.
			expect(bodies[1]).toBe(JSON.stringify({ title: 'doc', idempotency_key: '018f-key' }));
		} finally {
			flaky.close();
		}
	});
});
