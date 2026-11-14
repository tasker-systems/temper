import { defineSlackRenderer } from "eve/channels/slack";
import type { SlackEventContext, SlackRendererEvents } from "eve/channels/slack";

import { deliverEphemeral, resolveEphemeralRecipient } from "./ephemeral.js";

/**
 * Ephemeral (or content-free) replacements for every eve Slack default that
 * routes MODEL-DERIVED TEXT to a sink the whole channel can observe.
 *
 * ## Why this file exists
 *
 * A dispatched turn runs under the mentioning human's FULL temper reach,
 * including their personal contexts. eve's defaults render turn output with
 * `thread.post`, which is visible to the whole channel. Verified against
 * eve@0.91.0 in `eve/dist/src/public/channels/slack/defaults.js` (minified):
 * `content.completed` ends a reply in `deliverCompletedSlackReply`, and
 * `turn.settled` / `session.ended` each end a failure in `await t.thread.post(...)`
 * carrying the error's message and hint. `defaultInteractionEvents`'s
 * `interaction.opened` posts approval and question cards into the thread
 * (`postThreadCards` → `thread.post`). So these sinks must be private BEFORE any
 * dispatch exists, not after.
 *
 * ## `thread.post` is not the only public sink — the thread status is one too
 *
 * Several defaults push model-derived text into the thread's typing status
 * (`showStatus` → `thread.startTyping`), which is NOT private:
 *
 *   "content.delta"     reasoning deltas → `reasoningStatus(stream.reasoning)`
 *   "content.completed" a finished reasoning block → the same
 *   "call.requested"    `state.pendingToolCallMessage` (the model's own narration,
 *                       stashed by `content.completed`'s narration branch) or
 *                       `callLabel(call)`
 *   "call.progress"     `data.label`
 *   "call.settled"      `data.label`
 *
 * The reasoning trace quotes tool results verbatim; the narration is model
 * prose; a call label is built from the call's validated INPUT. All three are
 * produced under the mentioning human's reach, so each handler below shows the
 * CONSTANT `WORKING_STATUS` or nothing.
 *
 * `startTyping` currently resolves to `assistant.threads.setStatus`, which is
 * scoped to assistant threads and may well no-op in an ordinary public channel
 * — but that is a Slack-side accident, not a property of this code, and Slack
 * keeps widening agent surfaces into channels. The overrides do not depend on
 * it.
 *
 * ## The defaults deliberately left in place
 *
 * Each is content-free as eve 0.91 writes it, and `tests/events.test.ts` derives
 * that classification against eve's real default tables so a release that adds
 * or changes a key FAILS rather than silently opening a sink:
 *
 * - `turn.started`: three state resets and `showStatus("Thinking...")`.
 * - `turn.paused`: `hideStatus`.
 * - `model.started`: re-shows `state.threadStatus.text`, or `reviewingResults` of
 *   the task names `call.settled` recorded. Every handler here that sets a
 *   status calls `startTyping` directly and never writes `threadStatus`, and
 *   `callSettled` below records no task results, so this only ever re-shows
 *   `turn.started`'s constant.
 * - `model.settled`: `waitingOnTasks(names)` for a turn waiting on TASKS — task
 *   tool/agent names from eve's own tables, never model prose. This agent starts
 *   no tasks.
 * - `response.admitted`, `response.settled`, `interaction.settled`: edit or settle
 *   an approval/question card eve recorded in `state.interactionCards` when it
 *   posted one. `interactionOpened` below posts none, so they find nothing. A
 *   refused responder is told why with `postEphemeral` (private). A SIGN-IN's
 *   settlement shows `Connected to <displayName>. Resuming...` and edits its
 *   public, link-free line with an outcome code
 *   (`buildAuthCompletedText`, `.../slack/connections.js`).
 *
 * ## A handler that never calls `next` REPLACES the default
 *
 * Each handler is one link of a renderer chain (`slackChannel({ renderers })`)
 * and receives a `next` that runs the rest of it, ending with eve's default:
 *
 *   runChain(a, e, (e,n,i)=>e(n,userContext(r,t,n),i), e=>i?.(e,t))
 *   -- eve/dist/src/public/channels/slack/renderers.js, composeEvents
 *
 * The default (`i`) is reached only through `next`, so a handler that never
 * calls it is the ONLY thing that runs for its event. Anything the default did
 * that still matters is reproduced here. `interactionOpened` calls `next` for
 * a sign-in alone (see there).
 *
 * eve still owns two things outside the chain: if `content.completed` THROWS it
 * posts a content-free "couldn't deliver my answer" notice (`docs/channels/
 * slack.mdx`, "Customize rendering"), and it posts whatever card `taskCard`
 * returns — which is why `ephemeralRenderer` below returns none.
 *
 * ## Failure mode to keep in mind
 *
 * eve logs and swallows a handler that throws. A bug here therefore fails as
 * SILENCE, not as a leak — safe, but invisible (except for `content.completed`,
 * where eve adds its content-free notice). That is exactly why these paths are
 * unit-tested rather than eyeballed.
 *
 * ## Residual gap: a sign-in still posts eve's public status line
 *
 * Deliberately. A sign-in's `interaction.opened` context is narrowed to private
 * delivery only (`SlackAuthorizationEventContext`: no public `post`, no
 * `slack.request`), and eve's default additionally posts a framework-owned,
 * link-free public status (`buildAuthRequiredPublicText`) that it edits on
 * `interaction.settled`. The challenge itself goes privately. So the residual
 * disclosure is "this agent asked someone to connect an account" — documented
 * here rather than papered over. Not suppressed, because suppressing it means
 * re-delivering the challenge ourselves and losing the edit-in-place, and
 * `getToken` never opens a sign-in anyway — see CLAUDE.md.
 */

