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
 * Why `admit` refused this human's principal standing — mirrors the Rust `Refusal`
 * (`#[serde(tag = "kind", rename_all = "snake_case")]`,
 * `crates/temper-principal/src/refusal.rs:23`).
 *
 * Only the six variants `temper_principal::admit` can actually produce are modelled. The
 * transition-machine refusals (`illegal_transition`, `insufficient_authority`,
 * `no_prior_standing`) are unreachable through the mint, and that is *pinned* rather than
 * assumed: `only_admit_reachable_refusals_ever_surface`
 * (`crates/temper-services/src/services/slack_link_state.rs:173`) panics if `resolve` ever
 * surfaces one. Modelling them here would invent three user-facing sentences for states no
 * user can reach.
 *
 * HAND-MIRRORED, deliberately and temporarily. The authoritative spelling is the ts-rs export
 * at `packages/temper-ui/src/lib/types/generated/admission.ts` — which this package cannot
 * import, because it is workspace-isolated (see CLAUDE.md). Emitting the generated type into
 * *this* tree, and gating it against drift, is exactly Task 8/9 of the linked-identity plan
 * (`docs/superpowers/plans/2026-07-23-linked-identity-state-machine.md`). Until that lands, the
 * only thing keeping this in step with Rust is a human reading both — so if you are here
 * changing it, that gate is the fix, not a wider hand-mirror.
 */
export type StandingRefusal =
  | { readonly kind: "no_standing" }
  | { readonly kind: "unrecognized_standing"; readonly raw: string }
  | { readonly kind: "denied" }
  | { readonly kind: "requested" }
  | { readonly kind: "revoked" }
  | { readonly kind: "deactivated" };

/**
 * Why a mint was refused — mirrors the Rust `LinkRefusal`
 * (`#[serde(tag = "reason", rename_all = "snake_case")]`,
 * `crates/temper-core/src/types/slack.rs:30`).
 *
 * **The remedy splits on this tag, and that split is the entire point of the type.**
 * `not_vaulted` is fixed by re-linking. Every `standing` refusal is fixed by an ADMIN, and
 * re-linking does nothing for it — telling a denied human to reconnect sends them round a loop
 * that cannot terminate. The former flat `revoked` arm said exactly that to everyone, which is
 * the false remedy this whole type exists to end.
 */
export type LinkRefusal =
  | { readonly reason: "not_linked" }
  | { readonly reason: "not_vaulted" }
  | { readonly reason: "standing"; readonly refusal: StandingRefusal };

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
