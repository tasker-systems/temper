/**
 * `POST /oauth/clients` (Vercel entry point) — AS-mode RFC 7591 dynamic client registration.
 *
 * Served only on SAML/AS instances (`AS_ISSUER` set): their RFC 8414 document advertises this
 * path as `registration_endpoint`. Auth0-fronted instances never advertise it, so the entry
 * 404s there instead of serving a second, unread door beside `/oauth/register`.
 */

export async function POST(req: Request): Promise<Response> {
  // Dynamic import to avoid ESM/CJS conflict in Vercel's hybrid runtime — see api/oauth/token.ts.
  const { isTemperAsMode } = await import("../../packages/temper-cloud/src/oauth/env.js");
  if (!isTemperAsMode()) {
    return new Response("Not Found", { status: 404 });
  }
  const { handleClientRegistration } = await import(
    "../../packages/temper-cloud/src/oauth/register.js"
  );
  const { getDb } = await import("../../packages/temper-cloud/src/db.js");
  return handleClientRegistration(req, getDb());
}
