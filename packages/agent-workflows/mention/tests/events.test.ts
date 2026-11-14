import { afterEach, describe, expect, it, vi } from "vitest";

import {
  callProgress,
  callRequested,
  callSettled,
  contentCompleted,
  contentDelta,
  ephemeralEvents,
  ephemeralRenderer,
  interactionOpened,
  sessionEnded,
  turnSettled,
  WORKING_STATUS,
} from "../agent/lib/events.js";

/**
 * eve's REAL default handler tables, imported at runtime.
 *
 * Deliberately a deep relative path into `node_modules`: neither table is
 * re-exported from `eve/channels/slack`, and eve's export map refuses the deep
 * specifier (`ERR_PACKAGE_PATH_NOT_EXPORTED`), so a package-relative import
 * cannot reach them. The relative path can, and reaching the real objects is the
 * entire point — a hardcoded list of eve's keys could not detect eve GAINING a
 * sink, which is the failure mode that matters on an upgrade.
 *
 * `defaultEvents` is the innermost link for session events.
 * `defaultInteractionEvents(approvalChannel, interactionCard)` builds the
 * innermost link for the four interaction events; `slackChannel` spreads it over
 * `defaultEvents`, so the two together are every default this channel can run.
 */
const { defaultEvents, defaultInteractionEvents } = await import(
  "../node_modules/eve/dist/src/public/channels/slack/defaults.js"
);

/**
 * A stand-in for eve's `ChannelEventContext<SlackEventContext>`, carrying only
 * the members the handlers touch. Cast at each call site: constructing a real
 * one would mean constructing eve's whole channel surface, which is precisely
 * what these handlers are written to avoid depending on.
 *
 * `authUserId` is the Slack user on `session.auth.current`, the per-invocation
 * recipient; `null` means the session has no current auth.
 */
function fakeCtx(
  options: { authUserId?: string | null; triggeringUserId?: string | null } = {},
) {
  const { authUserId = "U_AUTH", triggeringUserId = "U_STATE" } = options;
  const request = vi.fn(async (_operation: string, _body: unknown) => ({ ok: true }));
  const post = vi.fn(async (_message: { text: string }) => undefined);
  const startTyping = vi.fn(async (_status?: string) => undefined);
  const state = { triggeringUserId, pendingToolCallMessage: null as string | null };
  const current =
    authUserId === null
      ? null
      : { attributes: { user_id: authUserId }, authenticator: "slack-webhook" };
  return {
    ctx: {
      channel: { slack: { channelId: "C123", request }, thread: { post, startTyping }, state },
      session: { auth: { current } },
    },
    request,
    post,
    startTyping,
    state,
  };
}

