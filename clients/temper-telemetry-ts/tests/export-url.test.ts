import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { afterAll, expect, it } from 'vitest';
import { forceFlush, getTracer, initTelemetry, isTelemetryEnabled } from '../src/otel.js';

// Its own file: initTelemetry registers once per module, and this test needs a successful
// registration that would otherwise leak into every other test in the process.

const received: string[] = [];
const collector = createServer((req, res) => {
	received.push(`${req.method} ${req.url}`);
	req.resume();
	req.on('end', () => res.writeHead(200).end());
});

afterAll(() => new Promise<void>((resolve) => collector.close(() => resolve())));

// FAILS IF: the URL initTelemetry hands the exporter is not the one the exporter itself would have
// derived from OTEL_EXPORTER_OTLP_ENDPOINT (base + `/v1/traces`). The exporter no longer resolves
// its own endpoint, so this construction is ours to get right.
it('exports to the general base plus /v1/traces', async () => {
	await new Promise<void>((resolve) => collector.listen(0, '127.0.0.1', resolve));
	const { port } = collector.address() as AddressInfo;
	const prev = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
	process.env.OTEL_EXPORTER_OTLP_ENDPOINT = `http://127.0.0.1:${port}/base`;
	try {
		initTelemetry({ serviceName: 'export-url-test' });
		expect(isTelemetryEnabled()).toBe(true);
		getTracer().startSpan('probe').end();
		await forceFlush();
		expect(received).toEqual(['POST /base/v1/traces']);
	} finally {
		if (prev === undefined) delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
		else process.env.OTEL_EXPORTER_OTLP_ENDPOINT = prev;
	}
});
