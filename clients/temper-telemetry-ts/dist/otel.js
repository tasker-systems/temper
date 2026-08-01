/**
 * OpenTelemetry span-export bootstrap for Temper's Node hops (temper-ui, eve agents).
 *
 * The TypeScript counterpart of the Rust `temper-telemetry` crate: it builds a
 * `NodeTracerProvider` that self-exports spans over OTLP/protobuf to the same Grafana
 * Cloud endpoint the Rust functions use, driven by the SAME env vars
 * (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME`).
 *
 * Why native `@opentelemetry`, not `@vercel/otel`: `@vercel/otel` presumes Next.js;
 * our hops are SvelteKit (temper-ui) and eve (steward/mention). Why OTLP/proto, not
 * JSON: same protocol as Rust, and it dodges the OTLP/JSON `TimeUnixNano` encoding bug.
 *
 * `@opentelemetry/api` is a versioned-global singleton (it registers on
 * `globalThis[Symbol.for('opentelemetry.js.api.1')]`), so multiple 1.x copies across a
 * consumer and this package share one context/propagator/provider — which is why this
 * works when temper-ui also imports `@opentelemetry/api` directly for its span glue.
 */
import { trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { BatchSpanProcessor, NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import { ATTR_SERVICE_NAME } from '@opentelemetry/semantic-conventions';
let provider = null;
let enabled = false;
let tracerName = 'temper-telemetry-ts';
/**
 * Build and register the tracer provider — **once**. Idempotent, so a repeated
 * side-effecting call (dev HMR, multiple entrypoints) does not double-register.
 *
 * Mirrors the Rust "no endpoint ⇒ no export" rule: when `OTEL_EXPORTER_OTLP_ENDPOINT`
 * is unset the provider is never built, span creation stays a no-op, and we never
 * default to `localhost:4318`. The exporter reads the endpoint and headers from the
 * standard env itself, so the only thing this function decides is *whether* to register
 * (and whether to add HTTP instrumentation).
 */
export function initTelemetry({ serviceName, instrumentHttp = false }) {
    if (provider)
        return;
    const endpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT?.trim();
    if (!endpoint) {
        console.info('[telemetry] OTEL_EXPORTER_OTLP_ENDPOINT unset; span export disabled');
        return;
    }
    // An `OTEL_SERVICE_NAME` env value (project-scoped on Vercel) wins over the passed
    // name; otherwise the consumer's name is authoritative.
    const resolvedServiceName = process.env.OTEL_SERVICE_NAME?.trim() || serviceName;
    tracerName = resolvedServiceName;
    // No `url`/`headers` passed: the OTLP/proto exporter reads
    // `OTEL_EXPORTER_OTLP_ENDPOINT` + `OTEL_EXPORTER_OTLP_HEADERS` natively, so config
    // stays in one place (env) shared with the Rust side.
    const exporter = new OTLPTraceExporter();
    const built = new NodeTracerProvider({
        resource: resourceFromAttributes({ [ATTR_SERVICE_NAME]: resolvedServiceName }),
        // BatchSpanProcessor + a per-request flush (the consumer's job) is the JS mirror of
        // the Rust `flush_within_budget`. The batch timer alone is unsafe on Vercel: the
        // sandbox freezes between invocations and the timer may never fire.
        spanProcessors: [new BatchSpanProcessor(exporter)]
    });
    built.register({
        contextManager: new AsyncHooksContextManager().enable(),
        propagator: new W3CTraceContextPropagator()
    });
    provider = built;
    enabled = true;
    if (instrumentHttp) {
        // Dynamic import so consumers that leave `instrumentHttp` off never load the undici
        // instrumentation. Fire-and-forget: undici instrumentation subscribes to
        // diagnostics_channel, so it captures requests made after this resolves — at server
        // startup that is well before the first request.
        void enableHttpInstrumentation();
    }
    console.info(`[telemetry] span export enabled: service.name=${resolvedServiceName} → ${endpoint}` +
        (instrumentHttp ? ' (+http instrumentation)' : ''));
}
async function enableHttpInstrumentation() {
    try {
        const [{ registerInstrumentations }, { UndiciInstrumentation }] = await Promise.all([
            import('@opentelemetry/instrumentation'),
            import('@opentelemetry/instrumentation-undici')
        ]);
        registerInstrumentations({ instrumentations: [new UndiciInstrumentation()] });
    }
    catch (err) {
        console.error('[telemetry] http instrumentation failed to register', err);
    }
}
/** Whether span export is registered (endpoint was configured). */
export function isTelemetryEnabled() {
    return enabled;
}
/** The tracer for this hop. A no-op tracer until {@link initTelemetry} registers a provider. */
export function getTracer() {
    return trace.getTracer(tracerName);
}
/**
 * Force-export any queued spans. Never rejects — a flush failure is logged, not
 * propagated, so a telemetry hiccup can never turn into a request failure. A no-op
 * when export is disabled.
 */
export async function forceFlush() {
    if (!provider)
        return;
    try {
        await provider.forceFlush();
    }
    catch (err) {
        console.error('[telemetry] forceFlush failed', err);
    }
}
