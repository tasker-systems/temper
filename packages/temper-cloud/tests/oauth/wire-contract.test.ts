import { readFileSync } from "node:fs";
import { importSPKI, jwtVerify } from "jose";
import { beforeAll, describe, expect, it } from "vitest";
import { type ReconcileRequest, signReconcile } from "../../src/oauth/reconcile.js";

/**
 * Cross-language wire-contract proof (M1 Task 1.4).
 *
 * The Temper Authorization Server mints EdDSA JWTs (jose `SignJWT`) that
 * `temper-api`'s Rust `require_auth` must accept. This test mints a token with
 * `mint.ts` signed by the SAME Ed25519 fixture keypair the Rust e2e verifies
 * against (`tests/e2e/tests/fixtures/test_ed25519.{pkcs8,pub.pem}`), then:
 *   1. verifies it with jose against the fixture PUBLIC key (proves the token is
 *      a valid EdDSA signature over that exact keypair — the one Rust loads via
 *      `DecodingKey::from_ed_pem(test_ed25519.pub.pem)` in `setup_eddsa`), and
 *   2. asserts the payload matches the Rust `JwtClaims` contract exactly
 *      (`sub: String, email: Option<String>, email_verified: Option<bool>,
 *       exp: i64, iat: i64`) plus the validated `iss`/`aud`.
 *
 * Combined with the M0 e2e (Rust `require_auth` accepts a fixture-key EdDSA
 * token of this shape → `resolve_from_claims`), this transitively locks the
 * mint→validate contract across the TS and Rust runtimes.
 */

const FIXTURE_PRIVATE = new URL(
  "../../../../tests/e2e/tests/fixtures/test_ed25519.pkcs8",
  import.meta.url,
);
const FIXTURE_PUBLIC = new URL(
  "../../../../tests/e2e/tests/fixtures/test_ed25519.pub.pem",
  import.meta.url,
);

// Matches `setup_eddsa`'s ApiConfig in tests/e2e/tests/common/mod.rs.
const ISSUER = "test-issuer";
const AUDIENCE = "temper-api";

describe("AS mint → Rust JwtClaims wire contract", () => {
  beforeAll(() => {
    process.env.AS_SIGNING_KEY_PKCS8 = readFileSync(FIXTURE_PRIVATE, "utf8");
    process.env.AS_SIGNING_KID = "test-ed25519";
    process.env.AS_ISSUER = ISSUER;
    process.env.AS_AUDIENCE = AUDIENCE;
  });

  it("mints an EdDSA token verifiable by the fixture public key with the Rust JwtClaims shape", async () => {
    const { mintAccessToken } = await import("../../src/oauth/mint.js");
    const jwt = await mintAccessToken({
      sub: "wire-persistent-nameid",
      email: "wire@test.example",
      email_verified: true,
    });

    const publicKey = await importSPKI(readFileSync(FIXTURE_PUBLIC, "utf8"), "EdDSA");
    const { payload, protectedHeader } = await jwtVerify(jwt, publicKey, {
      issuer: ISSUER,
      audience: AUDIENCE,
    });

    // Signature algorithm the Rust side allow-lists for an Ed25519 key.
    expect(protectedHeader.alg).toBe("EdDSA");

    // Exact Rust `JwtClaims` contract (auth.rs): types must line up so serde
    // deserialization on the Rust side succeeds.
    expect(typeof payload.sub).toBe("string");
    expect(payload.sub).toBe("wire-persistent-nameid");
    expect(typeof payload.email).toBe("string");
    expect(payload.email).toBe("wire@test.example");
    expect(typeof payload.email_verified).toBe("boolean");
    expect(payload.email_verified).toBe(true);
    expect(typeof payload.exp).toBe("number");
    expect(typeof payload.iat).toBe("number");
    // exp is in the future relative to iat (short-lived access token).
    expect((payload.exp as number) > (payload.iat as number)).toBe(true);
  });
});

describe("ReconcileRequest wire contract (mirrors Rust temper_core::types::ReconcileRequest)", () => {
  it("has exactly the Rust struct fields", () => {
    // A fully-populated value must satisfy the interface with no extra/missing keys.
    const value: ReconcileRequest = {
      provider: "saml:acme",
      external_user_id: "nid-1",
      email: "a@corp.io",
      email_verified: true,
      idp_key: "acme",
      groups: ["engineering"],
    };
    expect(Object.keys(value).sort()).toEqual([
      "email",
      "email_verified",
      "external_user_id",
      "groups",
      "idp_key",
      "provider",
    ]);
    // nullables accept null (matches Option<..> on the Rust side)
    const nulls: ReconcileRequest = { ...value, provider: null, email_verified: null };
    expect(nulls.provider).toBeNull();
  });
});

describe("internal reconcile signature (mirrors Rust temper_core::internal_sig)", () => {
  // Shared known-answer vector. The identical inputs and expected signature are asserted
  // on the Rust side in crates/temper-core/src/internal_sig.rs, so the TS signer and the
  // Rust verifier cannot drift on the HMAC construction.
  const KAT_SECRET = "topsecret-abcdefghijklmnopqrstuvwxyz012345";
  const KAT_TIMESTAMP = 1_750_000_000;
  const KAT_BODY =
    '{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["engineering"]}';
  const KAT_SIG = "41eed1973f8f2e35fa65ff4e300f076fa08c206ca6c51434bae7d8a0c827d485";

  it("produces the shared known-answer signature", () => {
    expect(signReconcile(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY)).toBe(KAT_SIG);
  });
});
