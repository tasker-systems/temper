import type { AuthClaims } from "./auth.js";
import { getIssuer, getJwksVerifier, verifyToken } from "./auth.js";
import { getDb, type NeonClient } from "./db.js";
import { getProfileId } from "./ingest.js";

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
