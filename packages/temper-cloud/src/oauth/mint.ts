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
 *
 * `audience` is the RFC 8707 resource indicator the authorization flow was granted — what the
 * caller requested and the flow validated. When omitted (the refresh grant, whose chain does not
 * carry the original request) the token is minted with the instance audience, AS_AUDIENCE; the
 * MCP middleware accepts both audiences, so a refreshed MCP session survives the omission.
 */
export async function mintAccessToken(claims: MintedClaims, audience?: string): Promise<string> {
  const { key, kid } = await getSigningKey();
  const issuer = requireEnv("AS_ISSUER");
  const resolvedAudience = audience?.trim() || requireEnv("AS_AUDIENCE");
  const nowSeconds = Math.floor(Date.now() / 1000);
  const expSeconds = nowSeconds + accessTtlSeconds();

  return await new SignJWT({
    email: claims.email,
    email_verified: claims.email_verified,
  })
    .setProtectedHeader({ alg: "EdDSA", kid })
    .setSubject(claims.sub)
    .setIssuer(issuer)
    .setAudience(resolvedAudience)
    .setIssuedAt(nowSeconds)
    .setExpirationTime(expSeconds)
    .sign(key);
}

/**
 * Mints an EdDSA access token for a temper-issued machine principal. The claim shape mirrors an
 * Auth0 client_credentials token exactly — `gty:"client-credentials"`, `azp:<client_id>`,
 * `sub:"<client_id>@clients"`, no email — so `normalize_machine` (Rust) detects it unchanged.
 */
export async function mintMachineAccessToken(clientId: string): Promise<string> {
  const { key, kid } = await getSigningKey();
  const issuer = requireEnv("AS_ISSUER");
  const audience = requireEnv("AS_AUDIENCE");
  const nowSeconds = Math.floor(Date.now() / 1000);
  const expSeconds = nowSeconds + accessTtlSeconds();

  return await new SignJWT({
    azp: clientId,
    gty: "client-credentials",
  })
    .setProtectedHeader({ alg: "EdDSA", kid })
    .setSubject(`${clientId}@clients`)
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

/**
 * Computes the sha256 hex digest of an opaque token, for storage/lookup.
 *
 * ## Why a fast hash is the right one here, and what would make it the wrong one
 *
 * A slow KDF — bcrypt, scrypt, argon2id — exists to make *guessing* expensive. It buys security
 * only when the input is drawn from a small space: a human-chosen password, a PIN, anything an
 * attacker holding the digest could enumerate offline. Its cost is entirely wasted against an
 * input with no exploitable structure to guess at.
 *
 * Every value this function is given is one of exactly two things:
 *
 *   1. **A token this server minted** — `newOpaqueToken()`, 32 bytes from the platform CSPRNG, or
 *      the Rust twin `temper_services::auth::secret::mint_secret` (32 bytes from `OsRng`), whose
 *      `sha256_hex` is byte-identical to this so a hash written there verifies here. Machine client
 *      secrets are server-minted on BOTH write paths (`machine_registration_service::issue` and
 *      `machine_client_service`'s rotate); no request type carries a caller-chosen secret.
 *   2. **A candidate being checked against one** — a presented refresh token, authorization code,
 *      or client secret, hashed so it can be compared with a stored digest.
 *
 * Against 256 bits of uniform randomness there is no dictionary, no structure, and no feasible
 * enumeration — so no cost factor changes the outcome, and a per-row salt prevents no
 * precomputation that was possible in the first place. This is the same reasoning behind hashing
 * OAuth refresh tokens at rest rather than running them through a KDF (RFC 6819 §5.1.4.1.3).
 *
 * **The invariant is the load-bearing part, not the algorithm.** This is safe because of what its
 * callers pass, and a `string` parameter cannot say so. Route a human-chosen or otherwise
 * low-entropy secret through here and the reasoning above collapses — that case needs argon2id and
 * a per-row salt, and it needs them at the site that introduces it.
 *
 * Flagged by CodeQL `js/insufficient-password-hash`, which classifies these credentials as
 * passwords by name. Judged not applicable for the reason above; making the invariant checkable
 * rather than asserted is tracked separately.
 */
export function hashToken(t: string): string {
  return createHash("sha256").update(t).digest("hex");
}
