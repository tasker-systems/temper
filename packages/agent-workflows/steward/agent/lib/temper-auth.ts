import { getToken } from "@vercel/connect";
import { BearerToken, ClientCredentials, type Credentials, type TokenResult } from "temper-ts";

import { fetchWithRetry, type RetryOptions } from "./fetch-retry.js";

/**
 * Machine-identity auth for reaching temper, shared by the MCP connection AND the code schedules so
 * the two can never drift on how they authenticate (they did: the schedules used a Connect-first
 * `temperToken()` while the connection used M2M-first `mintM2mToken`, and on the Auth0-fronted prod
 * instance the Connect connector has no M2M app behind it — so the schedules' REST fetches silently
 * failed while the MCP connection worked).
 *
 * The MINT itself lives in `temper-ts` (`ClientCredentials`), shared with the Ruby gem's
 * `Temper::Credentials` by way of one wire contract (tests/contracts/m2m-token-request.json). What
 * stays here is what is genuinely steward-specific: the env names, and the Vercel Connect / static
 * token strategies — eve and Vercel concepts with no business in a general-purpose client.
 *
 * Ordering is **machine-identity-first**, identical to what the connection declares:
 *   1. `TEMPER_M2M_CLIENT_ID` present → mint the agent's own token via the OAuth `client_credentials`
 *      grant. This is the production path, and it works against BOTH issuers a temper instance can be
 *      fronted by: an external IdP (`temper admin machine provision`, audience required) and temper's
 *      own AS (`temper admin machine issue`, a `tmpr_` client id, audience omitted).
 *   2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token (instances where that works).
 *   3. else `TEMPER_TOKEN` (the already-OAuth-obtained token that drives `eve dev`).
 */

let cached: Credentials | undefined;

/**
 * `TEMPER_M2M_AUDIENCE` is read but NOT required. Auth0 demands an audience; temper's own AS ignores
 * a request-supplied one entirely and mints with its server-side `AS_AUDIENCE`. So a temper-issued
 * (`tmpr_`) credential must be able to omit it — requiring it here is precisely what made this agent
 * unable to consume one.
 */
function credentials(): Credentials {
  if (cached !== undefined) {
    return cached;
  }

  const clientId = process.env.TEMPER_M2M_CLIENT_ID;
  if (clientId) {
    cached = new ClientCredentials({
      tokenUrl: requireEnv("TEMPER_M2M_TOKEN_URL"),
      clientId,
      clientSecret: requireEnv("TEMPER_M2M_CLIENT_SECRET"),
      audience: process.env.TEMPER_M2M_AUDIENCE || undefined,
    });
    return cached;
  }

  const connector = process.env.TEMPER_CONNECT_CONNECTOR;
  if (connector) {
    cached = {
      token: () => getToken(connector, { subject: { type: "app" } }),
      tokenResult: async () => ({
        token: await getToken(connector, { subject: { type: "app" } }),
        expiresAt: Number.POSITIVE_INFINITY,
      }),
      refresh: async () => ({
        token: await getToken(connector, { subject: { type: "app" } }),
        expiresAt: Number.POSITIVE_INFINITY,
      }),
    };
    return cached;
  }

  cached = new BearerToken(requireEnv("TEMPER_TOKEN"));
  return cached;
}

/**
 * The token + its ABSOLUTE expiry, handed straight to eve's `auth.getToken` by the MCP connection so
 * eve can refresh ahead of a 401. Name and shape are load-bearing — `connections/temper.ts` passes
 * this function itself as `getToken`.
 */
export async function mintM2mToken(): Promise<TokenResult> {
  return credentials().tokenResult();
}

/** A bearer token string for imperative temper REST/MCP `fetch`es from the code schedules. */
export async function temperToken(): Promise<string> {
  return credentials().token();
}

/**
 * `fetch` against temper, authenticated, with the 5xx cold-start retry AND a single re-mint on 401.
 *
 * The 401 branch is the fix for a bug the Ruby port documented against this very file: a schedule
 * resolves ONE token and then holds it across N parallel fetches, so a token that dies mid-tick
 * takes the tick down with it and nothing recovers. Refresh-ahead-of-expiry cannot help — the token
 * was live when it was checked. Temper's AS mints 900-second tokens by default, which makes a tick
 * outliving its token ordinary rather than exotic.
 *
 * Exactly ONE re-mint: a 401 that survives a fresh token is a real authorization failure (a revoked
 * credential, missing reach), and retrying it forever would only bury the error.
 */
export async function temperFetch(
  url: string,
  init: RequestInit,
  opts: RetryOptions = {},
): Promise<Response> {
  const creds = credentials();
  const headers = new Headers(init.headers);

  headers.set("authorization", `Bearer ${await creds.token()}`);
  const res = await fetchWithRetry(url, { ...init, headers }, opts);
  if (res.status !== 401) {
    return res;
  }

  const refreshed = await creds.refresh();
  console.log(`[temper-auth] ${opts.label ?? url} returned 401; re-minted and retrying once`);
  headers.set("authorization", `Bearer ${refreshed.token}`);
  return fetchWithRetry(url, { ...init, headers }, opts);
}

export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the steward's target/credential is never hardcoded`);
  }
  return value;
}
