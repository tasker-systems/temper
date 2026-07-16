/**
 * Inbound identity decisions for the @temper mention agent.
 *
 * Pure — no eve I/O, no Slack calls, no env. The channel file owns the
 * eve-coupled side (deriving the auth context, posting, dispatching); this
 * module owns the decision, so the forks are unit-testable without Slack.
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
