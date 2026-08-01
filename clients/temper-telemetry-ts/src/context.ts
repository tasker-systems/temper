/**
 * W3C trace-context helpers: extract an inbound parent, and read the active span's
 * `traceparent` for outbound propagation.
 *
 * These are the load-bearing half of Node-hop tracing. Extracting the inbound parent
 * and opening a span is inert on its own — a propagated `traceparent` only closes the
 * "internal dangle" (lets temper-api's post-auth `link` resolve to a real, exported
 * span) if the id sent outbound names the span that is exported.
 *
 * `activeTraceparent` is used by consumers that inject at a **known call site**
 * (temper-ui's proxy + SSR loaders). Consumers whose outbound HTTP is internal to a
 * framework (the eve agents' MCP client) cannot use it — they enable HTTP auto-
 * instrumentation via `initTelemetry({ instrumentHttp: true })` instead, which injects
 * per request from whatever span is active.
 */

import { context, isSpanContextValid, propagation, trace, type Context } from '@opentelemetry/api';

/**
 * Extract an inbound `traceparent`/`tracestate` into a {@link Context} usable as a
 * span parent. When the caller sent no trace context this returns the active context
 * unchanged, so a hop-originated request simply starts a fresh root trace.
 */
export function extractContext(headers: Headers): Context {
	const carrier: Record<string, string> = {};
	const traceparent = headers.get('traceparent');
	if (traceparent) carrier.traceparent = traceparent;
	const tracestate = headers.get('tracestate');
	if (tracestate) carrier.tracestate = tracestate;
	return propagation.extract(context.active(), carrier);
}

/**
 * The W3C `traceparent` string naming the **active** span, or `null` when there is
 * no valid active span (telemetry disabled, or called outside a request span).
 *
 * Built directly from the active span context rather than via `propagation.inject`
 * into a carrier — a known-call-site consumer only needs the one header value, and a
 * plain string keeps those call sites (which already set a `traceparent` header)
 * unchanged in shape. The value names the exported span, so a receiver's link resolves.
 */
export function activeTraceparent(): string | null {
	const span = trace.getSpan(context.active());
	if (!span) return null;
	const sc = span.spanContext();
	if (!isSpanContextValid(sc)) return null;
	const flags = (sc.traceFlags & 0x01) === 0x01 ? '01' : '00';
	return `00-${sc.traceId}-${sc.spanId}-${flags}`;
}
