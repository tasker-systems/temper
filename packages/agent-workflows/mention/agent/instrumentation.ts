import { defineInstrumentation } from "eve/instrumentation";
import { initTelemetry } from "temper-telemetry-ts";

/**
 * OTLP span export for the mention agent — the same shape as the steward's
 * (`../../steward/agent/instrumentation.ts`), pointed at Grafana via the shared
 * `temper-telemetry-ts` and the standard `OTEL_EXPORTER_OTLP_*` env. No endpoint ⇒ no export.
 *
 * eve runs `setup` at startup and, with this file present, exports its own `ai.*` spans to the
 * provider we register. **`instrumentHttp: true`** enables undici auto-instrumentation so the
 * agent's outbound temper-mcp `fetch` (internal to the AI SDK, no hand-inject seam) carries a
 * per-request `traceparent` naming a real exported span. Unlike the steward, this agent's
 * connection never stamped a static `traceparent` (`connections/temper.ts` has no `headers`),
 * so there is nothing to omit — undici is unambiguously the sole injector.
 *
 * **`mcpEndpoint`** stops the MCP client's SSE-negotiation probe — a `GET` the spec requires our
 * Streamable-HTTP endpoint to answer `405`, which undici then marks an error — from exporting as
 * a failure. Same reasoning and same seam as the steward's; the suppression is keyed on the
 * response shape and the endpoint, never on which agent made the call, so it covers every MCP
 * client we run. See `mcp-negotiation.ts`.
 *
 * **`recordInputs`/`recordOutputs: false`** — this agent reads a mentioning user's temper data
 * under their own credential; do not export model I/O to Grafana. Same reasoning as the
 * steward. Task `019fbf24` §Item-2.
 */
export default defineInstrumentation({
  recordInputs: false,
  recordOutputs: false,
  setup: ({ agentName }) => {
    initTelemetry({
      serviceName: agentName,
      instrumentHttp: true,
      mcpEndpoint: process.env.TEMPER_MCP_URL,
    });
  },
});
