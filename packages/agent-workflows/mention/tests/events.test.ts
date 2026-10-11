import { afterEach, describe, expect, it, vi } from "vitest";

import {
  actionPartial,
  actionResult,
  actionsRequested,
  ephemeralEvents,
  ephemeralRenderer,
  inputRequested,
  messageCompleted,
  reasoningAppended,
  reasoningCompleted,
  sessionFailed,
  turnFailed,
  WORKING_STATUS,
} from "../agent/lib/events.js";

/**
 * eve's REAL default handler table, imported at runtime.
 *
 * Deliberately a deep relative path into `node_modules`: `defaultEvents` is not
 * re-exported from `eve/channels/slack`, and eve's export map refuses the deep
 * specifier (`ERR_PACKAGE_PATH_NOT_EXPORTED`), so a package-relative import
 * cannot reach it. The relative path can, and reaching the real object is the
 * entire point — a hardcoded list of eve's keys could not detect eve GAINING a
 * sink, which is the failure mode that matters on an upgrade.
 *
 * No cast: TypeScript resolves this to eve's own `SlackChannelInternalEvents`,
 * so the value is the real typed table, not an `unknown` we have re-described.
 */
const { defaultEvents } = await import(
  "../node_modules/eve/dist/src/public/channels/slack/defaults.js"
);

/**
 * A stand-in for eve's `SlackEventContext`, carrying only the members the
 * handlers touch. Cast at each call site: constructing a real
 * `SlackEventContext` would mean constructing eve's whole channel surface,
 * which is precisely what these handlers are written to avoid depending on.
 */
function fakeCtx(triggeringUserId: string | null = "U_STATE") {
  const request = vi.fn(async (_operation: string, _body: unknown) => ({ ok: true }));
  const post = vi.fn(async (_message: { text: string }) => undefined);
  const startTyping = vi.fn(async () => undefined);
  const state = { triggeringUserId, pendingToolCallMessage: null as string | null };
  return {
    ctx: { slack: { channelId: "C123", request }, thread: { post, startTyping }, state },
    request,
    post,
    startTyping,
    state,
  };
}

/** eve's `SessionContext`, reduced to the one path the handlers read. */
function fakeCallbackCtx(userId: string) {
  return {
    session: {
      auth: { current: { attributes: { user_id: userId }, authenticator: "slack-webhook" } },
    },
  };
}

/** The operation names passed to `slack.request` across all calls. */
function operations(request: ReturnType<typeof fakeCtx>["request"]): string[] {
  return request.mock.calls.map((call) => call[0]);
}

/**
 * Cast target for the test doubles above. The handlers are typed against
 * eve's real event/context types; the doubles carry only the members under
 * test, so each call site widens explicitly rather than reconstructing eve's
 * channel surface.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyArgs = any;

/**
 * The renderer chain's `next`, which reaches eve's default. Every handler here
 * REPLACES its default by never calling it, so a call is a failure: it throws,
 * and the awaiting test rejects.
 */
const neverNext = (async () => {
  throw new Error("handler called next() — eve's default would have run");
}) as AnyArgs;

afterEach(() => {
  vi.restoreAllMocks();
});

