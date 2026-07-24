/**
 * Inbound identity decisions for the @temper mention agent.
 *
 * Pure — no eve I/O, no Slack calls, no env. The channel file owns the
 * eve-coupled side (deriving the auth context, posting, dispatching); this
 * module owns the decision, so the forks are unit-testable without Slack.
 * (`standingReply` writes one `console.error` on an unrecognized standing; that is a
 * diagnostic on an anomaly path, not I/O this module's callers depend on.)
 *
 * `Refusal` is imported from `agent/generated/` — the ts-rs output — rather than from the mint
 * module, which would have pulled `link.js`'s env reads in behind it and cost this module its
 * env-free-at-import property, which every test here relies on. It is `import type` and so
 * erased at compile in any case.
 *
 * ## principalId is OPAQUE
 *
 * eve's Slack channel mints `principalId` in FOUR shapes, because `teamId` is
 * nullable and bots get an extra segment (verified in
 * `eve/dist/src/public/channels/slack/auth.js`):
 *
 * | teamId | author | principalId                | principalType |
 * | ------ | ------ | -------------------------- | ------------- |
 * | yes    | human  | `slack:<team>:<user>`      | `user`        |
 * | yes    | bot    | `slack:<team>:bot:<user>`  | `service`     |
 * | no     | human  | `slack:<user>`             | `user`        |
 * | no     | bot    | `slack:bot:<user>`         | `service`     |
 *
 * So the segment COUNT varies from 2 to 4. Any code that splits on `:` and
 * indexes into the result is wrong for at least one of these shapes — it will
 * either throw or, worse, silently mis-key a user (reading `<user>` out of the
 * slot that holds `<team>`). We therefore treat the whole string as an opaque
 * key: store it whole, compare it whole, log it whole. Never parse it.
 *
 * The Slack-derived attributes (`user_id`, `team_id`, ...) already carry the
 * decomposed parts for anyone who needs them, which is the other reason
 * parsing the principalId is never necessary.
 */

import type { Refusal } from "../generated/admission.js";

/** Why an inbound principal was refused a dispatch. */
export type RejectionReason =
  /** eve returned no auth context — the Slack message carried no author. */
  | "no-auth"
  /** The principal is not a human (bots surface as `principalType: "service"`). */
  | "not-human";

/**
 * Outcome of deciding whether an inbound Slack principal may drive a turn.
 * A discriminated union so callers must handle both forks; `principalId` and
 * `auth` are only reachable on the accepted branch.
 *
 * Generic over the caller's auth shape (`T`) so the accepted arm can carry
 * the FULL `SessionAuthContext` (e.g. `attributes.user_id`) through to the
 * channel, while this module itself only ever reads the `PrincipalLike`
 * subset it needs — keeping it pure and testable without eve's types.
 */
export type IdentityDecision<T extends PrincipalLike = PrincipalLike> =
  | { readonly kind: "human"; readonly principalId: string; readonly auth: T }
  | { readonly kind: "rejected"; readonly reason: RejectionReason };

/**
 * The minimum of `SessionAuthContext` this decision needs.
 *
 * Structural, and deliberately NOT an import of eve's `SessionAuthContext`:
 * the real type is wider (attributes, authenticator, issuer), and depending on
 * only what we read keeps these tests free of eve's channel surface. A real
 * `SessionAuthContext` satisfies this shape.
 */
export interface PrincipalLike {
  readonly principalId: string;
  readonly principalType: string;
}

/**
 * eve's own fail-closed predicate: only `principalType === "user"` is a human.
 * Mirrored here rather than inverted (`!== "service"`) so a future
 * principalType eve adds is refused by default instead of admitted by accident.
 */
export function isHumanPrincipal(auth: PrincipalLike): boolean {
  return auth.principalType === "user";
}

/**
 * Decides whether an inbound Slack principal may dispatch a turn.
 *
 * Accepts `null` because `defaultSlackAuth` returns `null` for an authorless
 * message — the caller passes its result straight through.
 *
 * The returned `principalId` is eve's string VERBATIM. See the module comment.
 *
 * Returns the caller's own `auth` object back on the accepted arm (typed as
 * `T`), so a caller passing the real `SessionAuthContext` gets it back
 * intact — e.g. to read `attributes.user_id` — without this module needing
 * to know that wider shape.
 */
export function decideIdentity<T extends PrincipalLike>(auth: T | null): IdentityDecision<T> {
  if (auth === null) return { kind: "rejected", reason: "no-auth" };
  if (!isHumanPrincipal(auth)) return { kind: "rejected", reason: "not-human" };
  return { kind: "human", principalId: auth.principalId, auth };
}

