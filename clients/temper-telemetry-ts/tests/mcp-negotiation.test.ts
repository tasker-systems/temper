import { SpanKind, SpanStatusCode } from '@opentelemetry/api';
import {
	InMemorySpanExporter,
	type ReadableSpan,
	SimpleSpanProcessor
} from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import {
	ATTR_HTTP_REQUEST_METHOD,
	ATTR_HTTP_RESPONSE_STATUS_CODE,
	ATTR_URL_FULL
} from '@opentelemetry/semantic-conventions';
import { beforeEach, describe, expect, it } from 'vitest';
import { McpNegotiationStatusProcessor, negotiationKey } from '../src/mcp-negotiation.js';

const MCP_ENDPOINT = 'https://temperkb.io/mcp';
const ENDPOINT_KEY = 'https://temperkb.io/mcp';

let exporter: InMemorySpanExporter;
let provider: NodeTracerProvider;

/**
 * Drive a span through a real provider carrying the processor, exactly as `initTelemetry`
 * assembles it. Asserting the **exported** span rather than calling `onEnding` directly is the
 * point: it is what keeps the SDK's `@experimental` `onEnding` seam pinned. If a minor version
 * stops calling it, these tests go red instead of the dashboard quietly re-inverting.
 */
function exportClientSpan(attrs: {
	method: string;
	status: number;
	url: string;
}): ReadableSpan {
	const span = provider.getTracer('test').startSpan('HTTP GET', { kind: SpanKind.CLIENT });
	span.setAttributes({
		[ATTR_HTTP_REQUEST_METHOD]: attrs.method,
		[ATTR_HTTP_RESPONSE_STATUS_CODE]: attrs.status,
		[ATTR_URL_FULL]: attrs.url
	});
	// What `instrumentation-undici` does for any 4xx (`undici.js:310`), and the only reason
	// these spans read as failures at all.
	span.setStatus({ code: SpanStatusCode.ERROR });
	span.end();

	const finished = exporter.getFinishedSpans();
	expect(finished).toHaveLength(1);
	return finished[0];
}

beforeEach(() => {
	exporter = new InMemorySpanExporter();
	provider = new NodeTracerProvider({
		spanProcessors: [
			new McpNegotiationStatusProcessor(ENDPOINT_KEY),
			new SimpleSpanProcessor(exporter)
		]
	});
});

describe('negotiationKey', () => {
	it('reduces a URL to origin + path', () => {
		expect(negotiationKey('https://temperkb.io/mcp')).toBe('https://temperkb.io/mcp');
	});

	it('drops a trailing slash, query and fragment', () => {
		expect(negotiationKey('https://temperkb.io/mcp/?sessionId=abc#x')).toBe(
			'https://temperkb.io/mcp'
		);
	});

	it('keeps the port, since a self-hosted endpoint is commonly on one', () => {
		expect(negotiationKey('http://localhost:3000/mcp')).toBe('http://localhost:3000/mcp');
	});

	it('returns null for a non-URL rather than throwing', () => {
		expect(negotiationKey('not-a-url')).toBeNull();
		expect(negotiationKey('')).toBeNull();
	});
});

describe('McpNegotiationStatusProcessor', () => {
	it('clears the status of the spec-mandated GET → 405 against the MCP endpoint', () => {
		const span = exportClientSpan({ method: 'GET', status: 405, url: MCP_ENDPOINT });
		expect(span.status.code).toBe(SpanStatusCode.UNSET);
	});

	it('leaves the 405 attribute in place, so the round trip stays visible', () => {
		const span = exportClientSpan({ method: 'GET', status: 405, url: MCP_ENDPOINT });
		expect(span.attributes[ATTR_HTTP_RESPONSE_STATUS_CODE]).toBe(405);
	});

	it('matches through a query string, which the MCP client appends', () => {
		const span = exportClientSpan({
			method: 'GET',
			status: 405,
			url: `${MCP_ENDPOINT}?sessionId=abc`
		});
		expect(span.status.code).toBe(SpanStatusCode.UNSET);
	});

	it('leaves a different status against the MCP endpoint alone', () => {
		const span = exportClientSpan({ method: 'GET', status: 404, url: MCP_ENDPOINT });
		expect(span.status.code).toBe(SpanStatusCode.ERROR);
	});

	it('leaves a different method against the MCP endpoint alone', () => {
		const span = exportClientSpan({ method: 'POST', status: 405, url: MCP_ENDPOINT });
		expect(span.status.code).toBe(SpanStatusCode.ERROR);
	});

	it('leaves a GET → 405 against another host alone', () => {
		const span = exportClientSpan({
			method: 'GET',
			status: 405,
			url: 'https://vercel.com/api/v2/sandboxes'
		});
		expect(span.status.code).toBe(SpanStatusCode.ERROR);
	});

	it('leaves a GET → 405 against another path on the same host alone', () => {
		const span = exportClientSpan({
			method: 'GET',
			status: 405,
			url: 'https://temperkb.io/api/resources'
		});
		expect(span.status.code).toBe(SpanStatusCode.ERROR);
	});

	it('leaves a span carrying no URL attribute alone', () => {
		const span = provider.getTracer('test').startSpan('internal work');
		span.setAttributes({
			[ATTR_HTTP_REQUEST_METHOD]: 'GET',
			[ATTR_HTTP_RESPONSE_STATUS_CODE]: 405
		});
		span.setStatus({ code: SpanStatusCode.ERROR });
		span.end();

		expect(exporter.getFinishedSpans()[0].status.code).toBe(SpanStatusCode.ERROR);
	});
});
