import type { AuthClaims } from "./auth.js";
import { getIssuer, getJwksVerifier, verifyToken } from "./auth.js";
import { getDb, type NeonClient } from "./db.js";

/**
 * Look up the profile_id from auth claims.
 * Joins through kb_profile_auth_links using claims.sub as the external identity.
 * Returns null if no matching profile is found.
 */
export async function getProfileId(db: NeonClient, claims: AuthClaims): Promise<string | null> {
  const rows = await db`
    SELECT p.id
    FROM kb_profiles p
    JOIN kb_profile_auth_links pal ON pal.profile_id = p.id
    WHERE pal.auth_provider_user_id = ${claims.sub}
      AND p.is_active = true
    LIMIT 1
  `;
  if (rows.length === 0) return null;
  return rows[0].id as string;
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface AuthSuccess {
  ok: true;
  db: NeonClient;
  profileId: string;
  claims: AuthClaims;
}

export interface AuthFailure {
  ok: false;
  response: Response;
}

export type AuthResult = AuthSuccess | AuthFailure;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

// ---------------------------------------------------------------------------
// Shared middleware
// ---------------------------------------------------------------------------

/**
 * Validate that the request method matches. Returns an error Response or null.
 */
export function requireMethod(req: Request, method: string): Response | null {
  if (req.method !== method) {
    return jsonResponse(405, { error: "Method not allowed" });
  }
  return null;
}

/**
 * Authenticate an incoming request: extract bearer token, verify JWT,
 * resolve profile ID from auth claims.
 *
 * Returns `{ ok: true, db, profileId, claims }` on success, or
 * `{ ok: false, response }` with a ready-to-return error Response on failure.
 */
export async function authenticateRequest(req: Request): Promise<AuthResult> {
  const authHeader = req.headers.get("authorization");
  if (!authHeader?.startsWith("Bearer ")) {
    return {
      ok: false,
      response: jsonResponse(401, {
        error: { code: "UNAUTHORIZED", message: "Missing Authorization header" },
      }),
    };
  }

  let claims: AuthClaims;
  try {
    claims = await verifyToken(authHeader.slice(7), getJwksVerifier(), getIssuer());
  } catch {
    return {
      ok: false,
      response: jsonResponse(401, {
        error: { code: "UNAUTHORIZED", message: "Invalid token" },
      }),
    };
  }

  const db = getDb();
  const profileId = await getProfileId(db, claims);
  if (!profileId) {
    return {
      ok: false,
      response: jsonResponse(404, { error: "Profile not found" }),
    };
  }

  return { ok: true, db, profileId, claims };
}
