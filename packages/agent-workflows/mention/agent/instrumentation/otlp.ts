import { otelIntegration } from "eve/instrumentation/otel";
import { otlpSpanProcessors } from "@tasker-systems/temper-telemetry-ts";

/**
 * OTLP span export to Grafana via the shared `temper-telemetry-ts` and the standard
 * `OTEL_EXPORTER_OTLP_*` env. No endpoint ⇒ no processors ⇒ no export. Process-wide settings —
 * the model-I/O pin among them — are `./otel.ts`.
 *
 * **`mcpEndpoint`** stops the MCP client's SSE-negotiation probe — a `GET` the spec requires our
 * Streamable-HTTP endpoint to answer `405`, which undici then marks an error — from exporting as
 * a failure. Same reasoning and same seam as the steward's; the suppression is keyed on the
 * response shape and the endpoint, never on which agent made the call, so it covers every MCP
 * client we run. See `mcp-negotiation.ts`.
 */
export default otelIntegration({
  spanProcessors: otlpSpanProcessors({ mcpEndpoint: process.env.TEMPER_MCP_URL }),
});