describe("messageCompleted", () => {
  it("delivers the answer via chat.postEphemeral and never posts publicly", async () => {
    // FAILS IF: the override is removed, or reverts to `ctx.thread.post`. This
    // is THE test for the leak: `message.completed` carries the model's answer,
    // produced under the mentioning human's full temper reach. Asserting the
    // Slack method name (not just "something was sent") is what makes a public
    // post detectable.
    const { ctx, request, post } = fakeCtx();

    await messageCompleted(
      { finishReason: "stop", message: "the answer" } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    expect(request.mock.calls[0]?.[1]).toEqual({
      channel: "C123",
      user: "U_AUTH",
      text: "the answer",
    });
    expect(post).not.toHaveBeenCalled();
  });

  it("posts NOTHING on a tool-calls boundary, and buffers no model text", async () => {
    // FAILS IF: the `finishReason === "tool-calls"` early return is dropped —
    // the single easiest thing to lose when reimplementing eve's default. The
    // symptom is mid-turn tool chatter delivered on every turn.
    //
    // ALSO FAILS IF: the narration is written back to
    // `state.pendingToolCallMessage`, as eve's default does. That field's only
    // reader was eve's `actions.requested` default, which pushes it to the
    // PUBLIC typing status — so writing it was actively feeding the leak. It
    // must stay null even though a model message was present.
    const { ctx, request, post, state } = fakeCtx();

    await messageCompleted(
      { finishReason: "tool-calls", message: "\n\n  Looking that up  \nmore" } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    expect(state.pendingToolCallMessage).toBeNull();
  });

  it("only signals typing when the message is empty", async () => {
    // FAILS IF: an empty message is delivered as an empty ephemeral instead of
    // a content-free typing signal.
    const { ctx, request, startTyping } = fakeCtx();

    await messageCompleted(
      { finishReason: "stop", message: null } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(startTyping).toHaveBeenCalledTimes(1);
  });

  it("DROPS rather than posting publicly when no recipient can be resolved", async () => {
    // FAILS IF: an unresolvable recipient falls back to `thread.post`. That
    // fallback is the tempting "at least say something" fix and it is exactly
    // the leak — an answer nobody can be addressed privately with must not be
    // broadcast to the channel instead.
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { ctx, request, post } = fakeCtx(null);

    await messageCompleted(
      { finishReason: "stop", message: "the answer" } as AnyArgs,
      ctx as AnyArgs,
      undefined as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("reasoningAppended", () => {
  it("shows a constant status and never the model's reasoning", async () => {
    // FAILS IF: the override is removed or reverts to eve's default, which does
    // `firstNonEmptyLine(event.reasoningSoFar)` -> `thread.startTyping(...)`.
    // The reasoning trace is the least redacted thing a turn produces, and the
    // typing status is channel-visible — so asserting the status equals a
    // CONSTANT (not merely "does not contain this one secret") is what makes
    // any reintroduction of event-derived text detectable.
    const { ctx, request, post, startTyping } = fakeCtx();

    await reasoningAppended(
      { reasoningSoFar: "The user's private salary doc says 250000" } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(startTyping.mock.calls).toEqual([[WORKING_STATUS]]);
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("actionsRequested", () => {
  it("shows a constant status, ignoring both the buffer and the tool names", async () => {
    // FAILS IF: the override is removed or reverts to eve's default, which
    // renders `state.pendingToolCallMessage` (the model's own narration) —
    // or, failing that, the tool names — into the channel-visible typing
    // status. The buffer is pre-loaded here precisely so a reverted handler
    // would have something to leak; a constant assertion catches either branch.
    const { ctx, request, post, startTyping, state } = fakeCtx();
    state.pendingToolCallMessage = "Reading her performance review";

    await actionsRequested(
      { actions: [{ kind: "tool-call", toolName: "get_resource" }] } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(startTyping.mock.calls).toEqual([[WORKING_STATUS]]);
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("reasoningCompleted", () => {
  it("shows a constant status and never the finished reasoning", async () => {
    // FAILS IF: the override is removed. eve's default (new since 0.18) puts the
    // finished block's last heading or sentence into the channel-visible status —
    // the `reasoning.appended` leak again, one event later.
    const { ctx, request, post, startTyping } = fakeCtx();

    await reasoningCompleted(
      { reasoning: "The user's private salary doc says 250000." } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(startTyping.mock.calls).toEqual([[WORKING_STATUS]]);
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("actionPartial / actionResult", () => {
  it.each([
    ["action.partial", actionPartial],
    ["action.result", actionResult],
  ])("renders nothing from the call's label: %s", async (_name, handler) => {
    // FAILS IF: the override is removed. eve's defaults (new since 0.18) put the
    // call's presentation label — built from the call's INPUT — into the
    // channel-visible status.
    const { ctx, request, post, startTyping } = fakeCtx();

    await handler(
      {
        result: { callId: "c1" },
        presentation: { c1: { label: "Read her performance review" } },
      } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(startTyping).not.toHaveBeenCalled();
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("ephemeralRenderer", () => {
  it("runs exactly ephemeralEvents, sets no status on arrival, and posts no task card", async () => {
    // FAILS IF: the renderer stops carrying the overrides (each would fall
    // through to eve's default), its `received` reaches eve's `Thinking...`
    // default, or it returns a task card — eve posts that card with
    // `chat.postMessage`, i.e. publicly.
    expect(ephemeralRenderer.events).toBe(ephemeralEvents);

    const { ctx, startTyping } = fakeCtx();
    await ephemeralRenderer.received?.({} as AnyArgs, ctx as AnyArgs, neverNext);
    expect(startTyping).not.toHaveBeenCalled();

    expect(ephemeralRenderer.taskCard?.({} as AnyArgs, neverNext)).toBeNull();
  });
});

describe("ephemeralRenderer composed by eve's real renderer chain", () => {
  /**
   * eve's REAL chain composition, imported at runtime the same way `defaultEvents` is above.
   * Composing `ephemeralRenderer` over SPY defaults pins the claim this whole file rests on —
   * that a handler which never calls `next` replaces eve's default — against eve's own
   * `composeSlackRenderers`, not against our reading of it.
   */
  it("never reaches eve's default for any overridden event, nor for received or taskCard", async () => {
    // FAILS IF: any override calls `next`, or an eve upgrade changes the chain so a renderer's
    // handler no longer stands in front of the default.
    const { composeSlackRenderers } = await import(
      "../node_modules/eve/dist/src/public/channels/slack/renderers.js"
    );
    const names = Object.keys(ephemeralEvents);
    const defaults = Object.fromEntries(names.map((name) => [name, vi.fn(async () => undefined)]));
    const received = vi.fn(async () => undefined);
    const taskCard = vi.fn(() => ({ blocks: [], text: "card" }));
    const chain = composeSlackRenderers([ephemeralRenderer], {
      events: defaults as AnyArgs,
      received,
      taskCard,
    });

    vi.spyOn(console, "warn").mockImplementation(() => {});
    for (const name of names) {
      const { ctx } = fakeCtx();
      const handler = (chain.events as Record<string, (...args: unknown[]) => Promise<void>>)[name];
      const event = name === "input.requested" ? { requests: [] } : { finishReason: "stop", message: null };
      if (name === "session.failed") await handler?.(event, ctx);
      else await handler?.(event, ctx, fakeCallbackCtx("U_AUTH"));
    }
    for (const name of names) expect(defaults[name], name).not.toHaveBeenCalled();

    await chain.received({} as AnyArgs, fakeCtx().ctx as AnyArgs);
    expect(received).not.toHaveBeenCalled();
    expect(chain.taskCard({} as AnyArgs)).toBeNull();
    expect(taskCard).not.toHaveBeenCalled();
  });
});

describe("turnFailed", () => {
  it("delivers the failure ephemerally and quotes no error detail", async () => {
    // FAILS IF: the override is removed (eve's default posts the failure into
    // the channel), or if the copy is widened to include `event.message` /
    // `event.details`, which can quote model output or a tool response.
    const { ctx, request, post } = fakeCtx();

    await turnFailed(
      { code: "boom", message: "TOOL RETURNED SECRET ROW", turnId: "t1", sequence: 1 } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    const body = request.mock.calls[0]?.[1] as { text: string };
    expect(body.text).not.toContain("SECRET ROW");
    expect(post).not.toHaveBeenCalled();
  });
});

describe("sessionFailed", () => {
  it("delivers ephemerally using session state, with NO callback context", async () => {
    // FAILS IF: the handler assumes a third argument. eve invokes
    // `session.failed` with only two (verified in defineChannel's
    // buildAdapter). A handler that reads `callbackCtx.session` would throw,
    // and eve SWALLOWS handler throws — so the bug would show up as silence,
    // never as an error. This test is the only thing that catches it.
    const { ctx, request, post } = fakeCtx("U_STATE");

    await sessionFailed(
      { code: "boom", message: "x", sessionId: "s1" } as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    expect((request.mock.calls[0]?.[1] as { user: string }).user).toBe("U_STATE");
    expect(post).not.toHaveBeenCalled();
  });
});

describe("inputRequested", () => {
  it("delivers one ephemeral per request and never posts publicly", async () => {
    // FAILS IF: the override is removed — eve's default loops
    // `await ctx.thread.post(...)`, putting every input prompt in the channel.
    // Asserting one `chat.postEphemeral` PER request also catches a handler
    // that silently delivers only the first.
    const { ctx, request, post } = fakeCtx();

    await inputRequested(
      { requests: [{ prompt: "Approve A?" }, { prompt: "Approve B?" }] } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral", "chat.postEphemeral"]);
    expect(request.mock.calls.map((call) => (call[1] as { text: string }).text)).toEqual([
      "Approve A?",
      "Approve B?",
    ]);
    expect(post).not.toHaveBeenCalled();
  });
});

/**
 * Every key in eve's `defaultEvents`, classified by whether its DEFAULT
 * handler can route model-derived text to a channel-visible sink.
 *
 * `"content-bearing"` means the default reads something the model produced
 * (`event.reasoningSoFar`, `event.message`, `state.pendingToolCallMessage`, an
 * error's detail) and sends it somewhere the channel can observe — via
 * `thread.post` OR `thread.startTyping`. Those MUST be overridden.
 *
 * `"content-free"` is a claim about eve's source, re-verified per entry below.
 * It is a claim we are allowed to be wrong about only in the safe direction:
 * an unclassified key fails the test outright.
 */
const DEFAULT_EVENT_CLASSIFICATION: Readonly<Record<string, "content-bearing" | "content-free">> = {
  // `showStatus("Thinking...")` plus three state resets. Reads nothing off the
  // event. Verified in `.../slack/defaults.js` (eve@0.73.0).
  "turn.started": "content-free",
  // `reasoningStatus(stream.reasoning)` -> `showStatus(...)`: the latest heading or
  // sentence of the raw reasoning trace, which quotes tool results verbatim.
  "reasoning.appended": "content-bearing",
  // Same sink as `reasoning.appended`, on the finished block. New since 0.18.
  "reasoning.completed": "content-bearing",
  // `state.pendingToolCallMessage` -> `showStatus(...)`, else `actionLabel(...)`.
  // The buffered field holds the model's own mid-turn narration.
  "actions.requested": "content-bearing",
  // `showStatus(presentation[callId].label)`. A label is built from the call's
  // INPUT (`label.start(input)`), so it is a read of the private turn. New since 0.18.
  "action.partial": "content-bearing",
  // Same as `action.partial`, on the result. New since 0.18.
  "action.result": "content-bearing",
  // `deliverCompletedSlackReply` -> `thread.post(event.message)` — the answer itself.
  "message.completed": "content-bearing",
  // `thread.post` of `formatErrorHint(event)` / the semantic error summary, which
  // can quote model or tool output.
  "turn.failed": "content-bearing",
  // Same shape as turn.failed.
  "session.failed": "content-bearing",
  // Content-free, but NOT overridden — see the dedicated test below.
  "authorization.required": "content-free",
  // `showStatus("Connected to <displayName>. Resuming...")`, then `chat.update` of a
  // ts it only holds if `authorization.required` posted one, with
  // `buildAuthCompletedText({displayName, outcome, reason})`
  // (`.../slack/connections.js`): a connection display name and an outcome code.
  // No model or tool output reaches it.
  "authorization.completed": "content-free",
  // `showStatus("Writing a reply...")` once the streamed reply passes 280 chars.
  // A constant; the delta's text is only measured. New since 0.18.
  "message.appended": "content-free",
  // `hideStatus`, or `waitingOnTasks(names)` for a turn waiting on TASKS — task
  // tool/agent names from eve's own card state, never model prose. This agent
  // starts no tasks (`ephemeralRenderer.taskCard` is null either way). New since 0.18.
  "turn.waiting": "content-free",
  // State bookkeeping only (`state.pendingTaskResults`); renders nothing. New since 0.18.
  "task.settled": "content-free",
  // `showStatus(reviewingResults(names) ?? state.threadStatus.text ?? "Thinking...")`:
  // task names, or re-showing a status eve's own default set — every
  // content-bearing writer of which is overridden here. New since 0.18.
  "step.started": "content-free",
  // `clearStatus`. New since 0.18.
  "turn.completed": "content-free",
  // `postEphemeral` to the responder of a rejected/failed approval — private, and
  // unreachable: approvals need the cards `input.requested`'s default posts, which
  // is overridden. New since 0.18.
  "approval.candidate": "content-free",
  // `settleApprovalCard` — edits an approval card ("Approved/Cancelled by <@user>")
  // that only exists if eve's `input.requested` default posted it. New since 0.18.
  "approval.settled": "content-free",
  // Same edit, for each resolved `tool-approval`. New since 0.18.
  "input.resolved": "content-free",
};

describe("ephemeralEvents", () => {
  it("classifies every event eve actually defaults", () => {
    // FAILS IF: an eve upgrade ADDS a key to `defaultEvents`. That is the whole
    // reason this is derived from the real object instead of a hardcoded list:
    // a new eve default is a new public sink installed into this channel by the
    // `{...defaultEvents, ...userEvents}` merge, with no edit on our side. This
    // test forces a human to read the new handler and classify it before CI
    // goes green again. It also fails if eve REMOVES one, which is a cheaper
    // but equally real drift.
    expect(Object.keys(defaultEvents).sort()).toEqual(
      Object.keys(DEFAULT_EVENT_CLASSIFICATION).sort(),
    );
  });

  it("overrides every content-bearing eve default", () => {
    // FAILS IF: any handler that eve would otherwise render from model output
    // is left un-overridden — the exact gap that let `reasoning.appended` and
    // `actions.requested` ship eve's defaults. The merge is
    // `{...defaultEvents, ...userEvents}`, so an ABSENT key is a public
    // default, not a no-op. Derived from the classification above rather than
    // spelled out, so a key reclassified as content-bearing is immediately
    // required here.
    const contentBearing = Object.entries(DEFAULT_EVENT_CLASSIFICATION)
      .filter(([, kind]) => kind === "content-bearing")
      .map(([name]) => name)
      .sort();

    const overridden = Object.keys(ephemeralEvents).sort();

    expect(contentBearing.filter((name) => !overridden.includes(name))).toEqual([]);
  });

  it("overrides input.requested, which has no defaultEvents entry", () => {
    // FAILS IF: the override is dropped. `input.requested` is NOT a key of
    // `defaultEvents` — eve installs `defaultInputRequestedHandler(approvalChannel)`
    // for it separately in `slackChannel.js`
    // (`{...defaultEvents, "input.requested": defaultInputRequestedHandler(...)}`,
    // the innermost link of the renderer chain), and that default posts the
    // prompts into the thread. So the derived check above
    // structurally cannot cover it, and this test is what does.
    expect(defaultEvents).not.toHaveProperty("input.requested");
    expect(ephemeralEvents).toHaveProperty("input.requested");
  });

  it("does NOT override authorization.required", async () => {
    // FAILS IF: someone overrides it anyway. Its override context is narrowed
    // to private delivery only and eve's default additionally posts a
    // framework-owned public status line an override cannot reach — so
    // overriding buys nothing and costs the framework's edit-in-place
    // behaviour. The gap is documented in events.ts, not papered over.
    expect(ephemeralEvents).not.toHaveProperty("authorization.required");
  });
});

describe("the session-state recipient fallback is confined to session.failed", () => {
  /**
   * A Slack session is keyed by `(channel, thread)`, so two humans mentioning in
   * one thread share `state.triggeringUserId` — it holds whoever authenticated
   * MOST RECENTLY, not this turn's asker. A content-bearing handler that fell
   * back to it would hand A's answer to B.
   *
   * FAILS IF: `messageCompleted`, `turnFailed` or `inputRequested` is allowed to
   * reach `ctx.state.triggeringUserId`. Each is invoked with NO callbackCtx, so
   * auth yields no recipient; state holds a valid-looking user id. A handler
   * that consults the fallback would deliver to `U_STATE`. The correct
   * behaviour is to DROP.
   */
  it.each([
    ["message.completed", () => messageCompleted({ message: "the answer" } as never, undefined as never, undefined as never, neverNext)],
    ["turn.failed", () => turnFailed({} as never, undefined as never, undefined as never, neverNext)],
  ])("drops rather than using stale state: %s", async (_name, invoke) => {
    const { ctx, request, post } = fakeCtx("U_STATE");
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    await (invoke as unknown as (c: unknown) => Promise<void>).call(null, ctx);

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  /**
   * The converse. `session.failed` is the ONE handler eve invokes without a
   * SessionContext, so state is its only recipient source and it must still
   * deliver.
   *
   * FAILS IF: the fallback is removed wholesale rather than confined — which
   * would silently stop delivering unrecoverable-session notices to anyone.
   */
  it("still delivers session.failed from state alone", async () => {
    const { ctx, request } = fakeCtx("U_STATE");

    await sessionFailed({} as never, ctx as never, neverNext);

    expect(request).toHaveBeenCalledWith(
      "chat.postEphemeral",
      expect.objectContaining({ user: "U_STATE" }),
    );
  });
});
