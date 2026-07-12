import { getToken } from "@vercel/connect";

/**
 * Machine-identity auth for reaching temper, shared by the MCP connection AND the code schedules so
 * the two can never drift on how they authenticate (they did: the schedules used a Connect-first
 * `temperToken()` while the connection used M2M-first `mintM2mToken`, and on the Auth0-fronted prod
 * instance the Connect connector has no M2M app behind it — so the schedules' REST fetches silently
 * failed while the MCP connection worked).
 *
 * Ordering is **machine-identity-first**, identical to what the connection declares:
 *   1. `TEMPER_M2M_CLIENT_ID` present → mint the agent's own token via the OAuth `client_credentials`
 *      grant against Auth0 (`mintM2mToken`). This is the production path.
 *   2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token (instances where that works).
 *   3. else `TEMPER_TOKEN` (the already-OAuth-obtained token that drives `eve dev`).
 */

let cachedM2m: { token: string; expiresAt: number } | undefined;

/**
 * Mint the agent's own token via the `client_credentials` grant. Cached across calls until ~60s
 * before expiry. Returns `{ token, expiresAt }` where `expiresAt` is absolute ms-since-epoch,
 * matching eve's `TokenResult` (so the connection can hand this straight to `auth.getToken`).
 *
 * The body is form-urlencoded, which RFC 6749 §4 mandates for the token endpoint. Auth0 also
 * accepts JSON, which is why this sent JSON for as long as Auth0 was the only issuer it faced —
 * and why nothing was red. Temper's own AS (`packages/temper-cloud/src/oauth/endpoints.ts`) reads
 * the body with `req.formData()`, so a JSON mint could never have reached its grant branch.
 * Form-encoding works against BOTH issuers; this is the shape that lets the steward be repointed
 * at temper's issuer without touching this file again.
 *
 * `audience` is Auth0's: temper's AS ignores a request-supplied audience and mints with its own
 * `AS_AUDIENCE`. Still sent unconditionally — it is required by Auth0 and inert everywhere else.
 */
export async function mintM2mToken(): Promise<{ token: string; expiresAt: number }> {
  const skewMs = 60_000;
  if (cachedM2m && cachedM2m.expiresAt - skewMs > Date.now()) {
    return cachedM2m;
  }

  const res = await fetch(requireEnv("TEMPER_M2M_TOKEN_URL"), {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "client_credentials",
      client_id: requireEnv("TEMPER_M2M_CLIENT_ID"),
      client_secret: requireEnv("TEMPER_M2M_CLIENT_SECRET"),
      audience: requireEnv("TEMPER_M2M_AUDIENCE"),
    }),
  });
  if (!res.ok) {
    throw new Error(`M2M token mint failed (${res.status}): ${await res.text()}`);
  }

  const body = (await res.json()) as { access_token: string; expires_in: number };
  cachedM2m = {
    token: body.access_token,
    expiresAt: Date.now() + body.expires_in * 1000,
  };
  return cachedM2m;
}

/**
 * A bearer token string for imperative temper REST/MCP `fetch`es from the code schedules — the same
 * machine-identity-first ordering the connection uses. M2M-first is the fix: the previous
 * Connect-first ordering hit the dead-end connector on prod and threw before any request went out.
 */
export async function temperToken(): Promise<string> {
  if (process.env.TEMPER_M2M_CLIENT_ID) {
    return (await mintM2mToken()).token;
  }
  const connector = process.env.TEMPER_CONNECT_CONNECTOR;
  if (connector) {
    return getToken(connector, { subject: { type: "app" } });
  }
  return requireEnv("TEMPER_TOKEN");
}

export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the steward's target/credential is never hardcoded`);
  }
  return value;
}
