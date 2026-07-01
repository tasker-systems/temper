/**
 * `POST /oauth/saml/acs` (Vercel entry point) — the SAML Assertion Consumer Service endpoint.
 * Thin wrapper — handler logic and tests live in `packages/temper-cloud/src/oauth/endpoints.ts`.
 */

export async function POST(req: Request): Promise<Response> {
  // Dynamic import to avoid ESM/CJS conflict in Vercel's hybrid runtime: this
  // entry point is loaded via CommonJS `require()` (no `"type": "module"` at the
  // repo root), but the target lives under temper-cloud, which is `"type":
  // "module"`. A static value import would compile to `require()` of an ESM file
  // (ERR_REQUIRE_ESM). See api/upload.ts for the same pattern.
  const { handleSamlAcs } = await import("../../../packages/temper-cloud/src/oauth/endpoints.js");
  const { getDb } = await import("../../../packages/temper-cloud/src/db.js");
  return handleSamlAcs(req, getDb());
}
