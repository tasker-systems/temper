import type { SlackChannelEvents } from "eve/channels/slack";

import { deliverEphemeral, resolveEphemeralRecipient } from "./ephemeral.js";

/**
 * Ephemeral (or content-free) replacements for every eve Slack default that
 * routes MODEL-DERIVED TEXT to a sink the whole channel can observe.
 *
 * ## Why this file exists
 *
 * A dispatched turn runs under the mentioning human's FULL temper reach,
 * including their personal contexts. eve's defaults render turn output with
 * `ctx.thread.post`, which is visible to the whole channel — verified in
 * `eve/dist/src/public/channels/slack/defaults.js` (minified), where
 * `message.completed`, `turn.failed` and `session.failed` each end in
 * `await ...thread.post(...)`, and `defaultInputRequestedHandler` loops
 * `await t.thread.post(n)`. So these sinks must be private BEFORE any
 * dispatch exists, not after.
 *
 * ## `thread.post` is not the only public sink — `startTyping` is one too
 *
 * Two further defaults push model-derived text into the thread's typing
 * status, which is NOT private:
 *
 *   "reasoning.appended"(e,t,n){let r=firstNonEmptyLine(e.reasoningSoFar);
 *     ... await t.thread.startTyping(i) ...}
 *   "actions.requested"(e,t,n){let r=t.state.pendingToolCallMessage;
 *     ... if(r){await t.thread.startTyping(truncateTypingStatus(r));return}
 *     ... `Running ${i.join(", ")}...` ...}
 *   -- eve/dist/src/public/channels/slack/defaults.js
 *
 * `reasoningSoFar` is the model's raw reasoning; `pendingToolCallMessage` is
 * the model's own mid-turn narration. Both are produced under the mentioning
 * human's reach, so both are overridden below with a CONSTANT status string.
 *
 * `startTyping` currently resolves to `assistant.threads.setStatus`, which is
 * scoped to assistant threads and may well no-op in an ordinary public channel
 * — but that is a Slack-side accident, not a property of this code, and Slack
 * keeps widening agent surfaces into channels. The override does not depend on
 * it.
 *
 * ## The two defaults deliberately left in place
 *
 * `turn.started` calls `startTyping("Working...")` — a constant, no event data
 * read. `authorization.completed` `chat.update`s a message id it only holds
 * when `authorization.required` posted one, with text built from the
 * connection display name and an outcome/reason code
 * (`buildAuthCompletedText`, `.../slack/connections.js`) — no model or tool
 * output. Both are content-free; see `tests/events.test.ts`, which derives
 * this classification against eve's real `defaultEvents` so a future eve
 * release adding a sink FAILS rather than silently opening one.
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
 * The one status string this agent ever shows publicly.
 *
 * Constant by construction: it is the same string eve's own `turn.started`
 * default uses, and it is derived from NOTHING on the event. Every handler
 * that wants to signal "still working" uses this rather than anything the
 * model produced.
 */
export const WORKING_STATUS = "Working...";

/**
 * The completed assistant message — the actual answer, and the single
 * highest-risk public sink in the whole channel.
 *
 * The `finishReason === "tool-calls"` early return is REPRODUCED from eve's
 * default and is load-bearing: that branch fires on every mid-turn message
 * boundary where the model narrates before calling a tool. Omitting it would
 * deliver tool chatter on every turn instead of the answer once at the end.
 *
 * eve's default additionally stashes that narration in
 * `state.pendingToolCallMessage` for its `actions.requested` handler to render
 * as a typing status. This override deliberately does NOT: that field's only
 * reader was a public sink, so writing it was feeding model prose to the very
 * leak this file exists to close. `actionsRequested` below now overrides that
 * reader with a constant, and nothing writes the field.
 */
export const messageCompleted: NonNullable<SlackChannelEvents["message.completed"]> = async (
  event,
  ctx,
  callbackCtx,
) => {
  if (event.finishReason === "tool-calls") return;

  if (!event.message) {
    // No text to disclose. The typing indicator is public but content-free,
    // so it is kept: it is the only signal the user gets that work continues.
    await ctx.thread.startTyping();
    return;
  }

  await deliverToTriggeringUser(ctx, callbackCtx, event.message, "message.completed");
};

