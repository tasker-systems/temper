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
