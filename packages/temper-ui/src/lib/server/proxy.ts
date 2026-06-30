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
 * Forward the inbound request to the configured upstream, preserving method,
 * headers (incl. cookies/authorization), body, and query. The upstream response
 * (status, headers, body) is streamed back unchanged.
 *
 * `new Request(target, event.request)` copies the method/headers/body stream and
 * lets fetch manage the `Host` header for the new target URL.
 */
export async function proxyRequest(event: RequestEvent): Promise<Response> {
	const upstream = env.API_BASE_URL;
	if (!upstream) {
		throw error(500, 'Proxy upstream not configured: set API_BASE_URL');
	}
	const target = buildUpstreamUrl(upstream, event.url.pathname, event.url.search);
	return fetch(new Request(target, event.request));
}
