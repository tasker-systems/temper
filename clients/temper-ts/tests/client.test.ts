import { afterEach, describe, expect, it } from "vitest";

import { createTemperClient } from "../src/client.js";
import { ClientCredentials, TokenMintError } from "../src/credentials.js";
import { type MockApi, type MockIssuer, startMockApi } from "../src/testing/index.js";
import { CLIENT_ID, machineCredentials, startTemperAs } from "./support.js";

let api: MockApi | undefined;
let issuer: MockIssuer | undefined;

afterEach(async () => {
  await api?.close();
  await issuer?.close();
  api = undefined;
  issuer = undefined;
});

describe("createTemperClient", () => {
  it("calls a contract path with auth and the surface header", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const client = createTemperClient({
      baseUrl: new URL(api.url).origin,
      credentials: machineCredentials(issuer.url),
    });

    // `/api/health` is a real path in the contract — this line does not compile if the
    // generated schema does not carry it, which is the point of generating it.
    const { response } = await client.GET("/api/health");

    expect(response.status).toBe(200);
    expect(api.bearers).toHaveLength(1);
    expect(api.surfaces).toEqual(["sdk"]);
  });

  it("carries the 401 re-mint through to a contract call", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const client = createTemperClient({
      baseUrl: new URL(api.url).origin,
      credentials: machineCredentials(issuer.url),
    });

    const { response } = await client.GET("/api/health");

    expect(response.status).toBe(200);
    expect(api.bearers).toHaveLength(2);
    expect(api.bearers[0]).not.toBe(api.bearers[1]);
  });

  it("refuses a `.` or `..` path value before any request is sent", async () => {
    issuer = await startTemperAs();
    const seen: string[] = [];
    const client = createTemperClient({
      baseUrl: "https://temper.example",
      credentials: machineCredentials(issuer.url),
      fetch: async (input) => {
        seen.push(input.url);
        return new Response("{}", { headers: { "content-type": "application/json" } });
      },
    });

    // Encoding cannot carry these: the WHATWG URL parser behind fetch reads `%2E%2E` as a dot
    // segment as well, so the request would reach the parent route. This pins that premise.
    expect(new URL("https://h/api/schema/doc-types/%2E%2E").pathname).toBe("/api/schema/");

    for (const name of [".", ".."]) {
      await expect(
        client.GET("/api/schema/doc-types/{name}", { params: { path: { name } } }),
      ).rejects.toThrow(/parent route/);
    }
    // The serialized segment is what the URL parser sees: a one-element array serializes to `..`.
    await expect(
      client.GET("/api/schema/doc-types/{name}", {
        params: { path: { name: [".."] as unknown as string } },
      }),
    ).rejects.toThrow(/parent route/);
    expect(seen).toEqual([]);

    await client.GET("/api/schema/doc-types/{name}", { params: { path: { name: "a.b" } } });
    expect(seen).toEqual(["https://temper.example/api/schema/doc-types/a.b"]);
  });

  it("THROWS a mint failure — it does not arrive as a typed `error`", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const client = createTemperClient({
      baseUrl: new URL(api.url).origin,
      credentials: new ClientCredentials({
        tokenUrl: issuer.url,
        clientId: CLIENT_ID,
        clientSecret: "wrong",
      }),
    });

    // The authed fetch IS the injected fetch, and openapi-fetch rethrows whatever its fetch throws
    // — no `onError` middleware catches this. So the commonest M2M misconfiguration there is, a bad
    // secret, does NOT come back in `error`: an `if (error)` with no `try` around it lands as an
    // unhandled rejection. The README says so because this test says so.
    await expect(client.GET("/api/health")).rejects.toBeInstanceOf(TokenMintError);
    // Never sent: the mint fails before a request exists.
    expect(api.bearers).toEqual([]);
  });
});
