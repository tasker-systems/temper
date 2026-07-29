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
 * stays here is what is genuinely deployment-specific: the per-principal env names, and the Vercel
 * Connect / static token strategies — eve and Vercel concepts with no business in a general-purpose
 * client.
 *
 * Ordering is **machine-identity-first**, identical to what the connection declares:
 *   1. `TEMPER_M2M_CLIENT_ID` present → mint the agent's own token via the OAuth `client_credentials`
 *      grant. This is the production path, and it works against BOTH issuers a temper instance can be
 *      fronted by: an external IdP (`temper admin machine provision`, audience required) and temper's
 *      own AS (`temper admin machine issue`, a `tmpr_` client id, audience omitted).
 *   2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token (instances where that works).
 *   3. else `TEMPER_TOKEN` (the already-OAuth-obtained token that drives `eve dev`).
 *
 * Since Set 5 this file serves **two principals**, not one. The strategy ordering above is shared;
 * the env NAMES it reads are a parameter ([`CredentialEnv`]), because the citation auditor must
 * authenticate as its own registered machine client. See [`AUDITOR_CREDENTIALS`] for why that is a
 * hard requirement rather than hygiene, and why the auditor has no Connect branch.
 */

/**
 * Which env names one PRINCIPAL's credential is read from.
 *
 * Set 5 made this a parameter rather than a set of hardcoded names, because the citation auditor
 * must authenticate as a **second, separate machine principal** — never the steward's. One
 * credential is one `emitter_entity_id`, so a shared client would leave the ledger unable to tell an
 * audit from the citation it audits, collapsing "assessed by another party" into "asserted by the
 * same party wearing a different label" (spec §5.2; `docs/auth/machine-token-contract.md` §C).
 *
 * `connector` is optional and is the reason this is a shape rather than a prefix string: the
 * auditor has NO Vercel Connect branch. The Connect connector is per-deployment, not per-principal,
 * so an auditor falling back to it would authenticate as the deployment — i.e. as the steward — which
 * is exactly the collapse above, arriving silently through a fallback nobody chose.
 */
export interface CredentialEnv {
  clientId: string;
  clientSecret: string;
  tokenUrl: string;
  audience: string;
  /** Omitted for principals that must never fall back to a deployment-scoped identity. */
  connector?: string;
  staticToken: string;
}

/** The steward's credential env — the names that existed before Set 5, unchanged. */
export const STEWARD_CREDENTIALS: CredentialEnv = {
  clientId: "TEMPER_M2M_CLIENT_ID",
  clientSecret: "TEMPER_M2M_CLIENT_SECRET",
  tokenUrl: "TEMPER_M2M_TOKEN_URL",
  audience: "TEMPER_M2M_AUDIENCE",
  connector: "TEMPER_CONNECT_CONNECTOR",
  staticToken: "TEMPER_TOKEN",
};

/**
 * The citation auditor's credential env — a wholly disjoint name set, so there is no value of the
 * steward's env that can ever authenticate the auditor and no way to "forget" to configure it: an
 * unset `TEMPER_AUDITOR_TOKEN` with no `TEMPER_AUDITOR_M2M_CLIENT_ID` throws rather than quietly
 * borrowing the steward's.
 */
export const AUDITOR_CREDENTIALS: CredentialEnv = {
  clientId: "TEMPER_AUDITOR_M2M_CLIENT_ID",
  clientSecret: "TEMPER_AUDITOR_M2M_CLIENT_SECRET",
  tokenUrl: "TEMPER_AUDITOR_M2M_TOKEN_URL",
  audience: "TEMPER_AUDITOR_M2M_AUDIENCE",
  staticToken: "TEMPER_AUDITOR_TOKEN",
};

/**
 * Is a credential for this principal configured on this deployment AT ALL?
 *
 * **"Not configured" and "configured but broken" are different states and must stay
 * distinguishable.** [`build`] throws on both — correctly, because the alternative is borrowing
 * another principal's identity — but a caller that can meaningfully *skip* needs to tell them
 * apart. This answers only the first question; it never relaxes the second.
 *
 * Deliberately derived from the same [`CredentialEnv`] `build` reads, and in the same order, so the
 * two cannot drift on what "configured" means. A partially-configured principal (client id present,
 * secret absent) reads as **configured** here and then throws in `build` — which is the point: that
 * is a misconfiguration, not an absence, and it must be loud.
 *
 * Empty-string env vars count as absent, matching [`requireEnv`] and `build`'s own truthiness
 * checks — Vercel surfaces a declared-but-empty variable as `""`, not `undefined`.
 */
