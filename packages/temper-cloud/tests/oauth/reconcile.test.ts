import { afterEach, describe, expect, it, vi } from "vitest";
import {
  reconcileMemberships,
  SIGNATURE_HEADER,
  signReconcile,
  TIMESTAMP_HEADER,
} from "../../src/oauth/reconcile.js";

const payload = {
  provider: "saml:acme",
  external_user_id: "nid-1",
  email: "a@corp.io",
  email_verified: true,
  idp_key: "acme",
  groups: ["engineering"],
};

afterEach(() => vi.unstubAllGlobals());

describe("reconcileMemberships", () => {
  it("POSTs to INTERNAL_RECONCILE_URL with a valid HMAC signature over the body", async () => {
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

    // The signature must be valid for the exact body + timestamp that were sent.
    const timestamp = Number(headerRecord[TIMESTAMP_HEADER]);
    expect(Number.isInteger(timestamp)).toBe(true);
    expect(headerRecord[SIGNATURE_HEADER]).toBe(signReconcile("s3cr3t", timestamp, body as string));
    expect(JSON.parse(body as string)).toMatchObject({ idp_key: "acme" });
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
});
