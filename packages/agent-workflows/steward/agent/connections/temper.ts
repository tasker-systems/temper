import { connect } from "@vercel/connect/eve";
import { defineMcpClientConnection } from "eve/connections";
import { never } from "eve/tools/approval";

import { mintM2mToken, requireEnv } from "../lib/temper-auth.js";
import { makeTraceparent, otlpExportConfigured } from "../lib/trace.js";
import { STEWARD_TOOLS } from "../lib/tool-allowlists.js";

/**
 * The steward's sole seam to temper-mcp.
 *
 * - **URL is env-driven, never hardcoded** (`TEMPER_MCP_URL`) so one agent
 *   directory points at temperkb.io OR a self-hosted instance by env value alone.
 * - **Auth is env-carried, machine-identity-first.** Production mints the agent's
 *   own token via the OAuth `client_credentials` grant against Auth0
 *   (`mintM2mToken`), keyed on the `TEMPER_M2M_*` env — a distinct machine principal,
 *   never a proxied human. This is the path for the Auth0-fronted instance, where the
 *   Vercel Connect connector has no Auth0 M2M app behind it and so cannot mint an app
 *   token. If `TEMPER_M2M_CLIENT_ID` is absent, fall back to a Vercel Connect connector
 *   (`connect()`, for instances where that works), then to a static `TEMPER_TOKEN`
 *   (`eve dev`). `expiresAt` (ms since epoch) lets eve refresh ahead of 401; the mint
 *   is cached until ~60s before expiry.
 * - **Approval is `never()`** — the MVP steward is fully autonomous + audited (no
 *   HITL): a single team self-cogmap with no cross-map promotion (design D8).
 * - **24-tool allow-list** scoped to the steward persona (`../lib/tool-allowlists`). The 9 excluded
 *   tools (region reads + genesis/admin/access) are role-inappropriate for a steward — and
 *   `record_citation_audit` is excluded for a load-bearing reason: the auditor's fan-out opens a
 *   ROOT steward session before delegating to the subagent (eve offers no way to start a session on
 *   a subagent directly), and this list is what makes that hop capability-bounded rather than
 *   prompt-bounded. Asserted by `tests/auditor.test.ts`, not left to inspection.
 *
 * `mintM2mToken` + `requireEnv` are shared with the code schedules (`../lib/temper-auth`) so the
 * connection and the schedules can never drift on how they authenticate.
 */
export default defineMcpClientConnection({
  url: requireEnv("TEMPER_MCP_URL"),
  description:
    "Temper knowledge base: the team's own resources (the steward's ingest source) and the team cognitive map it tends. Authored-4 writes, the invocation envelope, and the steward ingest-delta live here.",
  auth: process.env.TEMPER_M2M_CLIENT_ID
    ? { getToken: mintM2mToken }
    : process.env.TEMPER_CONNECT_CONNECTOR
      ? connect({ connector: process.env.TEMPER_CONNECT_CONNECTOR, principalType: "app" })
      : { getToken: async () => ({ token: requireEnv("TEMPER_TOKEN") }) },
  // Trace propagation. When OTLP export is configured, `agent/instrumentation.ts` enables
  // undici auto-instrumentation, which injects a per-request `traceparent` naming a real
  // exported span — so we must NOT also stamp a static one here (it would be a second,
  // ambiguous header; see `otlpExportConfigured`). Without export, keep the session-derived
  // header as the cross-service log-correlation handle (PR #611).
  ...(otlpExportConfigured()
    ? {}
    : { headers: { traceparent: (ctx) => makeTraceparent(ctx.session.id) } }),
  approval: never(),
  tools: {
    allow: [...STEWARD_TOOLS],
  },
});
