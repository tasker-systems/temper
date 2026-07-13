import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, expect, it } from "vitest";
import { ClientCredentials } from "../src/index.js";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

// The client half of the cross-language wire contract in tests/contracts/m2m-token-request.json.
// The gem's spec asserts IT emits this shape; the AS's integration test asserts the server ACCEPTS
// it. Neither catches a mismatch alone — which is exactly how the gem shipped a JSON mint against a
// formData() parser with both suites green.
const contract = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../tests/contracts/m2m-token-request.json", import.meta.url)), "utf8"),
) as {
  content_type: string;
  required_params: string[];
  grant_type: string;
};

let issuer: MockIssuer | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
});

it("emits exactly the content type and params the shared wire contract requires", async () => {
  issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_c", clientSecret: "s3cr3t" });
  const creds = new ClientCredentials({
    tokenUrl: issuer.url,
    clientId: "tmpr_c",
    clientSecret: "s3cr3t",
  });

  await creds.token();

  const sent = issuer.requests[0];
  expect(sent?.contentType).toBe(contract.content_type);
  expect(Object.keys(sent?.params ?? {})).toEqual(expect.arrayContaining(contract.required_params));
  expect(sent?.params.grant_type).toBe(contract.grant_type);
});