/** A session event in eve's v27 envelope. */
function fact(type: string, data: Record<string, unknown>) {
  return { type, data, scope: { turnId: "t1", runId: "r1" } };
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
 * except a sign-in REPLACES its default by never calling it, so a call is a
 * failure: it throws, and the awaiting test rejects.
 */
const neverNext = (async () => {
  throw new Error("handler called next() — eve's default would have run");
}) as AnyArgs;

afterEach(() => {
  vi.restoreAllMocks();
});

describe("contentCompleted", () => {
  it("delivers the reply via chat.postEphemeral and never posts publicly", async () => {
    // FAILS IF: the override is removed, or reverts to a public post. This is
    // THE test for the leak: the `reply` text part is the model's answer,
    // produced under the mentioning human's full temper reach. Asserting the
    // Slack method name (not just "something was sent") is what makes a public
    // post detectable.
    const { ctx, request, post } = fakeCtx();

    await contentCompleted(
      fact("content.completed", { kind: "text", phase: "reply", value: "the answer" }) as AnyArgs,
      ctx as AnyArgs,
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

  it.each([["narration"], ["some-future-phase"]])(
    "posts NOTHING for a %s text part, and buffers no model text",
    async (phase) => {
      // FAILS IF: the narration skip is dropped — the single easiest thing to lose
      // when reimplementing eve's default. The symptom is mid-turn tool chatter
      // delivered on every turn. The unknown phase pins the fail-closed direction:
      // eve treats every phase but `reply` as narration, and so must this.
      //
      // ALSO FAILS IF: the narration is written to `state.pendingToolCallMessage`,
      // as eve's default does. That field's only reader is eve's `call.requested`
      // default, which pushes it to the PUBLIC status — so writing it is actively
      // feeding the leak.
      const { ctx, request, post, startTyping, state } = fakeCtx();

      await contentCompleted(
        fact("content.completed", { kind: "text", phase, value: "Looking that up" }) as AnyArgs,
        ctx as AnyArgs,
        neverNext,
      );

      expect(request).not.toHaveBeenCalled();
      expect(post).not.toHaveBeenCalled();
      expect(startTyping).not.toHaveBeenCalled();
      expect(state.pendingToolCallMessage).toBeNull();
    },
  );

  it("shows a constant status for finished reasoning, never the reasoning", async () => {
    // FAILS IF: a reasoning part reaches eve's default, which puts the block's
    // last heading or sentence into the channel-visible status. Asserting the
    // status equals a CONSTANT (not merely "does not contain this one secret")
    // makes any reintroduction of event-derived text detectable.
    const { ctx, request, post, startTyping } = fakeCtx();

    await contentCompleted(
      fact("content.completed", {
        kind: "reasoning",
        phase: "reply",
        value: "The user's private salary doc says 250000.",
      }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(startTyping.mock.calls).toEqual([[WORKING_STATUS]]);
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });

  it.each([
    ["an empty value", { value: "" }],
    ["a value stored by reference", { valueRef: { id: "v1" } }],
  ])("only clears the status for %s", async (_name, value) => {
    // FAILS IF: a reply with no inline text is delivered as an empty ephemeral
    // instead of the content-free status clear eve's default does.
    const { ctx, request, startTyping } = fakeCtx();

    await contentCompleted(
      fact("content.completed", { kind: "text", phase: "reply", ...value }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(startTyping.mock.calls).toEqual([[]]);
  });

  it("DROPS rather than posting publicly when no recipient can be resolved", async () => {
    // FAILS IF: an unresolvable recipient falls back to `thread.post`. That
    // fallback is the tempting "at least say something" fix and it is exactly
    // the leak — an answer nobody can be addressed privately with must not be
    // broadcast to the channel instead.
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { ctx, request, post } = fakeCtx({ authUserId: null, triggeringUserId: null });

    await contentCompleted(
      fact("content.completed", { kind: "text", phase: "reply", value: "the answer" }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("contentDelta", () => {
  it("renders nothing from a streamed reasoning delta", async () => {
    // FAILS IF: the override is removed. eve's default accumulates reasoning
    // deltas and shows the latest sentence in the channel-visible status.
    const { ctx, request, post, startTyping } = fakeCtx();

    await contentDelta(
      fact("content.delta", {
        partId: "p1",
        kind: "reasoning",
        delta: "The user's private salary doc says 250000.",
      }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(startTyping).not.toHaveBeenCalled();
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("callRequested", () => {
  it("shows a constant status, ignoring both the buffer and the call's label", async () => {
    // FAILS IF: the override is removed or reverts to eve's default, which
    // renders `state.pendingToolCallMessage` (the model's own narration) — or,
    // failing that, the call's label — into the channel-visible status. The
    // buffer is pre-loaded here precisely so a reverted handler would have
    // something to leak; a constant assertion catches either branch.
    const { ctx, request, post, startTyping, state } = fakeCtx();
    state.pendingToolCallMessage = "Reading her performance review";

    await callRequested(
      fact("call.requested", {
        callId: "c1",
        capability: { kind: "tool", name: "temper__get_resource" },
        label: "Read her performance review",
        owner: { runId: "r1" },
      }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(startTyping.mock.calls).toEqual([[WORKING_STATUS]]);
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("callProgress / callSettled", () => {
  it.each([
    ["call.progress", callProgress],
    ["call.settled", callSettled],
  ])("renders nothing from the call's label: %s", async (name, handler) => {
    // FAILS IF: the override is removed. eve's defaults put the call's label —
    // built from the call's INPUT — into the channel-visible status.
    const { ctx, request, post, startTyping, state } = fakeCtx();

    await (handler as AnyArgs)(
      fact(name, { callId: "c1", label: "Read her performance review", outcome: "completed" }),
      ctx,
      neverNext,
    );

    expect(startTyping).not.toHaveBeenCalled();
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    // `call.settled`'s default records task names for `model.started` to show;
    // leaving them unrecorded is what keeps that kept default content-free.
    expect(state).not.toHaveProperty("pendingTaskResults");
  });
});

describe("turnSettled", () => {
  it("delivers a failure ephemerally and quotes no error detail", async () => {
    // FAILS IF: the override is removed (eve's default posts the error's message
    // and hint into the channel), or if the copy is widened to include
    // `error.message` / `error.hint`, which can quote model or tool output.
    const { ctx, request, post } = fakeCtx();

    await turnSettled(
      fact("turn.settled", {
        turnId: "t1",
        outcome: "failed",
        error: { code: "boom", message: "TOOL RETURNED SECRET ROW", hint: "SECRET HINT" },
      }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    const body = request.mock.calls[0]?.[1] as { text: string; user: string };
    expect(body.user).toBe("U_AUTH");
    expect(body.text).not.toContain("SECRET");
    expect(post).not.toHaveBeenCalled();
  });

  it.each([["completed"], ["cancelled"]])(
    "clears the status and sends nothing for a %s turn",
    async (outcome) => {
      // FAILS IF: the status clear eve's default does on every settled turn is
      // lost (the status would linger on the thread), or a non-failure sends a
      // message.
      const { ctx, request, post, startTyping } = fakeCtx();

      await turnSettled(
        fact("turn.settled", { turnId: "t1", outcome }) as AnyArgs,
        ctx as AnyArgs,
        neverNext,
      );

      expect(startTyping.mock.calls).toEqual([[]]);
      expect(request).not.toHaveBeenCalled();
      expect(post).not.toHaveBeenCalled();
    },
  );
});

describe("sessionEnded", () => {
  it("delivers a failure ephemerally and quotes no error detail", async () => {
    // FAILS IF: the override is removed — eve's default posts the error into the
    // channel.
    const { ctx, request, post } = fakeCtx();

    await sessionEnded(
      fact("session.ended", {
        outcome: "failed",
        error: { code: "boom", message: "SECRET ROW" },
      }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    expect((request.mock.calls[0]?.[1] as { text: string }).text).not.toContain("SECRET");
    expect(post).not.toHaveBeenCalled();
  });

  it("sends nothing when the session ended without failing", async () => {
    const { ctx, request, post } = fakeCtx();

    await sessionEnded(
      fact("session.ended", { outcome: "completed" }) as AnyArgs,
      ctx as AnyArgs,
      neverNext,
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("interactionOpened", () => {
  it.each([["approval"], ["question"]])(
    "delivers a %s prompt ephemerally and never posts a card",
    async (kind) => {
      // FAILS IF: the override is removed — eve's default posts interactive cards
      // into the thread, and the prompt can quote the turn's context.
      const { ctx, request, post } = fakeCtx();

      await interactionOpened(
        fact("interaction.opened", {
          interactionId: "i1",
          request: { kind, prompt: "Approve reading A?" },
        }) as AnyArgs,
        ctx as AnyArgs,
        neverNext,
      );

      expect(operations(request)).toEqual(["chat.postEphemeral"]);
      expect((request.mock.calls[0]?.[1] as { text: string }).text).toBe("Approve reading A?");
      expect(post).not.toHaveBeenCalled();
    },
  );

  it("hands a sign-in to eve's default, which owns its private challenge", async () => {
    // FAILS IF: a sign-in stops reaching `next`. eve gives a sign-in handler only
    // the private delivery surface, and its default is what delivers the
    // challenge (a credential) ephemerally — swallowing it here would strand the
    // sign-in.
    const { request, post } = fakeCtx();
    const postEphemeral = vi.fn(async () => undefined);
    const next = vi.fn(async () => undefined);

    await interactionOpened(
      fact("interaction.opened", {
        interactionId: "i1",
        request: { kind: "sign-in", prompt: "Connect temper" },
      }) as AnyArgs,
      {
        channel: { postEphemeral, postDirectMessage: vi.fn(), state: {} },
        session: { auth: { current: null } },
      } as AnyArgs,
      next as AnyArgs,
    );

    expect(next).toHaveBeenCalledTimes(1);
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
    await ephemeralRenderer.received?.({} as AnyArgs, ctx.channel as AnyArgs, neverNext);
    expect(startTyping).not.toHaveBeenCalled();

    expect(ephemeralRenderer.taskCard?.({} as AnyArgs, neverNext)).toBeNull();
  });
});

describe("ephemeralRenderer composed by eve's real renderer chain", () => {
  /**
   * eve's REAL chain composition, imported at runtime the same way the default
   * tables are above. Composing `ephemeralRenderer` over SPY defaults pins the
   * claim this whole file rests on — that a handler which never calls `next`
   * replaces eve's default — against eve's own `composeSlackRenderers`, not
   * against our reading of it.
   */
  it("never reaches eve's default for any overridden event, nor for received or taskCard", async () => {
    // FAILS IF: any override (other than a sign-in) calls `next`, or an eve
    // upgrade changes the chain so a renderer's handler no longer stands in
    // front of the default.
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

    const events: Record<string, Record<string, unknown>> = {
      "content.completed": { kind: "text", phase: "reply", value: "the answer" },
      "turn.settled": { outcome: "failed" },
      "session.ended": { outcome: "failed" },
      "interaction.opened": { interactionId: "i1", request: { kind: "approval", prompt: "?" } },
    };
    for (const name of names) {
      const { ctx } = fakeCtx();
      const handler = (chain.events as Record<string, (...args: unknown[]) => Promise<void>>)[name];
      expect(handler, name).toBeDefined();
      await handler?.(fact(name, events[name] ?? {}), ctx);
    }
    for (const name of names) expect(defaults[name], name).not.toHaveBeenCalled();

    await chain.received({} as AnyArgs, fakeCtx().ctx.channel as AnyArgs);
    expect(received).not.toHaveBeenCalled();
    expect(chain.taskCard({} as AnyArgs)).toBeNull();
    expect(taskCard).not.toHaveBeenCalled();
  });
});

/**
 * Every key of eve's default tables, classified by whether its DEFAULT handler
 * can route model-derived text to a channel-visible sink. Verified against
 * `.../slack/defaults.js` and `.../slack/request-cards.js` at eve@0.91.0.
 *
 * `"content-bearing"` means the default reads something the model produced
 * (reasoning, a reply, narration, a call label, an error's detail, an
 * interaction prompt) and sends it somewhere the channel can observe — via
 * `thread.post` OR the thread status. Those MUST be overridden.
 *
 * `"content-free"` is a claim about eve's source, re-verified per entry below.
 * It is a claim we are allowed to be wrong about only in the safe direction:
 * an unclassified key fails the test outright.
 */
const DEFAULT_EVENT_CLASSIFICATION: Readonly<Record<string, "content-bearing" | "content-free">> = {
  // `showStatus("Thinking...")` plus three state resets. Reads nothing off the event.
  "turn.started": "content-free",
  // `hideStatus`.
  "turn.paused": "content-free",
  // `showStatus(reviewingResults(names) ?? state.threadStatus.text ?? "Thinking...")`:
  // task names `call.settled` recorded (overridden: records none), or re-showing a
  // status eve's own `showStatus` wrote — which no override here calls.
  "model.started": "content-free",
  // `waitingOnTasks(names)` for a turn waiting on TASKS: task tool/agent names from
  // eve's own tables, never model prose. This agent starts no tasks.
  "model.settled": "content-free",
  // Reasoning deltas → `reasoningStatus(...)` → `showStatus(...)`: the latest sentence
  // of the raw reasoning trace, which quotes tool results verbatim.
  "content.delta": "content-bearing",
  // A `reply` text part → `deliverCompletedSlackReply` (the answer itself, publicly);
  // a reasoning part → the status, as above; narration → `pendingToolCallMessage`.
  "content.completed": "content-bearing",
  // `pendingToolCallMessage` (model narration) or `callLabel(...)` → `showStatus`.
  "call.requested": "content-bearing",
  // `showStatus(data.label)`; a label is built from the call's INPUT.
  "call.progress": "content-bearing",
  // `showStatus(data.label)`, as `call.progress`.
  "call.settled": "content-bearing",
  // A failed turn → `thread.post` of the error's message and hint.
  "turn.settled": "content-bearing",
  // A failed session → `thread.post` of the error's message and hint.
  "session.ended": "content-bearing",
  // An approval or question → `postRequestCards` → `thread.post` of cards built from
  // the prompt and the call's input. (A sign-in reaches it too, through `next`, with
  // only the private surface — see the dedicated test.)
  "interaction.opened": "content-bearing",
  // `showAnswerChange` — rewrites a card recorded in `state.interactionCards`, which
  // only eve's `interaction.opened` default writes. Overridden there, so none exists.
  "response.admitted": "content-free",
  // `showAnswerChange` as above, and `notifyRefusedResponder` → `postEphemeral` to the
  // refused responder (private).
  "response.settled": "content-free",
  // A sign-in → `Connected to <displayName>. Resuming...` and `chat.update` of its
  // link-free public line with an outcome code; otherwise `settleRequestCard` on a
  // recorded card, of which there are none.
  "interaction.settled": "content-free",
};

/** Every key eve's default renderer handles, across both of its tables. */
function defaultKeys(): string[] {
  const interactionDefaults = defaultInteractionEvents(undefined, (card: AnyArgs, last: AnyArgs) =>
    last(card),
  );
  return [...new Set([...Object.keys(defaultEvents), ...Object.keys(interactionDefaults)])].sort();
}

describe("ephemeralEvents", () => {
  it("classifies every event eve actually defaults", () => {
    // FAILS IF: an eve upgrade ADDS a default key. That is the whole reason this
    // is derived from the real objects instead of a hardcoded list: a new eve
    // default is a new public sink installed into this channel with no edit on
    // our side. This test forces a human to read the new handler and classify it
    // before CI goes green again. It also fails if eve REMOVES one, which is a
    // cheaper but equally real drift.
    expect(defaultKeys()).toEqual(Object.keys(DEFAULT_EVENT_CLASSIFICATION).sort());
  });

  it("overrides every content-bearing eve default", () => {
    // FAILS IF: any handler that eve would otherwise render from model output is
    // left un-overridden. An ABSENT key runs the public default, not a no-op.
    // Derived from the classification above rather than spelled out, so a key
    // reclassified as content-bearing is immediately required here.
    const contentBearing = Object.entries(DEFAULT_EVENT_CLASSIFICATION)
      .filter(([, kind]) => kind === "content-bearing")
      .map(([name]) => name)
      .sort();

    const overridden = Object.keys(ephemeralEvents).sort();

    expect(contentBearing.filter((name) => !overridden.includes(name))).toEqual([]);
  });
});

describe("the session-state recipient fallback is confined to session.ended", () => {
  /**
   * A Slack session is keyed by `(channel, thread)`, so two humans mentioning in
   * one thread share `state.triggeringUserId` — it holds whoever triggered MOST
   * RECENTLY, not this turn's asker. A content-bearing handler that fell back to
   * it would hand A's answer to B.
   *
   * FAILS IF: `contentCompleted`, `turnSettled` or `interactionOpened` is allowed
   * to reach `state.triggeringUserId`. Each is invoked with NO current auth, so
   * auth yields no recipient; state holds a valid-looking user id. A handler that
   * consults the fallback would deliver to `U_STATE`. The correct behaviour is
   * to DROP.
   */
  it.each([
    [
      "content.completed",
      contentCompleted,
      { kind: "text", phase: "reply", value: "the answer" },
    ],
    ["turn.settled", turnSettled, { outcome: "failed" }],
    [
      "interaction.opened",
      interactionOpened,
      { interactionId: "i1", request: { kind: "question", prompt: "Which doc?" } },
    ],
  ])("drops rather than using stale state: %s", async (name, handler, data) => {
    const { ctx, request, post } = fakeCtx({ authUserId: null, triggeringUserId: "U_STATE" });
    vi.spyOn(console, "warn").mockImplementation(() => {});

    await (handler as AnyArgs)(fact(name, data), ctx, neverNext);

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });

  /**
   * The converse. A session can end with no current auth, so state is
   * `session.ended`'s last recipient source and it must still deliver.
   *
   * FAILS IF: the fallback is removed wholesale rather than confined — which
   * would silently stop delivering unrecoverable-session notices to anyone.
   */
  it("still delivers session.ended from state alone", async () => {
    const { ctx, request } = fakeCtx({ authUserId: null, triggeringUserId: "U_STATE" });

    await sessionEnded(fact("session.ended", { outcome: "failed" }) as AnyArgs, ctx as AnyArgs, neverNext);

    expect(request).toHaveBeenCalledWith(
      "chat.postEphemeral",
      expect.objectContaining({ user: "U_STATE" }),
    );
  });
});
