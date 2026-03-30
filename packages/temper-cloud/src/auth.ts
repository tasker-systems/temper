import * as jose from "jose";

export interface AuthClaims {
  sub: string;
  email: string;
  email_verified: boolean;
}

export async function verifyToken(
  token: string,
  key: jose.CryptoKey | jose.KeyObject | jose.JWK | Uint8Array | jose.JWTVerifyGetKey,
  issuer: string,
): Promise<AuthClaims> {
  const opts: jose.JWTVerifyOptions = { issuer, algorithms: ["RS256", "EdDSA"] };
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
  const email = payload.email as string | undefined;
  const emailVerified = payload.email_verified as boolean | undefined;

  if (!sub) {
    throw new Error("JWT missing sub claim");
  }
  if (!email) {
    throw new Error("JWT missing email claim");
  }

  return {
    sub,
    email,
    email_verified: emailVerified ?? false,
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

export function getIssuer(): string {
  const issuer = process.env.AUTH_ISSUER;
  if (!issuer) {
    throw new Error("AUTH_ISSUER environment variable is required");
  }
  return issuer;
}
