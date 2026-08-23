/**
 * Auth0 loopback redirect-uri proxy.
 *
 * Auth0's RFC 8252 loopback port-wildcard matching (Native app → `http://localhost`
 * matches any port) does not work on all tenants — even with OIDC Conformant enabled.
 * Without it, MCP clients like opencode that use `http://127.0.0.1:<port>/callback` as
 * their redirect_uri get a 403 "Callback URL mismatch" from Auth0's `/authorize`.
 *
 * This module solves it by proxying the authorize and token endpoints through
 * temperkb.io. The flow:
 *
 * 1. MCP client → `GET /oauth/authorize` (our proxy)
 *    - `redirect_uri=http://127.0.0.1:<port>/callback`
 *    - We stash the original `redirect_uri` + `state` in a signed token, then
 *      redirect to Auth0's `/authorize` with `redirect_uri` rewritten to our relay
 *      (`https://<base>/api/auth/mcp-callback`) and `state` set to the signed token.
 *
 * 2. Auth0 → `GET /api/auth/mcp-callback` (our relay)
 *    - Auth0 redirects here with `?code=...&state=<signed_token>`.
 *    - We verify the signed token, extract the original `redirect_uri` + `state`,
 *      and redirect the browser to `http://127.0.0.1:<port>/callback?code=...&state=...`.
 *
 * 3. MCP client → `POST /oauth/token` (our proxy)
 *    - The client exchanges the code using `redirect_uri=http://127.0.0.1:<port>/callback`
 *      (what it originally sent). We rewrite it to the relay URL before forwarding
 *      to Auth0, so Auth0 sees the `redirect_uri` it expects.
 *
 * PKCE passes through untouched — the client's `code_challenge` goes to Auth0 in
 * step 1, and the client's `code_verifier` goes to Auth0 in step 3. We never see
 * or touch either.
 *
 * This proxy only activates for Auth0-fronted instances (no `AS_ISSUER`). SAML
 * instances use the Temper AS (`endpoints.ts`) directly.
 */

import { createHmac, timingSafeEqual } from "node:crypto";

/** The relay URL that Auth0 redirects to after login. */
const RELAY_PATH = "/api/auth/mcp-callback";

/** How long a signed state token is valid (10 minutes, matching PENDING_FLOW_TTL_SECONDS). */
const STATE_TTL_MS = 10 * 60 * 1000;

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing required environment variable: ${name}`);
  return v;
}

/**
 * Derives the HMAC key from a dedicated secret env var.
 *
 * `MCP_PROXY_SECRET` must be a random string of at least 32 bytes, generated
 * once per instance (e.g. `openssl rand -base64 48`). It is never published in
 * any metadata or DCR response, so only the server can sign or verify state
 * tokens. Falls back to `AUTH_ISSUER` + `AUTH_AUDIENCE` for backward
 * compatibility with instances that have not yet set `MCP_PROXY_SECRET`, but
 * that fallback is deprecated and will be removed.
 */
function signingKey(): string {
  const secret = process.env.MCP_PROXY_SECRET;
  if (secret && secret.length >= 32) {
    return secret;
  }
  // Deprecated fallback — public values, forgeable. Removed once all instances
  // set MCP_PROXY_SECRET.
  return [requireEnv("MCP_BASE_URL"), requireEnv("AUTH_AUDIENCE")].join("|");
}

/** The stashed data we carry through Auth0's `state` parameter. */
interface StashedState {
  /** Original redirect_uri from the MCP client (e.g. `http://127.0.0.1:19876/mcp/oauth/callback`). */
  r: string;
  /** Original state from the MCP client (CSRF token). */
  s: string;
  /** Expiry (epoch ms). */
  e: number;
}

function b64url(input: Buffer | string): string {
  return Buffer.from(input).toString("base64url");
}

function sign(payload: string): string {
  return b64url(createHmac("sha256", signingKey()).update(payload).digest());
}

/** Encodes `{r, s, e}` into a signed `base64url(json).base64url(hmac)` token. */
function encodeStashedState(redirectUri: string, oauthState: string): string {
  const payload: StashedState = {
    r: redirectUri,
    s: oauthState,
    e: Date.now() + STATE_TTL_MS,
  };
  const json = b64url(JSON.stringify(payload));
  return `${json}.${sign(json)}`;
}

/** Verifies the HMAC and expiry, returning the stashed state. Throws on failure. */
function decodeStashedState(token: string): StashedState {
  const dot = token.indexOf(".");
  if (dot <= 0) throw new Error("malformed state token");

  const jsonPart = token.slice(0, dot);
  const sigPart = token.slice(dot + 1);
  const expected = sign(jsonPart);

  const a = Buffer.from(sigPart);
  const b = Buffer.from(expected);
  if (a.length !== b.length || !timingSafeEqual(a, b)) {
    throw new Error("invalid state signature");
  }

  const payload = JSON.parse(Buffer.from(jsonPart, "base64url").toString("utf8")) as StashedState;
  if (Date.now() > payload.e) throw new Error("expired state token");
  return payload;
}

