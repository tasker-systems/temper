import { otel } from "eve/instrumentation/otel";
import {
  httpInstrumentations,
  NEVER_RECORD_MODEL_IO,
  shouldExportSpans,
  telemetrySampler,
} from "@tasker-systems/temper-telemetry-ts";

/**
 * Process-wide OpenTelemetry settings for the mention agent — the same shape as the steward's
 * (`../../../steward/agent/instrumentation/otel.ts`). The OTLP destination itself is `./otlp.ts`.
 *
 * eve (≥0.62) discovers `agent/instrumentation/` one provider per file — the flat
 * `agent/instrumentation.ts` this replaces fails `eve build` — and OWNS the tracer provider,
 * refusing to start if something else registered one first. So this no longer calls
 * `initTelemetry`; `temper-telemetry-ts` hands eve the pieces instead, and eve exports its own
 * agent spans through them.
 *
 * **`tracePolicy` — `NEVER_RECORD_MODEL_IO`** (`recordInputs`/`recordOutputs: false`) — this agent
 * reads a mentioning user's temper data under their own credential; do not export model I/O. Under
 * eve ≥0.62 the pin moved here from the instrumentation definition (whose `recordInputs`/
 * `recordOutputs` were removed), and it is a CEILING over every OpenTelemetry destination —
 * including Vercel Agent Runs, which eve enables by default on preview and production and which
 * otherwise records content for PUBLIC audiences, which a public Slack channel is. The constant is
 * still the one place the rule is decided, and each agent's test suite asserts the pin survived.
 * Task `019fbf24` §Item-2.
 *
 * **`instrumentations`** — undici auto-instrumentation, only when export is on (as `initTelemetry`
 * did), so the agent's outbound temper-mcp `fetch` (internal to the AI SDK, no hand-inject seam)
 * carries a per-request `traceparent` naming a real exported span. This agent's connection never
 * stamped a static `traceparent` (`connections/temper.ts` has no `headers`), so undici is
 * unambiguously the sole injector.
 *
 * **`sampler`** — always-on, deliberately ignoring an inbound `sampled` flag (`telemetrySampler`).
 */
export default otel({
  tracePolicy: () => ({ emit: true, ...NEVER_RECORD_MODEL_IO }),
  sampler: telemetrySampler(),
  instrumentations: shouldExportSpans() ? await httpInstrumentations() : [],
});
