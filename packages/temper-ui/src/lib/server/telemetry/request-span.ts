/**
 * SvelteKit request-lifecycle glue for span export. This is the one telemetry module
 * that is deliberately NOT generic — it references `RequestEvent` and Vercel's
 * `waitUntil` — so it stays in temper-ui when `./otel` + `./context` lift into
 * `temper-telemetry-ts`.
 */

import { context, SpanKind, SpanStatusCode, trace } from '@opentelemetry/api';
import type { RequestEvent } from '@sveltejs/kit';
import { waitUntil } from '@vercel/functions';
import { extractContext } from './context';
import { forceFlush, getTracer, isTelemetryEnabled } from './otel';

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

	const span = getTracer().startSpan(
		`${request.method} ${event.route?.id ?? url.pathname}`,
		{
			kind: SpanKind.SERVER,
			attributes: {
				'http.request.method': request.method,
				'url.path': url.pathname,
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
