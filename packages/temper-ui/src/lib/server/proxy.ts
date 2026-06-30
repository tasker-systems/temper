/**
 * Browser-facing reverse proxy for the API/MCP/OAuth surface.
 *
 * The canonical deployment serves `/api`, `/mcp`, `/oauth`, and `/.well-known`
 * on the UI origin (so MCP clients can connect to `https://temperkb.io/mcp` and
 * OAuth discovery resolves at the apex domain). That used to be a static
 * `rewrites` block in `vercel.json` pointing at a hardcoded canonical host —
 * which a self-hosted install deployed from the monorepo cannot override,
 * because `vercel.json` rewrites don't interpolate env and the file is shared
 * across every UI target.
 *
 * Instead, `hooks.server.ts` forwards these prefixes to an operator-configurable
 * upstream (`API_BASE_URL`, read at runtime from `$env/dynamic/private`). The
 * UI's own server-side data loaders already use `API_BASE_URL` (api+mcp are
 * co-deployed on one host), so a single env var configures both. This keeps the
 * UI single-origin (cookies/headers pass straight through) and fully env-driven.
 *
 * `isProxiedPath` / `buildUpstreamUrl` are pure and unit-tested; `proxyRequest`
 * owns the env read and the fetch.
 */

import { env } from '$env/dynamic/private';
import { error, type RequestEvent } from '@sveltejs/kit';

/** Path roots forwarded to the upstream API/MCP host. */
const PROXIED_ROOTS = ['/mcp', '/oauth', '/.well-known', '/api'];

/**
 * Whether an inbound request path should be reverse-proxied to the upstream
 * rather than handled by SvelteKit. Matches each root exactly or as a path
 * prefix (`/mcp` and `/mcp/...`), but not paths that merely share a leading
 * substring (`/mcpfoo`).
 */
export function isProxiedPath(pathname: string): boolean {
	return PROXIED_ROOTS.some((root) => pathname === root || pathname.startsWith(`${root}/`));
}

/** Join the upstream base (trailing slash tolerated) with the request path + query. */
export function buildUpstreamUrl(upstreamBase: string, pathname: string, search: string): string {
	return `${upstreamBase.replace(/\/$/, '')}${pathname}${search}`;
}

/**
 * True when the upstream base resolves to the same host as the inbound request —
 * i.e. `API_BASE_URL` points at the UI's own origin. Forwarding to it would proxy
 * to ourselves forever (a platform 508 loop). This is a configuration mistake
 * that's easy to make when the UI and API share a public domain (the UI proxies
 * `/api` to the API): `API_BASE_URL` must be the API backend's *own* origin, not
 * the shared public origin. A malformed base is a different failure handled at
 * the `fetch` layer, so we don't flag it here.
 */
export function isSelfReferentialUpstream(upstreamBase: string, requestHost: string): boolean {
	try {
		return new URL(upstreamBase).host === requestHost;
	} catch {
		return false;
	}
}

/**
 * Forward a request to `upstreamBase`, preserving method, headers (incl.
 * cookies/authorization), body, and query, and relaying the upstream response.
 *
 * Two details the platform-level rewrite used to handle for us, now ours:
 *   - `redirect: 'manual'` so an upstream 3xx is relayed to the browser rather
 *     than followed server-side (OAuth/MCP flows depend on the browser seeing
 *     the redirect).
 *   - `fetch` (undici) transparently decompresses the body but leaves the
 *     upstream's `content-encoding`/`content-length` in place; those now
 *     mis-describe the decoded bytes, so we strip them before relaying or the
 *     browser fails to decode the response.
 *
 * `new Request(target, request)` copies the method/headers/body stream and lets
 * fetch manage the `Host` header for the new target URL.
 */
export async function forwardRequest(
	upstreamBase: string,
	pathname: string,
	search: string,
	request: Request
): Promise<Response> {
	const target = buildUpstreamUrl(upstreamBase, pathname, search);
	const upstream = await fetch(new Request(target, request), { redirect: 'manual' });
	const headers = new Headers(upstream.headers);
	headers.delete('content-encoding');
	headers.delete('content-length');
	return new Response(upstream.body, {
		status: upstream.status,
		statusText: upstream.statusText,
		headers
	});
}

/** Forward the inbound SvelteKit request to the operator-configured upstream. */
export async function proxyRequest(event: RequestEvent): Promise<Response> {
	const upstream = env.API_BASE_URL;
	if (!upstream) {
		throw error(500, 'Proxy upstream not configured: set API_BASE_URL');
	}
	if (isSelfReferentialUpstream(upstream, event.url.host)) {
		throw error(
			500,
			`Proxy upstream (API_BASE_URL=${upstream}) resolves to this same origin (${event.url.host}); ` +
				`set API_BASE_URL to the API backend's own origin, not the UI origin.`
		);
	}
	return forwardRequest(upstream, event.url.pathname, event.url.search, event.request);
}
