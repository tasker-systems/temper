import { assertSlackSecretsDistinct, requireEnv, signIntentRequest } from "./link.js";

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
 * `LinkRefusal` and `Refusal` are GENERATED from the Rust by ts-rs — not mirrored by hand.
 *
 * `cargo make generate-ts-types` writes `agent/generated/` from
 * `crates/temper-core/src/types/slack.rs` (`LinkRefusal`) and its transitive closure
 * (`Refusal`/`Standing`/`ActorAuthority` from temper-principal). The package gets its own copy
 * rather than importing temper-ui's because it is workspace-isolated by design (CLAUDE.md), and
 * `.github/scripts/check-ts-rs-drift.sh` fails the build if the committed files stop matching
 * what the Rust emits.
 *
 * **Do not edit those files, and do not re-mirror these types here.** A hand-written copy is
 * exactly what shipped a mention agent speaking a retired wire contract while `tsc` and 79 tests
 * stayed green (PR #498) — the drift was between languages, where no single-language gate can see
 * it.
 *
 * **The remedy splits on `reason`, and that split is the point of the type.** `not_vaulted` is
 * fixed by re-linking. Every `standing` refusal is fixed by an ADMIN, and re-linking does nothing
 * for it — telling a denied human to reconnect sends them round a loop that cannot terminate. The
 * former flat `revoked` arm said exactly that to everyone.
 */
export type { LinkRefusal } from "../generated/slack_link.js";
export type { Refusal } from "../generated/admission.js";

import type { LinkRefusal } from "../generated/slack_link.js";

/**
 * What temper says when asked to mint for this Slack principal — mirrors the Rust
 * `SlackMintResponse` (`#[serde(tag = "status", rename_all = "snake_case")]`,
 * `crates/temper-api/src/handlers/slack_mint.rs:33-47`), which itself mirrors `MintOutcome`.
 *
 * A discriminated union because neither arm is an error, so neither is an HTTP failure: a 200
 * carrying a refusal is the honest encoding of *"the request was fine; there is nothing to mint,
 * and here is exactly why."* Collapsing it into a nullable token would force the agent to say
 * one vague thing about every refusal.
 *
 * **Two arms outside, three tags nested.** `status` discriminates token-vs-refused, `reason`
 * discriminates the refusal, and `kind` discriminates the standing beneath it:
 * `{"status":"refused","reason":"standing","refusal":{"kind":"denied"}}`. The Rust side notes
 * these three tags are distinct precisely so they nest without collision — a newtype
 * `Standing(Refusal)` under a shared tag emitted a duplicate `kind` key and would arrive here as
 * an uninhabitable type.
 */
export type MintOutcome =
  | {
      readonly status: "token";
      readonly access_token: string;
      /**
       * Absolute expiry, epoch **MILLISECONDS** — `Date.now()`-comparable, which is the unit
       * eve's `TokenResult.expiresAt` expects. The server converts
       * (`slack_mint.rs`, `expires_at.timestamp_millis()`) so this side does no arithmetic:
       * a seconds value would deserialize just as well and merely claim the token expired
       * in 1970, and the resulting "always expired" cache would look like a server bug.
       */
      readonly expires_at_ms: number;
    }
  | ({ readonly status: "refused" } & LinkRefusal);

/**
 * Ask temper for an access token to act as the human who mentioned us.
 *
 * `principalId` is passed WHOLE. It has 2-4 segments and must never be split.
 */
export async function requestMintedToken(principalId: string): Promise<MintOutcome> {
  // The route this guards is the expensive one, so it re-asserts rather than trusting that
  // `requestLinkState` ran first: `getToken` (`lib/mcp-auth.ts`) mints without going through
  // link-state at all, so the mint path is genuinely reachable without the earlier check.
  assertSlackSecretsDistinct();

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
