import { exportPKCS8, generateKeyPair } from "jose";
import { beforeAll, describe, expect, it } from "vitest";

beforeAll(async () => {
  const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
  process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
  process.env.AS_SIGNING_KID = "test-kid-1";
});

describe("getPublicJwks", () => {
  it("publishes an EdDSA public JWKS without private material", async () => {
    const { getPublicJwks } = await import("../../src/oauth/keys.js");
    const jwks = await getPublicJwks();

    expect(jwks.keys).toHaveLength(1);
    expect(jwks.keys[0]).toMatchObject({
      kty: "OKP",
      crv: "Ed25519",
      alg: "EdDSA",
      use: "sig",
      kid: "test-kid-1",
    });
    expect(jwks.keys[0]).not.toHaveProperty("d");
  });
});

describe("getSigningKey", () => {
  it("returns the imported key with the configured kid", async () => {
    const { getSigningKey } = await import("../../src/oauth/keys.js");
    const { key, kid } = await getSigningKey();

    expect(kid).toBe("test-kid-1");
    expect(key).toBeDefined();
  });
});
