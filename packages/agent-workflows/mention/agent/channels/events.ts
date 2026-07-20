import type { SlackChannelEvents } from "eve/channels/slack";

import { deliverEphemeral, resolveEphemeralRecipient } from "../lib/ephemeral.js";

/**
 * Ephemeral replacements for every eve Slack default that posts PUBLICLY.
 *
 * ## Why this file exists
 *
 * A dispatched turn runs under the mentioning human's FULL temper reach,
 * including their personal contexts. eve's defaults render turn output with
 * `ctx.thread.post`, which is visible to the whole channel — verified in
 * `eve/dist/src/public/channels/slack/defaults.js` (minified), where
 * `message.completed`, `turn.failed` and `session.failed` each end in
 * `await ...thread.post(...)`, and `defaultInputRequestedHandler` loops
 * `await t.thread.post(n)`. So these four sinks must be private BEFORE any
 * dispatch exists, not after.
 *
 * ## A supplied handler REPLACES the default — there is no supplement
 *
 * Verified at three layers:
 *
 *   m={...defaultEvents,...e.events, ...}                    (user last)
 *   "input.requested":e.events?.[`input.requested`]??defaultInputRequestedHandler()
 *   -- eve/dist/src/public/channels/slack/slackChannel.js
 *
 *   for(let e of d){let t=f?.[e]; t&&(u=!0,l[e]=(r,i)=>{...})} (one fn per name)
 *   -- eve/dist/src/public/definitions/defineChannel.js, buildAdapter
 *
 *   let r=e[t.type]; if(r===void 0)return t; try{await r(...)}  (one read, one call)
 *   -- eve/dist/src/channel/adapter.js, callAdapterEventHandler
 *
 * So each handler below is the ONLY thing that runs for its event. Anything
 * the default did that still matters must be reproduced here.
 *
 * ## Failure mode to keep in mind
 *
 * `callAdapterEventHandler` swallows a thrown handler
 * (`log.error("adapter event handler threw — event swallowed")`). A bug here
 * therefore fails as SILENCE, not as a leak — safe, but invisible. That is
 * exactly why these paths are unit-tested rather than eyeballed.
 *
 * ## Residual gap: `authorization.required` is NOT overridden
 *
 * Deliberately. Its override context (`SlackAuthorizationEventContext`) is
 * narrowed to private delivery only — no public `post`, no `slack.request` —
 * and eve's default additionally posts a framework-owned public status line
 * that it edits on `authorization.completed`
 * (`state.pendingAuthMessageTs`). That public post is NOT reachable from an
 * override, so overriding cannot remove it and would only cost us the
 * framework's own edit-in-place behaviour. The public line is link-free by
 * construction (the challenge URL goes to the private surface), so the
 * residual disclosure is "this agent asked someone to connect an account" —
 * documented here rather than papered over.
 */

/**
 * First non-empty line of a string, or `null`.
 *
 * Mirrors eve's non-exported `firstNonEmptyLine`, used only to keep
 * `state.pendingToolCallMessage` byte-compatible with what the default
 * `actions.requested` handler expects to read back out of it.
 */
export function firstNonEmptyLine(text: string): string | null {
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.length > 0) return trimmed;
  }
  return null;
}

/**
 * The completed assistant message — the actual answer, and the single
 * highest-risk public sink in the whole channel.
 *
 * The `finishReason === "tool-calls"` early return is REPRODUCED from eve's
 * default and is load-bearing: that branch fires on every mid-turn message
 * boundary where the model narrates before calling a tool. Omitting it would
 * post tool chatter on every turn instead of once at the end. It writes the
 * narration to `state.pendingToolCallMessage`, which the default
 * `actions.requested` handler reads to render a typing indicator.
 */
