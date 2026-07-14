import { afterEach, describe, expect, it } from "vitest";

import { createAuthedFetch } from "../src/auth-fetch.js";
import { BearerToken, type Credentials, type TokenResult } from "../src/credentials.js";
import { type MockApi, type MockIssuer, startMockApi } from "../src/testing/index.js";
import { machineCredentials, startTemperAs } from "./support.js";

let api: MockApi | undefined;
let issuer: MockIssuer | undefined;

afterEach(async () => {
  await api?.close();
  await issuer?.close();
  api = undefined;
  issuer = undefined;
});

/** A promise a test resolves by hand — the only way to pin an interleaving without racing a timer. */
function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/**
 * A `Credentials` that COUNTS its mints, so a test can assert how many tokens a fan-out actually
 * bought. Neither mock issuer can stand in here: what is under test is the number of times the
 * caller decides to mint, not what an issuer does when asked.
 */
class CountingCredentials implements Credentials {
  readonly canRefresh = true;
  mints = 0;
  #serial = 1;
  #cached = "t1";
  /** Awaited INSIDE the mint — the test uses it to hold a mint open until the interleaving it wants exists. */
  beforeMint: Promise<void> = Promise.resolve();
  /** Called after each mint lands in the cache. */
  onMinted: () => void = () => {};

  async token(): Promise<string> {
    return this.#cached;
  }

  async tokenResult(): Promise<TokenResult> {
    return { token: this.#cached, expiresAt: Number.POSITIVE_INFINITY };
  }

  async refresh(): Promise<TokenResult> {
    await this.beforeMint;
    this.mints += 1;
    this.#serial += 1;
    this.#cached = `t${this.#serial}`;
    const result = await this.tokenResult();
    this.onMinted();
    return result;
  }
}

describe("createAuthedFetch", () => {
  it("presents a bearer token and the sdk surface header", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    expect(api.bearers).toHaveLength(1);
    expect(api.bearers[0]).not.toBe("");
    expect(api.surfaces).toEqual(["sdk"]);
  });

  it("re-mints once on a 401 and retries with the NEW token", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    // Two presentations, and the retry carried a DIFFERENT token — a blind replay of the
    // dead one would 401 forever.
    expect(api.bearers).toHaveLength(2);
    expect(api.bearers[0]).not.toBe(api.bearers[1]);
    // `requests` (NOT `mints`) is what MockIssuer records — one mint for the original token,
    // one for the re-mint.
    expect(issuer.requests).toHaveLength(2);
  });

  it("replays a POST body on the retry", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(
      new Request(api.url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ title: "hello" }),
      }),
    );

    expect(res.status).toBe(200);
    // The first send CONSUMES the request body. A retry built from the consumed request
    // would send an empty one — silently writing nothing, on the exact path a 401 recovery
    // exists to save.
    expect(api.bodies).toEqual(['{"title":"hello"}', '{"title":"hello"}']);
  });

  it("returns a 401 UNTOUCHED when the strategy cannot mint", async () => {
    api = await startMockApi({ rejectFirst: 1 });

    const authed = createAuthedFetch({ credentials: new BearerToken("static-token") });
    const res = await authed(new Request(api.url));

    // BearerToken.refresh() THROWS. Calling it here would replace temper's real 401 — the
    // answer a human is trying to read, body and all — with "BearerToken cannot refresh".
    expect(res.status).toBe(401);
    expect(await res.json()).toEqual({ error: "unauthorized" });
    expect(api.bearers).toEqual(["static-token"]);
  });

  it("retries exactly once — a 401 that survives a fresh token is a real denial", async () => {
    api = await startMockApi({ rejectFirst: 99 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(401);
    expect(api.bearers).toHaveLength(2); // original + one retry, never a third
  });

  it("rides a token another request already re-minted instead of buying a second one", async () => {
    // The shape this guards is the ORDINARY one: a caller fans N requests out over one token, the
    // token dies mid-flight, and the N 401s come back STAGGERED. `refresh()` mints
    // unconditionally, and its in-flight memo only coalesces callers that overlap the mint itself
    // — which staggered 401s, by definition, do not. So each late 401 buys another token for an
    // expiry that has already been paid for.
    const credentials = new CountingCredentials();
    const minted = deferred();
    const bothSent = deferred();
    credentials.onMinted = minted.resolve;

    // The first request's re-mint waits until BOTH requests have presented the dead token — that
    // is what makes them contemporaries rather than a sequence, with no timer to race.
    credentials.beforeMint = bothSent.promise;

    const bearers: string[] = [];
    let deadSends = 0;
    const inner = async (input: Request): Promise<Response> => {
      const bearer = (input.headers.get("authorization") ?? "").replace(/^Bearer /, "");
      bearers.push(bearer);
      if (bearer !== "t1") {
        return new Response(JSON.stringify({ ok: true }), { status: 200 });
      }
      deadSends += 1;
      if (deadSends === 2) {
        bothSent.resolve();
        // Hold the SECOND request's 401 until the first has finished re-minting. This is the
        // staggered arrival — the interleaving in which the memo does nothing.
        await minted.promise;
      }
      return new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 });
    };

    const authed = createAuthedFetch({ credentials, fetch: inner });
    const [first, second] = await Promise.all([
      authed(new Request("http://api.test/api/health")),
      authed(new Request("http://api.test/api/health")),
    ]);

    expect(first.status).toBe(200);
    expect(second.status).toBe(200);
    // ONE mint answers ONE expiry. The second 401 found `t2` already in the cache and rode it.
    expect(credentials.mints).toBe(1);
    expect(bearers).toEqual(["t1", "t1", "t2", "t2"]);
  });

  it("composes an inner fetch (the steward keeps its cold-start retry)", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const seen: string[] = [];
    const inner = (input: Request): Promise<Response> => {
      seen.push(input.url);
      return fetch(input);
    };

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url), fetch: inner });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    expect(seen).toEqual([api.url]);
  });
});
