import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { gzipSync } from 'node:zlib';
import {
	isProxiedPath,
	buildUpstreamUrl,
	forwardRequest,
	isSelfReferentialUpstream
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
			'https://api.example.com/api/profile?q=x'
		);
	});

	it('preserves an empty query string', () => {
		expect(buildUpstreamUrl('https://api.example.com', '/mcp', '')).toBe(
			'https://api.example.com/mcp'
		);
	});

	it('tolerates a trailing slash on the upstream base', () => {
		expect(buildUpstreamUrl('https://api.example.com/', '/oauth/token', '')).toBe(
			'https://api.example.com/oauth/token'
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
		expect(isSelfReferentialUpstream('https://temper-cloud.vercel.app', 'temperkb.io')).toBe(
			false
		);
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
	let lastRequest: { method: string; url: string; body: string; auth: string | undefined };

	beforeAll(async () => {
		server = createServer((req: IncomingMessage, res: ServerResponse) => {
			const chunks: Buffer[] = [];
			req.on('data', (c) => chunks.push(c as Buffer));
			req.on('end', () => {
				lastRequest = {
					method: req.method ?? '',
					url: req.url ?? '',
					body: Buffer.concat(chunks).toString('utf-8'),
					auth: req.headers.authorization
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
						'content-length': String(payload.byteLength)
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
				body: JSON.stringify({ a: 1 })
			})
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
				headers: { 'accept-encoding': 'gzip, br' }
			})
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
			new Request('http://ui.local/redirect')
		);
		expect(res.status).toBe(302);
		expect(res.headers.get('location')).toBe('/landed');
	});
});
