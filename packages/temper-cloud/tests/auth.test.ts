import { describe, it, expect, beforeAll } from "vitest";
import * as jose from "jose";
import { verifyToken } from "../src/auth.js";
import { readFileSync } from "fs";
import { resolve } from "path";

// Load the same Ed25519 test keys used by the Rust tests.
const privateKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.key"),
  "utf-8"
);
const publicKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.pub"),
  "utf-8"
);

let privateKey: jose.KeyLike;
let publicKey: jose.KeyLike;

beforeAll(async () => {
  privateKey = await jose.importPKCS8(privateKeyPem, "EdDSA");
  publicKey = await jose.importSPKI(publicKeyPem, "EdDSA");
});

async function signTestJwt(claims: Record<string, unknown>): Promise<string> {
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

    const claims = await verifyToken(token, publicKey, "test-issuer");
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

    await expect(verifyToken(token, publicKey, "test-issuer")).rejects.toThrow();
  });

  it("rejects a JWT with wrong issuer", async () => {
    const token = await signTestJwt({ sub: "user-789", email: "wrong@example.com", email_verified: true });

    await expect(verifyToken(token, publicKey, "wrong-issuer")).rejects.toThrow();
  });
});
