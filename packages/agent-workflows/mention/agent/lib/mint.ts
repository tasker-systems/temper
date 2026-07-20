import { requireEnv, signIntentRequest } from "./link.js";

/**
 * The act-as-the-human mint call: agent -> temper-api.
 *
 * Same signing scheme as the link-state call (`signIntentRequest`, reused rather than
 * reimplemented), but keyed on a DIFFERENT secret and that is load-bearing, not tidiness.
 * `SLACK_LINK_SECRET` gates an endpoint that answers a question — "is this principal linked?".
 * `SLACK_MINT_SECRET` gates one that hands back a token carrying that human's ENTIRE temper
 * reach. Sharing a key would make compromise of the cheap capability yield the expensive one.
 * temper states the split at `crates/temper-services/src/config.rs:32-48` and enforces it with a
 * separate router + layer (`slack_mint_internal_routes`, `require_slack_mint_signature`).
 *
 * That transport gate is also the ONLY thing enforcing "naming a principal must not be
 * sufficient to mint its token" — the principal in the body is trusted precisely because only a
 * holder of the mint secret could have put it there. So this module must derive `principalId`
 * from eve's signature-verified `app_mention`, never from anything a Slack user can type.
 */

/**
 * What temper says when asked to mint for this Slack principal — mirrors the Rust
 * `SlackMintResponse` (`#[serde(tag = "status", rename_all = "snake_case")]`,
 * `crates/temper-api/src/handlers/slack_mint.rs:38-57`), which itself mirrors `MintOutcome`.
 *
 * A discriminated union for the same reason `LinkState` is one: none of the three is an error,
 * so none is an HTTP failure, and "no grant on file" and "the grant was revoked" are different
 * facts the user needs different sentences for. Collapsing them into a nullable token would
 * force the agent to say something vague about both. Branching on `status` makes the compiler
 * insist all three arms are handled.
 */
export type MintOutcome =
  | {
      readonly status: "token";
      readonly access_token: string;
      /**
       * Absolute expiry, epoch **MILLISECONDS** — `Date.now()`-comparable, which is the unit
       * eve's `TokenResult.expiresAt` expects. The server converts
       * (`slack_mint.rs:79`, `expires_at.timestamp_millis()`) so this side does no arithmetic:
       * a seconds value would deserialize just as well and merely claim the token expired
       * in 1970, and the resulting "always expired" cache would look like a server bug.
       */
      readonly expires_at_ms: number;
    }
  | { readonly status: "revoked" }
  | { readonly status: "not_vaulted" };

/**
 * Ask temper for an access token to act as the human who mentioned us.
 *
 * `principalId` is passed WHOLE. It has 2-4 segments and must never be split.
 */
export async function requestMintedToken(principalId: string): Promise<MintOutcome> {
  const baseUrl = requireEnv("TEMPER_API_URL");
  const secret = requireEnv("SLACK_MINT_SECRET");

  const body = JSON.stringify({ slack_principal_id: principalId });
  const { timestamp, signature } = signIntentRequest(secret, Date.now() / 1000, body);

  const res = await fetch(`${baseUrl.replace(/\/$/, "")}/internal/slack/mint`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Timestamp": timestamp,
      "X-Temper-Signature": signature,
    },
    body,
  });

  if (!res.ok) {
    throw new Error(`mint failed: ${res.status}`);
  }

  return (await res.json()) as MintOutcome;
}
