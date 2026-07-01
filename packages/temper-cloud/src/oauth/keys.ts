import { createPublicKey } from "node:crypto";
import type { JWK } from "jose";
import { exportJWK, importPKCS8 } from "jose";

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

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

/**
 * Loads and caches the Ed25519 signing key from AS_SIGNING_KEY_PKCS8 /
 * AS_SIGNING_KID. The PEM is imported once per process.
 */
export async function getSigningKey(): Promise<SigningKey> {
  if (cachedSigningKey) {
    return cachedSigningKey;
  }

  const pem = requireEnv("AS_SIGNING_KEY_PKCS8");
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

  const pem = requireEnv("AS_SIGNING_KEY_PKCS8");
  const kid = requireEnv("AS_SIGNING_KID");

  const publicKey = createPublicKey(pem);
  const jwk = await exportJWK(publicKey);

  cachedPublicJwks = {
    keys: [{ ...jwk, alg: "EdDSA", use: "sig", kid }],
  };
  return cachedPublicJwks;
}
