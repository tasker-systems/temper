import { ConnectionAuthorizationFailedError } from "eve/connections";
import type { ConnectionPrincipal, TokenResult } from "eve/connections";

import { requestMintedToken } from "./mint.js";

/**
 * The credential seam for the `temper` MCP connection, and the read-only tool surface it is
 * allowed to reach.
 *
 * Lives here rather than in `connections/temper.ts` so both halves are unit-testable: the
 * connection file reads `TEMPER_MCP_URL` at MODULE LOAD (matching the steward's
 * `url: requireEnv(...)`), which would make a plain `import` of it throw in a test process.
 *
 * ## The principal is eve's `principalId`, passed through UNTRANSLATED
 *
 * `getTemperToken` hands `principal.id` straight to the mint route. That is only correct
 * because of what eve does one layer up (`eve/dist/src/runtime/connections/principal.js`,
 * `resolveConnectionPrincipal`):
 *
 *     return i.vercelConnect!==void 0 && isVercelDevelopmentUser(o)
 *       ? {attributes:o.attributes, id:o.subject??o.principalId, type:`user`}
 *       : {attributes:o.attributes, id:o.principalId, issuer:o.issuer??o.authenticator, type:`user`}
 *
 * On the second (non-Connect) branch `principal.id` IS the `SessionAuthContext.principalId`
 * — for this channel, the `slack:<team>:<user>` string `buildSlackAuthContext` minted and
 * that `slack.ts` already sends to `link-state`. Same string, no mapping table, no parse.
 *
 * **DO NOT add `@vercel/connect` as a dependency of this agent.** `vercelConnect` is a marker
 * `connect()` stamps onto the auth definition; its presence activates the FIRST branch, whose
 * id is `o.subject ?? o.principalId`. eve's Slack channel never sets `subject` (verified —
 * see CLAUDE.md's inbound identity contract), so today that branch happens to yield the same
 * value, but it is guarded by `isVercelDevelopmentUser` and is not the identity this agent's
 * mint route is keyed on. The equality above holds on the non-Connect branch ONLY. Introducing
 * the dependency to get Connect-managed Slack credentials would silently change which string
 * authenticates as the human. This is a hard prohibition, not a preference.
 */

/**
 * The connection's runtime name. eve derives it from the FILENAME
 * (`agent/connections/temper.ts` → `"temper"`, per `McpClientConnectionDefinition`'s doc), so
 * this constant does not register anything — it exists so the errors thrown here name the
 * same connection eve does. Renaming the file must change this string too.
 */
export const TEMPER_CONNECTION_NAME = "temper";

/**
 * The tools this agent may call. **READ-ONLY, and the allow-list is the enforcement point.**
 *
 * Writes are deliberately absent. They are not merely unimplemented: a read-only member of a
 * context can currently create a resource in it, so granting a write tool under a human's
 * minted token would exercise that bug with that human's whole reach. Until that is fixed,
 * nothing here may mutate.
 *
 * Taken as the READ HALF of the steward's 24-name list
 * (`packages/agent-workflows/steward/agent/connections/temper.ts`, its `// Reads` block).
 * Tools excluded for UNCERTAINTY rather than for being known writes — `ingest_blocks`,
 * `context_materialize`, `cogmap_materialize`, `resource_lineage`, `get_block_provenance`,
 * `describe_open_meta`, the `*_shape` / `*_region_metrics` / `*_analytics` region reads,
 * `invocation_show`, `invocation_list`, `list_my_invitations` — stay out. Several are plainly
 * reads; the rule an allow-list deserves is that "probably a read" is not a reason to grant,
 * and adding one later costs a line. If you add a name here, verify in
 * `crates/temper-mcp/src/service.rs` that it dispatches to a read.
 */
export const TEMPER_READ_TOOLS = [
  "search",
  "get_resource",
  "get_context",
  "list_contexts",
  "list_resources",
  "cogmap_read_charter",
  "describe_doc_type",
  "list_doc_types",
  "get_profile",
] as const;

