import { afterEach, describe, expect, it } from "vitest";

import { createAuthedFetch } from "../src/auth-fetch.js";
import { BearerToken, ClientCredentials } from "../src/credentials.js";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "../src/testing/index.js";

let api: MockApi | undefined;
let issuer: MockIssuer | undefined;

const CLIENT_ID = "tmpr_test";
const CLIENT_SECRET = "s3cr3t";

afterEach(async () => {
  await api?.close();
  await issuer?.close();
  api = undefined;
  issuer = undefined;
});

/**
 * `startMockIssuer` REQUIRES flavor/clientId/clientSecret — there is no zero-arg form. The
 * `temper-as` flavor is the one that matters here: it mints 900s tokens and ignores a
 * request-supplied audience, exactly as the real AS does.
 */
async function startTemperAs(): Promise<MockIssuer> {
  return startMockIssuer({ flavor: "temper-as", clientId: CLIENT_ID, clientSecret: CLIENT_SECRET });
}

function machineCredentials(issuerUrl: string): ClientCredentials {
  return new ClientCredentials({
    tokenUrl: issuerUrl,
    clientId: CLIENT_ID,
    clientSecret: CLIENT_SECRET,
  });
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
