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
 *
 * `OTEL_SDK_DISABLED` is honored here with the Rust exporter's exact semantics
 * (`temper-telemetry/src/export.rs`): the kill switch outranks a configured endpoint,
 * and only the literal value `true` — case-insensitive, surrounding whitespace
 * tolerated — disables. `1` and `yes` deliberately do not: the spec names exactly one
 * true value, and guessing at others would let a typo silently disable observability.
 */

import { trace, type Tracer } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { resourceFromAttributes } from '@opentelemetry/resources';
import type { Sampler, SpanProcessor } from '@opentelemetry/sdk-trace-base';
import { AlwaysOnSampler, BatchSpanProcessor, NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import { ATTR_SERVICE_NAME } from '@opentelemetry/semantic-conventions';
import { McpNegotiationStatusProcessor, negotiationKey } from './mcp-negotiation.js';

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
	/**
	 * The MCP endpoint this hop talks to (`TEMPER_MCP_URL`), when it runs an MCP client. Its
	 * only effect is to install {@link McpNegotiationStatusProcessor}, which stops the
	 * spec-mandated `GET` → `405` SSE-negotiation response from being exported as an error
	 * span — see `mcp-negotiation.ts` for why that response exists and what stays visible.
	 *
	 * Passed in rather than read from the environment here, and never defaulted to a hostname,
	 * because a self-hosted deployment's MCP endpoint is not `temperkb.io` — the suppression
	 * has to follow the endpoint the client was actually pointed at. An unparseable value is
	 * reported and ignored; telemetry config never fails a startup. Omit for hops with no MCP
	 * client (temper-ui).
	 */
	readonly mcpEndpoint?: string;
}

let provider: NodeTracerProvider | null = null;
let enabled = false;
let tracerName = 'temper-telemetry-ts';

/**
 * The `OTEL_SDK_DISABLED` kill switch, with the Rust exporter's exact semantics:
 * only the literal `true` — case-insensitive, surrounding whitespace tolerated —
 * disables. `1`, `yes`, and typos leave export on; see the module comment.
 */
export function isSdkDisabled(): boolean {
	return isSdkDisabledFrom(process.env);
}

/**
 * Whether span export should run, from the environment alone. The ONE decision —
 * `initTelemetry` and the agents' `otlpExportConfigured` both delegate here, so the
 * "do we export?" answer cannot drift between building the provider and deciding
 * whether to inject a static traceparent.
 *
 * Mirrors the Rust exporter: `OTEL_SDK_DISABLED` (see {@link isSdkDisabled}) is the
 * kill switch and outranks any endpoint; otherwise the signal-specific
 * `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` wins over the general
 * `OTEL_EXPORTER_OTLP_ENDPOINT` — the same precedence the Rust exporter and the
 * OTLP exporter itself apply, so a signal-specific endpoint turns every hop on or
 * none.
 */
export function shouldExportSpans(env: NodeJS.ProcessEnv = process.env): boolean {
	return resolveExport(env).kind === 'export';
}

/** What the environment says about span export — the one resolution both callers share. */
type ExportResolution =
	| { readonly kind: 'disabled' }
	| { readonly kind: 'unset' }
	| { readonly kind: 'refused'; readonly reason: string }
	| {
			readonly kind: 'export';
			readonly variable: string;
			readonly host: string;
			/** The exact traces URL, handed to the exporter so it never resolves its own. */
			readonly url: string;
	  };

/**
 * Resolve span export from the environment. Signal-specific endpoint first, as the exporter
 * reads it. A configured endpoint is **refused** — export off, never a startup failure — when
 * it is plaintext http to anything but this machine, or does not parse: the exporter sends
 * `OTEL_EXPORTER_OTLP_HEADERS` (the collector's credential) and every span on it. Mirrors the
 * Rust exporter's refusal (`temper-telemetry/src/export.rs`, `vet_endpoint`).
 *
 * What is vetted is the final traces URL, built as the exporter would build it (the
 * signal-specific value as-is, or the general base plus `v1/traces`), and `initTelemetry` passes
 * that URL to the exporter explicitly. Left to its own env reading, the exporter falls back to
 * the general variable when the signal-specific one fails to parse — a URL nobody vetted.
 */
function resolveExport(env: NodeJS.ProcessEnv): ExportResolution {
	if (isSdkDisabledFrom(env)) return { kind: 'disabled' };
	const variable = OTLP_ENDPOINT_VARS.find((name) => env[name]?.trim());
	if (!variable) return { kind: 'unset' };
	const value = env[variable]!.trim();
	let url: URL;
	try {
		url = new URL(
			variable === 'OTEL_EXPORTER_OTLP_ENDPOINT'
				? `${value}${value.endsWith('/') ? '' : '/'}v1/traces`
				: value
		);
	} catch {
		// The value is not echoed: it can carry userinfo.
		return { kind: 'refused', reason: `${variable} is not a parseable URL; span export disabled` };
	}
	if (url.protocol !== 'https:' && !(url.protocol === 'http:' && isLoopbackHost(url.hostname))) {
		return {
			kind: 'refused',
			reason:
				`${variable} is not https (plaintext http is accepted only for localhost, 127.0.0.0/8 ` +
				'and [::1]), so OTEL_EXPORTER_OTLP_HEADERS and every span would cross in the clear; ' +
				'span export disabled'
		};
	}
	return { kind: 'export', variable, host: url.host, url: url.href };
}

