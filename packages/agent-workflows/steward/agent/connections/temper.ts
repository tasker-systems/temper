import { connect } from "@vercel/connect/eve";
import { defineMcpClientConnection } from "eve/connections";
import { never } from "eve/tools/approval";

/**
 * The steward's sole seam to temper-mcp.
 *
 * - **URL is env-driven, never hardcoded** (`TEMPER_MCP_URL`) so one agent
 *   directory points at temperkb.io OR a self-hosted instance by env value alone.
 * - **Auth is platform-carried.** Production uses Vercel Connect (`connect()`),
 *   which owns the OAuth consent, encrypted token storage, and refresh — no secret
 *   in code, nothing the model ever sees. temper-mcp is a full OAuth 2.0 server
 *   (Auth0 discovery + PKCE + refresh_token), registered as a Connect connector in
 *   T6 (`vercel connect create`). Until that connector exists, a `getToken` that
 *   returns an already-OAuth-obtained temper token (env `TEMPER_TOKEN`) drives
 *   `eve dev` and the one-real-read gate; `expiresAt` lets eve refresh ahead of 401.
 * - **Approval is `never()`** — the MVP steward is fully autonomous + audited (no
 *   HITL): a single team self-cogmap with no cross-map promotion (design D8).
 * - **24-tool allow-list** scoped to the steward persona. The 9 excluded tools
 *   (region reads + genesis/admin/access) are role-inappropriate for a steward.
 */
export default defineMcpClientConnection({
  url: requireEnv("TEMPER_MCP_URL"),
  description:
    "Temper knowledge base: the team's own resources (the steward's ingest source) and the team cognitive map it tends. Authored-4 writes, the invocation envelope, and the steward ingest-delta live here.",
  auth: process.env.TEMPER_CONNECT_CONNECTOR
    ? connect({ connector: process.env.TEMPER_CONNECT_CONNECTOR, principalType: "app" })
    : { getToken: async () => ({ token: requireEnv("TEMPER_TOKEN") }) },
  approval: never(),
  tools: {
    allow: [
      // Authored-4
      "create_resource",
      "assert_relationship",
      "facet_set",
      "fold_relationship",
      // Invocation envelope
      "invocation_open",
      "invocation_close",
      "invocation_show",
      "invocation_list",
      // Steward delta / watermark
      "steward_ingest_delta",
      "steward_advance_watermark",
      // Reads
      "search",
      "get_resource",
      "get_context",
      "list_contexts",
      "list_resources",
      "cogmap_read_charter",
      "describe_doc_type",
      "list_doc_types",
      "get_profile",
      // Mutations (delete_resource is soft-delete: flips is_active via a resource_deleted event)
      "update_resource",
      "update_resource_meta",
      "delete_resource",
      "retype_relationship",
      "reweight_relationship",
    ],
  },
});

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the temper-mcp target/credential is never hardcoded`);
  }
  return value;
}
