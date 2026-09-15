import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import * as jose from "jose";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
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

describe("verifyToken email_verified handling", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // A token with no `email_verified` claim carries no verdict. `null` says "the IdP said
  // nothing"; `false` would say "the IdP says not verified" — a verdict nobody gave.
  it("returns null when the token omits the email_verified claim", async () => {
    const token = await signTestJwt({
      sub: "user-321",
      email: "silent@example.com",
    });

    const claims = await verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE);
    expect(claims.email_verified).toBeNull();
  });

  it("returns false when the token asserts email_verified false", async () => {
    const token = await signTestJwt({
      sub: "user-322",
      email: "unverified@example.com",
      email_verified: false,
    });

    const claims = await verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE);
    expect(claims.email_verified).toBe(false);
  });

  it("carries an absent userinfo email_verified as null", async () => {
    const token = await signTestJwt({ sub: "user-323" });
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ email: "userinfo@example.com" }), { status: 200 }),
      ),
    );

    const claims = await verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE);
    expect(claims.email).toBe("userinfo@example.com");
    expect(claims.email_verified).toBeNull();
  });

  it("carries a userinfo email_verified false as false", async () => {
    const token = await signTestJwt({ sub: "user-324" });
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ email: "userinfo@example.com", email_verified: false }), {
            status: 200,
          }),
      ),
    );

    const claims = await verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE);
    expect(claims.email_verified).toBe(false);
  });
});

describe("verifyToken userinfo body validation", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("rejects a userinfo body whose email is not a string", async () => {
    const token = await signTestJwt({ sub: "user-325" });
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ email: 123, email_verified: true }), { status: 200 }),
      ),
    );

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "non-string email",
    );
  });

  it("rejects a userinfo body whose email_verified is not a boolean", async () => {
    const token = await signTestJwt({ sub: "user-326" });
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ email: "userinfo@example.com", email_verified: "true" }), {
            status: 200,
          }),
      ),
    );

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "non-boolean email_verified",
    );
  });

  it("rejects a userinfo body that is not a JSON object", async () => {
    const token = await signTestJwt({ sub: "user-327" });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify("not-an-object"), { status: 200 })),
    );

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "not a JSON object",
    );
  });
});

describe("verifyToken JWT claim validation", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("rejects a JWT whose email claim is not a string", async () => {
    const token = await signTestJwt({
      sub: "user-328",
      email: 123,
      email_verified: true,
    });

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "non-string email claim",
    );
  });

  it("rejects a JWT whose sub claim is not a string", async () => {
    const token = await signTestJwt({
      sub: 123,
      email: "stringy@example.com",
      email_verified: true,
    });

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "non-string sub claim",
    );
  });

  it("rejects a JWT whose email_verified claim is not a boolean", async () => {
    const token = await signTestJwt({
      sub: "user-329",
      email: "stringy@example.com",
      email_verified: "true",
    });

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "non-boolean email_verified claim",
    );
  });

  it("rejects a token carrying neither email claim when userinfo yields no email either", async () => {
    const token = await signTestJwt({ sub: "user-330" });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({}), { status: 200 })),
    );

    await expect(verifyToken(token, publicKey, "test-issuer", TEST_AUDIENCE)).rejects.toThrow(
      "JWT missing email claim",
    );
  });
});
