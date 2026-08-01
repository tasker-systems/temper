import { defineInstrumentation } from "eve/instrumentation";
import { initTelemetry } from "temper-telemetry-ts";

/**
 * OTLP span export for the steward (and its declared auditor subagent).
 *
 * eve auto-discovers `agent/instrumentation.ts` and runs `setup` at server startup, before
 * any agent code — the framework's own bootstrap seam (no `NODE_OPTIONS --import` needed).
 * Its presence enables eve's telemetry, so eve exports its own `ai.eve.turn → ai.streamText →
 * ai.toolCall` spans to the provider we register here, and manages their flush. We point that
 * export at Grafana via the SAME `OTEL_EXPORTER_OTLP_*` env the Rust side uses, through the
 * shared `temper-telemetry-ts` (the TS analog of the `temper-telemetry` crate). No endpoint ⇒
 * no export (init is a no-op), mirroring the Rust rule.
 *
 * **`instrumentHttp: true`** registers undici HTTP auto-instrumentation. Unlike temper-ui,
 * the steward's outbound MCP `fetch` is INTERNAL to eve/the AI SDK (the MCP client bakes a
 * static header set at connect — see `connections/temper.ts`), so there is no hand-inject
 * call site. undici injects a per-request `traceparent` from the active `ai.toolCall` span,
 * which is what makes the steward → temper-mcp hop stitch onto a real, exported span. The
 * connections therefore drop their static `traceparent` when export is on (see
 * `lib/trace.ts::otlpExportConfigured`), so exactly one, correct `traceparent` is sent.
 *
 * **`recordInputs`/`recordOutputs: false`** — do NOT export full model message history or
 * outputs. The steward's model I/O carries team knowledge-base content; this extends PR #613's
 * 2a decision (no cross-linkable identifiers on exported spans) from span attributes to model
 * I/O. Task `019fbf24` §Item-2 spike.
 */
export default defineInstrumentation({
  recordInputs: false,
  recordOutputs: false,
  setup: ({ agentName }) => {
    initTelemetry({ serviceName: agentName, instrumentHttp: true });
  },
});