function redirect(location: string): Response {
  return new Response(null, { status: 302, headers: { location } });
}

function badRequest(reason: string): Response {
  return new Response(reason, { status: 400 });
}

function isValidUrl(value: string): boolean {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

/** True if a redirect_uri is a loopback address that needs proxying. */
function isLoopbackRedirect(uri: string): boolean {
  try {
    const u = new URL(uri);
    return (
      u.protocol === "http:" &&
      (u.hostname === "localhost" || u.hostname === "127.0.0.1" || u.hostname === "[::1]")
    );
  } catch {
    return false;
  }
}

/**
 * `GET /oauth/authorize` — Auth0 proxy entry point.
 *
 * If the client's `redirect_uri` is a loopback URL, rewrites it to the relay and
 * stashes the original in a signed `state` token. If it's already a non-loopback
 * URL (e.g. `https://claude.ai/api/mcp/auth_callback`), passes through to Auth0
 * unchanged — those work fine with Auth0's exact-match allowlist.
 */
export async function proxyAuthorize(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const params = url.searchParams;

  const redirectUri = params.get("redirect_uri");
  const state = params.get("state");

  if (!redirectUri || !isValidUrl(redirectUri))
    return badRequest("redirect_uri is required and must be a valid URL");
  if (!state) return badRequest("state is required");

  const auth0Domain = requireEnv("AUTH_ISSUER").replace(/\/+$/, "");
  const relayUri = `${requireEnv("MCP_BASE_URL")}${RELAY_PATH}`;

  if (isLoopbackRedirect(redirectUri)) {
    // Stash original redirect_uri + state, rewrite redirect_uri to our relay.
    const stashed = encodeStashedState(redirectUri, state);

    const auth0Params = new URLSearchParams(params);
    auth0Params.set("redirect_uri", relayUri);
    auth0Params.set("state", stashed);

    return redirect(`${auth0Domain}/authorize?${auth0Params.toString()}`);
  }

  // Non-loopback redirect (Claude Desktop's HTTPS callback, etc.) — pass through.
  return redirect(`${auth0Domain}/authorize?${params.toString()}`);
}

/**
 * `GET /api/auth/mcp-callback` — relay that Auth0 redirects to after login.
 *
 * Verifies the signed state token, extracts the original redirect_uri + state,
 * and redirects the browser to the MCP client's local callback server with the
 * authorization code.
 */
export function handleMcpCallback(req: Request): Response {
  const url = new URL(req.url);

  const error = url.searchParams.get("error");
  if (error) {
    const description = url.searchParams.get("error_description") ?? "unknown error";
    return new Response(`Authentication failed: ${error} — ${description}`, { status: 400 });
  }

  const code = url.searchParams.get("code");
  const stashed = url.searchParams.get("state");
  if (!code || !stashed) return badRequest("Missing code or state parameter");

  let original: StashedState;
  try {
    original = decodeStashedState(stashed);
  } catch {
    return badRequest("Invalid or expired state token");
  }

  const target = new URL(original.r);
  target.searchParams.set("code", code);
  target.searchParams.set("state", original.s);

  return redirect(target.toString());
}

/**
 * `POST /oauth/token` — Auth0 proxy token endpoint.
 *
 * For `authorization_code` grants, rewrites `redirect_uri` from the client's
 * loopback URL to the relay URL so Auth0 sees the `redirect_uri` it received
 * during `/authorize`. For `refresh_token` and `client_credentials` grants,
 * passes through to Auth0 unchanged (neither carries a `redirect_uri`).
 */
export async function proxyToken(req: Request): Promise<Response> {
  const auth0Domain = requireEnv("AUTH_ISSUER").replace(/\/+$/, "");
  const relayUri = `${requireEnv("MCP_BASE_URL")}${RELAY_PATH}`;

  const body = await req.text();
  const params = new URLSearchParams(body);
  const grantType = params.get("grant_type");

  if (grantType === "authorization_code") {
    const redirectUri = params.get("redirect_uri");
    if (redirectUri && isLoopbackRedirect(redirectUri)) {
      params.set("redirect_uri", relayUri);
    }
  }

  const tokenResp = await fetch(`${auth0Domain}/oauth/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: params.toString(),
  });

  const respBody = await tokenResp.text();
  return new Response(respBody, {
    status: tokenResp.status,
    headers: {
      "content-type": tokenResp.headers.get("content-type") ?? "application/json",
      "cache-control": "no-store",
    },
  });
}
