import { createHash, randomBytes } from "node:crypto";

/**
 * A W3C `traceparent` for an outbound MCP call, grouping every call made within
 * one eve session under a single trace.
 *
 * temper-mcp extracts an inbound `traceparent` into its per-request root span
 * (temper-telemetry `request_span`, `record_inbound_trace_context`), so stamping
 * it here is what lets the steward's / auditor's tool calls be correlated with
 * the API + MCP logs they trigger — the cross-service join key the 2026-08-01
 * error-anomaly triage had to reconstruct by hand from timestamps across three
 * Vercel projects.
 *
 * - **trace-id is derived from the eve session id** (sha256 → first 16 bytes),
 *   so every MCP call in a session shares one trace-id and groups together
 *   without threading a new id through the model turn. The steward design keeps
 *   its `correlationId` server-side (dispatch → `kb_workflow_jobs` →
 *   `kb_invocations`), so no app-level id is in TS scope at MCP-call time; the
 *   session id is the one stable handle available at the connection layer.
 * - **span-id is fresh per call** — each outbound request is its own span.
 * - version `00`, sampled (`01`). A sha256 digest is never all-zero, so the
 *   trace-id is always W3C-valid.
 *
 * Interim honesty — the same stance `temper-ui`'s proxy and `temper-telemetry`'s
 * `propagate.rs` take: until the steward exports its own spans (`@vercel/otel`,
 * Tier 2), the span-id names no recorded span, so a receiver's *link* to it
 * dangles. It is a correlation handle first, a navigable span reference only
 * once the steward exports.
 */
export function makeTraceparent(sessionId: string): string {
  const traceId = createHash("sha256").update(sessionId).digest("hex").slice(0, 32);
  const spanId = randomBytes(8).toString("hex");
  return `00-${traceId}-${spanId}-01`;
}

/**
 * Whether OTLP span export is configured for this process (an endpoint is set).
 *
 * When it is, `agent/instrumentation.ts` enables undici HTTP auto-instrumentation, which
 * injects a per-request `traceparent` naming a real exported CLIENT span (a child of the
 * active `ai.toolCall` span). A connection that *also* stamps a static {@link makeTraceparent}
 * header would put a SECOND `traceparent` on the wire — undici appends via `request.addHeader`,
 * and temper-mcp reads the first, static one, so its post-auth link would dangle at a span that
 * does not exist. So connections omit the static header when this is true, and keep it only when
 * export is off — where it remains the cross-service log-correlation handle (PR #611).
 *
 * The gate mirrors `temper-telemetry-ts`'s `initTelemetry` (both key on
 * `OTEL_EXPORTER_OTLP_ENDPOINT`), so the "do we export?" decision is the same on both sides.
 */
export function otlpExportConfigured(): boolean {
  return Boolean(process.env.OTEL_EXPORTER_OTLP_ENDPOINT?.trim());
}