/**
 * The one status string this agent ever shows publicly, besides eve's own
 * equally constant `Thinking...` (`turn.started`).
 *
 * Constant by construction: it is derived from NOTHING on the event. Every
 * handler that wants to signal "still working" uses this rather than anything
 * the model produced.
 */
export const WORKING_STATUS = "Working...";

/** The context a renderer handler receives, as this file reads it. */
type RenderContext = {
  readonly channel: SlackEventContext;
  readonly session: { readonly auth: { readonly current: unknown } };
};

/**
 * A completed content part. Only a TEXT part in the `reply` phase is the
 * answer, the single highest-risk public sink in the whole channel.
 *
 * - A `reasoning` part is the finished reasoning block, whose last heading or
 *   sentence eve's default puts into the public status. Ours shows the constant.
 * - A text part in any phase other than `reply` is NARRATION: the model talking
 *   before it calls a tool. Skipping it is REPRODUCED from eve's default and is
 *   load-bearing — omitting it would deliver tool chatter on every turn instead
 *   of the answer once at the end. eve's own test for this is
 *   `contentPhase(phase) === "narration"`, which treats every phase except the
 *   exact string `reply` as narration; checking `!== "reply"` here keeps the
 *   same fail-closed direction.
 *
 *   eve's default also stashes that narration in `state.pendingToolCallMessage`
 *   for its `call.requested` handler to put in the status. This override
 *   deliberately does NOT: that field's only reader was a public sink.
 *   `callRequested` below overrides that reader too, and nothing writes it.
 * - A reply with no string `value` (empty, or stored by `valueRef`) delivers
 *   nothing, as eve's default does, and clears the status.
 */
export const contentCompleted: NonNullable<SlackRendererEvents["content.completed"]> = async (
  event,
  ctx,
) => {
  const { kind, phase, value } = event.data;

  if (kind === "reasoning") {
    await ctx.channel.thread.startTyping(WORKING_STATUS);
    return;
  }
  if (kind !== "text" || phase !== "reply") return;

  if (typeof value !== "string" || value.length === 0) {
    await ctx.channel.thread.startTyping();
    return;
  }

  await deliverToTriggeringUser(ctx, value, "content.completed");
};

/**
 * A streamed content delta. eve's default pushes the running reasoning into the
 * public status, and measures a text reply to show `Writing a reply...`.
 *
 * Rendered as nothing. The status `turn.started` / `model.started` put up stays
 * on screen, so the asker still sees that work continues, and a per-delta
 * `startTyping` would be a Slack API call per streamed token.
 */