/** Signal-specific first, the precedence the OTLP exporter itself applies. */
const OTLP_ENDPOINT_VARS = ['OTEL_EXPORTER_OTLP_TRACES_ENDPOINT', 'OTEL_EXPORTER_OTLP_ENDPOINT'] as const;

/**
 * Whether `hostname` (as `URL` hands it over: lowercased, IPv6 bracketed) is known to stay on
 * this machine: `localhost`, 127.0.0.0/8, `[::1]`. Deliberately NOT `*.localhost` — on a server
 * runtime glibc's resolver sends `foo.localhost` to DNS. A local copy rather than temper-ts's
 * `isLoopback`, which this package does not depend on and which accepts `*.localhost`.
 */
function isLoopbackHost(hostname: string): boolean {
	const host = hostname.replace(/\.$/, '');
	return host === 'localhost' || host === '[::1]' || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host);
}

function isSdkDisabledFrom(env: NodeJS.ProcessEnv): boolean {
	return env.OTEL_SDK_DISABLED?.trim().toLowerCase() === 'true';
}

/**
 * The sampler every temper hop exports with. **Always-on, deliberately ignoring the
 * inbound `sampled` flag**: honoring a remote parent's flag would hand any caller
 * control of whether our server spans are recorded — the sampling cost attack the
 * Rust side pins as a violation (`temper-telemetry/src/export.rs`, the
 * `sampled_flag_is_recorded_never_obeyed` test). Trace-id stitching survives via the
 * remote parent; the sampling decision does not follow it.
 */
export function telemetrySampler(): Sampler {
	return new AlwaysOnSampler();
}

/**
 * Build and register the tracer provider — **once**. Idempotent, so a repeated
 * side-effecting call (dev HMR, multiple entrypoints) does not double-register.
 *
 * Mirrors the Rust "no endpoint ⇒ no export" rule: when no OTLP endpoint is configured
 * the provider is never built, span creation stays a no-op, and we never
 * default to `localhost:4318`. An endpoint that is not https off loopback is refused the same
 * way (see `resolveExport`). The exporter is handed the vetted traces URL and reads the headers
 * from the standard env itself.
 */
export function initTelemetry({
	serviceName,
	instrumentHttp = false,
	mcpEndpoint
}: InitTelemetryOptions): void {
	if (provider) return;

	const resolution = resolveExport(process.env);
	switch (resolution.kind) {
		case 'disabled':
			console.info('[telemetry] OTEL_SDK_DISABLED=true; span export disabled');
			return;
		case 'unset':
			console.info('[telemetry] no OTLP endpoint configured; span export disabled');
			return;
		case 'refused':
			console.error(`[telemetry] ${resolution.reason}`);
			return;
	}

	// An `OTEL_SERVICE_NAME` env value (project-scoped on Vercel) wins over the passed
	// name; otherwise the consumer's name is authoritative.
	const resolvedServiceName = process.env.OTEL_SERVICE_NAME?.trim() || serviceName;
	tracerName = resolvedServiceName;

	// `url` is the vetted one, so the exporter never resolves an endpoint of its own (see
	// `resolveExport`); headers stay in env (`OTEL_EXPORTER_OTLP_HEADERS`), shared with the Rust side.
	const exporter = new OTLPTraceExporter({ url: resolution.url });

	const spanProcessors: SpanProcessor[] = [];

	// Runs ahead of the exporting processor for readability only — it acts in `onEnding`,
	// which fires before any processor's `onEnd`, so the outcome does not depend on order.
	if (mcpEndpoint) {
		const key = negotiationKey(mcpEndpoint);
		if (key) {
			spanProcessors.push(new McpNegotiationStatusProcessor(key));
		} else {
			console.warn(
				`[telemetry] mcpEndpoint is not a URL (${mcpEndpoint}); ` +
					'MCP negotiation 405s will export as errors'
			);
		}
	}

	// BatchSpanProcessor + a per-request flush (the consumer's job) is the JS mirror of
	// the Rust `flush_within_budget`. The batch timer alone is unsafe on Vercel: the
	// sandbox freezes between invocations and the timer may never fire.
	spanProcessors.push(new BatchSpanProcessor(exporter));

	const built = new NodeTracerProvider({
		resource: resourceFromAttributes({ [ATTR_SERVICE_NAME]: resolvedServiceName }),
		sampler: telemetrySampler(),
		spanProcessors
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

	console.info(
		`[telemetry] span export enabled: service.name=${resolvedServiceName} → ${resolution.host} (${resolution.variable})` +
			(instrumentHttp ? ' (+http instrumentation)' : '') +
			(spanProcessors.length > 1 ? ` (+mcp negotiation status reset for ${mcpEndpoint})` : '')
	);
}

async function enableHttpInstrumentation(): Promise<void> {
	try {
		const [{ registerInstrumentations }, { UndiciInstrumentation }] = await Promise.all([
			import('@opentelemetry/instrumentation'),
			import('@opentelemetry/instrumentation-undici')
		]);
		registerInstrumentations({ instrumentations: [new UndiciInstrumentation()] });
	} catch (err) {
		console.error('[telemetry] http instrumentation failed to register', err);
	}
}

/** Whether span export is registered (endpoint was configured). */
export function isTelemetryEnabled(): boolean {
	return enabled;
}

/** The tracer for this hop. A no-op tracer until {@link initTelemetry} registers a provider. */
export function getTracer(): Tracer {
	return trace.getTracer(tracerName);
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
