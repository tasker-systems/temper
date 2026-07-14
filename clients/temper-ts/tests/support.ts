import { ClientCredentials } from "../src/credentials.js";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

/**
 * Shared fixtures for the tests that need a real issuer and a real M2M credential. They live here
 * rather than being copied into each test file: the two copies had already drifted in their
 * comments, and a drifting fixture is a fixture that will eventually drift in its VALUES — two
 * suites silently testing two different clients.
 */

export const CLIENT_ID = "tmpr_test";
export const CLIENT_SECRET = "s3cr3t";

/**
 * `startMockIssuer` REQUIRES flavor/clientId/clientSecret — there is no zero-arg form. The
 * `temper-as` flavor is the one that matters here: it mints 900s tokens and ignores a
 * request-supplied audience, exactly as the real AS does.
 */
export async function startTemperAs(): Promise<MockIssuer> {
  return startMockIssuer({ flavor: "temper-as", clientId: CLIENT_ID, clientSecret: CLIENT_SECRET });
}

export function machineCredentials(issuerUrl: string): ClientCredentials {
  return new ClientCredentials({
    tokenUrl: issuerUrl,
    clientId: CLIENT_ID,
    clientSecret: CLIENT_SECRET,
  });
}
