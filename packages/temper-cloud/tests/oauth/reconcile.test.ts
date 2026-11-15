import { afterEach, describe, expect, it, vi } from "vitest";
import { logger } from "../../src/logger.js";
import {
  NONCE_HEADER,
  RECONCILE_DOOR,
  reconcileMemberships,
  SIGNATURE_V2_HEADER,
  signRequestV2,
  TIMESTAMP_HEADER,
} from "../../src/oauth/reconcile.js";
import { classifyInternalCallFailure } from "../../src/oauth/reconcile-health.js";

const payload = {
  provider: "saml:acme",
  external_user_id: "nid-1",
  email: "a@corp.io",
  email_verified: true,
  idp_key: "acme",
  groups: ["engineering"],
};

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("reconcileMemberships", () => {
  it("POSTs to INTERNAL_RECONCILE_URL with a valid v2 signature over the body", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await reconcileMemberships(payload);

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    const { headers, body } = init as RequestInit;
    expect(url).toBe("https://api.internal/internal/saml/reconcile");
    expect((init as RequestInit).method).toBe("POST");

    // The old raw-secret header is gone; the secret never travels the wire.
    const headerRecord = headers as Record<string, string>;
    expect(headerRecord["X-Temper-Internal-Secret"]).toBeUndefined();
    expect(headerRecord["content-type"]).toBe("application/json");

    // FAILS IF: the retired `X-Temper-Signature` construction rides beside v2 again. It binds
    // neither the door nor the occasion, and temper-api reads only the v2 headers.
    expect(headerRecord["X-Temper-Signature"]).toBeUndefined();

    const timestamp = Number(headerRecord[TIMESTAMP_HEADER]);
    expect(Number.isInteger(timestamp)).toBe(true);
    expect(JSON.parse(body as string)).toMatchObject({ idp_key: "acme" });

    // The v2 signature names this door and a fresh nonce, so the verifier can hold it to one
    // door, once.
    const nonce = headerRecord[NONCE_HEADER];
    expect(nonce).toMatch(/^[0-9a-f]{32}$/);
    expect(headerRecord[SIGNATURE_V2_HEADER]).toBe(
      signRequestV2("s3cr3t", "POST", RECONCILE_DOOR, timestamp, nonce, body as string),
    );
  });

  it("signs each request under a fresh nonce", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await reconcileMemberships(payload);
    await reconcileMemberships(payload);

    const nonces = fetchMock.mock.calls.map(
      ([, init]) => ((init as RequestInit).headers as Record<string, string>)[NONCE_HEADER],
    );
    expect(nonces[0]).not.toBe(nonces[1]);
  });

  it("throws on a non-2xx response", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("nope", { status: 500 })),
    );
    await expect(reconcileMemberships(payload)).rejects.toThrow();
  });

  it("binds the group count and no identifier on the success log line", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(null, { status: 204 })),
    );
    const infoSpy = vi.spyOn(logger, "info").mockImplementation(() => undefined);

    await reconcileMemberships(payload);

    expect(infoSpy).toHaveBeenCalledOnce();
    const [bindings] = infoSpy.mock.calls[0] as unknown as [Record<string, unknown>, string];
    expect(bindings).toEqual({ groups: 1 });
  });

  // FAILS IF: a 401 from the API stops being the conclusive `unauthorized`. The reword widened
  // what it means (signature refused or door not served); it did not soften it.
  it("a 401 stays unauthorized", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("no", { status: 401 })),
    );
    const err = await reconcileMemberships(payload).then(
      () => null,
      (e: unknown) => e,
    );
    expect(classifyInternalCallFailure(err)).toEqual({ cause: "unauthorized", detail: "HTTP 401" });
  });

  // FAILS IF: a URL whose path is not the door is signed and sent. The API would answer it 401,
  // and the reader would page about the secret; caught here, the cause names the URL instead.
  it.each([
    ["a stray segment", "https://api.internal/v1/internal/saml/reconcile"],
    ["a trailing slash", "https://api.internal/internal/saml/reconcile/"],
    ["the wrong door", "https://api.internal/internal/principal/resolve"],
    ["no path at all", "https://api.internal"],
    ["a value that is not a URL", "api.internal/internal/saml/reconcile"],
  ])("%s is config_missing and nothing is sent", async (_what, url) => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", url);
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const err = await reconcileMemberships(payload).then(
      () => null,
      (e: unknown) => e,
    );
    expect(classifyInternalCallFailure(err)).toEqual({
      cause: "config_missing",
      detail: "INTERNAL_RECONCILE_URL path",
    });
    expect(fetchMock).not.toHaveBeenCalled();
    // The detail is a fixed string; the URL itself is not echoed into the message either.
    expect((err as Error).message).not.toContain("api.internal");
  });
});
