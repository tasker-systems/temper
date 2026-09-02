/**
 * SvelteKit request-lifecycle glue for span export. This is the one telemetry module
 * that is deliberately NOT generic — it references `RequestEvent` and Vercel's
 * `waitUntil` — so it stays in temper-ui when `./otel` + `./context` lift into
 * `temper-telemetry-ts`.
 */

import { context, SpanKind, SpanStatusCode, trace } from '@opentelemetry/api';
import type { RequestEvent } from '@sveltejs/kit';
import { waitUntil } from '@vercel/functions';
import { extractContext, forceFlush, getTracer, isTelemetryEnabled } from 'temper-telemetry-ts';
import { proxiedRoot } from '$lib/server/proxy';

/**
 * The identifying-free name of the door a request hit.
 *
 * A matched UI route contributes its **pattern** — `/vault/[owner]/[context]`, never
 * the owner or context actually requested. A proxied path contributes its root group
 * — `/api/*`, `/mcp/*` — because proxied requests short-circuit before SvelteKit
 * routes them and their precise, redacted path is already carried by the upstream
 * API's own root span. Anything else is `unmatched`.
 *
 * This replaces the raw `url.pathname` this span used to carry in its name and
 * `url.path` attribute: page paths carry titles, slugs, handles, and resource ids,
 * and none of that belongs in an exported span.
 */
function requestDoor(event: RequestEvent): string {
	const routeId = event.route?.id;
	if (routeId) return routeId;
	const root = proxiedRoot(event.url.pathname);
	return root ? `${root}/*` : 'unmatched';
}

/**
 * Wrap the whole `handle` body in a SERVER span whose parent is the inbound
 * `traceparent`, running the body inside the span's active context so every outbound
 * `fetch` made during the request (the reverse proxy AND the SSR data loaders) can
 * stamp {@link activeTraceparent}.
 *
 * The span is opened here — around the *entire* handler, before the proxy
 * short-circuit — precisely because the acceptance path (a browser hitting a proxied
 * `/api` route) returns early from `proxyRequest` and never reaches `resolve(event)`.
 * A span opened later would miss exactly the hop the task exists to stitch.
 *
 * When export is disabled (no OTLP endpoint) this is a straight pass-through with zero
 * overhead — the common case for local dev and any self-hosted install without an
 * endpoint configured.
 */
export async function traceRequest(
	event: RequestEvent,
	run: () => Promise<Response> | Response,
): Promise<Response> {
	if (!isTelemetryEnabled()) return run();

	const parentCtx = extractContext(event.request.headers);
	const { url, request } = event;
	const door = requestDoor(event);

	const span = getTracer().startSpan(
		`${request.method} ${door}`,
		{
			kind: SpanKind.SERVER,
			attributes: {
				'http.request.method': request.method,
				// The door, not the URL: a route pattern or proxy group, per the module
				// decision above. The semantic name is kept so TraceQL on `url.path`
				// keeps working — the value is deliberately not the raw path.
				'url.path': door,
				'url.scheme': url.protocol.replace(/:$/, ''),
				'server.address': url.host,
				// NB: NO end-user identifiers (sub / email / name) as span attributes.
				// PR #613's 2a dropped the raw OAuth `sub` from Rust spans for cross-system
				// linkability reasons; this new span site sits right next to those claims in
				// `hooks.server.ts` and must not reintroduce the surface. See task §PII.
			},
		},
		parentCtx,
	);

	const ctxWithSpan = trace.setSpan(parentCtx, span);
	try {
		const response = await context.with(ctxWithSpan, run);
		span.setAttribute('http.response.status_code', response.status);
		if (response.status >= 500) {
			span.setStatus({ code: SpanStatusCode.ERROR });
		}
		return response;
	} catch (err) {
		span.recordException(err as Error);
		span.setStatus({ code: SpanStatusCode.ERROR });
		throw err;
	} finally {
		span.end();
		// Flush-on-freeze: Vercel freezes the sandbox between invocations, so the
		// BatchSpanProcessor's timer may never fire and queued spans would be lost — the
		// JS mirror of the Rust per-response `force_flush`. `@vercel/functions`' waitUntil
		// keeps the sandbox alive until the export settles without blocking the response;
		// off-Vercel it is a safe no-op (the flush promise still runs in the dev process).
		waitUntil(forceFlush());
	}
}