/**
 * The model's running reasoning. eve's default pushes its first non-empty line
 * into the public typing status; ours shows a constant.
 *
 * The reasoning trace is the least redacted thing a turn produces — it quotes
 * tool results and names resources verbatim, all under the caller's reach. The
 * typing indicator is kept (rather than no-op'd) because it is the only signal
 * the asker gets that work continues, and `WORKING_STATUS` reads nothing off
 * the event.
 */
export const reasoningAppended: NonNullable<SlackChannelEvents["reasoning.appended"]> = async (
  _event,
  ctx,
) => {
  await ctx.thread.startTyping(WORKING_STATUS);
};

/**
 * The model is about to call tools. eve's default renders either the model's
 * buffered narration or the tool names into the public typing status; ours
 * shows the same constant.
 *
 * Tool names are excluded too. They are drawn from `TEMPER_READ_TOOLS`, so
 * they are not model prose — but which tool the model reached for, and when,
 * is still a read of the caller's private turn, and a constant costs nothing.
 */
export const actionsRequested: NonNullable<SlackChannelEvents["actions.requested"]> = async (
  _event,
  ctx,
) => {
  await ctx.thread.startTyping(WORKING_STATUS);
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
    // The ONLY caller permitted the state fallback — see `StateFallback`. eve
    // gives this handler no `SessionContext`, so state is its only recipient
    // source, and its text is a fixed literal with nothing reach-derived in it.
    "allow-state-fallback",
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

/**
 * The handlers, ready to spread into `slackChannel({ events })`.
 *
 * Every eve default that reads model-derived text is here. `tests/events.test.ts`
 * asserts that against eve's actual `defaultEvents` at runtime, so this map
 * cannot silently fall behind an eve upgrade.
 */
export const ephemeralEvents: SlackChannelEvents = {
  "message.completed": messageCompleted,
  "reasoning.appended": reasoningAppended,
  "actions.requested": actionsRequested,
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
/**
 * Whether a handler may fall back to `ctx.state.triggeringUserId` when the
 * per-invocation auth context yields no recipient.
 *
 * **Only `session.failed` may.** A Slack session is keyed by `(channel, thread)`
 * (`slackContinuationToken`), so two different humans mentioning in the SAME
 * thread share one session and therefore one `state.triggeringUserId` — eve's
 * `turn.started` wrapper overwrites it with whoever authenticated most recently.
 * The state is thus not "this turn's asker"; it is "the last asker in this
 * thread". Addressing a content-bearing message with it can hand A's answer to B.
 *
 * The per-invocation path (`session.auth.current`, read out of the async-local
 * container by `buildCallbackContext`) is correct and is always preferred. The
 * fallback exists solely because eve invokes `session.failed` with TWO arguments
 * and no `SessionContext` (`buildAdapter`:
 * `e===\`session.failed\`?t(r,a):t(r,a,buildCallbackContext())`), so that handler
 * has no auth to read and state is its only source.
 *
 * That trade is acceptable ONLY because `session.failed`'s text is a fixed
 * literal carrying nothing reach-derived: the worst case is B learning that A's
 * session died. It would NOT be acceptable for `message.completed`, whose text
 * is the answer itself — so those handlers take the default and cannot reach the
 * fallback at all. This is a capability removed rather than a rule written down.
 */
type StateFallback = "allow-state-fallback" | "no-state-fallback";

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
  allowStateFallback: StateFallback = "no-state-fallback",
): Promise<void> {
  const current = callbackCtx?.session.auth.current;
  const auth =
    current !== null && typeof current === "object"
      ? (current as { attributes: Record<string, string | readonly string[]>; authenticator: string })
      : null;

  const fallback =
    allowStateFallback === "allow-state-fallback" ? ctx.state.triggeringUserId : undefined;
  const userId = resolveEphemeralRecipient(auth, fallback);
  if (userId === null) {
    console.warn("dropping event: no ephemeral recipient", { eventType });
    return;
  }

  await deliverEphemeral(ctx, userId, text);
}
