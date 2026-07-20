import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ephemeralEvents,
  firstNonEmptyLine,
  inputRequested,
  messageCompleted,
  sessionFailed,
  turnFailed,
} from "../agent/channels/events.js";

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
    );

    expect(operations(request)).toEqual(["chat.postEphemeral"]);
    expect(request.mock.calls[0]?.[1]).toEqual({
      channel: "C123",
      user: "U_AUTH",
      text: "the answer",
    });
    expect(post).not.toHaveBeenCalled();
  });

  it("posts NOTHING on a tool-calls boundary, and buffers the narration", async () => {
    // FAILS IF: the `finishReason === "tool-calls"` early return is dropped —
    // the single easiest thing to lose when reimplementing eve's default. The
    // symptom is mid-turn tool chatter delivered on every turn. Also covers
    // the state write the default `actions.requested` handler reads back.
    const { ctx, request, post, state } = fakeCtx();

    await messageCompleted(
      { finishReason: "tool-calls", message: "\n\n  Looking that up  \nmore" } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    expect(state.pendingToolCallMessage).toBe("Looking that up");
  });

  it("clears the buffer and only signals typing when the message is empty", async () => {
    // FAILS IF: an empty message is delivered as an empty ephemeral, or the
    // stale `pendingToolCallMessage` is left set (which would make a later
    // typing indicator show text from a previous turn).
    const { ctx, request, startTyping, state } = fakeCtx();
    state.pendingToolCallMessage = "stale";

    await messageCompleted(
      { finishReason: "stop", message: null } as AnyArgs,
      ctx as AnyArgs,
      fakeCallbackCtx("U_AUTH") as AnyArgs,
    );

    expect(request).not.toHaveBeenCalled();
    expect(startTyping).toHaveBeenCalledTimes(1);
    expect(state.pendingToolCallMessage).toBeNull();
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
    );

    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
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

    await sessionFailed({ code: "boom", message: "x", sessionId: "s1" } as AnyArgs, ctx as AnyArgs);

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
    );

    expect(operations(request)).toEqual(["chat.postEphemeral", "chat.postEphemeral"]);
    expect(request.mock.calls.map((call) => (call[1] as { text: string }).text)).toEqual([
      "Approve A?",
      "Approve B?",
    ]);
    expect(post).not.toHaveBeenCalled();
  });
});

describe("ephemeralEvents", () => {
  it("overrides exactly the four public sinks", async () => {
    // FAILS IF: a handler is added to the module but never wired into the
    // exported map (so it is never installed), or one is dropped from the map
    // — leaving eve's PUBLIC default in place for that event, silently. The
    // merge is `{...defaultEvents, ...userEvents}`, so an absent key is a
    // public default, not a no-op.
    expect(Object.keys(ephemeralEvents).sort()).toEqual([
      "input.requested",
      "message.completed",
      "session.failed",
      "turn.failed",
    ]);
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

describe("firstNonEmptyLine", () => {
  it("returns the first non-blank line, trimmed", () => {
    // FAILS IF: the helper drifts from eve's non-exported original, which the
    // default `actions.requested` handler's typing indicator depends on.
    expect(firstNonEmptyLine("\n \n  hello  \nworld")).toBe("hello");
    expect(firstNonEmptyLine("solo")).toBe("solo");
  });

  it("returns null for blank input", () => {
    // FAILS IF: an empty string is returned instead of null — the state field
    // is typed `string | null` and a blank indicator is not a valid one.
    expect(firstNonEmptyLine("")).toBeNull();
    expect(firstNonEmptyLine("\n   \n\t")).toBeNull();
  });
});
