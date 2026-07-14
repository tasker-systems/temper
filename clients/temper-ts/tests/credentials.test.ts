import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { afterEach, describe, expect, it } from "vitest";
import { ClientCredentials, TokenMintError } from "../src/index.js";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

let issuer: MockIssuer | undefined;
let broken: Server | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
  if (broken !== undefined) {
    await new Promise<void>((resolve, reject) => broken?.close((err) => (err ? reject(err) : resolve())));
    broken = undefined;
  }
});

/**
 * An issuer that answers 200 with a body that is NOT the contract's. Deliberately not a mode of the
 * shared mock: that mock's job is to be faithful to two real issuers, and neither of them does this.
 */
async function startBrokenIssuer(body: unknown): Promise<string> {
  const server = createServer((_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(typeof body === "string" ? body : JSON.stringify(body));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  broken = server;
  return `http://127.0.0.1:${(server.address() as AddressInfo).port}/oauth/token`;
}

describe("ClientCredentials against a temper-issued (tmpr_) credential", () => {
  it("mints with NO audience — the AS ignores one, so sending it would be a lie", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
    expect(issuer.requests[0]?.contentType).toBe("application/x-www-form-urlencoded");
    expect(issuer.requests[0]?.params.audience).toBeUndefined();
  });

  it("caches against an ABSOLUTE expiry and re-mints only past the skew", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    let now = 1_000_000;
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      now: () => now,
    });

    expect(await creds.token()).toBe("temper-as-token-1");

    // Inside the token's life, outside the 60s skew — the cache holds.
    now += 800_000;
    expect(await creds.token()).toBe("temper-as-token-1");
    expect(issuer.requests).toHaveLength(1);

    // Inside the 60s skew of a 900s token — re-mint AHEAD of expiry rather than racing it.
    now += 60_000;
    expect(await creds.token()).toBe("temper-as-token-2");
    expect(issuer.requests).toHaveLength(2);
  });

  it("reports the absolute expiry eve needs to refresh ahead of a 401", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      now: () => 1_000_000,
    });

    expect(await creds.tokenResult()).toEqual({
      token: "temper-as-token-1",
      expiresAt: 1_000_000 + 900_000,
    });
  });

  it("re-mints on refresh() even when the cached token is still fresh", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
    // This is the fix the gem documented against the steward: refresh-ahead-of-expiry alone is
    // insufficient, because a tick can outlive a token it checked at the top.
    expect((await creds.refresh()).token).toBe("temper-as-token-2");
    expect(await creds.token()).toBe("temper-as-token-2");
  });

  it("mints with the rotated-out previous secret while its grace window is open", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() + 60_000,
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "old-secret",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
  });

  // The steward fans N maps out over one token; a token that dies mid-tick 401s all N at once, and
  // every one of them calls refresh(). Without coalescing that is N mints for one expiry — N billed
  // tokens, last-writer-wins on the cache. This is what the gem's mutex buys, in JS idiom.
  it("coalesces concurrent refreshes onto ONE mint", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    const tokens = await Promise.all([creds.refresh(), creds.refresh(), creds.refresh(), creds.refresh()]);

    expect(issuer.requests).toHaveLength(1);
    expect(tokens.map((t) => t.token)).toEqual(Array(4).fill("temper-as-token-1"));
  });

  it("coalesces the concurrent re-mints of an expired cache, and mints again once it settles", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    let now = 1_000_000;
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      now: () => now,
    });

    await creds.token();
    now += 900_000;

    expect(await Promise.all([creds.token(), creds.token(), creds.token()])).toEqual(
      Array(3).fill("temper-as-token-2"),
    );
    expect(issuer.requests).toHaveLength(2);

    // The memo is an IN-FLIGHT coalescer, not a second cache: once it settles it must be gone, or a
    // later genuine refresh would be answered with the stale minted token forever.
    expect((await creds.refresh()).token).toBe("temper-as-token-3");
  });

  // A failed mint must not wedge the memo: the next attempt has to reach the issuer again.
  it("clears the in-flight memo when a mint FAILS", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "wrong",
    });

    await expect(creds.refresh()).rejects.toThrow(TokenMintError);
    await expect(creds.refresh()).rejects.toThrow(TokenMintError);
    expect(issuer.requests).toHaveLength(2);
  });

  it("throws TokenMintError carrying the status on a bad secret", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "wrong",
    });

    await expect(creds.token()).rejects.toThrow(TokenMintError);
    await expect(creds.token()).rejects.toMatchObject({ status: 401 });
  });
});

// A cast would let all of these through. `Bearer undefined` on every request is the visible half; the
// invisible half is that `expiresAt` becomes NaN, every NaN comparison is false, and the cache is
// then judged expired forever — a mint on EVERY call, for the life of the process.
describe("ClientCredentials against an issuer that answers 200 with a body it should not", () => {
  it("throws rather than caching a token that is not there", async () => {
    const creds = new ClientCredentials({
      tokenUrl: await startBrokenIssuer({ token_type: "Bearer", expires_in: 900 }),
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    await expect(creds.token()).rejects.toThrow(TokenMintError);
    await expect(creds.token()).rejects.toMatchObject({ status: 200 });
  });

  it("throws rather than caching an expiry that is not a number", async () => {
    const creds = new ClientCredentials({
      tokenUrl: await startBrokenIssuer({ access_token: "tok", token_type: "Bearer" }),
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    await expect(creds.token()).rejects.toThrow(TokenMintError);
  });

  it("throws rather than reading a token out of a body that is not JSON at all", async () => {
    const creds = new ClientCredentials({
      tokenUrl: await startBrokenIssuer("<html>gateway says hello</html>"),
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    await expect(creds.token()).rejects.toThrow(TokenMintError);
  });
});

describe("ClientCredentials against an Auth0-provisioned credential", () => {
  it("sends the audience when configured — Auth0 requires it", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    expect(await creds.token()).toBe("auth0-token-1");
    expect(issuer.requests[0]?.params.audience).toBe("https://temperkb.io/api");
  });

  // The bite: the SAME client object, given no audience, must fail against Auth0. If this passes,
  // the audience is not actually reaching the wire and the test above proves nothing.
  it("fails against Auth0 when no audience is configured", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
    });

    await expect(creds.token()).rejects.toMatchObject({ status: 400 });
  });
});
