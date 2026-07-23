import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LinkState } from "../agent/lib/link.js";
import type { MintOutcome } from "../agent/lib/mint.js";

/**
 * The dispatch fork in `onAppMention`: who gets a model turn, and who gets which sentence.
 *
 * `slackChannel` is stubbed to the identity function so the channel module hands back its own
 * config object and `onAppMention` is directly callable. `defaultSlackAuth` is stubbed too —
 * building a real one would mean constructing eve's whole inbound webhook surface, and
 * `identity.test.ts` already covers the auth-context forks.
 */
const { slackChannel, defaultSlackAuth, requestLinkState, requestMintedToken } = vi.hoisted(
  () => ({
    slackChannel: vi.fn((config: unknown) => config),
    defaultSlackAuth: vi.fn(),
    requestLinkState: vi.fn<(p: string) => Promise<LinkState>>(),
    requestMintedToken: vi.fn<(p: string) => Promise<MintOutcome>>(),
  }),
);

vi.mock("eve/channels/slack", () => ({ slackChannel, defaultSlackAuth }));
vi.mock("../agent/lib/link.js", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../agent/lib/link.js")>()),
  requestLinkState,
}));
vi.mock("../agent/lib/mint.js", () => ({ requestMintedToken }));

/** The principal the whole pipeline is keyed on — 3 segments, never split. */
const PRINCIPAL = "slack:T012AB3CD:U024BE7LH";
const USER_ID = "U024BE7LH";

/** A human `SessionAuthContext`, reduced to what the channel reads. */
function humanAuth() {
  return {
    principalId: PRINCIPAL,
    principalType: "user",
    authenticator: "slack-webhook",
    attributes: { user_id: USER_ID, team_id: "T012AB3CD" },
  };
}

/** A pre-dispatch `SlackContext`: `slack` + `thread`, and no `state`. */
function fakeCtx() {
  const request = vi.fn(async (_operation: string, _body: unknown) => ({ ok: true }));
  const post = vi.fn(async (_message: { text: string }) => undefined);
  return { ctx: { slack: { channelId: "C123", request }, thread: { post } }, request, post };
}

