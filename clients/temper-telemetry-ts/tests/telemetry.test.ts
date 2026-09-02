import { context, propagation, TraceFlags, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { SamplingDecision } from '@opentelemetry/sdk-trace-base';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { activeTraceparent, extractContext } from '../src/context.js';
import {
	initTelemetry,
	isSdkDisabled,
	isTelemetryEnabled,
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
		[{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://collector' }, true, undefined],
		[
			{ OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'http://collector' },
			true,
			'signal-specific endpoint alone is enough'
		],
		[
			{
				OTEL_EXPORTER_OTLP_ENDPOINT: 'http://metrics-only',
				OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'http://collector'
			},
			true,
			undefined
		],
		// The kill switch outranks every endpoint — that is what kill switch means.
		[
			{ OTEL_EXPORTER_OTLP_ENDPOINT: 'http://collector', OTEL_SDK_DISABLED: 'true' },
			false,
			undefined
		],
		[
			{
				OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'http://collector',
				OTEL_SDK_DISABLED: 'TRUE'
			},
			false,
			undefined
		],
		[
			{
				OTEL_EXPORTER_OTLP_ENDPOINT: 'http://collector',
				OTEL_SDK_DISABLED: '1'
			},
			true,
			"'1' is deliberately not a true value"
		]
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