export const messageCompleted: NonNullable<SlackChannelEvents["message.completed"]> = async (
  event,
  ctx,
  callbackCtx,
) => {
  if (event.finishReason === "tool-calls") {
    ctx.state.pendingToolCallMessage = event.message ? firstNonEmptyLine(event.message) : null;
    return;
  }

  ctx.state.pendingToolCallMessage = null;

  if (!event.message) {
    // No text to disclose. The typing indicator is public but content-free,
    // so it is kept: it is the only signal the user gets that work continues.
    await ctx.thread.startTyping();
    return;
  }

  await deliverToTriggeringUser(ctx, callbackCtx, event.message, "message.completed");
};

/** A failed turn. eve's default posts the failure into the channel; ours does not. */
export const turnFailed: NonNullable<SlackChannelEvents["turn.failed"]> = async (
  _event,
  ctx,
  callbackCtx,
) => {
  // Error text can quote the model's own output or a tool's response, both of
  // which are reach-derived. So the ephemeral says only that it failed.
  await deliverToTriggeringUser(
    ctx,
    callbackCtx,
    [
      "I hit an error while handling your request.",
      "",
      "Please try again, rephrase, or reach out if it keeps failing.",
    ].join("\n"),
    "turn.failed",
  );
};

/**
 * An unrecoverable session. Note the TWO-argument signature: eve calls
 * `session.failed` without a `SessionContext`
 * (`e===\`session.failed\`?t(r,a):t(r,a,buildCallbackContext())` in
 * `buildAdapter`), so the recipient can only come from session state.
 */
export const sessionFailed: NonNullable<SlackChannelEvents["session.failed"]> = async (
  _event,
  ctx,
) => {
  await deliverToTriggeringUser(
    ctx,
    undefined,
    [
      "This session couldn't recover from an error.",
      "",
      "Start a new thread to continue — I can't pick this one back up.",
    ].join("\n"),
    "session.failed",
  );
};

/**
 * The harness is asking a human to answer something before it can continue.
 *
 * eve's default renders interactive Block Kit cards into the channel. This
 * override delivers the prompts as ephemeral TEXT: the block builders
 * (`renderInputRequestBlocks`) are not exported, and an interactive card
 * would need its own private-response wiring. The prompt itself can quote the
 * turn's context, so it cannot stay public in the meantime. Answering
 * interactively is not wired — nothing in this agent requests input yet.
 */
export const inputRequested: NonNullable<SlackChannelEvents["input.requested"]> = async (
  event,
  ctx,
  callbackCtx,
) => {
  for (const request of event.requests) {
    await deliverToTriggeringUser(ctx, callbackCtx, request.prompt, "input.requested");
  }
};

/** The four handlers, ready to spread into `slackChannel({ events })`. */
export const ephemeralEvents: SlackChannelEvents = {
  "message.completed": messageCompleted,
  "turn.failed": turnFailed,
  "session.failed": sessionFailed,
  "input.requested": inputRequested,
};

/**
 * Shared body of every override: resolve the recipient, then deliver privately.
 *
 * When no recipient can be resolved the message is DROPPED, not posted
 * publicly. That is the whole point of this file — a fallback to
 * `thread.post` here would reintroduce exactly the leak these handlers exist
 * to close. The drop is logged so it stays diagnosable.
 */
async function deliverToTriggeringUser(
  ctx: {
    readonly slack: {
      readonly channelId: string;
      request(operation: string, body: unknown): Promise<{ ok: boolean; error?: string }>;
    };
    readonly thread: { post(message: { text: string }): Promise<unknown> };
    readonly state: { triggeringUserId?: string | null };
  },
  callbackCtx: { readonly session: { readonly auth: { readonly current: unknown } } } | undefined,
  text: string,
  eventType: string,
): Promise<void> {
  const current = callbackCtx?.session.auth.current;
  const auth =
    current !== null && typeof current === "object"
      ? (current as { attributes: Record<string, string | readonly string[]>; authenticator: string })
      : null;

  const userId = resolveEphemeralRecipient(auth, ctx.state.triggeringUserId);
  if (userId === null) {
    console.warn("dropping event: no ephemeral recipient", { eventType });
    return;
  }

  await deliverEphemeral(ctx, userId, text);
}