export const contentDelta: NonNullable<SlackRendererEvents["content.delta"]> = async () => {};

/**
 * The model is calling a tool. eve's default shows the model's buffered
 * narration, or the call's label (built from its input), in the public status;
 * ours shows the constant.
 *
 * Tool names are excluded too. They are drawn from `TEMPER_READ_TOOLS`, so they
 * are not model prose — but which tool the model reached for, and when, is still
 * a read of the caller's private turn, and a constant costs nothing.
 */
export const callRequested: NonNullable<SlackRendererEvents["call.requested"]> = async (
  _event,
  ctx,
) => {
  await ctx.channel.thread.startTyping(WORKING_STATUS);
};

/**
 * A tool's progress or settlement. eve's defaults show the call's label — built
 * from the call's INPUT — in the public status. Rendered as nothing: the
 * `WORKING_STATUS` that `callRequested` set stays up.
 *
 * `call.settled`'s default also records finished TASK names in
 * `state.pendingTaskResults` for `model.started` to show. Not reproduced: this
 * agent starts no tasks, and leaving the field unset is what keeps the kept
 * `model.started` default content-free.
 */
export const callProgress: NonNullable<SlackRendererEvents["call.progress"]> = async () => {};
export const callSettled: NonNullable<SlackRendererEvents["call.settled"]> = async () => {};

/**
 * A settled turn. eve's default clears the status, and for a FAILED turn posts
 * the error's message and hint into the channel. Ours clears the status for
 * every outcome and tells only the asker that it failed.
 *
 * Error text can quote the model's own output or a tool's response, both of
 * which are reach-derived. So the ephemeral says only that it failed.
 */
export const turnSettled: NonNullable<SlackRendererEvents["turn.settled"]> = async (event, ctx) => {
  await ctx.channel.thread.startTyping();
  if (event.data.outcome !== "failed") return;

  await deliverToTriggeringUser(
    ctx,
    [
      "I hit an error while handling your request.",
      "",
      "Please try again, rephrase, or reach out if it keeps failing.",
    ].join("\n"),
    "turn.settled",
  );
};

/**
 * An ended session. eve's default posts a FAILED session's error into the
 * channel; ours tells the last asker, privately, with fixed text.
 *
 * The ONE handler allowed the session-state recipient fallback — see
 * `StateFallback`. A session can end outside any turn, with no current auth to
 * address, and its text is a fixed literal with nothing reach-derived in it.
 */
export const sessionEnded: NonNullable<SlackRendererEvents["session.ended"]> = async (event, ctx) => {
  if (event.data.outcome !== "failed") return;

  await deliverToTriggeringUser(
    ctx,
    [
      "This session couldn't recover from an error.",
      "",
      "Start a new thread to continue — I can't pick this one back up.",
    ].join("\n"),
    "session.ended",
    "allow-state-fallback",
  );
};

/**
 * The harness is asking a human to answer something before it can continue.
 *
 * For an approval or question, eve's default posts interactive Block Kit cards
 * into the thread. This override delivers the prompt as ephemeral TEXT: an
 * interactive card would need its own private-response wiring, and the prompt
 * itself can quote the turn's context, so it cannot stay public in the
 * meantime. Answering interactively is not wired — nothing in this agent asks
 * for input yet (the `temper` connection's approval is `never`).
 *
 * A SIGN-IN calls `next`, the one handler here that does. eve hands it only the
 * private delivery surface (no public `post`), its default delivers the
 * challenge — a credential — ephemerally, and its one public line is link-free.
 * See "Residual gap" above.
 */
export const interactionOpened: NonNullable<SlackRendererEvents["interaction.opened"]> = async (
  event,
  ctx,
  next,
) => {
  if (event.data.request.kind === "sign-in") {
    await next();
    return;
  }
  // Every other kind gets the full channel context (`SlackRendererEvents`'s
  // doc). Checked anyway rather than cast, so a change there drops the prompt
  // instead of reaching for a surface that is not there.
  if (!("slack" in ctx.channel)) {
    console.warn("dropping event: no private delivery surface", { eventType: "interaction.opened" });
    return;
  }

  await deliverToTriggeringUser(
    { channel: ctx.channel, session: ctx.session },
    event.data.request.prompt,
    "interaction.opened",
  );
};

