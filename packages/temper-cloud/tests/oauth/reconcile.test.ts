import { afterEach, describe, expect, it, vi } from "vitest";
import { reconcileMemberships } from "../../src/oauth/reconcile.js";

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
  it("POSTs to INTERNAL_RECONCILE_URL with the secret header", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await reconcileMemberships(payload);

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://api.internal/internal/saml/reconcile");
    expect((init as RequestInit).method).toBe("POST");
    expect((init as RequestInit).headers).toMatchObject({
      "content-type": "application/json",
      "X-Temper-Internal-Secret": "s3cr3t",
    });
    expect(JSON.parse((init as RequestInit).body as string)).toMatchObject({ idp_key: "acme" });
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
