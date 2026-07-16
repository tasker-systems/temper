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
 * A discriminated union so callers must handle both forks; `principalId` is
 * only reachable on the accepted branch.
 */
export type IdentityDecision =
  | { readonly kind: "human"; readonly principalId: string }
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
 */
export function decideIdentity(auth: PrincipalLike | null): IdentityDecision {
  if (auth === null) return { kind: "rejected", reason: "no-auth" };
  if (!isHumanPrincipal(auth)) return { kind: "rejected", reason: "not-human" };
  return { kind: "human", principalId: auth.principalId };
}

/**
 * The prompt shown to a Slack user with no linked temper account.
 *
 * T1 has NO temper reach — no machine token, no temper-ts, no link lookup — so
 * every human currently lands here. The echoed `principalId` is T1's acceptance
 * evidence (it proves the resolved identity made it through the pipe intact)
 * and is the exact key T2's account link will be stored under.
 */
export function unlinkedPrompt(principalId: string): string {
  return [
    "I don't have a temper account linked to your Slack identity yet.",
    "",
    `Resolved Slack principal: \`${principalId}\``,
    "",
    "Account linking isn't wired up yet — this is the mention pipe proving it can hear you.",
  ].join("\n");
}
