import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "temper-ts/testing";

let issuer: MockIssuer | undefined;
let api: MockApi | undefined;

beforeEach(() => {
  vi.resetModules();
  delete process.env.TEMPER_M2M_CLIENT_ID;
  delete process.env.TEMPER_M2M_CLIENT_SECRET;
  delete process.env.TEMPER_M2M_TOKEN_URL;
  delete process.env.TEMPER_M2M_AUDIENCE;
  delete process.env.TEMPER_CONNECT_CONNECTOR;
  delete process.env.TEMPER_TOKEN;
});

afterEach(async () => {
  await issuer?.close();
  await api?.close();
  issuer = undefined;
  api = undefined;
});

describe("temper-auth env composition", () => {
  it("omits the audience when TEMPER_M2M_AUDIENCE is unset — a tmpr_ credential must be able to", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("temper-as-token-1");
    expect(issuer.requests[0]?.params.audience).toBeUndefined();
  });

  it("sends the audience when TEMPER_M2M_AUDIENCE is set — Auth0 requires it", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    process.env.TEMPER_M2M_CLIENT_ID = "auth0_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;
    process.env.TEMPER_M2M_AUDIENCE = "https://temperkb.io/api";

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("auth0-token-1");
    expect(issuer.requests[0]?.params.audience).toBe("https://temperkb.io/api");
  });

  it("mintM2mToken reports an absolute expiry, which is what eve refreshes against", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { mintM2mToken } = await import("../agent/lib/temper-auth.js");
    const before = Date.now();
    const result = await mintM2mToken();

    expect(result.token).toBe("temper-as-token-1");
    expect(result.expiresAt).toBeGreaterThanOrEqual(before + 900_000);
  });

  it("falls back to the static TEMPER_TOKEN when no machine identity is configured", async () => {
    process.env.TEMPER_TOKEN = "dev-token";

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("dev-token");
  });
});

describe("temperFetch", () => {
  it("re-mints ONCE on a 401 and retries with the fresh token", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi({ rejectFirst: 1 });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(200);
    // The retry must carry a DIFFERENT token. Replaying the dead one would 401 again — and a test
    // that only asserted "two requests" would pass even then.
    expect(api.bearers).toEqual(["temper-as-token-1", "temper-as-token-2"]);
    expect(issuer.requests).toHaveLength(2);
  });

  it("gives up after ONE re-mint — a persistent 401 is a real authz failure, not an expiry", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi({ rejectFirst: 99 });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(401);
    expect(api.bearers).toHaveLength(2);
  });

  it("does not re-mint on a 200 — the happy path mints exactly once", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi();
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "GET" });

    expect(res.status).toBe(200);
    expect(issuer.requests).toHaveLength(1);
  });
});
