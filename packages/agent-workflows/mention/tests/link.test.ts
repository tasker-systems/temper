import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  assertSlackSecretsDistinct,
  requestLinkState,
  LINK_STATE_DOOR,
  MINT_DOOR,
  signedHeaders,
  signRequestV2,
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

describe("signedHeaders", () => {
  // FAILS IF: a signer sends anything but the v2 credential. temper-api reads only these three, and
  // the retired `X-Temper-Signature` construction is good at any door that shares its key.
  it("sends exactly the v2 headers, with the timestamp floored to whole seconds", () => {
    const body = JSON.stringify({ slack_principal_id: "slack:T1:U1" });
    const headers = signedHeaders("s3cret", MINT_DOOR, 1_700_000_000.9, body);

    expect(Object.keys(headers).sort()).toEqual([
      "X-Temper-Nonce",
      "X-Temper-Signature-V2",
      "X-Temper-Timestamp",
    ]);
    expect(headers).not.toHaveProperty("X-Temper-Signature");
    expect(headers["X-Temper-Timestamp"]).toBe("1700000000");
    expect(headers["X-Temper-Signature-V2"]).toBe(
      signRequestV2(
        "s3cret",
        "POST",
        MINT_DOOR,
        "1700000000",
        headers["X-Temper-Nonce"],
        body,
      ),
    );
  });
});

// FAILS IF: a door drifts from the route temper-api registers (`temper_core::internal_sig::DOORS`).
// The v2 signature names the door, so a drifted string is a 401 on every mention once temper-api
// requires v2 — and the header assertions below compare against these same constants, so only a
// literal can catch it.
describe("doors", () => {
  it("name the routes temper-api registers", () => {
    expect(LINK_STATE_DOOR).toBe("/internal/slack/link-state");
    expect(MINT_DOOR).toBe("/internal/slack/mint");
  });
});

describe("signRequestV2", () => {
  // The v2 known-answer vector, asserted identically in crates/temper-core/tests/internal_sig_v2.rs
  // (`sign_v2_matches_known_answer_vector`) and packages/temper-cloud/tests/oauth/wire-contract.test.ts.
  it("matches the shared v2 known-answer signature", () => {
    const body =
      '{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["engineering"]}';
    expect(
      signRequestV2(
        "topsecret-abcdefghijklmnopqrstuvwxyz012345",
        "POST",
        "/internal/saml/reconcile",
        "1750000000",
        "0123456789abcdef0123456789abcdef",
        body,
      ),
    ).toBe("47385733b968d6043c77d0f515723b670636b69b40e3678e8fe82dfef890c65a");
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
    expect(headers).not.toHaveProperty("X-Temper-Signature");
    expect(headers["X-Temper-Timestamp"]).toMatch(/^\d+$/);

    // FAILS IF: the v2 signature names a door other than link-state, or no fresh nonce. temper
    // holds a v2 signature to the door it names, so a wrong door here is a 401 on every mention.
    const nonce = headers["X-Temper-Nonce"];
    expect(nonce).toMatch(/^[0-9a-f]{32}$/);
    expect(headers["X-Temper-Signature-V2"]).toBe(
      signRequestV2(
        "s3cret",
        "POST",
        LINK_STATE_DOOR,
        headers["X-Temper-Timestamp"],
        nonce,
        init.body as string,
      ),
    );
  });

  it("throws on a non-2xx so the channel can say something honest", async () => {
    // A 401 here means the shared secret drifted. Swallowing it would leave the user with
    // silence; the channel's catch turns this throw into a visible message.
    stubFetch(401, {});

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow("link-state failed: 401");
  });

  // FAILS IF: a 200 whose body matches neither link-state arm is returned to the caller as
  // a `LinkState` anyway. The channel narrows with `status === "unlinked"`, so a drifted
  // status fell through as `linked` — the one arm that mints and dispatches. Fails CLOSED:
  // the channel's catch turns this throw into the generic retry ephemeral, which is the
  // honest answer when the response is not an answer.
  it("throws on a drifted status instead of returning it as a LinkState", async () => {
    stubFetch(200, { status: "quarantined" });

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow(
      "link-state returned an unrecognized response (status: quarantined)",
    );
  });

  // FAILS IF: the shape check tests the discriminant only. A `linked` body without its
  // `handle` (or an `unlinked` one without `authorize_url`) deserializes fine in
  // TypeScript's eyes and would reach the user copy as `undefined` interpolation.
  it("throws when a known arm is missing its field", async () => {
    stubFetch(200, { status: "linked" });

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow(
      "link-state returned an unrecognized response (status: linked)",
    );
  });

  it("throws when a required env var is missing", async () => {
    vi.stubEnv("SLACK_LINK_SECRET", "");
    stubFetch(200, { status: "linked", handle: "h" });

    await expect(requestLinkState(PRINCIPAL)).rejects.toThrow("SLACK_LINK_SECRET");
  });
});
