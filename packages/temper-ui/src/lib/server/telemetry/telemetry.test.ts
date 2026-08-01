import { context, propagation, TraceFlags, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { activeTraceparent, extractContext } from './context';
import { initTelemetry, isTelemetryEnabled } from './otel';

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
			isRemote: false,
		});
		const tp = context.with(trace.setSpan(context.active(), span), () => activeTraceparent());
		expect(tp).toBe(`00-${TRACE_ID}-${SPAN_ID}-01`);
	});

	it('marks an unsampled active span with -00 flags', () => {
		const span = trace.wrapSpanContext({
			traceId: TRACE_ID,
			spanId: SPAN_ID,
			traceFlags: TraceFlags.NONE,
			isRemote: false,
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
			expect(() => initTelemetry()).not.toThrow();
			expect(isTelemetryEnabled()).toBe(false);
		} finally {
			if (prev !== undefined) process.env.OTEL_EXPORTER_OTLP_ENDPOINT = prev;
		}
	});
});
