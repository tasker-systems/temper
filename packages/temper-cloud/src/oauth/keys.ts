import { createPublicKey } from "node:crypto";
import type { JWK } from "jose";
import { exportJWK, importPKCS8 } from "jose";
import { requireEnv } from "./env.js";

/** An imported Ed25519 signing key paired with its published key id. */
export interface SigningKey {
  key: CryptoKey;
  kid: string;
}

/** A published JWKS document containing only public key material. */
export interface PublicJwks {
  keys: JWK[];
}

let cachedSigningKey: SigningKey | undefined;
let cachedPublicJwks: PublicJwks | undefined;

/**
 * Loads and caches the Ed25519 signing key from AS_SIGNING_KEY_PKCS8 /
 * AS_SIGNING_KID. The PEM is imported once per process.
 */
export async function getSigningKey(): Promise<SigningKey> {
  if (cachedSigningKey) {
    return cachedSigningKey;
  }

  // AS_SIGNING_KEY_PKCS8 may arrive as a flat env-bundle value with literal `\n` escapes
  // (temper admin saml provision's render_env) or with real newlines (a Vercel field
  // paste) — normalize to real newlines so jose's PEM parser accepts either form.
  const pem = requireEnv("AS_SIGNING_KEY_PKCS8").replace(/\\n/g, "\n");
  const kid = requireEnv("AS_SIGNING_KID");
  const key = await importPKCS8(pem, "EdDSA");

  cachedSigningKey = { key, kid };
  return cachedSigningKey;
}

/**
 * Derives the public JWKS for the signing key: the public key exported as a
 * JWK, annotated with alg/use/kid, and containing no private material.
 */
export async function getPublicJwks(): Promise<PublicJwks> {
  if (cachedPublicJwks) {
    return cachedPublicJwks;
  }

  // Same normalization as getSigningKey — tolerate escaped or real newlines.
  const pem = requireEnv("AS_SIGNING_KEY_PKCS8").replace(/\\n/g, "\n");
  const kid = requireEnv("AS_SIGNING_KID");

  const publicKey = createPublicKey(pem);
  const jwk = await exportJWK(publicKey);

  cachedPublicJwks = {
    keys: [{ ...jwk, alg: "EdDSA", use: "sig", kid }],
  };
  return cachedPublicJwks;
}
