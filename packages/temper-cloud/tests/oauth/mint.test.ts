import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import { beforeAll, describe, expect, it } from "vitest";

beforeAll(async () => {
  const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
  process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
  process.env.AS_SIGNING_KID = "test-kid-1";
  process.env.AS_ISSUER = "https://issuer.test";
  process.env.AS_AUDIENCE = "https://audience.test";
});

describe("mintAccessToken", () => {
  it("mints an EdDSA access token verifiable via the public JWKS", async () => {
    const { mintAccessToken } = await import("../../src/oauth/mint.js");
    const { getPublicJwks } = await import("../../src/oauth/keys.js");

    const jwt = await mintAccessToken({
      sub: "u1",
      email: "u1@x.io",
      email_verified: true,
    });

    const JWKS = createLocalJWKSet(await getPublicJwks());
    const { payload, protectedHeader } = await jwtVerify(jwt, JWKS, {
      issuer: process.env.AS_ISSUER,
      audience: process.env.AS_AUDIENCE,
    });

    expect(protectedHeader.alg).toBe("EdDSA");
    expect(protectedHeader.kid).toBe("test-kid-1");
    expect(payload).toMatchObject({
      sub: "u1",
      email: "u1@x.io",
      email_verified: true,
    });
    expect(typeof payload.exp).toBe("number");
    expect(typeof payload.iat).toBe("number");
  });

  it("defaults the access TTL to 900 seconds when unset", async () => {
    delete process.env.AS_ACCESS_TTL_SECONDS;
    const { mintAccessToken } = await import("../../src/oauth/mint.js");

    const jwt = await mintAccessToken({
      sub: "u2",
      email: "u2@x.io",
      email_verified: false,
    });

    const parts = jwt.split(".");
    const payload = JSON.parse(Buffer.from(parts[1] ?? "", "base64url").toString("utf8"));
    expect(payload.exp - payload.iat).toBe(900);
  });
});

describe("newOpaqueToken / hashToken", () => {
  it("generates distinct 32-byte base64url tokens and a stable sha256 hex hash", async () => {
    const { newOpaqueToken, hashToken } = await import("../../src/oauth/mint.js");

    const a = newOpaqueToken();
    const b = newOpaqueToken();
    expect(a).not.toBe(b);
    expect(Buffer.from(a, "base64url").length).toBe(32);

    const hashA1 = hashToken(a);
    const hashA2 = hashToken(a);
    expect(hashA1).toBe(hashA2);
    expect(hashA1).toMatch(/^[0-9a-f]{64}$/);
  });
});
