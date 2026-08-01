/**
 * OpenTelemetry span-export bootstrap for the temper-ui Node hop.
 *
 * This is the SvelteKit-side counterpart of the Rust `temper-telemetry` crate: it
 * builds a `NodeTracerProvider` that self-exports spans over OTLP/protobuf to the
 * same Grafana Cloud endpoint the Rust functions use, driven by the SAME env vars
 * (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME`).
 *
 * Why native `@opentelemetry`, not `@vercel/otel`: `@vercel/otel` presumes Next.js;
 * temper-ui is SvelteKit (`@sveltejs/adapter-vercel`, nodejs22.x). Why OTLP/proto,
 * not JSON: same protocol as Rust, and it dodges the OTLP/JSON `TimeUnixNano`
 * encoding bug. See task `019fbf24`.
 *
 * **Deliberately generic.** Nothing here is SvelteKit-specific; the request-lifecycle
 * glue lives in `./request-span`. This split is so this module can be lifted verbatim
 * into a shared `temper-telemetry-ts` package (task §Shared) once the eve agents become
 * a second consumer — until then there is exactly one consumer and nothing to drift.
 */

import { type Tracer, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { BatchSpanProcessor, NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import { ATTR_SERVICE_NAME } from '@opentelemetry/semantic-conventions';

/** Instrumentation-scope name for the tracer this hop emits under. */
export const TRACER_NAME = 'temper-ui';

let provider: NodeTracerProvider | null = null;
let enabled = false;

/**
 * Build and register the tracer provider — **once**. Idempotent, so a repeated
 * side-effecting import (dev HMR, multiple entrypoints) does not double-register.
 *
 * Mirrors the Rust "no endpoint ⇒ no export" rule: when `OTEL_EXPORTER_OTLP_ENDPOINT`
 * is unset the provider is never built, span creation stays a no-op, and we never
 * default to `localhost:4318`. The exporter reads the endpoint and headers from the
 * standard env itself (it self-configures from `OTEL_EXPORTER_OTLP_*`), so the only
 * thing this function decides is *whether* to register.
 */
export function initTelemetry(): void {
	if (provider) return;

	const endpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT?.trim();
	if (!endpoint) {
		console.info('[telemetry] OTEL_EXPORTER_OTLP_ENDPOINT unset; span export disabled');
		return;
	}

	// `OTEL_SERVICE_NAME` is free on this project because the Rust functions self-name
	// in code (PR #613, 2b). Fall back to the hop's own name if it is somehow unset.
	const serviceName = process.env.OTEL_SERVICE_NAME?.trim() || TRACER_NAME;

	// No `url`/`headers` passed: the OTLP/proto exporter reads
	// `OTEL_EXPORTER_OTLP_ENDPOINT` + `OTEL_EXPORTER_OTLP_HEADERS` natively, so config
	// stays in one place (env) shared with the Rust side.
	const exporter = new OTLPTraceExporter();

	const built = new NodeTracerProvider({
		resource: resourceFromAttributes({ [ATTR_SERVICE_NAME]: serviceName }),
		// BatchSpanProcessor + per-request forceFlush is the JS mirror of the Rust
		// `flush_within_budget` — see `./request-span`. The batch timer alone is unsafe
		// on Vercel: the sandbox freezes between invocations and the timer may never fire.
		spanProcessors: [new BatchSpanProcessor(exporter)],
	});

	built.register({
		contextManager: new AsyncHooksContextManager().enable(),
		propagator: new W3CTraceContextPropagator(),
	});

	provider = built;
	enabled = true;
	console.info(`[telemetry] span export enabled: service.name=${serviceName} → ${endpoint}`);
}

/** Whether span export is registered (endpoint was configured). */
export function isTelemetryEnabled(): boolean {
	return enabled;
}

/** The tracer for this hop. A no-op tracer until {@link initTelemetry} registers a provider. */
export function getTracer(): Tracer {
	return trace.getTracer(TRACER_NAME);
}

/**
 * Force-export any queued spans. Never rejects — a flush failure is logged, not
 * propagated, so a telemetry hiccup can never turn into a request failure. A no-op
 * when export is disabled.
 */
export async function forceFlush(): Promise<void> {
	if (!provider) return;
	try {
		await provider.forceFlush();
	} catch (err) {
		console.error('[telemetry] forceFlush failed', err);
	}
}
