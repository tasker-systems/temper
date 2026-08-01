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
 * Time budget for the upstream to *begin* responding (send headers). The body
 * that follows is deliberately NOT bounded by this — an MCP streamed response
 * can hold the connection open well past it, and cutting the stream would be a
 * worse failure than the slow start we are guarding against. See `forwardOnce`.
 */
export const UPSTREAM_CONNECT_TIMEOUT_MS = 20_000;

/**
 * Methods safe to replay after a connection-level failure. A `TypeError: fetch
 * failed` (undici, no HTTP status) can mean the request never reached the
 * upstream OR that a response was lost in flight, and the proxy cannot tell
 * which. Replaying a write could double-apply it — this proxy carries
 * `PATCH /api/resources` and `POST /api/auditor/.../complete`, both
 * non-idempotent — so only idempotent, effectively-bodyless methods are
 * retried; everything else surfaces the failure as 502.
 */
const RETRYABLE_METHODS = new Set(['GET', 'HEAD']);

/** Options for {@link forwardRequest}; defaulted in production, overridden in tests. */
export interface ForwardOptions {
	/** Header/connect timeout in ms. Default {@link UPSTREAM_CONNECT_TIMEOUT_MS}. */
	connectTimeoutMs?: number;
	/** Retries beyond the first attempt, applied only to {@link RETRYABLE_METHODS}. Default 1. */
	maxRetries?: number;
}

/** True for the undici timeout we raise via `AbortController`, so it can map to 504 not 502. */
function isTimeout(err: unknown): boolean {
	return typeof err === 'object' && err !== null && (err as { name?: string }).name === 'AbortError';
}

/**
 * A single forward attempt: relay method/headers/body to `target` and hand back
 * the upstream response.
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
 *
 * The connect timeout is armed before the fetch and cleared the instant headers
 * arrive, so it bounds time-to-first-byte only and leaves the response body
 * stream to run for as long as it needs.
 */
async function forwardOnce(target: string, request: Request, timeoutMs: number): Promise<Response> {
	const controller = new AbortController();
	const timer = setTimeout(() => controller.abort(), timeoutMs);
	let upstream: Response;
	try {
		upstream = await fetch(new Request(target, request), {
			redirect: 'manual',
			signal: controller.signal
		});
	} finally {
		clearTimeout(timer);
	}
	const headers = new Headers(upstream.headers);
	headers.delete('content-encoding');
	headers.delete('content-length');
	return new Response(upstream.body, {
		status: upstream.status,
		statusText: upstream.statusText,
		headers
	});
}

/**
 * Forward a request to `upstreamBase`, preserving method, headers (incl.
 * cookies/authorization), body, and query, and relaying the upstream response.
 *
 * When the upstream cannot be reached, this returns a **502** (connection
 * failure) or **504** (connect timeout) rather than letting the `fetch`
 * rejection bubble into SvelteKit's generic `500 {"message":"Internal Error"}`.
 * The distinction is not cosmetic: temper-ui is a reverse proxy, so an upstream
 * blip is a *gateway* failure, not an application bug. Collapsing it into a 500
 * is what made a burst of transient proxy failures fire an error-anomaly alert
 * that looked like the UI had crashed (see the 2026-08-01 incident, where the
 * upstream API logged zero errors across the same window).
 */
export async function forwardRequest(
	upstreamBase: string,
	pathname: string,
	search: string,
	request: Request,
	options: ForwardOptions = {}
): Promise<Response> {
	const target = buildUpstreamUrl(upstreamBase, pathname, search);
	const timeoutMs = options.connectTimeoutMs ?? UPSTREAM_CONNECT_TIMEOUT_MS;
	const retries = RETRYABLE_METHODS.has(request.method.toUpperCase())
		? (options.maxRetries ?? 1)
		: 0;

	let lastErr: unknown;
	for (let attempt = 0; attempt <= retries; attempt++) {
		try {
			return await forwardOnce(target, request, timeoutMs);
		} catch (err) {
			lastErr = err;
		}
	}

	// Every attempt failed at the connection layer. Relay as a gateway error and
	// log enough to line the blip up against the upstream's own logs — the
	// inbound `traceparent` if the caller sent one (the Rust API links trusted
	// callers by it), plus the target and method. There is no upstream response
	// to read an `x-vercel-id` from here, precisely because the fetch never
	// completed.
	const timedOut = isTimeout(lastErr);
	console.error('proxy: upstream unreachable', {
		target,
		method: request.method,
		traceparent: request.headers.get('traceparent') ?? undefined,
		attempts: retries + 1,
		timedOut,
		error: lastErr instanceof Error ? lastErr.message : String(lastErr)
	});
	return new Response(
		JSON.stringify({
			message: timedOut
				? 'Gateway Timeout: upstream API did not respond in time'
				: 'Bad Gateway: upstream API unreachable'
		}),
		{ status: timedOut ? 504 : 502, headers: { 'content-type': 'application/json' } }
	);
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
