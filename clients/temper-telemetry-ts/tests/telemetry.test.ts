import { context, propagation, TraceFlags, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { BatchSpanProcessor, SamplingDecision } from '@opentelemetry/sdk-trace-base';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { activeTraceparent, extractContext } from '../src/context.js';
import { McpNegotiationStatusProcessor } from '../src/mcp-negotiation.js';
import {
	initTelemetry,
	isSdkDisabled,
	isTelemetryEnabled,
	otlpSpanProcessors,
	shouldExportSpans,
	telemetrySampler
} from '../src/otel.js';

const TRACE_ID = '0af7651916cd43dd8448eb211c80319c';
const SPAN_ID = 'b7ad6b7169203331';

describe('activeTraceparent', () => {
	let cm: AsyncHooksContextManager;

	beforeEach(() => {
		context.disable();
		cm = new AsyncHooksContextManager().enable();
		context.setGlobalContextManager(cm);
	});

	afterEach(() => {
		context.disable();
		cm.disable();
	});

	it('returns null when there is no active span', () => {
		expect(activeTraceparent()).toBeNull();
	});

	it('formats the active span context as a sampled W3C traceparent', () => {
		const span = trace.wrapSpanContext({
			traceId: TRACE_ID,
			spanId: SPAN_ID,
			traceFlags: TraceFlags.SAMPLED,
			isRemote: false
		});
		const tp = context.with(trace.setSpan(context.active(), span), () => activeTraceparent());
		expect(tp).toBe(`00-${TRACE_ID}-${SPAN_ID}-01`);
	});

	it('marks an unsampled active span with -00 flags', () => {
		const span = trace.wrapSpanContext({
			traceId: TRACE_ID,
			spanId: SPAN_ID,
			traceFlags: TraceFlags.NONE,
			isRemote: false
		});
		const tp = context.with(trace.setSpan(context.active(), span), () => activeTraceparent());
		expect(tp).toBe(`00-${TRACE_ID}-${SPAN_ID}-00`);
	});
});

describe('extractContext', () => {
	beforeEach(() => {
		propagation.setGlobalPropagator(new W3CTraceContextPropagator());
	});

	afterEach(() => {
		propagation.disable();
	});

	it('extracts an inbound traceparent as a remote parent', () => {
		const headers = new Headers({ traceparent: `00-${TRACE_ID}-${SPAN_ID}-01` });
		const sc = trace.getSpanContext(extractContext(headers));
		expect(sc?.traceId).toBe(TRACE_ID);
		expect(sc?.spanId).toBe(SPAN_ID);
		expect(sc?.isRemote).toBe(true);
	});

	it('yields no parent span context when the request carries no traceparent', () => {
		expect(trace.getSpanContext(extractContext(new Headers()))).toBeUndefined();
	});
});

describe('initTelemetry', () => {
	it('is a no-op and stays disabled when no OTLP endpoint is configured', () => {
		const prev = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
		delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
		try {
			expect(() => initTelemetry({ serviceName: 'test-service' })).not.toThrow();
			expect(isTelemetryEnabled()).toBe(false);
		} finally {
			if (prev !== undefined) process.env.OTEL_EXPORTER_OTLP_ENDPOINT = prev;
		}
	});

	it('the OTEL_SDK_DISABLED kill switch outranks a configured endpoint', () => {
		const prevDisabled = process.env.OTEL_SDK_DISABLED;
		const prevEndpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
		process.env.OTEL_SDK_DISABLED = 'true';
		process.env.OTEL_EXPORTER_OTLP_ENDPOINT = 'http://localhost:4318';
		try {
			expect(() => initTelemetry({ serviceName: 'test-service' })).not.toThrow();
			expect(isTelemetryEnabled()).toBe(false);
		} finally {
			restore(prevDisabled, 'OTEL_SDK_DISABLED');
			restore(prevEndpoint, 'OTEL_EXPORTER_OTLP_ENDPOINT');
		}
	});
});

describe('initTelemetry refuses a plaintext collector', () => {
	const VARIABLES = ['OTEL_EXPORTER_OTLP_ENDPOINT', 'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT'] as const;
	let saved: Record<string, string | undefined>;

	beforeEach(() => {
		saved = {};
		for (const name of VARIABLES) saved[name] = process.env[name];
		for (const name of VARIABLES) delete process.env[name];
	});

	afterEach(() => {
		for (const name of VARIABLES) restore(saved[name], name);
		vi.restoreAllMocks();
	});

	// FAILS IF: initTelemetry builds an exporter for a plaintext non-loopback endpoint (the
	// collector credential in OTEL_EXPORTER_OTLP_HEADERS would ride it), throws instead of
	// degrading, or names the endpoint's value — which can carry userinfo — in the refusal.
	it.each([
		['OTEL_EXPORTER_OTLP_ENDPOINT', 'http://user:tok3n@collector.example.com:4318'],
		['OTEL_EXPORTER_OTLP_TRACES_ENDPOINT', 'http://collector.example.com/v1/traces']
	])('%s=%s: export stays off, the refusal names the variable', (name, value) => {
		process.env[name] = value;
		const error = vi.spyOn(console, 'error').mockImplementation(() => {});

		expect(() => initTelemetry({ serviceName: 'test-service' })).not.toThrow();
		expect(isTelemetryEnabled()).toBe(false);
		expect(error).toHaveBeenCalledTimes(1);
		const message = String(error.mock.calls[0]?.[0]);
		expect(message).toContain(name);
		expect(message).not.toContain('tok3n');
	});
});

describe('otlpSpanProcessors', () => {
	const VARIABLES = [
		'OTEL_SDK_DISABLED',
		'OTEL_EXPORTER_OTLP_ENDPOINT',
		'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT'
	] as const;
	let saved: Record<string, string | undefined>;

	beforeEach(() => {
		saved = {};
		for (const name of VARIABLES) saved[name] = process.env[name];
		for (const name of VARIABLES) delete process.env[name];
		vi.spyOn(console, 'info').mockImplementation(() => {});
	});

	afterEach(() => {
		for (const name of VARIABLES) restore(saved[name], name);
		vi.restoreAllMocks();
	});

	// FAILS IF: the eve-side factory exports where `initTelemetry` would not — the same three
	// "off" resolutions must yield no processor at all.
	it('is empty when no endpoint is configured', () => {
		expect(otlpSpanProcessors({ mcpEndpoint: 'https://temperkb.io/mcp' })).toEqual([]);
	});

	it('is empty under the kill switch, even with an endpoint', () => {
		process.env.OTEL_SDK_DISABLED = 'true';
		process.env.OTEL_EXPORTER_OTLP_ENDPOINT = 'http://localhost:4318';
		expect(otlpSpanProcessors()).toEqual([]);
	});

	it('is empty for a refused plaintext endpoint', () => {
		process.env.OTEL_EXPORTER_OTLP_ENDPOINT = 'http://collector.example.com:4318';
		vi.spyOn(console, 'error').mockImplementation(() => {});
		expect(otlpSpanProcessors()).toEqual([]);
	});

	// FAILS IF: the negotiation reset is placed AFTER the exporter — under eve only `onEnd` runs,
	// in list order, so it must come first — or if the factory registers a provider of its own,
	// which would make eve refuse to start.
	it('puts the MCP negotiation reset ahead of the batching exporter, and registers nothing', () => {
		process.env.OTEL_EXPORTER_OTLP_ENDPOINT = 'http://localhost:4318';
		const processors = otlpSpanProcessors({ mcpEndpoint: 'https://temperkb.io/mcp' });
		expect(processors).toHaveLength(2);
		expect(processors[0]).toBeInstanceOf(McpNegotiationStatusProcessor);
		expect(processors[1]).toBeInstanceOf(BatchSpanProcessor);
		expect(isTelemetryEnabled()).toBe(false);
	});

	it('is just the exporter without an MCP endpoint', () => {
		process.env.OTEL_EXPORTER_OTLP_ENDPOINT = 'http://localhost:4318';
		const processors = otlpSpanProcessors();
		expect(processors).toHaveLength(1);
		expect(processors[0]).toBeInstanceOf(BatchSpanProcessor);
	});
});

describe('isSdkDisabled', () => {
	const VARIABLES = ['OTEL_SDK_DISABLED', 'OTEL_EXPORTER_OTLP_ENDPOINT'] as const;
	let saved: Record<string, string | undefined>;

	beforeEach(() => {
		saved = {};
		for (const name of VARIABLES) saved[name] = process.env[name];
		delete process.env.OTEL_SDK_DISABLED;
		delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
	});

	afterEach(() => {
		for (const name of VARIABLES) restore(saved[name], name);
	});

	// The value discipline is the point, and it mirrors the Rust exporter exactly: the
	// spec names exactly one true value, so a typo (`1`, `yes`, `TRUE `) must leave
	// observability on rather than silently off.
	it.each([
		['true', true],
		['TRUE', true],
		['True', true],
		[' true ', true],
		['1', false],
		['yes', false],
		['false', false],
		['', false],
		['tru', false]
	])('OTEL_SDK_DISABLED=%j → %j', (value, expected) => {
		process.env.OTEL_SDK_DISABLED = value;
		expect(isSdkDisabled()).toBe(expected);
	});

	it('is false when the variable is unset', () => {
		expect(isSdkDisabled()).toBe(false);
	});
});

function restore(prev: string | undefined, name: string): void {
	if (prev === undefined) delete process.env[name];
	else process.env[name] = prev;
}

describe('shouldExportSpans', () => {
	const VARIABLES = [
		'OTEL_SDK_DISABLED',
		'OTEL_EXPORTER_OTLP_ENDPOINT',
		'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT'
	] as const;
	let saved: Record<string, string | undefined>;

	beforeEach(() => {
		saved = {};
		for (const name of VARIABLES) saved[name] = process.env[name];
		for (const name of VARIABLES) delete process.env[name];
	});

	afterEach(() => {
		for (const name of VARIABLES) restore(saved[name], name);
	});

	it.each([
		// The signal-specific endpoint wins, then the general one.
		[
			{},
			false,
			'no endpoint configured — export must stay off, never default to localhost'
		],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'https://collector' }, true, undefined],
		[
			{ OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'https://collector' },
			true,
			'signal-specific endpoint alone is enough'
		],
		[
			{
				OTEL_EXPORTER_OTLP_ENDPOINT: 'https://metrics-only',
				OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'https://collector'
			},
			true,
			undefined
		],
		// The kill switch outranks every endpoint — that is what kill switch means.
		[
			{ OTEL_EXPORTER_OTLP_ENDPOINT: 'https://collector', OTEL_SDK_DISABLED: 'true' },
			false,
			undefined
		],
		[
			{
				OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'https://collector',
				OTEL_SDK_DISABLED: 'TRUE'
			},
			false,
			undefined
		],
		[
			{
				OTEL_EXPORTER_OTLP_ENDPOINT: 'https://collector',
				OTEL_SDK_DISABLED: '1'
			},
			true,
			"'1' is deliberately not a true value"
		],
		// Plaintext off loopback is refused: the exporter sends OTEL_EXPORTER_OTLP_HEADERS
		// (the collector's credential) and every span on it.
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://collector' }, false, 'plaintext off loopback'],
		[
			{
				OTEL_EXPORTER_OTLP_ENDPOINT: 'https://collector',
				OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'http://collector'
			},
			false,
			'the endpoint judged is the one the exporter uses — the signal-specific one'
		],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://foo.localhost:4318' }, false, '*.localhost may go to DNS'],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://localhost.:4318' }, false, 'localhost. may go to DNS'],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'not a url' }, false, 'unparseable'],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://localhost:4318' }, true, 'loopback http: local collector'],
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:4318' }, true, 'loopback http: local collector']
	])('env %j → %j', (env, expected, why) => {
		Object.assign(process.env, env);
		const result = shouldExportSpans();
		if (why) expect(result, why).toBe(expected);
		else expect(result).toBe(expected);
	});
});

describe('telemetrySampler', () => {
	// The decision under test: the sampler must NOT follow a remote parent's sampled
	// flag. Honoring it would hand any caller control of whether our server spans are
	// recorded — `...-01` forces export on flood traffic, `...-00` silently drops
	// spans whose ids other hops link to. The Rust exporter pins the same invariant.
	it('records a span whose remote parent arrived unsampled', () => {
		const remoteUnsampled = {
			traceId: TRACE_ID,
			spanId: SPAN_ID,
			traceFlags: TraceFlags.NONE,
			isRemote: true
		};
		const sampler = telemetrySampler();
		const decision = sampler.shouldSample(
			trace.setSpan(context.active(), trace.wrapSpanContext(remoteUnsampled)),
			TRACE_ID,
			'test-span',
			1, // SpanKind.SERVER
			{},
			[]
		);
		expect(decision.decision).toBe(SamplingDecision.RECORD_AND_SAMPLED);
	});
});
