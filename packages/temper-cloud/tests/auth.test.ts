import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import * as jose from "jose";
import { beforeAll, describe, expect, it } from "vitest";
import { verifyToken } from "../src/auth.js";

// Load the same Ed25519 test keys used by the Rust tests.
const privateKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.key"),
  "utf-8",
);
const publicKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.pub"),
  "utf-8",
);

let privateKey: jose.KeyLike;
let publicKey: jose.KeyLike;

beforeAll(async () => {
  privateKey = await jose.importPKCS8(privateKeyPem, "EdDSA");
  publicKey = await jose.importSPKI(publicKeyPem, "EdDSA");
});

const TEST_AUDIENCE = "test-audience";

async function signTestJwt(claims: Record<string, unknown>): Promise<string> {
  return new jose.SignJWT(claims as jose.JWTPayload)
    .setProtectedHeader({ alg: "EdDSA" })
    .setIssuedAt()
    .setExpirationTime("1h")
    .setIssuer("test-issuer")
    .setAudience(TEST_AUDIENCE)
    .sign(privateKey);
}

/// Sign a token that omits `aud` entirely — the case jose (like jsonwebtoken) skips rather than
/// rejects unless the claim is explicitly required.
async function signJwtWithoutAudience(claims: Record<string, unknown>): Promise<string> {
  return new jose.SignJWT(claims as jose.JWTPayload)
    .setProtectedHeader({ alg: "EdDSA" })
    .setIssuedAt()
    .setExpirationTime("1h")
    .setIssuer("test-issuer")
    .sign(privateKey);
}

describe("verifyToken", () => {
  it("accepts a valid JWT and returns claims", async () => {
    const token = await signTestJwt({
      sub: "user-123",
      email: "test@example.com",
      email_verified: true,
    });

    const claims = await verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE);
    expect(claims.sub).toBe("user-123");
    expect(claims.email).toBe("test@example.com");
    expect(claims.email_verified).toBe(true);
  });

  it("rejects an expired JWT", async () => {
    const token = await new jose.SignJWT({
      sub: "user-456",
      email: "expired@example.com",
      email_verified: true,
    } as jose.JWTPayload)
      .setProtectedHeader({ alg: "EdDSA" })
      .setIssuedAt(Math.floor(Date.now() / 1000) - 7200)
      .setExpirationTime(Math.floor(Date.now() / 1000) - 3600)
      .setIssuer("test-issuer")
      .sign(privateKey);

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow();
  });

  it("rejects a JWT with wrong issuer", async () => {
    const token = await signTestJwt({
      sub: "user-789",
      email: "wrong@example.com",
      email_verified: true,
    });

    await expect(verifyToken(token, publicKey, "wrong-issuer")).rejects.toThrow();
  });
});

describe("verifyToken audience enforcement", () => {
  // This verifier passed NO audience option at all: it accepted any correctly-signed token from the
  // issuer, regardless of which API it was minted for. It has no live route today, which is exactly
  // why nobody noticed — an unwired gun pointed at the same database is still a gun.
  it("rejects a token minted for a different audience", async () => {
    const token = await new jose.SignJWT({
      sub: "user-789",
      email: "other@example.com",
      email_verified: true,
    } as jose.JWTPayload)
      .setProtectedHeader({ alg: "EdDSA" })
      .setIssuedAt()
      .setExpirationTime("1h")
      .setIssuer("test-issuer")
      .setAudience("https://some-other-api.example/api")
      .sign(privateKey);

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow();
  });

  // The subtler half: jose only COMPARES `aud` when the claim exists. Requiring the value to match
  // is not the same as requiring the claim to be present — hence `requiredClaims: ["aud", ...]`.
  it("rejects a token with no aud claim at all", async () => {
    const token = await signJwtWithoutAudience({
      sub: "user-000",
      email: "noaud@example.com",
      email_verified: true,
    });

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow();
  });
});