export function credentialConfigured(names: CredentialEnv = STEWARD_CREDENTIALS): boolean {
  const connector = names.connector ? process.env[names.connector] : undefined;
  return Boolean(process.env[names.clientId] || connector || process.env[names.staticToken]);
}

/** One cached credential per principal, keyed by its client-id env name. */
const cache = new Map<string, Credentials>();

/**
 * `<PREFIX>_AUDIENCE` is read but NOT required. Auth0 demands an audience; temper's own AS ignores
 * a request-supplied one entirely and mints with its server-side `AS_AUDIENCE`. So a temper-issued
 * (`tmpr_`) credential must be able to omit it — requiring it here is precisely what made this agent
 * unable to consume one.
 */
function credentials(names: CredentialEnv = STEWARD_CREDENTIALS): Credentials {
  const hit = cache.get(names.clientId);
  if (hit !== undefined) {
    return hit;
  }

  const built = build(names);
  cache.set(names.clientId, built);
  return built;
}

function build(names: CredentialEnv): Credentials {
  const clientId = process.env[names.clientId];
  if (clientId) {
    return new ClientCredentials({
      tokenUrl: requireEnv(names.tokenUrl),
      clientId,
      clientSecret: requireEnv(names.clientSecret),
      audience: process.env[names.audience] || undefined,
    });
  }

  const connector = names.connector ? process.env[names.connector] : undefined;
  if (connector) {
    return {
      // Connect can hand out a fresh app token on demand, so a 401 IS worth one retry here.
      canRefresh: true,
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
  }

  return new BearerToken(requireEnv(names.staticToken));
}

/**
 * The token + its ABSOLUTE expiry, handed straight to eve's `auth.getToken` by the MCP connection so
 * eve can refresh ahead of a 401. Name and shape are load-bearing — `connections/temper.ts` passes
 * this function itself as `getToken`.
 */
export async function mintM2mToken(): Promise<TokenResult> {
  return credentials().tokenResult();
}

/**
 * The auditor's twin of [`mintM2mToken`], passed as `getToken` by the auditor subagent's own MCP
 * connection (`agent/subagents/auditor/connections/temper.ts`). Separate function rather than an
 * argument, for the same reason the connection is separate: eve's connection config takes a
 * zero-arg `getToken`, and a shared one would have to pick a principal from ambient state.
 */
export async function mintAuditorM2mToken(): Promise<TokenResult> {
  return credentials(AUDITOR_CREDENTIALS).tokenResult();
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
 *
 * A strategy that CANNOT mint (`TEMPER_TOKEN` under `eve dev`) gets its 401 back untouched. Calling
 * `refresh()` on it would throw "BearerToken cannot refresh" — replacing temper's real answer, body
 * and all, with a message about the client's own plumbing, on the one path a human actually reads.
 */
export async function temperFetch(
  url: string,
  init: RequestInit,
  opts: RetryOptions = {},
): Promise<Response> {
  return fetchAs(STEWARD_CREDENTIALS, url, init, opts);
}

/**
 * [`temperFetch`] as the CITATION AUDITOR — same retry and re-mint behavior, a different principal.
 *
 * The auditor's dispatch tick must run under the auditor's own credential, not the steward's,
 * because `audit_drift_sweep` is principal-scoped: the queue it fills is exactly what that principal
 * can read, and it is the same principal the audit write gate will later evaluate. Sweeping as one
 * identity and writing as another would hand the session work it is about to be refused.
 */
export async function auditorFetch(
  url: string,
  init: RequestInit,
  opts: RetryOptions = {},
): Promise<Response> {
  return fetchAs(AUDITOR_CREDENTIALS, url, init, opts);
}

async function fetchAs(
  names: CredentialEnv,
  url: string,
  init: RequestInit,
  opts: RetryOptions,
): Promise<Response> {
  const creds = credentials(names);
  const headers = new Headers(init.headers);

  headers.set("authorization", `Bearer ${await creds.token()}`);
  const res = await fetchWithRetry(url, { ...init, headers }, opts);
  if (res.status !== 401 || !creds.canRefresh) {
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
    throw new Error(`${name} is required — this agent's targets/credentials are never hardcoded`);
  }
  return value;
}
