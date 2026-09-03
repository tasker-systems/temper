import { describe, expect, it } from 'vitest';
import { NEVER_RECORD_MODEL_IO } from 'temper-telemetry-ts';
import instrumentation from "../agent/instrumentation.js";

/**
 * The AI SDK's `experimental_telemetry` defaults `recordInputs`/`recordOutputs` to
 * `true` — full model message history, exported. The steward's model I/O carries team
 * knowledge-base content, so the pin to `false` is load-bearing and this test is its
 * witness: an agent whose `instrumentation.ts` drops the spread (or a future agent
 * scaffolded without it) fails here rather than exporting message history.
 */
describe('instrumentation', () => {
	it('pins model I/O recording off — the AI SDK default is true, so omission exports', () => {
		expect(instrumentation.recordInputs).toBe(false);
		expect(instrumentation.recordOutputs).toBe(false);
	});

	it('pins come from the shared NEVER_RECORD_MODEL_IO constant, not an inline copy', () => {
		expect(instrumentation.recordInputs).toBe(NEVER_RECORD_MODEL_IO.recordInputs);
		expect(instrumentation.recordOutputs).toBe(NEVER_RECORD_MODEL_IO.recordOutputs);
		expect(NEVER_RECORD_MODEL_IO).toEqual({ recordInputs: false, recordOutputs: false });
	});
});
