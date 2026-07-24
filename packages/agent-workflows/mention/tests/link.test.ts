import { createHmac } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  assertSlackSecretsDistinct,
  requestLinkState,
  signIntentRequest,
} from "../agent/lib/link.js";

describe("assertSlackSecretsDistinct", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  // FAILS IF: this deployment may hold one value for both Slack secrets. link-state answers a
  // question; mint hands back a human's entire reach. temper-api refuses to BOOT on the same
  // collision (`check_secret_distinctness`), but the agent is a separate Vercel deployment with
  // its own environment, so a server-side check cannot see this one.
  it("refuses when the link and mint secrets hold the same value", () => {
    vi.stubEnv("SLACK_LINK_SECRET", "shared-by-copy-paste");
    vi.stubEnv("SLACK_MINT_SECRET", "shared-by-copy-paste");

    expect(() => assertSlackSecretsDistinct()).toThrow(/same value/);
  });

  // FAILS IF: the error text leaks the credential it is complaining about. It surfaces in eve's
  // logs on a dropped mention, which is not a place a shared secret may appear.
  it("never prints the colliding value", () => {
    const leaked = "s3cret-shared-by-copy-paste";
    vi.stubEnv("SLACK_LINK_SECRET", leaked);
    vi.stubEnv("SLACK_MINT_SECRET", leaked);

    expect(() => assertSlackSecretsDistinct()).toThrow(
      expect.objectContaining({ message: expect.not.stringContaining(leaked) }),
    );
  });

  // FAILS IF: a guard that rejects everything. The correct configuration must pass.
  it("accepts distinct secrets", () => {
    vi.stubEnv("SLACK_LINK_SECRET", "link-s3cret");
    vi.stubEnv("SLACK_MINT_SECRET", "mint-s3cret");

    expect(() => assertSlackSecretsDistinct()).not.toThrow();
  });

  // FAILS IF: absence is read as collision. An unset variable is `requireEnv`'s business — and
  // treating two absent secrets as equal would break every deployment that runs one flow and not
  // the other.
  it("ignores absent secrets", () => {
    vi.stubEnv("SLACK_LINK_SECRET", "");
    vi.stubEnv("SLACK_MINT_SECRET", "");
    expect(() => assertSlackSecretsDistinct()).not.toThrow();

    vi.stubEnv("SLACK_MINT_SECRET", "only-one-set");
    expect(() => assertSlackSecretsDistinct()).not.toThrow();
  });
});

describe("signIntentRequest", () => {
  it("signs HMAC-SHA256 over `{timestamp}.{body}` as lowercase hex", () => {
    const body = JSON.stringify({ slack_principal_id: "slack:T1:U1" });
    const { timestamp, signature } = signIntentRequest("s3cret", 1_700_000_000, body);

    expect(timestamp).toBe("1700000000");
    // The known-answer check: this MUST match temper_core::internal_sig::sign.
    const expected = createHmac("sha256", "s3cret")
      .update(`1700000000.${body}`)
      .digest("hex");
    expect(signature).toBe(expected);
    expect(signature).toMatch(/^[0-9a-f]{64}$/);
  });
});

describe("requestLinkState", () => {
  /** A 4-segment principal — the shape most likely to be silently mangled by a parse. */
  const PRINCIPAL = "slack:T012AB3CD:bot:U024BE7LH";

  /** Stub the endpoint with a fixed JSON body and capture what the agent sent. */
  function stubFetch(status: number, body: unknown) {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status }));
    vi.stubGlobal("fetch", fetchMock);
    return fetchMock;
  }

  beforeEach(() => {
    vi.stubEnv("TEMPER_API_URL", "https://temper.test/");
    vi.stubEnv("SLACK_LINK_SECRET", "s3cret");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  // FAILS IF: the distinctness assert is not WIRED into this call. A correct predicate nothing
  // invokes is exactly the gap this whole change exists to close, so it is asserted at the caller
  // and not only on the pure function. Fails CLOSED: no request is made at all.
  it("makes no signed call when the two secrets collide", async () => {
    vi.stubEnv("SLACK_MINT_SECRET", "s3cret"); // equal to the SLACK_LINK_SECRET stubbed above
    const fetchMock = stubFetch(200, { status: "linked", handle: "j-cole-taylor" });

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow(/same value/);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("returns the linked arm with the handle", async () => {
    stubFetch(200, { status: "linked", handle: "j-cole-taylor" });

    const state = await requestLinkState(PRINCIPAL);

    expect(state).toEqual({ status: "linked", handle: "j-cole-taylor" });
    // Narrowing must work off `status` alone — this is the contract with the Rust enum.
    if (state.status !== "linked") throw new Error("expected the linked arm");
    expect(state.handle).toBe("j-cole-taylor");
  });

  it("returns the unlinked arm with the authorize URL", async () => {
    stubFetch(200, {
      status: "unlinked",
      authorize_url: "https://idp.test/authorize?state=abc",
    });

    const state = await requestLinkState(PRINCIPAL);

    if (state.status !== "unlinked") throw new Error("expected the unlinked arm");
    expect(state.authorize_url).toBe("https://idp.test/authorize?state=abc");
  });

  it("posts the WHOLE principal to link-state, signed, with no trailing-slash dupe", async () => {
    const fetchMock = stubFetch(200, { status: "linked", handle: "h" });

    await requestLinkState(PRINCIPAL);

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    // The base URL's trailing slash is trimmed, not doubled up.
    expect(url).toBe("https://temper.test/internal/slack/link-state");
    // The principal rides WHOLE — no segment dropped, none reordered.
    expect(init.body).toBe(JSON.stringify({ slack_principal_id: PRINCIPAL }));

    const headers = init.headers as Record<string, string>;
    expect(headers["X-Temper-Signature"]).toMatch(/^[0-9a-f]{64}$/);
    expect(headers["X-Temper-Timestamp"]).toMatch(/^\d+$/);
  });

  it("throws on a non-2xx so the channel can say something honest", async () => {
    // A 401 here means the shared secret drifted. Swallowing it would leave the user with
    // silence; the channel's catch turns this throw into a visible message.
    stubFetch(401, {});

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow("link-state failed: 401");
  });

  it("throws when a required env var is missing", async () => {
    vi.stubEnv("SLACK_LINK_SECRET", "");
    stubFetch(200, { status: "linked", handle: "h" });

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow("SLACK_LINK_SECRET");
  });
});
