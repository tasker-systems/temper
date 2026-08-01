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
import { type Tracer } from '@opentelemetry/api';
export interface InitTelemetryOptions {
    /**
     * `service.name` for exported spans, and the instrumentation-scope name for
     * {@link getTracer}. temper-ui passes `"temper-ui"`; an eve agent passes its
     * agent name. An `OTEL_SERVICE_NAME` env value, if set, takes precedence.
     */
    readonly serviceName: string;
    /**
     * Register HTTP client auto-instrumentation (`@opentelemetry/instrumentation-undici`),
     * which injects `traceparent` per outbound request from the active span. Needed by
     * consumers whose outbound HTTP is **internal to a framework** and so has no hand-
     * inject call site — the eve agents' MCP client. temper-ui leaves this off and injects
     * at its own known call sites. Loaded via dynamic import so consumers that leave it off
     * never pull the instrumentation packages into their bundle. Default `false`.
     */
    readonly instrumentHttp?: boolean;
}
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
export declare function initTelemetry({ serviceName, instrumentHttp }: InitTelemetryOptions): void;
/** Whether span export is registered (endpoint was configured). */
export declare function isTelemetryEnabled(): boolean;
/** The tracer for this hop. A no-op tracer until {@link initTelemetry} registers a provider. */
export declare function getTracer(): Tracer;
/**
 * Force-export any queued spans. Never rejects — a flush failure is logged, not
 * propagated, so a telemetry hiccup can never turn into a request failure. A no-op
 * when export is disabled.
 */
export declare function forceFlush(): Promise<void>;
