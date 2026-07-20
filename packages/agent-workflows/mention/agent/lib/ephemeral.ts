/**
 * Channel-root ephemeral delivery for the @temper mention agent.
 *
 * Every reply this agent sends is private to the mentioning user. Once a
 * mention dispatches a model turn, that turn runs under the mentioning
 * human's FULL temper reach — including their personal contexts — so a reply
 * in a shared Slack channel is a disclosure to everyone in it. eve's default
 * event handlers all post via `ctx.thread.post`, which is public; this module
 * is the one delivery path that replaces them.
 *
 * ## Why the raw request, and why NOT `ctx.thread.postEphemeral`
 *
 * eve's thread helper inherits the mention's `thread_ts`, so the ephemeral
 * posts INTO a thread the user isn't viewing, where an ephemeral is invisible
 * and leaves no badge — the symptom is total silence in the channel. A
 * channel-root `chat.postEphemeral` (no `thread_ts`) shows inline where the
 * user actually mentioned. Verified the hard way in a live debugging session.
 * Do not "simplify" it back.
 *
 * `ctx.slack.request` also returns the raw Slack response instead of throwing
 * on `ok:false` (`SlackApiResponse` is `{ ok: boolean; error?: string; ... }`,
 * `eve/dist/src/compiled/@chat-adapter/slack/api.d.ts:3-15`). That matters
 * because eve's dispatcher SWALLOWS a thrown event handler:
 *
 *   catch(n){log.error(`adapter event handler threw — event swallowed`, ...)}
 *   -- eve/dist/src/channel/adapter.js, callAdapterEventHandler
 *
 * A throwing delivery would therefore produce silence, not a leak — safe, but
 * invisible. Returning `{ ok, error }` lets us surface WHY, publicly but with
 * only the Slack error code and never the reply text.
 *
 * The module is kept structurally typed (no eve imports) for the same reason
 * `identity.ts` is pure: the delivery forks are testable with plain fakes.
 * A real `SlackEventContext` / `SlackContext` satisfies `EphemeralDelivery`.
 */

/** The subset of a Slack API response this module reads. */
export interface SlackResponseLike {
  readonly ok: boolean;
  readonly error?: string;
}

/**
 * The minimum of eve's Slack context needed to deliver an ephemeral and to
 * surface a delivery failure. Deliberately NOT an import of `SlackEventContext`:
 * the real type is far wider, and depending on only what we call keeps these
 * paths testable without eve's channel surface.
 */
export interface EphemeralDelivery {
  readonly slack: {
    readonly channelId: string;
    request(operation: string, body: unknown): Promise<SlackResponseLike>;
  };
  readonly thread: {
    post(message: { text: string }): Promise<unknown>;
  };
}

/** Outcome of one ephemeral delivery attempt. */
export type EphemeralOutcome =
  | { readonly kind: "delivered" }
  /** Slack accepted the call but refused it (`ok:false`); `error` is Slack's code. */
  | { readonly kind: "failed"; readonly error: string };

/**
 * The public line posted when an ephemeral could NOT be delivered.
 *
 * Public on purpose — silence is the one outcome we refuse to ship again —
 * but it carries only Slack's own error code. Never the reply, never a URL,
 * never anything derived from the user's temper reach.
 */
export function ephemeralFailureNotice(error: string): string {
  return `I couldn't send you a private message (Slack: ${error}). Once that's sorted, mention me again.`;
}

/**
 * The shape of `SessionAuthContext` this module reads.
 *
 * Mirrors `eve/dist/src/channel/types.d.ts:47-54`. `attributes` values are
 * `string | readonly string[]`, which is why `user_id` is type-narrowed
 * rather than trusted.
 */
export interface AuthContextLike {
  readonly attributes: Readonly<Record<string, string | readonly string[]>>;
  readonly authenticator: string;
}

/**
 * Resolves the Slack user id an ephemeral should be addressed to.
 *
 * Two sources exist and they are NOT equivalent:
 *
 * 1. `ctx.session.auth.current.attributes.user_id` — the caller of the current
 *    request. Preferred.
 * 2. `ctx.state.triggeringUserId` — persisted session state, refreshed by eve's
 *    framework-owned `turn.started` wrapper. That refresh runs through
 *    `slackUserIdFromAuthContext`, which SKIPS when the authenticator is not
 *    `slack-webhook`:
 *
 *      function slackUserIdFromAuthContext(e){
 *        if(e?.authenticator!==`slack-webhook`)return; ... }
 *      -- eve/dist/src/public/channels/slack/auth.js
 *
 *    and `turn.started` only assigns when that returns a value
 *    (`r!==void 0&&(t.state.triggeringUserId=r)`,
 *    `eve/dist/src/public/channels/slack/slackChannel.js`). So state can be
 *    STALE — it holds whoever last authenticated via the webhook.
 *
 * Hence: prefer (1), fall back to (2). The fallback is not redundant —
 * `session.failed` handlers are invoked with only two arguments and never
 * receive a `SessionContext` at all:
 *
 *   e===`session.failed`?t(r,a):t(r,a,buildCallbackContext())
 *   -- eve/dist/src/public/definitions/defineChannel.js, buildAdapter
 *
 * Returns `null` when neither source yields a usable id. There is deliberately
 * no "post publicly instead" fallback: an undeliverable private reply is
 * dropped, not broadcast.
 *
 * The same predicate as eve's is applied to source (1) so a non-Slack
 * authenticator is refused by default rather than admitted by accident.
 */
export function resolveEphemeralRecipient(
  auth: AuthContextLike | null | undefined,
  fallbackUserId: string | null | undefined,
): string | null {
  if (auth?.authenticator === "slack-webhook") {
    const userId = auth.attributes.user_id;
    if (typeof userId === "string" && userId.length > 0) return userId;
  }
  return typeof fallbackUserId === "string" && fallbackUserId.length > 0 ? fallbackUserId : null;
}

/**
 * Posts `text` to `userId` as a CHANNEL-ROOT ephemeral, and surfaces a
 * delivery failure publicly (error code only).
 *
 * `thread_ts` is intentionally absent from the payload — see the module
 * comment. Adding it makes the message invisible.
 */
export async function deliverEphemeral(
  ctx: EphemeralDelivery,
  userId: string,
  text: string,
): Promise<EphemeralOutcome> {
  const res = await ctx.slack.request("chat.postEphemeral", {
    channel: ctx.slack.channelId,
    user: userId,
    text,
  });

  if (res.ok) return { kind: "delivered" };

  const error = res.error ?? "unknown_error";
  console.error("postEphemeral failed", { error });
  await ctx.thread.post({ text: ephemeralFailureNotice(error) });
  return { kind: "failed", error };
}
