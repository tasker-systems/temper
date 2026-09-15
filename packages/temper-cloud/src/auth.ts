import * as jose from "jose";

export interface AuthClaims {
  sub: string;
  email: string;
  /**
   * Three states, not two: `true` (the IdP verified the email), `false` (the IdP says it is not
   * verified), and `null` (the token and userinfo said nothing about verification). Absence is
   * carried, not collapsed: this mirrors the wire types (`ReconcileRequest` /
   * `ResolvePrincipalRequest` take `boolean | null`) and the Rust `AuthClaims` (`Option<bool>`),
   * whose verification gates test `== Some(true)` — a claim the IdP never asserted must not
   * arrive downstream pre-answered as false.
   */
  email_verified: boolean | null;
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
    // Absence (`undefined` from both the token and userinfo) is carried as `null` — "the IdP said
    // nothing" — rather than asserted as `false`, which would fabricate a verification verdict the
    // IdP never gave.
    email_verified: emailVerified ?? null,
  };
}

/**
 * Fetches the OIDC `/userinfo` document for the token. The body arrives at runtime from an
 * external IdP, so it is validated rather than asserted into shape: a body that is not a usable
 * userinfo document throws, landing on the same failure path as any other userinfo failure.
 */
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
  const parsed: unknown = await resp.json();
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("userinfo returned a body that is not a JSON object");
  }
  const { email, email_verified } = parsed as {
    email?: unknown;
    email_verified?: unknown;
  };
  if (email !== undefined && typeof email !== "string") {
    throw new Error("userinfo returned a non-string email");
  }
  if (email_verified !== undefined && typeof email_verified !== "boolean") {
    throw new Error("userinfo returned a non-boolean email_verified");
  }
  return {
    email: typeof email === "string" ? email : undefined,
    email_verified: typeof email_verified === "boolean" ? email_verified : undefined,
  };
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
