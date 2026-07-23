import { defineMcpClientConnection } from "eve/connections";
import { never } from "eve/tools/approval";

import { requireEnv } from "../lib/link.js";
import { TEMPER_READ_TOOLS, getTemperToken } from "../lib/mcp-auth.js";

/**
 * The mention agent's seam to temper-mcp, acting as the mentioning human.
 *
 * Registered by FILESYSTEM CONVENTION — there is no manifest. The filename gives the
 * connection its name (`temper`) and its tools become `temper__*`
 * (`McpClientConnectionDefinition`, `eve/dist/src/public/definitions/connections/mcp.d.ts`).
 *
 * - **URL is env-driven** (`TEMPER_MCP_URL`), like the steward's, so one agent directory
 *   points at temperkb.io or a self-hosted instance by env value alone.
 * - **`principalType: "user"`** — the whole point. The steward is app-scoped and speaks as a
 *   machine; this agent speaks as whoever mentioned it, under exactly their reach. eve keys
 *   the token cache on `user:${issuer}:${id}`, so two people mentioning at once cannot share
 *   a token. `getTemperToken` therefore memoizes NOTHING (the steward's `mintM2mToken`
 *   memoizes a process-wide singleton — copying it here would be a cross-user token leak).
 * - **Do not add `@vercel/connect`.** It would switch eve's principal resolution to a branch
 *   keyed on a different id. The full argument is in `../lib/mcp-auth.ts`; it is a hard
 *   prohibition.
 * - **`approval: never()`** — every tool below is a READ performed under the caller's own
 *   credential, so an approval prompt would ask a human to authorize reading their own data.
 *   It would also be undeliverable as posed: this agent's `input.requested` override
 *   (`channels/events.ts`) can only post ephemeral TEXT, so the prompt is a message with no
 *   affordance to answer it, and the turn stalls until it times out. `always()`/`once()`
 *   become worth revisiting when a write tool is added — and a write is what would make them
 *   worth building the interaction for.
 * - **Tool surface is READ-ONLY** and enumerated in `TEMPER_READ_TOOLS`, which carries the
 *   rationale and the list of tools left out for uncertainty.
 */
export default defineMcpClientConnection({
  url: requireEnv("TEMPER_MCP_URL"),
  description:
    "Temper knowledge base, read as the Slack user who mentioned this agent: their resources, contexts, doc types and search, under their own access and no one else's.",
  auth: { principalType: "user", getToken: getTemperToken },
  approval: never(),
  tools: { allow: TEMPER_READ_TOOLS },
});
