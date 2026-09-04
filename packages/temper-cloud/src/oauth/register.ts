/**
 * Real RFC 7591 dynamic client registration for the Temper AS — the door AS mode advertises as
 * `registration_endpoint` (`/oauth/clients`, `api/oauth/clients.ts`).
 *
 * The handler is class-dispatched, because two client populations read the same discovery
 * document and only one of them may mint a client:
 *
 * - **Connect class** — `grant_types` includes `client_credentials`, confidential auth, and the
 *   exact Vercel Connect callback. These requests persist a row in `kb_oauth_dcr_clients` and
 *   receive a `conn_`-prefixed client_id + secret. This is the through-path probe's instrument
 *   (temper-artifacts/plans/2026-09-04-connect-through-path-probe.md): the store is probe-local
 *   and MUST stay disjoint from `kb_machine_clients` — a DCR'd client is not a machine
 *   principal, holds no grants, and is refused at every Rust API gate (lookup-or-401) until the
 *   Phase 1 build binds invocation-envelope authority to it deliberately.
 * - **MCP-compat class** — authorization_code public clients with loopback redirects, the shape
 *   Claude Code/Desktop send on SAML instances. Since AS mode's metadata points
 *   `registration_endpoint` here, this class must keep the thin proxy's semantics
 *   (`crates/temper-mcp/src/discovery.rs`): return the pre-registered `MCP_CLIENT_ID`, filter
 *   redirect_uris through the `AS_CLIENTS` allowlist with the RFC 8252 §7.3 loopback
 *   flexibility (`clients.ts`), mint nothing, return no secret.
 *
 * Everything else is refused `invalid_client_metadata` — fail-closed, including the remote
 * redirect_uri case that `isRedirectUriAllowed` exists to prevent at `/oauth/authorize`.
 * RFC 7592 management is deferred; the response omits `registration_client_uri` /
 * `registration_access_token` rather than advertising a dead URL.
 */

import type { NeonClient } from "../db.js";
import { isRedirectUriAllowed, loadClientRegistry } from "./clients.js";
import { isTemperAsMode } from "./env.js";
import { hashToken, newOpaqueToken } from "./mint.js";

/** The one redirect URI Vercel Connect uses; the provider docs require an exact match. */
const CONNECT_CALLBACK = "https://connect.vercel.com/callback";

/** Grants the server knows how to serve on a registered client. */
const KNOWN_GRANTS = new Set(["authorization_code", "refresh_token", "client_credentials"]);

/** Confidential auth methods a Connect-class client may register with. */
const CONFIDENTIAL_METHODS = new Set(["client_secret_basic", "client_secret_post"]);

interface RegistrationRequest {
  client_name?: unknown;
  redirect_uris?: unknown;
  grant_types?: unknown;
  token_endpoint_auth_method?: unknown;
  logo_uri?: unknown;
}

interface RegistrationResponse {
  status: number;
  body: Record<string, unknown>;
}

function error(status: number, code: string, description?: string): RegistrationResponse {
  return {
    status,
    body: description ? { error: code, error_description: description } : { error: code },
  };
}

function stringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((v) => typeof v === "string")
    ? (value as string[])
    : null;
}

type ClientClass = "connect" | "mcp-compat" | "refused";

/**
 * The load-bearing dispatch. A request's class decides whether a client is minted, echoed, or
 * refused — deciding it on `grant_types` plus auth method keeps the two populations apart
 * without trusting `client_name` (attacker-controlled in an unauthenticated registration).
 * Redirect URIs are NOT a class input: the proxy's incumbent semantic is accept-and-filter
 * (the open-redirect guard lives at `/oauth/authorize`, `clients.ts`), so the MCP-compat echo
 * filters rather than refuses, whatever URIs arrive.
 */
export function classifyRegistration(req: RegistrationRequest): ClientClass {
  const grants = stringArray(req.grant_types);
  if (!grants || grants.length === 0 || grants.some((g) => !KNOWN_GRANTS.has(g))) {
    return "refused";
  }

  if (grants.includes("client_credentials")) {
    // Confidential machine client: the probe's posture — a public machine credential is the
    // downgrade the plan declined, so `none`/absent method is refused by the caller.
    return "connect";
  }

  const method =
    typeof req.token_endpoint_auth_method === "string" ? req.token_endpoint_auth_method : "none";
  if (method === "none") {
    return "mcp-compat";
  }

  return "refused";
}

/** Builds the RFC 7591 §3.2.1 response for a Connect-class client from the persisted row shape. */
function connectResponse(
  clientId: string,
  clientSecret: string,
  req: RegistrationRequest,
  grants: string[],
  redirects: string[],
): RegistrationResponse {
  const body: Record<string, unknown> = {
    client_id: clientId,
    client_secret: clientSecret,
    client_id_issued_at: Math.floor(Date.now() / 1000),
    // 0 = does not expire (RFC 7591 §3.2.1). Rotation is a Phase 1 concern; the probe
    // instance's store dies with the preview branch.
    client_secret_expires_at: 0,
    grant_types: grants,
    redirect_uris: redirects,
    token_endpoint_auth_method: req.token_endpoint_auth_method,
  };
  if (typeof req.client_name === "string" && req.client_name) {
    body.client_name = req.client_name;
  }
  if (typeof req.logo_uri === "string" && req.logo_uri) {
    body.logo_uri = req.logo_uri;
  }
  return { status: 201, body };
}

