import { otel } from "eve/instrumentation/otel";
import {
  httpInstrumentations,
  NEVER_RECORD_MODEL_IO,
  shouldExportSpans,
  telemetrySampler,
} from "@tasker-systems/temper-telemetry-ts";

/**
 * Process-wide OpenTelemetry settings for the steward (and its declared auditor subagent).
 * The OTLP destination itself is `./otlp.ts`.
 *
 * eve auto-discovers `agent/instrumentation/` (one provider per file; the flat
 * `agent/instrumentation.ts` this replaces fails `eve build` since eve 0.62) and OWNS the tracer
 * provider: it registers one pipeline from every file here, and refuses to start if something
 * else registered a global provider first. That is why this no longer calls `initTelemetry` —
 * the shared `temper-telemetry-ts` now hands eve the pieces (`otlpSpanProcessors`,
 * `httpInstrumentations`, `telemetrySampler`) instead of registering them itself. eve exports its
 * own `invoke_agent → agent.step → chat / execute_tool` spans through that pipeline and manages
 * their flush.
 *
 * **`tracePolicy` — `NEVER_RECORD_MODEL_IO`** (`recordInputs`/`recordOutputs: false`) — do NOT
 * export model message history or outputs. The steward's model I/O carries team knowledge-base
 * content; this extends PR #613's 2a decision (no cross-linkable identifiers on exported spans)
 * from span attributes to model I/O. Under eve ≥0.62 the setting moved here from the instrumentation
 * definition (the old top-level `recordInputs`/`recordOutputs` were removed), and it is now a
 * CEILING over every OpenTelemetry destination — including Vercel Agent Runs, which eve enables by
 * default on preview and production and which otherwise records content for public audiences. The
 * constant is still the one place the rule is decided, and `tests/instrumentation.test.ts` asserts
 * the pin survived. Task `019fbf24` §Item-2 spike.
 *
 * **`instrumentations` — undici HTTP auto-instrumentation, only when export is on.** The steward's
 * outbound MCP `fetch` is INTERNAL to eve/the AI SDK, so there is no hand-inject call site. undici
 * injects a per-request `traceparent` from the active tool span, which is what makes the steward →
 * temper-mcp hop stitch onto a real, exported span. The connections drop their static `traceparent`
 * exactly when export is on (`lib/trace.ts::otlpExportConfigured`, the same `shouldExportSpans`), so
 * gating the instrumentation on the same predicate keeps exactly one `traceparent` on the wire.
 *
 * **`sampler` — always-on**, deliberately ignoring an inbound `sampled` flag, as `initTelemetry` did
 * (see `telemetrySampler`). It never thins: eve's default would otherwise follow a remote parent.
 */
export default otel({
  tracePolicy: () => ({ emit: true, ...NEVER_RECORD_MODEL_IO }),
  sampler: telemetrySampler(),
  instrumentations: shouldExportSpans() ? await httpInstrumentations() : [],
});
