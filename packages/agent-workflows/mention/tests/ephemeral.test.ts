import { afterEach, describe, expect, it, vi } from "vitest";

import {
  type EphemeralDelivery,
  deliverEphemeral,
  ephemeralFailureNotice,
  resolveEphemeralRecipient,
} from "../agent/lib/ephemeral.js";

/**
 * A fake Slack context recording exactly which API operation was called and
 * with what payload. Recording the OPERATION NAME is the whole point: a test
 * that only asserts "something was called" would still pass if the code
 * posted publicly.
 */
function fakeCtx(response: { ok: boolean; error?: string } = { ok: true }) {
  const request = vi.fn(async (_operation: string, _body: unknown) => response);
  const post = vi.fn(async (_message: { text: string }) => undefined);
  const ctx: EphemeralDelivery = {
    slack: { channelId: "C123", request },
    thread: { post },
  };
  return { ctx, request, post };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("deliverEphemeral", () => {
  it("calls chat.postEphemeral — NOT a public post", async () => {
    // FAILS IF: the delivery is switched to `thread.post` / `chat.postMessage`,
    // i.e. the exact regression this whole module exists to prevent. Asserting
    // the operation string is what makes that detectable; asserting merely
    // "request was called" would not be.
    const { ctx, request, post } = fakeCtx();

    const outcome = await deliverEphemeral(ctx, "U1", "secret answer");

    expect(request).toHaveBeenCalledTimes(1);
    expect(request.mock.calls[0]?.[0]).toBe("chat.postEphemeral");
    expect(post).not.toHaveBeenCalled();
    expect(outcome).toEqual({ kind: "delivered" });
  });

  it("addresses the channel ROOT — the payload carries no thread_ts", async () => {
    // FAILS IF: someone "simplifies" this to `ctx.thread.postEphemeral`, or
    // adds `thread_ts` to the payload. Both make the message land in a thread
    // the user isn't viewing, where an ephemeral is invisible — the live
    // debugging session this constraint came from. A test that checked only
    // the operation name would NOT catch it.
    const { ctx, request } = fakeCtx();

    await deliverEphemeral(ctx, "U024BE7LH", "hello");

    const body = request.mock.calls[0]?.[1] as Record<string, unknown>;
    expect(body).toEqual({ channel: "C123", user: "U024BE7LH", text: "hello" });
    expect(body).not.toHaveProperty("thread_ts");
  });

  it("surfaces a delivery failure publicly, with the error code and NOT the reply", async () => {
    // FAILS IF: `ok:false` is treated as success (the raw request does not
    // throw, so an unchecked call fails silently — the "total silence"
    // outcome), or if the public fallback line leaks the ephemeral text.
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { ctx, post } = fakeCtx({ ok: false, error: "channel_not_found" });

    const outcome = await deliverEphemeral(ctx, "U1", "SENSITIVE-REACH-DERIVED-ANSWER");

    expect(outcome).toEqual({ kind: "failed", error: "channel_not_found" });
    expect(post).toHaveBeenCalledTimes(1);
    const text = post.mock.calls[0]?.[0].text ?? "";
    expect(text).toContain("channel_not_found");
    expect(text).not.toContain("SENSITIVE-REACH-DERIVED-ANSWER");
  });

  it("names the failure `unknown_error` when Slack returns ok:false with no code", async () => {
    // FAILS IF: the `?? "unknown_error"` default is dropped, which would put a
    // bare `undefined` in front of the user.
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { ctx, post } = fakeCtx({ ok: false });

    const outcome = await deliverEphemeral(ctx, "U1", "hi");

    expect(outcome).toEqual({ kind: "failed", error: "unknown_error" });
    expect(post.mock.calls[0]?.[0].text).toContain("unknown_error");
  });
});

describe("ephemeralFailureNotice", () => {
  it("carries the Slack error code and nothing else", () => {
    // FAILS IF: the notice is ever widened to include the undelivered reply.
    // It is posted PUBLICLY, so its content is a disclosure surface.
    expect(ephemeralFailureNotice("cant_post")).toContain("cant_post");
    expect(ephemeralFailureNotice("cant_post")).toMatch(/private message/i);
  });
});

describe("resolveEphemeralRecipient", () => {
  const SLACK_AUTH = {
    attributes: { user_id: "U_FROM_AUTH" },
    authenticator: "slack-webhook",
  };

  it("prefers the current request's auth over persisted session state", () => {
    // FAILS IF: the order is inverted. `state.triggeringUserId` is refreshed
    // only when the authenticator is `slack-webhook`, so it can hold a STALE
    // user — sending one person's answer to another. Preference order is the
    // correctness property here, not a style choice.
    expect(resolveEphemeralRecipient(SLACK_AUTH, "U_STALE")).toBe("U_FROM_AUTH");
  });

  it("falls back to session state when no auth context is supplied", () => {
    // FAILS IF: the fallback is removed. `session.failed` is invoked with only
    // two arguments (no SessionContext at all), so state is its ONLY source —
    // dropping the fallback silences that handler entirely.
    expect(resolveEphemeralRecipient(undefined, "U_STATE")).toBe("U_STATE");
    expect(resolveEphemeralRecipient(null, "U_STATE")).toBe("U_STATE");
  });

  it("refuses a non-slack-webhook authenticator and falls back", () => {
    // FAILS IF: the gate is loosened (e.g. to a truthiness check on
    // `attributes.user_id`). eve's own `slackUserIdFromAuthContext` applies
    // this exact predicate; a `user_id` minted by some other authenticator is
    // not a Slack user id and would address the ephemeral at nobody.
    const foreign = { attributes: { user_id: "NOT_A_SLACK_ID" }, authenticator: "oauth" };
    expect(resolveEphemeralRecipient(foreign, "U_STATE")).toBe("U_STATE");
    expect(resolveEphemeralRecipient(foreign, null)).toBeNull();
  });

  it("ignores a non-string or empty user_id", () => {
    // FAILS IF: the `typeof === "string"` narrowing is dropped. `attributes`
    // is typed `string | readonly string[]`, so an array is representable and
    // would be handed to Slack as the `user` field.
    const arrayValued = {
      attributes: { user_id: ["U1", "U2"] as readonly string[] },
      authenticator: "slack-webhook",
    };
    expect(resolveEphemeralRecipient(arrayValued, "U_STATE")).toBe("U_STATE");
    expect(
      resolveEphemeralRecipient({ attributes: { user_id: "" }, authenticator: "slack-webhook" }, null),
    ).toBeNull();
  });

  it("returns null when neither source yields an id", () => {
    // FAILS IF: an empty-string or otherwise falsy id is returned as a
    // recipient. Callers branch on `null` to DROP; a truthy junk id would
    // instead send a doomed request and swallow the outcome.
    expect(resolveEphemeralRecipient(null, null)).toBeNull();
    expect(resolveEphemeralRecipient(undefined, undefined)).toBeNull();
    expect(resolveEphemeralRecipient(null, "")).toBeNull();
  });
});