/** The MCP-compat echo, semantics-matched to the Rust thin proxy (discovery.rs). */
function mcpCompatResponse(req: RegistrationRequest): RegistrationResponse {
  const clientId = process.env.MCP_CLIENT_ID;
  if (!clientId) {
    return error(503, "temporarily_unavailable", "Dynamic client registration is not configured");
  }

  const oauth = loadClientRegistry();
  const redirects = stringArray(req.redirect_uris) ?? [];
  const allowed = redirects.filter((uri) => isRedirectUriAllowed(oauth, clientId, uri));

  return {
    status: 201,
    body: {
      client_id: clientId,
      // The proxy defaults an absent name ("MCP Client", discovery.rs); match it.
      client_name:
        typeof req.client_name === "string" && req.client_name ? req.client_name : "MCP Client",
      redirect_uris: allowed,
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      token_endpoint_auth_method: "none",
    },
  };
}

/** The registered grants of a DCR client, or null when no such client exists. */
export async function dcrClientGrants(db: NeonClient, clientId: string): Promise<string[] | null> {
  const rows = await db`
    SELECT grant_types FROM kb_oauth_dcr_clients WHERE client_id = ${clientId}
  `;
  const row = rows[0] as { grant_types: string[] } | undefined;
  return row ? row.grant_types : null;
}

/** Verifies a Connect-class client's secret at mint time; the DCR arm of `verifyMachineSecret`. */
export async function verifyDcrClientSecret(
  db: NeonClient,
  clientId: string,
  clientSecret: string,
): Promise<boolean> {
  const rows = await db`
    SELECT client_secret_hash FROM kb_oauth_dcr_clients WHERE client_id = ${clientId}
  `;
  const row = rows[0] as { client_secret_hash: string } | undefined;
  if (!row) {
    return false;
  }
  return hashToken(clientSecret) === row.client_secret_hash;
}

/** Persist the Connect-class client; the only write this surface performs. */
async function persistDcrClient(
  db: NeonClient,
  clientId: string,
  secretHash: string,
  req: RegistrationRequest,
  grants: string[],
  redirects: string[],
): Promise<void> {
  await db`
    INSERT INTO kb_oauth_dcr_clients
      (client_id, client_secret_hash, client_name, grant_types, redirect_uris,
       token_endpoint_auth_method, logo_uri)
    VALUES (${clientId}, ${secretHash}, ${typeof req.client_name === "string" ? req.client_name : null},
            ${grants}, ${redirects}, ${String(req.token_endpoint_auth_method)},
            ${typeof req.logo_uri === "string" ? req.logo_uri : null})
  `;
}

/**
 * `POST /oauth/clients` — RFC 7591 registration, AS mode only. Auth0-fronted instances do not
 * advertise this path (their document still points at `/oauth/register`), so the handler 404s
 * without AS_ISSUER rather than serving a second door nobody reads.
 */
export async function handleClientRegistration(req: Request, db: NeonClient): Promise<Response> {
  if (!isTemperAsMode()) {
    return new Response("Not Found", { status: 404 });
  }

  let parsed: RegistrationRequest;
  try {
    parsed = (await req.json()) as RegistrationRequest;
  } catch {
    return respond(error(400, "invalid_client_metadata", "body must be JSON"));
  }

  const klass = classifyRegistration(parsed);

  if (klass === "mcp-compat") {
    return respond(mcpCompatResponse(parsed));
  }

  if (klass === "refused") {
    return respond(
      error(400, "invalid_client_metadata", "this server registers only its documented classes"),
    );
  }

  // ── Connect class ──────────────────────────────────────────────────────────
  const method =
    typeof parsed.token_endpoint_auth_method === "string" ? parsed.token_endpoint_auth_method : "";
  if (!CONFIDENTIAL_METHODS.has(method)) {
    return respond(
      error(
        400,
        "invalid_client_metadata",
        "machine clients must authenticate confidentially (client_secret_basic or client_secret_post)",
      ),
    );
  }

  const redirects = stringArray(parsed.redirect_uris) ?? [];
  if (redirects.length === 0 || redirects.some((uri) => uri !== CONNECT_CALLBACK)) {
    return respond(
      error(400, "invalid_redirect_uri", `the only accepted redirect_uri is ${CONNECT_CALLBACK}`),
    );
  }

  const clientId = `conn_${newOpaqueToken().slice(0, 16)}`;
  const clientSecret = newOpaqueToken();

  // The measurement artifact: what Connect actually registers. Never the secret — this log
  // is read by humans on a preview deployment, and the secret is in the response alone.
  const { logger } = await import("../logger.js");
  logger.info(
    {
      class: klass,
      client_name: parsed.client_name,
      grant_types: parsed.grant_types,
      redirect_uris: redirects,
      token_endpoint_auth_method: method,
      logo_uri: parsed.logo_uri,
      client_id: clientId,
    },
    "DCR: Connect-class client registered",
  );

  await persistDcrClient(
    db,
    clientId,
    hashToken(clientSecret),
    parsed,
    stringArray(parsed.grant_types) ?? [],
    redirects,
  );

  return respond(
    connectResponse(
      clientId,
      clientSecret,
      parsed,
      stringArray(parsed.grant_types) ?? [],
      redirects,
    ),
  );
}

function respond(r: RegistrationResponse): Response {
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: {
      "content-type": "application/json",
      "cache-control": "no-store",
    },
  });
}
