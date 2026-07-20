import { createHmac } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { requestMintedToken } from "../agent/lib/mint.js";

describe("requestMintedToken", () => {
  /** A 4-segment principal — the shape most likely to be silently mangled by a parse. */
  const PRINCIPAL = "slack:T012AB3CD:bot:U024BE7LH";

  /** Distinct on purpose: every secret assertion below is vacuous if these are equal. */
  const MINT_SECRET = "mint-s3cret";
  const LINK_SECRET = "link-s3cret";

  /** Stub the endpoint with a fixed JSON body and capture what the agent sent. */
  function stubFetch(status: number, body: unknown) {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status }));
    vi.stubGlobal("fetch", fetchMock);
    return fetchMock;
  }

  function sentInit(fetchMock: ReturnType<typeof stubFetch>) {
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    return { url, init, headers: init.headers as Record<string, string> };
  }

  beforeEach(() => {
    vi.stubEnv("TEMPER_API_URL", "https://temper.test/");
    vi.stubEnv("SLACK_MINT_SECRET", MINT_SECRET);
    // Present, and WRONG for this route. If the module reaches for it, the signature tests fail.
    vi.stubEnv("SLACK_LINK_SECRET", LINK_SECRET);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  // FAILS IF: the module signs with SLACK_LINK_SECRET (or any key other than SLACK_MINT_SECRET).
  // This is the one that catches a reuse of the link key — the privilege split at
  // config.rs:32-48 is the whole reason `/internal/slack/mint` has its own router and layer.
  // temper's own e2e `mint_refuses_the_link_state_key` is the server-side half of this pair.
  it("signs with SLACK_MINT_SECRET, never SLACK_LINK_SECRET", async () => {
    const fetchMock = stubFetch(200, { status: "not_vaulted" });

    await requestMintedToken(PRINCIPAL);

    const { init, headers } = sentInit(fetchMock);
    const timestamp = headers["X-Temper-Timestamp"];
    const signed = `${timestamp}.${init.body as string}`;

    const withMint = createHmac("sha256", MINT_SECRET).update(signed).digest("hex");
    const withLink = createHmac("sha256", LINK_SECRET).update(signed).digest("hex");

    expect(headers["X-Temper-Signature"]).toBe(withMint);
    expect(headers["X-Temper-Signature"]).not.toBe(withLink);
  });

  // FAILS IF: SLACK_MINT_SECRET stops being required — e.g. someone "helpfully" falls back to
  // SLACK_LINK_SECRET when it is unset. With the link secret stubbed and non-empty, a fallback
  // would sign happily instead of throwing, so this asserts the absence of that fallback.
  it("throws when SLACK_MINT_SECRET is missing rather than falling back", async () => {
    vi.stubEnv("SLACK_MINT_SECRET", "");
    stubFetch(200, { status: "not_vaulted" });

    await expect(requestMintedToken(PRINCIPAL)).rejects.toThrow("SLACK_MINT_SECRET");
  });

  // FAILS IF: expires_at_ms is treated as seconds anywhere on this side — a `/ 1000` or a
  // `new Date(secs)` would land the expiry in 1970 while still "having a value". Asserting
  // presence would pass in that world; asserting MAGNITUDE and the resulting year does not.
  it("carries expires_at_ms through as epoch MILLISECONDS, unscaled", async () => {
    // 2026-07-20T00:00:00Z in ms. As seconds this instant is 1970-01-20.
    const EXPIRES_AT_MS = 1_784_505_600_000;
    stubFetch(200, { status: "token", access_token: "at-abc", expires_at_ms: EXPIRES_AT_MS });

    const outcome = await requestMintedToken(PRINCIPAL);

    if (outcome.status !== "token") throw new Error("expected the token arm");
    expect(outcome.expires_at_ms).toBe(EXPIRES_AT_MS);
    // Millisecond magnitude: epoch-seconds for any plausible date is < 1e11.
    expect(outcome.expires_at_ms).toBeGreaterThan(1e12);
    expect(new Date(outcome.expires_at_ms).getUTCFullYear()).toBe(2026);
  });

  // FAILS IF: the token arm loses its token, or the union stops narrowing on `status` — the
  // contract with the Rust `#[serde(tag = "status")]` enum at slack_mint.rs:38-57.
  it("returns the token arm with the access token", async () => {
    stubFetch(200, { status: "token", access_token: "at-abc", expires_at_ms: 1_784_505_600_000 });

    const outcome = await requestMintedToken(PRINCIPAL);

    if (outcome.status !== "token") throw new Error("expected the token arm");
    expect(outcome.access_token).toBe("at-abc");
  });

  // FAILS IF: `revoked` is mapped onto an error, or collapsed with not_vaulted. It is a 200 and
  // a distinct fact: the user must re-link, and retrying will never succeed.
  it("returns the revoked arm as a value, not a throw", async () => {
    stubFetch(200, { status: "revoked" });

    await expect(requestMintedToken(PRINCIPAL)).resolves.toEqual({ status: "revoked" });
  });

  // FAILS IF: `not_vaulted` is collapsed into revoked or into an error. It is reachable for a
  // user whom link-state calls `linked` (slack_mint.rs:52-56), so the agent must be able to
  // tell this apart and not claim things are working.
  it("returns the not_vaulted arm as a value, not a throw", async () => {
    stubFetch(200, { status: "not_vaulted" });

    await expect(requestMintedToken(PRINCIPAL)).resolves.toEqual({ status: "not_vaulted" });
  });

  // FAILS IF: the principal is split/reordered, the route path drifts from /internal/slack/mint
  // (e.g. copied as link-state), or the base URL's trailing slash is doubled up.
  it("posts the WHOLE principal to the mint route with no trailing-slash dupe", async () => {
    const fetchMock = stubFetch(200, { status: "not_vaulted" });

    await requestMintedToken(PRINCIPAL);

    const { url, init, headers } = sentInit(fetchMock);
    expect(url).toBe("https://temper.test/internal/slack/mint");
    expect(init.body).toBe(JSON.stringify({ slack_principal_id: PRINCIPAL }));
    expect(headers["X-Temper-Timestamp"]).toMatch(/^\d+$/);
    expect(headers["X-Temper-Signature"]).toMatch(/^[0-9a-f]{64}$/);
  });

  // FAILS IF: a non-2xx is swallowed and returned as an outcome. A 401 here means the mint
  // secret drifted from temper-api's; silence would leave the user with nothing.
  it("throws on a non-2xx", async () => {
    stubFetch(401, {});

    await expect(requestMintedToken(PRINCIPAL)).rejects.toThrow("mint failed: 401");
  });
});
