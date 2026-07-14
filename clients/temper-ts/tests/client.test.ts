import { afterEach, describe, expect, it } from "vitest";

import { createTemperClient } from "../src/client.js";
import { ClientCredentials } from "../src/credentials.js";
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

/** `startMockIssuer` requires flavor/clientId/clientSecret — there is no zero-arg form. */
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
});