/**
 * The handlers, keyed by event, for `ephemeralRenderer` below.
 *
 * Every eve default that reads model-derived text is here. `tests/events.test.ts`
 * asserts that against eve's actual default tables at runtime, so this map
 * cannot silently fall behind an eve upgrade.
 */
export const ephemeralEvents: SlackRendererEvents = {
  "content.completed": contentCompleted,
  "content.delta": contentDelta,
  "call.requested": callRequested,
  "call.progress": callProgress,
  "call.settled": callSettled,
  "turn.settled": turnSettled,
  "session.ended": sessionEnded,
  "interaction.opened": interactionOpened,
};

/**
 * The renderer `slackChannel({ renderers })` runs, outermost and only.
 *
 * Besides `ephemeralEvents` it replaces two non-event links of the chain, both
 * to keep what 0.18 rendered:
 *
 * - `received` — eve's default sets a `Thinking...` status the moment a mention
 *   arrives, while `onAppMention` is still deciding, and clears it on a drop.
 *   Content-free, but new: under 0.18 a supplied `onAppMention` replaced the
 *   default's typing indicator outright, and ours sets none, so a dropped
 *   mention (bot, unlinked, refused) showed nothing public at all. A no-op
 *   keeps it that way; a dispatched turn still gets `turn.started`'s status.
 * - `taskCard` — eve posts a PUBLIC card (`chat.postMessage`) for any turn that
 *   calls a tool, and a renderer may draw it from the turn's tool calls and
 *   their input. No card existed in 0.18; returning `null` writes none.
 */
export const ephemeralRenderer = defineSlackRenderer({
  received: async () => {},
  events: ephemeralEvents,
  taskCard: () => null,
});

/**
 * Whether a handler may fall back to `state.triggeringUserId` when the
 * session's current auth yields no recipient.
 *
 * **Only `session.ended` may.** A Slack session is keyed by `(channel, thread)`
 * (`slackContinuationToken`), so two different humans mentioning in the SAME
 * thread share one session and therefore one `state.triggeringUserId` — eve
 * overwrites it with whoever triggered most recently (its own doc on
 * `SlackChannelState.triggeringUserId`: "a later message can change it"). The
 * state is thus not "this turn's asker"; it is "the last asker in this thread".
 * Addressing a content-bearing message with it can hand A's answer to B.
 *
 * The per-invocation path (`ctx.session.auth.current`) is correct and always
 * preferred. The fallback exists solely for a session that ends with no current
 * auth to read.
 *
 * That trade is acceptable ONLY because `session.ended`'s text is a fixed
 * literal carrying nothing reach-derived: the worst case is B learning that A's
 * session died. It would NOT be acceptable for `content.completed`, whose text is
 * the answer itself — so every other handler takes the default and cannot reach
 * the fallback at all. This is a capability removed rather than a rule written
 * down.
 */
type StateFallback = "allow-state-fallback" | "no-state-fallback";

/**
 * Shared body of every override: resolve the recipient, then deliver privately.
 *
 * When no recipient can be resolved the message is DROPPED, not posted
 * publicly. That is the whole point of this file — a fallback to `thread.post`
 * here would reintroduce exactly the leak these handlers exist to close. The
 * drop is logged so it stays diagnosable.
 */
async function deliverToTriggeringUser(
  ctx: RenderContext,
  text: string,
  eventType: string,
  allowStateFallback: StateFallback = "no-state-fallback",
): Promise<void> {
  const current = ctx.session.auth.current;
  const auth =
    current !== null && typeof current === "object"
      ? (current as { attributes: Record<string, string | readonly string[]>; authenticator: string })
      : null;

  const fallback =
    allowStateFallback === "allow-state-fallback" ? ctx.channel.state.triggeringUserId : undefined;
  const userId = resolveEphemeralRecipient(auth, fallback);
  if (userId === null) {
    console.warn("dropping event: no ephemeral recipient", { eventType });
    return;
  }

  await deliverEphemeral(ctx.channel, userId, text);
}