/**
 * The prompt shown to a Slack user with no linked temper account.
 *
 * Carries a one-time authorize URL, which is why it MUST be delivered
 * ephemerally (`postEphemeral`), never via a public `thread.post`: whoever
 * opens this URL binds their temper identity to the mentioning Slack
 * principal. See `agent/channels/slack.ts` for the delivery side.
 */
export function unlinkedPrompt(authorizeUrl: string): string {
  return [
    "I don't have a temper account linked to your Slack identity yet.",
    "",
    `Connect your account: ${authorizeUrl}`,
    "",
    "This link is single-use and just for you — please don't forward it.",
  ].join("\n");
}

/**
 * The remedy for the one refusal re-linking actually fixes.
 *
 * There is no authorize URL to offer here, and that is structural rather than an omission:
 * `link-state` only carries `authorize_url` on its `unlinked` arm (`link.ts`, `LinkState`),
 * and this state is reached from the `linked` arm. So the honest instruction is the one that
 * MAKES the user unlinked — after `temper slack disconnect`, the next mention takes the
 * unlinked arm and gets a fresh URL from `unlinkedPrompt`.
 *
 * **Used by {@link notVaultedPrompt} and nothing else, deliberately.** It once ended the
 * revoked reply too, which is precisely how the false remedy shipped: a standing refusal is not
 * a link problem, so sending that user through disconnect-and-reconnect returns them to the
 * same refusal with one more step behind them. If you are about to add a second caller, check
 * first that re-linking really is the remedy for it.
 */
const RECONNECT_REMEDY =
  "Run `temper slack disconnect`, then mention me again — I'll send you a fresh connect link.";

/**
 * The reply for a linked user whose stored credential is missing.
 *
 * Distinct from {@link standingRefusedPrompt} on purpose. This user did nothing wrong and
 * nothing was withheld: their link predates the credential store, or the link never completed.
 * Their access is fine — the credential is what is missing, which is why this is the one
 * refusal whose remedy the user can carry out alone.
 * The load-bearing sentence is that RETRYING WILL NOT FIX IT — without it the user mentions
 * again, gets the same nothing, and reasonably concludes the agent is broken.
 */
export function notVaultedPrompt(handle: string): string {
  return [
    `You're connected as @${handle}, but I don't have a stored credential I can use to look things up as you.`,
    "",
    "Mentioning me again won't fix this — the connection needs to be made afresh.",
    "",
    RECONNECT_REMEDY,
  ].join("\n");
}

/**
 * The reply for a linked, vaulted user whose PRINCIPAL STANDING does not admit them.
 *
 * Covers four `Refusal` kinds — `denied`, `no_standing`, `revoked`, `deactivated` — because they
 * share one remedy, and the remedy is what the user needs. Grouping by remedy rather than by
 * variant is deliberate: four near-identical sentences differing only in a fact the user cannot
 * act on would be precision spent on the wrong axis.
 *
 * **The load-bearing sentence is "reconnecting won't change this."** This is the false remedy the
 * former flat `revoked` arm shipped: it told every refused human to re-link, which for a standing
 * refusal is a loop that cannot terminate — the link was never the problem, and re-making it
 * lands them in exactly the same state. Only an admin can move principal standing
 * (`temper_principal::admit` is the sole producer of these refusals).
 *
 * Deliberately NOT distinguishing revoked-from-denied in the copy: both mean "not approved right
 * now", both are fixed by the same person, and a user told "your access was REVOKED" when they
 * were merely never granted would go hunting for something that never happened.
 */
export function standingRefusedPrompt(handle: string): string {
  return [
    `You're connected as @${handle}, but your temper access isn't currently approved, so I can't look anything up as you.`,
    "",
    "Reconnecting won't change this — a temper admin has to approve your access.",
    "",
    "Ask an admin to approve you, then mention me again.",
  ].join("\n");
}

/**
 * The reply for a user whose access request is on file and undecided (`Refusal::Requested`).
 *
 * Its own message rather than folding into {@link standingRefusedPrompt} because the ACTION
 * differs, which is the axis this copy splits on: a denied user must go ask someone, a requested
 * user must do nothing at all. Telling someone with a pending request to "ask an admin to approve
 * you" invites them to file a second one.
 */