/**
 * Mint an access token that acts as the mentioning human.
 *
 * Takes `{ principal }` — the shape eve's `NonInteractiveAuthorizationDefinition.getToken`
 * is called with (`eve/dist/src/runtime/connections/types.d.ts`). It is deliberately NOT the
 * steward's zero-argument, process-memoized `mintM2mToken`: that one caches a single machine
 * token for the whole process, which under per-user tokens would hand one human's credential
 * to the next. eve already caches correctly here — `principalType: "user"` keys the cache on
 * `user:${issuer}:${id}`, "so concurrent users never share tokens" (same file) — so this
 * function holds no state of its own.
 *
 * `expiresAt` is `expires_at_ms` verbatim. `TokenResult.expiresAt` is "an optional absolute
 * expiration in **milliseconds since the Unix epoch**", which is exactly the unit the server
 * already converted to (`slack_mint.rs`, `expires_at.timestamp_millis()`). No arithmetic:
 * any scaling here would put the expiry in 1970 and make the cache refresh on every call.
 *
 * ## Why FAILED-and-terminal, not REQUIRED
 *
 * `not_vaulted` and `revoked` throw `ConnectionAuthorizationFailedError` with
 * `retryable: false`, not `ConnectionAuthorizationRequiredError`. `Required` tells eve an
 * authorization flow should be run — it emits `authorization.required`, whose default handler
 * posts a framework-owned PUBLIC status line an override cannot reach (CLAUDE.md, known
 * constraint 1). There is no interactive flow to run: re-linking happens out of band through
 * the Slack link route, so eve would be prompting for a door that does not exist, in public.
 * Both states are terminal until the human re-links, which is precisely what `retryable:false`
 * means — the same shape the runtime itself uses for its own terminal `principal_required`.
 *
 * In practice `onAppMention` pre-flights the mint and never dispatches on either of these, so
 * this path is the backstop for a grant revoked BETWEEN the pre-flight and the tool call. It
 * must still fail closed rather than call the MCP server with no credential.
 */
export async function getTemperToken({
  principal,
}: {
  principal: ConnectionPrincipal;
}): Promise<TokenResult> {
  if (principal.type !== "user") {
    // Not defensive padding: the union's `app` arm carries no `id`, so TypeScript requires
    // the narrow before `principal.id` is reachable. eve resolves `{ type: "app" }` only for
    // `principalType: "app"`, which this connection is not.
    throw new ConnectionAuthorizationFailedError(TEMPER_CONNECTION_NAME, {
      message: "The temper connection is user-scoped but was resolved with an app principal.",
      reason: "principal_required",
      retryable: false,
    });
  }

  const outcome = await requestMintedToken(principal.id);

  switch (outcome.status) {
    case "token":
      return { token: outcome.access_token, expiresAt: outcome.expires_at_ms };
    case "not_vaulted":
      throw new ConnectionAuthorizationFailedError(TEMPER_CONNECTION_NAME, {
        message: "No temper credential is stored for this Slack identity; it must be re-linked.",
        reason: "not_vaulted",
        retryable: false,
      });
    case "revoked":
      throw new ConnectionAuthorizationFailedError(TEMPER_CONNECTION_NAME, {
        message: "This Slack identity's temper access was revoked; it must be re-linked.",
        reason: "revoked",
        retryable: false,
      });

    default: {
      // A FOURTH mint status. Falling out of the switch would return `undefined`
      // where eve expects a `TokenResult` — the connection would then call the MCP
      // server with no credential, which is the one thing this function exists to
      // prevent. The `never` binding makes a new variant a COMPILE error; the
      // runtime arm covers a server that ships one before this agent redeploys.
      const unexpected: never = outcome;
      throw new ConnectionAuthorizationFailedError(TEMPER_CONNECTION_NAME, {
        message: `The temper mint route returned an unrecognized status: ${String(
          (unexpected as { status?: unknown }).status,
        )}`,
        reason: "unexpected_mint_status",
        retryable: false,
      });
    }
  }
}
