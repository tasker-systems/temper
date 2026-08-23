/**
 * `GET /oauth/authorize` (Vercel entry point).
 *
 * Dispatches to the Temper AS (SAML instances with `AS_ISSUER`) or the Auth0
 * loopback redirect-uri proxy (Auth0-fronted instances without `AS_ISSUER`).
 * The proxy rewrites loopback `redirect_uri`s to a relay URL so Auth0's
 * exact-match callback allowlist doesn't reject MCP CLI clients.
 */

export async function GET(req: Request): Promise<Response> {
  // Dynamic import to avoid ESM/CJS conflict in Vercel's hybrid runtime: this
  // entry point is loaded via CommonJS `require()` (no `"type": "module"` at the
  // repo root), but the target lives under temper-cloud, which is `"type":
  // "module"`. A static value import would compile to `require()` of an ESM file
  // (ERR_REQUIRE_ESM). See api/upload.ts for the same pattern.
  if (process.env.AS_ISSUER) {
    const { handleAuthorize } = await import("../../packages/temper-cloud/src/oauth/endpoints.js");
    const { getDb } = await import("../../packages/temper-cloud/src/db.js");
    return handleAuthorize(req, getDb());
  }
  const { proxyAuthorize } = await import("../../packages/temper-cloud/src/oauth/auth0-proxy.js");
  return proxyAuthorize(req);
}
