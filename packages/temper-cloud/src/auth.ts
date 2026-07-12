import * as jose from "jose";

export interface AuthClaims {
  sub: string;
  email: string;
  email_verified: boolean;
}

/**
 * Verify a bearer token: signature, issuer, AND audience.
 *
 * The audience check is not optional and `aud` must be PRESENT. This verifier previously passed no
 * `audience` option at all, so it accepted any correctly-signed token from the issuer regardless of
 * which API it was minted for — the same fall-open the Rust surfaces just closed, living on in a
 * third verifier. It has no live route today, which is exactly why it went unnoticed; an unwired gun
 * pointed at the same database is still a gun.
 *
 * `requiredClaims: ["aud"]` is load-bearing on its own: like jsonwebtoken, jose only *compares* the
 * audience when the claim exists, so a token omitting `aud` would otherwise pass the comparison by
 * skipping it.
 */
export async function verifyToken(
  token: string,
  key: jose.CryptoKey | jose.KeyObject | jose.JWK | Uint8Array | jose.JWTVerifyGetKey,
  issuer: string,
  audience: string = getAudience(),
): Promise<AuthClaims> {
  const opts: jose.JWTVerifyOptions = {
    issuer,
    audience,
    requiredClaims: ["aud", "iss", "exp"],
    algorithms: ["RS256", "EdDSA"],
  };
  // jose v6 has separate overloads for key vs getKey — narrow to match
  const { payload } =
    typeof key === "function"
      ? await jose.jwtVerify(token, key as jose.JWTVerifyGetKey, opts)
      : await jose.jwtVerify(
          token,
          key as jose.CryptoKey | jose.KeyObject | jose.JWK | Uint8Array,
          opts,
        );

  const sub = payload.sub;
  let email = payload.email as string | undefined;
  let emailVerified = payload.email_verified as boolean | undefined;

  if (!sub) {
    throw new Error("JWT missing sub claim");
  }

  // Auth0 access tokens don't include email by default. Fall back to /userinfo.
  if (!email) {
    const userinfo = await fetchUserinfo(token, issuer);
    email = userinfo.email;
    emailVerified = userinfo.email_verified;
  }

  if (!email) {
    throw new Error("JWT missing email claim and userinfo lookup failed");
  }

  return {
    sub,
    email,
    email_verified: emailVerified ?? false,
  };
}

async function fetchUserinfo(
  accessToken: string,
  issuer: string,
): Promise<{ email?: string; email_verified?: boolean }> {
  const url = `${issuer.replace(/\/$/, "")}/userinfo`;
  const resp = await fetch(url, {
    headers: { Authorization: `Bearer ${accessToken}` },
  });
  if (!resp.ok) {
    throw new Error(`userinfo returned status ${resp.status}`);
  }
  return (await resp.json()) as { email?: string; email_verified?: boolean };
}

let cachedJwks: jose.JWTVerifyGetKey | null = null;

export function getJwksVerifier(): jose.JWTVerifyGetKey {
  if (cachedJwks) return cachedJwks;

  const jwksUrl = process.env.JWKS_URL;
  if (!jwksUrl) {
    throw new Error("JWKS_URL environment variable is required");
  }

  cachedJwks = jose.createRemoteJWKSet(new URL(jwksUrl));
  return cachedJwks;
}

/**
 * The one audience this instance validates — the same value the Rust boot gate
 * (`temper-services::auth_config`) refuses to start without. There is exactly one per instance.
 */
export function getAudience(): string {
  const audience = process.env.AUTH_AUDIENCE;
  if (!audience) {
    throw new Error("AUTH_AUDIENCE environment variable is required");
  }
  return audience;
}

export function getIssuer(): string {
  const issuer = process.env.AUTH_ISSUER;
  if (!issuer) {
    throw new Error("AUTH_ISSUER environment variable is required");
  }
  return issuer;
}
