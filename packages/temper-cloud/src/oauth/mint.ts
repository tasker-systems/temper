import { createHash, randomBytes } from "node:crypto";
import { SignJWT } from "jose";
import { requireEnv } from "./env.js";
import { getSigningKey } from "./keys.js";

const DEFAULT_ACCESS_TTL_SECONDS = 900;

/** The claims minted into an access token for a given authenticated user. */
export interface MintedClaims {
  sub: string;
  email: string;
  email_verified: boolean;
}

/**
 * Validated access-token TTL, read from AS_ACCESS_TTL_SECONDS. Exported so callers advertising
 * `expires_in` (e.g. the /oauth/token response) use the exact same TTL the token was minted with,
 * rather than re-deriving it (and potentially disagreeing, e.g. producing `expires_in: NaN` when
 * the env var is unset).
 */
export function accessTtlSeconds(): number {
  const raw = process.env.AS_ACCESS_TTL_SECONDS;
  if (!raw) {
    return DEFAULT_ACCESS_TTL_SECONDS;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_ACCESS_TTL_SECONDS;
}

/**
 * Mints a signed EdDSA access token for the given claims, using the process
 * signing key and AS_ISSUER / AS_AUDIENCE / AS_ACCESS_TTL_SECONDS env config.
 */
export async function mintAccessToken(claims: MintedClaims): Promise<string> {
  const { key, kid } = await getSigningKey();
  const issuer = requireEnv("AS_ISSUER");
  const audience = requireEnv("AS_AUDIENCE");
  const nowSeconds = Math.floor(Date.now() / 1000);
  const expSeconds = nowSeconds + accessTtlSeconds();

  return await new SignJWT({
    email: claims.email,
    email_verified: claims.email_verified,
  })
    .setProtectedHeader({ alg: "EdDSA", kid })
    .setSubject(claims.sub)
    .setIssuer(issuer)
    .setAudience(audience)
    .setIssuedAt(nowSeconds)
    .setExpirationTime(expSeconds)
    .sign(key);
}

/** Generates a fresh 32-byte opaque token (base64url-encoded). */
export function newOpaqueToken(): string {
  return randomBytes(32).toString("base64url");
}

/** Computes the sha256 hex digest of an opaque token, for storage/lookup. */
export function hashToken(t: string): string {
  return createHash("sha256").update(t).digest("hex");
}
