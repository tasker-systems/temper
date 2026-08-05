/**
 * Neutralizes the span status of the MCP client's SSE-stream negotiation probe.
 *
 * The AI SDK's MCP client issues `GET <mcp endpoint>` to probe for a server-initiated SSE
 * stream. Our server does not offer SSE there — the deliberate Streamable-HTTP-on-serverless
 * choice — and the MCP spec requires a server without an SSE stream at that endpoint to answer
 * `405`. Client correct, server correct, spec satisfied. But
 * `@opentelemetry/instrumentation-undici` marks any `4xx` client span `status=error`, so a
 * mandated negotiation response was recorded as a failure 446 times a day, which was **77% of
 * total system error volume** and loud enough to bury everything that was not. Measured
 * 2026-08-04; research `019fce6a-75a5-7012-99cb-ca71fb2e7711`.
 *
 * **What this does and does not hide.** Only the *status* is reset. The span, its timing and
 * its `http.response.status_code=405` attribute all survive, so the per-tool-call round trip
 * stays visible in Tempo and stays countable. Suppressing the symptom and eliminating the probe
 * are different fixes answering different questions; this is the first, and it does not
 * foreclose the second.
 *
 * **Why a span processor and not `responseHook`.** The instrumentation invokes `responseHook`
 * at `undici.js:291` and *then* sets the status at `:310`, so anything a hook writes is
 * overwritten. And the status cannot be cleared through the API at all: `Span.setStatus`
 * returns early on `UNSET` by design (`@opentelemetry/sdk-trace` `Span.js:221`). What works is
 * `onEnding`, which `Span.end()` calls *before* marking the span ended and before any
 * `onEnd` — and which `MultiSpanProcessor` fans out to every processor that defines it, so this
 * is order-independent with respect to the exporting processor. The field is then assigned
 * directly, which is legitimate because `Span.status` is declared writable.
 *
 * **`onEnding` is marked `@experimental` in the SDK** and may change in a minor version. That
 * is pinned by `tests/mcp-negotiation.test.ts`, which asserts the exported status rather than
 * the hook — if the seam ever stops firing, the suite goes red rather than the dashboard
 * quietly re-inverting.
 */
import { SpanStatusCode } from '@opentelemetry/api';
import { ATTR_HTTP_REQUEST_METHOD, ATTR_HTTP_RESPONSE_STATUS_CODE, ATTR_URL_FULL } from '@opentelemetry/semantic-conventions';
/**
 * Reduce a URL to the `origin + pathname` form the predicate compares on, dropping query,
 * fragment and any trailing slash. Returns `null` for anything unparseable — a caller that
 * cannot be resolved to an endpoint simply never matches, since telemetry must not throw.
 */
export function negotiationKey(raw) {
    let parsed;
    try {
        parsed = new URL(raw);
    }
    catch {
        return null;
    }
    const path = parsed.pathname.replace(/\/+$/, '');
    return `${parsed.origin}${path}`;
}
/**
 * Re-expose the status field as writable, which it genuinely is.
 *
 * `Span` is declared as `APISpan & ReadableSpan` and `ReadableSpan.status` is `readonly`, so the
 * *type* forbids assignment — but the concrete `SpanImpl` the SDK hands a processor declares
 * `status: SpanStatus`, plainly mutable (`@opentelemetry/sdk-trace` `Span.d.ts:45`). The
 * readonly view exists to stop callers editing a span they merely read, which is the right
 * default; a processor holding the span mid-`end()` is the one caller for which it is wrong.
 *
 * Assignment is the only route: `setStatus` cannot express this, because it returns early on
 * `UNSET` (`Span.js:221`) precisely to stop a late caller *downgrading* a status — which is what
 * we are deliberately doing, for a response that was never a failure.
 */
function writableStatus(span) {
    return span;
}
/**
 * A {@link SpanProcessor} that resets the status of the spec-mandated `GET` → `405` MCP
 * negotiation response against one known endpoint. Every other span is left untouched,
 * including a `405` from any other host and any other method against this one.
 */
export class McpNegotiationStatusProcessor {
    endpointKey;
    /**
     * @param endpointKey the `origin + pathname` of the MCP endpoint, as produced by
     * {@link negotiationKey}. Taking the normalized form rather than a raw string keeps the
     * "is this even a URL?" decision at the call site, where it can be reported once at init
     * instead of silently per span.
     */
    constructor(endpointKey) {
        this.endpointKey = endpointKey;
    }
    onStart() { }
    onEnding(span) {
        if (!this.isMandatedNegotiation(span))
            return;
        writableStatus(span).status = { code: SpanStatusCode.UNSET };
    }
    onEnd() { }
    forceFlush() {
        return Promise.resolve();
    }
    shutdown() {
        return Promise.resolve();
    }
    /**
     * Keyed on the response shape — method and status — plus the endpoint, so it fires for
     * every MCP client we run rather than for one named service, and so a `GET` that genuinely
     * fails with `405` somewhere else still reads as an error.
     */
    isMandatedNegotiation(span) {
        const attrs = span.attributes;
        if (attrs[ATTR_HTTP_REQUEST_METHOD] !== 'GET')
            return false;
        if (attrs[ATTR_HTTP_RESPONSE_STATUS_CODE] !== 405)
            return false;
        const url = attrs[ATTR_URL_FULL];
        if (typeof url !== 'string')
            return false;
        return negotiationKey(url) === this.endpointKey;
    }
}