/** The text of every ephemeral delivered during a call. */
function ephemeralTexts(request: ReturnType<typeof fakeCtx>["request"]): string[] {
  return request.mock.calls
    .filter(([operation]) => operation === "chat.postEphemeral")
    .map(([, body]) => (body as { text: string }).text);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyArgs = any;

/**
 * Drive one mention. Imported lazily so the module-level `vi.mock`s are installed first, and
 * so each test gets the config object built by the stubbed `slackChannel`.
 */
async function mention(ctx: unknown) {
  const channel = (await import("../agent/channels/slack.js")).default as AnyArgs;
  return channel.onAppMention(ctx as AnyArgs, {} as AnyArgs);
}

beforeEach(() => {
  defaultSlackAuth.mockReturnValue(humanAuth());
  requestLinkState.mockReset();
  requestMintedToken.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("onAppMention — the mint pre-flight", () => {
  // FAILS IF: a linked user with a live credential is still dropped (the pre-Beat-D
  // `return null`), or dispatched without `auth` — which would run the turn under no
  // identity and give the model no reach at all.
  it("DISPATCHES with the caller's auth when a token is minted", async () => {
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({
      status: "token",
      access_token: "at-1",
      expires_at_ms: 1_784_505_600_000,
    });
    const { ctx, request } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toEqual({ auth: humanAuth() });
    // No ephemeral on the happy path: the model's answer is the reply, delivered by the
    // `message.completed` override.
    expect(ephemeralTexts(request)).toEqual([]);
  });

  // FAILS IF: a not_vaulted user is DISPATCHED. That is the headline bug this fork exists to
  // prevent — the turn would die inside `getToken` and the user would get `turn.failed`'s
  // deliberately detail-free line instead of the one message that tells them what to do.
  it("never dispatches a not_vaulted user, and says something specific instead", async () => {
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({ status: "refused", reason: "not_vaulted" });
    const { ctx, request, post } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toBeNull();
    const [text] = ephemeralTexts(request);
    expect(text).toBeDefined();
    // The load-bearing content: retrying is futile, and here is the remedy.
    expect(text).toMatch(/won't fix/i);
    expect(text).toContain("temper slack disconnect");
    // Private, always. A credential-adjacent status line must not reach the channel.
    expect(post).not.toHaveBeenCalled();
  });

  // FAILS IF: a human refused on STANDING is DISPATCHED, or is told to re-link. Same
  // dispatch failure mode as above, plus the false-remedy bug: re-linking cannot move
  // principal standing, so `temper slack disconnect` here is advice that leads nowhere.
  it("never dispatches a standing-refused user, and names the ADMIN remedy", async () => {
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({
      status: "refused",
      reason: "standing",
      refusal: { kind: "denied" },
    });
    const { ctx, request, post } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toBeNull();
    const [text] = ephemeralTexts(request);
    expect(text).toMatch(/admin/i);
    expect(text).not.toContain("temper slack disconnect");
    expect(post).not.toHaveBeenCalled();
  });

  // FAILS IF: the refusal replies are collapsed into one generic message. Each must differ,
  // so a shared "something went wrong" string — the tempting simplification — trips this even
  // though every individual reply still "says something".
  it("gives unlinked and each mint refusal a DISTINCT reply", async () => {
    const replies: string[] = [];

    requestLinkState.mockResolvedValue({
      status: "unlinked",
      authorize_url: "https://temper.test/authorize/abc123",
    });
    let harness = fakeCtx();
    await mention(harness.ctx);
    replies.push(...ephemeralTexts(harness.request));

    // One per REMEDY, which is the axis the copy splits on: re-link, admin-decides,
    // wait, our-bug, and the link-vanished race.
    const refusals = [
      { status: "refused", reason: "not_vaulted" },
      { status: "refused", reason: "standing", refusal: { kind: "denied" } },
      { status: "refused", reason: "standing", refusal: { kind: "requested" } },
      { status: "refused", reason: "standing", refusal: { kind: "unrecognized_standing", raw: "x" } },
      { status: "refused", reason: "not_linked" },
    ] as const;

    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    for (const outcome of refusals) {
      requestMintedToken.mockResolvedValue(outcome);
      harness = fakeCtx();
      await mention(harness.ctx);
      replies.push(...ephemeralTexts(harness.request));
    }
    errorSpy.mockRestore();

    expect(replies).toHaveLength(refusals.length + 1);
    expect(new Set(replies).size).toBe(refusals.length + 1);
    // And the unlinked one is the only one carrying a URL — every refusal is reached from
    // the `linked` arm, which has none to offer.
    expect(replies.filter((r) => r.includes("http"))).toHaveLength(1);
  });

  // FAILS IF: the mint is called with anything but the whole principal, or the pre-flight is
  // moved after the dispatch (in which case it would not run at all on this path).
  it("mints for the WHOLE principal before deciding", async () => {
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({ status: "refused", reason: "not_vaulted" });

    await mention(fakeCtx().ctx);

    expect(requestMintedToken).toHaveBeenCalledWith(PRINCIPAL);
  });

  // FAILS IF: the unlinked arm starts minting. `link-state` already said there is no link, so
  // a mint could only ever answer not_vaulted — a wasted signed call to the expensive route.
  it("does not mint for an unlinked user", async () => {
    requestLinkState.mockResolvedValue({
      status: "unlinked",
      authorize_url: "https://temper.test/authorize/abc123",
    });

    const result = await mention(fakeCtx().ctx);

    expect(result).toBeNull();
    expect(requestMintedToken).not.toHaveBeenCalled();
  });

  // FAILS IF: a mint transport failure escapes the try/catch. eve SWALLOWS a thrown handler,
  // so the symptom would be total silence — and a 401 here (mint secret drift) is exactly the
  // outage where silence is most expensive.
  it("says something when the mint call itself throws, and drops", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockRejectedValue(new Error("mint failed: 401"));
    const { ctx, request } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toBeNull();
    const [text] = ephemeralTexts(request);
    expect(text).toMatch(/try again/i);
    // Never quote the transport error: it is derived from an endpoint that hands out
    // credentials, and the user can do nothing with a status code.
    expect(text).not.toContain("401");
  });

  // FAILS IF: the bot gate is lost when the fork was rewritten. A dispatched bot is how
  // mention loops start, and it would now dispatch a bot with a MINTED token.
  it("still drops a non-human principal before any lookup", async () => {
    defaultSlackAuth.mockReturnValue({
      principalId: "slack:T012AB3CD:bot:U024BE7LH",
      principalType: "service",
      authenticator: "slack-webhook",
      attributes: { user_id: "U024BE7LH" },
    });

    const result = await mention(fakeCtx().ctx);

    expect(result).toBeNull();
    expect(requestLinkState).not.toHaveBeenCalled();
    expect(requestMintedToken).not.toHaveBeenCalled();
  });

  // FAILS IF: the no-user_id drop is lost. Without a user id there is nowhere to deliver an
  // ephemeral, and dispatching anyway would send the model's answer through handlers that
  // cannot address it privately either.
  it("still drops when there is no user_id to deliver to", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    defaultSlackAuth.mockReturnValue({ ...humanAuth(), attributes: {} });

    const result = await mention(fakeCtx().ctx);

    expect(result).toBeNull();
    expect(requestMintedToken).not.toHaveBeenCalled();
  });
});

describe("onDirectMessage — the gate that must exist", () => {
  /** The channel config object, so absence-of-key is distinguishable from a null return. */
  async function channelConfig() {
    return (await import("../agent/channels/slack.js")).default as AnyArgs;
  }

  // FAILS IF: `onDirectMessage` is removed from the channel config. eve resolves
  // `onDirectMessage ?? defaultOnDirectMessage`, so an ABSENT key is not "DMs are ignored" —
  // it is eve's `defaultOnDirectMessage`, which does
  // `startTyping("Thinking...")` then `{auth: defaultSlackAuth(...)}`: an UNCONDITIONAL
  // dispatch with no `decideIdentity`, no `principalType === "user"` gate, no link-state and
  // no mint pre-flight. Asserting the key is defined is the only way to tell the two apart,
  // because both look like "nothing happens" from outside until `message.im` is subscribed.
  it("is supplied explicitly, so eve's unconditional default cannot apply", async () => {
    const channel = await channelConfig();

    expect(channel.onDirectMessage).toBeDefined();
    expect(typeof channel.onDirectMessage).toBe("function");
  });

  // FAILS IF: the handler ever starts dispatching. DMs are out of T4 scope; serving one means
  // wiring the identity pipeline first. A non-null return here would run a model turn under
  // an identity nothing has checked.
  it("drops every DM, dispatching nothing", async () => {
    const channel = await channelConfig();
    const { ctx, request, post } = fakeCtx();

    const result = await channel.onDirectMessage(ctx as AnyArgs, {} as AnyArgs);

    expect(result).toBeNull();
    expect(request).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    expect(requestLinkState).not.toHaveBeenCalled();
    expect(requestMintedToken).not.toHaveBeenCalled();
  });
});

describe("onAppMention — an unrecognized mint status", () => {
  // FAILS IF: the `default:` arm is dropped from the switch. Without it the switch falls
  // through, `onAppMention` returns `undefined`, and the mention dies with NO ephemeral and
  // NO log — failing closed, but silently and indistinguishably from a lost mention. The
  // `never` binding makes this a compile error too; this test covers the runtime case where
  // the server ships a new status before the agent redeploys, which types cannot catch.
  it("delivers the generic retry ephemeral and logs, instead of silently returning undefined", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({ status: "quarantined" } as unknown as MintOutcome);
    const { ctx, request } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toBeNull();
    expect(ephemeralTexts(request)).toHaveLength(1);
    expect(ephemeralTexts(request)[0]).toMatch(/try again/i);
    expect(error).toHaveBeenCalled();
  });

  // FAILS IF: the INNER switch loses its `default:`. The outer one being covered proves
  // nothing about the inner one — a refusal with an unknown `reason` gets past `case
  // "refused"` and then falls out of the nested switch, which is the same silent-undefined
  // death one level down. This is the arm a new Rust `LinkRefusal` variant lands in.
  it("also covers an unrecognized refusal REASON, not just an unrecognized status", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    requestLinkState.mockResolvedValue({ status: "linked", handle: "j-cole-taylor" });
    requestMintedToken.mockResolvedValue({
      status: "refused",
      reason: "embargoed",
    } as unknown as MintOutcome);
    const { ctx, request } = fakeCtx();

    const result = await mention(ctx);

    expect(result).toBeNull();
    expect(ephemeralTexts(request)).toHaveLength(1);
    expect(ephemeralTexts(request)[0]).toMatch(/try again/i);
    expect(error).toHaveBeenCalled();
  });
});
