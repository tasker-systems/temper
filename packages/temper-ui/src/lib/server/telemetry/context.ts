/**
 * W3C trace-context helpers: extract an inbound parent, and read the active span's
 * `traceparent` for outbound propagation.
 *
 * These are the load-bearing half of the task. Extracting the inbound parent and
 * opening a span is inert on its own — the propagated `traceparent` only closes the
 * "internal dangle" (lets temper-api's post-auth `link` resolve to a real, exported
 * span) if the id we *send outbound* names the span we *export*. So every outbound
 * hop (`proxy.ts`, `api.ts`) stamps {@link activeTraceparent}, which is derived from
 * the active span rather than minted independently.
 *
 * Generic (no SvelteKit types) so it lifts into `temper-telemetry-ts` — see `./otel`.
 */

import { type Context, context, isSpanContextValid, propagation, trace } from '@opentelemetry/api';

/**
 * Extract an inbound `traceparent`/`tracestate` into a {@link Context} usable as a
 * span parent. When the caller sent no trace context this returns the active context
 * unchanged, so a UI-originated request simply starts a fresh root trace.
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
 * into a carrier — the outbound callers only need the one header value, and a plain
 * string keeps the call sites (which already set a `traceparent` header) unchanged in
 * shape. The value names the exported UI span, so a receiver's link to it resolves.
 */
export function activeTraceparent(): string | null {
	const span = trace.getSpan(context.active());
	if (!span) return null;
	const sc = span.spanContext();
	if (!isSpanContextValid(sc)) return null;
	const flags = (sc.traceFlags & 0x01) === 0x01 ? '01' : '00';
	return `00-${sc.traceId}-${sc.spanId}-${flags}`;
}
