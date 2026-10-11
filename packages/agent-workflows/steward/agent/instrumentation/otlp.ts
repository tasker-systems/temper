import { otelIntegration } from "eve/instrumentation/otel";
import { otlpSpanProcessors } from "@tasker-systems/temper-telemetry-ts";

/**
 * OTLP span export to Grafana via the SAME `OTEL_EXPORTER_OTLP_*` env the Rust side uses, through
 * the shared `temper-telemetry-ts` (the TS analog of the `temper-telemetry` crate). No endpoint (or
 * `OTEL_SDK_DISABLED=true`, or a plaintext non-loopback endpoint) ⇒ no processors ⇒ no export,
 * mirroring the Rust rule. Process-wide settings — the model-I/O pin among them — are `./otel.ts`.
 *
 * **`mcpEndpoint`** is passed only so span export can tell the MCP client's SSE-negotiation
 * probe apart from a real failure. That probe is a `GET` the spec requires our Streamable-HTTP
 * endpoint to answer `405`, and undici marks every 4xx an error — 446 false error spans a day
 * from this agent alone, 77% of total system error volume. `mcp-negotiation.ts` explains what
 * stays visible. Read straight from the env rather than through `requireEnv` because a missing
 * value must degrade telemetry, never fail startup; the connections already require it.
 */
export default otelIntegration({
  spanProcessors: otlpSpanProcessors({ mcpEndpoint: process.env.TEMPER_MCP_URL }),
});