export function pendingApprovalPrompt(handle: string): string {
  return [
    `You're connected as @${handle}, and your temper access request hasn't been decided yet, so I can't look anything up as you.`,
    "",
    "Nothing more is needed from you — an admin will approve or decline it.",
    "",
    "Mention me again once you've been approved.",
  ].join("\n");
}

/**
 * The reply for a standing value this build does not recognize (`Refusal::UnrecognizedStanding`).
 *
 * Reachable only through version skew: temper stored a standing state that postdates this
 * deployment of the agent. So it is the one refusal that is genuinely OUR problem, and the copy
 * says so rather than implying the user has been refused something. No remedy is offered because
 * the user has none — the caller logs the raw value, which is where the actual fix starts.
 */
export function unknownStandingPrompt(handle: string): string {
  return [
    `You're connected as @${handle}, but I can't tell what your temper access is right now.`,
    "",
    "That's a problem on our side, not something you did. Please let a temper admin know if it keeps happening.",
  ].join("\n");
}

/**
 * Pick the reply for a `standing` refusal — the single place six `Refusal` kinds collapse onto
 * three remedies.
 *
 * One function rather than a `switch` at each call site (the channel's ephemeral and, later,
 * anything else that must explain a refusal) so the grouping is stated once. The grouping is the
 * decision worth protecting: {@link standingRefusedPrompt} for the four an admin must act on,
 * {@link pendingApprovalPrompt} for the one where the user must NOT act, and
 * {@link unknownStandingPrompt} for the one that is our bug.
 *
 * No `default` arm, deliberately: the `never` binding makes a seventh `Refusal` kind a COMPILE
 * error here rather than a silent fall-through to `undefined`. That is the whole reason the kinds
 * are modelled as a union instead of a `string` — and until the Task 8/9 drift gate lands, this
 * compile error is the only mechanism that will notice a new Rust variant at all.
 */
export function standingReply(refusal: Refusal, handle: string): string {
  switch (refusal.kind) {
    // Never approved, no standing row at all, approved-then-withdrawn, or the profile itself
    // disabled. Four different histories, one remedy: an admin decides.
    case "denied":
    case "no_standing":
    case "revoked":
    case "deactivated":
      return standingRefusedPrompt(handle);

    case "requested":
      return pendingApprovalPrompt(handle);

    case "unrecognized_standing":
      // The raw value is the ONLY diagnostic for this state, and it is a value this build has
      // never heard of — so it is logged here, beside the branch that knows it exists, rather
      // than left for a caller to dig back out by re-switching on the kind.
      console.error("temper returned an unrecognized principal standing", { raw: refusal.raw });
      return unknownStandingPrompt(handle);

    // The three transition-machine refusals. `Refusal` is the WHOLE generated type, but only the
    // six above are reachable through the mint: `resolve` delegates standing to
    // `temper_principal::admit`, and `only_admit_reachable_refusals_ever_surface`
    // (`crates/temper-services/src/services/slack_link_state.rs:173`) panics if any of these ever
    // surfaces there. So arriving here means that Rust invariant broke — which makes it our bug,
    // not a state to write user-facing copy for, and it reuses the our-bug reply for that reason.
    //
    // Handled EXPLICITLY rather than by a `default:` so the switch stays exhaustive: a tenth
    // `Refusal` variant is then a compile error here, which — until it reaches this file — is the
    // earliest anything notices a new Rust variant.
    case "illegal_transition":
    case "insufficient_authority":
    case "no_prior_standing":
      console.error("mint surfaced a refusal admit cannot produce — check slack_link_state", {
        kind: refusal.kind,
      });
      return unknownStandingPrompt(handle);
  }
}

/**
 * The reply for a mint that came back `not_linked` — the link vanished mid-mention.
 *
 * Reachable only as a race: `onAppMention` asks link-state first, got `linked`, and the link was
 * removed (a `temper slack disconnect`) before the mint landed. Rare, but it is a real ordering
 * and the generic error line would misdescribe it as a hiccup worth retrying.
 *
 * **This is the one broken-credential reply where "mention me again" IS the remedy**, and the
 * asymmetry is the point: the user is now genuinely unlinked, so their next mention takes
 * link-state's `unlinked` arm and gets a fresh authorize URL from {@link unlinkedPrompt}. We
 * cannot hand them that URL here — this arm is reached from `linked`, which carries no
 * `authorize_url` — so the next mention is how they get one.
 */
export function linkVanishedPrompt(handle: string): string {
  return [
    `Your temper account link for @${handle} isn't there any more, so I can't look anything up as you.`,
    "",
    "Mention me again and I'll send you a fresh connect link.",
  ].join("\n");
}
