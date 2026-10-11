import { describe, expect, it } from 'vitest';
import { NEVER_RECORD_MODEL_IO } from '@tasker-systems/temper-telemetry-ts';
import otelSettings from "../agent/instrumentation/otel.js";

/**
 * The AI SDK's `experimental_telemetry` defaults `recordInputs`/`recordOutputs` to
 * `true` — full model message history, exported — and eve's own OpenTelemetry default
 * records content for public audiences, which a public Slack channel is. The mention agent
 * reads a mentioning user's temper data under that user's own credential, so the pin to
 * `false` is load-bearing and this test is its witness: an agent whose
 * `instrumentation/otel.ts` drops the spread fails here rather than exporting message history.
 *
 * Since eve 0.62 the pin is a `tracePolicy` decision on `otel()` — the old top-level
 * `recordInputs`/`recordOutputs` on the instrumentation definition were removed — so the
 * assertions read the decision the policy returns, for every audience it can be asked about.
 */
const policy = otelSettings.options.tracePolicy;
const CONTEXTS = (["public", "private", "unknown"] as const).flatMap((audience) =>
	(["development", "preview", "production"] as const).map(
		(environment) => ({ agentName: "mention", audience, environment }) as never,
	),
);

describe('instrumentation', () => {
	it('pins model I/O recording off — eve and the AI SDK default to recording, so omission exports', () => {
		expect(policy).toBeTypeOf('function');
		for (const context of CONTEXTS) {
			expect(policy?.(context)).toMatchObject({ emit: true, recordInputs: false, recordOutputs: false });
		}
	});

	it('pins come from the shared NEVER_RECORD_MODEL_IO constant, not an inline copy', () => {
		for (const context of CONTEXTS) {
			const decision = policy?.(context) as { recordInputs: boolean; recordOutputs: boolean };
			expect(decision.recordInputs).toBe(NEVER_RECORD_MODEL_IO.recordInputs);
			expect(decision.recordOutputs).toBe(NEVER_RECORD_MODEL_IO.recordOutputs);
		}
		expect(NEVER_RECORD_MODEL_IO).toEqual({ recordInputs: false, recordOutputs: false });
	});
});
