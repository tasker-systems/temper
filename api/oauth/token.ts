/**
 * `POST /oauth/token` (Vercel entry point).
 *
 * Dispatches to the Temper AS (SAML instances with `AS_ISSUER`) or the Auth0
 * proxy (Auth0-fronted instances). The proxy rewrites `redirect_uri` in
 * `authorization_code` grants to match the relay URL used during `/authorize`.
 */

export async function POST(req: Request): Promise<Response> {
  // Dynamic import to avoid ESM/CJS conflict in Vercel's hybrid runtime: this
  // entry point is loaded via CommonJS `require()` (no `"type": "module"` at the
  // repo root), but the target lives under temper-cloud, which is `"type":
  // "module"`. A static value import would compile to `require()` of an ESM file
  // (ERR_REQUIRE_ESM). See api/upload.ts for the same pattern.
  if (process.env.AS_ISSUER) {
    const { handleToken } = await import("../../packages/temper-cloud/src/oauth/endpoints.js");
    const { getDb } = await import("../../packages/temper-cloud/src/db.js");
    return handleToken(req, getDb());
  }
  const { proxyToken } = await import("../../packages/temper-cloud/src/oauth/auth0-proxy.js");
  return proxyToken(req);
}
